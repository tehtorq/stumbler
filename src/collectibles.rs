use crate::geometry::Vertex;
use crate::terrain::{terrain_height, TERRAIN_HALF_SIZE, WORLD_SCALE};

pub struct Collectible {
    pub x: f32,
    pub z: f32,
    /// Ground height under the orb (worked out once; it doesn't move).
    ground: f32,
    pub collected: bool,
}

/// Orbs further than this from the camera are too small to see.
const DRAW_RANGE: f32 = 1200.0;

/// Generate collectible positions spread across the world.
/// Avoids track areas. Places them at interesting/hard-to-reach spots.
pub fn create_collectibles() -> Vec<Collectible> {
    let mut items = Vec::new();
    let half = TERRAIN_HALF_SIZE;

    // Grid of candidates with pseudo-random jitter, sparser in a bigger
    // world so there are four times as many orbs for sixteen times the land
    let spacing = 650.0 * WORLD_SCALE / 2.0;
    let cells = (half * 2.0 / spacing) as i32;

    for cz in 0..cells {
        for cx in 0..cells {
            let bx = -half + cx as f32 * spacing;
            let bz = -half + cz as f32 * spacing;

            // Hash to determine if this cell gets a collectible (~15% chance).
            // Hashed on the cell's index: hashing its coordinates loses all
            // precision far from the centre
            let (i, j) = (cx as f32, cz as f32);
            let h = hash(i * 0.31 + 50.0, j * 0.37 + 90.0);
            if h > 0.15 { continue; }

            // Jitter within cell
            let x = bx + hash(i, j) * spacing * 0.8 + spacing * 0.1;
            let z = bz + hash(i + 7.0, j + 3.0) * spacing * 0.8 + spacing * 0.1;

            let y = terrain_height(x, z);
            if y < -8.0 { continue; } // skip water

            items.push(Collectible { x, z, ground: y, collected: false });
        }
    }

    items
}

/// Build mesh for all uncollected orbs. Call each frame (or when collection state changes).
/// Each orb is an octahedron (6 verts, 8 triangles) floating above the ground.
pub fn build_collectible_mesh(items: &[Collectible], camera: glam::Vec3, time: f32) -> (Vec<Vertex>, Vec<u16>) {
    let mut verts = Vec::new();
    let mut idxs = Vec::new();

    let orb_color = [1.0, 0.85, 0.2, 1.0]; // golden
    let orb_glow = [1.0, 0.95, 0.5, 1.0];  // bright center

    for item in items {
        let (dx, dz) = (item.x - camera.x, item.z - camera.z);
        if item.collected || dx * dx + dz * dz > DRAW_RANGE * DRAW_RANGE { continue; }

        let y = item.ground + 4.0 + (time * 2.0 + item.x * 0.1).sin() * 1.0;
        let r = 1.5; // radius

        let base = verts.len() as u16;
        // Octahedron: 6 vertices (top, bottom, 4 equator)
        verts.push(Vertex { pos: [item.x, y + r, item.z], color: orb_glow });     // 0: top
        verts.push(Vertex { pos: [item.x, y - r, item.z], color: orb_glow });     // 1: bottom
        verts.push(Vertex { pos: [item.x + r, y, item.z], color: orb_color });    // 2: +x
        verts.push(Vertex { pos: [item.x, y, item.z + r], color: orb_color });    // 3: +z
        verts.push(Vertex { pos: [item.x - r, y, item.z], color: orb_color });    // 4: -x
        verts.push(Vertex { pos: [item.x, y, item.z - r], color: orb_color });    // 5: -z

        // 8 faces (CW from outside for LH projection)
        let faces: [[u16; 3]; 8] = [
            [0, 2, 3], [0, 3, 4], [0, 4, 5], [0, 5, 2], // top
            [1, 3, 2], [1, 4, 3], [1, 5, 4], [1, 2, 5], // bottom
        ];
        for f in &faces {
            idxs.extend_from_slice(&[base + f[0], base + f[1], base + f[2]]);
        }
    }

    (verts, idxs)
}

fn hash(x: f32, y: f32) -> f32 {
    let kx = 0.3183099_f32;
    let ky = 0.3678794_f32;
    let px = x * kx + ky;
    let py = y * ky + kx;
    (16.0 * kx * (px * py * (px + py)).fract()).fract()
}
