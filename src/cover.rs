//! Ground cover: the small plants and stones between the trees, built only
//! near the camera. Meadows get grass tufts and drifts of wildflowers, forests
//! ferns, mushrooms and fallen logs, shores reeds and lily pads, the desert dry
//! tufts, pebbles and barrel cacti, the tundra and peaks rocks and alpine
//! flowers, and farm fields their crops in rows.
//!
//! Vertex alpha is height above the ground / 4, which the mesh shader uses for
//! wind sway and to sink plants into the ground as they near the edge of range.

use glam::{Quat, Vec2, Vec3, vec3};

use crate::farms::{self, Crop, Farm, Field, Spot};
use crate::geometry::Vertex;
use crate::landmarks::{self, Kind};
use crate::noise::{hash, noise, patchy};
use crate::road;
use crate::terrain::{self, terrain_height, TERRAIN_HALF_SIZE, WATER_LEVEL};

/// Placement grid spacing.
const CELL: f32 = 5.0;
/// Heights are stored in vertex alpha as a fraction of this.
const ALPHA_HEIGHT: f32 = 4.0;

type Rgb = [f32; 3];

fn vary(c: Rgb, seed: f32, amount: f32) -> Rgb {
    let b = 1.0 + (hash(seed, 1.3) - 0.5) * 2.0 * amount;
    let w = (hash(seed, 5.9) - 0.5) * amount;
    [(c[0] + w * 0.5) * b, (c[1] + w * 0.3) * b, c[2] * b]
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

const GRASS: Rgb = [0.33, 0.58, 0.18];
const GRASS_DRY: Rgb = [0.66, 0.62, 0.32];
const FERN: Rgb = [0.2, 0.46, 0.15];
const REED: Rgb = [0.4, 0.55, 0.22];
const LILY: Rgb = [0.2, 0.5, 0.2];
const STONE: Rgb = [0.5, 0.48, 0.45];
const BARK: Rgb = [0.36, 0.26, 0.16];
const MOSS: Rgb = [0.3, 0.45, 0.16];
/// Wildflower colours: daisy, buttercup, poppy, cornflower, lavender, pink campion.
const BLOOMS: [Rgb; 6] = [[0.95, 0.95, 0.92], [0.98, 0.85, 0.15], [0.9, 0.18, 0.12], [0.3, 0.45, 0.95], [0.65, 0.45, 0.85], [0.95, 0.5, 0.7]];

struct Cover {
    verts: Vec<Vertex>,
    idxs: Vec<u32>,
    ground: f32,
}

impl Cover {
    fn vertex(&mut self, p: Vec3, c: Rgb) -> u32 {
        let a = ((p.y - self.ground) / ALPHA_HEIGHT).clamp(0.0, 1.0);
        self.verts.push(Vertex { pos: p.into(), color: [c[0], c[1], c[2], a] });
        self.verts.len() as u32 - 1
    }

    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: Rgb) {
        let i = [self.vertex(a, col), self.vertex(b, col), self.vertex(c, col)];
        self.idxs.extend_from_slice(&i);
    }

    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, col: Rgb) {
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    /// Small jittered octahedron.
    fn pebble(&mut self, c: Vec3, r: f32, col: Rgb, seed: f32) {
        let j = |k: f32| 0.7 + 0.6 * hash(seed, k);
        let rot = Quat::from_rotation_y(seed * 3.0);
        let p = [
            vec3(r * j(1.0), 0.0, 0.0), vec3(-r * j(2.0), 0.0, 0.0),
            vec3(0.0, r * 0.75 * j(3.0), 0.0), vec3(0.0, -r * 0.4, 0.0),
            vec3(0.0, 0.0, r * j(4.0)), vec3(0.0, 0.0, -r * j(5.0)),
        ].map(|v| c + rot * v);
        for (k, (a, b, d)) in [(0, 2, 4), (4, 2, 1), (1, 2, 5), (5, 2, 0), (4, 3, 0), (1, 3, 4), (5, 3, 1), (0, 3, 5)].into_iter().enumerate() {
            self.tri(p[a], p[b], p[d], vary(col, seed + k as f32, 0.06));
        }
    }

    /// A clump of grass blades leaning out from `at`.
    fn tuft(&mut self, at: Vec3, height: f32, col: Rgb, seed: f32) {
        let blades = 5 + (hash(seed, 2.0) * 3.0) as usize;
        for k in 0..blades {
            let a = seed * 2.0 + k as f32 * 2.39;
            let dir = vec3(a.cos(), 0.0, a.sin());
            let side = vec3(-a.sin(), 0.0, a.cos()) * 0.3;
            let h = height * (0.65 + 0.5 * hash(seed, k as f32));
            let base = at + dir * 0.15;
            let tip = at + dir * h * 0.45 + Vec3::Y * h;
            let shade = if k % 2 == 0 { col } else { vary(col, seed + k as f32, 0.12) };
            self.tri(base - side, base + side, tip, shade);
        }
    }

    /// A wildflower: a stem, two leaves and a little star-shaped head.
    fn flower(&mut self, at: Vec3, height: f32, bloom: Rgb, seed: f32) {
        let lean = vec3(hash(seed, 3.0) - 0.5, 0.0, hash(seed, 4.0) - 0.5) * 0.4;
        let head = at + Vec3::Y * height + lean;
        let stem = vec3(0.07, 0.0, 0.0);
        self.tri(at - stem, at + stem, head, GRASS);
        for s in [-1.0, 1.0] {
            let leaf = at + vec3(s * 0.4, height * 0.35, 0.1);
            self.tri(at, at + Vec3::Y * 0.15, leaf, GRASS);
        }
        let petals = 5;
        let r = 0.45 + 0.2 * hash(seed, 6.0);
        let centre = head + Vec3::Y * 0.05;
        for k in 0..petals {
            let a0 = seed + k as f32 * std::f32::consts::TAU / petals as f32;
            let a1 = a0 + std::f32::consts::TAU / petals as f32 * 0.5;
            let p0 = head + vec3(a0.cos(), 0.0, a0.sin()) * r;
            let p1 = head + vec3(a1.cos(), 0.0, a1.sin()) * r * 0.45;
            self.tri(centre, p0, p1, bloom);
            let a2 = a0 - std::f32::consts::TAU / petals as f32 * 0.5;
            let p2 = head + vec3(a2.cos(), 0.0, a2.sin()) * r * 0.45;
            self.tri(centre, p2, p0, bloom);
        }
        self.pebble(centre + Vec3::Y * 0.02, 0.14, [0.95, 0.75, 0.1], seed);
    }

    /// A clump of wheat: upright stalks, each topped with a darker ear.
    fn wheat(&mut self, at: Vec3, height: f32, seed: f32) {
        const STALK: Rgb = [0.86, 0.72, 0.34];
        const EAR: Rgb = [0.74, 0.55, 0.22];
        for k in 0..4 {
            let a = seed * 1.7 + k as f32 * 1.57;
            let dir = vec3(a.cos(), 0.0, a.sin());
            let side = vec3(-a.sin(), 0.0, a.cos()) * 0.06;
            let h = height * (0.85 + 0.25 * hash(seed, k as f32));
            let base = at + dir * 0.2;
            let top = base + dir * 0.15 + Vec3::Y * h;
            self.tri(base - side, base + side, top, vary(STALK, seed + k as f32, 0.06));
            let ear = side * 2.2;
            self.tri(top - ear - Vec3::Y * 0.5, top + ear - Vec3::Y * 0.5, top + Vec3::Y * 0.25, EAR);
        }
    }

    /// Rapeseed: a stalk crowned with clusters of bright yellow flowers.
    fn rapeseed(&mut self, at: Vec3, height: f32, seed: f32) {
        const BLOOM: Rgb = [0.98, 0.86, 0.12];
        let side = vec3(0.08, 0.0, 0.0);
        let top = at + Vec3::Y * height;
        self.tri(at - side, at + side, top, [0.3, 0.5, 0.15]);
        for k in 0..2 {
            let a = seed + k as f32 * 3.1;
            let off = vec3(a.cos(), -0.15 * k as f32, a.sin()) * 0.35;
            self.pebble(top + off, 0.38, vary(BLOOM, seed + k as f32, 0.05), seed + k as f32);
        }
    }

    /// A sunflower: a tall stem, two broad leaves, and a dark-centred head
    /// turned toward `face`.
    fn sunflower(&mut self, at: Vec3, height: f32, face: Vec3, seed: f32) {
        const STEM: Rgb = [0.3, 0.5, 0.16];
        const PETAL: Rgb = [0.98, 0.78, 0.1];
        const DISC: Rgb = [0.3, 0.18, 0.08];
        let head = at + Vec3::Y * height;
        let s = vec3(0.09, 0.0, 0.09);
        self.tri(at - s, at + s, head, STEM);
        self.tri(at - vec3(s.x, 0.0, -s.z), at + vec3(s.x, 0.0, -s.z), head, STEM);
        for (k, side) in [-1.0, 1.0].into_iter().enumerate() {
            let a = seed + k as f32 * 3.0;
            let leaf_at = at + Vec3::Y * height * (0.4 + 0.15 * k as f32);
            let tip = leaf_at + vec3(a.cos(), 0.0, a.sin()) * 0.9 * side + Vec3::Y * 0.1;
            let w = vec3(-a.sin(), 0.0, a.cos()) * 0.3;
            self.tri(leaf_at, tip + w, tip - w, STEM);
        }
        // The head nods a little, facing the way the whole field does
        let f = (face * 0.9 - Vec3::Y * 0.3 + vec3(hash(seed, 7.0) - 0.5, 0.0, hash(seed, 8.0) - 0.5) * 0.3).normalize();
        let right = Vec3::Y.cross(f).normalize_or_zero();
        let up = f.cross(right);
        let centre = head + f * 0.15;
        let ring = |k: f32, r: f32| centre + (right * k.cos() + up * k.sin()) * r;
        let petals = 10;
        let step = std::f32::consts::TAU / petals as f32;
        for k in 0..petals {
            let a = seed + k as f32 * step;
            self.tri(ring(a - step * 0.4, 0.45), ring(a + step * 0.4, 0.45), ring(a, 1.05 + 0.1 * hash(seed, k as f32)), PETAL);
            self.tri(centre + f * 0.05, ring(a, 0.48), ring(a + step, 0.48), DISC);
        }
    }

    /// One stretch of ploughed ridge, from `at` along `run`.
    fn furrow(&mut self, at: Vec3, run: Vec3, seed: f32) {
        let across = Vec3::Y.cross(run).normalize_or_zero() * 0.75;
        let crest = Vec3::Y * (0.3 + 0.08 * hash(seed, 1.0));
        let col = vary([0.34, 0.24, 0.15], seed, 0.05);
        self.quad(at - across, at + run - across, at + run + crest, at + crest, col);
        self.quad(at + crest, at + run + crest, at + run + across, at + across, col);
    }

    /// A little bunch of flowers of one kind, with grass among them.
    fn bunch(&mut self, at: Vec3, ground: impl Fn(f32, f32) -> f32, bloom: Rgb, seed: f32) {
        let count = 2 + (hash(seed, 12.0) * 3.0) as usize;
        for k in 0..count {
            let a = seed + k as f32 * 2.39;
            let off = vec3(a.cos(), 0.0, a.sin()) * (0.6 + 1.2 * hash(seed, k as f32 + 20.0));
            let p = at + off;
            let p = vec3(p.x, ground(p.x, p.z) - 0.05, p.z);
            self.flower(p, 1.3 + hash(seed, k as f32 + 30.0) * 0.8, vary(bloom, seed + k as f32, 0.06), seed + k as f32 * 7.0);
        }
        self.tuft(at, 1.3, GRASS, seed);
    }

    /// Fern: arching fronds that rise from the middle and droop at the tips.
    fn fern(&mut self, at: Vec3, size: f32, col: Rgb, seed: f32) {
        let fronds = 5 + (hash(seed, 7.0) * 3.0) as usize;
        for k in 0..fronds {
            let a = seed + k as f32 * std::f32::consts::TAU / fronds as f32;
            let out = vec3(a.cos(), 0.0, a.sin());
            let side = vec3(-a.sin(), 0.0, a.cos()) * size * 0.16;
            let mid = at + out * size * 0.5 + Vec3::Y * size * 0.55;
            let tip = at + out * size * 1.05 + Vec3::Y * size * 0.25;
            let c = vary(col, seed + k as f32, 0.1);
            self.tri(at, mid + side, mid - side, c);
            self.tri(mid - side, mid + side, tip, vary(c, seed, 0.08));
        }
    }

    /// Toadstool: a pale stem under a domed cap.
    fn mushroom(&mut self, at: Vec3, size: f32, seed: f32) {
        let cap: Rgb = if hash(seed, 8.0) < 0.5 { [0.8, 0.15, 0.1] } else { [0.55, 0.38, 0.22] };
        let stem: Rgb = [0.92, 0.9, 0.82];
        let sides = 6;
        let ring = |c: Vec3, r: f32, k: usize| {
            let a = std::f32::consts::TAU * k as f32 / sides as f32;
            c + vec3(a.cos() * r, 0.0, a.sin() * r)
        };
        let top = at + Vec3::Y * size;
        for k in 0..sides {
            self.quad(ring(at, size * 0.12, k), ring(at, size * 0.12, k + 1), ring(top, size * 0.1, k + 1), ring(top, size * 0.1, k), stem);
            self.tri(top + Vec3::Y * size * 0.45, ring(top, size * 0.5, k + 1), ring(top, size * 0.5, k), cap);
            self.tri(top + Vec3::Y * size * 0.05, ring(top, size * 0.5, k), ring(top, size * 0.5, k + 1), [0.85, 0.8, 0.7]);
        }
        // A few pale spots on the red caps
        if cap[0] > 0.7 {
            for k in 0..3 {
                let a = seed + k as f32 * 2.1;
                self.pebble(top + vec3(a.cos() * size * 0.22, size * 0.28, a.sin() * size * 0.22), size * 0.06, [0.95, 0.93, 0.88], seed + k as f32);
            }
        }
    }

    /// Fallen log: a bark-brown prism lying on the ground, moss on top.
    fn log(&mut self, at: Vec3, length: f32, radius: f32, yaw: f32, seed: f32) {
        let axis = vec3(yaw.cos(), 0.0, yaw.sin());
        let a = at - axis * length * 0.5 + Vec3::Y * radius * 0.7;
        let b = at + axis * length * 0.5 + Vec3::Y * radius * 0.7;
        let sides = 6;
        let rot = Quat::from_rotation_arc(Vec3::Y, axis);
        let ring = |c: Vec3, k: usize| {
            let t = std::f32::consts::TAU * k as f32 / sides as f32 + seed;
            c + rot * vec3(t.cos() * radius, 0.0, t.sin() * radius)
        };
        for k in 0..sides {
            let (p0, p1, q0, q1) = (ring(a, k), ring(a, k + 1), ring(b, k), ring(b, k + 1));
            let up = (p0 + p1).y * 0.5 > a.y + radius * 0.3;
            let col = if up { vary(MOSS, seed + k as f32, 0.1) } else { vary(BARK, seed + k as f32, 0.1) };
            self.quad(p0, q0, q1, p1, col);
            // Pale cut ends
            self.tri(a, p1, p0, [0.7, 0.58, 0.4]);
            self.tri(b, q0, q1, [0.7, 0.58, 0.4]);
        }
    }

    /// Reeds: a stand of tall blades, some with brown cattail heads.
    fn reeds(&mut self, at: Vec3, height: f32, seed: f32) {
        let stems = 4 + (hash(seed, 9.0) * 4.0) as usize;
        for k in 0..stems {
            let a = seed + k as f32 * 2.39;
            let off = vec3(a.cos(), 0.0, a.sin()) * 0.6 * hash(seed, k as f32);
            let h = height * (0.7 + 0.45 * hash(k as f32, seed));
            let base = at + off;
            let tip = base + Vec3::Y * h + vec3(a.cos(), 0.0, a.sin()) * h * 0.08;
            let side = vec3(-a.sin(), 0.0, a.cos()) * 0.12;
            self.tri(base - side, base + side, tip, vary(REED, seed + k as f32, 0.12));
            if k % 3 == 0 {
                let c = base + Vec3::Y * h * 0.8;
                self.quad(c - side * 1.4, c + side * 1.4, c + side * 1.4 + Vec3::Y * h * 0.15, c - side * 1.4 + Vec3::Y * h * 0.15, [0.42, 0.26, 0.14]);
            }
        }
    }

    /// Lily pad: a notched disc floating on the water, sometimes in flower.
    fn lily(&mut self, at: Vec3, r: f32, seed: f32) {
        let segs = 8;
        let notch = seed * 6.0;
        let col = vary(LILY, seed, 0.1);
        for k in 0..segs {
            let a0 = notch + 0.35 + k as f32 * (std::f32::consts::TAU - 0.7) / segs as f32;
            let a1 = notch + 0.35 + (k + 1) as f32 * (std::f32::consts::TAU - 0.7) / segs as f32;
            self.tri(at, at + vec3(a1.cos(), 0.0, a1.sin()) * r, at + vec3(a0.cos(), 0.0, a0.sin()) * r, col);
        }
        if hash(seed, 11.0) < 0.3 {
            for k in 0..5 {
                let a = seed + k as f32 * 1.256;
                let p = at + Vec3::Y * 0.1;
                self.tri(p, p + vec3(a.cos() * 0.35, 0.3, a.sin() * 0.35), p + vec3((a + 0.6).cos() * 0.35, 0.3, (a + 0.6).sin() * 0.35), [0.98, 0.7, 0.85]);
            }
        }
    }

    /// Barrel cactus: a squat ribbed drum with flowers on top.
    fn barrel(&mut self, at: Vec3, size: f32, seed: f32) {
        let sides = 8;
        let col = vary([0.32, 0.52, 0.28], seed, 0.1);
        let ring = |y: f32, r: f32, k: usize| {
            let a = std::f32::consts::TAU * k as f32 / sides as f32;
            at + vec3(a.cos() * r, y, a.sin() * r)
        };
        for k in 0..sides {
            let shade = if k % 2 == 0 { col } else { vary(col, seed, 0.15) };
            self.quad(ring(0.0, size * 0.45, k), ring(0.0, size * 0.45, k + 1), ring(size * 0.7, size * 0.5, k + 1), ring(size * 0.7, size * 0.5, k), shade);
            self.tri(at + Vec3::Y * size * 0.95, ring(size * 0.7, size * 0.5, k + 1), ring(size * 0.7, size * 0.5, k), shade);
        }
        for k in 0..3 {
            let a = seed + k as f32 * 2.1;
            self.pebble(at + vec3(a.cos() * size * 0.15, size * 0.95, a.sin() * size * 0.15), size * 0.1, [0.95, 0.4, 0.55], seed + k as f32);
        }
    }
}

/// Small plants and stones whose cell starts in [lo, hi), as one mesh.
pub fn generate(lo: Vec2, hi: Vec2) -> (Vec<Vertex>, Vec<u32>) {
    let mut c = Cover { verts: Vec::new(), idxs: Vec::new(), ground: 0.0 };
    let half = TERRAIN_HALF_SIZE;
    let first = |v: f32| ((v + half) / CELL).ceil() as i32;
    let landmarks: Vec<_> = landmarks::all().iter()
        .filter(|l| l.pos.x > lo.x - 200.0 && l.pos.x < hi.x + 200.0 && l.pos.y > lo.y - 200.0 && l.pos.y < hi.y + 200.0)
        .collect();

    for cz in first(lo.y)..first(hi.y) {
        for cx in first(lo.x)..first(hi.x) {
            let bx = -half + cx as f32 * CELL;
            let bz = -half + cz as f32 * CELL;
            let x = bx + hash(bx, bz) * CELL;
            let z = bz + hash(bz + 3.0, bx - 7.0) * CELL;
            let seed = hash(x * 0.37, z * 0.71) * 100.0;
            let roll = hash(x * 1.31, z * 0.83);
            place(&mut c, x, z, seed, roll, &landmarks);
        }
    }
    for farm in farms::all() {
        for field in &farm.fields {
            crops(&mut c, farm, field, lo, hi);
        }
    }
    (c.verts, c.idxs)
}

/// The crop in a field, in rows running away from the road, for the part of
/// the field in [lo, hi).
fn crops(c: &mut Cover, farm: &Farm, field: &Field, lo: Vec2, hi: Vec2) {
    let (row_gap, step) = match field.crop {
        Crop::Wheat => (2.2, 1.2),
        Crop::Rapeseed => (2.4, 1.4),
        Crop::Sunflowers => (3.6, 2.3),
        Crop::Shoots => (2.6, 1.3),
        Crop::Ploughed => (2.4, 4.0),
        Crop::Pasture => return,
    };
    let reach = field.half.length() + 10.0;
    if field.centre.x + reach < lo.x || field.centre.x - reach > hi.x || field.centre.y + reach < lo.y || field.centre.y - reach > hi.y {
        return;
    }
    let inside = |p: Vec2| p.x >= lo.x && p.y >= lo.y && p.x < hi.x && p.y < hi.y;
    let rows = (field.half.x * 2.0 / row_gap) as i32;
    let steps = (field.half.y * 2.0 / step) as i32;
    // Sunflowers all turn the same way: toward the road
    let face = -vec3(farm.out.x, 0.0, farm.out.y);
    let out = vec3(farm.out.x, 0.0, farm.out.y);
    for r in 0..rows {
        let lx = -field.half.x + (r as f32 + 0.5) * row_gap;
        for k in 0..steps {
            let ly = -field.half.y + (k as f32 + 0.5) * step;
            let mut p = field.centre + farm.along * lx + farm.out * ly;
            if !inside(p) {
                continue;
            }
            let seed = hash(p.x * 0.37, p.y * 0.71) * 100.0;
            if field.crop != Crop::Ploughed {
                p += farm.out * (hash(seed, 1.0) - 0.5) * step * 0.6 + farm.along * (hash(seed, 2.0) - 0.5) * 0.4;
            }
            let ground = terrain_height(p.x, p.y);
            c.ground = ground;
            let at = vec3(p.x, ground - 0.05, p.y);
            match field.crop {
                Crop::Wheat => c.wheat(at, 1.7 + 0.4 * hash(seed, 3.0), seed),
                Crop::Rapeseed => c.rapeseed(at, 1.2 + 0.4 * hash(seed, 3.0), seed),
                Crop::Sunflowers => c.sunflower(at, 2.9 + 0.9 * hash(seed, 3.0), face, seed),
                Crop::Shoots => c.tuft(at, 0.5 + 0.3 * hash(seed, 3.0), vary([0.38, 0.66, 0.18], seed, 0.08), seed),
                Crop::Ploughed => c.furrow(vec3(p.x, ground - 0.1, p.y), out * step, seed),
                Crop::Pasture => {}
            }
        }
    }
}

fn place(c: &mut Cover, x: f32, z: f32, seed: f32, roll: f32, landmarks: &[&landmarks::Landmark]) {
    let raw = terrain::raw_height(x, z);
    let desert = terrain::biome_desert(x, z);
    let tundra = terrain::biome_tundra(x, z);

    // Keep off road surfaces (the shoulders are fair game) and out of buildings
    if let Some(s) = road::network().and_then(|n| n.nearest(x, z)) {
        if s.dist < s.half_width() + road::KERB_WIDTH + 1.0 {
            return;
        }
    }
    let p = Vec2::new(x, z);
    if landmarks.iter().any(|l| l.pos.distance(p) < if l.kind == Kind::Village { 140.0 } else { 30.0 }) {
        return;
    }
    // Crops are planted separately, in rows; pasture is just grass, and the
    // farmyard is trodden bare but for the odd stone
    match farms::at(p) {
        Some(Spot::Field(field)) => {
            if field.crop == Crop::Pasture && roll < 0.45 {
                let ground = terrain_height(x, z);
                c.ground = ground;
                c.tuft(vec3(x, ground - 0.05, z), 0.8 + 0.6 * hash(seed, 3.0), vary(GRASS, seed, 0.1), seed);
            }
            return;
        }
        Some(Spot::Yard) => {
            if roll < 0.06 {
                let ground = terrain_height(x, z);
                c.ground = ground;
                c.pebble(vec3(x, ground, z), 0.3 + 0.3 * hash(seed, 4.0), STONE, seed);
            }
            return;
        }
        None => {}
    }

    // On the water: lily pads gather in sheltered patches of the shallows
    if raw < WATER_LEVEL {
        let depth = WATER_LEVEL - raw;
        let pads = noise(x + 77.0, z - 31.0, 60.0);
        if (0.4..3.0).contains(&depth) && desert < 0.3 && tundra < 0.2 && pads > 0.62 && roll < 0.55 {
            c.ground = WATER_LEVEL;
            c.lily(vec3(x, WATER_LEVEL + 0.45, z), 0.9 + 0.6 * hash(seed, 1.0), seed);
        }
        return;
    }

    let ground = terrain_height(x, z);
    c.ground = ground;
    let at = vec3(x, ground - 0.05, z);

    // The water's edge: reeds in stands
    if raw < WATER_LEVEL + 1.2 && tundra < 0.3 {
        if noise(x - 50.0, z + 90.0, 45.0) > 0.45 && roll < 0.5 {
            c.reeds(at, 3.2 + hash(seed, 2.0) * 1.6, seed);
        }
        return;
    }

    let forest = terrain::biome_forest(x, z);
    let mountain = terrain::biome_mountain(x, z);
    let grass = (1.0 - desert - forest - mountain - tundra).max(0.0);
    let total = desert + forest + mountain + tundra + grass;
    let mut draw = hash(x * 0.53, z * 1.17) * total;
    let mut take = |w: f32| { draw -= w; draw < 0.0 };

    let moist = patchy(x, z, 700.0);
    let lush = mix(GRASS_DRY, GRASS, (moist - 0.25) * 2.5);

    if take(desert) {
        if roll < 0.04 {
            c.tuft(at, 1.0 + hash(seed, 3.0), vary(GRASS_DRY, seed, 0.12), seed);
        } else if roll < 0.06 {
            c.pebble(at, 0.4 + hash(seed, 4.0) * 0.6, vary([0.72, 0.55, 0.38], seed, 0.1), seed);
        } else if roll < 0.066 {
            c.barrel(at, 1.2 + hash(seed, 5.0) * 0.8, seed);
        }
    } else if take(tundra) {
        if roll < 0.04 {
            c.pebble(at, 0.5 + hash(seed, 4.0) * 1.2, vary(STONE, seed, 0.1), seed);
        } else if roll < 0.08 {
            c.tuft(at, 0.8 + hash(seed, 3.0) * 0.6, vary(GRASS_DRY, seed, 0.1), seed);
        }
    } else if take(forest) {
        if roll < 0.12 {
            c.fern(at, 1.6 + hash(seed, 6.0) * 1.4, FERN, seed);
        } else if roll < 0.24 {
            c.tuft(at, 1.2 + hash(seed, 3.0) * 0.8, vary(mix(GRASS, FERN, 0.6), seed, 0.1), seed);
        } else if roll < 0.25 {
            c.mushroom(at, 0.7 + hash(seed, 7.0) * 0.6, seed);
        } else if roll < 0.258 {
            c.pebble(at, 0.6 + hash(seed, 4.0), vary(mix(STONE, MOSS, 0.4), seed, 0.1), seed);
        } else if roll < 0.2595 {
            c.log(at, 7.0 + hash(seed, 8.0) * 6.0, 0.6 + hash(seed, 9.0) * 0.4, seed, seed);
        }
    } else if take(mountain) {
        let alpine = noise(x + 300.0, z, 90.0) > 0.6;
        if ground > 160.0 {
            if roll < 0.04 {
                c.pebble(at, 0.5 + hash(seed, 4.0) * 1.3, vary(STONE, seed, 0.1), seed);
            }
        } else if alpine && roll < 0.18 {
            c.bunch(at, terrain_height, BLOOMS[if hash(seed, 9.0) < 0.5 { 3 } else { 4 }], seed);
        } else if roll < 0.3 {
            c.tuft(at, 1.3 + hash(seed, 3.0) * 0.9, vary(mix(lush, GRASS_DRY, 0.3), seed, 0.1), seed);
        } else if roll < 0.33 {
            c.pebble(at, 0.5 + hash(seed, 4.0) * 1.2, vary(STONE, seed, 0.1), seed);
        }
    } else {
        // Meadow: wildflowers in drifts of one or two colours, grass all round
        let drift = noise(x - 200.0, z + 450.0, 70.0);
        let palette = noise(x + 1000.0, z - 700.0, 180.0);
        if drift > 0.6 && roll < 0.5 {
            let k = ((palette * 5.0) as usize + (hash(seed, 10.0) < 0.25) as usize) % BLOOMS.len();
            c.bunch(at, terrain_height, BLOOMS[k], seed);
        } else if roll < 0.22 + 0.25 * moist {
            c.tuft(at, 1.3 + hash(seed, 3.0) * 1.2, vary(lush, seed, 0.12), seed);
        } else if roll < 0.23 + 0.25 * moist {
            c.pebble(at, 0.3 + hash(seed, 4.0) * 0.5, vary(STONE, seed, 0.1), seed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dev aid: print land spots for looking at each kind of ground cover.
    #[test]
    #[ignore]
    fn find_cover_spots() {
        let _ = crate::sim_tests::network();
        let mut found = [false; 3];
        let mut z = -7000.0;
        while z < 7000.0 {
            let mut x = -7000.0;
            while x < 7000.0 {
                let raw = terrain::raw_height(x, z);
                let clear = road::network().and_then(|n| n.nearest(x, z)).is_none();
                if raw > WATER_LEVEL + 6.0 && clear {
                    let forest = terrain::biome_forest(x, z);
                    let grass = 1.0 - terrain::biome_desert(x, z) - forest - terrain::biome_mountain(x, z) - terrain::biome_tundra(x, z);
                    let drift = noise(x - 200.0, z + 450.0, 70.0);
                    let kinds = [grass > 0.9 && drift > 0.7, forest > 0.8, terrain::biome_mountain(x, z) > 0.5 && raw < 150.0 && noise(x + 300.0, z, 90.0) > 0.7];
                    for (k, ok) in kinds.into_iter().enumerate() {
                        if ok && !found[k] {
                            found[k] = true;
                            println!("kind {k} at {x:.1},{z:.1} raw {raw:.0}");
                        }
                    }
                }
                x += 37.3;
            }
            z += 41.7;
        }
    }
}

