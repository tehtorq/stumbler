//! Folk out on the roads and trails: cyclists riding the country roads
//! alone or in a little group, keeping to the edge and pulling onto the
//! verge when a car comes up behind; and walkers on the trails out to the
//! landmarks, some with a rucksack, most with a dog snuffling about them
//! on its lead, stepping off the trail when a car comes along.
//!
//! Like the wildlife they appear in a ring round the player and are
//! forgotten once left behind.

use std::collections::HashMap;
use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::props::{hash, PERSON_SCALE};
use crate::road::{self, PathKind, SAMPLE_SPACING};
use crate::townsfolk::{Mesh, Pose};

/// How many groups of each are kept about.
const RIDES: usize = 5;
const WALKS: usize = 5;
/// Spawn between these distances from the player; forget beyond the last.
const SPAWN: (f32, f32) = (180.0, 480.0);
const FORGET: f32 = 700.0;
/// Drawn out to this far.
const DRAW: f32 = 520.0;
/// Riding and walking pace (a unit is half a metre).
const RIDE: (f32, f32) = (11.0, 16.0);
const WALK: (f32, f32) = (2.4, 3.2);
/// Cells for finding the roads and trails near a point.
const CELL: f32 = 200.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Cyclist,
    Walker { rucksack: bool, dog: bool },
}

struct Traveller {
    kind: Kind,
    path: usize,
    /// Along the path, in samples (fractional).
    at: f32,
    /// Which way along it: +1 or -1.
    way: f32,
    /// Out from the centreline, toward the travelling direction's right,
    /// and where they'd like to be.
    off: f32,
    keep: f32,
    pace: f32,
    /// Pedal or step clock.
    phase: f32,
    seed: f32,
    /// A dog's place about its walker (relative), and where it's headed.
    dog: Vec2,
    sniff: Vec2,
}

/// Road and trail samples by cell, to spawn near the player.
fn grid() -> &'static HashMap<(i32, i32), Vec<(usize, usize)>> {
    static GRID: OnceLock<HashMap<(i32, i32), Vec<(usize, usize)>>> = OnceLock::new();
    GRID.get_or_init(|| {
        let mut g: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        if let Some(net) = road::network() {
            for (pi, path) in net.paths.iter().enumerate().filter(|(_, p)| matches!(p.kind, PathKind::Road | PathKind::Trail)) {
                for (i, q) in path.points.iter().enumerate().step_by(5) {
                    g.entry(((q.x / CELL).floor() as i32, (q.y / CELL).floor() as i32)).or_default().push((pi, i));
                }
            }
        }
        g
    })
}

pub struct Travellers {
    folk: Vec<Traveller>,
    rng: u32,
    /// Spawn ring (dev aid STUMBLER_TRAVELLERS_CLOSE=1 brings it in for screenshots).
    ring: (f32, f32),
}

impl Default for Travellers {
    fn default() -> Self {
        let close = std::env::var_os("STUMBLER_TRAVELLERS_CLOSE").is_some();
        Self { folk: Vec::new(), rng: 0x7a7e_11e5, ring: if close { (25.0, 90.0) } else { SPAWN } }
    }
}

impl Travellers {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `cars`: where each car near enough to matter is and how it's moving.
    /// `clock`: the time of day (0..1 from midnight); nobody's out after dark.
    /// `rain` (0..1): only the hardiest are out in the wet.
    pub fn update(&mut self, player: Vec3, cars: &[(Vec3, Vec3)], clock: f32, rain: f32, dt: f32) {
        let Some(net) = road::network() else { return };
        let me = vec2(player.x, player.z);
        self.folk.retain(|t| position(net, t).distance(me) < FORGET);
        let h = clock * 24.0;
        let out = if (7.0..20.0).contains(&h) { 1.0 } else if (6.0..21.0).contains(&h) { 0.4 } else { 0.0 };
        let out = out * (1.0 - 0.8 * rain.min(1.0));
        self.spawn(net, me, out);
        for t in &mut self.folk {
            t.update(net, cars, dt);
        }
    }

    fn spawn(&mut self, net: &road::RoadNetwork, me: Vec2, out: f32) {
        let groups = |f: &Self, cyclist: bool| {
            let mut seeds: Vec<i32> = f.folk.iter().filter(|t| matches!(t.kind, Kind::Cyclist) == cyclist).map(|t| (t.seed * 0.01) as i32).collect();
            seeds.sort();
            seeds.dedup();
            seeds.len()
        };
        for _ in 0..2 {
            let a = self.rand() * std::f32::consts::TAU;
            let d = self.ring.0 + (self.ring.1 - self.ring.0) * self.rand();
            let q = me + Vec2::from_angle(a) * d;
            let Some(cands) = grid().get(&((q.x / CELL).floor() as i32, (q.y / CELL).floor() as i32)) else { continue };
            let (pi, i) = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
            let path = &net.paths[pi];
            let cyclist = path.kind == PathKind::Road;
            let want = ((if cyclist { RIDES } else { WALKS }) as f32 * out).round() as usize;
            if groups(self, cyclist) >= want {
                continue;
            }
            // Not in a town (the townsfolk have that), nor right by someone else
            if crate::towns::occupied(path.points[i], 20.0) || self.folk.iter().any(|t| position(net, t).distance(path.points[i]) < self.ring.0.min(120.0)) {
                continue;
            }
            let way = if self.rand() < 0.5 { 1.0 } else { -1.0 };
            let group = (self.rand() * 1000.0).floor() * 100.0;
            if cyclist {
                // A lone rider, or a few riding in a line
                let riders = if self.rand() < 0.5 { 1 } else { 2 + (self.rand() * 3.0) as usize };
                let pace = RIDE.0 + (RIDE.1 - RIDE.0) * self.rand();
                let hw = path.kind.half_width();
                for r in 0..riders {
                    let keep = hw - 4.0 - (r % 2) as f32 * 1.6;
                    let phase = self.rand() * 6.0;
                    self.folk.push(Traveller { kind: Kind::Cyclist, path: pi, at: i as f32 - way * r as f32 * 1.6, way, off: keep, keep, pace, phase, seed: group + r as f32, dog: Vec2::ZERO, sniff: Vec2::ZERO });
                }
            } else {
                let walkers = 1 + (self.rand() * 2.0) as usize;
                let pace = WALK.0 + (WALK.1 - WALK.0) * self.rand();
                let dog = self.rand() < 0.7;
                for w in 0..walkers {
                    let kind = Kind::Walker { rucksack: self.rand() < 0.4, dog: dog && w == 0 };
                    let keep = (w as f32 - 0.5) * 1.6;
                    let phase = self.rand() * 6.0;
                    self.folk.push(Traveller { kind, path: pi, at: i as f32, way, off: keep, keep, pace, phase, seed: group + w as f32, dog: vec2(2.0, 3.0), sniff: vec2(3.0, 4.0) });
                }
            }
        }
    }

    /// Each traveller: a cyclist?, where they are, how far out from the
    /// centreline, and their path's half width.
    #[cfg(test)]
    pub fn report(&self) -> Vec<(bool, Vec2, f32, f32)> {
        let net = road::network().unwrap();
        self.folk.iter().map(|t| (t.kind == Kind::Cyclist, position(net, t), t.off, net.paths[t.path].kind.half_width())).collect()
    }

    /// Where the walkers' dogs are.
    pub fn dogs(&self) -> Vec<Vec2> {
        let Some(net) = road::network() else { return Vec::new() };
        self.folk.iter().filter(|t| matches!(t.kind, Kind::Walker { dog: true, .. })).map(|t| position(net, t)).collect()
    }

    /// Everyone near enough to see.
    pub fn build_mesh(&self, camera: Vec3) -> (Vec<Vertex>, Vec<u16>) {
        let mut m = Mesh::default();
        let Some(net) = road::network() else { return (m.verts, m.idxs) };
        let cam = vec2(camera.x, camera.z);
        for t in &self.folk {
            let p = position(net, t);
            if p.distance(cam) > DRAW {
                continue;
            }
            let path = &net.paths[t.path];
            let fwd = path.tangent(t.at as usize) * t.way;
            let y = surface(net, t);
            let feet = vec3(p.x, y, p.y);
            match t.kind {
                Kind::Cyclist => {
                    bicycle(&mut m, feet, fwd, t.phase, t.seed);
                    m.figure(feet, fwd, 0.0, Pose::Riding(t.phase), t.seed);
                }
                Kind::Walker { rucksack, dog } => {
                    m.figure(feet, fwd, t.phase, Pose::Walking, t.seed);
                    let k = PERSON_SCALE;
                    let (right, front) = (vec3(fwd.y, 0.0, -fwd.x), vec3(fwd.x, 0.0, fwd.y));
                    if rucksack {
                        m.box_at(feet + Vec3::Y * 1.2 * k - front * 0.25 * k, right, front, vec3(0.2, 0.25, 0.1) * k, [0.7, 0.3, 0.15], [0.6, 0.25, 0.12]);
                    }
                    if dog {
                        // The dog about them, on its lead from the walker's hand
                        let d = p + fwd * t.dog.y + fwd.perp() * t.dog.x;
                        let dy = crate::terrain::terrain_height(d.x, d.y);
                        let heading = (t.sniff - t.dog).try_normalize().unwrap_or(fwd);
                        let dog_dir = (fwd * heading.y + fwd.perp() * heading.x).try_normalize().unwrap_or(fwd);
                        crate::picnics::dog_at(&mut m, vec3(d.x, dy, d.y), dog_dir, t.phase * 1.6, t.seed);
                        let hand = feet + Vec3::Y * 0.8 * k + right * 0.3 * k;
                        let collar = vec3(d.x, dy + 0.55 * k, d.y) + vec3(dog_dir.x, 0.0, dog_dir.y) * 0.35 * k;
                        m.limb(hand, collar, 0.03, [0.7, 0.15, 0.1]);
                    }
                }
            }
        }
        (m.verts, m.idxs)
    }
}

/// Where a traveller is, on the ground.
fn position(net: &road::RoadNetwork, t: &Traveller) -> Vec2 {
    let path = &net.paths[t.path];
    let n = path.points.len();
    let i = (t.at.floor().max(0.0) as usize).min(n - 2);
    let f = (t.at - i as f32).clamp(0.0, 1.0);
    let p = path.points[i].lerp(path.points[i + 1], f);
    // Their right as they travel
    let right = -path.tangent(i).perp() * t.way;
    p + right * t.off
}

/// The height they're riding or walking at: the road's surface, or the
/// ground off its edge.
fn surface(net: &road::RoadNetwork, t: &Traveller) -> f32 {
    let path = &net.paths[t.path];
    let n = path.points.len();
    let i = (t.at.max(0.0) as usize).min(n - 1);
    let p = position(net, t);
    if t.off.abs() < path.kind.half_width() {
        path.heights[i].max(crate::terrain::terrain_height(p.x, p.y))
    } else {
        crate::terrain::terrain_height(p.x, p.y)
    }
}

impl Traveller {
    fn update(&mut self, net: &road::RoadNetwork, cars: &[(Vec3, Vec3)], dt: f32) {
        let path = &net.paths[self.path];
        let n = path.points.len() as f32;
        let me = position(net, self);
        let hw = path.kind.half_width();
        // A car coming: cyclists pull over onto the verge, walkers step off
        let coming = cars.iter().any(|(c, v)| {
            let to = vec2(c.x, c.z) - me;
            let d = to.length();
            let vel = vec2(v.x, v.z);
            d < 60.0 && vel.length() > 3.0 && vel.dot(-to) > 0.0
        });
        let aside = match self.kind {
            Kind::Cyclist => hw + 3.0,
            Kind::Walker { .. } => hw + 4.0 + self.keep.abs(),
        };
        let target = if coming { aside } else { self.keep };
        self.off += (target - self.off).clamp(-6.0 * dt, 6.0 * dt);
        let slow = if coming && matches!(self.kind, Kind::Walker { .. }) { 0.3 } else { 1.0 };
        let speed = self.pace * slow;
        self.at += self.way * speed * dt / SAMPLE_SPACING;
        // At the end of a trail (or a road that stops), turn back
        if self.at < 1.0 || self.at > n - 2.0 {
            if path.closed {
                self.at = self.at.rem_euclid(n - 1.0);
            } else {
                self.way = -self.way;
                self.at = self.at.clamp(1.0, n - 2.0);
            }
        }
        self.phase += dt * match self.kind {
            // Cadence about 80 a minute, and steps to match the pace
            Kind::Cyclist => 8.4,
            Kind::Walker { .. } => speed * 2.2 / PERSON_SCALE,
        };
        if let Kind::Walker { dog: true, .. } = self.kind {
            // The dog trots to a spot near its walker, sniffs, picks another
            let to = self.sniff - self.dog;
            if to.length() < 0.5 {
                let r = |k: f32| hash(self.seed + self.phase.floor(), k);
                self.sniff = vec2((r(1.0) - 0.5) * 8.0, 1.0 + r(2.0) * 5.0);
            } else {
                // Keeps up with the walker as well as wandering
                self.dog += to.normalize() * (speed * 0.9 + 1.5) * dt;
            }
        }
    }
}

/// A bicycle under its rider: wheels, frame, bars and saddle.
fn bicycle(m: &mut Mesh, at: Vec3, fwd: Vec2, crank: f32, seed: f32) {
    let k = PERSON_SCALE;
    const FRAMES: [[f32; 3]; 5] = [[0.85, 0.12, 0.1], [0.1, 0.3, 0.75], [0.95, 0.85, 0.15], [0.12, 0.12, 0.13], [0.2, 0.6, 0.3]];
    let paint = FRAMES[(hash(seed, 9.0) * 5.0) as usize % 5];
    let tyre = [0.08, 0.08, 0.08];
    let f = vec3(fwd.x, 0.0, fwd.y);
    let r = vec3(fwd.y, 0.0, -fwd.x);
    let p = |along: f32, up: f32| at + (f * along + Vec3::Y * up) * k;
    let radius = 0.34;
    for axle in [-0.52, 0.52] {
        let c = p(axle, radius);
        for s in 0..10 {
            let (a0, a1) = (s as f32 / 10.0 * std::f32::consts::TAU, (s + 1) as f32 / 10.0 * std::f32::consts::TAU);
            let rim = |a: f32| c + (f * a.cos() + Vec3::Y * a.sin()) * radius * k;
            m.limb(rim(a0), rim(a1), 0.035 * k, tyre);
        }
    }
    let (rear, front, bracket, seat, head) = (p(-0.52, radius), p(0.52, radius), p(0.05, 0.3), p(-0.12, 0.88), p(0.42, 0.85));
    for (a, b) in [(rear, bracket), (rear, seat), (bracket, seat), (bracket, head), (seat, head), (head, front)] {
        m.limb(a, b, 0.03 * k, paint);
    }
    // Saddle, bars, and the cranks going round
    m.box_at(seat + Vec3::Y * 0.05 * k, r, f, vec3(0.06, 0.03, 0.13) * k, tyre, tyre);
    let bars = p(0.45, 1.0);
    m.limb(head, bars, 0.025 * k, paint);
    m.limb(bars - r * 0.22 * k, bars + r * 0.22 * k, 0.025 * k, tyre);
    for s in [0.0, std::f32::consts::PI] {
        let a = crank + s;
        m.limb(bracket, bracket + (f * a.cos() + Vec3::Y * a.sin()) * 0.17 * k, 0.025 * k, [0.6, 0.6, 0.62]);
    }
}
