//! Race day at the circuits: grandstands packed with fans, the pits and the
//! paddock behind them, fans camped out round the lap, marshals, and the
//! sponsors' bridge over the track. Each thing claims a plot of ground, and
//! only goes where it fits; trees and grass keep off the plots. The people
//! and the flags are drawn in layers the shaders animate (see `props::Layer`).

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec3};

use crate::noise::CellMap;
use crate::props::{hash, ground, Builder, Layer};
use crate::road::{PathKind, RoadNetwork, RoadPath};
use crate::terrain;
use crate::trackside::{self, side, SPONSORS};

/// Distance from a circuit's centreline to the front of what stands by it.
const STAND_FRONT: f32 = 60.0;
/// How far from the centreline the pits may stand, nearest first, and how
/// long they may be, longest first.
const PIT_FRONTS: [f32; 3] = [62.0, 70.0, 80.0];
const PIT_HALVES: [f32; 3] = [104.0, 84.0, 64.0];
/// Where the pit wall stands, from the centreline.
const PIT_WALL: f32 = 36.0;
const GARAGE_WIDTH: f32 = 15.0;
const BUILDING_DEPTH: f32 = 26.0;
/// How deep the rows of a stand are, and how much each rises on the last.
const ROW_DEPTH: f32 = 2.3;
const ROW_RISE: f32 = 1.2;
/// Height of the wall a stand's first row sits on.
const STAND_WALL: f32 = 3.2;
/// The stands along the start straight.
const MAIN_STAND_ROWS: usize = 11;
const CORNER_STAND_ROWS: usize = 7;
const STAND_HALF: f32 = 37.0;
/// Fans along the rows, this far apart.
const SEAT: f32 = 1.75;
/// Samples between chances for a camp of fans, and for a marshals' post.
const CAMP_EVERY: usize = 42;
const MARSHAL_EVERY: usize = 90;
/// How much the ground may vary under a plot of each kind.
const STAND_RELIEF: f32 = 13.0;
const CAMP_RELIEF: f32 = 11.0;
/// Clearance kept round a plot from trees and plants.
const PLOT_MARGIN: f32 = 5.0;
/// Grid cell for finding the plots near a point.
const CELL: f32 = 128.0;

const CONCRETE: [f32; 4] = [0.72, 0.71, 0.68, 1.0];
const CONCRETE_DARK: [f32; 4] = [0.5, 0.5, 0.49, 1.0];
const STEEL: [f32; 4] = [0.42, 0.44, 0.48, 1.0];
const PAINT: [f32; 4] = [0.93, 0.93, 0.9, 1.0];
const SHADE: [f32; 4] = [0.1, 0.11, 0.13, 1.0];
const TARMAC: [f32; 4] = [0.3, 0.3, 0.33, 1.0];
const GLASS: [f32; 4] = [0.2, 0.34, 0.46, 1.0];
const WARM_LIGHT: [f32; 4] = [1.0, 0.86, 0.6, 1.0];
/// A stand's seats, two tones a side, and its roof.
const LIVERIES: [([f32; 4], [f32; 4], [f32; 4]); 5] = [
    ([0.14, 0.32, 0.72, 1.0], [0.82, 0.84, 0.88, 1.0], [0.8, 0.14, 0.12, 1.0]),
    ([0.72, 0.14, 0.14, 1.0], [0.9, 0.84, 0.72, 1.0], [0.16, 0.2, 0.34, 1.0]),
    ([0.1, 0.5, 0.42, 1.0], [0.9, 0.86, 0.5, 1.0], [0.9, 0.9, 0.88, 1.0]),
    ([0.88, 0.5, 0.1, 1.0], [0.24, 0.24, 0.3, 1.0], [0.88, 0.5, 0.1, 1.0]),
    ([0.42, 0.2, 0.62, 1.0], [0.88, 0.8, 0.9, 1.0], [0.25, 0.65, 0.75, 1.0]),
];
/// Team colours, for the garages, their transporters and their cars.
const TEAMS: [[f32; 4]; 8] = [
    [0.86, 0.12, 0.1, 1.0], [0.1, 0.3, 0.85, 1.0], [0.95, 0.75, 0.1, 1.0], [0.08, 0.55, 0.3, 1.0],
    [0.92, 0.45, 0.08, 1.0], [0.08, 0.6, 0.7, 1.0], [0.5, 0.15, 0.65, 1.0], [0.12, 0.12, 0.14, 1.0],
];
pub(crate) const SHIRTS: [[f32; 3]; 12] = [
    [0.86, 0.16, 0.14], [0.14, 0.38, 0.82], [0.95, 0.8, 0.14], [0.14, 0.66, 0.3], [0.94, 0.5, 0.1], [0.72, 0.2, 0.66],
    [0.93, 0.93, 0.9], [0.12, 0.13, 0.16], [0.1, 0.62, 0.74], [0.96, 0.46, 0.62], [0.4, 0.42, 0.46], [0.55, 0.72, 0.18],
];
pub(crate) const SKINS: [[f32; 3]; 5] = [[0.93, 0.76, 0.62], [0.84, 0.64, 0.48], [0.65, 0.44, 0.3], [0.42, 0.28, 0.2], [0.96, 0.82, 0.7]];
pub(crate) const HAIR: [[f32; 3]; 4] = [[0.1, 0.08, 0.07], [0.32, 0.2, 0.1], [0.72, 0.56, 0.24], [0.66, 0.66, 0.68]];
const BUNTING: [[f32; 3]; 5] = [[0.9, 0.15, 0.12], [0.96, 0.96, 0.92], [0.12, 0.34, 0.82], [0.98, 0.8, 0.12], [0.12, 0.62, 0.34]];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Pits,
    /// A grandstand with this many rows.
    Stand(usize),
    /// Fans camped on the grass.
    Camp,
    Marshals,
    /// The funfair at the home circuit.
    Fair,
}

/// A piece of ground beside a circuit. `front` is the middle of the edge
/// nearest the track; `along` runs beside it and `out` away from it.
struct Plot {
    kind: Kind,
    front: Vec2,
    along: Vec2,
    out: Vec2,
    half: f32,
    depth: f32,
    /// Lowest ground under it.
    base: f32,
    seed: f32,
    /// How far its front is from the circuit's centreline.
    offset: f32,
}

impl Plot {
    fn new(kind: Kind, front: Vec2, along: Vec2, out: Vec2, half: f32, depth: f32, seed: f32) -> Self {
        Self { kind, front, along, out, half, depth, base: 0.0, seed, offset: STAND_FRONT }
    }

    fn holds(&self, p: Vec2, margin: f32) -> bool {
        let d = p - self.front;
        d.dot(self.along).abs() < self.half + margin && d.dot(self.out) > -margin && d.dot(self.out) < self.depth + margin
    }

    fn middle(&self) -> Vec2 {
        self.front + self.out * self.depth * 0.5
    }

    /// World position `u` along, `v` out and `y` above the base.
    fn at(&self, u: f32, v: f32, y: f32) -> Vec3 {
        let p = self.front + self.along * u + self.out * v;
        vec3(p.x, self.base + y, p.y)
    }

    /// Rotation for a `Builder::block` with its Z along the plot.
    fn yaw(&self) -> f32 {
        (-self.along.x).atan2(-self.along.y)
    }

    /// A block `half` long and `half_out` deep with its bottom's middle at
    /// (u, v, y).
    fn block(&self, b: &mut Builder, u: f32, v: f32, y: f32, half: f32, half_out: f32, height: f32, color: [f32; 4], top: [f32; 4]) {
        b.block(self.at(u, v, y), half_out, height, half, self.yaw(), color, top);
    }

    fn solid(&self, b: &mut Builder, u: f32, v: f32, y: f32, half: f32, half_out: f32, height: f32, color: [f32; 4], top: [f32; 4]) {
        b.solid_block(self.at(u, v, y), half_out, height, half, self.yaw(), color, top);
    }
}

/// The plots round all the circuits, bucketed by cell.
struct Venues {
    plots: Vec<Plot>,
    grid: CellMap<Vec<u32>>,
}

static VENUES: OnceLock<Venues> = OnceLock::new();

fn key(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

fn venues(network: &RoadNetwork) -> &'static Venues {
    VENUES.get_or_init(|| {
        // The circuits are far enough apart to lay out each on its own
        let circuits: Vec<(usize, &RoadPath)> = network.paths.iter().filter(|p| p.circuit()).enumerate().collect();
        let plots: Vec<Plot> = crate::layout::parallel_map(circuits, |(k, path)| {
            let mut plots = Vec::new();
            lay_out(network, path, k, &mut plots);
            plots
        }).into_iter().flatten().collect();
        let mut grid: CellMap<Vec<u32>> = CellMap::default();
        for (i, plot) in plots.iter().enumerate() {
            let reach = plot.half.max(plot.depth) + PLOT_MARGIN * 3.0;
            let (lo, hi) = (key(plot.middle() - reach), key(plot.middle() + reach));
            for z in lo.1..=hi.1 {
                for x in lo.0..=hi.0 {
                    grid.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        Venues { plots, grid }
    })
}

/// Lay everything out now, rather than when the first chunk asks.
pub fn prepare(network: &RoadNetwork) {
    venues(network);
    trackside::streetlights(network);
    crate::roadside::prepare(network);
}

/// Whether `p` is in the pit lane or on the pit wall, between a circuit's
/// pits and the track.
pub fn pit_lane(p: Vec2) -> bool {
    let Some(venues) = crate::road::network().map(venues) else { return false };
    venues.plots.iter().filter(|plot| plot.kind == Kind::Pits).any(|plot| {
        let d = p - plot.front;
        d.dot(plot.along).abs() < plot.half + 14.0 && d.dot(plot.out) > PIT_WALL - plot.offset - 3.0 && d.dot(plot.out) < 2.0
    })
}

/// How loud the crowd is at `p`, 0..1: up close to a packed stand, a little
/// for fans camped out, nothing away from the circuits.
pub fn crowd_near(p: Vec2) -> f32 {
    let Some(venues) = crate::road::network().map(venues) else { return 0.0 };
    venues.grid.get(&key(p)).into_iter().flatten().chain(venues.grid.get(&key(p + Vec2::splat(CELL * 0.5))).into_iter().flatten())
        .map(|&i| {
            let plot = &venues.plots[i as usize];
            let weight = match plot.kind {
                Kind::Stand(rows) => rows as f32 / MAIN_STAND_ROWS as f32,
                Kind::Pits | Kind::Fair => 0.6,
                Kind::Camp => 0.3,
                Kind::Marshals => 0.0,
            };
            let d = p.distance(plot.middle()) - plot.half * 0.6;
            weight * (1.0 - ((d - 20.0) / 260.0).clamp(0.0, 1.0))
        })
        .fold(0.0, f32::max)
}

/// Whether trees and plants should keep off `p`.
pub fn occupied(p: Vec2, margin: f32) -> bool {
    let Some(venues) = crate::road::network().map(venues) else { return false };
    venues.grid.get(&key(p)).is_some_and(|cell| cell.iter().any(|&i| venues.plots[i as usize].holds(p, margin)))
}

/// Set out `plot` if it fits: on dry land, keeping `reach` from any circuit
/// and off other paths and plots, on ground that doesn't vary by over `relief`.
fn claim(network: &RoadNetwork, plots: &mut Vec<Plot>, mut plot: Plot, reach: f32, relief: f32) -> bool {
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    let mut lowest = f32::MAX;
    let rows = (plot.depth / 8.0).ceil().max(1.0) as i32;
    let columns = (plot.half / 8.0).ceil().max(1.0) as i32;
    // Corners first: on a bend it's the ends that come near the track, so
    // most plots that won't fit are turned away in a sample or two
    let mut spots: Vec<(i32, i32)> = vec![(0, -columns), (0, columns), (rows, -columns), (rows, columns)];
    spots.extend((0..=rows).flat_map(|r| (-columns..=columns).map(move |c| (r, c))).filter(|&(r, c)| !((r == 0 || r == rows) && c.abs() == columns)));
    for (r, c) in spots {
        // A little past the ends, which reach out over the edge
        let u = c as f32 / columns as f32 * (plot.half + 1.0);
        let v = r as f32 / rows as f32 * plot.depth;
        let p = plot.front + plot.along * u + plot.out * v;
        match network.nearest(p.x, p.y) {
            Some(s) if s.kind == PathKind::Circuit && s.dist < reach => return false,
            Some(s) if s.kind != PathKind::Circuit && s.dist < s.half_width() + 8.0 => return false,
            _ => {}
        }
        if terrain::raw_height(p.x, p.y) < terrain::WATER_LEVEL + 2.0 || plots.iter().any(|other| other.holds(p, 3.0)) {
            return false;
        }
        let h = terrain::terrain_height(p.x, p.y);
        low = low.min(h);
        high = high.max(h);
        if high - low > relief {
            return false;
        }
    }
    // It fits: rest it on the lowest of the ground as drawn, the pit lane's
    // included (the pit wall stands out on it)
    let lane = if plot.kind == Kind::Pits { plot.offset - PIT_WALL + 2.0 } else { 0.0 };
    for r in -1..=rows {
        for c in -columns..=columns {
            let v = if r < 0 { -lane } else { r as f32 / rows as f32 * plot.depth };
            let p = plot.front + plot.along * (c as f32 / columns as f32 * (plot.half + 1.0)) + plot.out * v;
            lowest = lowest.min(terrain::settled(p.x, p.y));
        }
    }
    plot.base = lowest.min(ground(plot.middle(), plot.half.min(plot.depth * 0.5))) - 0.1;
    plots.push(plot);
    true
}

/// Everything round circuit number `circuit`.
fn lay_out(network: &RoadNetwork, path: &RoadPath, circuit: usize, plots: &mut Vec<Plot>) {
    let n = path.points.len();
    let at = |i: isize| i.rem_euclid(n as isize) as usize;
    let centre = path.points.iter().copied().sum::<Vec2>() / n as f32;
    let infield = side(path, 0).dot(centre - path.points[0]).signum();
    let seed = circuit as f32 * 7.3 + 1.0;

    // The pits, on the infield of the start straight if they'll go: as long
    // as they can be, as near the track, as near the line
    let mut pit_side = None;
    // Out either way from just past the line, as far as a quarter of a lap
    let spots: Vec<isize> = (0..(n / 40) as isize).map(|k| if k % 2 == 0 { 24 + k * 10 } else { 24 - (k + 1) * 10 }).collect();
    'pits: for relief in [STAND_RELIEF, STAND_RELIEF * 1.5] {
        for half in PIT_HALVES {
            for front in PIT_FRONTS {
                for s in [infield, -infield] {
                    for &i in &spots {
                        let i = at(i);
                        let out = side(path, i) * s;
                        let plot = Plot { offset: front, ..Plot::new(Kind::Pits, path.points[i] + out * front, path.tangent(i), out, half, 62.0, seed) };
                        if claim(network, plots, plot, 56.0, relief) {
                            pit_side = Some(s);
                            break 'pits;
                        }
                    }
                }
            }
        }
    }

    // Stands along the start straight, the far side from the pits first
    for s in [-pit_side.unwrap_or(infield), pit_side.unwrap_or(infield)] {
        for k in -4..=6_isize {
            let i = at(k * 20 - 2);
            let out = side(path, i) * s;
            let depth = MAIN_STAND_ROWS as f32 * ROW_DEPTH + 6.0;
            let plot = Plot::new(Kind::Stand(MAIN_STAND_ROWS), path.points[i] + out * STAND_FRONT, path.tangent(i), out, STAND_HALF, depth, seed + k as f32);
            claim(network, plots, plot, STAND_FRONT - 7.0, STAND_RELIEF);
        }
    }

    // Stands round the outside of the corners
    for (start, end, outside) in trackside::corners(network, path) {
        let i = (start + end) / 2 % n;
        let out = side(path, i) * outside;
        let depth = CORNER_STAND_ROWS as f32 * ROW_DEPTH + 6.0;
        let plot = Plot::new(Kind::Stand(CORNER_STAND_ROWS), path.points[i] + out * STAND_FRONT, path.tangent(i), out, STAND_HALF * 0.8, depth, seed + i as f32);
        claim(network, plots, plot, STAND_FRONT - 8.0, STAND_RELIEF);
    }

    // The funfair in the middle of the home circuit
    if circuit == 0 {
        let toward = (centre - path.points[0]).normalize_or_zero();
        let along = Vec2::new(toward.y, -toward.x);
        for reach in [0.0, 60.0, -60.0, 120.0, -120.0, 180.0] {
            if claim(network, plots, Plot::new(Kind::Fair, centre + toward * reach - toward * 60.0, along, toward, 80.0, 120.0, seed), 60.0, 16.0) {
                break;
            }
        }
    }

    // Fans camped round the rest of the lap, and the marshals who mind them
    for i in (10..n).step_by(CAMP_EVERY) {
        for s in [-1.0, 1.0] {
            let i = (i + if s > 0.0 { 0 } else { CAMP_EVERY / 2 }) % n;
            let pick = hash(i as f32 * 0.37 + seed, s);
            if pick > 0.72 {
                continue;
            }
            let out = side(path, i) * s;
            let plot = Plot::new(Kind::Camp, path.points[i] + out * STAND_FRONT, path.tangent(i), out, 17.0, 16.0, pick * 100.0);
            claim(network, plots, plot, STAND_FRONT - 7.0, CAMP_RELIEF);
        }
    }
    for (k, i) in (6..n).step_by(MARSHAL_EVERY).enumerate() {
        let s = if k % 2 == 0 { 1.0 } else { -1.0 };
        let out = side(path, i) * s;
        let plot = Plot::new(Kind::Marshals, path.points[i] + out * 53.0, path.tangent(i), out, 3.0, 5.0, i as f32);
        claim(network, plots, plot, 50.0, 6.0);
    }
}

/// Everything anchored in the chunk `inside` accepts.
pub fn build(b: &mut Builder, network: &RoadNetwork, inside: &impl Fn(Vec2) -> bool) {
    for plot in venues(network).plots.iter().filter(|p| inside(p.front)) {
        b.tag(match plot.kind {
            Kind::Pits => "pits",
            Kind::Stand(_) => "stands",
            Kind::Camp => "camps",
            Kind::Marshals => "marshals",
            Kind::Fair => "fair",
        });
        match plot.kind {
            Kind::Pits => pits(b, plot),
            Kind::Stand(rows) => stand(b, plot, rows),
            Kind::Camp => camp(b, plot),
            Kind::Marshals => marshals(b, plot),
            Kind::Fair => fair(b, plot),
        }
    }
}

/// The two-tone cloth of a flag from a pole top, a little bigger toward the
/// edge that's free to flap.
pub fn flag(b: &mut Builder, top: Vec3, dir: Vec2, length: f32, height: f32, color: [f32; 3]) {
    let d = vec3(dir.x, 0.0, dir.y);
    let layer = std::mem::replace(&mut b.layer, Layer::Flag);
    for k in 0..4 {
        let (f0, f1) = (k as f32 / 4.0, (k + 1) as f32 / 4.0);
        let (p0, p1) = (top + d * length * f0, top + d * length * f1);
        let shade = if k % 2 == 0 { 1.0 } else { 0.88 };
        b.cloth([p0, p1, p1 - Vec3::Y * height, p0 - Vec3::Y * height], [f0, f1, f1, f0], color.map(|c| c * shade));
    }
    b.layer = layer;
}

/// A flag on its own pole.
pub fn flagpole(b: &mut Builder, foot: Vec3, height: f32, dir: Vec2, color: [f32; 3]) {
    b.limb(foot, foot + Vec3::Y * height, 0.14, 0.08, 4, 0.0, STEEL);
    flag(b, foot + Vec3::Y * height, dir, 4.5, 2.6, color);
}

/// Somebody standing at `at`, facing `facing`, in colours and with a rhythm
/// picked by `seed`. `keen` raises their arms the higher it is.
pub fn person(b: &mut Builder, at: Vec3, facing: Vec2, seed: f32, keen: f32) {
    let pick = |list: usize, salt: f32| ((hash(seed, salt) * list as f32) as usize).min(list - 1);
    let (shirt, skin, hair) = (SHIRTS[pick(SHIRTS.len(), 1.0)], SKINS[pick(SKINS.len(), 2.0)], HAIR[pick(HAIR.len(), 3.0)]);
    let phase = hash(seed, 4.0) * 0.49;
    let tone = |c: [f32; 3], arm: bool| [c[0], c[1], c[2], phase + if arm { 0.5 } else { 0.0 }];
    let layer = std::mem::replace(&mut b.layer, Layer::Crowd);
    let first = b.crowd_verts.len();
    let planting = std::mem::replace(&mut b.planting, false);
    let height = 1.25 + 0.2 * hash(seed, 5.0);
    let right = vec3(facing.y, 0.0, -facing.x);
    let front = vec3(facing.x, 0.0, facing.y);
    // Only the faces that show from the track: no backs (the row behind
    // hides them), no top to the body (the head does)
    let box_faces = |b: &mut Builder, base: Vec3, half_w: f32, half_d: f32, h: f32, side: [f32; 4], top: Option<[f32; 4]>| {
        let c = |x: f32, y: f32, z: f32| base + right * x + Vec3::Y * y + front * z;
        b.quad(c(-half_w, 0.0, half_d), c(half_w, 0.0, half_d), c(half_w, h, half_d), c(-half_w, h, half_d), side);
        for s in [-1.0, 1.0] {
            let x = s * half_w;
            b.quad(c(x, 0.0, -half_d), c(x, 0.0, half_d), c(x, h, half_d), c(x, h, -half_d), side);
        }
        if let Some(top) = top {
            b.quad(c(-half_w, h, -half_d), c(half_w, h, -half_d), c(half_w, h, half_d), c(-half_w, h, half_d), top);
        }
    };
    box_faces(b, at, 0.36, 0.22, height, tone(shirt, false), None);
    let head = at + Vec3::Y * height;
    box_faces(b, head, 0.2, 0.2, 0.42, tone(skin, false), Some(tone(hair, false)));
    for s in [-1.0, 1.0] {
        let shoulder = head - Vec3::Y * 0.12 + right * s * 0.42;
        let hand = shoulder + vec3(right.x * s * 0.1, 0.45 + 0.4 * keen, right.z * s * 0.1);
        let wide = right * 0.11;
        b.quad(shoulder - wide, shoulder + wide, hand + wide, hand - wide, tone(if s > 0.0 { skin } else { shirt }, true));
    }
    b.layer = layer;
    b.planting = planting;
    // Modelled a unit to the metre; the world is two units to it
    for v in &mut b.crowd_verts[first..] {
        let p = at + (Vec3::from(v.pos) - at) * crate::props::PERSON_SCALE;
        v.pos = p.into();
    }
}

/// A grandstand: rows rising from a wall at the front to a roofed back wall.
fn stand(b: &mut Builder, plot: &Plot, rows: usize) {
    let (seat_a, seat_b, roof) = LIVERIES[(hash(plot.seed, 0.7) * LIVERIES.len() as f32) as usize % LIVERIES.len()];
    let sponsor = SPONSORS[(hash(plot.seed, 0.3) * SPONSORS.len() as f32) as usize % SPONSORS.len()];
    let top = STAND_WALL + rows as f32 * ROW_RISE;
    let back = 1.0 + rows as f32 * ROW_DEPTH;
    let half = plot.half;

    // The wall at the front, with the sponsor along it
    plot.solid(b, 0.0, 0.5, 0.0, half, 0.5, STAND_WALL, CONCRETE, CONCRETE_DARK);
    let bays = (half / 14.0).ceil() as i32;
    for k in -bays + 1..bays {
        let u = k as f32 * half / bays as f32;
        let name = if k.rem_euclid(2) == 0 { sponsor } else { SPONSORS[(k.unsigned_abs() as usize * 3 + plot.seed as usize) % SPONSORS.len()] };
        let c = plot.at(u, -0.06, 0.0);
        trackside::panel(b, vec3(c.x, plot.base + 1.5, c.z), -plot.out, half / bays as f32 - 0.5, 1.2, name);
    }

    // The rows, then the fans on them
    for row in 0..rows {
        let v = 1.0 + (row as f32 + 0.5) * ROW_DEPTH;
        let height = STAND_WALL + (row + 1) as f32 * ROW_RISE;
        let tone = if row % 3 == 2 { seat_b } else { seat_a };
        plot.block(b, 0.0, v, 0.0, half, ROW_DEPTH * 0.5, height, CONCRETE_DARK, tone);
        // A crush barrier every few rows
        if row % 3 == 2 {
            let (l, r) = (plot.at(-half, v + ROW_DEPTH * 0.4, height + 1.0), plot.at(half, v + ROW_DEPTH * 0.4, height + 1.0));
            b.limb(l, r, 0.07, 0.07, 4, 0.0, STEEL);
        }
    }
    // Aisles of pale steps through the seats
    let source = mounted(b);
    let aisles = (half / 16.0) as i32;
    for k in -aisles..=aisles {
        let u = k as f32 * 16.0;
        for row in 0..rows {
            let v = 1.0 + (row as f32 + 0.5) * ROW_DEPTH;
            let height = STAND_WALL + (row + 1) as f32 * ROW_RISE;
            plot.block(b, u, v, height, 0.9, ROW_DEPTH * 0.5, 0.05, PAINT, PAINT);
        }
    }
    b.tag(source);
    for row in 0..rows {
        let v = 1.0 + (row as f32 + 0.45) * ROW_DEPTH;
        let height = STAND_WALL + (row + 1) as f32 * ROW_RISE;
        let seats = (half * 2.0 / SEAT) as i32;
        for k in 0..seats {
            let u = -half + (k as f32 + 0.5) * SEAT;
            let seed = plot.seed * 3.1 + row as f32 * 17.3 + k as f32 * 0.91;
            let fringe = (u / 16.0 - (u / 16.0).round()).abs() * 16.0 < 1.6;
            if fringe || hash(seed, 9.0) < 0.1 {
                continue;
            }
            let jog = plot.along * (hash(seed, 7.0) - 0.5) * 0.5;
            person(b, plot.at(u, v, height) + vec3(jog.x, 0.0, jog.y), -plot.out, seed, hash(seed, 8.0));
        }
    }

    // Back wall, ends, and the roof
    plot.solid(b, 0.0, back + 0.4, 0.0, half + 0.4, 0.4, top + 6.5, CONCRETE, CONCRETE_DARK);
    // The ends step up with the rows, faced both sides
    for s in [-1.0_f32, 1.0] {
        for face in [-0.2, 0.2] {
            let u = s * (half + 0.2) + face;
            let (front_low, back_low) = (plot.at(u, 0.0, -1.0), plot.at(u, back + 0.8, -1.0));
            let (front_high, back_high) = (plot.at(u, 0.0, STAND_WALL + 0.6), plot.at(u, back + 0.8, top + 1.0));
            b.quad(front_low, back_low, back_high, front_high, CONCRETE);
        }
        b.quad(plot.at(s * (half + 0.4), 0.0, STAND_WALL + 0.6), plot.at(s * half, 0.0, STAND_WALL + 0.6), plot.at(s * half, back + 0.8, top + 1.0), plot.at(s * (half + 0.4), back + 0.8, top + 1.0), CONCRETE_DARK);
    }
    let reach = back * 0.86;
    let (eave, ridge) = (top + 5.0, top + 7.4);
    let corner = |u: f32, v: f32, y: f32| plot.at(u, v, y);
    let (front_l, front_r, back_l, back_r) = (corner(-half - 1.0, back - reach, eave), corner(half + 1.0, back - reach, eave), corner(-half - 1.0, back + 1.0, ridge), corner(half + 1.0, back + 1.0, ridge));
    b.quad(front_l, front_r, back_r, back_l, roof);
    let lip = Vec3::Y * 0.5;
    b.quad(front_l - lip, front_r - lip, front_r, front_l, PAINT);
    b.quad(front_l - lip, front_r - lip, back_r - lip, back_l - lip, SHADE);
    // Columns behind the rows, ties out to the roof's front edge
    let columns = (half / 18.0).ceil() as i32;
    for k in -columns..=columns {
        let u = k as f32 * half / columns as f32;
        // Up from the ground through the rows, so it stands on something
        b.limb(plot.at(u, back - reach * 0.5, 0.0), plot.at(u, back - reach * 0.5, eave + 1.1), 0.16, 0.16, 4, 0.0, STEEL);
        b.limb(plot.at(u, back + 0.8, ridge + 2.6), plot.at(u, back - reach, eave + 0.05), 0.09, 0.09, 4, 0.0, STEEL);
        b.limb(plot.at(u, back + 0.8, 0.0), plot.at(u, back + 0.8, ridge + 2.6), 0.25, 0.2, 4, 0.0, STEEL);
    }
    // Flags along the roof's back edge
    let source = mounted(b);
    let wind = -plot.along;
    for k in -3..=3 {
        let c = BUNTING[(k + 3) as usize % BUNTING.len()];
        flagpole(b, plot.at(k as f32 * half / 3.5, back + 1.0, ridge), 4.5, wind, c);
    }
    b.tag(source);
    // The stand's name in lights along its back, under the roof
    let source = mounted(b);
    b.glowing = true;
    let strip = plot.at(0.0, back - reach + 0.1, eave - 0.5);
    b.block(strip, 0.08, 0.14, half * 0.96, plot.yaw(), WARM_LIGHT, WARM_LIGHT);
    b.glowing = false;
    b.tag(source);
}

/// One garage door: the dark inside, framed in the team's colour.
fn garage(b: &mut Builder, plot: &Plot, u: f32, team: [f32; 4], number: usize) {
    let half = GARAGE_WIDTH * 0.5 - 0.5;
    let at = |x: f32, y: f32| plot.at(u + x, -0.06, y);
    b.quad(at(-half, 0.0), at(half, 0.0), at(half, 4.6), at(-half, 4.6), SHADE);
    b.quad(at(-half - 0.4, 4.6), at(half + 0.4, 4.6), at(half + 0.4, 6.2), at(-half - 0.4, 6.2), team);
    for s in [-1.0, 1.0] {
        let x = s * (half + 0.2);
        b.quad(at(x - 0.2, 0.0), at(x + 0.2, 0.0), at(x + 0.2, 4.6), at(x - 0.2, 4.6), team);
    }
    let label = number.to_string();
    let px = 0.17;
    let width = crate::roadside::text_units(&label) * px;
    // Reading left to right for someone on the track looking at it
    let right = vec3(plot.out.y, 0.0, -plot.out.x);
    let origin = at(0.0, 5.75) - right * width * 0.5 + vec3(-plot.out.x, 0.0, -plot.out.y) * 0.03;
    crate::roadside::lettering(b, origin, right, -Vec3::Y, px, &label, PAINT);
}

/// A race car parked at (u, v), nose out, in low-poly blocks.
fn parked_car(b: &mut Builder, plot: &Plot, u: f32, v: f32, team: [f32; 4]) {
    let source = mounted(b);
    let dark = [team[0] * 0.5, team[1] * 0.5, team[2] * 0.5, 1.0];
    plot.block(b, u, v, 0.8, 1.8, 3.8, 1.0, team, team);
    plot.block(b, u, v + 0.6, 1.8, 1.4, 1.7, 0.8, SHADE, dark);
    plot.block(b, u, v + 3.4, 2.0, 1.9, 0.4, 0.2, dark, dark);
    for (x, w) in [(-1.8, -2.3), (1.8, -2.3), (-1.8, 2.3), (1.8, 2.3)] {
        let (a, c) = (plot.at(u + x - 0.5, v + w, 0.8), plot.at(u + x + 0.5, v + w, 0.8));
        b.limb(a, c, 0.8, 0.8, 8, 0.0, SHADE);
    }
    b.tag(source);
}

/// Mark what follows as resting on something other than the ground (wheels,
/// a platform), for `ground_audit`. Returns what to `tag` back afterwards.
fn mounted(b: &mut Builder) -> &'static str {
    #[cfg(test)]
    {
        let was = b.source;
        b.tag("deck");
        was
    }
    #[cfg(not(test))]
    {
        let _ = b;
        ""
    }
}

/// The pit lane, garages with hospitality above, the pit wall, a control
/// tower, and the transporters in the paddock behind.
fn pits(b: &mut Builder, plot: &Plot) {
    let half = plot.half;
    let wall_v = PIT_WALL - plot.offset;
    let lane = |u: f32, v: f32| {
        let p = plot.at(u, v, 0.0);
        vec3(p.x, trackside::surface(Vec2::new(p.x, p.z)), p.z)
    };

    // The lane itself, its edge line, and the bays' boxes
    let steps = (half / 10.0) as i32;
    for k in -steps..steps {
        let (u0, u1) = (k as f32 * 10.0, (k + 1) as f32 * 10.0);
        let vs = [wall_v + 1.2, wall_v * 0.66, wall_v * 0.33, 0.0];
        for w in vs.windows(2) {
            b.quad(lane(u0, w[0]), lane(u1, w[0]), lane(u1, w[1]), lane(u0, w[1]), TARMAC);
        }
        let line = |u: f32, v: f32| lane(u, v) + Vec3::Y * 0.04;
        b.quad(line(u0, -9.2), line(u1, -9.2), line(u1, -8.8), line(u0, -8.8), PAINT);
    }

    // The pit wall, stripes at its ends, with crews' stands along it
    let wall_half = half - 10.0;
    plot.solid(b, 0.0, wall_v, 0.0, wall_half, 0.5, 2.2, PAINT, CONCRETE_DARK);
    for s in [-1.0, 1.0] {
        for k in 0..4 {
            let tone = if k % 2 == 0 { [0.85, 0.12, 0.1, 1.0] } else { PAINT };
            plot.block(b, s * (wall_half - 1.5 - k as f32 * 3.0), wall_v, 0.0, 1.5, 0.56, 2.24, tone, tone);
        }
    }
    let teams = (half * 2.0 / GARAGE_WIDTH) as i32;
    for k in 0..teams {
        let u = -half + (k as f32 + 0.5) * GARAGE_WIDTH;
        let team = TEAMS[k as usize % TEAMS.len()];
        garage(b, plot, u, team, k as usize + 1);
        // A car being seen to, with its crew round it
        let seed = plot.seed + k as f32 * 3.3;
        if k % 2 == 0 {
            parked_car(b, plot, u, -3.8, team);
            for (x, v) in [(-3.3, -5.0), (3.3, -3.0), (0.0, -8.2), (-2.5, -1.0)] {
                person(b, plot.at(u + x, v, 0.0), -plot.out, seed + x, 0.1);
            }
        }
        if k % 3 == 0 {
            let source = mounted(b);
            let at = plot.at(u, wall_v + 1.6, 0.0);
            plot.block(b, u, wall_v + 1.6, 0.0, 3.2, 1.1, 0.6, STEEL, STEEL);
            plot.block(b, u, wall_v + 1.2, 3.8, 3.2, 1.6, 0.2, team, team);
            for x in [-1.6, 1.6] {
                b.limb(plot.at(u + x * 1.5, wall_v + 1.6, 0.6), plot.at(u + x * 1.5, wall_v + 1.6, 3.8), 0.06, 0.06, 4, 0.0, STEEL);
                person(b, at + vec3(plot.along.x, 0.0, plot.along.y) * x + Vec3::Y * 0.6, -plot.out, seed + x * 1.7, 0.3);
            }
            b.tag(source);
        }
    }

    // The building: garages, then hospitality set back behind a terrace
    let deep = BUILDING_DEPTH * 0.5;
    plot.solid(b, 0.0, deep, 0.0, half, deep, 7.0, CONCRETE, CONCRETE_DARK);
    // Everything above stands on the garages
    let ground_floor = mounted(b);
    plot.block(b, 0.0, deep + 2.5, 7.0, half, deep - 2.5, 5.4, GLASS, CONCRETE);
    plot.block(b, 0.0, deep + 1.0, 12.4, half + 1.0, deep + 1.0, 0.8, PAINT, CONCRETE_DARK);
    // Lit windows along the glass, mullions over them
    b.glowing = true;
    plot.block(b, 0.0, 4.92, 8.0, half - 2.0, 0.05, 2.6, WARM_LIGHT, WARM_LIGHT);
    b.glowing = false;
    for k in -((half / 4.0) as i32)..=(half / 4.0) as i32 {
        plot.block(b, k as f32 * 4.0, 4.96, 7.0, 0.14, 0.05, 5.4, CONCRETE, CONCRETE);
    }
    // The terrace: rail, guests, and bunting overhead
    b.limb(plot.at(-half, 0.3, 8.1), plot.at(half, 0.3, 8.1), 0.07, 0.07, 4, 0.0, PAINT);
    for k in (0..(half * 2.0 / 2.6) as i32).filter(|k| hash(plot.seed, *k as f32) < 0.75) {
        let u = -half + (k as f32 + 0.5) * 2.6;
        person(b, plot.at(u, 1.6 + hash(u, 2.0) * 2.2, 7.0), -plot.out, plot.seed + u, 0.6);
    }
    let source = mounted(b);
    let sponsor = SPONSORS[(plot.seed as usize) % SPONSORS.len()];
    trackside::panel(b, plot.at(0.0, 2.25, 15.4), -plot.out, half * 0.6, 2.2, sponsor);
    plot.block(b, 0.0, 2.6, 13.2, half * 0.6, 0.3, 4.4, STEEL, STEEL);

    b.tag(ground_floor);
    // The control tower at the far end, with the timing board on its side
    let tower = half - 11.0;
    let ground_floor = mounted(b);
    plot.solid(b, tower, 14.0, 13.2, 9.0, 10.0, 11.0, CONCRETE, CONCRETE_DARK);
    plot.block(b, tower, 14.0, 24.2, 11.0, 12.0, 5.0, GLASS, CONCRETE);
    plot.block(b, tower, 14.0, 29.2, 12.0, 13.0, 0.8, PAINT, CONCRETE_DARK);
    b.limb(plot.at(tower, 14.0, 30.0), plot.at(tower, 14.0, 37.0), 0.2, 0.08, 4, 0.0, STEEL);
    b.glowing = true;
    plot.block(b, tower, 1.96, 25.0, 10.6, 0.05, 3.2, WARM_LIGHT, WARM_LIGHT);
    b.glowing = false;
    flagpole(b, plot.at(-half + 4.0, 3.0, 13.2), 12.0, plot.along, [0.1, 0.1, 0.12]);
    flagpole(b, plot.at(-half + 16.0, 3.0, 13.2), 12.0, plot.along, [0.9, 0.9, 0.86]);
    b.tag(source);
    b.tag(ground_floor);

    // The paddock behind: a transporter for each team, awnings beside them
    let v = BUILDING_DEPTH + 13.0;
    // As many as the plot is long: each takes 29 along, its awning reaching 10 past it
    let transporters = ((half * 2.0 - 26.0) / 29.0).floor().max(0.0) as usize;
    for k in 0..transporters {
        let u = -half + 16.0 + k as f32 * 29.0;
        let team = TEAMS[k % TEAMS.len()];
        let source = mounted(b);
        plot.solid(b, u, v, 1.0, 3.0, 11.0, 7.0, team, PAINT);
        plot.block(b, u, v - 13.2, 1.0, 2.9, 2.2, 4.2, PAINT, team);
        plot.block(b, u, v - 15.0, 2.8, 2.5, 0.5, 2.0, GLASS, GLASS);
        for x in [-2.8, 2.8] {
            for w in [-13.6, -8.0, 7.0, 9.4] {
                let (a, c) = (plot.at(u + x - 0.5, v + w, 1.0), plot.at(u + x + 0.5, v + w, 1.0));
                b.limb(a, c, 0.95, 0.95, 8, 0.0, SHADE);
            }
        }
        plot.block(b, u + 6.5, v, 4.2, 3.2, 8.0, 0.2, team, team);
        b.tag(source);
        for (x, w) in [(9.4, -8.0), (9.4, 8.0)] {
            b.limb(plot.at(u + x, v + w, 0.0), plot.at(u + x, v + w, 4.2), 0.08, 0.08, 4, 0.0, STEEL);
        }
        person(b, plot.at(u + 6.0, v - 2.0, 0.0), plot.along, plot.seed + k as f32, 0.0);
        person(b, plot.at(u + 7.0, v + 2.5, 0.0), -plot.along, plot.seed - k as f32, 0.0);
    }
}

/// Fans camped on the grass by the fence: a gazebo, a car or a camper van,
/// folk on their feet, and their colours flying.
fn camp(b: &mut Builder, plot: &Plot) {
    let seed = plot.seed;
    let tone = BUNTING[(seed as usize) % BUNTING.len()];
    let awning = [tone[0], tone[1], tone[2], 1.0];
    let ground_at = |u: f32, v: f32| {
        let p = plot.at(u, v, 0.0);
        vec3(p.x, terrain::settled(p.x, p.z), p.z)
    };
    // The gazebo: four poles under a striped peak
    if hash(seed, 1.0) < 0.75 {
        let (u, v) = (-7.0 + hash(seed, 2.0) * 8.0, 8.0);
        let peak = ground_at(u, v) + Vec3::Y * 5.0;
        let c = [(-3.5, -3.5), (3.5, -3.5), (3.5, 3.5), (-3.5, 3.5)].map(|(x, w)| ground_at(u + x, v + w));
        for p in c {
            b.limb(p, vec3(p.x, peak.y - 1.6, p.z), 0.07, 0.07, 4, 0.0, STEEL);
        }
        for k in 0..4 {
            let (p, q) = (c[k], c[(k + 1) % 4]);
            let lift = |o: Vec3| vec3(o.x, peak.y - 1.6, o.z);
            b.tri(lift(p), lift(q), peak, if k % 2 == 0 { awning } else { PAINT });
        }
    }
    // A van, or a car with its boot open
    let body = TEAMS[(seed * 1.7) as usize % TEAMS.len()];
    if hash(seed, 3.0) < 0.5 {
        let u = 9.0 - hash(seed, 4.0) * 3.0;
        let foot = ground_at(u, 11.0);
        let yaw = plot.yaw();
        let source = mounted(b);
        b.solid_block(foot + Vec3::Y * 0.6, 1.9, 4.2, 4.2, yaw, PAINT, PAINT);
        b.block(foot + Vec3::Y * 2.8, 1.94, 1.1, 2.6, yaw, GLASS, GLASS);
        b.block(foot + Vec3::Y * 0.6, 1.96, 1.0, 4.26, yaw, body, body);
        // On its wheels
        let across = vec3(plot.out.x, 0.0, plot.out.y);
        let along = vec3(plot.along.x, 0.0, plot.along.y);
        for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let hub = foot + along * x * 1.75 + across * z * 2.8 + Vec3::Y * 0.6;
            b.limb(hub - along * 0.35, hub + along * 0.35, 0.62, 0.62, 8, 0.0, SHADE);
        }
        b.tag(source);
    }
    // Their colours on a tall pole
    flagpole(b, ground_at(12.0 - hash(seed, 5.0) * 24.0, 2.0), 9.0, plot.along, tone);
    // Fans at the fence, and milling about behind
    let crowd = 6 + (hash(seed, 6.0) * 10.0) as i32;
    for k in 0..crowd {
        let s = seed + k as f32 * 1.3;
        let (u, v) = ((hash(s, 7.0) - 0.5) * plot.half * 1.9, if k % 3 == 0 { 2.0 + hash(s, 8.0) * 10.0 } else { 0.8 + hash(s, 8.0) * 2.0 });
        person(b, ground_at(u, v), -plot.out, s, hash(s, 9.0));
    }
}

/// A marshals' post: a raised box, its crew, a flag in hand.
fn marshals(b: &mut Builder, plot: &Plot) {
    let foot = plot.at(0.0, 2.2, 0.0);
    plot.block(b, 0.0, 2.2, 0.0, 2.4, 1.8, 1.6, [0.9, 0.5, 0.1, 1.0], CONCRETE);
    let source = mounted(b);
    for (x, v) in [(-2.2, 0.6), (2.2, 0.6), (-2.2, 3.8), (2.2, 3.8)] {
        b.limb(plot.at(x, v, 1.6), plot.at(x, v, 4.8), 0.07, 0.07, 4, 0.0, STEEL);
    }
    plot.block(b, 0.0, 2.2, 4.8, 2.7, 2.2, 0.16, [0.9, 0.5, 0.1, 1.0], PAINT);
    let up = Vec3::Y * 1.6;
    person(b, foot + up + vec3(plot.along.x, 0.0, plot.along.y) * 1.0, -plot.out, plot.seed, 1.0);
    person(b, foot + up - vec3(plot.along.x, 0.0, plot.along.y) * 1.0, -plot.out, plot.seed + 3.0, 0.2);
    flagpole(b, plot.at(-2.0, 0.4, 1.6), 3.6, plot.along, [0.15, 0.6, 0.25]);
    b.tag(source);
}

/// A striped tent with a pointed top.
fn tent(b: &mut Builder, at: Vec3, radius: f32, height: f32, tones: [[f32; 4]; 2]) {
    const SIDES: usize = 10;
    let ring = |k: usize, r: f32, y: f32| {
        let a = std::f32::consts::TAU * k as f32 / SIDES as f32;
        at + vec3(a.cos() * r, y, a.sin() * r)
    };
    for k in 0..SIDES {
        let tone = tones[k % 2];
        b.quad(ring(k, radius, 0.0), ring(k + 1, radius, 0.0), ring(k + 1, radius, height * 0.5), ring(k, radius, height * 0.5), tone);
        b.tri(ring(k, radius * 1.15, height * 0.45), ring(k + 1, radius * 1.15, height * 0.45), at + Vec3::Y * height, tones[(k + 1) % 2]);
    }
}

/// Where the funfair's big wheel stands, if there is one: its hub, and the
/// way its axle lies.
pub fn wheel_site(network: &RoadNetwork) -> Option<(Vec3, Vec3)> {
    venues(network).plots.iter().find(|p| p.kind == Kind::Fair).map(|plot| {
        (plot.at(0.0, 56.0, WHEEL_HUB), vec3(plot.out.x, 0.0, plot.out.y))
    })
}

/// How high the big wheel's hub stands, and its reach.
pub const WHEEL_HUB: f32 = 34.0;
pub const WHEEL_RADIUS: f32 = 30.0;

/// The funfair: stalls, tents, a helter-skelter and the big wheel's frame.
fn fair(b: &mut Builder, plot: &Plot) {
    // The wheel's two A-frames; the wheel itself turns (see `raceday`)
    for v in [51.5, 60.5] {
        for u in [-16.0, 16.0] {
            b.limb(plot.at(u, v, 0.0), plot.at(0.0, v, WHEEL_HUB), 0.9, 0.55, 6, 0.0, PAINT);
        }
        b.limb(plot.at(-8.0, v, WHEEL_HUB * 0.5), plot.at(8.0, v, WHEEL_HUB * 0.5), 0.3, 0.3, 4, 0.0, PAINT);
    }
    plot.solid(b, 0.0, 56.0, 0.0, 19.0, 6.5, 1.6, CONCRETE, CONCRETE_DARK);

    let reds = [[0.85, 0.14, 0.12, 1.0], PAINT];
    let blues = [[0.14, 0.36, 0.8, 1.0], [0.98, 0.84, 0.2, 1.0]];
    let greens = [[0.1, 0.55, 0.36, 1.0], PAINT];
    for (u, v, r, tones) in [(-54.0, 26.0, 11.0, reds), (52.0, 28.0, 13.0, blues), (-52.0, 88.0, 12.0, greens), (54.0, 92.0, 10.0, reds)] {
        let at = plot.at(u, v, 0.0);
        tent(b, at, r, r * 1.5, tones);
        b.solids.push(crate::collisions::Solid { centre: Vec2::new(at.x, at.z), half: Vec2::splat(r), yaw: 0.0, top: at.y + r, round: true });
        flagpole(b, at + Vec3::Y * r * 1.5, 3.5, plot.along, [0.95, 0.8, 0.15]);
    }
    // A row of stalls, awnings out, and the helter-skelter
    for k in -3..=3 {
        let u = k as f32 * 13.0;
        let tone = [TEAMS[(k + 3) as usize][0], TEAMS[(k + 3) as usize][1], TEAMS[(k + 3) as usize][2], 1.0];
        plot.solid(b, u, 16.0, 0.0, 5.5, 3.0, 3.6, PAINT, tone);
        plot.block(b, u, 12.6, 3.0, 5.8, 1.2, 0.2, tone, tone);
        person(b, plot.at(u + 2.0, 10.4, 0.0), plot.out, plot.seed + u, 0.2);
    }
    let foot = plot.at(30.0, 66.0, 0.0);
    b.column(foot, 4.2, 2.2, 24.0, 8, [0.9, 0.86, 0.75, 1.0]);
    let turns = 34;
    for k in 0..turns {
        let (a0, a1) = (k as f32 * 0.55, (k + 1) as f32 * 0.55);
        let (y0, y1) = (24.0 * (1.0 - k as f32 / turns as f32), 24.0 * (1.0 - (k + 1) as f32 / turns as f32));
        let ring = |a: f32, r: f32, y: f32| foot + vec3(a.cos() * r, y, a.sin() * r);
        b.quad(ring(a0, 3.8, y0), ring(a1, 3.8, y1), ring(a1, 7.0, y1), ring(a0, 7.0, y0), if k % 4 < 2 { [0.86, 0.14, 0.12, 1.0] } else { PAINT });
    }
    // Everyone out enjoying it
    for k in 0..90 {
        let s = plot.seed + k as f32 * 0.77;
        let (u, v) = ((hash(s, 1.0) - 0.5) * 150.0, 22.0 + hash(s, 2.0) * 92.0);
        if (u.abs() < 22.0 && (v - 56.0).abs() < 10.0) || (u - 30.0).hypot(v - 66.0) < 9.0 {
            continue;
        }
        let p = plot.at(u, v, 0.0);
        let a = hash(s, 3.0) * std::f32::consts::TAU;
        person(b, vec3(p.x, terrain::settled(p.x, p.z), p.z), Vec2::from_angle(a), s, hash(s, 4.0) * 0.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(path: &RoadPath, plot: &Plot) -> bool {
        path.points.iter().step_by(2).any(|q| q.distance(plot.front) < PIT_FRONTS[2] + 10.0)
    }

    /// Dev aid: what each circuit got, and where the funfair is.
    #[test]
    #[ignore]
    fn venue_report() {
        let network = crate::sim_tests::network();
        let names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
        let venues = venues(network);
        for (k, path) in network.paths.iter().filter(|p| p.circuit()).enumerate() {
            let count = |f: &dyn Fn(Kind) -> bool| venues.plots.iter().filter(|p| near(path, p) && f(p.kind)).count();
            println!("{:18} pits {} stands {:2} (main {:2}) camps {:2} marshals {:2}", names[k], count(&|k| k == Kind::Pits),
                count(&|k| matches!(k, Kind::Stand(_))), count(&|k| k == Kind::Stand(MAIN_STAND_ROWS)), count(&|k| k == Kind::Camp),
                count(&|k| k == Kind::Marshals));
        }
        if let Some(plot) = venues.plots.iter().find(|p| p.kind == Kind::Fair) {
            println!("fair at {} facing {}", plot.middle(), plot.out);
        }
    }

    /// Every circuit gets its pits and stands.
    #[test]
    fn every_circuit_has_a_race_day() {
        let network = crate::sim_tests::network();
        let venues = venues(network);
        for (k, path) in network.paths.iter().filter(|p| p.circuit()).enumerate() {
            let pits = venues.plots.iter().filter(|p| near(path, p) && p.kind == Kind::Pits).count();
            let stands = venues.plots.iter().filter(|p| near(path, p) && matches!(p.kind, Kind::Stand(_))).count();
            assert!(pits == 1 && stands >= 3, "circuit {k}: {pits} pits, {stands} stands");
        }
    }
}
