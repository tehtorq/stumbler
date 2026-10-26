//! Streams the world in square chunks: terrain at several levels of detail,
//! plus per-chunk trees, road surface, props and water nearby. Meshes are
//! built on worker threads; the main thread uploads them and evicts far ones.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::geometry::Vertex;
use crate::gpu::{GpuContext, Mesh};
use crate::terrain::{self, TERRAIN_HALF_SIZE, WATER_LEVEL};
use crate::collisions::Solid;
use crate::{cover, props, road, trees};

pub const CHUNK_SIZE: f32 = 500.0;
/// Cells per chunk side at each level of detail.
const LOD_CELLS: [usize; 4] = [40, 20, 10, 5];
/// Use a level of detail while the chunk is nearer than this.
const LOD_RANGE: [f32; 4] = [900.0, 1800.0, 3400.0, f32::MAX];
/// Nothing is drawn beyond this (fog has swallowed it).
pub const DRAW_DISTANCE: f32 = 4200.0;
/// Trees, roads and props load within this distance.
const DETAIL_RANGE: f32 = 2600.0;
/// Grass, flowers and other small plants load within this distance (they
/// shrink away before the edge; see `COVER_FADE` in the mesh shader).
const COVER_RANGE: f32 = 750.0;
/// Beyond this distance chunks draw their trees in lighter detail.
const TREE_LOD_DISTANCE: f32 = 1500.0;
/// Ground cover is built in tiles this many to a chunk side, so tiles that
/// are out of view or past the fade-out can be skipped.
const COVER_TILES: usize = 4;
/// Tiles wholly beyond this distance have sunk away entirely; don't draw them.
const COVER_DRAW: f32 = 745.0;
/// Loaded meshes are dropped once this much further out than needed.
const EVICT_MARGIN: f32 = 600.0;
const MAX_UPLOADS_PER_FRAME: usize = 12;

/// Water colour in the shallows and out in deep water.
const WATER_SHALLOW: [f32; 3] = [0.18, 0.52, 0.56];
const WATER_DEEP: [f32; 3] = [0.07, 0.22, 0.42];
/// Depth at which water reaches its deep colour (and full waves).
const WATER_DEEP_AT: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkCoord(pub i32, pub i32);

impl ChunkCoord {
    pub fn containing(x: f32, z: f32) -> Self {
        Self(((x + TERRAIN_HALF_SIZE) / CHUNK_SIZE).floor() as i32, ((z + TERRAIN_HALF_SIZE) / CHUNK_SIZE).floor() as i32)
    }

    fn origin(self) -> Vec2 {
        Vec2::new(self.0 as f32 * CHUNK_SIZE - TERRAIN_HALF_SIZE, self.1 as f32 * CHUNK_SIZE - TERRAIN_HALF_SIZE)
    }

    fn centre(self) -> Vec2 {
        self.origin() + Vec2::splat(CHUNK_SIZE * 0.5)
    }

    /// Distance from `p` to the nearest point of this chunk (XZ plane).
    fn distance(self, p: Vec2) -> f32 {
        let lo = self.origin();
        let hi = lo + Vec2::splat(CHUNK_SIZE);
        (p.clamp(lo, hi) - p).length()
    }
}

pub fn chunks_per_side() -> i32 {
    (TERRAIN_HALF_SIZE * 2.0 / CHUNK_SIZE).ceil() as i32
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Job {
    Terrain(ChunkCoord, usize),
    Detail(ChunkCoord),
    Cover(ChunkCoord),
}

enum JobResult {
    Terrain(ChunkCoord, usize, terrain::TerrainChunk),
    Detail(ChunkCoord, DetailData),
    Cover(ChunkCoord, Vec<CoverTileData>),
}

/// Geometry for everything on a chunk except the ground itself.
struct DetailData {
    meshes: Vec<(Vec<Vertex>, Vec<u32>)>,
    foliage: Option<(Vec<Vertex>, Vec<u32>)>,
    foliage_far: Option<(Vec<Vertex>, Vec<u32>)>,
    glow: Option<(Vec<Vertex>, Vec<u32>)>,
    water: Option<(Vec<Vertex>, Vec<u32>)>,
    tree_positions: Vec<[f32; 2]>,
    solids: Vec<Solid>,
    min_y: f32,
    max_y: f32,
}

fn run_job(job: Job) -> JobResult {
    match job {
        Job::Terrain(c, lod) => {
            let o = c.origin();
            JobResult::Terrain(c, lod, terrain::generate_terrain_chunk(o.x, o.y, CHUNK_SIZE, LOD_CELLS[lod]))
        }
        Job::Detail(c) => JobResult::Detail(c, build_detail(c)),
        Job::Cover(c) => {
            let step = CHUNK_SIZE / COVER_TILES as f32;
            let tiles = (0..COVER_TILES * COVER_TILES)
                .filter_map(|k| {
                    let lo = c.origin() + Vec2::new((k % COVER_TILES) as f32, (k / COVER_TILES) as f32) * step;
                    let (verts, idxs) = cover::generate(lo, lo + Vec2::splat(step));
                    let (min_y, max_y) = verts.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.pos[1]), b.max(v.pos[1])));
                    // Plants lean a little past their cell
                    (!verts.is_empty()).then_some(CoverTileData { lo: lo - Vec2::splat(2.0), hi: lo + Vec2::splat(step + 2.0), min_y, max_y, verts, idxs })
                })
                .collect();
            JobResult::Cover(c, tiles)
        }
    }
}

fn build_detail(c: ChunkCoord) -> DetailData {
    let lo = c.origin();
    let hi = lo + Vec2::splat(CHUNK_SIZE);
    let mut meshes = Vec::new();

    let trees = trees::generate_trees(lo.x, lo.y, hi.x, hi.y);
    let tree_positions = trees.trunks;
    let foliage = (!trees.near.0.is_empty()).then_some(trees.near);
    let foliage_far = (!trees.far.0.is_empty()).then_some(trees.far);
    if let Some(network) = road::network() {
        meshes.push(network.build_mesh(lo, hi, terrain::terrain_height));
    }
    let props = props::generate_props(lo, hi);
    let solids = props.solids;
    meshes.push(props.mesh);
    let glow = (!props.glow.0.is_empty()).then_some(props.glow);
    meshes.retain(|(v, _)| !v.is_empty());

    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for (v, _) in meshes.iter().chain(&foliage).chain(&glow) {
        for vert in v {
            min_y = min_y.min(vert.pos[1]);
            max_y = max_y.max(vert.pos[1]);
        }
    }

    let water = build_water(lo);
    if water.is_some() {
        min_y = min_y.min(WATER_LEVEL - 1.0);
        max_y = max_y.max(WATER_LEVEL + 1.5);
    }

    DetailData { meshes, foliage, foliage_far, glow, water, tree_positions, solids, min_y, max_y }
}

/// Water surface over the wet cells of the chunk at `lo`, on the finest
/// terrain lattice. Vertex alpha is how deep the water is (0 at the shore,
/// 1 from `WATER_DEEP_AT` down), which the shader turns into transparency,
/// foam and wave height.
fn build_water(lo: Vec2) -> Option<(Vec<Vertex>, Vec<u32>)> {
    let cells = LOD_CELLS[0];
    let step = CHUNK_SIZE / cells as f32;
    let row = cells + 1;
    let depth: Vec<f32> = (0..row * row)
        .map(|k| {
            let p = lo + Vec2::new((k % row) as f32, (k / row) as f32) * step;
            WATER_LEVEL - terrain::raw_height(p.x, p.y)
        })
        .collect();
    let mut idxs = Vec::new();
    for iz in 0..cells {
        for ix in 0..cells {
            let tl = iz * row + ix;
            let corners = [tl, tl + 1, tl + row, tl + row + 1];
            // Any corner under water (or nearly), so the shoreline is covered
            if corners.iter().all(|&k| depth[k] < -0.5) {
                continue;
            }
            let [tl, tr, bl, br] = corners.map(|k| k as u32);
            idxs.extend_from_slice(&[tl, tr, br, tl, br, bl]);
        }
    }
    if idxs.is_empty() {
        return None;
    }
    let verts = (0..row * row)
        .map(|k| {
            let p = lo + Vec2::new((k % row) as f32, (k / row) as f32) * step;
            let d = (depth[k] / WATER_DEEP_AT).clamp(0.0, 1.0);
            let c = [0, 1, 2].map(|i| WATER_SHALLOW[i] + (WATER_DEEP[i] - WATER_SHALLOW[i]) * d);
            Vertex { pos: [p.x, WATER_LEVEL + 0.3, p.y], color: [c[0], c[1], c[2], d] }
        })
        .collect();
    Some((verts, idxs))
}

struct CoverTileData {
    lo: Vec2,
    hi: Vec2,
    min_y: f32,
    max_y: f32,
    verts: Vec<Vertex>,
    idxs: Vec<u32>,
}

struct CoverTile {
    lo: Vec2,
    hi: Vec2,
    min_y: f32,
    max_y: f32,
    mesh: Mesh,
}

struct TerrainEntry {
    mesh: Mesh,
    min_y: f32,
    max_y: f32,
}

struct DetailEntry {
    meshes: Vec<Mesh>,
    foliage: Option<Mesh>,
    foliage_far: Option<Mesh>,
    glow: Option<Mesh>,
    water: Option<Mesh>,
    tree_positions: Vec<[f32; 2]>,
    solids: Vec<Solid>,
    min_y: f32,
    max_y: f32,
}

/// Draw lists for one frame, split by pipeline.
#[derive(Default)]
pub struct ChunkDraws<'a> {
    pub terrain: Vec<&'a Mesh>,
    pub opaque: Vec<&'a Mesh>,
    /// Trees and plants (sway weight in vertex alpha).
    pub foliage: Vec<&'a Mesh>,
    /// Grass, flowers and other small plants near the camera.
    pub cover: Vec<&'a Mesh>,
    /// Things that glow after dark.
    pub glow: Vec<&'a Mesh>,
    pub water: Vec<&'a Mesh>,
}

pub struct ChunkManager {
    jobs: Arc<Mutex<Vec<Job>>>,
    wake: Sender<()>,
    results: Receiver<JobResult>,
    in_flight: HashSet<Job>,
    terrain: HashMap<(ChunkCoord, usize), TerrainEntry>,
    detail: HashMap<ChunkCoord, DetailEntry>,
    /// Ground cover per chunk (None where nothing grows).
    cover: HashMap<ChunkCoord, Vec<CoverTile>>,
}

impl ChunkManager {
    /// Spawn workers and build the coarsest terrain for the whole map, plus
    /// full detail around `start`, before returning.
    pub fn new(gpu: &GpuContext, start: Vec2) -> Self {
        let jobs: Arc<Mutex<Vec<Job>>> = Arc::new(Mutex::new(Vec::new()));
        let (result_tx, results) = mpsc::channel();
        let (wake, wake_rx) = mpsc::channel::<()>();
        let wake_rx = Arc::new(Mutex::new(wake_rx));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1));
        for _ in 0..workers {
            let jobs = Arc::clone(&jobs);
            let tx: Sender<JobResult> = result_tx.clone();
            let wake_rx = Arc::clone(&wake_rx);
            std::thread::spawn(move || loop {
                // Take the job at the end of the queue (the nearest; see `update`)
                let job = jobs.lock().unwrap().pop();
                match job {
                    Some(job) => {
                        if tx.send(run_job(job)).is_err() {
                            return;
                        }
                    }
                    None => {
                        if wake_rx.lock().unwrap().recv().is_err() {
                            return;
                        }
                    }
                }
            });
        }

        let mut manager = Self {
            jobs,
            wake,
            results,
            in_flight: HashSet::new(),
            terrain: HashMap::new(),
            detail: HashMap::new(),
            cover: HashMap::new(),
        };

        // Coarse terrain everywhere so there are never holes in the world
        let n = chunks_per_side();
        let coarse = LOD_CELLS.len() - 1;
        let all: Vec<Job> = (0..n).flat_map(|z| (0..n).map(move |x| Job::Terrain(ChunkCoord(x, z), coarse))).collect();
        let mut wanted = manager.wanted_jobs(start);
        wanted.retain(|j| !matches!(j, Job::Terrain(_, l) if *l == coarse));
        let mut initial = all;
        initial.extend(wanted);
        let t0 = std::time::Instant::now();
        let count = initial.len();
        manager.submit(initial);
        while !manager.in_flight.is_empty() {
            if let Ok(result) = manager.results.recv() {
                manager.accept(gpu, result);
            }
        }
        eprintln!("[timing] {count} chunk jobs {:.2}s", t0.elapsed().as_secs_f32());
        manager
    }

    fn submit(&mut self, jobs: Vec<Job>) {
        let mut queue = self.jobs.lock().unwrap();
        for job in jobs {
            if self.in_flight.insert(job) {
                queue.push(job);
            }
        }
        drop(queue);
        for _ in 0..8 {
            let _ = self.wake.send(());
        }
    }

    fn accept(&mut self, gpu: &GpuContext, result: JobResult) {
        match result {
            JobResult::Terrain(c, lod, chunk) => {
                self.in_flight.remove(&Job::Terrain(c, lod));
                let mesh = Mesh::new(gpu, &chunk.verts, &chunk.idxs);
                self.terrain.insert((c, lod), TerrainEntry { mesh, min_y: chunk.min_y, max_y: chunk.max_y });
            }
            JobResult::Detail(c, d) => {
                self.in_flight.remove(&Job::Detail(c));
                let meshes = d.meshes.iter().map(|(v, i)| Mesh::new(gpu, v, i)).collect();
                let foliage = d.foliage.as_ref().map(|(v, i)| Mesh::new(gpu, v, i));
                let foliage_far = d.foliage_far.as_ref().map(|(v, i)| Mesh::new(gpu, v, i));
                let glow = d.glow.as_ref().map(|(v, i)| Mesh::new(gpu, v, i));
                let water = d.water.as_ref().map(|(v, i)| Mesh::new(gpu, v, i));
                self.detail.insert(c, DetailEntry { meshes, foliage, foliage_far, glow, water, tree_positions: d.tree_positions, solids: d.solids, min_y: d.min_y, max_y: d.max_y });
            }
            JobResult::Cover(c, tiles) => {
                self.in_flight.remove(&Job::Cover(c));
                let tiles = tiles.into_iter()
                    .map(|t| CoverTile { lo: t.lo, hi: t.hi, min_y: t.min_y, max_y: t.max_y, mesh: Mesh::new(gpu, &t.verts, &t.idxs) })
                    .collect();
                self.cover.insert(c, tiles);
            }
        }
    }

    fn desired_lod(dist: f32) -> usize {
        LOD_RANGE.iter().position(|&r| dist < r).unwrap_or(LOD_CELLS.len() - 1)
    }

    /// Jobs needed around `p`, nearest first.
    fn wanted_jobs(&self, p: Vec2) -> Vec<Job> {
        let n = chunks_per_side();
        let reach = (DRAW_DISTANCE / CHUNK_SIZE).ceil() as i32 + 1;
        let here = ChunkCoord::containing(p.x, p.y);
        let mut wanted: Vec<(f32, Job)> = Vec::new();
        for z in (here.1 - reach).max(0)..=(here.1 + reach).min(n - 1) {
            for x in (here.0 - reach).max(0)..=(here.0 + reach).min(n - 1) {
                let c = ChunkCoord(x, z);
                let d = c.distance(p);
                if d > DRAW_DISTANCE {
                    continue;
                }
                let lod = Self::desired_lod(d);
                if !self.terrain.contains_key(&(c, lod)) {
                    wanted.push((d, Job::Terrain(c, lod)));
                }
                if d < DETAIL_RANGE && !self.detail.contains_key(&c) {
                    wanted.push((d + 1.0, Job::Detail(c)));
                }
                if d < COVER_RANGE && !self.cover.contains_key(&c) {
                    wanted.push((d + 2.0, Job::Cover(c)));
                }
            }
        }
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        wanted.into_iter().map(|(_, j)| j).collect()
    }

    /// Queue what's needed near the camera, upload finished meshes, drop far ones.
    pub fn update(&mut self, gpu: &GpuContext, camera: Vec3) {
        let p = Vec2::new(camera.x, camera.z);

        // Workers pop from the end, so queue nearest last
        let mut wanted = self.wanted_jobs(p);
        wanted.retain(|j| !self.in_flight.contains(j));
        if !wanted.is_empty() {
            wanted.reverse();
            self.submit(wanted);
        }
        // Re-sort what's still queued so the nearest jobs run first after moving
        {
            let mut queue = self.jobs.lock().unwrap();
            queue.sort_by(|a, b| {
                let d = |j: &Job| match j {
                    Job::Terrain(c, _) | Job::Detail(c) | Job::Cover(c) => c.distance(p),
                };
                d(b).total_cmp(&d(a))
            });
        }

        for _ in 0..MAX_UPLOADS_PER_FRAME {
            match self.results.try_recv() {
                Ok(result) => self.accept(gpu, result),
                Err(_) => break,
            }
        }

        let coarse = LOD_CELLS.len() - 1;
        self.terrain.retain(|&(c, lod), _| {
            if lod == coarse {
                return true;
            }
            let d = c.distance(p);
            let range = LOD_RANGE[lod] + EVICT_MARGIN;
            let finer_range = if lod == 0 { 0.0 } else { LOD_RANGE[lod - 1] - EVICT_MARGIN };
            d < range && d > finer_range.max(0.0) - EVICT_MARGIN * 2.0
        });
        self.detail.retain(|c, _| c.distance(p) < DETAIL_RANGE + EVICT_MARGIN);
        self.cover.retain(|c, _| c.distance(p) < COVER_RANGE + EVICT_MARGIN * 0.5);
    }

    /// Visible meshes for this frame.
    pub fn draws(&self, camera: Vec3, view_proj: Mat4) -> ChunkDraws<'_> {
        let p = Vec2::new(camera.x, camera.z);
        let n = chunks_per_side();
        let reach = (DRAW_DISTANCE / CHUNK_SIZE).ceil() as i32 + 1;
        let here = ChunkCoord::containing(p.x, p.y);
        // Nearest chunks first, so the depth test can reject what they hide
        // (hills, trees) before it is shaded
        let mut order: Vec<(f32, ChunkCoord)> = Vec::new();
        for z in (here.1 - reach).max(0)..=(here.1 + reach).min(n - 1) {
            for x in (here.0 - reach).max(0)..=(here.0 + reach).min(n - 1) {
                let c = ChunkCoord(x, z);
                let d = c.distance(p);
                if d <= DRAW_DISTANCE {
                    order.push((d, c));
                }
            }
        }
        order.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut out = ChunkDraws::default();
        for (d, c) in order {
            // Best loaded level at or coarser than the one we want, else finer
            let want = Self::desired_lod(d);
            let entry = (want..LOD_CELLS.len()).chain((0..want).rev())
                .find_map(|lod| self.terrain.get(&(c, lod)));
            if let Some(t) = entry {
                if visible(view_proj, c, t.min_y, t.max_y) {
                    out.terrain.push(&t.mesh);
                }
            }
            if let Some(detail) = self.detail.get(&c) {
                if visible(view_proj, c, detail.min_y, detail.max_y) {
                    out.opaque.extend(detail.meshes.iter());
                    let trees = if d > TREE_LOD_DISTANCE { &detail.foliage_far } else { &detail.foliage };
                    out.foliage.extend(trees.iter());
                    out.glow.extend(detail.glow.iter());
                    for tile in self.cover.get(&c).into_iter().flatten() {
                        let near = (p.clamp(tile.lo, tile.hi) - p).length();
                        if near < COVER_DRAW && box_visible(view_proj, tile.lo, tile.hi, tile.min_y, tile.max_y) {
                            out.cover.push(&tile.mesh);
                        }
                    }
                    out.water.extend(detail.water.iter());
                }
            }
        }
        // Water is see-through: far to near blends best
        out.water.reverse();
        out
    }

    /// Solid scenery within the chunks around (x, z).
    pub fn solids_near(&self, x: f32, z: f32) -> impl Iterator<Item = &Solid> {
        let c = ChunkCoord::containing(x, z);
        (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| ChunkCoord(c.0 + dx, c.1 + dz)))
            .filter_map(|c| self.detail.get(&c))
            .flat_map(|d| d.solids.iter())
    }

    /// Tree trunks within the chunks around (x, z).
    pub fn trees_near(&self, x: f32, z: f32) -> impl Iterator<Item = &[f32; 2]> {
        let c = ChunkCoord::containing(x, z);
        (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| ChunkCoord(c.0 + dx, c.1 + dz)))
            .filter_map(|c| self.detail.get(&c))
            .flat_map(|d| d.tree_positions.iter())
    }
}

/// Is any part of the chunk's bounding box inside the view frustum?
fn visible(view_proj: Mat4, c: ChunkCoord, min_y: f32, max_y: f32) -> bool {
    box_visible(view_proj, c.origin(), c.origin() + Vec2::splat(CHUNK_SIZE), min_y, max_y)
}

/// Is any part of the box [lo, hi] x [min_y, max_y] inside the view frustum?
fn box_visible(view_proj: Mat4, lo: Vec2, hi: Vec2, min_y: f32, max_y: f32) -> bool {
    if min_y > max_y {
        return false;
    }
    let corners: [Vec4; 8] = std::array::from_fn(|k| {
        let x = if k & 1 == 0 { lo.x } else { hi.x };
        let y = if k & 2 == 0 { min_y } else { max_y };
        let z = if k & 4 == 0 { lo.y } else { hi.y };
        view_proj * Vec4::new(x, y, z, 1.0)
    });
    // Outside if every corner is beyond the same clip plane
    let all = |f: &dyn Fn(&Vec4) -> bool| corners.iter().all(f);
    !(all(&|v| v.x < -v.w) || all(&|v| v.x > v.w) || all(&|v| v.y < -v.w) || all(&|v| v.y > v.w) || all(&|v| v.w < 0.0))
}

#[allow(dead_code)]
fn _assert_centre_used(c: ChunkCoord) -> Vec2 {
    c.centre()
}

