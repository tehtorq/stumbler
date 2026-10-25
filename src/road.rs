//! Road network: evenly resampled track paths with smoothed height profiles,
//! a spatial grid for nearest-road queries, and the road surface mesh.
//!
//! Once installed, `terrain::terrain_height` flattens the ground under and
//! around every road, so physics, trees and the road mesh all agree.

use std::sync::OnceLock;

use glam::Vec2;

use crate::geometry::Vertex;
use crate::track::Track;

/// Half the asphalt width.
pub const ROAD_HALF_WIDTH: f32 = 24.0;
/// Half the width of a dirt trail.
pub const TRAIL_HALF_WIDTH: f32 = 7.0;
/// Half the width of the railway's bed (ballast, sleepers and rails).
pub const RAIL_HALF_WIDTH: f32 = 9.0;
/// Distance between the two rails.
pub const RAIL_GAUGE: f32 = 3.6;
/// Kerb strip width, outside the asphalt on corners.
pub const KERB_WIDTH: f32 = 3.0;
/// Flat ground beyond the asphalt edge before the terrain blends back in.
/// Wide enough that the finest terrain grid can sit below the road without
/// the drivable ground and the visible mesh disagreeing.
pub const SHOULDER: f32 = 18.0;
/// Circuits have wider run-off: flat ground beyond the kerbs, so a car that
/// runs wide has room to gather itself rather than tumbling down a bank.
pub const CIRCUIT_SHOULDER: f32 = 32.0;
/// Distance over which the shoulder blends into natural terrain.
pub const BLEND: f32 = 60.0;
/// Influence radius of a road on the terrain.
pub const INFLUENCE: f32 = ROAD_HALF_WIDTH + CIRCUIT_SHOULDER + BLEND;

/// Spacing between resampled path points.
pub const SAMPLE_SPACING: f32 = 4.0;
/// Corners tighter than this radius get kerbs.
const KERB_RADIUS: f32 = 300.0;
const GRID_CELL: f32 = 64.0;

const ASPHALT: [f32; 4] = [0.24, 0.24, 0.27, 1.0];
const ROAD_TARMAC: [f32; 4] = [0.36, 0.33, 0.30, 1.0];
const LINE_WHITE: [f32; 4] = [0.92, 0.92, 0.88, 1.0];
const LINE_YELLOW: [f32; 4] = [0.95, 0.8, 0.25, 1.0];
const KERB_RED: [f32; 4] = [0.85, 0.12, 0.1, 1.0];
const CHECKER_BLACK: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
const DIRT: [f32; 4] = [0.56, 0.43, 0.29, 1.0];
const DIRT_RUT: [f32; 4] = [0.46, 0.34, 0.22, 1.0];
const BALLAST: [f32; 4] = [0.5, 0.47, 0.43, 1.0];
const SLEEPER: [f32; 4] = [0.3, 0.22, 0.16, 1.0];
const RAIL_SIDE: [f32; 4] = [0.36, 0.3, 0.27, 1.0];
const RAIL_TOP: [f32; 4] = [0.78, 0.78, 0.8, 1.0];
/// Trails give less grip than tarmac, and kick up dust.
const TRAIL_GRIP: f32 = 0.45;
/// Loose ballast and sleepers give a little more than a trail.
const RAIL_GRIP: f32 = 0.55;

/// What a path is. Where paths overlap, the later kind wins (a road
/// crossing the railway is level with it, and its surface runs over the bed).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum PathKind {
    Trail,
    Rail,
    Road,
    Circuit,
}

impl PathKind {
    pub fn half_width(self) -> f32 {
        match self {
            PathKind::Trail => TRAIL_HALF_WIDTH,
            PathKind::Rail => RAIL_HALF_WIDTH,
            PathKind::Road | PathKind::Circuit => ROAD_HALF_WIDTH,
        }
    }

    /// Flat ground beyond the surface before the terrain blends back in.
    pub fn shoulder(self) -> f32 {
        if self == PathKind::Circuit { CIRCUIT_SHOULDER } else { SHOULDER }
    }
}

/// What carries a path at each point. Only the railway leaves the ground.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Structure {
    /// On the terrain, which is flattened to meet it.
    Ground,
    /// High over a valley on a stone viaduct; the ground below is left be.
    Viaduct,
    /// Through a hill; the hill is left be.
    Tunnel,
}

pub struct RoadPath {
    pub points: Vec<Vec2>,
    pub heights: Vec<f32>,
    pub structure: Vec<Structure>,
    /// Unsigned curvature (1/radius) at each point.
    pub curvature: Vec<f32>,
    pub closed: bool,
    pub kind: PathKind,
}

impl RoadPath {
    fn len(&self) -> usize {
        self.points.len()
    }

    pub fn circuit(&self) -> bool {
        self.kind == PathKind::Circuit
    }

    /// `off` (across, + toward the tangent's left-hand normal) pulled in on
    /// the inside of a tight bend so the strip can't fold over itself.
    fn fold_safe(&self, i: usize, off: f32) -> f32 {
        let n = self.len();
        let (a, b) = if self.closed { ((i + n - 2) % n, (i + 2) % n) } else { (i.saturating_sub(2), (i + 2).min(n - 1)) };
        let turn = self.tangent(a).perp_dot(self.tangent(b));
        if turn * off <= 0.0 || self.curvature[i] <= 0.0 {
            return off;
        }
        let limit = 0.85 / self.curvature[i];
        off.clamp(-limit, limit)
    }

    /// Unit normal at point `i` (the tangent's left-hand side), from the
    /// tangents a couple of samples either way, so strips laid out from it
    /// don't zig-zag on tight bends.
    pub fn smooth_side(&self, i: usize) -> Vec2 {
        let n = self.len();
        let mut t = Vec2::ZERO;
        for o in -2_i32..=2 {
            let k = if self.closed { (i as i32 + o).rem_euclid(n as i32) as usize } else { (i as i32 + o).clamp(0, n as i32 - 1) as usize };
            t += self.tangent(k);
        }
        let t = t.normalize_or_zero();
        Vec2::new(-t.y, t.x)
    }

    /// Unit tangent at point `i`.
    pub fn tangent(&self, i: usize) -> Vec2 {
        let n = self.len();
        let (prev, next) = if self.closed {
            ((i + n - 1) % n, (i + 1) % n)
        } else {
            (i.saturating_sub(1), (i + 1).min(n - 1))
        };
        (self.points[next] - self.points[prev]).normalize_or_zero()
    }
}

struct Segment {
    a: Vec2,
    b: Vec2,
    ha: f32,
    hb: f32,
    kind: PathKind,
    /// On a viaduct: things keep clear of it, but the terrain ignores it.
    raised: bool,
}

#[derive(Clone, Copy)]
pub struct RoadSample {
    /// Distance from the road centreline.
    pub dist: f32,
    /// Road surface height at the nearest centreline point.
    pub height: f32,
    pub kind: PathKind,
    /// Up on a viaduct.
    pub raised: bool,
}

impl RoadSample {
    pub fn half_width(&self) -> f32 {
        self.kind.half_width()
    }

    pub fn shoulder(&self) -> f32 {
        self.kind.shoulder()
    }
}

pub struct RoadNetwork {
    pub paths: Vec<RoadPath>,
    segments: Vec<Segment>,
    grid: Vec<Vec<u32>>,
    grid_dim: i32,
    grid_half: f32,
}

static NETWORK: OnceLock<RoadNetwork> = OnceLock::new();

/// Make the global road network used by terrain queries, building it on
/// first use. Later calls return the existing network.
pub fn install(build: impl FnOnce() -> RoadNetwork) -> &'static RoadNetwork {
    NETWORK.get_or_init(build)
}

pub fn network() -> Option<&'static RoadNetwork> {
    NETWORK.get()
}

/// Whether (x, z) is on tarmac: a road or circuit's surface or kerb.
pub fn paved(x: f32, z: f32) -> bool {
    network().and_then(|n| n.nearest_ground(x, z))
        .is_some_and(|s| s.kind >= PathKind::Road && s.dist < s.half_width() + KERB_WIDTH)
}

/// 1.0 on asphalt (or kerb), less on a trail, fading to 0.0 across the shoulder.
pub fn on_road(x: f32, z: f32) -> f32 {
    match network().and_then(|n| n.nearest_ground(x, z)) {
        Some(s) => {
            let surface = match s.kind {
                PathKind::Trail => TRAIL_GRIP,
                PathKind::Rail => RAIL_GRIP,
                _ => 1.0,
            };
            surface * (1.0 - ((s.dist - s.half_width() - KERB_WIDTH) / SHOULDER).clamp(0.0, 1.0))
        }
        None => 0.0,
    }
}

impl RoadNetwork {
    /// Build from circuits and connecting roads. `natural_height` is the terrain
    /// without road flattening. Paths outside `bounds` (half-size) are skipped.
    pub fn build(
        circuits: &[Track],
        roads: &[Track],
        bounds: f32,
        natural_height: impl Fn(f32, f32) -> f32,
    ) -> Self {
        let limit = bounds - INFLUENCE;
        let inside = |t: &Track| t.points_2d.iter().all(|p| p.x.abs() < limit && p.y.abs() < limit);

        let mut paths: Vec<RoadPath> = circuits
            .iter()
            .map(|t| (t, PathKind::Circuit))
            .chain(roads.iter().map(|t| (t, PathKind::Road)))
            .filter(|(t, _)| t.points_2d.len() >= 2 && inside(t))
            .map(|(t, kind)| build_path(&t.points_2d, t.closed, kind, &natural_height))
            .collect();
        blend_junctions(&mut paths);

        let mut network = Self { paths, segments: Vec::new(), grid: Vec::new(), grid_dim: 0, grid_half: bounds };
        network.index();
        network
    }

    /// Add dirt trails, each starting on a road, a circuit or an earlier
    /// trail. Their heights ease into whatever they leave so the join has no step.
    pub fn add_trails(&mut self, trails: &[Track], natural_height: impl Fn(f32, f32) -> f32) {
        let mut added: Vec<RoadPath> = Vec::new();
        for track in trails.iter().filter(|t| t.points_2d.len() >= 2) {
            let mut path = build_path(&track.points_2d, track.closed, PathKind::Trail, &natural_height);
            // Where it starts: the nearest point on a road or circuit, or on an earlier trail
            let start = path.points[0];
            let host = self.nearest_matching(start.x, start.y, |k| k > PathKind::Trail)
                .map(|s| (s.dist, s.height, s.half_width()))
                .into_iter()
                .chain(added.iter().flat_map(|t| t.points.iter().zip(&t.heights))
                    .map(|(q, &h)| (q.distance(start), h, TRAIL_HALF_WIDTH)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, host_height, host_half_width)) = host {
                let blend_start = host_half_width + KERB_WIDTH;
                let blend_end = blend_start + BLEND;
                let mut along = 0.0;
                for i in 0..path.len() {
                    if i > 0 {
                        along += path.points[i].distance(path.points[i - 1]);
                    }
                    let t = ((along - blend_start) / (blend_end - blend_start)).clamp(0.0, 1.0);
                    let w = 1.0 - t * t * (3.0 - 2.0 * t);
                    path.heights[i] += (host_height - path.heights[i]) * w;
                }
            }
            // Where it crosses the railway, it rises or dips to meet the bed
            let reach = RAIL_HALF_WIDTH + SHOULDER;
            for i in 0..path.len() {
                let p = path.points[i];
                if let Some(rail) = self.nearest_matching(p.x, p.y, |k| k == PathKind::Rail).filter(|r| !r.raised) {
                    let t = ((rail.dist - reach) / 120.0).clamp(0.0, 1.0);
                    let w = 1.0 - t * t * (3.0 - 2.0 * t);
                    path.heights[i] += (rail.height - path.heights[i]) * w;
                }
            }
            added.push(path);
        }
        self.paths.extend(added);
        self.index();
    }

    /// Add railway lines (built like any path, but far smoother and gentler).
    /// Where one crosses a road the two meet at one height, each easing in to
    /// it, so the crossing is level; where a line's end runs onto another
    /// line (a junction), its heights ease into that line's.
    pub fn add_rail(&mut self, lines: &[Track], natural_height: impl Fn(f32, f32) -> f32) {
        let mut rails: Vec<RoadPath> = lines.iter()
            .filter(|t| t.points_2d.len() >= 2)
            .map(|t| build_path(&t.points_2d, t.closed, PathKind::Rail, &natural_height))
            .collect();

        // Level crossings: the closest rail sample to each road it crosses.
        // Each eases in over a stretch long enough to keep to its own grade
        // (a smoothstep is at most 1.5 times its average steepness)
        const FLAT: f32 = ROAD_HALF_WIDTH + SHOULDER;
        let ease = |rise: f32, grade: f32, least: f32| (rise.abs() * 1.5 / grade).max(least);
        let mut pinned: Vec<Vec<usize>> = vec![Vec::new(); rails.len()];
        for (rail, pins) in rails.iter_mut().zip(&mut pinned) {
            let n = rail.len();
            let mut crossings: Vec<(usize, usize, usize)> = Vec::new();
            for (r, road) in self.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
                let lo = road.points.iter().fold(Vec2::MAX, |a, b| a.min(*b)) - 40.0;
                let hi = road.points.iter().fold(Vec2::MIN, |a, b| a.max(*b)) + 40.0;
                let mut best: Option<(f32, usize, usize)> = None;
                for i in 0..n {
                    let p = rail.points[i];
                    if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y {
                        continue;
                    }
                    // Nearest road sample, coarse then fine
                    let Some((k, d)) = road.points.iter().enumerate().step_by(4)
                        .map(|(k, q)| (k, q.distance(p)))
                        .min_by(|a, b| a.1.total_cmp(&b.1)) else { continue };
                    if d > 40.0 {
                        if let Some(b) = best.take() {
                            crossings.push((b.1, r, b.2));
                        }
                        continue;
                    }
                    let lo = k.saturating_sub(4);
                    let hi = (k + 4).min(road.len() - 1);
                    let (k, d) = (lo..=hi).map(|k| (k, road.points[k].distance(p))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, i, k));
                    }
                }
                if let Some(b) = best {
                    crossings.push((b.1, r, b.2));
                }
            }
            for (i, r, k) in crossings {
                // Meet halfway: the road comes down (or up) to it, the rail the rest
                let target = (rail.heights[i] + self.paths[r].heights[k]) * 0.5;
                let rise = target - rail.heights[i];
                ease_toward(&mut self.paths[r], k, target, FLAT, ease(rise, 0.08, 220.0));
                ease_toward(rail, i, target, FLAT, ease(rise, 0.02, 400.0));
                pins.push(i);
            }
        }

        // Junctions: an open line's ends ease into the line they join
        for a in 0..rails.len() {
            if rails[a].closed {
                continue;
            }
            for end in [0, rails[a].len() - 1] {
                let p = rails[a].points[end];
                let host = rails.iter().enumerate()
                    .filter(|&(b, _)| b != a)
                    .flat_map(|(_, r)| r.points.iter().zip(&r.heights))
                    .min_by(|x, y| x.0.distance_squared(p).total_cmp(&y.0.distance_squared(p)))
                    .map(|(_, &h)| h);
                if let Some(h) = host {
                    let rise = h - rails[a].heights[end];
                    ease_toward(&mut rails[a], end, h, 400.0, ease(rise, 0.02, 400.0));
                    pinned[a].push(end);
                }
            }
            // Where the two beds overlap (leaving and rejoining), it takes
            // the other line's heights outright, letting go as they part
            let len = rails[a].len();
            let reach = (1200.0 / SAMPLE_SPACING) as usize;
            for i in (0..reach.min(len)).chain(len.saturating_sub(reach)..len) {
                let p = rails[a].points[i];
                let host = rails.iter().enumerate()
                    .filter(|&(b, _)| b != a)
                    .flat_map(|(_, r)| r.points.iter().zip(&r.heights))
                    .map(|(q, &h)| (q.distance(p), h))
                    .min_by(|x, y| x.0.total_cmp(&y.0));
                if let Some((d, h)) = host {
                    let t = ((d - RAIL_HALF_WIDTH * 2.0) / 60.0).clamp(0.0, 1.0);
                    let w = 1.0 - t * t * (3.0 - 2.0 * t);
                    rails[a].heights[i] += (h - rails[a].heights[i]) * w;
                    if w > 0.999 {
                        pinned[a].push(i);
                    }
                }
            }
        }
        // Eases that overlap can add up: hold the crossings and junctions
        // where they are and limit the grade again between them
        for (rail, pins) in rails.iter_mut().zip(&pinned) {
            limit_grade(rail, SAMPLE_SPACING * 0.03, pins);
            classify_structures(rail, &natural_height);
        }

        self.paths.extend(rails);
        self.index();
    }

    /// Rebuild the segment list and spatial grid from `paths`.
    fn index(&mut self) {
        let bounds = self.grid_half;
        let paths = &self.paths;
        let mut segments = Vec::new();
        for path in paths {
            let n = path.len();
            let count = if path.closed { n } else { n - 1 };
            for i in 0..count {
                let j = (i + 1) % n;
                // Tunnels are out of everyone's way
                let ends = [path.structure[i], path.structure[j]];
                if ends.contains(&Structure::Tunnel) {
                    continue;
                }
                segments.push(Segment {
                    a: path.points[i],
                    b: path.points[j],
                    ha: path.heights[i],
                    hb: path.heights[j],
                    kind: path.kind,
                    raised: ends.contains(&Structure::Viaduct),
                });
            }
        }

        let grid_dim = (bounds * 2.0 / GRID_CELL).ceil() as i32;
        let mut grid = vec![Vec::new(); (grid_dim * grid_dim) as usize];
        for (si, seg) in segments.iter().enumerate() {
            let min = seg.a.min(seg.b) - Vec2::splat(INFLUENCE);
            let max = seg.a.max(seg.b) + Vec2::splat(INFLUENCE);
            let to_cell = |v: f32| (((v + bounds) / GRID_CELL).floor() as i32).clamp(0, grid_dim - 1);
            for cz in to_cell(min.y)..=to_cell(max.y) {
                for cx in to_cell(min.x)..=to_cell(max.x) {
                    grid[(cz * grid_dim + cx) as usize].push(si as u32);
                }
            }
        }

        self.segments = segments;
        self.grid = grid;
        self.grid_dim = grid_dim;
    }

    /// Nearest road within `INFLUENCE` of (x, z). Inside a path's surface and
    /// shoulder, a higher-ranked kind always wins (circuit over road over
    /// trail), so junctions can't put a step in it.
    /// Viaducts count: this is for keeping things clear of paths.
    pub fn nearest(&self, x: f32, z: f32) -> Option<RoadSample> {
        self.nearest_of(x, z, true)
    }

    /// Like `nearest`, but only paths the ground meets (not viaducts): this
    /// is the one the terrain is shaped by.
    pub fn nearest_ground(&self, x: f32, z: f32) -> Option<RoadSample> {
        self.nearest_of(x, z, false)
    }

    fn nearest_of(&self, x: f32, z: f32, raised: bool) -> Option<RoadSample> {
        let mut best: [Option<RoadSample>; 4] = [None; 4];
        for (seg, dist, t) in self.segments_near(Vec2::new(x, z)).filter(|(seg, _, _)| raised || !seg.raised) {
            let slot = &mut best[seg.kind as usize];
            if slot.is_none_or(|b| dist < b.dist) {
                *slot = Some(RoadSample { dist, height: seg.ha + (seg.hb - seg.ha) * t, kind: seg.kind, raised: seg.raised });
            }
        }
        if let Some(s) = best.iter().rev().flatten().find(|s| s.dist <= s.half_width() + s.shoulder()) {
            return Some(*s);
        }
        best.into_iter().flatten().min_by(|a, b| a.dist.total_cmp(&b.dist))
    }

    /// Nearest path within `INFLUENCE` whose kind passes `matches`.
    pub fn nearest_matching(&self, x: f32, z: f32, matches: impl Fn(PathKind) -> bool) -> Option<RoadSample> {
        self.segments_near(Vec2::new(x, z))
            .filter(|(seg, _, _)| matches(seg.kind))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(seg, dist, t)| RoadSample { dist, height: seg.ha + (seg.hb - seg.ha) * t, kind: seg.kind, raised: seg.raised })
    }

    /// Segments within `INFLUENCE` of `p`, with the distance to each and the
    /// position along it of the closest point.
    fn segments_near(&self, p: Vec2) -> impl Iterator<Item = (&Segment, f32, f32)> {
        let cx = ((p.x + self.grid_half) / GRID_CELL).floor() as i32;
        let cz = ((p.y + self.grid_half) / GRID_CELL).floor() as i32;
        let cell: &[u32] = if cx < 0 || cz < 0 || cx >= self.grid_dim || cz >= self.grid_dim {
            &[]
        } else {
            &self.grid[(cz * self.grid_dim + cx) as usize]
        };
        cell.iter()
            .map(move |&si| {
                let seg = &self.segments[si as usize];
                let ab = seg.b - seg.a;
                let t = ((p - seg.a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                (seg, p.distance(seg.a + ab * t), t)
            })
            .filter(|&(_, dist, _)| dist <= INFLUENCE)
    }

    /// Road surface mesh for the stretch of road starting inside the rectangle
    /// `min..max`: asphalt, edge lines, kerbs, centre dashes, start lines.
    /// `surface_height` should be the final (flattened) terrain height.
    pub fn build_mesh(&self, min: Vec2, max: Vec2, surface_height: impl Fn(f32, f32) -> f32) -> (Vec<Vertex>, Vec<u32>) {
        let mut mesh = StripBuilder { verts: Vec::new(), idxs: Vec::new(), height: &surface_height, deck: false };
        let inside = |p: Vec2| p.x >= min.x && p.y >= min.y && p.x < max.x && p.y < max.y;

        for path in &self.paths {
            let n = path.len();
            let hw = path.kind.half_width();
            let quads = if path.closed { n } else { n - 1 };
            // Higher-ranked paths sit a hair above lower ones so junctions don't z-fight
            let (base, surface) = match path.kind {
                PathKind::Circuit => (0.04, ASPHALT),
                PathKind::Road => (0.0, ROAD_TARMAC),
                PathKind::Rail => (-0.02, BALLAST),
                PathKind::Trail => (-0.03, DIRT),
            };

            for i in 0..quads {
                let j = (i + 1) % n;
                if !inside(path.points[i]) {
                    continue;
                }
                if path.kind == PathKind::Rail {
                    let ends = [path.structure[i], path.structure[j]];
                    if ends.contains(&Structure::Tunnel) {
                        continue;
                    }
                    // Up on a viaduct the track lies on the deck at its own
                    // height, between low parapets
                    mesh.deck = ends.contains(&Structure::Viaduct);
                    if mesh.deck {
                        const STONE: [f32; 4] = [0.6, 0.56, 0.5, 1.0];
                        const STONE_DARK: [f32; 4] = [0.5, 0.46, 0.41, 1.0];
                        mesh.slab(path, i, j, (0.0, 1.0), (-hw, hw), (-2.6, -0.05), STONE_DARK, STONE, false);
                        mesh.slab(path, i, j, (0.0, 1.0), (-hw - 0.4, -hw + 0.6), (-2.6, 1.3), STONE, STONE, false);
                        mesh.slab(path, i, j, (0.0, 1.0), (hw - 0.6, hw + 0.4), (-2.6, 1.3), STONE, STONE, false);
                        // Underside
                        mesh.quad(path, i, j, -hw - 0.4, hw + 0.4, -2.6, STONE_DARK);
                    }
                    // Ballast and sleepers, except where a road runs over the
                    // line; the rails carry on across it, flush with the tarmac
                    let covered = |k: usize| {
                        let p = path.points[k];
                        self.nearest_matching(p.x, p.y, |kind| kind > PathKind::Rail).is_some_and(|s| s.dist < s.half_width() + KERB_WIDTH)
                    };
                    let crossing = covered(i) || covered(j);
                    if !crossing {
                        mesh.quad(path, i, j, -hw * 0.7, hw * 0.7, base, surface);
                        for t in [1.0 / 6.0, 0.5, 5.0 / 6.0] {
                            mesh.slab(path, i, j, (t - 0.09, t + 0.09), (-3.0, 3.0), (base, base + 0.35), SLEEPER, SLEEPER, true);
                        }
                    }
                    let (y0, y1) = if crossing { (0.05, 0.12) } else { (0.3, 0.75) };
                    for c in [-RAIL_GAUGE * 0.5, RAIL_GAUGE * 0.5] {
                        mesh.slab(path, i, j, (0.0, 1.0), (c - 0.22, c + 0.22), (y0, y1), RAIL_SIDE, RAIL_TOP, false);
                    }
                    mesh.deck = false;
                    continue;
                }
                // Roads and trails stop where they run onto a wider path's
                // surface; their markings stop just short of it
                let mut marked = true;
                if !path.circuit() {
                    // Kerbs, and the run-off a little way beyond
                    let near = |k: usize, margin: f32| {
                        let p = path.points[k];
                        self.nearest_matching(p.x, p.y, |kind| kind > path.kind).is_some_and(|s| s.dist < s.half_width() + KERB_WIDTH + margin)
                    };
                    if near(i, 0.0) && near(j, 0.0) {
                        continue;
                    }
                    marked = !near(i, hw) && !near(j, hw);
                }
                mesh.quad(path, i, j, -hw, hw, base, surface);
                if !marked {
                    continue;
                }

                if path.kind == PathKind::Trail {
                    // Worn wheel ruts
                    for c in [-3.2, 3.2] {
                        mesh.quad(path, i, j, c - 0.8, c + 0.8, base + 0.03, DIRT_RUT);
                    }
                } else if path.circuit() {
                    // Edge lines
                    mesh.quad(path, i, j, -hw + 1.2, -hw + 2.2, base + 0.03, LINE_WHITE);
                    mesh.quad(path, i, j, hw - 2.2, hw - 1.2, base + 0.03, LINE_WHITE);

                    // Kerbs on corners, alternating red/white every 8 units
                    if path.curvature[i] > 1.0 / KERB_RADIUS || path.curvature[j] > 1.0 / KERB_RADIUS {
                        let color = if (i / 2) % 2 == 0 { KERB_RED } else { LINE_WHITE };
                        mesh.quad(path, i, j, -hw - KERB_WIDTH, -hw, base + 0.12, color);
                        mesh.quad(path, i, j, hw, hw + KERB_WIDTH, base + 0.12, color);
                    }
                } else {
                    // Connecting roads: solid edge lines, a dashed centre line
                    // that turns solid on bends where overtaking is unwise
                    mesh.quad(path, i, j, -hw + 1.5, -hw + 2.3, base + 0.03, LINE_WHITE);
                    mesh.quad(path, i, j, hw - 2.3, hw - 1.5, base + 0.03, LINE_WHITE);
                    let bend = path.curvature[i].max(path.curvature[j]) > 1.0 / 250.0;
                    if bend {
                        mesh.quad(path, i, j, -1.1, -0.3, base + 0.03, LINE_YELLOW);
                        mesh.quad(path, i, j, 0.3, 1.1, base + 0.03, LINE_YELLOW);
                    } else if (i / 3) % 2 == 0 {
                        mesh.quad(path, i, j, -0.6, 0.6, base + 0.03, LINE_YELLOW);
                    }
                }
            }

            if path.circuit() && inside(path.points[0]) {
                // Chequered start/finish line across the first few samples
                let squares = 12;
                let w = 2.0 * hw / squares as f32;
                for row in 0..2 {
                    for k in 0..squares {
                        let color = if (k + row) % 2 == 0 { LINE_WHITE } else { CHECKER_BLACK };
                        let l = -hw + k as f32 * w;
                        mesh.quad(path, row, row + 1, l, l + w, base + 0.05, color);
                    }
                }
            }
        }

        (mesh.verts, mesh.idxs)
    }

    /// Spots on long circuit straights for boost pads: (position, direction).
    pub fn straights(&self, spacing: f32) -> Vec<(Vec2, Vec2)> {
        let mut spots = Vec::new();
        let run_needed = (120.0 / SAMPLE_SPACING) as usize;
        for path in self.paths.iter().filter(|p| p.circuit()) {
            let mut run = 0;
            let mut since_last = spacing * 0.5; // first pad half a spacing in
            for i in 0..path.len() {
                since_last += SAMPLE_SPACING;
                if path.curvature[i] < 1.0 / 2000.0 {
                    run += 1;
                } else {
                    run = 0;
                }
                if run >= run_needed && since_last >= spacing {
                    let back = i - run_needed / 2;
                    spots.push((path.points[back], path.tangent(back)));
                    since_last = 0.0;
                    run = 0;
                }
            }
        }
        spots
    }
}

struct StripBuilder<'a, F: Fn(f32, f32) -> f32> {
    verts: Vec<Vertex>,
    idxs: Vec<u32>,
    height: &'a F,
    /// Heights come from the path itself, not the ground (on a viaduct).
    deck: bool,
}

impl<F: Fn(f32, f32) -> f32> StripBuilder<'_, F> {
    /// Quad between cross sections `i` and `j`, from lateral offset `l` to `r`.
    fn quad(&mut self, path: &RoadPath, i: usize, j: usize, l: f32, r: f32, lift: f32, color: [f32; 4]) {
        let base = self.verts.len() as u32;
        for (k, off) in [(i, l), (i, r), (j, r), (j, l)] {
            let side = path.smooth_side(k);
            let p = path.points[k] + side * path.fold_safe(k, off);
            let y = if self.deck { path.heights[k] } else { (self.height)(p.x, p.y) } + lift;
            self.verts.push(Vertex { pos: [p.x, y, p.y], color });
        }
        // Both windings end up visible: the mesh pipeline doesn't cull
        self.idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A box on the surface between cross sections `i` and `j`: from `t.0`
    /// to `t.1` of the way along, `off` across, `y` above the surface. Top
    /// and long sides, plus the end faces if `ends`.
    #[allow(clippy::too_many_arguments)]
    fn slab(&mut self, path: &RoadPath, i: usize, j: usize, t: (f32, f32), off: (f32, f32), y: (f32, f32), side: [f32; 4], top: [f32; 4], ends: bool) {
        let at = |s: f32, o: f32, up: f32| {
            let side = path.smooth_side(i).lerp(path.smooth_side(j), s).normalize_or_zero();
            let o = path.fold_safe(if s < 0.5 { i } else { j }, o);
            let p = path.points[i].lerp(path.points[j], s) + side * o;
            let ground = if self.deck { path.heights[i] + (path.heights[j] - path.heights[i]) * s } else { (self.height)(p.x, p.y) };
            [p.x, ground + up, p.y]
        };
        let corner = |s: f32, o: f32, up: f32| at(s, o, up);
        let mut face = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], color: [f32; 4]| {
            let base = self.verts.len() as u32;
            for pos in [a, b, c, d] {
                self.verts.push(Vertex { pos, color });
            }
            self.idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        };
        let (t0, t1) = t;
        let (l, r) = off;
        let (lo, hi) = y;
        face(corner(t0, l, hi), corner(t0, r, hi), corner(t1, r, hi), corner(t1, l, hi), top);
        face(corner(t0, l, lo), corner(t1, l, lo), corner(t1, l, hi), corner(t0, l, hi), side);
        face(corner(t0, r, lo), corner(t1, r, lo), corner(t1, r, hi), corner(t0, r, hi), side);
        if ends {
            face(corner(t0, l, lo), corner(t0, r, lo), corner(t0, r, hi), corner(t0, l, hi), side);
            face(corner(t1, l, lo), corner(t1, r, lo), corner(t1, r, hi), corner(t1, l, hi), side);
        }
    }
}

fn build_path(
    control: &[Vec2],
    closed: bool,
    kind: PathKind,
    natural_height: &impl Fn(f32, f32) -> f32,
) -> RoadPath {
    let points = resample(control, closed, SAMPLE_SPACING);
    let n = points.len();

    // Height profile: natural terrain along the centreline, smoothed so the road
    // keeps hills and ramps but loses cross-slope and small bumps.
    let mut heights: Vec<f32> = points.iter().map(|p| natural_height(p.x, p.y)).collect();
    // The railway irons out far more: it can't climb much, so it runs on
    // embankments and through cuttings, and on causeways across the lakes
    let (radius, passes) = if kind == PathKind::Rail { (45, 3) } else { (5, 2) }; // samples (±20 units for roads)
    if kind == PathKind::Rail {
        heights.iter_mut().for_each(|h| *h = h.max(crate::terrain::WATER_LEVEL + 3.0));
    }
    for _ in 0..passes {
        heights = (0..n)
            .map(|i| {
                let mut sum = 0.0;
                let mut count = 0.0;
                for o in -radius..=radius {
                    let k = i as i32 + o;
                    let k = if closed {
                        k.rem_euclid(n as i32) as usize
                    } else {
                        k.clamp(0, n as i32 - 1) as usize
                    };
                    sum += heights[k];
                    count += 1.0;
                }
                sum / count
            })
            .collect();
    }

    // Where a circuit crosses itself (Suzuka's figure of eight) there's no
    // bridge, so both passes meet at one height; then re-smooth either side
    if kind == PathKind::Circuit {
        level_crossings(&points, &mut heights, closed);
    }

    // Limit the gradient so mountain sections stay drivable; the terrain
    // blend turns the difference into cuttings and embankments
    let max_rise = SAMPLE_SPACING * match kind {
        PathKind::Circuit => 0.10,
        PathKind::Road => 0.15,
        PathKind::Trail => 0.22,
        PathKind::Rail => 0.025,
    };
    for _ in 0..4 {
        for i in 1..n {
            heights[i] = heights[i].clamp(heights[i - 1] - max_rise, heights[i - 1] + max_rise);
        }
        for i in (0..n - 1).rev() {
            heights[i] = heights[i].clamp(heights[i + 1] - max_rise, heights[i + 1] + max_rise);
        }
        if closed {
            heights[0] = heights[0].clamp(heights[n - 1] - max_rise, heights[n - 1] + max_rise);
        }
    }

    // The limit leaves sharp crests and dips where it cut a slope short;
    // round them into vertical curves so cars don't take off over them
    // (averaging can't make any stretch steeper)
    round_vertical_curves(&mut heights, closed, if kind == PathKind::Trail { 4 } else { 10 });

    let mut path = RoadPath { points, heights, structure: vec![Structure::Ground; n], curvature: vec![0.0; n], closed, kind };
    // Curvature from heading change over a few samples (smooths out noise)
    let span = 3;
    for i in 0..n {
        let (a, b) = if closed {
            ((i + n - span) % n, (i + span) % n)
        } else {
            (i.saturating_sub(span), (i + span).min(n - 1))
        };
        let ta = path.tangent(a);
        let tb = path.tangent(b);
        let angle = ta.perp_dot(tb).atan2(ta.dot(tb)).abs();
        let steps = if closed { 2 * span } else { (b - a).max(1) };
        path.curvature[i] = angle / (SAMPLE_SPACING * steps as f32);
    }
    path
}

/// Three passes of a moving average `radius` samples either side.
fn round_vertical_curves(heights: &mut Vec<f32>, closed: bool, radius: i32) {
    let n = heights.len() as i32;
    for _ in 0..3 {
        let prev = heights.clone();
        for i in 0..n {
            let mut sum = 0.0;
            for o in -radius..=radius {
                let k = if closed { (i + o).rem_euclid(n) } else { (i + o).clamp(0, n - 1) };
                sum += prev[k as usize];
            }
            heights[i as usize] = sum / (2 * radius + 1) as f32;
        }
    }
}

/// Make the passes through any place a closed path crosses itself (or runs
/// so close the surfaces overlap) level with each other: flat at their
/// average height through the overlap, eased back over the approach.
fn level_crossings(points: &[Vec2], heights: &mut [f32], closed: bool) {
    let n = points.len();
    if !closed || n < 200 {
        return;
    }
    let near_along = (300.0 / SAMPLE_SPACING) as usize;
    // Anywhere the two flattened surfaces (road and shoulder) would overlap
    let reach = (ROAD_HALF_WIDTH + CIRCUIT_SHOULDER) * 2.0;
    // Level across the whole overlap, then eased back over the approach
    let flat = (90.0 / SAMPLE_SPACING) as i32;
    let ease = (160.0 / SAMPLE_SPACING) as i32;
    // Closest pair of samples on each crossing
    let mut crossings: Vec<(usize, usize)> = Vec::new();
    // Only compare points in neighbouring cells
    let key = |p: Vec2| ((p.x / reach).floor() as i32, (p.y / reach).floor() as i32);
    let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
    for (i, p) in points.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i);
    }
    for i in 0..n {
        let (kx, kz) = key(points[i]);
        let near: Vec<usize> = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (kx + dx, kz + dz)))
            .flat_map(|k| grid.get(&k).into_iter().flatten().copied())
            .collect();
        for j in near {
            if j < i + near_along || n - (j - i) < near_along || points[i].distance(points[j]) > reach {
                continue;
            }
            let better = |&(a, b): &(usize, usize)| points[a].distance(points[b]) <= points[i].distance(points[j]);
            match crossings.iter_mut().find(|(a, b)| a.abs_diff(i) < near_along && b.abs_diff(j) < near_along) {
                Some(c) if better(c) => {}
                Some(c) => *c = (i, j),
                None => crossings.push((i, j)),
            }
        }
    }
    for (i, j) in crossings {
        let target = (heights[i] + heights[j]) * 0.5;
        for centre in [i, j] {
            // Flat at the shared height through the overlap, blending back
            // to the road's own profile over the approach
            for o in -(flat + ease)..=(flat + ease) {
                let k = (centre as i32 + o).rem_euclid(n as i32) as usize;
                let t = (1.0 - (o.abs() - flat).max(0) as f32 / (ease + 1) as f32).clamp(0.0, 1.0);
                heights[k] += (target - heights[k]) * t * t * (3.0 - 2.0 * t);
            }
        }
    }
}

/// Where the railway runs high above the natural ground it goes onto a
/// viaduct, and where it runs deep below, into a tunnel; short stretches of
/// either are left as embankments and cuttings.
fn classify_structures(path: &mut RoadPath, natural_height: &impl Fn(f32, f32) -> f32) {
    const VIADUCT_ABOVE: f32 = 12.0;
    const TUNNEL_BELOW: f32 = 30.0;
    let n = path.len();
    let gap: Vec<f32> = (0..n).map(|i| path.heights[i] - natural_height(path.points[i].x, path.points[i].y)).collect();
    for (kind, test, shortest) in [
        (Structure::Viaduct, Box::new(|g: f32| g > VIADUCT_ABOVE) as Box<dyn Fn(f32) -> bool>, 15),
        (Structure::Tunnel, Box::new(|g: f32| g < -TUNNEL_BELOW), 40),
    ] {
        let mut flags: Vec<bool> = gap.iter().map(|&g| test(g)).collect();
        // Bridge short gaps, then drop runs too short to bother with
        for (want, longest) in [(false, 25), (true, shortest)] {
            let mut i = 0;
            while i < n {
                if flags[i] != want {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < n && flags[i] == want {
                    i += 1;
                }
                // Runs touching an open path's ends are left alone
                let inner = path.closed || (start > 0 && i < n);
                if inner && i - start < longest {
                    flags[start..i].iter_mut().for_each(|f| *f = !want);
                }
            }
        }
        for i in 0..n {
            if flags[i] && path.structure[i] == Structure::Ground {
                path.structure[i] = kind;
            }
        }
    }
}

/// Keep `path`'s grade within `max_rise` per sample, never moving the
/// samples in `pinned`.
fn limit_grade(path: &mut RoadPath, max_rise: f32, pinned: &[usize]) {
    let n = path.len();
    let free = |i: usize| !pinned.contains(&i);
    for _ in 0..4 {
        for i in 1..n {
            if free(i) {
                path.heights[i] = path.heights[i].clamp(path.heights[i - 1] - max_rise, path.heights[i - 1] + max_rise);
            }
        }
        for i in (0..n - 1).rev() {
            if free(i) {
                path.heights[i] = path.heights[i].clamp(path.heights[i + 1] - max_rise, path.heights[i + 1] + max_rise);
            }
        }
    }
}

/// Bring `path`'s height at sample `at` to `target`: flat for `flat` units
/// either side, easing back to its own profile over a further `ease`.
fn ease_toward(path: &mut RoadPath, at: usize, target: f32, flat: f32, ease: f32) {
    let n = path.len() as i32;
    let (flat, ease) = ((flat / SAMPLE_SPACING) as i32, (ease / SAMPLE_SPACING) as i32);
    for o in -(flat + ease)..=(flat + ease) {
        let k = at as i32 + o;
        let k = if path.closed { k.rem_euclid(n) } else if (0..n).contains(&k) { k } else { continue };
        let t = (1.0 - (o.abs() - flat).max(0) as f32 / (ease + 1) as f32).clamp(0.0, 1.0);
        let k = k as usize;
        path.heights[k] += (target - path.heights[k]) * t * t * (3.0 - 2.0 * t);
    }
}

/// Where a connecting road meets a circuit, ease its height into the
/// circuit's so the two surfaces join without a step.
fn blend_junctions(paths: &mut [RoadPath]) {
    let blend_start = ROAD_HALF_WIDTH + KERB_WIDTH + CIRCUIT_SHOULDER;
    let blend_end = blend_start + 120.0;
    // Circuit points bucketed by cell, so each road point only looks nearby
    let key = |p: Vec2| ((p.x / blend_end).floor() as i32, (p.y / blend_end).floor() as i32);
    let mut grid: std::collections::HashMap<(i32, i32), Vec<(Vec2, f32)>> = Default::default();
    for path in paths.iter().filter(|p| p.circuit()) {
        for (q, &h) in path.points.iter().zip(&path.heights) {
            grid.entry(key(*q)).or_default().push((*q, h));
        }
    }
    for path in paths.iter_mut().filter(|p| p.kind == PathKind::Road) {
        for (i, p) in path.points.iter().enumerate() {
            let (kx, kz) = key(*p);
            let nearest = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (kx + dx, kz + dz)))
                .flat_map(|k| grid.get(&k).into_iter().flatten())
                .map(|&(q, h)| (q.distance(*p), h))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((d, h)) = nearest {
                let t = ((d - blend_start) / (blend_end - blend_start)).clamp(0.0, 1.0);
                let w = 1.0 - t * t * (3.0 - 2.0 * t);
                path.heights[i] += (h - path.heights[i]) * w;
            }
        }
    }
}

/// Round off corners tighter than `min_radius` by repeatedly smoothing just
/// the points around them, keeping the ends of an open path in place.
/// Returns the path resampled every `SAMPLE_SPACING`.
pub fn ease_tight_corners(points: &[Vec2], closed: bool, min_radius: f32) -> Vec<Vec2> {
    const SPAN: usize = 3;
    const REACH: i32 = 20;
    let mut pts = resample(points, closed, SAMPLE_SPACING);
    let n = pts.len();
    if n < 2 * SPAN + 1 {
        return pts;
    }
    // Open paths: only where both neighbours exist
    let (lo, hi) = if closed { (0, n) } else { (SPAN, n - SPAN) };
    for _ in 0..3000 {
        let tight: Vec<usize> = (lo..hi).filter(|&i| corner_radius(&pts, i, SPAN) < min_radius).collect();
        if tight.is_empty() {
            break;
        }
        let mut smooth = vec![false; n];
        for &i in &tight {
            for o in -REACH..=REACH {
                let k = i as i32 + o;
                if closed {
                    smooth[k.rem_euclid(n as i32) as usize] = true;
                } else if (2..n as i32 - 2).contains(&k) {
                    smooth[k as usize] = true;
                }
            }
        }
        let prev = pts.clone();
        for i in (0..n).filter(|&i| smooth[i]) {
            // Near and next-but-one neighbours both, so even a point-to-point
            // zig-zag gets smoothed out
            let (a1, b1) = (prev[(i + n - 1) % n], prev[(i + 1) % n]);
            let (a2, b2) = (prev[(i + n - 2) % n], prev[(i + 2) % n]);
            pts[i] = prev[i] * 0.4 + (a1 + b1) * 0.2 + (a2 + b2) * 0.1;
        }
    }
    resample(&pts, closed, SAMPLE_SPACING)
}

/// Radius of the circle through the points `span` either side of `i`
/// (wrapping round, as for a closed path).
pub fn corner_radius(pts: &[Vec2], i: usize, span: usize) -> f32 {
    let n = pts.len();
    let (a, b, c) = (pts[(i + n - span) % n], pts[i], pts[(i + span) % n]);
    let twice_area = (b - a).perp_dot(c - a).abs();
    if twice_area < 1e-6 {
        return f32::MAX;
    }
    a.distance(b) * b.distance(c) * c.distance(a) / (2.0 * twice_area)
}

/// Resample a polyline at (approximately) even spacing.
pub fn resample(points: &[Vec2], closed: bool, spacing: f32) -> Vec<Vec2> {
    let mut poly = points.to_vec();
    if closed {
        poly.push(points[0]);
    }
    let total: f32 = poly.windows(2).map(|w| w[0].distance(w[1])).sum();
    let count = ((total / spacing).round() as usize).max(2);
    let step = total / count as f32;

    let mut out = Vec::with_capacity(count + 1);
    let mut seg = 0;
    let mut seg_start = 0.0;
    let samples = if closed { count } else { count + 1 };
    for k in 0..samples {
        let d = k as f32 * step;
        while seg < poly.len() - 2 && seg_start + poly[seg].distance(poly[seg + 1]) < d {
            seg_start += poly[seg].distance(poly[seg + 1]);
            seg += 1;
        }
        let len = poly[seg].distance(poly[seg + 1]).max(1e-6);
        let t = ((d - seg_start) / len).clamp(0.0, 1.0);
        out.push(poly[seg].lerp(poly[seg + 1], t));
    }
    out
}
