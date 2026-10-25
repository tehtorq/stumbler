//! Ambient wildlife around the player: deer herds grazing in meadows and at
//! forest edges, rabbits hopping about, and butterflies over the flowers.
//! Animals appear in a ring around the player where the land suits them,
//! bolt when the car comes close, and are forgotten once left behind.

use glam::{Quat, Vec2, Vec3, vec3};

use crate::geometry::Vertex;
use crate::noise::noise;
use crate::road;
use crate::terrain::{self, terrain_height, WATER_LEVEL};

/// How many of each are kept around the player.
const DEER_HERDS: usize = 4;
const RABBITS: usize = 10;
const BUTTERFLIES: usize = 14;
/// Spawn between these distances from the player; forget beyond the last.
const SPAWN_NEAR: f32 = 140.0;
const SPAWN_FAR: f32 = 420.0;
const FORGET: f32 = 650.0;

#[derive(Clone, Copy, PartialEq)]
enum Species {
    /// `age` 1 for adults, less for fawns.
    Deer { stag: bool, age: f32 },
    Rabbit,
    Butterfly { colour: [f32; 3] },
}

#[derive(Clone, Copy, PartialEq)]
enum Mood {
    /// Head down eating (or sitting, for rabbits) for a while.
    Grazing(f32),
    /// Ambling toward a spot.
    Walking(Vec2),
    /// Running from the car until this much time has passed.
    Fleeing(f32),
}

struct Animal {
    species: Species,
    pos: Vec2,
    /// Height above the ground (butterflies, and deer/rabbits mid-leap).
    lift: f32,
    heading: f32,
    mood: Mood,
    /// Animation clock (legs, wings, hops).
    phase: f32,
    herd: u32,
    /// How far the head is lowered toward the grass (0..1).
    head_down: f32,
    /// Ground height, refreshed only once the animal has moved a little.
    ground: f32,
    ground_at: Vec2,
}

pub struct Wildlife {
    animals: Vec<Animal>,
    rng: u32,
    next_herd: u32,
    /// Spawn ring (dev aid STUMBLER_WILDLIFE_CLOSE=1 brings it in for screenshots).
    ring: (f32, f32),
}

impl Wildlife {
    pub fn new(seed: u32) -> Self {
        let close = std::env::var_os("STUMBLER_WILDLIFE_CLOSE").is_some();
        Self { animals: Vec::new(), rng: seed | 1, next_herd: 1, ring: if close { (35.0, 70.0) } else { (SPAWN_NEAR, SPAWN_FAR) } }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng & 0xffffff) as f32 / 0x1000000 as f32
    }

    pub fn update(&mut self, player: Vec3, player_vel: Vec3, dt: f32) {
        let me = Vec2::new(player.x, player.z);
        let speed = Vec2::new(player_vel.x, player_vel.z).length();
        self.animals.retain(|a| a.pos.distance(me) < FORGET);
        self.spawn(me);

        // A herd bolts together when any of it takes fright
        let mut spooked: Vec<u32> = Vec::new();
        for a in &self.animals {
            let d = a.pos.distance(me);
            let wary = match a.species {
                Species::Deer { .. } => if speed > 3.0 { 75.0 } else { 30.0 },
                Species::Rabbit => if speed > 3.0 { 40.0 } else { 15.0 },
                Species::Butterfly { .. } => 12.0,
            };
            if d < wary && !matches!(a.mood, Mood::Fleeing(_)) {
                spooked.push(a.herd);
            }
        }

        for i in 0..self.animals.len() {
            let r = [self.rand(), self.rand(), self.rand()];
            let a = &mut self.animals[i];
            if spooked.contains(&a.herd) && !matches!(a.mood, Mood::Fleeing(_)) {
                a.mood = Mood::Fleeing(match a.species { Species::Deer { .. } => 4.0, Species::Rabbit => 2.0, Species::Butterfly { .. } => 1.5 });
            }
            a.update(me, dt, r);
        }
    }

    /// Top up each kind of animal, at random spots in the ring around the player.
    fn spawn(&mut self, me: Vec2) {
        let count = |w: &Self, f: fn(&Species) -> bool| w.animals.iter().filter(|a| f(&a.species)).count();
        // A few attempts per frame keeps spawning cheap
        for _ in 0..3 {
            let a = self.rand() * std::f32::consts::TAU;
            let d = self.ring.0 + (self.ring.1 - self.ring.0) * self.rand();
            let p = me + Vec2::new(a.cos(), a.sin()) * d;
            let Some(habitat) = habitat(p) else { continue };
            let herds = self.animals.iter().filter(|a| matches!(a.species, Species::Deer { .. })).map(|a| a.herd).collect::<std::collections::HashSet<_>>().len();
            if habitat.deer && herds < DEER_HERDS {
                let herd = self.next_herd;
                self.next_herd += 1;
                let n = 2 + (self.rand() * 4.0) as usize;
                for k in 0..n {
                    let off = Vec2::new(self.rand() - 0.5, self.rand() - 0.5) * 25.0;
                    let stag = k == 0 && self.rand() < 0.5;
                    let age = if k == n - 1 && n > 2 && self.rand() < 0.6 { 0.6 } else { 1.0 };
                    let heading = self.rand() * std::f32::consts::TAU;
                    let phase = self.rand() * 10.0;
                    let graze = 2.0 + self.rand() * 6.0;
                    self.animals.push(Animal::new(Species::Deer { stag, age }, p + off, heading, phase, herd, graze));
                }
            } else if habitat.meadow && count(self, |s| *s == Species::Rabbit) < RABBITS {
                let herd = self.next_herd;
                self.next_herd += 1;
                let (h, ph, g) = (self.rand() * 6.3, self.rand() * 10.0, 1.0 + self.rand() * 3.0);
                self.animals.push(Animal::new(Species::Rabbit, p, h, ph, herd, g));
            }
            // Butterflies stay close, over flowers
            let near = me + Vec2::new(a.cos(), a.sin()) * (20.0 + 60.0 * self.rand());
            if count(self, |s| matches!(s, Species::Butterfly { .. })) < BUTTERFLIES && habitat_of(near).is_some_and(|h| h.flowers) {
                const WINGS: [[f32; 3]; 4] = [[0.98, 0.75, 0.15], [0.95, 0.95, 0.92], [0.4, 0.6, 0.98], [0.95, 0.45, 0.15]];
                let colour = WINGS[(self.rand() * 4.0) as usize % 4];
                let herd = self.next_herd;
                self.next_herd += 1;
                let (h, ph) = (self.rand() * 6.3, self.rand() * 10.0);
                let mut b = Animal::new(Species::Butterfly { colour }, near, h, ph, herd, 0.0);
                b.lift = 1.5 + self.rand() * 2.0;
                self.animals.push(b);
            }
        }
    }

    /// Mesh of every animal near enough to see.
    pub fn build_mesh(&self, camera: Vec3) -> (Vec<Vertex>, Vec<u16>) {
        let mut m = Builder::default();
        for a in &self.animals {
            if a.pos.distance(Vec2::new(camera.x, camera.z)) > 450.0 {
                continue;
            }
            let ground = a.ground;
            let at = vec3(a.pos.x, ground + a.lift, a.pos.y);
            let shadow_at = vec3(at.x, ground + 0.08, at.z);
            match a.species {
                Species::Deer { stag, age } => {
                    m.shadow(shadow_at, a.heading, 0.8 * age, 2.0 * age, a.lift);
                    m.deer(at, a.heading, a.phase, a.head_down, matches!(a.mood, Mood::Fleeing(_) | Mood::Walking(_)), stag, age, a.herd);
                }
                Species::Rabbit => {
                    m.shadow(shadow_at, a.heading, 0.45, 0.7, a.lift);
                    m.rabbit(at, a.heading, a.phase, a.head_down);
                }
                Species::Butterfly { colour } => m.butterfly(at, a.heading, a.phase, colour),
            }
        }
        (m.verts, m.idxs)
    }
}

impl Animal {
    fn new(species: Species, pos: Vec2, heading: f32, phase: f32, herd: u32, graze: f32) -> Self {
        Self { species, pos, lift: 0.0, heading, mood: Mood::Grazing(graze), phase, herd, head_down: 1.0, ground: terrain_height(pos.x, pos.y), ground_at: pos }
    }

    fn update(&mut self, me: Vec2, dt: f32, r: [f32; 3]) {
        let (walk_speed, run_speed) = match self.species {
            Species::Deer { .. } => (5.0, 32.0),
            Species::Rabbit => (6.0, 22.0),
            Species::Butterfly { .. } => (4.0, 9.0),
        };
        let mut speed = 0.0;
        match self.mood {
            Mood::Grazing(t) => {
                // Now and then look up from the grass
                let looking = (self.phase * 0.7).sin() > 0.75;
                self.head_down += ((if looking { 0.0 } else { 1.0 }) - self.head_down) * (dt * 3.0).min(1.0);
                self.mood = if t <= 0.0 {
                    let a = self.heading + (r[0] - 0.5) * 2.5;
                    Mood::Walking(self.pos + Vec2::new(a.sin(), a.cos()) * (8.0 + 20.0 * r[1]))
                } else {
                    Mood::Grazing(t - dt)
                };
                self.phase += dt;
            }
            Mood::Walking(target) => {
                self.head_down += (0.0 - self.head_down) * (dt * 3.0).min(1.0);
                let to = target - self.pos;
                if to.length() < 1.0 || habitat_of(target).is_none() {
                    self.mood = Mood::Grazing(3.0 + 8.0 * r[2]);
                } else {
                    self.turn_toward(to, dt * 2.0);
                    speed = walk_speed;
                }
            }
            Mood::Fleeing(t) => {
                self.head_down = 0.0;
                let away = (self.pos - me).normalize_or_zero();
                // Rabbits zig-zag
                let jink = if self.species == Species::Rabbit { (self.phase * 1.3).sin() * 0.8 } else { 0.0 };
                let dir = Vec2::from_angle(jink).rotate(away);
                self.turn_toward(dir, dt * 6.0);
                speed = run_speed;
                self.mood = if t <= 0.0 { Mood::Grazing(2.0 + 4.0 * r[0]) } else { Mood::Fleeing(t - dt) };
            }
        }
        let forward = Vec2::new(self.heading.sin(), self.heading.cos());
        let next = self.pos + forward * speed * dt;
        if self.pos.distance(self.ground_at) > 1.5 {
            self.ground = terrain_height(self.pos.x, self.pos.y);
            self.ground_at = self.pos;
        }
        // Don't run into the water or onto the road
        if habitat_of(next).is_some() || matches!(self.mood, Mood::Fleeing(_)) && terrain::raw_height(next.x, next.y) > WATER_LEVEL + 0.5 {
            self.pos = next;
        } else {
            self.heading += std::f32::consts::FRAC_PI_2 * dt * 4.0;
        }
        // Gait: legs and hops speed up with pace
        self.phase += dt * speed * 0.35;
        self.lift = match self.species {
            Species::Deer { .. } if speed > 15.0 => (self.phase * 1.0).sin().abs() * 1.6,
            Species::Rabbit if speed > 0.0 => (self.phase * 1.6).sin().abs() * 0.8,
            Species::Butterfly { .. } => 1.8 + (self.phase * 0.9).sin() * 0.8 + (self.phase * 2.3).sin() * 0.3,
            _ => 0.0,
        };
        if let Species::Butterfly { .. } = self.species {
            // Erratic flight: keep changing course
            self.heading += ((self.phase * 1.7).sin() + (self.phase * 0.6).cos()) * dt * 2.5;
            if !matches!(self.mood, Mood::Walking(_)) {
                self.mood = Mood::Walking(self.pos + forward * 10.0);
            }
            self.phase += dt * 3.0;
        }
    }

    fn turn_toward(&mut self, dir: Vec2, rate: f32) {
        let want = dir.x.atan2(dir.y);
        let mut diff = want - self.heading;
        while diff > std::f32::consts::PI { diff -= std::f32::consts::TAU; }
        while diff < -std::f32::consts::PI { diff += std::f32::consts::TAU; }
        self.heading += diff.clamp(-rate, rate);
    }
}

struct Habitat {
    deer: bool,
    meadow: bool,
    flowers: bool,
}

fn habitat_of(p: Vec2) -> Option<Habitat> {
    habitat(p)
}

/// What could live at `p`: dry land, off the roads, not too steep or snowy.
fn habitat(p: Vec2) -> Option<Habitat> {
    let raw = terrain::raw_height(p.x, p.y);
    if raw < WATER_LEVEL + 1.0 || raw > 150.0 {
        return None;
    }
    if road::network().and_then(|n| n.nearest(p.x, p.y)).is_some_and(|s| s.dist < s.half_width() + road::KERB_WIDTH + 3.0) {
        return None;
    }
    let desert = terrain::biome_desert(p.x, p.y);
    let tundra = terrain::biome_tundra(p.x, p.y);
    let forest = terrain::biome_forest(p.x, p.y);
    let mountain = terrain::biome_mountain(p.x, p.y);
    let grass = 1.0 - desert - tundra - forest - mountain;
    Some(Habitat {
        deer: desert < 0.2 && tundra < 0.5,
        meadow: grass > 0.5,
        flowers: grass > 0.5 && noise(p.x - 200.0, p.y + 450.0, 70.0) > 0.55,
    })
}

// ── Models ──

#[derive(Default)]
struct Builder {
    verts: Vec<Vertex>,
    idxs: Vec<u16>,
}

impl Builder {
    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: [f32; 3]) {
        if self.verts.len() > 65000 {
            return;
        }
        let base = self.verts.len() as u16;
        for p in [a, b, c] {
            self.verts.push(Vertex { pos: p.into(), color: [col[0], col[1], col[2], 1.0] });
        }
        self.idxs.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Box with corners given in the animal's frame (`to_world` places them).
    fn block(&mut self, to_world: &impl Fn(Vec3) -> Vec3, lo: Vec3, hi: Vec3, col: [f32; 3], belly: [f32; 3]) {
        let c = |x: f32, y: f32, z: f32| to_world(vec3(x, y, z));
        let (a, b) = (lo, hi);
        let p = [c(a.x, a.y, a.z), c(b.x, a.y, a.z), c(b.x, a.y, b.z), c(a.x, a.y, b.z), c(a.x, b.y, a.z), c(b.x, b.y, a.z), c(b.x, b.y, b.z), c(a.x, b.y, b.z)];
        for (q, colr) in [([4, 5, 6, 7], col), ([0, 3, 2, 1], belly), ([0, 1, 5, 4], col), ([2, 3, 7, 6], col), ([1, 2, 6, 5], col), ([3, 0, 4, 7], col)] {
            self.tri(p[q[0]], p[q[1]], p[q[2]], colr);
            self.tri(p[q[0]], p[q[2]], p[q[3]], colr);
        }
    }

    /// Square-section limb from `a` to `b` (in the animal's frame), `t` thick.
    fn limb(&mut self, to_world: &impl Fn(Vec3) -> Vec3, a: Vec3, b: Vec3, t: f32, col: [f32; 3]) {
        let axis = (b - a).normalize_or_zero();
        let side = axis.cross(Vec3::X).try_normalize().unwrap_or(Vec3::Z) * t;
        let up = axis.cross(side).normalize_or_zero() * t;
        let corners = [side + up, side - up, -side - up, -side + up];
        for k in 0..4 {
            let (c0, c1) = (corners[k], corners[(k + 1) % 4]);
            let (p0, p1, q0, q1) = (to_world(a + c0), to_world(a + c1), to_world(b + c0), to_world(b + c1));
            self.tri(p0, q0, q1, col);
            self.tri(p0, q1, p1, col);
        }
    }

    /// Soft dark blob on the ground, shrinking as the animal leaps.
    fn shadow(&mut self, at: Vec3, heading: f32, half_width: f32, half_length: f32, lift: f32) {
        let rot = Quat::from_rotation_y(heading);
        let s = 1.0 / (1.0 + lift * 0.4);
        let col = [0.1, 0.12, 0.08];
        let sides = 8;
        for k in 0..sides {
            let a0 = std::f32::consts::TAU * k as f32 / sides as f32;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / sides as f32;
            let p = |a: f32| at + rot * vec3(a.cos() * half_width * s, 0.0, a.sin() * half_length * s);
            self.tri(at, p(a1), p(a0), col);
        }
    }

    /// Deer, about 3.5 units long (scaled by `age`). +Z is forward in its own frame.
    #[allow(clippy::too_many_arguments)]
    fn deer(&mut self, at: Vec3, heading: f32, phase: f32, head_down: f32, moving: bool, stag: bool, age: f32, herd: u32) {
        let rot = Quat::from_rotation_y(heading);
        let size = 1.2 * age;
        let w = |p: Vec3| at + rot * (p * size);
        // Each herd has its own shade of brown
        let tint = 0.9 + 0.2 * ((herd as f32 * 0.618).fract());
        let coat = [0.55 * tint, 0.36 * tint, 0.2 * tint];
        let pale = [0.85, 0.78, 0.65];
        let dark = [0.2, 0.15, 0.1];
        if age < 1.0 {
            // Fawns are dappled
            for (x, y, z) in [(0.46, 2.0, 0.6), (0.46, 1.8, -0.2), (-0.46, 2.1, 0.1), (-0.46, 1.85, -0.7), (0.3, 2.3, -0.5)] {
                let p = vec3(x, y, z);
                self.block(&w, p - Vec3::splat(0.07), p + Vec3::splat(0.07), pale, pale);
            }
        }
        // Body, slightly tapered by using two blocks
        self.block(&w, vec3(-0.45, 1.5, -1.3), vec3(0.45, 2.3, 0.3), coat, pale);
        self.block(&w, vec3(-0.5, 1.45, 0.2), vec3(0.5, 2.4, 1.2), coat, pale);
        // White rump and tail
        self.block(&w, vec3(-0.3, 1.8, -1.42), vec3(0.3, 2.25, -1.28), pale, pale);
        // Neck and head: lowered to graze, raised to look about
        let neck_base = vec3(0.0, 2.2, 1.0);
        let lift = 1.0 - head_down;
        let head = neck_base + vec3(0.0, -1.3 + 2.4 * lift, 1.0 - 0.3 * lift);
        self.limb(&w, neck_base, head + vec3(0.0, 0.0, 0.1), 0.2, coat);
        self.block(&w, head - vec3(0.2, 0.2, 0.15), head + vec3(0.2, 0.25, 0.55), coat, pale);
        self.block(&w, head + vec3(-0.08, -0.05, 0.5), head + vec3(0.08, 0.1, 0.62), dark, dark);
        for s in [-1.0, 1.0] {
            // Ears
            let ear = head + vec3(s * 0.25, 0.3, -0.05);
            self.tri(w(ear), w(ear + vec3(s * 0.35, 0.3, -0.1)), w(ear + vec3(0.0, 0.1, 0.12)), coat);
            if stag {
                // Antlers: a main beam with two tines
                let base = head + vec3(s * 0.12, 0.3, 0.05);
                let tip = base + vec3(s * 0.5, 1.0, -0.2);
                let antler = [0.75, 0.68, 0.55];
                self.block(&w, base.min(tip) - Vec3::splat(0.04), base.max(tip) + Vec3::splat(0.04), antler, antler);
                let mid = (base + tip) * 0.5;
                self.block(&w, mid - Vec3::splat(0.04), mid + vec3(s * 0.05, 0.45, 0.25), antler, antler);
            }
        }
        // Legs swing when moving
        for (k, (x, z)) in [(-0.3, 0.95), (0.3, 0.95), (-0.3, -1.0), (0.3, -1.0)].into_iter().enumerate() {
            let swing = if moving { (phase * 1.0 + if k % 3 == 0 { 0.0 } else { std::f32::consts::PI }).sin() * 0.45 } else { 0.0 };
            let hip = vec3(x, 1.6, z);
            let hoof = hip + vec3(0.0, -1.6, swing);
            self.limb(&w, hip, hoof + vec3(0.0, 0.15, 0.0), 0.09, coat);
            self.block(&w, hoof - vec3(0.1, 0.0, 0.1), hoof + vec3(0.1, 0.18, 0.1), dark, dark);
        }
    }

    /// Rabbit: a hunched grey-brown lump with long ears and a white scut.
    fn rabbit(&mut self, at: Vec3, heading: f32, phase: f32, head_down: f32) {
        let rot = Quat::from_rotation_y(heading);
        let w = |p: Vec3| at + rot * (p * 1.6);
        let fur = [0.52, 0.45, 0.36];
        let pale = [0.85, 0.82, 0.76];
        self.block(&w, vec3(-0.25, 0.0, -0.35), vec3(0.25, 0.45, 0.25), fur, pale);
        let head = vec3(0.0, 0.45 - 0.15 * head_down, 0.35);
        self.block(&w, head - vec3(0.16, 0.12, 0.1), head + vec3(0.16, 0.16, 0.2), fur, fur);
        let flop = (phase * 2.0).sin() * 0.05;
        for s in [-1.0, 1.0] {
            let base = head + vec3(s * 0.07, 0.14, -0.02);
            self.block(&w, base - vec3(0.04, 0.0, 0.04), base + vec3(0.04 + s * flop, 0.42, 0.07), fur, pale);
        }
        self.block(&w, vec3(-0.09, 0.2, -0.45), vec3(0.09, 0.36, -0.33), pale, pale);
    }

    /// Butterfly: two flapping wings either side of a dark body.
    fn butterfly(&mut self, at: Vec3, heading: f32, phase: f32, colour: [f32; 3]) {
        let rot = Quat::from_rotation_y(heading);
        let flap = (phase * 9.0).sin() * 1.1;
        let body = [0.12, 0.1, 0.08];
        let front = at + rot * vec3(0.0, 0.0, 0.18);
        let back = at + rot * vec3(0.0, 0.0, -0.18);
        for s in [-1.0_f32, 1.0] {
            let wing = Quat::from_axis_angle(rot * Vec3::Z, s * flap);
            let out = |x: f32, z: f32| at + wing * (rot * vec3(s * x, 0.0, z));
            self.tri(front, out(0.55, 0.3), out(0.5, -0.05), colour);
            self.tri(back, out(0.45, -0.05), out(0.35, -0.35), [colour[0] * 0.8, colour[1] * 0.8, colour[2] * 0.8]);
        }
        self.tri(front, back, at + Vec3::Y * 0.06, body);
    }
}
