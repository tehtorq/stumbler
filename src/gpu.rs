//! wgpu renderer: device/surface setup, meshes, pipelines and frame submission.
//!
//! Bind group layout used by every pipeline:
//! - group 0: per-frame `Globals` (camera, sun, fog, lights, sway, screen size)
//! - group 1 (mesh pipelines): per-draw `Object` model matrix at a dynamic offset
//!
//! The world renders in linear HDR with 4x MSAA, resolved into a float
//! target. Post passes then work out ambient occlusion from the depth
//! buffer, blur the brightest parts into bloom, and tonemap the lot into the
//! swapchain image, with the HUD drawn on top of that.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::geometry::Vertex;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The lit world before tonemapping: linear light, values above 1 allowed.
/// Packed into 32 bits where the GPU can render to that (half the memory
/// traffic of 16-bit floats, which counts on integrated graphics).
const HDR_PACKED: wgpu::TextureFormat = wgpu::TextureFormat::Rg11b10Ufloat;
const HDR_WIDE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Ambient occlusion and the distance to the surface, at half resolution.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// Depth at half resolution, single-sampled, for the AO pass to read.
const HALF_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Steps down the bloom chain from half resolution (fewer on a small window).
const BLOOM_MIPS: u32 = 6;
const SAMPLE_COUNT: u32 = 4;
/// Texels a side of the sun's shadow map.
const SHADOW_SIZE: u32 = 2048;
/// World units from the map's middle to its edge. Must fit `shadow.wgsl`'s texel size.
pub const SHADOW_REACH: f32 = 420.0;
/// How far toward and away from the sun the map holds casters.
pub const SHADOW_DEPTH: f32 = 1400.0;
const CLEAR_COLOR: wgpu::Color = wgpu::Color { r: 0.7, g: 0.9, b: 1.0, a: 1.0 };

/// Per-frame uniforms. Must match `Globals` in shaders/common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Globals {
    pub view_proj: Mat4,
    pub inv_view_proj: Mat4,
    pub sun_dir: [f32; 3],
    pub wind_time: f32,
    pub cam_pos: [f32; 3],
    pub light_count: u32,
    pub sun_screen: [f32; 3],
    pub sway_count: u32,
    /// Two per struck tree: (x, z, base height, bend), (bend direction x, z, -, -).
    pub sway: [[f32; 4]; 16],
    pub lights: [[f32; 4]; 24],
    /// rgb = fog / horizon colour, a = density
    pub fog: [f32; 4],
    /// xy = framebuffer size in pixels, z = rain, w = road wetness
    pub screen: [f32; 4],
    /// World to the sun's shadow map.
    pub shadow_view_proj: Mat4,
    /// xy = the car they're cheering, z = excitement 0..1, w = spare
    pub crowd: [f32; 4],
    /// The streetlights nearest the camera: lamp xyz, w = how far on (0..1).
    pub street: [[f32; 4]; MAX_STREET_LAMPS],
    pub street_count: u32,
    pub _pad: [u32; 3],
}

/// Streetlights the shaders light the world with at once (the nearest).
pub const MAX_STREET_LAMPS: usize = 32;

/// Times the GPU's passes (profiling only). Results are read
/// back a few frames later through a small ring of buffers, so the pipeline
/// never stalls waiting for them.
pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    ring: Vec<TimerSlot>,
    next: usize,
    /// Nanoseconds per timestamp tick.
    period: f32,
    total_ms: [f64; TIMED_PASSES.len()],
    frames: u32,
}

/// The stretches of the frame timed, each between two timestamps.
const TIMED_PASSES: [&str; 5] = ["shadow", "main", "ao", "bloom", "post+hud"];
const TIMER_BYTES: u64 = TIMED_PASSES.len() as u64 * 16;

struct TimerSlot {
    buffer: wgpu::Buffer,
    /// Set by the map callback once the buffer can be read.
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    in_flight: bool,
}

const TIMER_RING: usize = 4;

impl GpuTimer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let buffer = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: TIMER_BYTES, usage, mapped_at_creation: false });
        Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timer"),
                ty: wgpu::QueryType::Timestamp,
                count: TIMED_PASSES.len() as u32 * 2,
            }),
            resolve: buffer("timer resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            ring: (0..TIMER_RING)
                .map(|_| TimerSlot {
                    buffer: buffer("timer readback", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
                    ready: Default::default(),
                    in_flight: false,
                })
                .collect(),
            next: 0,
            period: queue.get_timestamp_period(),
            total_ms: [0.0; TIMED_PASSES.len()],
            frames: 0,
        }
    }

    /// The readback buffer this frame's result goes to, if one is free.
    fn slot(&self) -> Option<&wgpu::Buffer> {
        let slot = &self.ring[self.next];
        (!slot.in_flight).then_some(&slot.buffer)
    }

    /// After submitting: start mapping this frame's result, and collect any
    /// earlier results that have arrived.
    fn after_submit(&mut self, device: &wgpu::Device) {
        use std::sync::atomic::Ordering;
        let slot = &mut self.ring[self.next];
        if !slot.in_flight {
            slot.in_flight = true;
            let ready = slot.ready.clone();
            slot.buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| ready.store(r.is_ok(), Ordering::Release));
            self.next = (self.next + 1) % TIMER_RING;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.ring {
            if slot.in_flight && slot.ready.swap(false, Ordering::Acquire) {
                if let Ok(data) = slot.buffer.slice(..).get_mapped_range() {
                    let t: &[u64] = bytemuck::cast_slice(&data);
                    for (k, total) in self.total_ms.iter_mut().enumerate() {
                        *total += t[2 * k + 1].saturating_sub(t[2 * k]) as f64 * self.period as f64 / 1e6;
                    }
                    self.frames += 1;
                }
                slot.buffer.unmap();
                slot.in_flight = false;
            }
        }
    }

    /// Average GPU time of each timed pass so far, in milliseconds.
    pub fn average_ms(&self) -> String {
        let n = self.frames.max(1) as f64;
        let parts: Vec<String> = TIMED_PASSES.iter().zip(&self.total_ms).map(|(name, ms)| format!("{name} {:.2}", ms / n)).collect();
        parts.join(", ")
    }

    /// Timestamps for timed pass `k`, at its start and/or its end.
    fn writes(&self, k: usize, start: bool, end: bool) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        (start || end).then_some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: start.then_some(2 * k as u32),
            end_of_pass_write_index: end.then_some(2 * k as u32 + 1),
        })
    }
}

/// Which pipeline a draw uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PipelineKind {
    /// Opaque flat-coloured ground.
    Terrain,
    /// Vertex-colored geometry. Alpha-blended with depth write, no culling.
    Mesh,
    /// Alpha-blended billboards, no depth write. Draw after opaque geometry.
    Particles,
    /// Screen-space overlay: vertex positions are in pixels. Draw last.
    Hud,
}

pub trait IndexType: bytemuck::Pod {
    const FORMAT: wgpu::IndexFormat;
}
impl IndexType for u16 {
    const FORMAT: wgpu::IndexFormat = wgpu::IndexFormat::Uint16;
}
impl IndexType for u32 {
    const FORMAT: wgpu::IndexFormat = wgpu::IndexFormat::Uint32;
}

pub struct Mesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_format: wgpu::IndexFormat,
    pub index_count: u32,
}

impl Mesh {
    pub fn new<I: IndexType>(gpu: &GpuContext, vertices: &[Vertex], indices: &[I]) -> Self {
        let vertex_buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self { vertex_buffer, index_buffer, index_format: I::FORMAT, index_count: indices.len() as u32 }
    }

    /// Overwrite vertex data in place (same vertex count, e.g. recolouring a car).
    pub fn write_vertices(&self, gpu: &GpuContext, vertices: &[Vertex]) {
        gpu.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(vertices));
    }
}

/// A mesh rebuilt on the CPU every frame. Buffers grow as needed.
pub struct DynamicMesh {
    mesh: Mesh,
    vertex_capacity: u64,
    index_capacity: u64,
}

impl DynamicMesh {
    pub fn new(gpu: &GpuContext, label: &str) -> Self {
        let vertex_capacity = 4096;
        let index_capacity = 4096;
        Self {
            mesh: Mesh {
                vertex_buffer: Self::buffer(gpu, label, vertex_capacity, wgpu::BufferUsages::VERTEX),
                index_buffer: Self::buffer(gpu, label, index_capacity, wgpu::BufferUsages::INDEX),
                index_format: wgpu::IndexFormat::Uint16,
                index_count: 0,
            },
            vertex_capacity,
            index_capacity,
        }
    }

    fn buffer(gpu: &GpuContext, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn update(&mut self, gpu: &GpuContext, vertices: &[Vertex], indices: &[u16]) {
        self.mesh.index_count = indices.len() as u32;
        if indices.is_empty() {
            return;
        }

        let vertex_bytes: &[u8] = bytemuck::cast_slice(vertices);
        // write_buffer needs a 4-byte multiple; pad odd u16 counts
        let mut index_bytes: Vec<u8> = bytemuck::cast_slice(indices).to_vec();
        index_bytes.resize(index_bytes.len().next_multiple_of(4), 0);

        if vertex_bytes.len() as u64 > self.vertex_capacity {
            self.vertex_capacity = (vertex_bytes.len() as u64).next_power_of_two();
            self.mesh.vertex_buffer =
                Self::buffer(gpu, "dynamic vertices", self.vertex_capacity, wgpu::BufferUsages::VERTEX);
        }
        if index_bytes.len() as u64 > self.index_capacity {
            self.index_capacity = (index_bytes.len() as u64).next_power_of_two();
            self.mesh.index_buffer =
                Self::buffer(gpu, "dynamic indices", self.index_capacity, wgpu::BufferUsages::INDEX);
        }

        gpu.queue.write_buffer(&self.mesh.vertex_buffer, 0, vertex_bytes);
        gpu.queue.write_buffer(&self.mesh.index_buffer, 0, &index_bytes);
    }

    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }
}

/// One draw call for `Gpu::render`.
pub struct Draw<'a> {
    pub pipeline: PipelineKind,
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub kind: MeshKind,
}

/// How the mesh shader treats a draw.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MeshKind {
    Plain = 0,
    /// Vegetation: vertex alpha is a sway weight (0 at the base), not opacity,
    /// and only foliage bends in the wind or when struck.
    Foliage = 1,
    /// Water: vertex alpha is depth (0 at the shore); waves, foam and glints.
    Water = 2,
    /// Small plants near the camera: like foliage, and vertex alpha (height
    /// above the ground / 4) also lets them sink away with distance.
    Cover = 3,
    /// Lit like any mesh by day, and gives off its own colour after dark.
    Glow = 4,
    /// Smoke, dust, steam and fireflies: their own colour, dimmed at night;
    /// any channel above 1 marks a light (fireflies) shown at full strength.
    Particle = 5,
    /// People: vertex alpha holds their rhythm (see `props::Layer`).
    Crowd = 6,
    /// Cloth: vertex alpha is how free it is to flap.
    Flag = 7,
    /// Car paint and glass: glossy, catching the sun and reflecting the sky.
    Paint = 8,
}

impl<'a> Draw<'a> {
    pub fn new(pipeline: PipelineKind, mesh: &'a Mesh, model: Mat4) -> Self {
        Self { pipeline, mesh, model, kind: MeshKind::Plain }
    }

    pub fn mesh(mesh: &'a Mesh, model: Mat4) -> Self {
        Self::new(PipelineKind::Mesh, mesh, model)
    }

    pub fn foliage(mesh: &'a Mesh) -> Self {
        Self { kind: MeshKind::Foliage, ..Self::mesh(mesh, Mat4::IDENTITY) }
    }

    pub fn cover(mesh: &'a Mesh) -> Self {
        Self { kind: MeshKind::Cover, ..Self::mesh(mesh, Mat4::IDENTITY) }
    }

    pub fn glow(mesh: &'a Mesh) -> Self {
        Self { kind: MeshKind::Glow, ..Self::mesh(mesh, Mat4::IDENTITY) }
    }

    pub fn crowd(mesh: &'a Mesh) -> Self {
        Self { kind: MeshKind::Crowd, ..Self::mesh(mesh, Mat4::IDENTITY) }
    }

    pub fn flags(mesh: &'a Mesh) -> Self {
        Self { kind: MeshKind::Flag, ..Self::mesh(mesh, Mat4::IDENTITY) }
    }

    /// Flag cloth placed by `model` (for cloth that moves as a whole).
    pub fn flags_at(mesh: &'a Mesh, model: Mat4) -> Self {
        Self { kind: MeshKind::Flag, ..Self::mesh(mesh, model) }
    }

    pub fn paint(mesh: &'a Mesh, model: Mat4) -> Self {
        Self { kind: MeshKind::Paint, ..Self::mesh(mesh, model) }
    }

    pub fn water(mesh: &'a Mesh) -> Self {
        Self { kind: MeshKind::Water, ..Self::mesh(mesh, Mat4::IDENTITY) }
    }
}

/// Per-draw uniforms. Must match `Object` in shaders/mesh.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectUniform {
    model: Mat4,
    /// x = `MeshKind`
    params: [f32; 4],
}

/// The device and queue: all that building meshes needs. Cheap to clone, so
/// the world can load on another thread while the window keeps drawing.
#[derive(Clone)]
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

pub struct Gpu {
    pub window: Arc<Window>,
    /// Triangles submitted in the last frame (reported by screenshot runs).
    pub last_triangles: u64,
    /// Draw calls in the last frame.
    pub last_draw_calls: u32,
    /// GPU timing when profiling.
    pub timer: Option<GpuTimer>,
    surface: wgpu::Surface<'static>,
    context: GpuContext,
    config: wgpu::SurfaceConfiguration,
    targets: Targets,
    post_layouts: PostLayouts,
    linear_sampler: wgpu::Sampler,

    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,

    object_layout: wgpu::BindGroupLayout,
    object_buffer: wgpu::Buffer,
    object_bind_group: wgpu::BindGroup,
    object_capacity: u64,
    object_stride: u64,

    shadow_view: wgpu::TextureView,
    shadow_globals: wgpu::BindGroup,
    caster_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    terrain_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    particle_pipeline: wgpu::RenderPipeline,
    hud_pipeline: wgpu::RenderPipeline,
    ao_pipeline: wgpu::RenderPipeline,
    half_depth_pipeline: wgpu::RenderPipeline,
    bloom_prefilter_pipeline: wgpu::RenderPipeline,
    bloom_down_pipeline: wgpu::RenderPipeline,
    bloom_up_pipeline: wgpu::RenderPipeline,
    post_pipeline: wgpu::RenderPipeline,

    /// Dev aid: save the next frame to this path (binary PPM).
    pub capture_path: Option<PathBuf>,
}

impl std::ops::Deref for Gpu {
    type Target = GpuContext;

    fn deref(&self) -> &GpuContext {
        &self.context
    }
}

impl Gpu {
    /// A handle for building meshes on another thread.
    pub fn context(&self) -> GpuContext {
        self.context.clone()
    }

    pub async fn new(window: Arc<Window>, display: winit::event_loop::OwnedDisplayHandle) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
            Box::new(display),
        ));
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no suitable GPU adapter");
        // Dev aid: STUMBLER_PROFILE=1 times the GPU's work with timestamp queries
        let profile = std::env::var_os("STUMBLER_PROFILE").is_some()
            && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let packed_hdr = adapter.features().contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE);
        let hdr_format = if packed_hdr { HDR_PACKED } else { HDR_WIDE };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                required_features: if profile { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() }
                    | if packed_hdr { wgpu::Features::RG11B10UFLOAT_RENDERABLE } else { wgpu::Features::empty() },
                ..Default::default()
            })
            .await
            .expect("request device");
        let timer = profile.then(|| GpuTimer::new(&device, &queue));

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        // Shaders output display-ready colors (as the GL version did), so avoid sRGB encoding
        let caps = surface.get_capabilities(&adapter);
        let preferred = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm];
        if let Some(&format) = preferred.iter().find(|f| caps.formats.contains(f)) {
            config.format = format;
        }
        // Uncapped frame rate: present immediately if the surface allows it
        // (may tear), else mailbox (no tearing), else fall back to vsync.
        // STUMBLER_VSYNC=1 turns vsync back on.
        let wanted: &[wgpu::PresentMode] = if std::env::var_os("STUMBLER_VSYNC").is_some() {
            &[wgpu::PresentMode::Fifo]
        } else {
            &[wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo]
        };
        config.present_mode = wanted.iter().copied().find(|m| caps.present_modes.contains(m)).unwrap_or(wgpu::PresentMode::Fifo);
        eprintln!("present mode {:?} (surface offers {:?})", config.present_mode, caps.present_modes);
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC; // for frame captures
        }
        surface.configure(&device, &config);

        // --- Group 0: globals ---
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[
                uniform_entry(0, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow globals layout"),
            entries: &[uniform_entry(0, false)],
        });
        let shadow_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow map"),
                size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        let shadow_globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow globals"),
            layout: &shadow_globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() }],
        });

        // --- Group 1 (mesh): per-object model matrix at a dynamic offset ---
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object layout"),
            entries: &[uniform_entry(0, true)],
        });
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let object_stride = (std::mem::size_of::<ObjectUniform>() as u64).next_multiple_of(align);
        let object_capacity = 256;
        let (object_buffer, object_bind_group) =
            create_object_buffer(&device, &object_layout, object_stride, object_capacity);

        // --- Pipelines ---
        let shader = |name: &str, src: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(format!("{}\n{}", include_str!("shaders/common.wgsl"), src).into()),
            })
        };
        let lit = |src: &str| format!("{}\n{src}", include_str!("shaders/shadow.wgsl"));
        let mesh_shader = shader("mesh", &lit(include_str!("shaders/mesh.wgsl")));
        let terrain_shader = shader("terrain", &lit(include_str!("shaders/terrain.wgsl")));
        let caster_shader = shader("caster", include_str!("shaders/caster.wgsl"));
        let sky_shader = shader("sky", include_str!("shaders/sky.wgsl"));
        let hud_shader = shader("hud", include_str!("shaders/hud.wgsl"));

        let layout = |label: &str, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let mesh_layout = layout("mesh", &[Some(&globals_layout), Some(&object_layout)]);
        let globals_only = layout("globals only", &[Some(&globals_layout)]);

        // The casters' pass can't have the map it's drawing bound as well
        let caster_layout = layout("caster", &[Some(&shadow_globals_layout), Some(&object_layout)]);
        let caster_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("caster"),
            layout: Some(&caster_layout),
            vertex: wgpu::VertexState {
                module: &caster_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout())],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 4, slope_scale: 2.5, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let builder = PipelineBuilder { device: &device, format: hdr_format, samples: SAMPLE_COUNT, depth: true };
        let sky_pipeline = builder.build(PipelineDesc {
            label: "sky",
            layout: &globals_only,
            shader: &sky_shader,
            vertex_buffer: false,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: None,
            blend: None,
        });
        let terrain_pipeline = builder.build(PipelineDesc {
            label: "terrain",
            layout: &globals_only,
            shader: &terrain_shader,
            vertex_buffer: true,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: None, // chunk skirts are seen from either side
            blend: None,
        });
        let mesh_pipeline = builder.build(PipelineDesc {
            label: "mesh",
            layout: &mesh_layout,
            shader: &mesh_shader,
            vertex_buffer: true,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: None,
            blend: Some(alpha_blend()),
        });
        let particle_pipeline = builder.build(PipelineDesc {
            label: "particles",
            layout: &mesh_layout,
            shader: &mesh_shader,
            vertex_buffer: true,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: None,
            blend: Some(alpha_blend()),
        });
        // The HUD goes straight onto the finished, tonemapped frame
        let screen_builder = PipelineBuilder { device: &device, format: config.format, samples: 1, depth: false };
        let hud_pipeline = screen_builder.build(PipelineDesc {
            label: "hud",
            layout: &globals_only,
            shader: &hud_shader,
            vertex_buffer: true,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            cull: None,
            blend: Some(alpha_blend()),
        });

        // --- Post: AO, bloom, tonemap ---
        let texture_entry = |binding, sample_type, multisampled| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
            count: None,
        };
        let filterable = wgpu::TextureSampleType::Float { filterable: true };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout"),
            entries: &[
                uniform_entry(0, false),
                texture_entry(1, filterable, false),
                texture_entry(2, filterable, false),
                texture_entry(3, filterable, false),
                sampler_entry(5),
            ],
        });
        // Depth to half resolution, then AO from that (neither can read its own target)
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("half depth layout"),
            entries: &[uniform_entry(0, false), texture_entry(4, wgpu::TextureSampleType::Depth, true)],
        });
        let ao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ao layout"),
            entries: &[uniform_entry(0, false), texture_entry(6, wgpu::TextureSampleType::Float { filterable: false }, false)],
        });
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom layout"),
            entries: &[texture_entry(0, filterable, false), sampler_entry(1)],
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let fullscreen = include_str!("shaders/fullscreen.wgsl");
        let ao_shader = shader("ao", &format!("{fullscreen}\n{}", include_str!("shaders/ao.wgsl")));
        let post_shader = shader("post", &format!("{fullscreen}\n{}", include_str!("shaders/post.wgsl")));
        let bloom_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(format!("{fullscreen}\n{}", include_str!("shaders/bloom.wgsl")).into()),
        });
        let post_pipeline_layout = layout("post", &[Some(&post_layout)]);
        let bloom_pipeline_layout = layout("bloom", &[Some(&bloom_layout)]);
        let fullscreen_pipeline = |label, layout, module, entry, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let ao_pipeline_layout = layout("ao", &[Some(&ao_layout)]);
        let ao_pipeline = fullscreen_pipeline("ao", &ao_pipeline_layout, &ao_shader, "fs_main", AO_FORMAT, None);
        let depth_pipeline_layout = layout("half depth", &[Some(&depth_layout)]);
        let half_depth_pipeline = fullscreen_pipeline("half depth", &depth_pipeline_layout, &ao_shader, "fs_half_depth", HALF_DEPTH_FORMAT, None);
        let post_pipeline = fullscreen_pipeline("post", &post_pipeline_layout, &post_shader, "fs_main", config.format, None);
        let bloom_prefilter_pipeline = fullscreen_pipeline("bloom prefilter", &bloom_pipeline_layout, &bloom_shader, "fs_prefilter", hdr_format, None);
        let bloom_down_pipeline = fullscreen_pipeline("bloom down", &bloom_pipeline_layout, &bloom_shader, "fs_down", hdr_format, None);
        let bloom_up_pipeline = fullscreen_pipeline("bloom up", &bloom_pipeline_layout, &bloom_shader, "fs_up", hdr_format, Some(additive));
        let post_layouts = PostLayouts { post: post_layout, depth: depth_layout, ao: ao_layout, bloom: bloom_layout, hdr_format };
        let targets = Targets::new(&device, &config, &post_layouts, &globals_buffer, &linear_sampler);

        Self {
            window,
            surface,
            context: GpuContext { device, queue },
            config,
            targets,
            post_layouts,
            linear_sampler,
            globals_buffer,
            globals_bind_group,
            object_layout,
            object_buffer,
            object_bind_group,
            object_capacity,
            object_stride,
            shadow_view,
            shadow_globals,
            caster_pipeline,
            sky_pipeline,
            terrain_pipeline,
            mesh_pipeline,
            particle_pipeline,
            hud_pipeline,
            ao_pipeline,
            half_depth_pipeline,
            bloom_prefilter_pipeline,
            bloom_down_pipeline,
            bloom_up_pipeline,
            post_pipeline,
            capture_path: None,
            timer,
            last_draw_calls: 0,
            last_triangles: 0,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.context.device, &self.config);
        self.targets = Targets::new(&self.context.device, &self.config, &self.post_layouts, &self.globals_buffer, &self.linear_sampler);
    }

    /// Framebuffer size in pixels.
    pub fn size(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    /// Bring the surface in line with the window's actual size. A Resized
    /// event may not have arrived yet, so the cached config can be stale.
    fn reconfigure(&mut self, force: bool) {
        let size = self.window.inner_size();
        if force || (size.width, size.height) != (self.config.width, self.config.height) {
            self.resize(size.width, size.height);
        }
    }

    /// Render one frame: sky, then `draws` in order.
    /// `casters` are drawn into the sun's shadow map first.
    pub fn render(&mut self, globals: &Globals, casters: &[Draw], draws: &[Draw]) {
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            // Still presentable: draw it, then reconfigure once it's out
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure(true);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Upload per-frame and per-object uniforms
        self.context.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(globals));
        let objects_needed = (draws.len() + casters.len()) as u64;
        if objects_needed > self.object_capacity {
            self.object_capacity = objects_needed.next_power_of_two();
            (self.object_buffer, self.object_bind_group) =
                create_object_buffer(&self.context.device, &self.object_layout, self.object_stride, self.object_capacity);
        }
        let mut objects = vec![0u8; objects_needed as usize * self.object_stride as usize];
        for (i, draw) in draws.iter().chain(casters).enumerate() {
            let offset = i * self.object_stride as usize;
            let object = ObjectUniform { model: draw.model, params: [draw.kind as u32 as f32, 0.0, 0.0, 0.0] };
            let size = std::mem::size_of::<ObjectUniform>();
            objects[offset..offset + size].copy_from_slice(bytemuck::bytes_of(&object));
        }
        if !objects.is_empty() {
            self.context.queue.write_buffer(&self.object_buffer, 0, &objects);
        }

        let mut encoder = self.context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(0, true, true)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.caster_pipeline);
            pass.set_bind_group(0, &self.shadow_globals, &[]);
            for (i, draw) in casters.iter().enumerate() {
                if draw.mesh.index_count == 0 {
                    continue;
                }
                let offset = ((draws.len() + i) as u64 * self.object_stride) as u32;
                pass.set_bind_group(1, &self.object_bind_group, &[offset]);
                pass.set_vertex_buffer(0, draw.mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(draw.mesh.index_buffer.slice(..), draw.mesh.index_format);
                pass.draw_indexed(0..draw.mesh.index_count, 0, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.color,
                    depth_slice: None,
                    resolve_target: Some(&self.targets.hdr),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(CLEAR_COLOR), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth,
                    // Kept for the AO and post passes
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(1, true, true)),
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_bind_group(0, &self.globals_bind_group, &[]);

            let mut current = None;
            let mut sky_drawn = false;
            self.last_triangles = 0;
            self.last_draw_calls = 0;
            for (i, draw) in draws.iter().enumerate() {
                // The HUD waits for the finished frame
                if draw.mesh.index_count == 0 || draw.pipeline == PipelineKind::Hud {
                    continue;
                }
                // The sky goes in once the opaque world is down (it only fills
                // uncovered pixels) and before anything see-through
                let see_through = draw.kind == MeshKind::Water || draw.pipeline == PipelineKind::Particles;
                if see_through && !sky_drawn {
                    pass.set_pipeline(&self.sky_pipeline);
                    pass.draw(0..3, 0..1);
                    sky_drawn = true;
                    current = None;
                }
                if current != Some(draw.pipeline) {
                    current = Some(draw.pipeline);
                    pass.set_pipeline(match draw.pipeline {
                        PipelineKind::Terrain => &self.terrain_pipeline,
                        PipelineKind::Mesh => &self.mesh_pipeline,
                        PipelineKind::Particles => &self.particle_pipeline,
                        PipelineKind::Hud => unreachable!(),
                    });
                }
                if matches!(draw.pipeline, PipelineKind::Mesh | PipelineKind::Particles) {
                    let offset = (i as u64 * self.object_stride) as u32;
                    pass.set_bind_group(1, &self.object_bind_group, &[offset]);
                }
                pass.set_vertex_buffer(0, draw.mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(draw.mesh.index_buffer.slice(..), draw.mesh.index_format);
                pass.draw_indexed(0..draw.mesh.index_count, 0, 0..1);
                self.last_triangles += draw.mesh.index_count as u64 / 3;
                self.last_draw_calls += 1;
            }
            if !sky_drawn {
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
            }
        }
        self.encode_post(&mut encoder, &view);
        {
            let stamps = self.timer.as_ref().and_then(|t| t.writes(4, false, true));
            let mut pass = screen_pass(&mut encoder, "hud pass", &view, wgpu::LoadOp::Load, stamps);
            pass.set_pipeline(&self.hud_pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            for draw in draws.iter().filter(|d| d.pipeline == PipelineKind::Hud && d.mesh.index_count > 0) {
                pass.set_vertex_buffer(0, draw.mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(draw.mesh.index_buffer.slice(..), draw.mesh.index_format);
                pass.draw_indexed(0..draw.mesh.index_count, 0, 0..1);
                self.last_triangles += draw.mesh.index_count as u64 / 3;
                self.last_draw_calls += 1;
            }
        }
        if let Some(t) = &self.timer {
            if let Some(readback) = t.slot() {
                encoder.resolve_query_set(&t.queries, 0..TIMED_PASSES.len() as u32 * 2, &t.resolve, 0);
                encoder.copy_buffer_to_buffer(&t.resolve, 0, readback, 0, TIMER_BYTES);
            }
        }
        let capture = self.capture_path.take().map(|path| (path, self.encode_capture(&mut encoder, &frame.texture)));
        self.context.queue.submit([encoder.finish()]);
        if let Some((path, (buffer, padded_row))) = capture {
            self.save_capture(&path, &buffer, padded_row);
        }
        if let Some(t) = &mut self.timer {
            t.after_submit(&self.context.device);
        }
        // Tells winit to pace redraws by the compositor's frame callbacks
        // (Wayland), which would cap the frame rate; only wanted with vsync
        if self.config.present_mode == wgpu::PresentMode::Fifo {
            self.window.pre_present_notify();
        }
        self.context.queue.present(frame);
        // Only on a size change: reconfiguring at the same size wouldn't clear
        // it and would rebuild the swapchain every frame
        if suboptimal {
            self.reconfigure(false);
        }
    }
}

impl Gpu {
    /// AO and bloom from the resolved frame, then the tonemapped frame into `view`.
    fn encode_post(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let t = &self.targets;
        {
            let stamps = self.timer.as_ref().and_then(|t| t.writes(2, true, false));
            let mut pass = screen_pass(encoder, "half depth pass", &t.half_depth, wgpu::LoadOp::Clear(wgpu::Color::WHITE), stamps);
            pass.set_pipeline(&self.half_depth_pipeline);
            pass.set_bind_group(0, &t.depth_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let stamps = self.timer.as_ref().and_then(|t| t.writes(2, false, true));
            let mut pass = screen_pass(encoder, "ao pass", &t.ao, wgpu::LoadOp::Clear(wgpu::Color::WHITE), stamps);
            pass.set_pipeline(&self.ao_pipeline);
            pass.set_bind_group(0, &t.ao_group, &[]);
            pass.draw(0..3, 0..1);
        }
        // Down the chain: the frame into the first mip, each mip into the next
        for (i, mip) in t.bloom_mips.iter().enumerate() {
            let stamps = self.timer.as_ref().and_then(|t| t.writes(3, i == 0, false));
            let mut pass = screen_pass(encoder, "bloom down", mip, wgpu::LoadOp::Clear(wgpu::Color::BLACK), stamps);
            pass.set_pipeline(if i == 0 { &self.bloom_prefilter_pipeline } else { &self.bloom_down_pipeline });
            pass.set_bind_group(0, &t.bloom_groups[i], &[]);
            pass.draw(0..3, 0..1);
        }
        // And back up, each mip blurred and added into the one above
        for i in (1..t.bloom_mips.len()).rev() {
            let stamps = self.timer.as_ref().and_then(|t| t.writes(3, false, i == 1));
            let mut pass = screen_pass(encoder, "bloom up", &t.bloom_mips[i - 1], wgpu::LoadOp::Load, stamps);
            pass.set_pipeline(&self.bloom_up_pipeline);
            pass.set_bind_group(0, &t.bloom_groups[i + 1], &[]);
            pass.draw(0..3, 0..1);
        }
        let stamps = self.timer.as_ref().and_then(|t| t.writes(4, true, false));
        let mut pass = screen_pass(encoder, "post pass", view, wgpu::LoadOp::Clear(wgpu::Color::BLACK), stamps);
        pass.set_pipeline(&self.post_pipeline);
        pass.set_bind_group(0, &t.post_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn encode_capture(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> (wgpu::Buffer, u32) {
        let padded_row = (self.config.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: padded_row as u64 * self.config.height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        (buffer, padded_row)
    }

    fn save_capture(&self, path: &std::path::Path, buffer: &wgpu::Buffer, padded_row: u32) {
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        if self.context.device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
            eprintln!("capture: GPU poll failed");
            return;
        }
        let data = match buffer.slice(..).get_mapped_range() {
            Ok(data) => data,
            Err(e) => {
                eprintln!("capture: failed to map buffer: {e:?}");
                return;
            }
        };
        let (w, h) = (self.config.width as usize, self.config.height as usize);
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for row in data.chunks(padded_row as usize).take(h) {
            for px in row[..w * 4].chunks(4) {
                if bgra {
                    ppm.extend_from_slice(&[px[2], px[1], px[0]]);
                } else {
                    ppm.extend_from_slice(&px[..3]);
                }
            }
        }
        match std::fs::write(path, ppm) {
            Ok(()) => eprintln!("capture: saved {}", path.display()),
            Err(e) => eprintln!("capture: failed to write {}: {e}", path.display()),
        }
    }
}

fn uniform_entry(binding: u32, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

fn create_object_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("objects"),
        size: stride * capacity,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("objects"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
            }),
        }],
    });
    (buffer, bind_group)
}

/// A pass drawing one colour target and nothing else.
fn screen_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &str,
    view: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'a>>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Everything sized to the window: the multisampled world, its resolve, and
/// the post passes' textures, with the bind groups that read them.
struct Targets {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    hdr: wgpu::TextureView,
    half_depth: wgpu::TextureView,
    ao: wgpu::TextureView,
    /// Half resolution and down
    bloom_mips: Vec<wgpu::TextureView>,
    /// Reading the frame, then each bloom mip in turn
    bloom_groups: Vec<wgpu::BindGroup>,
    post_group: wgpu::BindGroup,
    depth_group: wgpu::BindGroup,
    ao_group: wgpu::BindGroup,
}

/// The post passes' bind group layouts, and the HDR format chosen.
struct PostLayouts {
    post: wgpu::BindGroupLayout,
    depth: wgpu::BindGroupLayout,
    ao: wgpu::BindGroupLayout,
    bloom: wgpu::BindGroupLayout,
    hdr_format: wgpu::TextureFormat,
}

impl Targets {
    fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        layouts: &PostLayouts,
        globals: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let texture = |label, (width, height): (u32, u32), format, samples, mips, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let hdr_format = layouts.hdr_format;
        let full = (config.width, config.height);
        let half = (config.width.div_ceil(2), config.height.div_ceil(2));
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let read = attach | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = |t: wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let color = view(texture("msaa color", full, hdr_format, SAMPLE_COUNT, 1, attach));
        let depth = view(texture("depth", full, DEPTH_FORMAT, SAMPLE_COUNT, 1, read));
        let hdr = view(texture("hdr", full, hdr_format, 1, 1, read));
        let half_depth = view(texture("half depth", half, HALF_DEPTH_FORMAT, 1, 1, read));
        let ao = view(texture("ao", half, AO_FORMAT, 1, 1, read));
        let mips = BLOOM_MIPS.min(half.0.min(half.1).max(2).ilog2());
        let bloom = texture("bloom", half, hdr_format, 1, mips, read);
        let bloom_mips: Vec<_> = (0..mips)
            .map(|level| bloom.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() }))
            .collect();
        let bloom_groups = std::iter::once(&hdr)
            .chain(&bloom_mips)
            .map(|source| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bloom"),
                    layout: &layouts.bloom,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                    ],
                })
            })
            .collect();
        let post_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &layouts.post,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&hdr) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&bloom_mips[0]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&ao) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        let depth_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("half depth"),
            layout: &layouts.depth,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&depth) },
            ],
        });
        let ao_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ao"),
            layout: &layouts.ao,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&half_depth) },
            ],
        });
        Self { color, depth, hdr, half_depth, ao, bloom_mips, bloom_groups, post_group, depth_group, ao_group }
    }
}

fn alpha_blend() -> wgpu::BlendState {
    // Matches the GL version: blend color, keep destination alpha
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

struct PipelineDesc<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    shader: &'a wgpu::ShaderModule,
    vertex_buffer: bool,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
    cull: Option<wgpu::Face>,
    blend: Option<wgpu::BlendState>,
}

struct PipelineBuilder<'a> {
    device: &'a wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    /// Whether the pass has a depth buffer
    depth: bool,
}

impl PipelineBuilder<'_> {
    fn build(&self, desc: PipelineDesc) -> wgpu::RenderPipeline {
        let vertex_layout = [Some(vertex_layout())];
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(desc.label),
            layout: Some(desc.layout),
            vertex: wgpu::VertexState {
                module: desc.shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: if desc.vertex_buffer { &vertex_layout } else { &[] },
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: desc.cull,
                ..Default::default()
            },
            depth_stencil: self.depth.then(|| wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(desc.depth_write),
                depth_compare: Some(desc.depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: self.samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: desc.shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: desc.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}
