//! Birds on the water: little parties of ducks paddling along the shore,
//! pairs of swans gliding, and herons standing stock still in the
//! shallows. Ducks paddle off when the car comes near, and take to the air
//! if it comes at them fast; herons lift away on slow, heavy wingbeats.
//! Like the wildlife they come and go in a ring round the player.

use glam::{Vec2, Vec3, vec2, vec3};

use crate::boats::surface_at;
use crate::geometry::Vertex;
use crate::props::PERSON_SCALE;
use crate::terrain::{self, WATER_LEVEL};
use crate::townsfolk::Mesh;

const DUCK_PARTIES: usize = 4;
const SWAN_PAIRS: usize = 2;
const HERONS: usize = 2;
/// Spawn between these distances; forget beyond the last.
const SPAWN: (f32, f32) = (90.0, 340.0);
const FORGET: f32 = 480.0;
const DRAW: f32 = 400.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A mallard, drake or duck.
    Duck { drake: bool },
    Swan,
    Heron,
}

#[derive(Clone, Copy, PartialEq)]
enum Doing {
    /// On the water (or standing), drifting about.
    Idle,
    /// Paddling away from the car, for so long more.
    Paddling(f32),
    /// In the air, away from the car, for so long more (then down again,
    /// or for a heron, gone).
    Flying(f32),
}

struct Bird {
    kind: Kind,
    pos: Vec2,
    heading: f32,
    /// Height above the water, while flying.
    lift: f32,
    doing: Doing,
    group: u32,
    phase: f32,
    seed: f32,
}

pub struct Waterfowl {
    birds: Vec<Bird>,
    rng: u32,
    next_group: u32,
    /// Spawn ring (dev aid STUMBLER_BIRDS_CLOSE=1 brings it in for screenshots).
    ring: (f32, f32),
}

impl Default for Waterfowl {
    fn default() -> Self {
        let close = std::env::var_os("STUMBLER_BIRDS_CLOSE").is_some();
        Self { birds: Vec::new(), rng: 0xd0c5, next_group: 1, ring: if close { (20.0, 70.0) } else { SPAWN } }
    }
}

/// How deep the water is at `p` (negative on land).
fn depth(p: Vec2) -> f32 {
    WATER_LEVEL - terrain::raw_height(p.x, p.y)
}

/// Water near a shore: land within `reach` some way round.
fn by_the_shore(p: Vec2, reach: f32) -> bool {
    (0..8).any(|k| depth(p + Vec2::from_angle(k as f32 * std::f32::consts::FRAC_PI_4) * reach) < 0.0)
}

impl Waterfowl {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223) | 1;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn update(&mut self, player: Vec3, player_vel: Vec3, dt: f32) {
        let me = vec2(player.x, player.z);
        let speed = vec2(player_vel.x, player_vel.z).length();
        self.birds.retain(|b| b.pos.distance(me) < FORGET && !(b.kind == Kind::Heron && matches!(b.doing, Doing::Flying(t) if t <= 0.0)));
        self.spawn(me);
        // A party takes fright together
        let mut frightened: Vec<(u32, bool)> = Vec::new();
        for b in &self.birds {
            let d = b.pos.distance(me);
            let (wary, bolt) = match b.kind {
                Kind::Duck { .. } => (35.0, 14.0),
                Kind::Swan => (30.0, 0.0),
                Kind::Heron => (55.0, 55.0),
            };
            if matches!(b.doing, Doing::Idle) && d < wary {
                frightened.push((b.group, d < bolt && (speed > 6.0 || b.kind == Kind::Heron)));
            }
        }
        for i in 0..self.birds.len() {
            let r = self.rand();
            let b = &mut self.birds[i];
            if let Some(&(_, fly)) = frightened.iter().find(|(g, _)| *g == b.group) {
                b.doing = if fly { Doing::Flying(if b.kind == Kind::Heron { 9.0 } else { 3.0 + r }) } else { Doing::Paddling(4.0 + 2.0 * r) };
                let away = (b.pos - me).normalize_or_zero();
                b.heading = away.x.atan2(away.y) + (r - 0.5) * 0.6;
            }
            b.update(dt, r);
        }
    }

    fn spawn(&mut self, me: Vec2) {
        let count = |w: &Self, f: fn(Kind) -> bool| {
            let mut g: Vec<u32> = w.birds.iter().filter(|b| f(b.kind)).map(|b| b.group).collect();
            g.sort();
            g.dedup();
            g.len()
        };
        for _ in 0..2 {
            let a = self.rand() * std::f32::consts::TAU;
            let d = self.ring.0 + (self.ring.1 - self.ring.0) * self.rand();
            let p = me + Vec2::from_angle(a) * d;
            let dp = depth(p);
            let group = self.next_group;
            let seed = self.rand() * 1000.0;
            if (-0.2..1.0).contains(&dp) && by_the_shore(p, 15.0) && count(self, |k| k == Kind::Heron) < HERONS {
                self.next_group += 1;
                let heading = self.rand() * std::f32::consts::TAU;
                self.birds.push(Bird { kind: Kind::Heron, pos: p, heading, lift: 0.0, doing: Doing::Idle, group, phase: 0.0, seed });
            } else if (0.6..8.0).contains(&dp) && by_the_shore(p, 45.0) {
                if count(self, |k| matches!(k, Kind::Duck { .. })) < DUCK_PARTIES {
                    self.next_group += 1;
                    let n = 3 + (self.rand() * 5.0) as usize;
                    for k in 0..n {
                        let off = vec2(self.rand() - 0.5, self.rand() - 0.5) * 8.0;
                        let heading = self.rand() * std::f32::consts::TAU;
                        let phase = self.rand() * 10.0;
                        self.birds.push(Bird { kind: Kind::Duck { drake: k % 2 == 0 }, pos: p + off, heading, lift: 0.0, doing: Doing::Idle, group, phase, seed: seed + k as f32 });
                    }
                } else if count(self, |k| k == Kind::Swan) < SWAN_PAIRS && dp > 1.5 {
                    self.next_group += 1;
                    let heading = self.rand() * std::f32::consts::TAU;
                    for k in 0..2 {
                        let off = Vec2::from_angle(heading) * k as f32 * 4.0;
                        self.birds.push(Bird { kind: Kind::Swan, pos: p + off, heading, lift: 0.0, doing: Doing::Idle, group, phase: 0.0, seed: seed + k as f32 });
                    }
                }
            }
        }
    }

    /// How many ducks, swans and herons are about.
    #[cfg(test)]
    pub fn census(&self) -> (usize, usize, usize) {
        let n = |f: fn(Kind) -> bool| self.birds.iter().filter(|b| f(b.kind)).count();
        (n(|k| matches!(k, Kind::Duck { .. })), n(|k| k == Kind::Swan), n(|k| k == Kind::Heron))
    }

    /// Every bird near enough to see.
    pub fn build_mesh(&self, camera: Vec3, time: f32) -> (Vec<Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let mut m = Mesh::default();
        for b in self.birds.iter().filter(|b| b.pos.distance(cam) < DRAW) {
            let dir = vec2(b.heading.sin(), b.heading.cos());
            let flying = matches!(b.doing, Doing::Flying(_));
            let base = match b.kind {
                // Standing in the shallows on the bed, or wading
                Kind::Heron if !flying => terrain::terrain_height(b.pos.x, b.pos.y).max(WATER_LEVEL - 0.8),
                _ => surface_at(b.pos, time) - 0.05,
            };
            let at = vec3(b.pos.x, base + b.lift, b.pos.y);
            match b.kind {
                Kind::Duck { drake } => duck(&mut m, at, dir, drake, flying, b.phase),
                Kind::Swan => swan(&mut m, at, dir, b.phase),
                Kind::Heron => heron(&mut m, at, dir, flying, b.phase, b.seed),
            }
        }
        (m.verts, m.idxs)
    }
}

impl Bird {
    fn update(&mut self, dt: f32, r: f32) {
        let (idle, paddle, fly) = match self.kind {
            Kind::Duck { .. } => (0.6, 3.5, 14.0),
            Kind::Swan => (0.8, 2.5, 12.0),
            Kind::Heron => (0.0, 0.0, 9.0),
        };
        let speed = match self.doing {
            Doing::Idle => {
                // Drifting about, turning now and then
                self.heading += (r - 0.5) * dt * 1.5;
                idle
            }
            Doing::Paddling(t) => {
                self.doing = if t <= 0.0 { Doing::Idle } else { Doing::Paddling(t - dt) };
                paddle
            }
            Doing::Flying(t) => {
                // Up, along, and (ducks) back down onto the water
                let climb = if self.kind == Kind::Heron { 3.0 } else if t > 1.0 { 5.0 } else { -6.0 };
                self.lift = (self.lift + climb * dt).clamp(0.0, if self.kind == Kind::Heron { 40.0 } else { 7.0 });
                self.doing = if t <= 0.0 && self.kind != Kind::Heron { self.lift = 0.0; Doing::Idle } else { Doing::Flying(t - dt) };
                fly
            }
        };
        let fwd = vec2(self.heading.sin(), self.heading.cos());
        let next = self.pos + fwd * speed * dt;
        let flying = matches!(self.doing, Doing::Flying(_));
        // Keep to the water (herons only ever wade or fly)
        if flying || depth(next) > 0.4 || (self.kind == Kind::Heron && depth(next) > -0.3) {
            self.pos = next;
        } else {
            self.heading += std::f32::consts::PI * 0.6;
        }
        self.phase += dt * if flying { if self.kind == Kind::Heron { 3.0 } else { 14.0 } } else { 1.0 + speed };
    }
}

/// Part of a bird's frame: `p(along, up, across)` in metres from `at`.
fn frame(at: Vec3, dir: Vec2) -> (Vec3, Vec3, impl Fn(f32, f32, f32) -> Vec3) {
    let f = vec3(dir.x, 0.0, dir.y);
    let r = vec3(dir.y, 0.0, -dir.x);
    (f, r, move |a: f32, u: f32, c: f32| at + (f * a + Vec3::Y * u + r * c) * PERSON_SCALE)
}

fn wings(m: &mut Mesh, root: [Vec3; 2], r: Vec3, span: f32, flap: f32, col: [f32; 3]) {
    for s in [-1.0, 1.0] {
        let tip = (root[0] + root[1]) * 0.5 + (r * s * span + Vec3::Y * flap * span) * PERSON_SCALE;
        m.tri(root[0], root[1], tip, col);
    }
}

/// A mallard: the drake with his green head and white collar, the duck brown.
fn duck(m: &mut Mesh, at: Vec3, dir: Vec2, drake: bool, flying: bool, phase: f32) {
    let (f, r, p) = frame(at, dir);
    let k = PERSON_SCALE;
    let body = if drake { [0.55, 0.5, 0.45] } else { [0.5, 0.36, 0.22] };
    let head = if drake { [0.1, 0.4, 0.2] } else { [0.45, 0.32, 0.2] };
    m.box_at(p(0.0, 0.08, 0.0), r, f, vec3(0.13, 0.08, 0.22) * k, body, body);
    m.box_at(p(-0.2, 0.12, 0.0), r, f, vec3(0.05, 0.05, 0.06) * k, [0.15, 0.12, 0.1], [0.15, 0.12, 0.1]);
    if drake {
        m.box_at(p(0.18, 0.2, 0.0), r, f, vec3(0.06, 0.02, 0.05) * k, [0.95, 0.95, 0.92], [0.95, 0.95, 0.92]);
    }
    m.box_at(p(0.2, 0.27, 0.0), r, f, vec3(0.06, 0.06, 0.07) * k, head, head);
    m.box_at(p(0.3, 0.25, 0.0), r, f, vec3(0.03, 0.015, 0.05) * k, [0.95, 0.75, 0.15], [0.95, 0.75, 0.15]);
    if flying {
        wings(m, [p(0.08, 0.14, 0.0), p(-0.1, 0.14, 0.0)], r, 0.38, phase.sin() * 0.8, body);
    }
}

/// A mute swan, neck up in its curve, orange bill.
fn swan(m: &mut Mesh, at: Vec3, dir: Vec2, phase: f32) {
    let (f, r, p) = frame(at, dir);
    let k = PERSON_SCALE;
    let white = [0.96, 0.96, 0.94];
    m.box_at(p(0.0, 0.18, 0.0), r, f, vec3(0.3, 0.18, 0.6) * k, white, white);
    // Wings raised a little over the back
    m.box_at(p(-0.15, 0.36, 0.0), r, f, vec3(0.26, 0.08, 0.4) * k, white, white);
    // The neck in an S, bobbing a touch
    let bob = (phase * 0.8).sin() * 0.03;
    let neck = [p(0.45, 0.25, 0.0), p(0.55, 0.55, 0.0), p(0.5, 0.85 + bob, 0.0), p(0.6, 1.0 + bob, 0.0)];
    for w in neck.windows(2) {
        m.limb(w[0], w[1], 0.06 * k, white);
    }
    m.box_at(p(0.62, 1.02 + bob, 0.0), r, f, vec3(0.05, 0.05, 0.08) * k, white, white);
    m.box_at(p(0.74, 1.0 + bob, 0.0), r, f, vec3(0.025, 0.025, 0.07) * k, [0.95, 0.5, 0.1], [0.95, 0.5, 0.1]);
}

/// A grey heron standing hunched in the shallows, or flying off with its
/// neck tucked in and legs trailing.
fn heron(m: &mut Mesh, at: Vec3, dir: Vec2, flying: bool, phase: f32, seed: f32) {
    let (f, r, p) = frame(at, dir);
    let k = PERSON_SCALE;
    let grey = [0.55, 0.57, 0.6];
    let pale = [0.85, 0.85, 0.82];
    let legs = [0.6, 0.55, 0.35];
    if flying {
        m.box_at(p(0.0, 0.0, 0.0), r, f, vec3(0.12, 0.1, 0.32) * k, grey, grey);
        m.box_at(p(0.33, 0.05, 0.0), r, f, vec3(0.07, 0.07, 0.08) * k, pale, pale);
        m.limb(p(0.4, 0.05, 0.0), p(0.62, 0.04, 0.0), 0.02 * k, [0.85, 0.7, 0.2]);
        m.limb(p(-0.25, -0.02, 0.0), p(-0.7, -0.05, 0.0), 0.02 * k, legs);
        // Big, slow, bowed wings
        let flap = phase.sin() * 0.5;
        wings(m, [p(0.18, 0.05, 0.0), p(-0.2, 0.05, 0.0)], r, 0.9, flap, grey);
    } else {
        // Now and then it shifts, neck up to look, or down to watch the water
        let alert = ((phase * 0.3 + seed).sin() > 0.6) as u8 as f32;
        for s in [-1.0, 1.0] {
            m.limb(p(0.0, 0.0, s * 0.06), p(0.0, 0.55, s * 0.05), 0.02 * k, legs);
        }
        m.box_at(p(0.0, 0.7, 0.0), r, f, vec3(0.12, 0.17, 0.25) * k, grey, pale);
        let neck = [p(0.15, 0.8, 0.0), p(0.22, 0.95 + 0.2 * alert, 0.0), p(0.2, 1.1 + 0.25 * alert, 0.0)];
        for w in neck.windows(2) {
            m.limb(w[0], w[1], 0.035 * k, pale);
        }
        let head = neck[2];
        m.box_at(head, r, f, vec3(0.04, 0.045, 0.07) * k, pale, [0.15, 0.15, 0.17]);
        m.limb(head + f * 0.05 * k, head + f * 0.25 * k - Vec3::Y * 0.03 * k, 0.018 * k, [0.85, 0.7, 0.2]);
    }
}
