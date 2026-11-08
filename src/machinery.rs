//! The moving parts of the landmarks, rebuilt each frame for those in
//! view: windmill sails turning in the wind, and after dark the lighthouse
//! lamps sweeping their beams out across the water.

use glam::{Quat, Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::landmarks::{self, Kind};
use crate::props::{ground, hash};
use crate::terrain::{self, WATER_LEVEL};

/// Windmills and lighthouses are drawn moving out to this far.
const SAIL_DRAW: f32 = 2500.0;
const BEAM_DRAW: f32 = 5000.0;
/// A sail's reach from the hub, and its width.
const SAIL_LENGTH: f32 = 20.0;
const SAIL_WIDTH: f32 = 2.2;
/// Longest a beam's visible shaft runs.
const BEAM_LENGTH: f32 = 600.0;

const WOOD: [f32; 4] = [0.45, 0.3, 0.18, 1.0];
const CLOTH: [f32; 4] = [0.85, 0.8, 0.7, 1.0];

/// Where a windmill's hub is and which way it faces (as `props` builds the tower).
pub fn windmill_hub(p: Vec2) -> (Vec3, f32) {
    let yaw = hash(p.x, p.y) * std::f32::consts::TAU;
    let base = vec3(p.x, ground(p, 8.0) - 1.0, p.y);
    (base + Vec3::Y * 24.0 + Quat::from_rotation_y(yaw) * vec3(0.0, 0.0, -6.0), yaw)
}

/// Where a lighthouse's lamp is (as `props` builds the tower).
pub fn lighthouse_lamp(p: Vec2) -> Vec3 {
    vec3(p.x, ground(p, 7.0) - 1.0 + 3.0 + 36.0 + 0.8 + 2.0, p.y)
}

#[derive(Default)]
pub struct Mesh {
    pub verts: Vec<Vertex>,
    pub idxs: Vec<u16>,
}

impl Mesh {
    fn quad(&mut self, p: [Vec3; 4], color: [[f32; 4]; 4]) {
        if self.verts.len() > 65000 {
            return;
        }
        let base = self.verts.len() as u16;
        for (q, c) in p.into_iter().zip(color) {
            self.verts.push(Vertex { pos: q.into(), color: c });
        }
        // Both faces
        self.idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3, base, base + 2, base + 1, base, base + 3, base + 2]);
    }
}

/// The sails (drawn solid), and the beams (drawn as see-through light),
/// `night` 0 by day to 1 after dark.
pub fn build(camera: Vec3, time: f32, night: f32, keepers: &Keepers) -> (Mesh, Mesh) {
    let cam = vec2(camera.x, camera.z);
    let mut solid = Mesh::default();
    let mut light = Mesh::default();
    for l in landmarks::all() {
        let d = l.pos.distance(cam);
        match l.kind {
            Kind::Windmill if d < SAIL_DRAW => {
                let (hub, yaw) = windmill_hub(l.pos);
                let r = Quat::from_rotation_y(yaw);
                // Turning steadily, a little faster in the gusts
                let seed = hash(l.pos.y, l.pos.x) * 10.0;
                let turn = time * 0.55 + 0.6 * (time * 0.13 + seed).sin() + seed;
                for k in 0..4 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_2 + turn;
                    let dir = r * vec3(a.cos(), a.sin(), 0.0);
                    let across = r * vec3(-a.sin(), a.cos(), 0.0) * SAIL_WIDTH;
                    let tip = hub + dir * SAIL_LENGTH;
                    let root = hub + dir * 3.0;
                    // The wooden stock, then the cloth on its lattice, set
                    // just behind it
                    let back = r * vec3(0.0, 0.0, 0.15);
                    let stock = across * 0.12;
                    solid.quad([hub - stock, tip - stock, tip + stock, hub + stock], [WOOD; 4]);
                    solid.quad([root + back, tip + back, tip + across + back, root + across + back], [CLOTH; 4]);
                }
            }
            _ => {}
        }
    }
    if night > 0.05 {
        for k in keepers.lit() {
            beam(&mut light, k, camera, night);
        }
    }
    (solid, light)
}

// ── Lighthouses ──
//
// Each lighthouse is kept: its lamp sweeps the water a stretch at a time,
// pausing on a spot before moving on, and when something moves near its
// beam (a boat, or a car) it takes an interest and follows it for a while.

/// How fast the lamp swings when scanning, and when following something
/// (radians a second, at most).
const SCAN_TURN: f32 = 0.45;
const TRACK_TURN: f32 = 1.1;
/// It notices movement this far off, this close to where it's pointing.
const NOTICE_RANGE: f32 = 520.0;
const NOTICE_ANGLE: f32 = 0.7;
/// It looks this far out over the water when scanning.
const SCAN_REACH: (f32, f32) = (110.0, 380.0);
/// The beam's cone (half angle, degrees) as a light, and how bright.
const SPOT_ANGLE: f32 = 5.0;
const SPOT_POWER: f32 = 9.0;

#[derive(Clone, Copy, PartialEq)]
enum Mind {
    /// Easing toward a spot on the water, then resting on it a while.
    Scanning { at: Vec3, rest: f32 },
    /// Following whatever moves at this index of the movers, for so long.
    Following { who: usize, left: f32 },
}

pub struct Keeper {
    lamp: Vec3,
    /// Where the beam points.
    aim: Vec3,
    mind: Mind,
    /// Spots over the water it may look at.
    sea: Vec<Vec3>,
    /// Not taking any interest in movement until this runs out.
    bored: f32,
    /// Until it next looks round for movement.
    glance: f32,
}

/// The lighthouses near enough to matter, each with a mind of its own.
#[derive(Default)]
pub struct Keepers {
    keepers: Vec<(usize, Keeper)>,
    rng: u32,
}

impl Keepers {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Where each beam is pointing.
    #[cfg(test)]
    pub fn aims(&self) -> Vec<Vec3> {
        self.keepers.iter().map(|(_, k)| k.aim).collect()
    }

    /// The lamps with something to light: the nearest first.
    pub fn lit(&self) -> impl Iterator<Item = &Keeper> {
        self.keepers.iter().map(|(_, k)| k)
    }

    /// Where each lit beam lands, as a spot light: nearest the camera first.
    pub fn spot_lights(&self, camera: Vec3, night: f32, max: usize) -> Vec<crate::lights::Light> {
        let mut list: Vec<&Keeper> = self.keepers.iter().map(|(_, k)| k).collect();
        list.sort_by(|a, b| a.aim.distance_squared(camera).total_cmp(&b.aim.distance_squared(camera)));
        list.into_iter().take(max).filter(|k| k.aim.distance(camera) < 1600.0).map(|k| {
            let reach = k.lamp.distance(k.aim) * 1.5 + 120.0;
            crate::lights::Light::spot(k.lamp, k.aim - k.lamp, reach, SPOT_ANGLE, vec3(1.0, 0.95, 0.8), SPOT_POWER * night)
        }).collect()
    }

    /// Think, for the lighthouses within reach of `camera`. `movers` is
    /// everything moving that might catch a keeper's eye.
    pub fn update(&mut self, camera: Vec3, movers: &[Vec3], dt: f32) {
        let cam = vec2(camera.x, camera.z);
        let all = landmarks::all();
        self.keepers.retain(|(i, _)| all[*i].pos.distance(cam) < BEAM_DRAW + 200.0);
        for (i, l) in all.iter().enumerate() {
            if l.kind == Kind::Lighthouse && l.pos.distance(cam) < BEAM_DRAW && !self.keepers.iter().any(|(j, _)| *j == i) {
                let keeper = Keeper::new(l.pos);
                if !keeper.sea.is_empty() {
                    self.keepers.push((i, keeper));
                }
            }
        }
        for n in 0..self.keepers.len() {
            let r = [self.rand(), self.rand(), self.rand()];
            self.keepers[n].1.think(movers, dt, r);
        }
    }
}

impl Keeper {
    fn new(p: Vec2) -> Self {
        let lamp = lighthouse_lamp(p);
        let water = WATER_LEVEL + 0.3;
        let mut sea = Vec::new();
        for k in 0..48 {
            let dir = Vec2::from_angle(k as f32 * std::f32::consts::TAU / 48.0);
            for d in [SCAN_REACH.0, (SCAN_REACH.0 + SCAN_REACH.1) * 0.5, SCAN_REACH.1] {
                let q = p + dir * d;
                if terrain::raw_height(q.x, q.y) < WATER_LEVEL - 1.0 {
                    sea.push(vec3(q.x, water, q.y));
                }
            }
        }
        let aim = sea.first().copied().unwrap_or(lamp + Vec3::X * 100.0);
        Self { lamp, aim, mind: Mind::Scanning { at: aim, rest: 1.0 }, sea, bored: 0.0, glance: 0.0 }
    }

    fn think(&mut self, movers: &[Vec3], dt: f32, r: [f32; 3]) {
        self.bored -= dt;
        self.glance -= dt;
        // Now and then look round for anything moving near the beam
        if self.glance <= 0.0 {
            self.glance = 0.6;
            if self.bored <= 0.0 && !matches!(self.mind, Mind::Following { .. }) {
                let facing = flat(self.aim - self.lamp);
                let caught = movers.iter().enumerate()
                    .filter(|(_, m)| flat(**m - self.lamp).length() < NOTICE_RANGE)
                    .map(|(i, m)| (i, flat(*m - self.lamp).normalize_or_zero().angle_between(facing).abs()))
                    .filter(|&(_, a)| a < NOTICE_ANGLE)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((who, _)) = caught.filter(|_| r[0] < 0.6) {
                    self.mind = Mind::Following { who, left: 5.0 + 6.0 * r[1] };
                }
            }
        }
        let (target, turn) = match &mut self.mind {
            Mind::Following { who, left } => {
                *left -= dt;
                match movers.get(*who) {
                    Some(&m) if *left > 0.0 && flat(m - self.lamp).length() < NOTICE_RANGE * 1.2 => (m, TRACK_TURN),
                    _ => {
                        // Lost interest: back to the water, and leave things be a while
                        self.bored = 8.0 + 8.0 * r[2];
                        let at = self.next_spot(r);
                        self.mind = Mind::Scanning { at, rest: 1.0 };
                        (at, SCAN_TURN)
                    }
                }
            }
            Mind::Scanning { at, rest } => {
                let target = *at;
                // Arrived: rest a moment, then pick the next spot
                if flat(self.aim - target).length() < 6.0 {
                    *rest -= dt;
                    if *rest <= 0.0 {
                        let next = self.next_spot(r);
                        self.mind = Mind::Scanning { at: next, rest: 0.4 + 2.2 * r[2] };
                    }
                }
                (target, SCAN_TURN)
            }
        };
        self.swing_toward(target, turn, dt);
    }

    /// Somewhere else on the water: mostly along from where it's looking,
    /// sometimes right across.
    fn next_spot(&self, r: [f32; 3]) -> Vec3 {
        let facing = flat(self.aim - self.lamp);
        let near: Vec<&Vec3> = self.sea.iter().filter(|q| {
            let a = flat(**q - self.lamp).angle_between(facing).abs();
            (0.25..0.9).contains(&a)
        }).collect();
        if r[0] < 0.8 && !near.is_empty() {
            *near[(r[1] * near.len() as f32) as usize % near.len()]
        } else {
            self.sea[(r[1] * self.sea.len() as f32) as usize % self.sea.len()]
        }
    }

    /// Swing the beam round toward `target` and run it out (or in) to it,
    /// no faster than `turn`.
    fn swing_toward(&mut self, target: Vec3, turn: f32, dt: f32) {
        let now = flat(self.aim - self.lamp);
        let want = flat(target - self.lamp);
        let a = now.angle_between(want).clamp(-turn * dt, turn * dt);
        // Ease in as it comes round
        let a = a * (now.angle_between(want).abs() * 3.0).min(1.0);
        let len = now.length() + (want.length() - now.length()) * (dt * 1.5).min(1.0);
        let dir = Vec2::from_angle(a).rotate(now.normalize_or_zero()) * len;
        let y = self.aim.y + (target.y - self.aim.y) * (dt * 2.0).min(1.0);
        self.aim = vec3(self.lamp.x + dir.x, y, self.lamp.z + dir.y);
    }
}

fn flat(v: Vec3) -> Vec2 {
    vec2(v.x, v.z)
}

/// The visible shaft from the lamp to past where it lands, in steps (colours
/// are flat per triangle): a bright core and softer bands either side.
fn beam(light: &mut Mesh, k: &Keeper, camera: Vec3, night: f32) {
    let dir = (k.aim - k.lamp).normalize_or_zero();
    // Running on a little beyond the spot it lights, fading as it goes
    let length = (k.lamp.distance(k.aim) * 1.25).min(BEAM_LENGTH);
    let c = [1.5, 1.4, 1.15];
    let mid = k.lamp + dir * length * 0.3;
    let spread = dir.cross(camera - mid).try_normalize().unwrap_or(Vec3::X);
    const STEPS: usize = 14;
    const BANDS: [(f32, f32, f32); 3] = [(0.0, 0.3, 1.0), (0.3, 0.65, 0.5), (0.65, 1.0, 0.2)];
    let cone = SPOT_ANGLE.to_radians().tan() * 1.6;
    for s in 0..STEPS {
        let (f0, f1) = (s as f32 / STEPS as f32, (s + 1) as f32 / STEPS as f32);
        let fade = 0.3 * (1.0 - (f0 + f1) * 0.5).powf(1.6) * night;
        let (p0, p1) = (k.lamp + dir * length * f0, k.lamp + dir * length * f1);
        let (h0, h1) = (0.6 + length * f0 * cone, 0.6 + length * f1 * cone);
        for (inner, outer, share) in BANDS {
            let col = [c[0], c[1], c[2], fade * share];
            for e in [-1.0, 1.0] {
                let q = |p: Vec3, h: f32, at: f32| p + spread * h * at * e;
                light.quad([q(p0, h0, inner), q(p1, h1, inner), q(p1, h1, outer), q(p0, h0, outer)], [col; 4]);
            }
        }
    }
    // The lamp itself, a bright spot whichever way you look
    let glow = [2.0, 1.9, 1.5, 0.9 * night];
    let to_cam = (camera - k.lamp).normalize_or_zero();
    let right = to_cam.cross(Vec3::Y).normalize_or_zero() * 2.2;
    let up = right.cross(to_cam).normalize_or_zero() * 2.2;
    light.quad([k.lamp - right - up, k.lamp + right - up, k.lamp + right + up, k.lamp - right + up], [glow; 4]);
}
