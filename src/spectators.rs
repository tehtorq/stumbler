use glam::Vec2;
use crate::geometry::{Vertex, push_box};
use crate::road;
use crate::terrain::terrain_height;

pub struct Spectator {
    pub x: f32,
    pub z: f32,
    /// Ground height where they stand (worked out once).
    ground: f32,
    pub color: [f32; 4],
    pub phase: f32,
}

pub fn create_spectators(circuit_points: &[Vec<Vec2>]) -> Vec<Spectator> {
    let mut specs = Vec::new();
    let shirt_colors: [[f32; 4]; 6] = [
        [0.9, 0.2, 0.2, 1.0],
        [0.2, 0.5, 0.9, 1.0],
        [0.9, 0.9, 0.2, 1.0],
        [0.2, 0.9, 0.3, 1.0],
        [0.9, 0.5, 0.1, 1.0],
        [0.8, 0.2, 0.8, 1.0],
    ];

    for (ci, pts) in circuit_points.iter().enumerate() {
        if pts.len() < 2 { continue; }
        let p0 = pts[0];
        let p1 = pts[1];
        let dx = p1.x - p0.x;
        let dz = p1.y - p0.y;
        let len = (dx * dx + dz * dz).sqrt().max(0.1);
        let perp_x = -dz / len;
        let perp_z = dx / len;
        let fwd_x = dx / len;
        let fwd_z = dz / len;

        for side in &[-1.0_f32, 1.0] {
            for i in 0..3 {
                let along = (i as f32 - 1.0) * 5.0;
                let lateral = road::ROAD_HALF_WIDTH + road::KERB_WIDTH + 9.0 + (ci as f32 * 3.7 + i as f32 * 5.1).sin().abs() * 3.0;
                let sx = p0.x + fwd_x * along + perp_x * lateral * side;
                let sz = p0.y + fwd_z * along + perp_z * lateral * side;
                let color_idx = (ci * 3 + i) % shirt_colors.len();
                specs.push(Spectator {
                    x: sx,
                    z: sz,
                    ground: terrain_height(sx, sz),
                    color: shirt_colors[color_idx],
                    phase: ci as f32 * 2.1 + i as f32 * 1.3 + side * 0.7,
                });
            }
        }
    }

    specs
}

/// Mesh of the spectators within `range` of the camera.
pub fn build_spectator_mesh(spectators: &[Spectator], camera: glam::Vec3, range: f32, time: f32) -> (Vec<Vertex>, Vec<u16>) {
    let mut verts = Vec::new();
    let mut idxs = Vec::new();
    let skin: [f32; 4] = [0.85, 0.7, 0.55, 1.0];

    for spec in spectators {
        let (dx, dz) = (spec.x - camera.x, spec.z - camera.z);
        if dx * dx + dz * dz > range * range {
            continue;
        }
        let ground = spec.ground;
        let bounce = (time * 3.5 + spec.phase).sin().abs() * 0.4;
        let y = ground + bounce;

        // Body
        push_box(&mut verts, &mut idxs,
            spec.x, y + 1.0, spec.z,
            0.35, 0.6, 0.2, [spec.color; 6]);

        // Head
        push_box(&mut verts, &mut idxs,
            spec.x, y + 1.9, spec.z,
            0.2, 0.2, 0.2, [skin; 6]);

        // Waving arm
        let wave = (time * 4.0 + spec.phase).sin() * 0.4;
        push_box(&mut verts, &mut idxs,
            spec.x + 0.5, y + 1.4 + wave, spec.z,
            0.1, 0.3, 0.1, [spec.color; 6]);
    }

    (verts, idxs)
}
