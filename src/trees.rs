//! Trees and shrubs, chosen and spaced per biome: dense conifer forest with
//! clearings in the north-west, groves and lone trees across the grassland,
//! a treeline on the mountains, snowy stragglers on the tundra, cacti in the
//! desert and palms round its water.
//!
//! Plants are built as foliage: vertex alpha holds a sway weight (0 at the
//! base, rising up the plant) that the mesh shader uses for wind and impacts.

use glam::{Quat, Vec2, Vec3, vec3};

use crate::geometry::Vertex;
use crate::landmarks::{self, Kind};
use crate::noise::{hash, noise};
use crate::road;
use crate::terrain::{self, terrain_height, TERRAIN_HALF_SIZE, WATER_LEVEL};

/// Placement grid spacing; each cell holds at most one plant.
const CELL: f32 = 24.0;

// ── Colour ──

type Rgb = [f32; 3];

/// Brightness and a nudge toward yellow, so no two trees are quite alike.
fn vary(c: Rgb, seed: f32, amount: f32) -> Rgb {
    let bright = 1.0 + (hash(seed, 3.1) - 0.5) * 2.0 * amount;
    let warm = (hash(seed, 7.7) - 0.5) * amount;
    [(c[0] + warm * 0.5) * bright, (c[1] + warm * 0.25) * bright, c[2] * bright]
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

const BARK: Rgb = [0.33, 0.22, 0.13];
const BARK_GREY: Rgb = [0.38, 0.34, 0.3];
const PINE: Rgb = [0.13, 0.38, 0.16];
const SPRUCE: Rgb = [0.11, 0.33, 0.19];
const LEAF: Rgb = [0.22, 0.48, 0.13];
const BIRCH_LEAF: Rgb = [0.45, 0.62, 0.2];
const BIRCH_BARK: Rgb = [0.88, 0.86, 0.8];
const AUTUMN: [Rgb; 3] = [[0.85, 0.42, 0.1], [0.8, 0.62, 0.12], [0.68, 0.2, 0.1]];
const SNOW: Rgb = [0.9, 0.93, 0.97];
const CACTUS: Rgb = [0.3, 0.5, 0.25];
const DRY_SHRUB: Rgb = [0.45, 0.45, 0.25];
const PALM_LEAF: Rgb = [0.25, 0.52, 0.15];
const PALM_BARK: Rgb = [0.5, 0.4, 0.26];

// ── Mesh building ──

/// Accumulates plant geometry. Each vertex's alpha is its sway weight,
/// measured from the base of the plant being built.
struct Plants {
    verts: Vec<Vertex>,
    idxs: Vec<u32>,
    base: f32,
    /// How readily the current plant sways (per unit of height).
    give: f32,
    /// The distant version: same shapes and colours, fewer faces.
    far: bool,
}

impl Plants {
    fn new(far: bool) -> Self {
        Self { verts: Vec::new(), idxs: Vec::new(), base: 0.0, give: 0.0, far }
    }

    /// Sides for a round shape: fewer far away.
    fn sides(&self, near: usize) -> usize {
        if self.far { (near * 2 / 3).max(3) } else { near }
    }

    fn start(&mut self, base: f32, give: f32) {
        self.base = base;
        self.give = give;
    }

    fn vertex(&mut self, p: Vec3, c: Rgb) -> u32 {
        let sway = ((p.y - self.base) * self.give).clamp(0.0, 1.0);
        self.verts.push(Vertex { pos: p.into(), color: [c[0], c[1], c[2], sway] });
        self.verts.len() as u32 - 1
    }

    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: Rgb) {
        let i = [self.vertex(a, col), self.vertex(b, col), self.vertex(c, col)];
        self.idxs.extend_from_slice(&i);
    }

    /// Tapered prism from `a` to `b` (radii `r0`, `r1`), capped at the top.
    fn limb(&mut self, a: Vec3, b: Vec3, r0: f32, r1: f32, sides: usize, col: Rgb, twist: f32) {
        let sides = self.sides(sides);
        let axis = (b - a).normalize_or_zero();
        let rot = Quat::from_rotation_arc(Vec3::Y, axis);
        let ring = |c: Vec3, r: f32, k: usize| {
            let t = std::f32::consts::TAU * k as f32 / sides as f32 + twist;
            c + rot * vec3(t.cos() * r, 0.0, t.sin() * r)
        };
        for k in 0..sides {
            let (p0, p1) = (ring(a, r0, k), ring(a, r0, k + 1));
            let (q0, q1) = (ring(b, r1, k), ring(b, r1, k + 1));
            self.tri(p0, q0, q1, col);
            self.tri(p0, q1, p1, col);
            if r1 > 0.0 {
                self.tri(b, q1, q0, col);
            }
        }
    }

    /// Cone with its base ring at `base`, pointing up.
    fn cone(&mut self, base: Vec3, radius: f32, height: f32, sides: usize, col: Rgb, twist: f32, droop: f32) {
        let sides = self.sides(sides);
        let apex = base + Vec3::Y * height;
        for k in 0..sides {
            let a0 = std::f32::consts::TAU * k as f32 / sides as f32 + twist;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / sides as f32 + twist;
            // Rim points dip a little, so tiers look like drooping boughs
            let p0 = base + vec3(a0.cos() * radius, -droop, a0.sin() * radius);
            let p1 = base + vec3(a1.cos() * radius, -droop, a1.sin() * radius);
            self.tri(apex, p1, p0, col);
            // The underside can't be seen from afar
            if !self.far {
                self.tri(base + Vec3::Y * height * 0.15, p0, p1, scale(col, 0.7));
            }
        }
    }

    /// Faceted lump: a jittered icosahedron, squashed by `squash` vertically.
    fn blob(&mut self, c: Vec3, r: f32, squash: f32, col: Rgb, seed: f32) {
        if self.far {
            // An octahedron reads the same at a distance
            let spin = Quat::from_rotation_y(seed * 5.0);
            let p = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                .map(|v| c + { let d = spin * v * r; vec3(d.x, d.y * squash, d.z) });
            for (k, (a, b, d)) in [(0, 2, 4), (4, 2, 1), (1, 2, 5), (5, 2, 0), (4, 3, 0), (1, 3, 4), (5, 3, 1), (0, 3, 5)].into_iter().enumerate() {
                let shade = if k < 4 { 1.06 } else { 0.82 };
                self.tri(p[a], p[b], p[d], scale(vary(col, seed + k as f32 * 0.37, 0.05), shade));
            }
            return;
        }
        let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
        let raw = [
            vec3(-1.0, t, 0.0), vec3(1.0, t, 0.0), vec3(-1.0, -t, 0.0), vec3(1.0, -t, 0.0),
            vec3(0.0, -1.0, t), vec3(0.0, 1.0, t), vec3(0.0, -1.0, -t), vec3(0.0, 1.0, -t),
            vec3(t, 0.0, -1.0), vec3(t, 0.0, 1.0), vec3(-t, 0.0, -1.0), vec3(-t, 0.0, 1.0),
        ];
        let spin = Quat::from_rotation_y(seed * 5.0);
        let pts: Vec<Vec3> = raw.iter().enumerate().map(|(k, v)| {
            let j = 0.82 + 0.36 * hash(seed + k as f32, seed * 1.3);
            let d = spin * v.normalize() * r * j;
            c + vec3(d.x, d.y * squash, d.z)
        }).collect();
        const FACES: [[usize; 3]; 20] = [
            [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
            [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
            [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
            [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
        ];
        for (k, f) in FACES.iter().enumerate() {
            // Undersides in the canopy's shade, tops catching the light
            let mid = (pts[f[0]].y + pts[f[1]].y + pts[f[2]].y) / 3.0 - c.y;
            let shade = if mid < 0.0 { 0.82 } else { 1.0 + 0.12 * (mid / r).min(1.0) };
            let face = scale(vary(col, seed + k as f32 * 0.37, 0.05), shade);
            self.tri(pts[f[0]], pts[f[1]], pts[f[2]], face);
        }
    }
}

fn scale(c: Rgb, s: f32) -> Rgb {
    [c[0] * s, c[1] * s, c[2] * s]
}

// ── Species ──

/// Conifer: a tapered trunk under stacked drooping tiers. `slim` < 1 for spruce.
fn conifer(p: &mut Plants, at: Vec3, height: f32, slim: f32, needles: Rgb, snow: f32, seed: f32) {
    p.start(at.y, 0.035);
    let trunk_top = at + Vec3::Y * height * 0.35;
    p.limb(at - Vec3::Y * 0.5, trunk_top, height * 0.035, height * 0.025, 5, vary(BARK, seed, 0.1), seed);
    let tiers = if slim < 0.8 { 5 } else { 4 };
    let mut y = height * 0.18;
    for t in 0..tiers {
        let f = t as f32 / tiers as f32;
        let radius = height * 0.3 * slim * (1.0 - f * 0.72);
        let h = height * (0.42 - f * 0.12);
        // Lower boughs darker, tips lighter
        let col = vary(scale(needles, 0.85 + 0.3 * f), seed + t as f32, 0.08);
        let base = at + Vec3::Y * y;
        p.cone(base, radius, h, 7, col, seed + t as f32 * 0.45, radius * 0.18);
        if snow > 0.0 {
            // Snow lies on the upper part of each tier
            let cap = 0.35 + 0.35 * snow;
            p.cone(base + Vec3::Y * h * (1.0 - cap) + Vec3::Y * 0.05, radius * cap * 1.04, h * cap, 7, SNOW, seed + t as f32 * 0.45, 0.0);
        }
        y += h * 0.55;
    }
}

/// Broadleaf: a trunk splitting into a couple of limbs under a lumpy crown.
fn broadleaf(p: &mut Plants, at: Vec3, height: f32, leaves: Rgb, seed: f32) {
    p.start(at.y, 0.03);
    let bark = vary(BARK, seed, 0.12);
    let fork = at + Vec3::Y * height * 0.45;
    p.limb(at - Vec3::Y * 0.5, fork, height * 0.06, height * 0.045, 6, bark, seed);
    let crown = at + Vec3::Y * height * 0.72;
    let spread = height * 0.32;
    let lumps = if p.far { 3 } else { 4 + (hash(seed, 1.0) * 3.0) as usize };
    for k in 0..lumps {
        let a = seed * 3.0 + k as f32 * 2.4;
        let out = if k == 0 { 0.0 } else { spread * (0.55 + 0.35 * hash(seed, k as f32)) };
        let c = crown + vec3(a.cos() * out, (hash(k as f32, seed) - 0.3) * height * 0.22, a.sin() * out);
        if k > 0 && k < 3 && !p.far {
            p.limb(fork, c - Vec3::Y * height * 0.08, height * 0.03, height * 0.015, 4, bark, seed + k as f32);
        }
        let r = height * (0.24 + 0.1 * hash(seed + 5.0, k as f32)) * if k == 0 { 1.25 } else { 1.0 };
        p.blob(c, r, 0.8, vary(leaves, seed + k as f32 * 0.3, 0.06), seed + k as f32 * 1.7);
    }
}

/// Birch: slender white trunk with dark bands and a light, open crown.
fn birch(p: &mut Plants, at: Vec3, height: f32, leaves: Rgb, seed: f32) {
    p.start(at.y, 0.04);
    let lean = vec3(hash(seed, 2.0) - 0.5, 0.0, hash(seed, 4.0) - 0.5) * height * 0.08;
    let bands = if p.far { 2 } else { 6 };
    for k in 0..bands {
        let a = at + Vec3::Y * height * 0.75 * k as f32 / bands as f32 + lean * (k as f32 / bands as f32);
        let b = at + Vec3::Y * height * 0.75 * (k + 1) as f32 / bands as f32 + lean * ((k + 1) as f32 / bands as f32);
        let col = if (k + (seed as usize)) % 3 == 1 { [0.2, 0.19, 0.18] } else { BIRCH_BARK };
        let r = height * 0.03 * (1.0 - 0.4 * k as f32 / bands as f32);
        p.limb(a - Vec3::Y * if k == 0 { 0.5 } else { 0.0 }, b, r, r * 0.92, 5, col, seed);
    }
    let top = at + Vec3::Y * height * 0.75 + lean;
    for k in 0..if p.far { 2 } else { 3 } {
        let a = seed + k as f32 * 2.1;
        let c = top + vec3(a.cos() * height * 0.1, (k as f32 - 0.5) * height * 0.12, a.sin() * height * 0.1);
        p.blob(c, height * 0.17, 1.25, vary(leaves, seed + k as f32, 0.08), seed + k as f32 * 3.1);
    }
}

/// Low shrub: two or three lumps hugging the ground.
fn bush(p: &mut Plants, at: Vec3, size: f32, leaves: Rgb, seed: f32) {
    p.start(at.y, 0.05);
    let lumps = if p.far { 1 } else { 2 + (hash(seed, 9.0) * 2.0) as usize };
    for k in 0..lumps {
        let a = seed + k as f32 * 2.3;
        let c = at + vec3(a.cos() * size * 0.5, size * 0.35, a.sin() * size * 0.5);
        p.blob(c, size * (0.55 + 0.2 * hash(seed, k as f32)), 0.7, vary(leaves, seed + k as f32, 0.08), seed + k as f32);
    }
}

/// Saguaro: a ribbed column with an arm or two curving up.
fn cactus(p: &mut Plants, at: Vec3, height: f32, seed: f32) {
    p.start(at.y, 0.01);
    let col = vary(CACTUS, seed, 0.1);
    let r = height * 0.08;
    let top = at + Vec3::Y * height;
    p.limb(at - Vec3::Y * 0.5, top - Vec3::Y * r, r, r * 0.9, 8, col, seed);
    p.cone(top - Vec3::Y * r, r * 0.9, r * 1.2, 8, col, seed, 0.0);
    let arms = 1 + (hash(seed, 1.0) * 2.0) as usize;
    for k in 0..arms {
        let a = seed * 2.0 + k as f32 * std::f32::consts::PI;
        let dir = vec3(a.cos(), 0.0, a.sin());
        let from = at + Vec3::Y * height * (0.35 + 0.15 * hash(seed, k as f32 + 3.0));
        let elbow = from + dir * height * 0.22 + Vec3::Y * height * 0.06;
        let tip = elbow + Vec3::Y * height * (0.22 + 0.12 * hash(seed, k as f32 + 5.0));
        p.limb(from, elbow, r * 0.65, r * 0.6, 6, col, seed);
        p.limb(elbow - Vec3::Y * r * 0.4, tip, r * 0.6, r * 0.55, 6, col, seed);
        p.cone(tip, r * 0.55, r * 0.7, 6, col, seed, 0.0);
    }
}

/// Palm: a curving trunk topped with drooping fronds.
fn palm(p: &mut Plants, at: Vec3, height: f32, seed: f32) {
    p.start(at.y, 0.04);
    let a = seed * 6.0;
    let lean = vec3(a.cos(), 0.0, a.sin());
    let segments = if p.far { 2 } else { 5 };
    let mut prev = at - Vec3::Y * 0.5;
    let bark = vary(PALM_BARK, seed, 0.1);
    for k in 1..=segments {
        let f = k as f32 / segments as f32;
        let next = at + Vec3::Y * height * f + lean * height * 0.25 * f * f;
        let r = height * 0.04 * (1.0 - 0.35 * f);
        let col = if k % 2 == 0 { scale(bark, 0.85) } else { bark };
        p.limb(prev, next, r * 1.1, r, 6, col, seed);
        prev = next;
    }
    let crown = prev;
    let fronds = 8;
    let leaf = vary(PALM_LEAF, seed, 0.1);
    for k in 0..fronds {
        let a = seed + k as f32 * std::f32::consts::TAU / fronds as f32;
        let out = vec3(a.cos(), 0.0, a.sin());
        let side = vec3(-a.sin(), 0.0, a.cos()) * height * 0.07;
        let len = height * (0.42 + 0.1 * hash(seed, k as f32));
        let mid = crown + out * len * 0.5 + Vec3::Y * height * 0.06;
        let tip = crown + out * len - Vec3::Y * height * 0.18;
        let col = vary(leaf, seed + k as f32, 0.08);
        // Two panels per frond, bending down at the middle
        p.tri(crown, mid + side, mid - side, col);
        p.tri(mid - side, mid + side, tip, col);
        if !p.far {
            p.tri(crown, mid - side, mid + side, scale(col, 0.8));
            p.tri(mid + side, mid - side, tip, scale(col, 0.8));
        }
    }
    // Coconuts
    for k in 0..if p.far { 0 } else { 3 } {
        let a = seed * 2.0 + k as f32 * 2.1;
        p.blob(crown + vec3(a.cos(), -0.8, a.sin()) * height * 0.04, height * 0.035, 1.0, [0.35, 0.25, 0.12], seed + k as f32);
    }
}

/// Dead tree: grey bark, a trunk splitting into crooked bare limbs.
fn snag(p: &mut Plants, at: Vec3, height: f32, seed: f32) {
    p.start(at.y, 0.02);
    let bark = vary(BARK_GREY, seed, 0.1);
    let fork = at + Vec3::Y * height * 0.5 + vec3(hash(seed, 1.0) - 0.5, 0.0, hash(seed, 2.0) - 0.5) * height * 0.1;
    p.limb(at - Vec3::Y * 0.5, fork, height * 0.05, height * 0.035, 5, bark, seed);
    for k in 0..if p.far { 2 } else { 4 } {
        let a = seed * 3.0 + k as f32 * 1.7;
        let out = vec3(a.cos(), 0.0, a.sin());
        let mid = fork + out * height * 0.15 + Vec3::Y * height * (0.15 + 0.1 * hash(seed, k as f32));
        let tip = mid + out * height * 0.12 + Vec3::Y * height * (0.12 + 0.15 * hash(k as f32, seed));
        p.limb(fork, mid, height * 0.025, height * 0.015, 4, bark, seed);
        p.limb(mid, tip, height * 0.015, 0.0, 4, bark, seed);
    }
}

// ── Placement ──

/// What grows at a spot.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Species {
    Pine,
    Spruce,
    Broadleaf,
    Birch,
    Bush,
    Cactus,
    Palm,
    Snag,
}

/// Plants per cell (0..1) at a spot `ground` high.
pub fn density(x: f32, z: f32, ground: f32) -> f32 {
    let desert = terrain::biome_desert(x, z);
    let mountain = terrain::biome_mountain(x, z);
    let forest = terrain::biome_forest(x, z);
    let tundra = terrain::biome_tundra(x, z);
    let grass = (1.0 - desert - mountain - forest - tundra).max(0.0);

    // Clearings and groves: patches of open ground and thicker woods
    let patch = noise(x, z, 420.0);
    let grove = noise(x + 900.0, z - 300.0, 260.0);
    let treeline = 125.0 + 35.0 * noise(x, z, 800.0);
    let near_water = terrain::raw_height(x, z) < WATER_LEVEL + 6.0;

    let mut density = forest * (0.15 + 0.85 * ((patch - 0.25) * 2.0).clamp(0.0, 1.0))
        + grass * if grove > 0.62 { 0.45 } else { 0.035 }
        + mountain * 0.3 * ((treeline - ground) / 30.0).clamp(0.0, 1.0)
        + tundra * 0.04
        + desert * if near_water { 0.3 } else { 0.03 };
    // Nothing grows above the treeline or on the volcano's ash
    let to_volcano = Vec2::new(x - terrain::VOLCANO.0, z - terrain::VOLCANO.1).length();
    if ground > treeline + 20.0 || to_volcano < 480.0 {
        density = 0.0;
    }
    density
}

/// Which species, if any, for a spot `ground` high.
fn choose(x: f32, z: f32, ground: f32, roll: f32) -> Option<(Species, f32)> {
    if roll > density(x, z, ground) {
        return None;
    }
    let desert = terrain::biome_desert(x, z);
    let mountain = terrain::biome_mountain(x, z);
    let forest = terrain::biome_forest(x, z);
    let tundra = terrain::biome_tundra(x, z);
    let grass = (1.0 - desert - mountain - forest - tundra).max(0.0);
    let near_water = terrain::raw_height(x, z) < WATER_LEVEL + 6.0;

    // Which biome this plant belongs to: a draw weighted by the local mix,
    // so species thin out gradually across a biome edge
    let wooded = (forest + mountain).min(1.0);
    let total = desert + tundra + wooded + grass;
    let mut draw = hash(x * 0.91, z * 0.27) * total;
    let mut take = |w: f32| { draw -= w; draw < 0.0 };
    let pick = hash(x * 0.37, z * 0.53);
    let altitude = ((ground - 40.0) / 80.0).clamp(0.0, 1.0);
    let species = if take(desert) {
        if near_water { if pick < 0.7 { Species::Palm } else { Species::Bush } }
        else if pick < 0.65 { Species::Cactus } else if pick < 0.8 { Species::Snag } else { Species::Bush }
    } else if take(tundra) {
        if pick < 0.65 { Species::Pine } else if pick < 0.85 { Species::Snag } else { Species::Bush }
    } else if take(wooded) {
        let conifer = 0.55 + 0.4 * altitude + 0.2 * mountain;
        if pick < conifer * 0.6 { Species::Spruce } else if pick < conifer { Species::Pine }
        else if pick < conifer + 0.12 { Species::Birch } else if pick < 0.97 { Species::Broadleaf } else { Species::Snag }
    } else if pick < 0.5 {
        Species::Broadleaf
    } else if pick < 0.65 {
        Species::Birch
    } else if pick < 0.82 {
        Species::Pine
    } else {
        Species::Bush
    };
    Some((species, desert.max(tundra)))
}

/// The plants of a region: a full-detail mesh, a lighter one for distant
/// views, and trunk positions for collisions.
pub struct Trees {
    pub near: (Vec<Vertex>, Vec<u32>),
    pub far: (Vec<Vertex>, Vec<u32>),
    pub trunks: Vec<[f32; 2]>,
}

/// Generate the plants whose placement cell starts inside the rectangle
/// [x0, x1) x [z0, z1). Cells are on a global grid, so splitting the map into
/// regions gives the same forest.
pub fn generate_trees(x0: f32, z0: f32, x1: f32, z1: f32) -> Trees {
    let (mut near, mut far) = (Plants::new(false), Plants::new(true));
    let mut trunks = Vec::new();
    let half = TERRAIN_HALF_SIZE;
    let first = |v: f32| ((v + half) / CELL).ceil() as i32;
    let nearby_landmarks: Vec<_> = landmarks::all().iter()
        .filter(|l| l.pos.x > x0 - 200.0 && l.pos.x < x1 + 200.0 && l.pos.y > z0 - 200.0 && l.pos.y < z1 + 200.0)
        .collect();

    for cz in first(z0)..first(z1) {
        for cx in first(x0)..first(x1) {
            let bx = -half + cx as f32 * CELL;
            let bz = -half + cz as f32 * CELL;
            let x = bx + (0.1 + 0.8 * hash(bx, bz)) * CELL;
            let z = bz + (0.1 + 0.8 * hash(bz + 17.0, bx - 5.0)) * CELL;
            let seed = hash(x * 0.71, z * 1.33) * 100.0;

            let raw = terrain::raw_height(x, z);
            if raw < WATER_LEVEL + 1.0 || is_near_road(x, z) || crate::farms::occupied(Vec2::new(x, z), 6.0) {
                continue;
            }
            // Not on cliffs
            let e = 4.0;
            let slope = Vec2::new(
                terrain::raw_height(x + e, z) - terrain::raw_height(x - e, z),
                terrain::raw_height(x, z + e) - terrain::raw_height(x, z - e),
            ).length() / (2.0 * e);
            if slope > 0.7 {
                continue;
            }
            let clear_of = |l: &&landmarks::Landmark| l.pos.distance(Vec2::new(x, z)) < if l.kind == Kind::Village { 150.0 } else { 40.0 };
            if nearby_landmarks.iter().any(clear_of) {
                continue;
            }
            let ground = terrain_height(x, z);
            let Some((species, dry)) = choose(x, z, ground, hash(x * 1.9, z * 0.7)) else { continue };

            let at = vec3(x, ground, z);
            let size = 0.75 + 0.5 * hash(seed, 0.5);
            let snow = if species == Species::Pine || species == Species::Spruce {
                (terrain::biome_tundra(x, z) * 1.2 + ((ground - 110.0) / 40.0).clamp(0.0, 1.0)).min(1.0)
            } else {
                0.0
            };
            // Autumn comes to patches of the broadleaf woods
            let autumn = noise(x - 400.0, z + 700.0, 600.0) > 0.66 && terrain::biome_desert(x, z) < 0.1;
            let leaves = if autumn { AUTUMN[(seed as usize) % 3] } else { LEAF };
            let build = |p: &mut Plants| match species {
                Species::Pine => conifer(p, at, 20.0 * size, 1.0, PINE, snow, seed),
                Species::Spruce => conifer(p, at, 26.0 * size, 0.7, SPRUCE, snow, seed),
                Species::Broadleaf => broadleaf(p, at, 17.0 * size, leaves, seed),
                Species::Birch => {
                    let leaves = if autumn { mix(BIRCH_LEAF, AUTUMN[1], 0.8) } else { BIRCH_LEAF };
                    birch(p, at, 18.0 * size, leaves, seed)
                }
                Species::Bush => bush(p, at, 3.0 * size, mix(mix(LEAF, [0.15, 0.35, 0.12], 0.4), DRY_SHRUB, dry), seed),
                Species::Cactus => cactus(p, at, 11.0 * size, seed),
                Species::Palm => palm(p, at, 19.0 * size, seed),
                Species::Snag => snag(p, at, 13.0 * size, seed),
            };
            build(&mut near);
            build(&mut far);
            if species != Species::Bush {
                trunks.push([x, z]);
            }
        }
    }

    hedgerows(&mut near, &mut far, &mut trunks, Vec2::new(x0, z0), Vec2::new(x1, z1));
    Trees { near: (near.verts, near.idxs), far: (far.verts, far.idxs), trunks }
}

/// Farm hedgerows whose bushes fall in [lo, hi): a dense line of shrubs,
/// with an oak standing out of it here and there.
fn hedgerows(near: &mut Plants, far: &mut Plants, trunks: &mut Vec<[f32; 2]>, lo: Vec2, hi: Vec2) {
    const HEDGE: Rgb = [0.18, 0.4, 0.13];
    let inside = |p: Vec2| p.x >= lo.x && p.y >= lo.y && p.x < hi.x && p.y < hi.y;
    for farm in crate::farms::all() {
        if farm.yard.x < lo.x - 400.0 || farm.yard.x > hi.x + 400.0 || farm.yard.y < lo.y - 400.0 || farm.yard.y > hi.y + 400.0 {
            continue;
        }
        for &(a, b) in &farm.hedges {
            let steps = (a.distance(b) / 4.0).ceil() as usize;
            for k in 0..=steps {
                let p = a.lerp(b, k as f32 / steps.max(1) as f32);
                if !inside(p) || is_near_road(p.x, p.y) {
                    continue;
                }
                let seed = hash(p.x * 0.31, p.y * 0.77) * 100.0;
                let at = vec3(p.x, terrain_height(p.x, p.y), p.y);
                if hash(seed, 4.0) < 0.035 {
                    let size = 0.8 + 0.4 * hash(seed, 5.0);
                    broadleaf(near, at, 16.0 * size, LEAF, seed);
                    broadleaf(far, at, 16.0 * size, LEAF, seed);
                    trunks.push([p.x, p.y]);
                } else {
                    let size = 2.6 + 1.0 * hash(seed, 6.0);
                    let leaves = vary(HEDGE, seed, 0.1);
                    bush(near, at, size, leaves, seed);
                    bush(far, at, size, leaves, seed);
                }
            }
        }
    }
}

/// Keep plants off the road (or trail) and its shoulder.
fn is_near_road(x: f32, z: f32) -> bool {
    road::network()
        .and_then(|n| n.nearest(x, z))
        .is_some_and(|s| s.dist < s.half_width() + s.shoulder() + 6.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dev aid: print a spot (on land, away from roads) where each species grows.
    #[test]
    #[ignore]
    fn find_species() {
        let _ = crate::sim_tests::network();
        let mut found: Vec<(Species, bool)> = Vec::new();
        let mut z = -8000.0;
        while z < 8000.0 {
            let mut x = -8000.0;
            while x < 8000.0 {
                let g = terrain_height(x, z);
                if let Some((s, _)) = choose(x, z, g, 0.0) {
                    let autumn = noise(x - 400.0, z + 700.0, 600.0) > 0.66 && s == Species::Broadleaf;
                    if !found.contains(&(s, autumn)) && !is_near_road(x, z) {
                        found.push((s, autumn));
                        println!("{s:?} autumn={autumn} at {x},{z} ground {g:.0}");
                    }
                }
                x += 97.0;
            }
            z += 89.0;
        }
    }
}
