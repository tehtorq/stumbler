//! Work on the farms: a tractor ploughing up and down a ploughed field,
//! dust rolling off behind it and gulls wheeling over the fresh furrows;
//! a combine harvester working through the wheat in a cloud of chaff.
//! They work the daylight hours and stand at the field's gate overnight.
//!
//! Where a machine is follows from the clock alone, so nothing needs
//! keeping: it runs along a row, swings round on the headland into the
//! next, and when the field's done works back the way it came.

use glam::{Vec2, Vec3, vec2, vec3};

use crate::farms::{self, Crop, Farm, Field};
use crate::geometry::Vertex;
use crate::particles::Particles;
use crate::props::{hash, PERSON_SCALE};
use crate::terrain::terrain_height;
use crate::townsfolk::{Mesh, Pose};

/// Machines are worked out to this far from the player, drawn to the next.
const NEAR: f32 = 900.0;
const DRAW: f32 = 800.0;
/// Pace across the field (a unit is half a metre).
const PLOUGH_PACE: f32 = 4.5;
const COMBINE_PACE: f32 = 3.5;
/// The width each pass covers.
const PLOUGH_WIDTH: f32 = 7.0;
const COMBINE_WIDTH: f32 = 13.0;
/// Kept in from the field's edge.
const HEADLAND: f32 = 9.0;

#[derive(Clone, Copy, PartialEq)]
enum Machine {
    Tractor,
    Combine,
}

/// Where a machine working this field is at `t` seconds, which way it's
/// heading, and whether it's on a straight row (working) or turning.
fn place(farm: &Farm, field: &Field, width: f32, pace: f32, t: f32) -> (Vec2, Vec2, bool) {
    let len = (field.half.x - HEADLAND) * 2.0;
    let rows = (((field.half.y - HEADLAND) * 2.0 / width).floor() as usize).max(1);
    let r = width * 0.5;
    let turn = std::f32::consts::PI * r;
    let lap = rows as f32 * len + (rows - 1) as f32 * turn;
    // There and back again
    let s = (t * pace).rem_euclid(lap * 2.0);
    let (s, back) = if s < lap { (s, false) } else { (lap * 2.0 - s, true) };
    let k = ((s / (len + turn)).floor() as usize).min(rows - 1);
    let into = s - k as f32 * (len + turn);
    let dir = if k % 2 == 0 { 1.0 } else { -1.0 };
    let row_y = -field.half.y + HEADLAND + r + k as f32 * width;
    let x0 = -dir * len * 0.5;
    let (local, heading, working) = if into <= len {
        (vec2(x0 + dir * into, row_y), vec2(dir, 0.0), true)
    } else {
        // Round the headland into the next row
        let a = (into - len) / r;
        let centre = vec2(dir * len * 0.5, row_y + r);
        (centre + vec2(dir * a.sin() * r, -a.cos() * r), vec2(dir * a.cos(), a.sin()), false)
    };
    let heading = if back { -heading } else { heading };
    let world = field.centre + farm.along * local.x + farm.out * local.y;
    let dir = (farm.along * heading.x + farm.out * heading.y).normalize_or_zero();
    (world, dir, working)
}

/// The machines at work near `player` at `clock` (0..1 from midnight),
/// `time` the running seconds: the farm, its field, which machine, where,
/// facing, and whether at work just now.
fn working(player: Vec3, clock: f32, time: f32) -> Vec<(Machine, Vec2, Vec2, bool, f32)> {
    let me = vec2(player.x, player.z);
    let h = clock * 24.0;
    let daylight = (7.0..20.0).contains(&h);
    let mut list = Vec::new();
    for farm in farms::all().iter().filter(|f| f.yard.distance(me) < NEAR + 300.0) {
        for field in &farm.fields {
            let machine = match field.crop {
                Crop::Ploughed => Machine::Tractor,
                Crop::Wheat if hash(field.seed, 1.0) < 0.5 => Machine::Combine,
                _ => continue,
            };
            if field.centre.distance(me) > NEAR || field.half.x < HEADLAND * 2.0 || field.half.y < HEADLAND * 2.0 {
                continue;
            }
            let (width, pace) = if machine == Machine::Tractor { (PLOUGH_WIDTH, PLOUGH_PACE) } else { (COMBINE_WIDTH, COMBINE_PACE) };
            if daylight {
                let t = time + hash(field.seed, 2.0) * 10_000.0;
                let (at, dir, busy) = place(farm, field, width, pace, t);
                list.push((machine, at, dir, busy, field.seed));
            } else {
                // Left by the gate for the night, facing the yard
                let corner = field.centre - farm.along * (field.half.x - HEADLAND) - farm.out * (field.half.y - HEADLAND);
                list.push((machine, corner, farm.along, false, field.seed));
            }
        }
    }
    list
}

/// The machines at work near the player: where, and firing strokes a second.
pub fn engines(player: Vec3, clock: f32, time: f32) -> Vec<(Vec3, f32)> {
    working(player, clock, time).into_iter().filter(|w| w.3).map(|(machine, at, ..)| {
        let rate = if machine == Machine::Tractor { 13.0 } else { 11.0 };
        (vec3(at.x, terrain_height(at.x, at.y) + 2.0, at.y), rate)
    }).collect()
}

/// The day's work: dust off the machines that are busy.
pub fn update(particles: &mut Particles, player: Vec3, clock: f32, time: f32, dt: f32) {
    for (machine, at, dir, busy, _) in working(player, clock, time) {
        if !busy || at.distance(vec2(player.x, player.z)) > 400.0 {
            continue;
        }
        let behind = at - dir * if machine == Machine::Tractor { 9.0 } else { 6.0 };
        let y = terrain_height(behind.x, behind.y);
        let (color, rate) = match machine {
            Machine::Tractor => ([0.5, 0.38, 0.26], 6.0),
            Machine::Combine => ([0.85, 0.75, 0.5], 9.0),
        };
        particles.field_dust(vec3(behind.x, y, behind.y), color, rate, dt);
    }
}

/// Every machine near enough to see.
pub fn build_mesh(player: Vec3, camera: Vec3, clock: f32, time: f32) -> (Vec<Vertex>, Vec<u16>) {
    let cam = vec2(camera.x, camera.z);
    let mut m = Mesh::default();
    for (machine, at, dir, busy, seed) in working(player, clock, time) {
        if at.distance(cam) > DRAW {
            continue;
        }
        let ground = terrain_height(at.x, at.y);
        let base = vec3(at.x, ground, at.y);
        match machine {
            Machine::Tractor => {
                tractor(&mut m, base, dir, time, seed, busy);
                if busy {
                    gulls(&mut m, base - vec3(dir.x, 0.0, dir.y) * 12.0, time, seed);
                }
            }
            Machine::Combine => combine(&mut m, base, dir, time, seed, busy),
        }
    }
    (m.verts, m.idxs)
}

/// A frame for building a machine: `p(along, up, across)` in metres.
struct Frame {
    base: Vec3,
    f: Vec3,
    r: Vec3,
}

impl Frame {
    fn new(base: Vec3, dir: Vec2) -> Self {
        Self { base, f: vec3(dir.x, 0.0, dir.y), r: vec3(dir.y, 0.0, -dir.x) }
    }
    fn p(&self, along: f32, up: f32, across: f32) -> Vec3 {
        self.base + (self.f * along + Vec3::Y * up + self.r * across) * PERSON_SCALE
    }
    fn block(&self, m: &mut Mesh, along: f32, up: f32, across: f32, half: Vec3, side: [f32; 3], top: [f32; 3]) {
        m.box_at(self.p(along, up, across), self.r, self.f, half * PERSON_SCALE, side, top);
    }
    /// A wheel on the axle at `along`, `across`, turned to `spin`.
    fn wheel(&self, m: &mut Mesh, along: f32, across: f32, radius: f32, width: f32, spin: f32) {
        let tyre = [0.1, 0.1, 0.1];
        let hub = [0.85, 0.7, 0.15];
        let c = self.p(along, radius, across);
        let sides = 10;
        for k in 0..sides {
            let (a0, a1) = (spin + k as f32 / sides as f32 * std::f32::consts::TAU, spin + (k + 1) as f32 / sides as f32 * std::f32::consts::TAU);
            let rim = |a: f32, o: f32| c + (self.f * a.cos() + Vec3::Y * a.sin()) * radius * PERSON_SCALE + self.r * o * PERSON_SCALE;
            let w = width * 0.5;
            m.quad(rim(a0, -w), rim(a1, -w), rim(a1, w), rim(a0, w), tyre);
            // The sidewalls, and a chunky tread every other facet
            m.tri(c - self.r * w * PERSON_SCALE, rim(a1, -w), rim(a0, -w), if k % 2 == 0 { tyre } else { [0.16, 0.16, 0.16] });
            m.tri(c + self.r * w * PERSON_SCALE, rim(a0, w), rim(a1, w), tyre);
        }
        m.box_at(c, self.r, self.f, vec3(width * 0.52, radius * 0.4, radius * 0.4) * PERSON_SCALE, hub, hub);
    }
}

/// A tractor with its plough down behind.
fn tractor(m: &mut Mesh, base: Vec3, dir: Vec2, time: f32, seed: f32, busy: bool) {
    const PAINT: [[f32; 3]; 4] = [[0.2, 0.5, 0.2], [0.8, 0.15, 0.1], [0.15, 0.35, 0.65], [0.9, 0.6, 0.1]];
    let paint = PAINT[(hash(seed, 3.0) * 4.0) as usize % 4];
    let dark = [0.15, 0.15, 0.16];
    let glass = [0.3, 0.42, 0.5];
    let fr = Frame::new(base, dir);
    let spin = if busy { -time * PLOUGH_PACE / (0.8 * PERSON_SCALE) } else { 0.0 };
    // Big wheels at the back, small at the front
    for s in [-1.0, 1.0] {
        fr.wheel(m, -0.8, s * 0.9, 0.8, 0.5, spin);
        fr.wheel(m, 1.4, s * 0.8, 0.45, 0.3, spin * 1.8);
    }
    // Bonnet, body and cab
    fr.block(m, 0.8, 1.0, 0.0, vec3(0.45, 0.45, 1.0), paint, paint);
    fr.block(m, -0.6, 1.1, 0.0, vec3(0.5, 0.35, 0.7), paint, paint);
    fr.block(m, -0.7, 2.05, 0.0, vec3(0.62, 0.6, 0.62), glass, dark);
    fr.block(m, -0.7, 2.7, 0.0, vec3(0.7, 0.06, 0.7), paint, paint);
    // The driver at the wheel, and the exhaust stack
    m.figure(fr.p(-0.75, 0.85, 0.0), dir, 0.0, Pose::Sitting, seed + 5.0);
    m.limb(fr.p(1.0, 1.4, 0.3), fr.p(1.0, 2.6, 0.3), 0.06 * PERSON_SCALE, dark);
    // The plough on its hitch: a beam out behind with blades turning the soil
    let steel = [0.55, 0.56, 0.58];
    m.limb(fr.p(-1.5, 0.8, 0.0), fr.p(-2.4, 0.6, 0.0), 0.08 * PERSON_SCALE, paint);
    m.limb(fr.p(-2.4, 0.6, -1.2), fr.p(-4.6, 0.6, 1.2), 0.1 * PERSON_SCALE, paint);
    for k in 0..4 {
        let t = k as f32 / 3.0;
        let along = -2.6 - 1.8 * t;
        let across = -1.0 + 2.0 * t;
        m.limb(fr.p(along, 0.6, across), fr.p(along - 0.3, if busy { -0.05 } else { 0.25 }, across + 0.2), 0.07 * PERSON_SCALE, steel);
    }
}

/// A combine harvester with its reel turning at the front.
fn combine(m: &mut Mesh, base: Vec3, dir: Vec2, time: f32, seed: f32, busy: bool) {
    const PAINT: [[f32; 3]; 3] = [[0.2, 0.5, 0.18], [0.75, 0.12, 0.1], [0.85, 0.82, 0.75]];
    let paint = PAINT[(hash(seed, 4.0) * 3.0) as usize % 3];
    let dark = [0.15, 0.15, 0.16];
    let glass = [0.3, 0.42, 0.5];
    let fr = Frame::new(base, dir);
    let spin = if busy { -time * COMBINE_PACE / (0.85 * PERSON_SCALE) } else { 0.0 };
    for s in [-1.0, 1.0] {
        fr.wheel(m, 0.8, s * 1.5, 0.85, 0.7, spin);
        fr.wheel(m, -2.6, s * 1.3, 0.55, 0.4, spin * 1.5);
    }
    // The body with its grain tank, the cab up front, a pipe to unload
    fr.block(m, -0.8, 1.9, 0.0, vec3(2.3, 1.1, 1.3), paint, paint);
    fr.block(m, -0.4, 3.3, 0.0, vec3(1.3, 0.3, 1.2), paint, [0.8, 0.68, 0.4]);
    fr.block(m, 1.4, 3.2, -0.3, vec3(0.6, 0.6, 0.7), glass, dark);
    fr.block(m, 1.4, 3.85, -0.3, vec3(0.68, 0.06, 0.78), paint, paint);
    m.figure(fr.p(1.35, 2.3, -0.3), dir, 0.0, Pose::Sitting, seed + 6.0);
    m.limb(fr.p(-0.6, 3.4, 1.2), fr.p(-1.2, 3.6, 3.6), 0.15 * PERSON_SCALE, paint);
    // The header: a wide cutter bar with the reel turning over it
    fr.block(m, 2.8, 0.55, 0.0, vec3(0.5, 0.35, 3.1), paint, paint);
    m.limb(fr.p(1.9, 1.0, 0.0), fr.p(2.6, 0.7, 0.0), 0.35 * PERSON_SCALE, paint);
    let reel = fr.p(3.1, 1.35, 0.0);
    for k in 0..5 {
        let a = time * 2.4 * if busy { 1.0 } else { 0.0 } + k as f32 * std::f32::consts::TAU / 5.0;
        let o = (fr.f * a.cos() + Vec3::Y * a.sin()) * 0.45 * PERSON_SCALE;
        m.limb(reel + o - fr.r * 3.0 * PERSON_SCALE, reel + o + fr.r * 3.0 * PERSON_SCALE, 0.04 * PERSON_SCALE, [0.85, 0.75, 0.2]);
    }
    for s in [-1.0, 1.0] {
        m.limb(reel - fr.r * s * 3.0 * PERSON_SCALE, fr.p(1.6, 1.8, s * 2.6), 0.06 * PERSON_SCALE, dark);
    }
}

/// Gulls following the plough, dropping onto the turned earth.
fn gulls(m: &mut Mesh, over: Vec3, t: f32, seed: f32) {
    for k in 0..6 {
        let s = seed * 13.0 + k as f32 * 2.3;
        let a = t * (0.5 + 0.2 * (s * 1.7).sin().abs()) * if k % 2 == 0 { 1.0 } else { -1.0 } + s;
        let r = 4.0 + 6.0 * (s * 0.9).sin().abs();
        let c = over + vec3(a.cos() * r, 4.0 + 5.0 * (s * 2.1).sin().abs() + (t * 0.8 + s).sin() * 1.5, a.sin() * r);
        let along = vec3(-a.sin(), 0.0, a.cos()) * if k % 2 == 0 { 1.0 } else { -1.0 };
        let side = vec3(along.z, 0.0, -along.x);
        let flap = (t * 4.5 + s * 5.0).sin() * 0.4;
        for w in [-1.0, 1.0] {
            let tip = c + side * w * 1.3 + Vec3::Y * flap;
            m.tri(c + along * 0.35, c - along * 0.3, tip - along * 0.15, [0.82, 0.84, 0.86]);
        }
        m.tri(c + along * 0.55, c - along * 0.6 + side * 0.14, c - along * 0.6 - side * 0.14, [0.95, 0.95, 0.93]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_keep_to_their_fields() {
        let _ = crate::sim_tests::network();
        let mut fields = 0;
        for farm in farms::all() {
            for field in farm.fields.iter().filter(|f| matches!(f.crop, Crop::Ploughed | Crop::Wheat)) {
                if field.half.x < HEADLAND * 2.0 || field.half.y < HEADLAND * 2.0 {
                    continue;
                }
                fields += 1;
                let combine = field.crop == Crop::Wheat && hash(field.seed, 1.0) < 0.5;
                if fields < 40 && (field.crop == Crop::Ploughed || combine) && fields % 8 == 1 {
                    let eye = field.centre - farm.along * (field.half.x + 20.0);
                    let look = field.centre - eye;
                    println!("a {:?} field at {:.0},{:.0}: STUMBLER_AT={:.0},{:.0},{:.0}", field.crop, field.centre.x, field.centre.y, eye.x, eye.y, (-look.x).atan2(-look.y).to_degrees());
                }
                for k in 0..2000 {
                    let (at, dir, _) = place(farm, field, PLOUGH_WIDTH, PLOUGH_PACE, k as f32 * 3.1);
                    let l = field.local(farm, at);
                    assert!(l.x.abs() <= field.half.x && l.y.abs() <= field.half.y, "a tractor left its field: {l} in {}", field.half);
                    assert!((dir.length() - 1.0).abs() < 0.01);
                }
            }
        }
        println!("{fields} fields worked");
        assert!(fields > 5);
    }
}
