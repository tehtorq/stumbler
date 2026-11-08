//! Places to stop: a lay-by every so often along the country roads (a
//! gravel pull-in off the road, a picnic table and a bin), and a table at
//! each viewpoint. By day, folk come out to them: a car parked up, a rug
//! and a hamper on the grass, the grown-ups sat in the sun while a child
//! and the dog chase each other round; at a viewpoint, someone at the
//! telescope admiring the view.
//!
//! The pull-ins and tables are scenery (built with the chunks); the
//! visitors come and go with the hours (see `Picnics`).

use std::sync::OnceLock;

use glam::{Quat, Vec2, Vec3, vec2, vec3};

use crate::car::CAR_COLORS;
use crate::landmarks::{self, Kind};
use crate::physics::{CarPhysics, WorldGround};
use crate::props::{hash, Builder, PERSON_SCALE};
use crate::road::{self, PathKind};
use crate::terrain::{self, terrain_height, WATER_LEVEL};
use crate::townsfolk::{Mesh, Pose};

/// A lay-by at most every this far along a road.
const LAYBY_EVERY: f32 = 1400.0;
/// The pull-in's half length, and how far it reaches out from the road's edge.
const PULL_IN: (f32, f32) = (26.0, 12.0);
/// Visitors are about inside this distance of the player, drawn out to the next.
const NEAR: f32 = 700.0;
const DRAW: f32 = 500.0;

const GRAVEL: [f32; 4] = [0.6, 0.57, 0.5, 1.0];
const GRAVEL_DARK: [f32; 4] = [0.52, 0.49, 0.43, 1.0];
const TIMBER: [f32; 4] = [0.55, 0.4, 0.25, 1.0];
const TIMBER_TOP: [f32; 4] = [0.62, 0.47, 0.3, 1.0];
const BIN: [f32; 4] = [0.2, 0.3, 0.22, 1.0];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Layby,
    Viewpoint,
}

pub struct Spot {
    pub place: Place,
    /// Where the rug goes down, and which way the view is.
    pub picnic: Vec2,
    pub view: Vec2,
    /// Where the car parks, and the way it faces.
    pub park: Vec2,
    pub park_dir: Vec2,
    /// For a lay-by: the road's edge at its middle, along the road, and out from it.
    pub edge: Vec2,
    pub along: Vec2,
    pub out: Vec2,
    /// At a viewpoint, where its coin telescope stands.
    pub scope: Option<Vec2>,
    pub seed: f32,
}

static SPOTS: OnceLock<Vec<Spot>> = OnceLock::new();

/// Every lay-by and viewpoint picnic spot (none until the roads are in).
pub fn spots() -> &'static [Spot] {
    match road::network() {
        Some(n) => SPOTS.get_or_init(|| site(n)),
        None => &[],
    }
}

fn site(network: &road::RoadNetwork) -> Vec<Spot> {
    let mut spots = Vec::new();
    let clear_of_paths = |q: Vec2, margin: f32| network.nearest(q.x, q.y).is_none_or(|s| s.dist > s.half_width() + margin);
    let flat = |q: Vec2, r: f32| {
        let h = terrain_height(q.x, q.y);
        (0..6).all(|k| {
            let d = Vec2::from_angle(k as f32 * 1.047) * r;
            (terrain_height(q.x + d.x, q.y + d.y) - h).abs() < r * 0.12
        }) && terrain::raw_height(q.x, q.y) > WATER_LEVEL + 2.0
    };
    // Lay-bys along the connecting roads, on straight and level stretches
    for path in network.paths.iter().filter(|p| p.kind == PathKind::Road) {
        let n = path.points.len();
        let step = (LAYBY_EVERY / road::SAMPLE_SPACING) as usize;
        let mut i = step / 2;
        while i + 4 < n {
            let here = i;
            i += step;
            let (p, t) = (path.points[here], path.tangent(here));
            let straight = path.tangent(here.saturating_sub(4)).dot(path.tangent((here + 4).min(n - 1))) > 0.99;
            if !straight {
                continue;
            }
            let hw = path.kind.half_width();
            // The side with the better view: lower ground beyond
            let side = [1.0, -1.0].into_iter().max_by(|a, b| {
                let beyond = |s: f32| -terrain_height(p.x - t.y * s * 120.0, p.y + t.x * s * 120.0);
                beyond(*a).total_cmp(&beyond(*b))
            }).unwrap();
            let out = vec2(-t.y, t.x) * side;
            let edge = p + out * hw;
            let picnic = edge + out * (PULL_IN.1 + 12.0);
            let seed = hash(p.x, p.y) * 1000.0;
            // Not raised up off the land, where a guard rail runs along the
            // edge (`props::raised_road_supports` puts one where it's 4 up)
            let reach = (PULL_IN.0 / road::SAMPLE_SPACING) as i32 + 3;
            let level = (-reach..=reach).all(|d| {
                let j = (here as i32 + d).clamp(0, n as i32 - 1) as usize;
                let q = path.points[j];
                path.heights[j] - terrain::raw_height(q.x, q.y) < 2.5
            });
            let ok = level && flat(edge + out * PULL_IN.1 * 0.5, PULL_IN.0) && flat(picnic, 10.0)
                && !crate::towns::occupied(picnic, 60.0) && crate::farms::at(picnic).is_none()
                && landmarks::all().iter().all(|l| l.pos.distance(picnic) > 200.0)
                && spots.iter().all(|s: &Spot| s.picnic.distance(picnic) > 600.0)
                // Nothing else crossing it: only this road near
                && network.paths.iter().filter(|q| q.kind != PathKind::Road || !std::ptr::eq(*q, path))
                    .all(|q| q.points.iter().step_by(4).all(|r| r.distance(picnic) > q.kind.half_width() + 50.0));
            if ok && clear_of_paths(picnic, 14.0) {
                let park = edge + out * (PULL_IN.1 * 0.55) + t * 8.0;
                spots.push(Spot { place: Place::Layby, picnic, view: out, park, park_dir: t, edge, along: t, out, scope: None, seed });
            }
        }
    }
    // A table at each viewpoint, the car left at the end of the trail
    for l in landmarks::all().iter().filter(|l| l.kind == Kind::Viewpoint) {
        let yaw = hash(l.pos.x, l.pos.y) * std::f32::consts::TAU;
        let r = Quat::from_rotation_y(yaw);
        let facing = r * vec3(0.0, 0.0, -1.0);
        let view = vec2(facing.x, facing.z).normalize_or_zero();
        let picnic = l.pos + view.perp() * 16.0 + view * 4.0;
        let trail_end = network.paths.iter().filter(|p| p.kind == PathKind::Trail)
            .flat_map(|p| p.points.iter().enumerate().map(move |(i, q)| (p, i, *q)))
            .min_by(|a, b| a.2.distance_squared(l.pos).total_cmp(&b.2.distance_squared(l.pos)));
        let Some((trail, i, end)) = trail_end.filter(|t| t.2.distance(l.pos) < 120.0) else { continue };
        let dir = trail.tangent(i);
        let dir = if (l.pos - end).dot(dir) < 0.0 { -dir } else { dir };
        let park = end - dir * 10.0 + dir.perp() * 7.0;
        // As `props` builds it
        let scope = l.pos + { let s = r * vec3(5.0, 0.0, -1.0); vec2(s.x, s.z) };
        spots.push(Spot { place: Place::Viewpoint, picnic, view, park, park_dir: dir, edge: end, along: dir, out: dir.perp(), scope: Some(scope), seed: l.seed as f32 % 1000.0 });
    }
    spots
}

/// The lay-bys' pull-ins and the picnic tables, for the chunk.
pub fn build(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    b.tag("picnics");
    for spot in spots().iter().filter(|s| inside(s.picnic)) {
        if spot.place == Place::Layby {
            pull_in(b, spot);
            let bin = spot.picnic - spot.view * 6.0 + spot.along * 9.0;
            let k = PERSON_SCALE;
            b.column(vec3(bin.x, terrain_height(bin.x, bin.y) - 0.1, bin.y), 0.3 * k, 0.32 * k, 1.0 * k, 8, BIN);
        }
        table(b, spot.picnic + spot.along * 7.0, spot.along);
    }
}

/// Gravel off the road's edge, ramped in at either end.
fn pull_in(b: &mut Builder, spot: &Spot) {
    let steps = 13;
    let at = |s: f32, o: f32| {
        let q = spot.edge + spot.along * s + spot.out * o;
        vec3(q.x, terrain_height(q.x, q.y) + 0.12, q.y)
    };
    for k in 0..steps {
        let (s0, s1) = (-PULL_IN.0 + 2.0 * PULL_IN.0 * k as f32 / steps as f32, -PULL_IN.0 + 2.0 * PULL_IN.0 * (k + 1) as f32 / steps as f32);
        // Tapered: full depth in the middle, nothing at the ends
        let depth = |s: f32| PULL_IN.1 * (1.0 - ((s.abs() - PULL_IN.0 * 0.5) / (PULL_IN.0 * 0.5)).clamp(0.0, 1.0));
        let (d0, d1) = (depth(s0), depth(s1));
        for j in 0..2 {
            let (f0, f1) = (j as f32 * 0.5, (j + 1) as f32 * 0.5);
            let tone = if (k + j) % 2 == 0 { GRAVEL } else { GRAVEL_DARK };
            b.quad(at(s0, -0.5 + d0 * f0), at(s1, -0.5 + d1 * f0), at(s1, -0.5 + d1 * f1), at(s0, -0.5 + d0 * f1), tone);
        }
    }
}

/// A wooden picnic table with its benches, long side along `along`.
fn table(b: &mut Builder, at: Vec2, along: Vec2) {
    let k = PERSON_SCALE;
    let y = terrain_height(at.x, at.y) - 0.05;
    let yaw = along.x.atan2(along.y);
    let across = along.perp();
    let p = |o: f32| { let q = at + across * o * k; vec3(q.x, y, q.y) };
    // Legs, top, and a bench either side
    for s in [-0.7, 0.7] {
        let leg = at + along * s * k;
        b.block(vec3(leg.x, y, leg.y), 0.75 * k, 0.75 * k, 0.06 * k, yaw, TIMBER, TIMBER);
    }
    b.block(p(0.0) + Vec3::Y * 0.72 * k, 0.4 * k, 0.05 * k, 0.9 * k, yaw, TIMBER, TIMBER_TOP);
    for s in [-1.0, 1.0] {
        b.block(p(s * 0.7) + Vec3::Y * 0.42 * k, 0.15 * k, 0.04 * k, 0.9 * k, yaw, TIMBER, TIMBER_TOP);
    }
    b.post(vec3(at.x, y, at.y), 1.4 * k, 0.8 * k);
}

/// Who's out at the picnic spots near the player just now.
#[derive(Default)]
pub struct Picnics {
    /// Spots with visitors, each with its parked car and the car's paint.
    out: Vec<(usize, CarPhysics, usize)>,
}

/// Whether there's anyone at spot `seed` at `clock` (0..1 from midnight):
/// each spot has its own visitors' hours, most days.
fn visited(seed: f32, clock: f32) -> bool {
    let h = clock * 24.0;
    let arrive = 9.0 + 4.0 * hash(seed, 1.0);
    let stay = 2.5 + 4.0 * hash(seed, 2.0);
    hash(seed, 3.0) < 0.75 && (arrive..arrive + stay).contains(&h)
}

impl Picnics {
    /// `rain` (0..1): nobody picnics in the wet; out already, they pack up.
    pub fn update(&mut self, player: Vec3, clock: f32, rain: f32) {
        let me = vec2(player.x, player.z);
        let all = spots();
        let dry = rain < 0.2;
        self.out.retain(|(i, ..)| dry && all[*i].picnic.distance(me) < NEAR + 100.0 && visited(all[*i].seed, clock));
        for (i, spot) in all.iter().enumerate() {
            if dry && spot.picnic.distance(me) < NEAR && visited(spot.seed, clock) && !self.out.iter().any(|(j, ..)| *j == i) {
                let f = spot.park_dir;
                let yaw = (-f.x).atan2(-f.y);
                let mut car = CarPhysics::new(vec3(spot.park.x, 0.0, spot.park.y), yaw);
                car.place(&WorldGround, spot.park.x, spot.park.y, yaw);
                let paint = (hash(spot.seed, 4.0) * CAR_COLORS.len() as f32) as usize % CAR_COLORS.len();
                self.out.push((i, car, paint));
            }
        }
    }

    /// Where the families (and their dogs) are out.
    pub fn parties(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.out.iter().map(|(i, ..)| spots()[*i].picnic)
    }

    /// The parked cars, with their paint.
    pub fn cars(&self) -> impl Iterator<Item = (&CarPhysics, usize)> {
        self.out.iter().map(|(_, car, paint)| (car, *paint))
    }

    /// The visitors near enough to see: rugs, hampers, people and dogs.
    pub fn build_mesh(&self, camera: Vec3, time: f32) -> (Vec<crate::geometry::Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let mut m = Mesh::default();
        for (i, ..) in &self.out {
            let spot = &spots()[*i];
            if spot.picnic.distance(cam) > DRAW {
                continue;
            }
            party(&mut m, spot, time);
        }
        (m.verts, m.idxs)
    }
}

fn ground(p: Vec2) -> Vec3 {
    vec3(p.x, terrain_height(p.x, p.y) + 0.05, p.y)
}

/// A family out for the day: the rug down with a hamper on it, the
/// grown-ups sat facing the view, a child and the dog running rings round.
fn party(m: &mut Mesh, spot: &Spot, time: f32) {
    let k = PERSON_SCALE;
    let r = |n: f32| hash(spot.seed, n);
    let (view, side) = (spot.view, spot.view.perp());
    let c = spot.picnic;
    // The rug: a check of two colours
    const RUGS: [([f32; 3], [f32; 3]); 3] = [([0.8, 0.15, 0.12], [0.95, 0.93, 0.88]), ([0.15, 0.35, 0.6], [0.92, 0.9, 0.85]), ([0.2, 0.45, 0.25], [0.85, 0.75, 0.3])];
    let (ra, rb) = RUGS[(r(5.0) * 3.0) as usize % 3];
    let cells = 4;
    let size = 1.8 * k;
    for a in 0..cells {
        for b in 0..cells {
            let corner = |i: usize, j: usize| ground(c + side * ((i as f32 / cells as f32 - 0.5) * size) + view * ((j as f32 / cells as f32 - 0.5) * size));
            m.quad(corner(a, b), corner(a + 1, b), corner(a + 1, b + 1), corner(a, b + 1), if (a + b) % 2 == 0 { ra } else { rb });
        }
    }
    // A wicker hamper, lid shut
    let hamper = ground(c - view * 0.5 * k + side * 0.5 * k);
    let (rgt, fwd) = (vec3(side.x, 0.0, side.y), vec3(view.x, 0.0, view.y));
    m.box_at(hamper + Vec3::Y * 0.2 * k, rgt, fwd, vec3(0.3, 0.2, 0.2) * k, [0.7, 0.55, 0.3], [0.62, 0.48, 0.26]);
    // The grown-ups, sat looking at the view
    let adults = 1 + (r(6.0) * 2.0) as usize + 1;
    for a in 0..adults.min(3) {
        let at = c + side * ((a as f32 - 0.5) * 0.7 * k) - view * 0.3 * k;
        m.figure(ground(at), view, 0.0, Pose::OnGround, spot.seed + a as f32 * 7.0);
    }
    if let Some(scope) = spot.scope {
        // Someone at the telescope, looking out
        m.figure(ground(scope - view * 1.6), view, 0.0, Pose::Standing, spot.seed + 11.0);
    }
    // A child and the dog chasing round and round, now and then swapping who chases
    let lap = time * 0.45 + r(7.0) * 10.0;
    let ring = 4.5 * k;
    let orbit = |a: f32| c + Vec2::from_angle(a) * ring * (1.0 + 0.15 * (a * 2.0).sin());
    let child = orbit(lap);
    let ahead = orbit(lap + 0.05);
    let child_dir = (ahead - child).normalize_or_zero();
    let chasing = (lap * 0.3).sin() > 0.0;
    let dog_a = lap + if chasing { 0.35 } else { -0.35 };
    let dog = orbit(dog_a);
    let dog_dir = (orbit(dog_a + 0.05) - dog).normalize_or_zero();
    // The child is small: drawn three quarters size about their feet
    let first = m.verts.len();
    m.figure(ground(child), child_dir, lap * 9.0, Pose::Walking, spot.seed + 3.0);
    let feet = ground(child);
    for v in &mut m.verts[first..] {
        v.pos = (feet + (Vec3::from(v.pos) - feet) * 0.62).into();
    }
    dog_at(m, ground(dog), dog_dir, lap * 12.0, spot.seed);
}

/// A dog bounding along, ears up, tail going.
pub(crate) fn dog_at(m: &mut Mesh, at: Vec3, dir: Vec2, phase: f32, seed: f32) {
    const COATS: [[f32; 3]; 4] = [[0.85, 0.65, 0.3], [0.12, 0.1, 0.1], [0.55, 0.35, 0.2], [0.9, 0.88, 0.82]];
    dog_in(m, at, dir, phase, COATS[(hash(seed, 8.0) * 4.0) as usize % 4]);
}

/// A dog of the given coat bounding along.
pub(crate) fn dog_in(m: &mut Mesh, at: Vec3, dir: Vec2, phase: f32, coat: [f32; 3]) {
    let k = PERSON_SCALE;
    let f = vec3(dir.x, 0.0, dir.y);
    let r = vec3(dir.y, 0.0, -dir.x);
    let bound = phase.sin().abs() * 0.12 * k;
    let body = at + Vec3::Y * (0.45 * k + bound);
    m.box_at(body, r, f, vec3(0.13, 0.13, 0.32) * k, coat, coat);
    let head = body + f * 0.38 * k + Vec3::Y * 0.14 * k;
    m.box_at(head, r, f, vec3(0.1, 0.1, 0.13) * k, coat, coat);
    m.box_at(head + f * 0.15 * k - Vec3::Y * 0.03 * k, r, f, vec3(0.06, 0.05, 0.06) * k, [0.1, 0.08, 0.08], coat);
    for s in [-1.0, 1.0] {
        m.box_at(head + r * s * 0.07 * k + Vec3::Y * 0.12 * k, r, f, vec3(0.03, 0.05, 0.02) * k, coat, coat);
    }
    // Legs galloping, front and back pairs out of step
    for (z, off) in [(0.24, 0.0), (-0.24, std::f32::consts::PI)] {
        let swing = (phase + off).sin() * 0.25 * k;
        for s in [-1.0, 1.0] {
            let hip = body + f * z * k + r * s * 0.08 * k;
            let paw = at + f * (z * k + swing) + r * s * 0.08 * k + Vec3::Y * 0.03;
            m.limb(hip, paw, 0.04 * k, coat);
        }
    }
    // The tail, wagging
    let wag = (phase * 2.0).sin() * 0.15 * k;
    m.limb(body - f * 0.3 * k, body - f * 0.48 * k + Vec3::Y * 0.18 * k + r * wag, 0.03 * k, coat);
}
