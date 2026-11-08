//! Everyday traffic on the connecting roads: family cars, delivery vans and
//! the country bus, keeping to their side of the road at a steady pace,
//! easing off for bends and through the towns, keeping their distance from
//! whatever's ahead, stopping for anyone at a zebra crossing, and the bus
//! calling at each town's stop. Bump one and it stops, hazards going.
//!
//! They drive themselves along the road's centreline (no physics); to other
//! cars they're solid, moving obstacles (`solids`), and other drivers see
//! them as traffic.

use std::collections::HashMap;
use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::collisions::Solid;
use crate::geometry::Vertex;
use crate::props::{hash, PERSON_SCALE};
use crate::road::{self, PathKind, SAMPLE_SPACING};
use crate::towns;
use crate::townsfolk::Mesh;

/// How many are kept on the roads round the player.
const VEHICLES: usize = 12;
/// Spawn between these distances (out of sight round the bends), forget beyond.
const SPAWN: (f32, f32) = (350.0, 900.0);
const FORGET: f32 = 1200.0;
const DRAW: f32 = 900.0;
/// Cruising and town speeds (a unit is half a metre: 46 is about 80 km/h).
const CRUISE: f32 = 46.0;
const TOWN: f32 = 22.0;
/// Out from the centreline to the middle of their lane (right of the way
/// they're going), as a share of the road's half width.
const LANE: f32 = 0.45;
/// Gap kept to whatever's ahead, in seconds of travel (and at least this far).
const HEADWAY: f32 = 1.6;
const MIN_GAP: f32 = 14.0;
/// Cells for finding roads near a point.
const CELL: f32 = 250.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Car,
    Van,
    Bus,
    /// The post van on its round.
    Post,
}

impl Kind {
    /// Half width, half length, height (metres).
    fn size(self) -> (f32, f32, f32) {
        match self {
            Kind::Car => (0.9, 2.15, 1.45),
            Kind::Van | Kind::Post => (1.0, 2.6, 2.4),
            Kind::Bus => (1.25, 5.6, 3.1),
        }
    }
}

struct Vehicle {
    kind: Kind,
    path: usize,
    /// Along the path in samples, and which way.
    at: f32,
    way: f32,
    speed: f32,
    /// Stopped after being hit, for this long more.
    shunted: f32,
    /// The bus: waiting at the stop for this long more, and the town it last called at.
    dwell: f32,
    called: Option<usize>,
    /// How hard it's braking just now (0..1), for the brake lights.
    braking: f32,
    /// How far over into the other lane it's moved (0..1), past road works.
    shift: f32,
    /// The post van: the farm gates and towns' pillar boxes already called at.
    visited: Vec<u32>,
    seed: f32,
}

/// Road samples by cell, to spawn near the player.
fn grid() -> &'static HashMap<(i32, i32), Vec<(usize, usize)>> {
    static GRID: OnceLock<HashMap<(i32, i32), Vec<(usize, usize)>>> = OnceLock::new();
    GRID.get_or_init(|| {
        let mut g: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        if let Some(net) = road::network() {
            for (pi, path) in net.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
                for (i, q) in path.points.iter().enumerate().step_by(6) {
                    g.entry(((q.x / CELL).floor() as i32, (q.y / CELL).floor() as i32)).or_default().push((pi, i));
                }
            }
        }
        g
    })
}

pub struct Traffic {
    vehicles: Vec<Vehicle>,
    rng: u32,
    /// Whether the post's out on its round.
    post_hours: bool,
    /// Spawn ring (dev aid STUMBLER_TRAFFIC_CLOSE=1 brings it in for screenshots).
    ring: (f32, f32),
}

impl Default for Traffic {
    fn default() -> Self {
        let close = std::env::var_os("STUMBLER_TRAFFIC_CLOSE").is_some();
        Self { vehicles: Vec::new(), rng: 0x7aff_1c, post_hours: false, ring: if close { (60.0, 220.0) } else { SPAWN } }
    }
}

/// Where a vehicle is (on the ground, in its lane) and which way it faces.
fn pose(net: &road::RoadNetwork, v: &Vehicle) -> (Vec3, Vec2) {
    let path = &net.paths[v.path];
    let n = path.points.len();
    let i = (v.at.floor().max(0.0) as usize).min(n - 2);
    let f = (v.at - i as f32).clamp(0.0, 1.0);
    let p = path.points[i].lerp(path.points[i + 1], f);
    let h = path.heights[i] + (path.heights[i + 1] - path.heights[i]) * f;
    let dir = path.tangent(i) * v.way;
    let right = vec2(dir.y, -dir.x);
    let at = p + right * path.kind.half_width() * LANE * (1.0 - 1.6 * v.shift);
    (vec3(at.x, h.max(crate::terrain::terrain_height(at.x, at.y)), at.y), dir)
}

impl Traffic {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223) | 1;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `others`: every other car about (position, velocity), to keep a
    /// gap to. `clock`: time of day (0..1 from midnight); quieter at night.
    /// `works`: today's road works, `time` the running clock their lights keep.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, player: Vec3, others: &[(Vec3, Vec3)], folk: &crate::townsfolk::Townsfolk, works: &[crate::roadworks::Works], time: f32, clock: f32, dt: f32) {
        let Some(net) = road::network() else { return };
        let me = vec2(player.x, player.z);
        self.vehicles.retain(|v| { let p = pose(net, v).0; vec2(p.x, p.z).distance(me) < FORGET });
        let h = clock * 24.0;
        self.post_hours = (8.0..17.0).contains(&h);
        let want = if (7.0..20.0).contains(&h) { VEHICLES } else if (5.5..23.0).contains(&h) { VEHICLES / 2 } else { 2 };
        if self.vehicles.len() < want {
            self.spawn(net, me);
        }
        // Everyone's place and motion, to keep gaps by
        let poses: Vec<(Vec3, Vec2, f32)> = self.vehicles.iter().map(|v| { let (p, d) = pose(net, v); (p, d, v.speed) }).collect();
        let towns = towns::all();
        for (k, v) in self.vehicles.iter_mut().enumerate() {
            let path = &net.paths[v.path];
            let n = path.points.len() as f32;
            let (pos, dir, _) = poses[k];
            let here = vec2(pos.x, pos.z);
            let (_, half_len, _) = v.kind.size();
            // How fast it'd like to go: the bend ahead, the town it's in
            let ahead_i = (v.at + v.way * 8.0).clamp(0.0, n - 1.0) as usize;
            let bend = path.curvature[ahead_i].max(path.curvature[v.at.clamp(0.0, n - 1.0) as usize]);
            let mut target = CRUISE.min((26.0 / bend.max(1e-4)).sqrt());
            let town = towns::at(here).and_then(|t| towns.iter().position(|o| std::ptr::eq(o, t)));
            if town.is_some() {
                target = target.min(TOWN);
            }
            // Keep a gap to anything ahead in its lane (other vehicles, cars)
            let gap_wanted = (v.speed * HEADWAY).max(MIN_GAP) + half_len * PERSON_SCALE;
            let mut room = f32::MAX;
            let mut ahead = |p: Vec3, len: f32| {
                let to = vec2(p.x, p.z) - here;
                let along = to.dot(dir);
                let across = (to - dir * along).length();
                if along > 0.0 && across < 5.0 {
                    room = room.min(along - len);
                }
            };
            for (j, &(p, d, _)) in poses.iter().enumerate() {
                if j != k && d.dot(dir) > 0.3 {
                    ahead(p, 0.0);
                }
            }
            for &(p, vel) in others {
                if vec2(vel.x, vel.z).dot(dir) > -5.0 {
                    ahead(p, 4.0);
                }
            }
            if room < gap_wanted {
                target = target.min(((room - MIN_GAP * 0.5).max(0.0) / HEADWAY).min(target));
            }
            // A zebra crossing with someone on it or waiting: stop short of it
            if let Some(t) = town {
                let tw = &towns[t];
                if let Some(l) = tw.local(here) {
                    let towards = tw.local(here + dir * 5.0).map(|a| (a.x - l.x).signum()).unwrap_or(0.0);
                    let to_crossing = (towns::CROSSING - l.x) * towards;
                    if to_crossing > 0.0 && to_crossing < 45.0 && folk.crossing_busy(t) {
                        target = target.min(((to_crossing - 10.0).max(0.0) * 0.8).min(target));
                    }
                    // The bus calls at the stop, just before the square
                    if v.kind == Kind::Bus && v.called != Some(t) {
                        let to_stop = (-towns::SQUARE_HALF - 20.0 - l.x) * towards;
                        if (0.0..40.0).contains(&to_stop) {
                            target = target.min(to_stop * 0.7);
                            if to_stop < 2.0 {
                                v.dwell = 8.0;
                                v.called = Some(t);
                            }
                        }
                    }
                }
            }
            // Road works: wait at the lights on red, crawl through, and move
            // over into the open lane past the coned-off one
            let mut past_works = false;
            for w in works.iter().filter(|w| w.path == v.path) {
                let to_line = (w.stop_line(v.way) - v.at) * v.way;
                if to_line > 0.0 && to_line < 14.0 && !w.green_for(v.way, time) {
                    target = target.min(((to_line * SAMPLE_SPACING - 3.0).max(0.0) * 0.6).min(target));
                }
                if v.at > w.from - 10.0 && v.at < w.to + 10.0 {
                    target = target.min(14.0);
                    past_works |= v.way == w.closed;
                }
            }
            let to = if past_works { 1.0 } else { 0.0 };
            v.shift += (to - v.shift).clamp(-dt * 0.8, dt * 0.8);
            // The post van calls at each farm gate and pillar box it comes to
            if v.kind == Kind::Post && v.dwell <= 0.0 {
                let half = net.paths[v.path].kind.half_width();
                let mut calls: Vec<(u32, Vec2, f32)> = crate::farms::all().iter().enumerate()
                    .filter(|(_, f)| f.gate.distance(here) < 140.0)
                    .map(|(i, f)| (i as u32, f.gate, half + 6.0)).collect();
                for (t, town) in towns.iter().enumerate().filter(|(_, t)| t.centre.distance(here) < 400.0) {
                    calls.push((10_000 + t as u32, town.world(-towns::HALF_LEN * 0.4, -town.square * (towns::KERB + 1.8)), half + 8.0));
                }
                for (key, at, wide) in calls {
                    let to = at - here;
                    let ahead = to.dot(dir);
                    if v.visited.contains(&key) || ahead < 0.0 || ahead > 45.0 || (to - dir * ahead).length() > wide {
                        continue;
                    }
                    target = target.min((ahead - 5.0).max(0.0) * 0.6);
                    if ahead < 7.0 {
                        v.dwell = 12.0;
                        v.visited.push(key);
                    }
                }
            }
            if v.dwell > 0.0 {
                v.dwell -= dt;
                target = 0.0;
            }
            if v.shunted > 0.0 {
                v.shunted -= dt;
                target = 0.0;
            }
            // Gentle acceleration, firmer braking
            let accel = if target > v.speed { 6.0 } else { 16.0 };
            let before = v.speed;
            v.speed += (target - v.speed).clamp(-accel * dt, accel * dt);
            v.braking = if v.speed < before - 0.01 || v.speed < 0.5 { 1.0 } else { 0.0 };
            v.at += v.way * v.speed * dt / SAMPLE_SPACING;
            // At a road's end, turn round (out of sight, on a dead end)
            if v.at < 1.0 || v.at > n - 2.0 {
                if path.closed {
                    v.at = v.at.rem_euclid(n - 1.0);
                } else {
                    v.way = -v.way;
                    v.at = v.at.clamp(1.0, n - 2.0);
                }
            }
        }
    }

    fn spawn(&mut self, net: &road::RoadNetwork, me: Vec2) {
        let a = self.rand() * std::f32::consts::TAU;
        let d = self.ring.0 + (self.ring.1 - self.ring.0) * self.rand();
        let q = me + Vec2::from_angle(a) * d;
        let Some(cands) = grid().get(&((q.x / CELL).floor() as i32, (q.y / CELL).floor() as i32)) else { return };
        let (pi, i) = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
        let p = net.paths[pi].points[i];
        if towns::occupied(p, 30.0) {
            return;
        }
        let way = if self.rand() < 0.5 { 1.0 } else { -1.0 };
        let probe = Vehicle { kind: Kind::Car, path: pi, at: i as f32, way, speed: 0.0, shunted: 0.0, dwell: 0.0, called: None, braking: 0.0, shift: 0.0, visited: Vec::new(), seed: 0.0 };
        let spot = pose(net, &probe).0;
        if self.vehicles.iter().any(|v| pose(net, v).0.distance(spot) < self.ring.0.min(80.0)) {
            return;
        }
        let roll = self.rand();
        let buses = self.vehicles.iter().filter(|v| v.kind == Kind::Bus).count();
        let posts = self.vehicles.iter().filter(|v| v.kind == Kind::Post).count();
        let kind = if self.post_hours && posts == 0 && roll < 0.3 { Kind::Post }
            else if roll < 0.12 && buses == 0 { Kind::Bus } else if roll < 0.35 { Kind::Van } else { Kind::Car };
        let seed = self.rand() * 1000.0;
        self.vehicles.push(Vehicle { kind, speed: CRUISE * 0.8, seed, ..probe });
    }

    /// How many calls the post vans about have made.
    #[cfg(test)]
    pub fn post_calls(&self) -> usize {
        self.vehicles.iter().filter(|v| v.kind == Kind::Post).map(|v| v.visited.len()).sum()
    }

    /// Solid, moving boxes for other cars to bump into, with each one's velocity.
    pub fn solids(&self) -> Vec<(Solid, Vec3)> {
        let Some(net) = road::network() else { return Vec::new() };
        self.vehicles.iter().map(|v| {
            let (p, dir) = pose(net, v);
            let (hw, hl, h) = v.kind.size();
            let solid = Solid { centre: vec2(p.x, p.z), half: vec2(hw, hl) * PERSON_SCALE, yaw: dir.x.atan2(dir.y), top: p.y + h * PERSON_SCALE, round: false };
            (solid, vec3(dir.x, 0.0, dir.y) * v.speed)
        }).collect()
    }

    /// Something hit the vehicle nearest `at`: it stops and waits.
    pub fn shunt(&mut self, at: Vec3) {
        let Some(net) = road::network() else { return };
        if let Some(v) = self.vehicles.iter_mut().min_by(|a, b| pose(net, a).0.distance(at).total_cmp(&pose(net, b).0.distance(at))) {
            v.shunted = 6.0;
            v.speed *= 0.3;
        }
    }

    /// Each vehicle as other drivers see it: where, how it's moving, braking.
    pub fn as_traffic(&self) -> Vec<(Vec3, Vec3, bool)> {
        let Some(net) = road::network() else { return Vec::new() };
        self.vehicles.iter().map(|v| {
            let (p, dir) = pose(net, v);
            (p, vec3(dir.x, 0.0, dir.y) * v.speed, v.braking > 0.5)
        }).collect()
    }

    /// The vehicles near enough to see, and their lamps (to glow).
    pub fn build_mesh(&self, camera: Vec3, night: f32, time: f32) -> ((Vec<Vertex>, Vec<u16>), (Vec<Vertex>, Vec<u16>)) {
        let mut m = Mesh::default();
        let mut lamps = Mesh::default();
        let Some(net) = road::network() else { return ((m.verts, m.idxs), (lamps.verts, lamps.idxs)) };
        for v in &self.vehicles {
            let (p, dir) = pose(net, v);
            if p.distance(camera) > DRAW {
                continue;
            }
            let stopped_off = v.shunted > 0.0 || (v.kind == Kind::Post && v.dwell > 0.0);
            let hazards = stopped_off && (time * 3.0).fract() < 0.5;
            vehicle(&mut m, &mut lamps, p, dir, v.kind, v.seed, v.braking > 0.5, night > 0.3, hazards);
            if v.kind == Kind::Post && v.dwell > 0.0 && v.dwell < 11.0 {
                // The postie out with a parcel, off to the gate (or the box) and back
                let right = Vec2::new(dir.y, -dir.x);
                let out = 3.0 + 4.0 * (1.0 - (v.dwell - 5.5).abs() / 5.5);
                let at = Vec2::new(p.x, p.z) + right * out;
                let facing = if v.dwell > 5.5 { right } else { -right };
                let feet = Vec3::new(at.x, crate::terrain::terrain_height(at.x, at.y).max(p.y - 0.5), at.y);
                m.figure(feet, facing, time * 4.0, crate::townsfolk::Pose::Walking, v.seed + 7.0);
                let k = PERSON_SCALE;
                let f3 = Vec3::new(facing.x, 0.0, facing.y);
                let r3 = Vec3::new(facing.y, 0.0, -facing.x);
                m.box_at(feet + Vec3::Y * 1.1 * k + f3 * 0.35 * k, r3, f3, Vec3::new(0.2, 0.15, 0.15) * k, [0.7, 0.55, 0.35], [0.75, 0.6, 0.4]);
            }
        }
        ((m.verts, m.idxs), (lamps.verts, lamps.idxs))
    }
}

/// A vehicle standing at `at`, nose along `dir`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn vehicle(m: &mut Mesh, lamps: &mut Mesh, at: Vec3, dir: Vec2, kind: Kind, seed: f32, braking: bool, lights: bool, hazards: bool) {
    let k = PERSON_SCALE;
    let f = vec3(dir.x, 0.0, dir.y);
    let r = vec3(dir.y, 0.0, -dir.x);
    let p = |along: f32, up: f32, across: f32| at + (f * along + Vec3::Y * up + r * across) * k;
    let (hw, hl, _) = kind.size();
    const PAINTS: [[f32; 3]; 8] = [[0.75, 0.1, 0.1], [0.12, 0.25, 0.55], [0.85, 0.85, 0.82], [0.15, 0.15, 0.16], [0.55, 0.57, 0.6], [0.2, 0.42, 0.28], [0.85, 0.62, 0.2], [0.45, 0.2, 0.35]];
    let glass = [0.2, 0.26, 0.32];
    let tyre = [0.08, 0.08, 0.08];
    let bx = |m: &mut Mesh, along: f32, up: f32, across: f32, half: Vec3, side: [f32; 3], top: [f32; 3]| m.box_at(p(along, up, across), r, f, half * k, side, top);
    let paint = match kind {
        Kind::Car => PAINTS[(hash(seed, 1.0) * 8.0) as usize % 8],
        Kind::Van => if hash(seed, 1.0) < 0.6 { [0.92, 0.92, 0.9] } else { PAINTS[(hash(seed, 2.0) * 8.0) as usize % 8] },
        Kind::Post => [0.8, 0.08, 0.06],
        Kind::Bus => [0.15, 0.45, 0.25],
    };
    // Wheels
    let axles: &[f32] = match kind { Kind::Bus => &[3.6, -3.4], _ => &[hl * 0.62, -hl * 0.62] };
    for &a in axles {
        for s in [-1.0, 1.0] {
            bx(m, a, 0.34, s * (hw - 0.12), vec3(0.14, 0.34, 0.34), tyre, tyre);
        }
    }
    match kind {
        Kind::Car => {
            bx(m, 0.0, 0.62, 0.0, vec3(hw, 0.32, hl), paint, paint);
            // The cabin, set back, with its windows
            bx(m, -0.25, 1.17, 0.0, vec3(hw - 0.08, 0.26, hl * 0.52), glass, paint);
            bx(m, -0.25, 1.45, 0.0, vec3(hw - 0.1, 0.03, hl * 0.5), paint, paint);
        }
        Kind::Van | Kind::Post => {
            bx(m, 0.4, 1.3, 0.0, vec3(hw, 0.95, hl - 0.6), paint, paint);
            if kind == Kind::Post {
                // A yellow band down each side
                for s in [-1.0, 1.0] {
                    bx(m, 0.4, 0.95, s * (hw + 0.01), vec3(0.01, 0.1, hl - 0.65), [0.95, 0.8, 0.1], [0.95, 0.8, 0.1]);
                }
            }
            bx(m, hl - 0.45, 0.75, 0.0, vec3(hw, 0.4, 0.45), paint, paint);
            bx(m, hl - 0.75, 1.65, 0.0, vec3(hw - 0.05, 0.35, 0.2), glass, paint);
            // A sign on the side
            if kind == Kind::Van && hash(seed, 3.0) < 0.6 {
                let livery = PAINTS[(hash(seed, 4.0) * 8.0) as usize % 8];
                for s in [-1.0, 1.0] {
                    bx(m, 0.2, 1.55, s * (hw + 0.01), vec3(0.01, 0.35, 1.4), livery, livery);
                }
            }
        }
        Kind::Bus => {
            bx(m, 0.0, 1.7, 0.0, vec3(hw, 1.35, hl), paint, [0.9, 0.9, 0.86]);
            // A band of windows down each side and across the front
            for s in [-1.0, 1.0] {
                bx(m, 0.2, 2.15, s * (hw + 0.01), vec3(0.01, 0.45, hl - 0.6), glass, glass);
            }
            bx(m, hl + 0.01, 2.1, 0.0, vec3(hw - 0.15, 0.6, 0.01), glass, glass);
            bx(m, 0.0, 1.05, 0.0, vec3(hw + 0.01, 0.1, hl + 0.01), [0.9, 0.85, 0.2], [0.9, 0.85, 0.2]);
            // Destination board
            bx(m, hl + 0.02, 2.85, 0.0, vec3(hw * 0.7, 0.15, 0.01), [0.1, 0.1, 0.1], [0.1, 0.1, 0.1]);
        }
    }
    // Lamps: headlights when it's dark, brake lights, the hazards blinking
    let front_up = 0.7;
    if lights {
        for s in [-1.0, 1.0] {
            bx(lamps, hl + 0.02, front_up, s * (hw - 0.3), vec3(0.18, 0.1, 0.02), [1.0, 0.95, 0.8], [1.0, 0.95, 0.8]);
        }
    }
    let rear = if braking || lights { if braking { [1.0, 0.1, 0.08] } else { [0.6, 0.05, 0.05] } } else { [0.35, 0.05, 0.05] };
    for s in [-1.0, 1.0] {
        let target = if braking || lights { &mut *lamps } else { &mut *m };
        bx(target, -hl - 0.02, front_up + 0.1, s * (hw - 0.25), vec3(0.16, 0.12, 0.02), rear, rear);
        if hazards {
            for end in [hl + 0.03, -hl - 0.03] {
                bx(lamps, end, front_up + 0.25, s * (hw - 0.1), vec3(0.08, 0.06, 0.02), [1.0, 0.6, 0.1], [1.0, 0.6, 0.1]);
            }
        }
    }
}
