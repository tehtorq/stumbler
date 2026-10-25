//! wgpu renderer: device/surface setup, meshes, pipelines and frame submission.
//!
//! Bind group layout used by every pipeline:
//! - group 0: per-frame `Globals` (camera, sun, fog, lights, sway, screen size)
//! - group 1 (mesh pipelines): per-draw `Object` model matrix at a dynamic offset
//!
//! Renders with 4x MSAA, resolved into the swapchain image.

use std::path::PathBuf;
use std::sync::Arc;

use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::geometry::Vertex;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLE_COUNT: u32 = 4;
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
}

/// Times the main render pass on the GPU (profiling only). Results are read
/// back a few frames later through a small ring of buffers, so the pipeline
/// never stalls waiting for them.
pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    ring: Vec<TimerSlot>,
    next: usize,
    /// Nanoseconds per timestamp tick.
    period: f32,
    total_ms: f64,
    frames: u32,
}

struct TimerSlot {
    buffer: wgpu::Buffer,
    /// Set by the map callback once the buffer can be read.
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    in_flight: bool,
}

const TIMER_RING: usize = 4;

impl GpuTimer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let buffer = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 16, usage, mapped_at_creation: false });
        Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pass timer"), ty: wgpu::QueryType::Timestamp, count: 2 }),
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
            total_ms: 0.0,
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
                    self.total_ms += t[1].saturating_sub(t[0]) as f64 * self.period as f64 / 1e6;
                    self.frames += 1;
                }
                slot.buffer.unmap();
                slot.in_flight = false;
            }
        }
    }

    /// Average GPU time of the main pass so far, in milliseconds.
    pub fn average_ms(&self) -> f64 {
        self.total_ms / self.frames.max(1) as f64
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
    pub fn new<I: IndexType>(gpu: &Gpu, vertices: &[Vertex], indices: &[I]) -> Self {
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
    pub fn write_vertices(&self, gpu: &Gpu, vertices: &[Vertex]) {
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
    pub fn new(gpu: &Gpu, label: &str) -> Self {
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

    fn buffer(gpu: &Gpu, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn update(&mut self, gpu: &Gpu, vertices: &[Vertex], indices: &[u16]) {
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

pub struct Gpu {
    pub window: Arc<Window>,
    /// Triangles submitted in the last frame (reported by screenshot runs).
    pub last_triangles: u64,
    /// Draw calls in the last frame.
    pub last_draw_calls: u32,
    /// GPU timing when profiling.
    pub timer: Option<GpuTimer>,
    surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,

    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,

    object_layout: wgpu::BindGroupLayout,
    object_buffer: wgpu::Buffer,
    object_bind_group: wgpu::BindGroup,
    object_capacity: u64,
    object_stride: u64,

    sky_pipeline: wgpu::RenderPipeline,
    terrain_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    particle_pipeline: wgpu::RenderPipeline,
    hud_pipeline: wgpu::RenderPipeline,

    /// Dev aid: save the next frame to this path (binary PPM).
    pub capture_path: Option<PathBuf>,
}

impl Gpu {
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
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                required_features: if profile { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() },
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
        let (color_view, depth_view) = create_render_targets(&device, &config);

        // --- Group 0: globals ---
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[uniform_entry(0, false)],
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
        let mesh_shader = shader("mesh", include_str!("shaders/mesh.wgsl"));
        let terrain_shader = shader("terrain", include_str!("shaders/terrain.wgsl"));
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

        let builder = PipelineBuilder { device: &device, format: config.format };
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
        let hud_pipeline = builder.build(PipelineDesc {
            label: "hud",
            layout: &globals_only,
            shader: &hud_shader,
            vertex_buffer: true,
            depth_write: false,
            depth_compare: wgpu::CompareFunction::Always,
            cull: None,
            blend: Some(alpha_blend()),
        });

        Self {
            window,
            surface,
            device,
            queue,
            config,
            color_view,
            depth_view,
            globals_buffer,
            globals_bind_group,
            object_layout,
            object_buffer,
            object_bind_group,
            object_capacity,
            object_stride,
            sky_pipeline,
            terrain_pipeline,
            mesh_pipeline,
            particle_pipeline,
            hud_pipeline,
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
        self.surface.configure(&self.device, &self.config);
        (self.color_view, self.depth_view) = create_render_targets(&self.device, &self.config);
    }

    /// Framebuffer size in pixels.
    pub fn size(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    /// Render one frame: sky, then `draws` in order.
    pub fn render(&mut self, globals: &Globals, draws: &[Draw]) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Upload per-frame and per-object uniforms
        self.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(globals));
        if draws.len() as u64 > self.object_capacity {
            self.object_capacity = (draws.len() as u64).next_power_of_two();
            (self.object_buffer, self.object_bind_group) =
                create_object_buffer(&self.device, &self.object_layout, self.object_stride, self.object_capacity);
        }
        let mut objects = vec![0u8; draws.len() * self.object_stride as usize];
        for (i, draw) in draws.iter().enumerate() {
            let offset = i * self.object_stride as usize;
            let object = ObjectUniform { model: draw.model, params: [draw.kind as u32 as f32, 0.0, 0.0, 0.0] };
            let size = std::mem::size_of::<ObjectUniform>();
            objects[offset..offset + size].copy_from_slice(bytemuck::bytes_of(&object));
        }
        if !objects.is_empty() {
            self.queue.write_buffer(&self.object_buffer, 0, &objects);
        }

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_view,
                    depth_slice: None,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(CLEAR_COLOR), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().map(|t| wgpu::RenderPassTimestampWrites {
                    query_set: &t.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_bind_group(0, &self.globals_bind_group, &[]);

            let mut current = None;
            let mut sky_drawn = false;
            self.last_triangles = 0;
            self.last_draw_calls = 0;
            for (i, draw) in draws.iter().enumerate() {
                if draw.mesh.index_count == 0 {
                    continue;
                }
                // The sky goes in once the opaque world is down (it only fills
                // uncovered pixels) and before anything see-through
                let see_through = draw.kind == MeshKind::Water || matches!(draw.pipeline, PipelineKind::Particles | PipelineKind::Hud);
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
                        PipelineKind::Hud => &self.hud_pipeline,
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
        if let Some(t) = &self.timer {
            if let Some(readback) = t.slot() {
                encoder.resolve_query_set(&t.queries, 0..2, &t.resolve, 0);
                encoder.copy_buffer_to_buffer(&t.resolve, 0, readback, 0, 16);
            }
        }
        let capture = self.capture_path.take().map(|path| (path, self.encode_capture(&mut encoder, &frame.texture)));
        self.queue.submit([encoder.finish()]);
        if let Some((path, (buffer, padded_row))) = capture {
            self.save_capture(&path, &buffer, padded_row);
        }
        if let Some(t) = &mut self.timer {
            t.after_submit(&self.device);
        }
        // Tells winit to pace redraws by the compositor's frame callbacks
        // (Wayland), which would cap the frame rate; only wanted with vsync
        if self.config.present_mode == wgpu::PresentMode::Fifo {
            self.window.pre_present_notify();
        }
        self.queue.present(frame);
    }
}

impl Gpu {
    fn encode_capture(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> (wgpu::Buffer, u32) {
        let padded_row = (self.config.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
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
        if self.device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
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

/// Multisampled colour and depth targets matching the surface size.
fn create_render_targets(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> (wgpu::TextureView, wgpu::TextureView) {
    let target = |label, format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: SAMPLE_COUNT,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    };
    (target("msaa color", config.format), target("depth", DEPTH_FORMAT))
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
}

impl PipelineBuilder<'_> {
    fn build(&self, desc: PipelineDesc) -> wgpu::RenderPipeline {
        const ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let vertex_layout = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        })];
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(desc.depth_write),
                depth_compare: Some(desc.depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLE_COUNT, ..Default::default() },
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
