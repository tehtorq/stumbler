use glam::{Vec2, Vec3, vec3};
use crate::geometry::Vertex;
use crate::terrain::terrain_height;

pub struct Bird {
    pos: Vec3,
    vel: Vec3,
    phase: f32,
}

pub struct Flock {
    center: Vec3,
    radius: f32,
    pub birds: Vec<Bird>,
    scatter_timer: f32,
    /// Seconds left of gliding back into the circle after a scatter.
    returning: f32,
    angular_speed: f32,
}

/// How long scattered birds take to find their places in the circle again.
const RETURN_TIME: f32 = 3.0;

pub fn create_flocks(circuit_centroids: &[(f32, f32)]) -> Vec<Flock> {
    circuit_centroids.iter().enumerate().map(|(i, &(cx, cz))| {
        let ground_y = terrain_height(cx, cz);
        let altitude = 40.0 + (i as f32 * 7.3) % 20.0;
        let radius = 25.0 + (i as f32 * 11.1) % 20.0;
        let angular_speed = 0.4 + (i as f32 * 3.7) % 0.4;

        let num_birds = 4 + (i % 3);
        let birds = (0..num_birds).map(|j| {
            let angle = j as f32 / num_birds as f32 * std::f32::consts::TAU;
            Bird {
                pos: vec3(
                    cx + angle.cos() * radius,
                    ground_y + altitude,
                    cz + angle.sin() * radius,
                ),
                vel: Vec3::ZERO,
                phase: i as f32 * 2.1 + j as f32 * 1.3,
            }
        }).collect();

        Flock {
            center: vec3(cx, ground_y + altitude, cz),
            radius,
            birds,
            scatter_timer: 0.0,
            returning: 0.0,
            angular_speed,
        }
    }).collect()
}

impl Flock {
    /// Send every bird flying off outward and up.
    fn scatter(&mut self) {
        self.scatter_timer = 4.0;
        self.returning = 0.0;
        for bird in &mut self.birds {
            let bx = bird.pos.x - self.center.x;
            let bz = bird.pos.z - self.center.z;
            let len = (bx * bx + bz * bz).sqrt().max(1.0);
            bird.vel = vec3(
                bx / len * 25.0 + (bird.phase * 3.1).sin() * 8.0,
                12.0 + (bird.phase * 5.7).sin().abs() * 8.0,
                bz / len * 25.0 + (bird.phase * 7.3).sin() * 8.0,
            );
        }
    }
}

/// A sudden din at `at` (a field of engines at full revs) puts up every
/// flock within `reach`.
pub fn startle(flocks: &mut [Flock], at: Vec3, reach: f32) {
    for flock in flocks.iter_mut().filter(|f| f.scatter_timer <= 0.0) {
        if Vec2::new(flock.center.x - at.x, flock.center.z - at.z).length() < reach {
            flock.scatter();
        }
    }
}

pub fn update_flocks(flocks: &mut [Flock], player_pos: Vec3, dt: f32, time: f32) {
    for flock in flocks.iter_mut() {
        let dx = player_pos.x - flock.center.x;
        let dz = player_pos.z - flock.center.z;
        let dist_sq = dx * dx + dz * dz;

        // Scatter when player drives close
        if dist_sq < 80.0 * 80.0 && flock.scatter_timer <= 0.0 {
            flock.scatter();
        }

        if flock.scatter_timer > 0.0 {
            flock.scatter_timer -= dt;
            for bird in &mut flock.birds {
                bird.pos += bird.vel * dt;
                bird.vel.y -= 4.0 * dt;
                bird.vel *= 1.0 - 0.4 * dt;
            }
            // Then glide back to the circle rather than jump into it
            if flock.scatter_timer <= 0.0 {
                flock.returning = RETURN_TIME;
                for bird in &mut flock.birds {
                    bird.vel = Vec3::ZERO;
                }
            }
        } else {
            // Orbit around center with gentle bob, easing in more and more
            // firmly while they find their places again
            let pull = if flock.returning > 0.0 {
                flock.returning -= dt;
                let settled = 1.0 - flock.returning.max(0.0) / RETURN_TIME;
                (dt * (0.6 + 7.0 * settled * settled)).min(1.0)
            } else {
                1.0
            };
            let n = flock.birds.len();
            for (j, bird) in flock.birds.iter_mut().enumerate() {
                let angle = time * flock.angular_speed
                    + j as f32 / n as f32 * std::f32::consts::TAU;
                let bob = (time * 2.0 + bird.phase).sin() * 1.5;
                let place = vec3(
                    flock.center.x + angle.cos() * flock.radius,
                    flock.center.y + bob,
                    flock.center.z + angle.sin() * flock.radius,
                );
                bird.pos += (place - bird.pos) * pull;
            }
        }
    }
}

/// Build bird triangles for rendering. Each bird is a V-shape (3 verts, double-sided).
pub fn build_bird_mesh(flocks: &[Flock], time: f32) -> (Vec<Vertex>, Vec<u16>) {
    let mut verts = Vec::new();
    let mut idxs = Vec::new();
    let color: [f32; 4] = [0.08, 0.08, 0.12, 1.0];

    for flock in flocks {
        let n = flock.birds.len();
        for (j, bird) in flock.birds.iter().enumerate() {
            // Flight direction
            let (dx, dz) = if flock.scatter_timer > 0.0 && bird.vel.length_squared() > 1.0 {
                (bird.vel.x, bird.vel.z)
            } else {
                let angle = time * flock.angular_speed
                    + j as f32 / n as f32 * std::f32::consts::TAU;
                (-angle.sin(), angle.cos())
            };
            let len = (dx * dx + dz * dz).sqrt().max(0.01);
            let fwd_x = dx / len;
            let fwd_z = dz / len;
            let wing_x = -fwd_z;
            let wing_z = fwd_x;

            let flap = (time * 8.0 + bird.phase * 3.0).sin() * 0.6;

            let base = verts.len() as u16;
            // Beak
            verts.push(Vertex {
                pos: [bird.pos.x + fwd_x * 0.8, bird.pos.y, bird.pos.z + fwd_z * 0.8],
                color,
            });
            // Left wing tip
            verts.push(Vertex {
                pos: [
                    bird.pos.x - fwd_x * 0.3 + wing_x * 1.2,
                    bird.pos.y + flap,
                    bird.pos.z - fwd_z * 0.3 + wing_z * 1.2,
                ],
                color,
            });
            // Right wing tip
            verts.push(Vertex {
                pos: [
                    bird.pos.x - fwd_x * 0.3 - wing_x * 1.2,
                    bird.pos.y - flap,
                    bird.pos.z - fwd_z * 0.3 - wing_z * 1.2,
                ],
                color,
            });
            // Double-sided
            idxs.extend_from_slice(&[base, base + 1, base + 2]);
            idxs.extend_from_slice(&[base, base + 2, base + 1]);
        }
    }

    (verts, idxs)
}
