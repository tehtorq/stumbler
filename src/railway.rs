//! The railway: a line right round the edge of the map, and a second running
//! straight through the middle of it between the circuits, branching off the
//! loop in the south and rejoining it in the north. One steam train with four
//! coaches makes its slow way round: the whole loop, then up through the
//! middle, then home round the far side, and again.
//!
//! The lines are routed over the terrain (round the circuits, along the
//! gentlest ground, across the lakes only when going round is far longer),
//! then built into the road network as `PathKind::Rail`, which gives them
//! cuttings, embankments and level crossings with the roads.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::sync::OnceLock;

use glam::{Mat4, Quat, Vec2, Vec3, vec3};

use crate::collisions::Solid;
use crate::geometry::Vertex;
use crate::props::Builder;
use crate::road::{self, corner_radius, resample, PathKind, RoadNetwork, RoadPath, Structure, RAIL_HALF_WIDTH, ROAD_HALF_WIDTH};
use crate::terrain::{self, WATER_LEVEL};
use crate::track::Track;

/// Routing grid spacing.
const CELL: f32 = 50.0;
/// Lines keep inside this half-size.
const EDGE: f32 = 9150.0;
/// Where the loop runs, as a half-size: just outside the outermost circuits.
const LOOP: f32 = 8850.0;
/// The line through the middle runs north–south along this x, in the gap
/// between two columns of circuits.
const THROUGH_X: f32 = 1890.0;
/// Lines stay this far from circuit centrelines.
const CIRCUIT_CLEARANCE: f32 = 220.0;
/// Tightest curve on the railway.
const MIN_RADIUS: f32 = 350.0;
/// How far the line through the middle shares the loop's alignment at each
/// end before it peels away (in loop samples).
const JUNCTION_RUN: usize = 150;

/// Train speed in units per second: an unhurried 40-odd km/h.
pub const TRAIN_SPEED: f32 = 20.0;
/// Height of the rail tops above the bed.
const RAIL_TOP: f32 = 0.75;
/// Gap between buffers.
const COUPLING: f32 = 1.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CarKind {
    Locomotive,
    Tender,
    Coach,
}

impl CarKind {
    fn length(self) -> f32 {
        match self {
            CarKind::Locomotive => 23.0,
            CarKind::Tender => 12.0,
            CarKind::Coach => 27.0,
        }
    }
}

/// Front to back.
const CONSIST: [CarKind; 6] = [CarKind::Locomotive, CarKind::Tender, CarKind::Coach, CarKind::Coach, CarKind::Coach, CarKind::Coach];
/// Half the width of the train, for collisions.
const TRAIN_HALF_WIDTH: f32 = 3.2;

/// A level crossing: where the railway crosses a road.
pub struct Crossing {
    /// On the railway's centreline, at the middle of the road.
    pub pos: Vec3,
    pub road_dir: Vec2,
    /// How far along the train's tour the crossing comes.
    pub at: f32,
}

impl Crossing {
    /// The two barriers: (pivot at the top of the post, direction the arm
    /// reaches when lowered, direction the traffic it stops comes from).
    /// One each side of the line, each closing the lane that leads onto it.
    pub fn barriers(&self) -> [(Vec3, Vec3, Vec3); 2] {
        let across = Vec2::new(-self.road_dir.y, self.road_dir.x);
        [1.0, -1.0].map(|s| {
            let post = self.pos.truncate_xz() + self.road_dir * s * 17.0 + across * s * (ROAD_HALF_WIDTH + 2.5);
            let y = terrain::terrain_height(post.x, post.y);
            (vec3(post.x, y + 2.6, post.y), vec3(-across.x * s, 0.0, -across.y * s), vec3(self.road_dir.x * s, 0.0, self.road_dir.y * s))
        })
    }
}

trait XZ {
    fn truncate_xz(self) -> Vec2;
}

impl XZ for Vec3 {
    fn truncate_xz(self) -> Vec2 {
        Vec2::new(self.x, self.z)
    }
}

pub struct Railway {
    /// The train's route as one closed loop (bed height), evenly spaced.
    pub tour: Vec<Vec3>,
    /// Distance along the tour to each point.
    distances: Vec<f32>,
    /// Whether each tour point is in a tunnel.
    tunnel: Vec<bool>,
    pub length: f32,
    pub crossings: Vec<Crossing>,
    /// Where the train is at the start: coming up through the middle,
    /// a little south of the home circuit.
    pub start: f32,
    /// Both lines, for maps.
    pub outlines: Vec<Vec<Vec2>>,
}

static RAILWAY: OnceLock<Railway> = OnceLock::new();

pub fn get() -> Option<&'static Railway> {
    RAILWAY.get()
}

// ── Routing ──

/// The loop round the edge and the line through the middle, as tracks for
/// `RoadNetwork::add_rail`. `network` should hold the circuits and roads.
pub fn route(network: &RoadNetwork) -> Vec<Track> {
    let blocked = circuit_cells(network);
    let l = LOOP;
    let c = LOOP * 0.75;
    let (a, b) = (LOOP / 4.0, LOOP * 3.0 / 4.0);
    // Anticlockwise seen from above with north (+z) up: east along the south
    // edge, north up the east side, and so on; the corners are cut
    let waypoints = [
        Vec2::new(-b, -l), Vec2::new(-a, -l), Vec2::new(a, -l), Vec2::new(b, -l),
        Vec2::new(l, -c), Vec2::new(l, -a), Vec2::new(l, a), Vec2::new(l, c),
        Vec2::new(b, l), Vec2::new(a, l), Vec2::new(-a, l), Vec2::new(-b, l),
        Vec2::new(-l, c), Vec2::new(-l, a), Vec2::new(-l, -a), Vec2::new(-l, -c),
    ];
    // Each leg is routed on its own thread
    let legs = crate::layout::parallel_map((0..waypoints.len()).collect(), |k| astar(waypoints[k], waypoints[(k + 1) % waypoints.len()], &blocked));
    let mut raw = Vec::new();
    for (k, leg) in legs.into_iter().enumerate() {
        raw.extend(leg.into_iter().skip(usize::from(k > 0)));
    }
    raw.pop(); // closes on itself
    let ring = smooth(&raw, true);

    // The line through the middle leaves the loop heading east along the
    // south edge, turns north, and rejoins it heading west along the north
    let n = ring.len();
    let nearest = |q: Vec2| (0..n).min_by(|&i, &j| ring[i].distance_squared(q).total_cmp(&ring[j].distance_squared(q))).unwrap();
    let south = nearest(Vec2::new(THROUGH_X, -EDGE));
    let north = nearest(Vec2::new(THROUGH_X, EDGE));
    let at = |i: isize| ring[(i.rem_euclid(n as isize)) as usize];
    let (s, nn) = (south as isize, north as isize);
    let run = JUNCTION_RUN as isize;
    let mut control = vec![at(s - run), at(s - run / 2), at(s)];
    let middle = [
        at(s) + Vec2::new(0.0, 450.0),
        Vec2::new(THROUGH_X, -LOOP * 0.6), Vec2::new(THROUGH_X, -LOOP * 0.2),
        Vec2::new(THROUGH_X, LOOP * 0.2), Vec2::new(THROUGH_X, LOOP * 0.6),
        at(nn) - Vec2::new(0.0, 450.0),
    ];
    let legs = crate::layout::parallel_map((0..middle.len() - 1).collect(), |k| astar(middle[k], middle[k + 1], &blocked));
    for leg in legs {
        control.extend(leg.into_iter().skip(1));
    }
    control.extend([at(nn), at(nn + run / 2), at(nn + run)]);
    let through = smooth(&control, false);

    vec![Track { points_2d: ring, closed: true }, Track { points_2d: through, closed: false }]
}

/// Routing cells too close to a circuit.
fn circuit_cells(network: &RoadNetwork) -> HashSet<(i32, i32)> {
    let r = (CIRCUIT_CLEARANCE / CELL).ceil() as i32;
    let mut cells = HashSet::new();
    for p in network.paths.iter().filter(|p| p.circuit()).flat_map(|p| p.points.iter().step_by(3)) {
        let (kx, kz) = cell_of(*p);
        for dz in -r..=r {
            for dx in -r..=r {
                let c = Vec2::new((kx + dx) as f32, (kz + dz) as f32) * CELL;
                if c.distance(*p) < CIRCUIT_CLEARANCE {
                    cells.insert((kx + dx, kz + dz));
                }
            }
        }
    }
    cells
}

fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).round() as i32, (p.y / CELL).round() as i32)
}

/// Cheapest way from `from` to `to` over the grid: steep ground costs a lot
/// (a railway can barely climb), water a fair bit (a causeway), and circuits
/// are out of bounds. Returns cell centres, starting at `from`'s cell.
fn astar(from: Vec2, to: Vec2, blocked: &HashSet<(i32, i32)>) -> Vec<Vec2> {
    let margin = 1800.0;
    let lo = cell_of((from.min(to) - margin).max(Vec2::splat(-EDGE)));
    let hi = cell_of((from.max(to) + margin).min(Vec2::splat(EDGE)));
    let (w, h) = ((hi.0 - lo.0 + 1) as usize, (hi.1 - lo.1 + 1) as usize);
    let pos = |i: usize| Vec2::new((lo.0 + (i % w) as i32) as f32, (lo.1 + (i / w) as i32) as f32) * CELL;
    let index = |c: (i32, i32)| ((c.1 - lo.1) as usize) * w + (c.0 - lo.0) as usize;
    let start = index(cell_of(from));
    let goal = index(cell_of(to));

    let mut height = vec![f32::NAN; w * h];
    let mut ground = |i: usize| {
        if height[i].is_nan() {
            let p = pos(i);
            let wet = terrain::raw_height(p.x, p.y) < WATER_LEVEL + 0.5;
            // Water is marked with -inf
            height[i] = if wet { f32::NEG_INFINITY } else { terrain::natural_height(p.x, p.y) };
        }
        height[i]
    };
    let mut cost = vec![f32::INFINITY; w * h];
    let mut came = vec![usize::MAX; w * h];
    let mut open = BinaryHeap::new();
    cost[start] = 0.0;
    open.push(Reverse((0u32, start)));
    while let Some(Reverse((_, i))) = open.pop() {
        if i == goal {
            break;
        }
        let here = ground(i);
        let (cx, cz) = ((i % w) as i32, (i / w) as i32);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (cx + dx, cz + dz);
            if nx < 0 || nz < 0 || nx >= w as i32 || nz >= h as i32 {
                continue;
            }
            let j = nz as usize * w + nx as usize;
            if j != goal && blocked.contains(&(lo.0 + nx, lo.1 + nz)) {
                continue;
            }
            let step = CELL * if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            let there = ground(j);
            let factor = if there == f32::NEG_INFINITY || here == f32::NEG_INFINITY {
                3.5
            } else {
                let slope = (there - here).abs() / step;
                1.0 + 60.0 * slope * slope
            };
            let next = cost[i] + step * factor;
            if next < cost[j] {
                cost[j] = next;
                came[j] = i;
                // Positive floats order the same as their bit patterns
                open.push(Reverse(((next + pos(j).distance(to)).to_bits(), j)));
            }
        }
    }

    let mut path = vec![goal];
    while let Some(&last) = path.last() {
        if last == start || came[last] == usize::MAX {
            break;
        }
        path.push(came[last]);
    }
    path.reverse();
    let mut points: Vec<Vec2> = path.into_iter().map(pos).collect();
    points[0] = from;
    *points.last_mut().unwrap() = to;
    points
}

/// Grid staircase to sweeping curves: resample coarsely, relax, round off
/// anything still tighter than `MIN_RADIUS`, then spline through what's
/// left. Open paths keep their ends.
fn smooth(points: &[Vec2], closed: bool) -> Vec<Vec2> {
    let mut pts = resample(points, closed, 60.0);
    let n = pts.len();
    let relax = |pts: &mut Vec<Vec2>, which: &dyn Fn(usize) -> bool| {
        let prev = pts.clone();
        for i in (0..n).filter(|&i| (closed || (1..n - 1).contains(&i)) && which(i)) {
            let (a, b) = (prev[(i + n - 1) % n], prev[(i + 1) % n]);
            pts[i] = prev[i] * 0.5 + (a + b) * 0.25;
        }
    };
    for _ in 0..30 {
        relax(&mut pts, &|_| true);
    }
    // At this spacing a span of two points either side is ±120 units
    let inner = if closed { 0..n } else { 2..n - 2 };
    for _ in 0..500 {
        let tight: Vec<usize> = inner.clone().filter(|&i| corner_radius(&pts, i, 2) < MIN_RADIUS * 1.1).collect();
        if tight.is_empty() {
            break;
        }
        let near = |i: usize| tight.iter().any(|&t| (i as isize - t as isize).rem_euclid(n as isize).min((t as isize - i as isize).rem_euclid(n as isize)) <= 3);
        relax(&mut pts, &near);
    }
    // The spline bends hardest at its control points; even that out
    let mut fine = resample(&Track::from_control_points(&pts, closed).points_2d, closed, road::SAMPLE_SPACING);
    let m = fine.len();
    for _ in 0..60 {
        let prev = fine.clone();
        for i in (0..m).filter(|&i| closed || (1..m - 1).contains(&i)) {
            fine[i] = prev[i] * 0.5 + (prev[(i + m - 1) % m] + prev[(i + 1) % m]) * 0.25;
        }
    }
    fine
}

// ── The built railway ──

/// Gather the railway the network built: the train's tour, the level
/// crossings, and outlines for maps. Later calls are ignored.
pub fn install(network: &RoadNetwork) {
    RAILWAY.get_or_init(|| build(network));
}

fn build(network: &RoadNetwork) -> Railway {
    let rails: Vec<&road::RoadPath> = network.paths.iter().filter(|p| p.kind == PathKind::Rail).collect();
    let ring = rails.iter().find(|p| p.closed).expect("the loop");
    let through = rails.iter().find(|p| !p.closed).expect("the line through the middle");
    let at = |p: &road::RoadPath, i: usize| vec3(p.points[i].x, p.heights[i], p.points[i].y);
    let dark = |p: &road::RoadPath, i: usize| p.structure[i] == Structure::Tunnel;

    // Round the whole loop, up the middle, then round the far side home
    let n = ring.points.len();
    let nearest = |q: Vec2| (0..n).min_by(|&i, &j| ring.points[i].distance_squared(q).total_cmp(&ring.points[j].distance_squared(q))).unwrap();
    let leave = nearest(through.points[0]);
    let rejoin = nearest(*through.points.last().unwrap());
    let mut tour: Vec<Vec3> = (0..n).map(|k| at(ring, (leave + k) % n)).collect();
    let mut tunnel: Vec<bool> = (0..n).map(|k| dark(ring, (leave + k) % n)).collect();
    tour.extend((1..through.points.len() - 1).map(|i| at(through, i)));
    tunnel.extend((1..through.points.len() - 1).map(|i| dark(through, i)));
    let mut k = rejoin;
    while k != leave {
        tour.push(at(ring, k));
        tunnel.push(dark(ring, k));
        k = (k + 1) % n;
    }
    let mut distances = Vec::with_capacity(tour.len());
    let mut d = 0.0;
    for i in 0..tour.len() {
        distances.push(d);
        d += tour[i].truncate_xz().distance(tour[(i + 1) % tour.len()].truncate_xz());
    }
    let length = d;

    // Start a little south of the middle, heading north
    let through_start = n as f32 * road::SAMPLE_SPACING;
    let start = (0..through.points.len())
        .min_by(|&a, &b| through.points[a].distance_squared(Vec2::new(THROUGH_X, -900.0)).total_cmp(&through.points[b].distance_squared(Vec2::new(THROUGH_X, -900.0))))
        .map_or(0.0, |i| through_start + i as f32 * road::SAMPLE_SPACING);

    // Level crossings: where the tour runs over a road's centreline
    let mut crossings: Vec<Crossing> = Vec::new();
    for (i, p) in tour.iter().enumerate() {
        let q = p.truncate_xz();
        let Some(s) = network.nearest_matching(q.x, q.y, |k| k == PathKind::Road) else { continue };
        if s.dist > 6.0 || crossings.iter().any(|c| c.pos.truncate_xz().distance(q) < 80.0) {
            continue;
        }
        // The road's own direction there
        let road_dir = network.paths.iter()
            .filter(|r| r.kind == PathKind::Road)
            .flat_map(|r| (0..r.points.len()).map(move |k| (r, k)))
            .min_by(|a, b| a.0.points[a.1].distance_squared(q).total_cmp(&b.0.points[b.1].distance_squared(q)))
            .map(|(r, k)| r.tangent(k))
            .unwrap_or(Vec2::X);
        crossings.push(Crossing { pos: vec3(q.x, s.height, q.y), road_dir, at: distances[i] });
    }

    let outlines = rails.iter().map(|p| {
        let mut o: Vec<Vec2> = p.points.iter().step_by(10).copied().collect();
        if p.closed {
            o.push(p.points[0]);
        } else {
            o.push(*p.points.last().unwrap());
        }
        o
    }).collect();

    Railway { tour, distances, tunnel, length, crossings, start, outlines }
}

impl Railway {
    /// Point on the tour `s` units along (wrapping), at bed height.
    pub fn sample(&self, s: f32) -> Vec3 {
        let s = s.rem_euclid(self.length);
        let i = self.distances.partition_point(|&d| d <= s).saturating_sub(1);
        let j = (i + 1) % self.tour.len();
        let next = if j == 0 { self.length } else { self.distances[j] };
        let t = ((s - self.distances[i]) / (next - self.distances[i]).max(1e-4)).clamp(0.0, 1.0);
        self.tour[i].lerp(self.tour[j], t)
    }

    /// Whether the point `s` units along the tour is inside a tunnel.
    pub fn in_tunnel(&self, s: f32) -> bool {
        let s = s.rem_euclid(self.length);
        self.tunnel[self.distances.partition_point(|&d| d <= s).saturating_sub(1)]
    }

    /// Distance from `a` forward to `b` along the tour.
    fn ahead(&self, a: f32, b: f32) -> f32 {
        (b - a).rem_euclid(self.length)
    }
}

// ── The train ──

/// Where one car of the train is, this frame.
pub struct CarPose {
    pub kind: CarKind,
    pub model: Mat4,
    /// Middle of the car at rail-top height.
    pub centre: Vec3,
    pub forward: Vec3,
}

pub struct Train {
    /// How far the front of the locomotive is along the tour.
    pub distance: f32,
    /// Barrier angles at each crossing: 0 lowered, 1 raised.
    pub barriers: Vec<f32>,
    smoke_accum: f32,
}

impl Train {
    pub fn new(railway: &Railway) -> Self {
        Self { distance: railway.start, barriers: vec![1.0; railway.crossings.len()], smoke_accum: 0.0 }
    }

    pub fn update(&mut self, railway: &Railway, dt: f32) {
        self.distance = (self.distance + TRAIN_SPEED * dt).rem_euclid(railway.length);
        let length: f32 = CONSIST.iter().map(|k| k.length() + COUPLING).sum();
        for (c, crossing) in railway.crossings.iter().enumerate() {
            // Down from a quarter of a minute before the train arrives until
            // its last coach has cleared
            let coming = railway.ahead(self.distance, crossing.at) < TRAIN_SPEED * 15.0;
            let passing = railway.ahead(crossing.at, self.distance) < length + 30.0;
            let target = if coming || passing { 0.0 } else { 1.0 };
            let rate = if target > self.barriers[c] { 0.25 } else { 0.4 };
            self.barriers[c] += (target - self.barriers[c]).clamp(-rate * dt, rate * dt);
        }
    }

    /// Whether the warning lights at crossing `c` are flashing.
    pub fn warning(&self, railway: &Railway, c: usize) -> bool {
        let length: f32 = CONSIST.iter().map(|k| k.length() + COUPLING).sum();
        let at = railway.crossings[c].at;
        railway.ahead(self.distance, at) < TRAIN_SPEED * 18.0 || railway.ahead(at, self.distance) < length + 30.0
    }

    pub fn poses(&self, railway: &Railway) -> Vec<CarPose> {
        let mut front = self.distance;
        CONSIST.iter().map(|&kind| {
            let len = kind.length();
            let mid = front - len * 0.5;
            front -= len + COUPLING;
            // Lie along the line between the two bogies
            let a = railway.sample(mid + len * 0.35);
            let b = railway.sample(mid - len * 0.35);
            let forward = (a - b).normalize_or_zero();
            let right = forward.cross(Vec3::Y).normalize_or_zero();
            let up = right.cross(forward);
            let centre = (a + b) * 0.5 + Vec3::Y * RAIL_TOP;
            let model = Mat4::from_cols(right.extend(0.0), up.extend(0.0), (-forward).extend(0.0), centre.extend(1.0));
            CarPose { kind, model, centre, forward }
        }).collect()
    }

    /// Smoke from the chimney, rising and drifting back as the train goes.
    pub fn smoke(&mut self, railway: &Railway, poses: &[CarPose], particles: &mut crate::particles::Particles, dt: f32) {
        let Some(loco) = poses.first() else { return };
        // Not while the chimney is underground
        if railway.in_tunnel(self.distance - 2.0) {
            return;
        }
        self.smoke_accum += dt * 6.0;
        while self.smoke_accum >= 1.0 {
            self.smoke_accum -= 1.0;
            let chimney = loco.model.transform_point3(vec3(0.0, 10.4, -9.6));
            particles.chimney_smoke(chimney, loco.forward * TRAIN_SPEED * 0.6);
        }
    }

    /// The cars as solids moving at the train's speed, for collisions.
    pub fn solids(poses: &[CarPose]) -> Vec<(Solid, Vec3)> {
        poses.iter().map(|p| {
            let yaw = (-p.forward.x).atan2(-p.forward.z);
            let solid = Solid {
                centre: Vec2::new(p.centre.x, p.centre.z),
                half: Vec2::new(TRAIN_HALF_WIDTH, p.kind.length() * 0.5),
                yaw,
                top: p.centre.y + 9.0,
                round: false,
            };
            (solid, p.forward * TRAIN_SPEED)
        }).collect()
    }
}

// ── Static props at the crossings ──

const STONE: [f32; 4] = [0.6, 0.56, 0.5, 1.0];
const STONE_DARK: [f32; 4] = [0.5, 0.46, 0.41, 1.0];
const STONE_LIGHT: [f32; 4] = [0.7, 0.66, 0.6, 1.0];
const BORE: [f32; 4] = [0.04, 0.04, 0.05, 1.0];
/// Samples between a viaduct's piers (48 units).
const PIER_EVERY: usize = 12;
/// Half the width of a viaduct or tunnel.
const STRUCTURE_HALF_WIDTH: f32 = RAIL_HALF_WIDTH + 0.4;

/// Viaducts, tunnel portals, and the barrier posts and crossbuck signs at
/// the level crossings, for whatever's anchored in the chunk `inside` accepts.
pub fn build_props(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    if let Some(network) = road::network() {
        for path in network.paths.iter().filter(|p| p.kind == PathKind::Rail) {
            viaducts(b, path, inside);
            portals(b, path, inside);
        }
    }
    crossing_props(b, inside);
}

/// Point and unit side vector at fractional sample `f` along `path`.
fn along(path: &RoadPath, f: f32) -> (Vec2, Vec2, f32) {
    let n = path.points.len();
    let i = (f.floor() as usize).min(n - 1);
    let j = if path.closed { (i + 1) % n } else { (i + 1).min(n - 1) };
    let t = f - f.floor();
    let p = path.points[i].lerp(path.points[j], t);
    let tan = path.tangent(i).lerp(path.tangent(j), t).normalize_or_zero();
    (p, Vec2::new(-tan.y, tan.x), path.heights[i] + (path.heights[j] - path.heights[i]) * t)
}

/// Stone piers every `PIER_EVERY` samples under each viaduct, with round
/// arches between them where there's the height (a wall where there isn't).
fn viaducts(b: &mut Builder, path: &RoadPath, inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    let w = STRUCTURE_HALF_WIDTH;
    let ground = |p: Vec2| terrain::raw_height(p.x, p.y).max(WATER_LEVEL - 4.0);
    for i in (0..n).step_by(PIER_EVERY) {
        let next = i + PIER_EVERY;
        if path.structure[i] != Structure::Viaduct || !inside(path.points[i]) {
            continue;
        }
        let (p, side, h) = along(path, i as f32);
        let under = h - 2.6;
        let foot = ground(p);
        // Blocks lie along the line (local Z) and across it (local X)
        let tan = Vec2::new(side.y, -side.x);
        let block_yaw = (-tan.x).atan2(-tan.y);
        b.solid_block(vec3(p.x, foot - 1.0, p.y), w + 0.6, under - foot + 1.0, 2.6, block_yaw, STONE, STONE);
        // A string course where the pier meets the arches
        b.block(vec3(p.x, under - 0.6, p.y), w + 0.9, 0.6, 3.0, block_yaw, STONE_LIGHT, STONE_LIGHT);

        if next >= n && !path.closed || path.structure[next % n] != Structure::Viaduct {
            continue;
        }
        // The span to the next pier, following the curve of the line
        let (a, z) = (i as f32 + 0.65, next as f32 - 0.65);
        let clear = under - ground(along(path, (a + z) * 0.5).0);
        let rise = ((z - a) * road::SAMPLE_SPACING * 0.5).min(clear - 4.0);
        const STEPS: usize = 10;
        let point = |k: usize| {
            let t = k as f32 / STEPS as f32;
            let (q, side, hh) = along(path, (a + (z - a) * t).rem_euclid(n as f32));
            let top = hh - 2.6;
            let crown = if rise > 3.0 { top - 1.2 - rise * (1.0 - (std::f32::consts::PI * t).sin()) } else { ground(q) - 1.0 };
            (q, side, top, crown)
        };
        for k in 0..STEPS {
            let (q0, s0, top0, y0) = point(k);
            let (q1, s1, top1, y1) = point(k + 1);
            for sgn in [-1.0, 1.0] {
                let (e0, e1) = (q0 + s0 * sgn * w, q1 + s1 * sgn * w);
                // Spandrel: the wall face between the arch and the deck
                b.quad(vec3(e0.x, y0, e0.y), vec3(e1.x, y1, e1.y), vec3(e1.x, top1, e1.y), vec3(e0.x, top0, e0.y), STONE);
            }
            // The arch's underside
            let (l0, r0, l1, r1) = (q0 - s0 * w, q0 + s0 * w, q1 - s1 * w, q1 + s1 * w);
            b.quad(vec3(l0.x, y0, l0.y), vec3(r0.x, y0, r0.y), vec3(r1.x, y1, r1.y), vec3(l1.x, y1, l1.y), STONE_DARK);
        }
    }
}

/// A stone portal at each tunnel mouth, with a short lined bore behind it so
/// the train is out of sight the moment it's through.
fn portals(b: &mut Builder, path: &RoadPath, inside: &impl Fn(Vec2) -> bool) {
    const INSET: usize = 5;
    const LINER: usize = 25;
    let n = path.points.len();
    let tunnel = |i: usize| path.structure[i % n] == Structure::Tunnel;
    for i in 0..n {
        let prev = if i == 0 { if path.closed { n - 1 } else { continue } } else { i - 1 };
        // Mouths: where a tunnel starts (facing back) or ends (facing on)
        let (mouth, dir): (usize, f32) = if tunnel(i) && !tunnel(prev) {
            (i + INSET, 1.0)
        } else if !tunnel(i) && tunnel(prev) {
            ((i + n - 1 - INSET) % n, -1.0)
        } else {
            continue;
        };
        let mouth = mouth % n;
        if !inside(path.points[mouth]) {
            continue;
        }
        let (p, side, h) = along(path, mouth as f32);
        let tan = Vec2::new(side.y, -side.x);
        // `out` points away from the hill (`dir` is the way into it)
        let out = vec3(-tan.x * dir, 0.0, -tan.y * dir);
        let s3 = vec3(side.x, 0.0, side.y);
        let base = vec3(p.x, h, p.y);
        let yaw = (-tan.x).atan2(-tan.y);
        b.block(base - Vec3::Y * 3.0, 17.0, 27.0, 2.0, yaw, STONE, STONE);
        b.block(base + Vec3::Y * 24.0 + out * 0.4, 18.0, 1.4, 2.6, yaw, STONE_LIGHT, STONE_LIGHT);
        // The dark mouth: straight sides and a round head
        let face = base + out * 2.05;
        let (half, spring) = (5.8, 8.5);
        b.quad(face - s3 * half, face + s3 * half, face + s3 * half + Vec3::Y * spring, face - s3 * half + Vec3::Y * spring, BORE);
        let ring = |a: f32, r: f32| face + Vec3::Y * spring + s3 * a.cos() * r + Vec3::Y * a.sin() * r;
        for k in 0..10 {
            let (a0, a1) = (std::f32::consts::PI * k as f32 / 10.0, std::f32::consts::PI * (k + 1) as f32 / 10.0);
            b.tri(face + Vec3::Y * spring, ring(a0, half), ring(a1, half), BORE);
            // Voussoirs round the arch, a shade lighter
            let o = out * 0.05;
            b.quad(ring(a0, half) + o, ring(a1, half) + o, ring(a1, half + 1.3) + o, ring(a0, half + 1.3) + o, STONE_LIGHT);
        }
        // The lined bore, walls and roof, running into the hill
        for k in 0..LINER {
            let (f0, f1) = (mouth as f32 + dir * k as f32, mouth as f32 + dir * (k + 1) as f32);
            if f1 < 0.0 && !path.closed {
                break;
            }
            let (q0, sd0, h0) = along(path, f0.rem_euclid(n as f32));
            let (q1, sd1, h1) = along(path, f1.rem_euclid(n as f32));
            let w = STRUCTURE_HALF_WIDTH - 1.0;
            for sgn in [-1.0, 1.0] {
                let (e0, e1) = (q0 + sd0 * sgn * w, q1 + sd1 * sgn * w);
                b.quad(vec3(e0.x, h0 - 1.0, e0.y), vec3(e1.x, h1 - 1.0, e1.y), vec3(e1.x, h1 + 14.0, e1.y), vec3(e0.x, h0 + 14.0, e0.y), STONE_DARK);
            }
            let (l0, r0, l1, r1) = (q0 - sd0 * w, q0 + sd0 * w, q1 - sd1 * w, q1 + sd1 * w);
            b.quad(vec3(l0.x, h0 + 14.0, l0.y), vec3(r0.x, h0 + 14.0, r0.y), vec3(r1.x, h1 + 14.0, r1.y), vec3(l1.x, h1 + 14.0, l1.y), STONE_DARK);
        }
    }
}

/// Barrier posts and crossbuck signs at the level crossings whose centre is
/// in the chunk `inside` accepts.
fn crossing_props(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    const POST: [f32; 4] = [0.85, 0.85, 0.82, 1.0];
    const BOX: [f32; 4] = [0.75, 0.7, 0.2, 1.0];
    const RED: [f32; 4] = [0.8, 0.12, 0.1, 1.0];
    const WHITE: [f32; 4] = [0.95, 0.95, 0.92, 1.0];
    let Some(railway) = get() else { return };
    for crossing in railway.crossings.iter().filter(|c| inside(c.pos.truncate_xz())) {
        for (pivot, reach, face) in crossing.barriers() {
            let foot = vec3(pivot.x, pivot.y - 3.0, pivot.z);
            b.block(foot, 0.3, 3.2, 0.3, 0.0, POST, POST);
            b.post(foot, 0.4, 3.2);
            b.block(foot - reach * 1.0, 0.6, 1.2, 0.6, 0.0, BOX, BOX);
            // Black housings behind the warning lamps
            for lamp in lamp_positions(pivot, reach, face) {
                b.block(lamp - Vec3::Y * 0.6 - face * 0.25, 0.6, 1.2, 0.6, 0.0, BLACK, BLACK);
            }
            // Crossbuck on its own post beside it, facing the oncoming road
            let sign_at = vec3(pivot.x, foot.y, pivot.z) - reach * 3.5;
            b.block(sign_at, 0.15, 6.0, 0.15, 0.0, POST, POST);
            let right = Vec3::Y.cross(face).normalize_or_zero();
            let c = sign_at + Vec3::Y * 5.2 + face * 0.2;
            for angle in [0.75_f32, -0.75] {
                let u = (right * angle.cos() + Vec3::Y * angle.sin()) * 2.6;
                let v = (right * -angle.sin() + Vec3::Y * angle.cos()) * 0.4;
                for (w, col, lift) in [(1.0, RED, 0.0), (0.7, WHITE, 0.03)] {
                    let (u, v) = (u * (0.96 + 0.04 * w), v * w);
                    let o = face * lift;
                    b.quad(c - u - v + o, c + u - v + o, c + u + v + o, c - u + v + o, col);
                }
            }
        }
    }
}

// ── Meshes ──

/// Meshes for the train and crossings, built once: each is (verts, idxs) for
/// what's always drawn, plus what glows (lit windows and lamps).
pub struct MeshData {
    pub loco: (Vec<Vertex>, Vec<u32>),
    pub loco_glow: (Vec<Vertex>, Vec<u32>),
    pub tender: (Vec<Vertex>, Vec<u32>),
    pub coach: (Vec<Vertex>, Vec<u32>),
    pub coach_glow: (Vec<Vertex>, Vec<u32>),
    /// Barrier arm, pivot at the origin, reaching along +X.
    pub barrier: (Vec<Vertex>, Vec<u32>),
    /// A warning lamp, centred on the origin.
    pub lamp: (Vec<Vertex>, Vec<u32>),
}

const GREEN: [f32; 4] = [0.1, 0.36, 0.22, 1.0];
const BLACK: [f32; 4] = [0.1, 0.1, 0.11, 1.0];
const BRASS: [f32; 4] = [0.82, 0.64, 0.26, 1.0];
const WHEEL_RED: [f32; 4] = [0.55, 0.12, 0.08, 1.0];
const STEEL: [f32; 4] = [0.7, 0.7, 0.72, 1.0];
const BUFFER_RED: [f32; 4] = [0.75, 0.13, 0.1, 1.0];
const CHOCOLATE: [f32; 4] = [0.36, 0.19, 0.12, 1.0];
const CREAM: [f32; 4] = [0.9, 0.84, 0.68, 1.0];
const ROOF: [f32; 4] = [0.4, 0.4, 0.43, 1.0];
const GLASS: [f32; 4] = [0.14, 0.17, 0.2, 1.0];
const WARM: [f32; 4] = [1.0, 0.82, 0.5, 1.0];
const FIRE: [f32; 4] = [1.0, 0.55, 0.2, 1.0];

/// Wheel with its axle across the car, `x` out to the side.
fn wheel(b: &mut Builder, x: f32, z: f32, r: f32, color: [f32; 4]) {
    let c = vec3(x, r, z);
    let out = vec3(x.signum() * 0.5, 0.0, 0.0);
    b.limb(c - out, c + out, r, r, 12, 0.0, color);
    b.limb(c + out, c + out * 1.3, r * 0.3, r * 0.3, 6, 0.0, STEEL);
}

fn buffers(b: &mut Builder, z: f32, dir: f32) {
    for x in [-2.0, 2.0] {
        let a = vec3(x, 2.4, z);
        b.limb(a, a + vec3(0.0, 0.0, dir * 0.8), 0.32, 0.32, 6, 0.0, BLACK);
        b.limb(a + vec3(0.0, 0.0, dir * 0.8), a + vec3(0.0, 0.0, dir * 0.95), 0.5, 0.5, 8, 0.0, BLACK);
    }
}

pub fn build_meshes() -> MeshData {
    // Locomotive: front at -Z
    let mut b = Builder::default();
    for x in [-1.9, 1.9] {
        for z in [-3.6, 0.8, 5.2] {
            wheel(&mut b, x, z, 2.0, WHEEL_RED);
        }
        wheel(&mut b, x, -8.6, 1.1, WHEEL_RED);
        // Coupling rod linking the driving wheels
        b.block(vec3(x * 1.38, 1.85, 0.8), 0.1, 0.3, 4.6, 0.0, STEEL, STEEL);
        // Cylinder
        b.limb(vec3(x * 1.4, 2.9, -10.2), vec3(x * 1.4, 2.9, -6.8), 0.85, 0.85, 8, 0.0, GREEN);
    }
    b.block(vec3(0.0, 1.0, 0.0), 1.4, 2.6, 11.0, 0.0, BLACK, BLACK);
    b.block(vec3(0.0, 3.5, -0.3), 3.0, 0.45, 11.2, 0.0, BLACK, BLACK);
    // Boiler with brass bands, smokebox, chimney and dome
    b.limb(vec3(0.0, 6.2, -10.0), vec3(0.0, 6.2, 3.2), 2.3, 2.3, 14, 0.0, GREEN);
    for z in [-7.0, -3.0, 1.0] {
        b.limb(vec3(0.0, 6.2, z - 0.15), vec3(0.0, 6.2, z + 0.15), 2.36, 2.36, 14, 0.0, BRASS);
    }
    b.limb(vec3(0.0, 6.2, -11.3), vec3(0.0, 6.2, -10.0), 2.4, 2.4, 14, 0.0, BLACK);
    b.limb(vec3(0.0, 6.2, -11.4), vec3(0.0, 6.2, -11.3), 1.6, 1.6, 10, 0.0, [0.2, 0.2, 0.22, 1.0]);
    b.limb(vec3(0.0, 8.0, -9.6), vec3(0.0, 10.1, -9.6), 0.75, 0.85, 10, 0.0, BLACK);
    b.limb(vec3(0.0, 10.1, -9.6), vec3(0.0, 10.4, -9.6), 1.05, 1.05, 10, 0.0, BLACK);
    b.limb(vec3(0.0, 8.0, -3.6), vec3(0.0, 9.1, -3.6), 1.05, 0.7, 10, 0.0, BRASS);
    b.limb(vec3(0.0, 8.2, 1.4), vec3(0.0, 9.0, 1.4), 0.35, 0.3, 6, 0.0, BRASS);
    // Cab
    b.block(vec3(0.0, 3.95, 6.4), 3.0, 5.1, 3.2, 0.0, GREEN, GREEN);
    b.block(vec3(0.0, 9.05, 6.6), 3.35, 0.4, 3.8, 0.0, BLACK, BLACK);
    for x in [-1.0, 1.0] {
        b.quad(vec3(x * 3.02, 5.8, 4.4), vec3(x * 3.02, 5.8, 8.2), vec3(x * 3.02, 8.4, 8.2), vec3(x * 3.02, 8.4, 4.4), BLACK);
    }
    // Buffer beam, buffers and headlamp
    b.block(vec3(0.0, 1.6, -11.6), 3.0, 1.5, 0.35, 0.0, BUFFER_RED, BUFFER_RED);
    buffers(&mut b, -11.95, -1.0);
    b.block(vec3(0.0, 8.4, -11.0), 0.5, 0.8, 0.4, 0.0, BLACK, BLACK);
    b.glowing = true;
    for x in [-1.0, 1.0] {
        // Spectacle glasses in the cab front, firelit after dark
        b.quad(vec3(x * 1.8 - 0.6, 7.2, 3.18), vec3(x * 1.8 + 0.6, 7.2, 3.18), vec3(x * 1.8 + 0.6, 8.3, 3.18), vec3(x * 1.8 - 0.6, 8.3, 3.18), FIRE);
    }
    b.quad(vec3(-0.35, 8.55, -11.42), vec3(0.35, 8.55, -11.42), vec3(0.35, 9.05, -11.42), vec3(-0.35, 9.05, -11.42), [1.0, 0.95, 0.8, 1.0]);
    b.glowing = false;
    let loco = (std::mem::take(&mut b.verts), std::mem::take(&mut b.idxs));
    let loco_glow = (std::mem::take(&mut b.glow_verts), std::mem::take(&mut b.glow_idxs));

    // Tender: coal heaped on top
    let mut b = Builder::default();
    for x in [-1.9, 1.9] {
        for z in [-4.0, 0.0, 4.0] {
            wheel(&mut b, x, z, 1.05, BLACK);
        }
    }
    b.block(vec3(0.0, 1.9, 0.0), 2.8, 0.5, 5.9, 0.0, BLACK, BLACK);
    b.block(vec3(0.0, 2.4, 0.0), 2.9, 4.0, 5.8, 0.0, GREEN, GREEN);
    b.block(vec3(0.0, 6.4, 0.0), 2.95, 0.3, 5.85, 0.0, BLACK, BLACK);
    b.blob(vec3(0.0, 6.5, -0.8), 2.6, 0.4, [0.08, 0.08, 0.09, 1.0], 3.0);
    b.blob(vec3(0.6, 6.4, 2.4), 1.8, 0.4, [0.1, 0.1, 0.11, 1.0], 7.0);
    buffers(&mut b, 6.0, 1.0);
    let tender = (b.verts, b.idxs);

    // Coach: chocolate and cream, on two bogies, windows lit after dark
    let mut b = Builder::default();
    for bogie in [-9.0, 9.0] {
        b.block(vec3(0.0, 0.7, bogie), 2.0, 1.3, 2.6, 0.0, BLACK, BLACK);
        for x in [-1.9, 1.9] {
            for z in [-1.6, 1.6] {
                wheel(&mut b, x, bogie + z, 0.9, BLACK);
            }
        }
    }
    b.block(vec3(0.0, 2.0, 0.0), 2.8, 0.8, 13.0, 0.0, BLACK, BLACK);
    b.block(vec3(0.0, 2.8, 0.0), 3.1, 2.8, 13.3, 0.0, CHOCOLATE, CHOCOLATE);
    b.block(vec3(0.0, 5.6, 0.0), 3.1, 2.4, 13.3, 0.0, CREAM, CREAM);
    b.block(vec3(0.0, 8.0, 0.0), 3.3, 0.5, 13.45, 0.0, ROOF, ROOF);
    b.block(vec3(0.0, 8.5, 0.0), 2.4, 0.45, 13.2, 0.0, ROOF, ROOF);
    for z in [-13.6, 13.6] {
        b.block(vec3(0.0, 2.8, z), 1.4, 5.0, 0.3, 0.0, BLACK, BLACK);
        buffers(&mut b, z.signum() * 13.6, z.signum());
    }
    for k in 0..9 {
        let z = -11.0 + k as f32 * 2.75;
        for x in [-1.0_f32, 1.0] {
            let fx = x * 3.11;
            b.quad(vec3(fx, 5.9, z - 0.9), vec3(fx, 5.9, z + 0.9), vec3(fx, 7.6, z + 0.9), vec3(fx, 7.6, z - 0.9), GLASS);
            b.glowing = true;
            let gx = x * 3.13;
            b.quad(vec3(gx, 5.95, z - 0.85), vec3(gx, 5.95, z + 0.85), vec3(gx, 7.55, z + 0.85), vec3(gx, 7.55, z - 0.85), WARM);
            b.glowing = false;
        }
    }
    let coach = (std::mem::take(&mut b.verts), std::mem::take(&mut b.idxs));
    let coach_glow = (std::mem::take(&mut b.glow_verts), std::mem::take(&mut b.glow_idxs));

    // Barrier arm: red and white, with a counterweight behind the pivot
    let mut b = Builder::default();
    for k in 0..11 {
        let col = if k % 2 == 0 { [0.85, 0.12, 0.1, 1.0] } else { [0.95, 0.95, 0.92, 1.0] };
        b.block(vec3(1.0 + k as f32 * 2.0, -0.25, 0.0), 1.0, 0.5, 0.2, 0.0, col, col);
    }
    b.block(vec3(-1.0, -0.6, 0.0), 1.0, 1.2, 0.35, 0.0, BLACK, BLACK);
    let barrier = (b.verts, b.idxs);

    let mut b = Builder::default();
    b.blob(Vec3::ZERO, 0.42, 1.0, [1.0, 0.15, 0.1, 1.0], 1.0);
    let lamp = (b.verts, b.idxs);

    MeshData { loco, loco_glow, tender, coach, coach_glow, barrier, lamp }
}

/// Transform for a barrier arm: pivot, the way it reaches when down, and how
/// far up it is (0 down, 1 up).
pub fn barrier_transform(pivot: Vec3, reach: Vec3, raised: f32) -> Mat4 {
    let yaw = Quat::from_rotation_arc(Vec3::X, reach.normalize_or_zero());
    let lift = Quat::from_rotation_z(raised.clamp(0.0, 1.0) * 1.45);
    Mat4::from_rotation_translation(yaw * lift, pivot)
}

/// The pair of warning lamps on the post at `pivot`, facing `face`.
pub fn lamp_positions(pivot: Vec3, reach: Vec3, face: Vec3) -> [Vec3; 2] {
    [1.0, -1.0].map(|s| pivot + Vec3::Y * 1.3 + reach * s * 0.8 + face * 0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn railway_runs_round_and_through_clear_of_the_circuits() {
        let network = crate::sim_tests::network();
        let railway = get().expect("railway installed");
        println!("tour {:.0} units ({:.0} minutes at speed), {} level crossings", railway.length, railway.length / TRAIN_SPEED / 60.0, railway.crossings.len());
        let rails: Vec<_> = network.paths.iter().filter(|p| p.kind == PathKind::Rail).collect();
        assert_eq!(rails.len(), 2, "the loop and the line through the middle");
        for rail in &rails {
            let n = rail.points.len();
            let mut worst_grade = 0.0_f32;
            for i in 0..n {
                let p = rail.points[i];
                let circuit = network.nearest_matching(p.x, p.y, |k| k == PathKind::Circuit);
                assert!(circuit.is_none(), "railway at {p} runs within {:.0} of a circuit", circuit.unwrap().dist);
                assert!(p.x.abs() < EDGE + 50.0 && p.y.abs() < EDGE + 50.0, "railway off the map at {p}");
                if i + 1 < n {
                    let grade = (rail.heights[i + 1] - rail.heights[i]).abs() / road::SAMPLE_SPACING;
                    worst_grade = worst_grade.max(grade);
                }
            }
            let worst_bank = (0..n).filter(|&i| rail.structure[i] == road::Structure::Ground)
                .map(|i| (rail.heights[i] - terrain::natural_height(rail.points[i].x, rail.points[i].y)).abs())
                .fold(0.0, f32::max);
            assert!(worst_bank < 45.0, "an embankment or cutting {worst_bank:.0} deep where there should be a viaduct or tunnel");
            let tightest = (3..n - 3).map(|i| road::corner_radius(&rail.points, i, 3)).fold(f32::MAX, f32::min);
            println!("{} line: {:.0} units, steepest {:.1}%, tightest curve {:.0}", if rail.closed { "loop" } else { "through" }, n as f32 * road::SAMPLE_SPACING, worst_grade * 100.0, tightest);
            assert!(worst_grade < 0.06, "too steep for a train: {:.1}%", worst_grade * 100.0);
            assert!(tightest > MIN_RADIUS * 0.6, "curve of radius {tightest:.0}");
        }
        // The tour is continuous: no jumps between consecutive points
        for i in 0..railway.tour.len() {
            let gap = railway.tour[i].distance(railway.tour[(i + 1) % railway.tour.len()]);
            assert!(gap < 12.0, "tour jumps {gap:.1} at {i}");
        }
        assert!(railway.crossings.len() >= 3, "the line through the middle crosses the roads between the circuits");
    }
}
