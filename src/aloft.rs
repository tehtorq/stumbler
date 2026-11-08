//! Up in the air: hot-air balloons drifting over on calm, dry mornings
//! and evenings, the burner roaring now and then to keep them up; and
//! paragliders riding the thermals over the mountain summits through the
//! middle of the day, turning slow circles and sinking and climbing.

use glam::{Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::landmarks::{self, Kind};
use crate::particles::Particles;
use crate::props::{hash, PERSON_SCALE};
use crate::terrain::terrain_height;
use crate::townsfolk::{Mesh, Pose};

/// How many balloons are about at most, and where they appear and fade.
const BALLOONS: usize = 4;
const SPAWN: (f32, f32) = (900.0, 1800.0);
const FORGET: f32 = 2400.0;
const DRAW: f32 = 2600.0;
/// The wind they drift on (as the boats' and the smoke's).
const WIND: Vec2 = vec2(0.8, 0.6);
/// Paragliders: how near a summit has to be to have them, and how many.
const SUMMIT_NEAR: f32 = 1800.0;
const GLIDERS: usize = 3;

struct Balloon {
    pos: Vec2,
    /// Height above the ground under it, eased toward `cruise`.
    height: f32,
    cruise: f32,
    /// Until the burner next fires, and how long it stays lit.
    next_burn: f32,
    burning: f32,
    seed: f32,
}

#[derive(Default)]
pub struct Aloft {
    balloons: Vec<Balloon>,
    /// How hard it's raining: nobody flies in the wet.
    rain: f32,
    /// Burners lit since last asked (for their roar).
    flares: Vec<Vec3>,
    rng: u32,
}

/// Whether it's balloon weather at `clock` (0..1 from midnight): the calm
/// of early morning or evening.
fn balloon_hours(clock: f32) -> bool {
    let h = clock * 24.0;
    (5.5..10.0).contains(&h) || (16.5..20.5).contains(&h)
}

impl Aloft {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223) | 1;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `rain` (0..1): balloons stay down in the wet.
    pub fn update(&mut self, particles: &mut Particles, player: Vec3, clock: f32, rain: f32, dt: f32) {
        self.rain = rain;
        let me = vec2(player.x, player.z);
        self.balloons.retain(|b| b.pos.distance(me) < FORGET);
        let flying = balloon_hours(clock) && rain < 0.2;
        if flying && self.balloons.len() < BALLOONS && self.rand() < dt * 0.5 {
            // Upwind, so it drifts across
            let a = self.rand() * 2.0 - 1.0;
            let from = (-WIND.normalize()).rotate(Vec2::from_angle(a * 1.1));
            let d = SPAWN.0 + (SPAWN.1 - SPAWN.0) * self.rand();
            let pos = me + from * d;
            if self.balloons.iter().all(|b| b.pos.distance(pos) > 250.0) {
                let cruise = 90.0 + 220.0 * self.rand();
                let seed = self.rand() * 1000.0;
                let next_burn = 2.0 + self.rand() * 6.0;
                self.balloons.push(Balloon { pos, height: cruise, cruise, next_burn, burning: 0.0, seed });
            }
        }
        if !flying {
            // Coming down out of sight once the calm's over
            self.balloons.retain(|b| b.pos.distance(me) < SPAWN.0);
        }
        for b in &mut self.balloons {
            let speed = 3.0 + 2.0 * hash(b.seed, 1.0);
            b.pos += WIND.normalize() * speed * dt;
            b.next_burn -= dt;
            if b.next_burn <= 0.0 {
                b.burning = 1.5 + 2.5 * hash(b.seed, b.pos.x.floor());
                let ground = terrain_height(b.pos.x, b.pos.y);
                self.flares.push(vec3(b.pos.x, ground + b.height + 3.2 * PERSON_SCALE, b.pos.y));
                b.next_burn = b.burning + 6.0 + 10.0 * hash(b.seed, b.pos.y.floor());
            }
            // The burner lifts it; between burns it sinks gently
            let climb = if b.burning > 0.0 { 1.4 } else { -0.5 };
            b.height = (b.height + climb * dt).clamp(b.cruise - 40.0, b.cruise + 40.0);
            if b.burning > 0.0 {
                b.burning -= dt;
                if b.pos.distance(me) < 900.0 {
                    let ground = terrain_height(b.pos.x, b.pos.y);
                    let k = PERSON_SCALE;
                    for _ in 0..2 {
                        particles.burner(vec3(b.pos.x, ground + b.height + 3.2 * k, b.pos.y));
                    }
                }
            }
        }
    }

    /// Where burners have been lit since last asked.
    pub fn take_flares(&mut self) -> Vec<Vec3> {
        std::mem::take(&mut self.flares)
    }

    /// The balloons and paragliders near enough to see.
    pub fn build_mesh(&self, camera: Vec3, player: Vec3, clock: f32, time: f32) -> (Vec<Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let mut m = Mesh::default();
        for b in self.balloons.iter().filter(|b| b.pos.distance(cam) < DRAW) {
            let ground = terrain_height(b.pos.x, b.pos.y);
            balloon(&mut m, vec3(b.pos.x, ground + b.height, b.pos.y), b.seed, time, b.burning > 0.0);
        }
        // Paragliders over the summits near the player, through the middle of the day
        let h = clock * 24.0;
        if (10.0..17.5).contains(&h) && self.rain < 0.2 {
            let me = vec2(player.x, player.z);
            for l in landmarks::all().iter().filter(|l| l.kind == Kind::Summit && l.pos.distance(me) < SUMMIT_NEAR) {
                let top = terrain_height(l.pos.x, l.pos.y);
                for g in 0..GLIDERS {
                    let seed = l.seed as f32 % 1000.0 + g as f32 * 37.0;
                    // Circling a thermal, the circle itself wandering round the summit
                    let wander = time * 0.02 + hash(seed, 1.0) * 6.28;
                    let centre = l.pos + Vec2::from_angle(wander) * (110.0 + 150.0 * hash(seed, 2.0));
                    let r = 30.0 + 25.0 * hash(seed, 3.0);
                    let turn = if hash(seed, 4.0) < 0.5 { 1.0 } else { -1.0 };
                    let a = time * 0.16 * turn * (1.0 + 0.3 * hash(seed, 5.0)) + hash(seed, 6.0) * 6.28;
                    let at = centre + Vec2::from_angle(a) * r;
                    let lift = 28.0 + 40.0 * hash(seed, 7.0) + 14.0 * (time * 0.03 + seed).sin();
                    let pos = vec3(at.x, top.max(terrain_height(at.x, at.y)) + lift, at.y);
                    if pos.distance(camera) > DRAW {
                        continue;
                    }
                    let along = Vec2::from_angle(a + turn * std::f32::consts::FRAC_PI_2);
                    paraglider(&mut m, pos, along, turn * 0.35, seed);
                }
            }
        }
        (m.verts, m.idxs)
    }
}

/// A hot-air balloon with its basket hanging at `basket`.
fn balloon(m: &mut Mesh, basket: Vec3, seed: f32, time: f32, burning: bool) {
    let k = PERSON_SCALE;
    const COLOURS: [[f32; 3]; 7] = [[0.9, 0.15, 0.12], [0.98, 0.8, 0.15], [0.15, 0.4, 0.8], [0.95, 0.95, 0.92], [0.2, 0.6, 0.3], [0.95, 0.5, 0.1], [0.6, 0.2, 0.6]];
    let pick = |n: f32| COLOURS[(hash(seed, n) * 7.0) as usize % 7];
    let (a, b, c) = (pick(10.0), pick(11.0), pick(12.0));
    // The envelope: a lathe of rings, widest a third of the way down
    let profile: [(f32, f32); 9] = [(4.0, 1.2), (5.5, 3.5), (7.5, 6.0), (10.5, 7.6), (13.5, 8.2), (16.5, 7.8), (19.0, 6.6), (20.8, 4.4), (21.6, 0.0)];
    let sway = Vec3::new((time * 0.3 + seed).sin(), 0.0, (time * 0.23 + seed).cos()) * 0.04;
    let up = (Vec3::Y + sway).normalize();
    let side = up.cross(Vec3::Z).normalize();
    let fwd = side.cross(up);
    let gores = 14;
    let ring = |level: usize, g: usize| {
        let (h, r) = profile[level];
        let ang = g as f32 / gores as f32 * std::f32::consts::TAU;
        basket + (up * h + (side * ang.cos() + fwd * ang.sin()) * r) * k
    };
    for level in 0..profile.len() - 1 {
        for g in 0..gores {
            // Stripes down the gores, a band round the middle
            let col = if (3..5).contains(&level) { c } else if g % 2 == 0 { a } else { b };
            m.quad(ring(level, g), ring(level, g + 1), ring(level + 1, g + 1), ring(level + 1, g), col);
        }
    }
    // Ropes down to the basket, the burner frame, the basket, and heads
    let rope = [0.3, 0.25, 0.2];
    for g in (0..gores).step_by(gores / 4) {
        let corner = basket + (side * if g < gores / 2 { 0.6 } else { -0.6 } + fwd * if g % 7 < 4 { 0.6 } else { -0.6 }) * k + up * 1.1 * k;
        m.limb(ring(0, g), corner, 0.03 * k, rope);
    }
    m.box_at(basket + up * 0.55 * k, side, fwd, vec3(0.65, 0.55, 0.65) * k, [0.6, 0.45, 0.25], [0.45, 0.33, 0.18]);
    m.box_at(basket + up * 3.0 * k, side, fwd, vec3(0.3, 0.15, 0.3) * k, [0.5, 0.5, 0.52], [0.4, 0.4, 0.42]);
    if burning {
        m.box_at(basket + up * 3.5 * k, side, fwd, vec3(0.15, 0.4, 0.15) * k, [1.0, 0.75, 0.2], [1.0, 0.9, 0.5]);
    }
    for (s, f) in [(-0.3, 0.2), (0.3, -0.25)] {
        m.box_at(basket + (up * 1.35 + side * s + fwd * f) * k, side, fwd, vec3(0.14, 0.15, 0.14) * k, [0.9, 0.72, 0.6], [0.3, 0.22, 0.15]);
    }
}

/// A paraglider: the curved wing above, lines down to the pilot in the
/// harness, banked `bank` into the turn.
fn paraglider(m: &mut Mesh, pilot: Vec3, along: Vec2, bank: f32, seed: f32) {
    let k = PERSON_SCALE;
    const WINGS: [([f32; 3], [f32; 3]); 4] = [([0.95, 0.3, 0.1], [0.98, 0.85, 0.2]), ([0.15, 0.45, 0.85], [0.95, 0.95, 0.95]), ([0.85, 0.1, 0.4], [0.3, 0.15, 0.5]), ([0.2, 0.7, 0.3], [0.95, 0.9, 0.3])];
    let (wa, wb) = WINGS[(hash(seed, 9.0) * 4.0) as usize % 4];
    let f = vec3(along.x, 0.0, along.y);
    let flat_side = vec3(along.y, 0.0, -along.x);
    let up = (Vec3::Y * bank.cos() + flat_side * bank.sin()).normalize();
    let side = f.cross(up).normalize();
    // The wing: an arc of cells over the pilot, 10 m span, 7 m up
    let cells = 10;
    let arc = |i: usize, chord: f32| {
        let t = i as f32 / cells as f32 * 2.0 - 1.0;
        let a = t * 1.05;
        pilot + (up * (7.0 * a.cos()) + side * (5.5 * a.sin()) + f * chord) * k
    };
    for i in 0..cells {
        let col = if i % 2 == 0 { wa } else { wb };
        m.quad(arc(i, 1.2), arc(i + 1, 1.2), arc(i + 1, -1.0), arc(i, -1.0), col);
    }
    // Lines fanning down to the risers
    let line = [0.85, 0.85, 0.85];
    for i in (0..=cells).step_by(2) {
        m.limb(arc(i, 0.3), pilot + up * 1.4 * k, 0.012 * k, line);
    }
    m.figure(pilot - up * 0.2 * k, along, 0.0, Pose::Sitting, seed + 3.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balloons_go_up_on_calm_dry_mornings() {
        let count = |clock: f32, rain: f32| {
            let mut aloft = Aloft::default();
            let mut particles = Particles::new();
            for _ in 0..1200 {
                aloft.update(&mut particles, Vec3::ZERO, clock, rain, 0.05);
            }
            aloft.balloons.len()
        };
        assert!(count(7.0 / 24.0, 0.0) > 0, "no balloons on a dry morning");
        assert_eq!(count(13.0 / 24.0, 0.0), 0, "balloons in the midday heat");
        assert_eq!(count(7.0 / 24.0, 0.8), 0, "balloons in the rain");
    }
}
