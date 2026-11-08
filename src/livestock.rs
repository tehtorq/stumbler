//! The flocks and herds out at pasture, alive: sheep grazing and drifting,
//! bunching and scurrying off from a car passing close, and on some farms
//! a sheepdog working round them while the farmer watches from the corner;
//! cows ambling as they graze, and at milking time walking to the corner of
//! the field nearest the yard and waiting there. Each stays in its field.

use glam::{Vec2, Vec3, vec2, vec3};

use crate::collisions::Solid;
use crate::farms::{self, Crop, Farm, Field};
use crate::geometry::Vertex;
use crate::props::{hash, PERSON_SCALE};
use crate::terrain::terrain_height;
use crate::townsfolk::{Mesh, Pose};

/// Fields are stocked inside this distance of the player, emptied beyond the next.
const NEAR: f32 = 550.0;
const GONE: f32 = 750.0;
const DRAW: f32 = 500.0;
/// Kept this far in from the hedges.
const MARGIN: f32 = 5.0;

#[derive(Clone, Copy, PartialEq)]
enum Doing {
    Grazing(f32),
    Walking(Vec2),
    /// Scurrying from the car, this way, for so long more.
    Fleeing(Vec2, f32),
}

struct Animal {
    /// Where in the field (along the farm's road, out from it, from the field's centre).
    at: Vec2,
    heading: f32,
    doing: Doing,
    phase: f32,
    seed: f32,
}

struct Herd {
    farm: usize,
    field: usize,
    sheep: bool,
    animals: Vec<Animal>,
    /// A sheepdog working the flock: how far round it is.
    dog: Option<f32>,
}

#[derive(Default)]
pub struct Livestock {
    herds: Vec<Herd>,
    rng: u32,
}

fn world(farm: &Farm, field: &Field, l: Vec2) -> Vec2 {
    field.centre + farm.along * l.x + farm.out * l.y
}

/// Whether it's milking time at `clock` (0..1 from midnight).
fn milking(clock: f32) -> bool {
    let h = clock * 24.0;
    (6.0..7.5).contains(&h) || (16.0..17.5).contains(&h)
}

impl Livestock {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223) | 1;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn update(&mut self, player: Vec3, player_vel: Vec3, clock: f32, dt: f32) {
        let me = vec2(player.x, player.z);
        let speed = vec2(player_vel.x, player_vel.z).length();
        let all = farms::all();
        self.herds.retain(|h| all[h.farm].fields[h.field].centre.distance(me) < GONE);
        for (fi, farm) in all.iter().enumerate() {
            if farm.yard.distance(me) > NEAR + 400.0 {
                continue;
            }
            for (pi, field) in farm.fields.iter().enumerate().filter(|(_, f)| f.crop == Crop::Pasture) {
                if field.centre.distance(me) < NEAR && !self.herds.iter().any(|h| h.farm == fi && h.field == pi) {
                    let sheep = hash(farm.seed, 11.0) < 0.5;
                    let count = 6 + (hash(field.seed, 12.0) * 5.0) as usize + if sheep { 4 } else { 0 };
                    let animals = (0..count).map(|k| {
                        let seed = field.seed + k as f32 * 3.7;
                        let at = vec2((hash(seed, 1.0) - 0.5) * 2.0 * (field.half.x - MARGIN), (hash(seed, 2.0) - 0.5) * 2.0 * (field.half.y - MARGIN));
                        Animal { at, heading: hash(seed, 3.0) * 6.28, doing: Doing::Grazing(hash(seed, 4.0) * 8.0), phase: hash(seed, 5.0) * 10.0, seed }
                    }).collect();
                    let dog = (sheep && hash(field.seed, 13.0) < 0.5).then_some(0.0);
                    self.herds.push(Herd { farm: fi, field: pi, sheep, animals, dog });
                }
            }
        }
        let milk = milking(clock);
        for h in 0..self.herds.len() {
            let rolls: Vec<f32> = (0..self.herds[h].animals.len() * 2).map(|_| self.rand()).collect();
            let herd = &mut self.herds[h];
            let farm = &all[herd.farm];
            let field = &farm.fields[herd.field];
            let bound = vec2(field.half.x - MARGIN, field.half.y - MARGIN);
            // The car, in the field's frame
            let rel = me - field.centre;
            let car = vec2(rel.dot(farm.along), rel.dot(farm.out));
            let centroid = herd.animals.iter().map(|a| a.at).sum::<Vec2>() / herd.animals.len().max(1) as f32;
            // The corner nearest the yard, where the cows wait to be milked
            let to_yard = farm.yard - field.centre;
            let gate = vec2(to_yard.dot(farm.along).signum() * (bound.x - 2.0), to_yard.dot(farm.out).signum() * (bound.y - 2.0));
            // The dog runs round the flock, keeping it together
            let dog_at = herd.dog.map(|a| {
                let spread = herd.animals.iter().map(|x| x.at.distance(centroid)).fold(0.0, f32::max);
                centroid + Vec2::from_angle(a) * (spread + 7.0)
            });
            if let Some(a) = herd.dog.as_mut() {
                *a += dt * 0.35;
            }
            for (k, an) in herd.animals.iter_mut().enumerate() {
                let (r0, r1) = (rolls[k * 2], rolls[k * 2 + 1]);
                let wary = if herd.sheep { 45.0 } else { 18.0 };
                if speed > 4.0 && an.at.distance(car) < wary && !matches!(an.doing, Doing::Fleeing(..)) {
                    let away = (an.at - car).normalize_or_zero();
                    // Sheep run off together, away and toward the flock
                    let dir = if herd.sheep { (away + (centroid - an.at).normalize_or_zero() * 0.5).normalize_or_zero() } else { away };
                    an.doing = Doing::Fleeing(dir, 2.0 + r0);
                }
                if let Some(d) = dog_at.filter(|d| d.distance(an.at) < 9.0) {
                    if !matches!(an.doing, Doing::Fleeing(..)) {
                        an.doing = Doing::Fleeing((an.at - d).normalize_or_zero(), 0.8);
                    }
                }
                let (pace, step) = if herd.sheep { (1.2, 6.0) } else { (1.0, 3.5) };
                let mut moving = 0.0;
                an.doing = match an.doing {
                    _ if !herd.sheep && milk && !matches!(an.doing, Doing::Fleeing(..)) => {
                        // Off to the gate, waiting in a loose knot
                        let spot = gate + Vec2::from_angle(an.seed) * (2.0 + (an.seed % 3.0) * 2.0);
                        if an.at.distance(spot) > 1.0 { Doing::Walking(spot) } else { Doing::Grazing(1.0) }
                    }
                    Doing::Grazing(t) if t <= 0.0 => {
                        // Off to fresh grass, keeping near the others
                        let pull = if herd.sheep { (centroid - an.at) * 0.3 } else { Vec2::ZERO };
                        let to = an.at + Vec2::from_angle(r0 * 6.28) * (4.0 + 8.0 * r1) + pull;
                        Doing::Walking(to.clamp(-bound, bound))
                    }
                    Doing::Grazing(t) => Doing::Grazing(t - dt),
                    Doing::Walking(to) if an.at.distance(to) < 0.8 => Doing::Grazing(3.0 + 8.0 * r0),
                    Doing::Fleeing(_, t) if t <= 0.0 => Doing::Grazing(1.0 + 3.0 * r0),
                    Doing::Fleeing(dir, t) => Doing::Fleeing(dir, t - dt),
                    other => other,
                };
                let dir = match an.doing {
                    Doing::Walking(to) => { moving = pace; (to - an.at).normalize_or_zero() }
                    Doing::Fleeing(d, _) => { moving = step; d }
                    _ => Vec2::ZERO,
                };
                if moving > 0.0 {
                    an.at = (an.at + dir * moving * dt).clamp(-bound, bound);
                    let want = dir.x.atan2(dir.y);
                    let mut diff = want - an.heading;
                    while diff > std::f32::consts::PI { diff -= std::f32::consts::TAU; }
                    while diff < -std::f32::consts::PI { diff += std::f32::consts::TAU; }
                    an.heading += diff.clamp(-dt * 4.0, dt * 4.0);
                }
                an.phase += dt * (1.0 + moving * 2.0);
            }
        }
    }

    /// Each animal: where in its field, the field's half size less the
    /// margin, whether it's running from something, and whether it's a sheep.
    #[cfg(test)]
    pub fn report(&self) -> Vec<(Vec2, Vec2, bool, bool)> {
        let all = farms::all();
        self.herds.iter().flat_map(|h| {
            let f = &all[h.farm].fields[h.field];
            h.animals.iter().map(move |a| (a.at, vec2(f.half.x - MARGIN, f.half.y - MARGIN), matches!(a.doing, Doing::Fleeing(..)), h.sheep))
        }).collect()
    }

    /// The cows, for cars to bump into.
    pub fn solids(&self, near: Vec3) -> Vec<(Solid, Vec3)> {
        let all = farms::all();
        let me = vec2(near.x, near.z);
        let mut list = Vec::new();
        for herd in self.herds.iter().filter(|h| !h.sheep) {
            let farm = &all[herd.farm];
            let field = &farm.fields[herd.field];
            for an in &herd.animals {
                let p = world(farm, field, an.at);
                if p.distance(me) < 80.0 {
                    list.push((Solid { centre: p, half: Vec2::splat(1.0 * PERSON_SCALE), yaw: 0.0, top: terrain_height(p.x, p.y) + 3.0, round: true }, Vec3::ZERO));
                }
            }
        }
        list
    }

    /// Every animal near enough to see (and the sheepdogs and their farmers).
    pub fn build_mesh(&self, camera: Vec3, time: f32) -> (Vec<Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let all = farms::all();
        let mut m = Mesh::default();
        for herd in &self.herds {
            let farm = &all[herd.farm];
            let field = &farm.fields[herd.field];
            if field.centre.distance(cam) > DRAW + field.half.length() {
                continue;
            }
            // In the field's frame, which way is which in the world
            let frame = |d: Vec2| (farm.along * d.x + farm.out * d.y).normalize_or_zero();
            for an in &herd.animals {
                let p = world(farm, field, an.at);
                let at = vec3(p.x, terrain_height(p.x, p.y), p.y);
                let dir = frame(vec2(an.heading.sin(), an.heading.cos()));
                let grazing = matches!(an.doing, Doing::Grazing(_));
                let moving = !grazing;
                if herd.sheep { sheep(&mut m, at, dir, grazing, moving, an.phase, an.seed) } else { cow(&mut m, at, dir, grazing, moving, an.phase, an.seed) }
            }
            if let Some(a) = herd.dog {
                let centroid = herd.animals.iter().map(|x| x.at).sum::<Vec2>() / herd.animals.len().max(1) as f32;
                let spread = herd.animals.iter().map(|x| x.at.distance(centroid)).fold(0.0, f32::max);
                let bound = vec2(field.half.x - 2.0, field.half.y - 2.0);
                let l = (centroid + Vec2::from_angle(a) * (spread + 7.0)).clamp(-bound, bound);
                let p = world(farm, field, l);
                let run = frame(vec2(-a.sin(), a.cos()));
                crate::picnics::dog_in(&mut m, vec3(p.x, terrain_height(p.x, p.y), p.y), run, time * 14.0, [0.1, 0.1, 0.1]);
                // The farmer, crook in hand, at the field's corner
                let corner = world(farm, field, vec2(field.half.x - 3.0, -(field.half.y - 3.0)));
                let watch = (world(farm, field, centroid) - corner).normalize_or_zero();
                let feet = vec3(corner.x, terrain_height(corner.x, corner.y), corner.y);
                m.figure(feet, watch, 0.0, Pose::Standing, farm.seed + 2.0);
                let side = vec3(watch.y, 0.0, -watch.x);
                m.limb(feet + side * 0.6 + Vec3::Y * 0.05, feet + side * 0.7 + Vec3::Y * 3.4, 0.05, [0.4, 0.3, 0.2]);
            }
        }
        (m.verts, m.idxs)
    }
}

/// A sheep: a woolly body on thin dark legs, head down to graze or up.
fn sheep(m: &mut Mesh, at: Vec3, dir: Vec2, grazing: bool, moving: bool, phase: f32, seed: f32) {
    let k = PERSON_SCALE;
    let f = vec3(dir.x, 0.0, dir.y);
    let r = vec3(dir.y, 0.0, -dir.x);
    let fleece = [0.93, 0.92, 0.86];
    let face = [0.15, 0.13, 0.12];
    let hop = if moving { (phase * 6.0).sin().abs() * 0.05 * k } else { 0.0 };
    for (a, s) in [(0.25, -1.0), (0.25, 1.0), (-0.25, -1.0), (-0.25, 1.0)] {
        let swing = if moving { (phase * 6.0 + a * 10.0 * s).sin() * 0.08 } else { 0.0 };
        let hip = at + (f * a + r * s * 0.12 + Vec3::Y * 0.45) * k;
        m.limb(hip, at + (f * (a + swing) + r * s * 0.12) * k + Vec3::Y * 0.02, 0.04 * k, face);
    }
    let body = at + Vec3::Y * (0.6 * k) + Vec3::Y * hop;
    m.box_at(body, r, f, vec3(0.27, 0.22, 0.42) * k, fleece, fleece);
    m.box_at(body + Vec3::Y * 0.05 * k, r, f, vec3(0.3, 0.18, 0.36) * k, fleece, fleece);
    let head = if grazing { body + f * 0.45 * k - Vec3::Y * 0.3 * k } else { body + f * 0.48 * k + Vec3::Y * 0.12 * k };
    m.box_at(head, r, f, vec3(0.09, 0.1, 0.13) * k, face, face);
    let _ = seed;
}

/// A cow: black and white or brown, head down to the grass or up and about.
fn cow(m: &mut Mesh, at: Vec3, dir: Vec2, grazing: bool, moving: bool, phase: f32, seed: f32) {
    let k = PERSON_SCALE * 1.4;
    let f = vec3(dir.x, 0.0, dir.y);
    let r = vec3(dir.y, 0.0, -dir.x);
    let white = [0.92, 0.9, 0.86];
    let black = [0.1, 0.09, 0.09];
    let brown = [0.55, 0.32, 0.18];
    let coat = if hash(seed, 5.0) < 0.3 { brown } else { white };
    for (a, s) in [(0.55, -1.0), (0.55, 1.0), (-0.55, -1.0), (-0.55, 1.0)] {
        let swing = if moving { (phase * 4.0 + a * 6.0 * s).sin() * 0.1 } else { 0.0 };
        let hip = at + (f * a + r * s * 0.25 + Vec3::Y * 0.65) * k;
        m.limb(hip, at + (f * (a + swing) + r * s * 0.25) * k, 0.07 * k, black);
    }
    let body = at + Vec3::Y * 0.85 * k;
    m.box_at(body, r, f, vec3(0.38, 0.3, 0.8) * k, coat, coat);
    if coat == white {
        for p in 0..2 {
            let o = (hash(seed, p as f32 + 6.0) - 0.5) * 0.9;
            m.box_at(body + f * o * k + Vec3::Y * 0.08 * k, r, f, vec3(0.39, 0.2, 0.22) * k, black, black);
        }
    }
    let head = if grazing { body + f * 0.9 * k - Vec3::Y * 0.45 * k } else { body + f * 0.95 * k + Vec3::Y * 0.15 * k };
    m.box_at(head, r, f, vec3(0.16, 0.18, 0.22) * k, if coat == white { black } else { coat }, black);
    m.box_at(head + f * 0.2 * k - Vec3::Y * 0.06 * k, r, f, vec3(0.13, 0.08, 0.06) * k, [0.85, 0.6, 0.6], [0.85, 0.6, 0.6]);
}
