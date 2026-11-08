//! Market day: some days a town fills its square with stalls for the
//! morning, each under a striped awning, trestles piled with crates of
//! fruit, vegetables, flowers or bread, a seller behind each; and the
//! square busy with shoppers carrying bags (see `townsfolk`).

use glam::{Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::props::{hash, PERSON_SCALE};
use crate::towns::{self, Town, FRONT, SQUARE_DEPTH, SQUARE_HALF};
use crate::townsfolk::{Mesh, Pose};

/// Markets are drawn within this distance of the camera.
const DRAW: f32 = 500.0;

/// Whether town `t` holds its market on `day` at `clock` (0..1 from midnight).
pub fn market_on(t: usize, day: u32, clock: f32) -> bool {
    let town = &towns::all()[t];
    let h = clock * 24.0;
    hash(town.seed, day as f32 * 3.7 + 11.0) < 0.4 && (8.0..15.0).contains(&h)
}

/// Where the stalls stand in the square (along, out in the town's frame)
/// and which way they face (toward the shoppers).
pub fn stalls(town: &Town) -> Vec<(Vec2, Vec2)> {
    let back = FRONT + SQUARE_DEPTH - 6.0;
    let side = town.square;
    let mut list = Vec::new();
    for s in [-18.0, -6.0, 6.0, 18.0] {
        list.push((vec2(s, side * back), vec2(0.0, -side)));
    }
    for s in [-1.0, 1.0] {
        list.push((vec2(s * (SQUARE_HALF - 4.0), side * (FRONT + SQUARE_DEPTH * 0.5)), vec2(-s, 0.0)));
    }
    list
}

/// The stalls and their sellers in the towns near the camera holding market.
pub fn build_mesh(camera: Vec3, day: u32, clock: f32) -> (Vec<Vertex>, Vec<u16>) {
    let cam = vec2(camera.x, camera.z);
    let mut m = Mesh::default();
    for (t, town) in towns::all().iter().enumerate() {
        if town.centre.distance(cam) > DRAW || !market_on(t, day, clock) {
            continue;
        }
        for (k, (l, face)) in stalls(town).into_iter().enumerate() {
            let p = town.world(l.x, l.y);
            let ahead = town.world(l.x + face.x, l.y + face.y);
            let f2 = (ahead - p).normalize_or_zero();
            let y = crate::terrain::terrain_height(p.x, p.y) + towns::PAVE;
            stall(&mut m, vec3(p.x, y, p.y), f2, town.seed + k as f32 * 13.0);
        }
    }
    (m.verts, m.idxs)
}

/// One stall: a trestle loaded with crates, a striped awning on poles, and
/// the seller standing behind, facing `facing`.
fn stall(m: &mut Mesh, at: Vec3, facing: Vec2, seed: f32) {
    let k = PERSON_SCALE;
    let f = vec3(facing.x, 0.0, facing.y);
    let r = vec3(facing.y, 0.0, -facing.x);
    let p = |a: f32, up: f32, c: f32| at + (f * a + Vec3::Y * up + r * c) * k;
    let wood = [0.55, 0.4, 0.25];
    const STRIPES: [[f32; 3]; 4] = [[0.8, 0.15, 0.12], [0.15, 0.4, 0.3], [0.2, 0.3, 0.65], [0.85, 0.6, 0.1]];
    let stripe = STRIPES[(hash(seed, 1.0) * 4.0) as usize % 4];
    // The trestle table
    m.box_at(p(0.4, 0.85, 0.0), r, f, vec3(1.6, 0.04, 0.5) * k, wood, [0.95, 0.95, 0.9]);
    for (a, c) in [(0.0, -1.5), (0.0, 1.5), (0.8, -1.5), (0.8, 1.5)] {
        m.limb(p(a, 0.0, c), p(a, 0.83, c), 0.03 * k, wood);
    }
    // Crates of produce on it: fruit, veg, flowers or loaves
    const GOODS: [[f32; 3]; 7] = [[0.85, 0.1, 0.08], [0.95, 0.55, 0.1], [0.4, 0.7, 0.2], [0.95, 0.85, 0.2], [0.55, 0.2, 0.5], [0.75, 0.55, 0.3], [0.95, 0.5, 0.6]];
    for c in 0..4 {
        let across = -1.2 + c as f32 * 0.8;
        let crate_at = p(0.4, 0.89, across);
        m.box_at(crate_at + Vec3::Y * 0.1 * k, r, f, vec3(0.35, 0.1, 0.3) * k, [0.7, 0.55, 0.35], [0.7, 0.55, 0.35]);
        let goods = GOODS[(hash(seed, 2.0 + c as f32) * 7.0) as usize % 7];
        m.box_at(crate_at + Vec3::Y * 0.24 * k, r, f, vec3(0.32, 0.06, 0.27) * k, goods, goods);
    }
    // The awning on its four poles, striped
    for (a, c) in [(-0.3, -1.8), (-0.3, 1.8), (1.1, -1.8), (1.1, 1.8)] {
        m.limb(p(a, 0.0, c), p(a, if a < 0.0 { 2.4 } else { 2.1 }, c), 0.035 * k, [0.4, 0.4, 0.42]);
    }
    let bands = 8;
    for b in 0..bands {
        let (c0, c1) = (-1.9 + 3.8 * b as f32 / bands as f32, -1.9 + 3.8 * (b + 1) as f32 / bands as f32);
        let col = if b % 2 == 0 { stripe } else { [0.95, 0.95, 0.92] };
        m.quad(p(-0.4, 2.45, c0), p(-0.4, 2.45, c1), p(1.25, 2.1, c1), p(1.25, 2.1, c0), col);
        m.quad(p(1.25, 2.1, c0), p(1.25, 2.1, c1), p(1.25, 1.9, c1), p(1.25, 1.9, c0), col);
    }
    // The seller, behind the table
    m.figure(p(-0.4, 0.0, 0.3 * (hash(seed, 9.0) - 0.5)), facing, 0.0, Pose::Standing, seed + 5.0);
}
