//! Smoke, dust, exhaust and ambient particles, drawn as low-poly hexagons.

use glam::{Vec3, vec3};

use crate::car::WHEEL_MOUNTS;
use crate::geometry::Vertex;
use crate::physics::CarPhysics;
use crate::terrain::terrain_height;

pub const MAX_PARTICLES: usize = 1500;
const HEX_SIDES: usize = 6;

struct Particle {
    pos: Vec3,
    vel: Vec3,
    /// 1.0 = new, 0.0 = dead
    life: f32,
    /// Life lost per second.
    decay: f32,
    size: f32,
    growth: f32,
    color: [f32; 3],
    alpha: f32,
    /// Downward pull (units/s²): smoke barely falls, gravel drops.
    gravity: f32,
    /// Air resistance (share of speed lost per second).
    drag: f32,
}

pub struct Particles {
    list: Vec<Particle>,
    rng: u32,
    /// Accumulators so spawn rates are frame-rate independent.
    smoke_accum: [f32; 4],
    dust_accum: f32,
    exhaust_accum: f32,
    rubble_accum: [f32; 2],
    spray_accum: f32,
    ambient_accum: f32,
    place_accum: f32,
}

impl Particles {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            rng: 0x9e37_79b9,
            smoke_accum: [0.0; 4],
            dust_accum: 0.0,
            exhaust_accum: 0.0,
            rubble_accum: [0.0; 2],
            spray_accum: 0.0,
            ambient_accum: 0.0,
            place_accum: 0.0,
        }
    }

    /// xorshift32 in [0, 1)
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn spread(&mut self, amount: f32) -> f32 {
        (self.rand() * 2.0 - 1.0) * amount
    }

    fn push(&mut self, p: Particle) {
        if self.list.len() < MAX_PARTICLES {
            self.list.push(p);
        }
    }

    /// Spawn particles from the car: smoke from sliding tyres, dust off-road,
    /// exhaust puffs when accelerating hard.
    pub fn emit_from_car(&mut self, car: &CarPhysics, on_road: f32, throttle: f32, dt: f32) {
        let speed = car.speed.abs();
        for i in 0..4 {
            if !car.wheel_contact[i] {
                continue;
            }
            let mount = car.pos + car.rot * Vec3::from(WHEEL_MOUNTS[i]);
            let ground = terrain_height(mount.x, mount.z);
            let at = vec3(mount.x, ground + 0.4, mount.z);

            // Tyre smoke: only while the tyre is actually sliding
            let slip = car.wheel_slip[i];
            if car.wheel_sliding[i] && slip > 6.0 && on_road > 0.5 {
                self.smoke_accum[i] += dt * (slip * 2.5).min(40.0);
                while self.smoke_accum[i] >= 1.0 {
                    self.smoke_accum[i] -= 1.0;
                    let vel = vec3(self.spread(2.0), 2.0 + self.rand() * 2.0, self.spread(2.0)) + car.vel * 0.1;
                    let grey = 0.78 + self.rand() * 0.12;
                    self.push(Particle { gravity: 1.0, drag: 1.5,
                        pos: at, vel, life: 1.0, decay: 0.6, size: 1.0, growth: 2.2,
                        color: [grey, grey, grey * 1.02], alpha: 0.45,
                    });
                }
            }
        }

        // Dust kicked up off the tarmac: gravel, dirt and grass
        if on_road < 0.6 && speed > 15.0 && car.grounded_wheels() > 0 {
            self.dust_accum += dt * (speed * 0.25).min(30.0);
            while self.dust_accum >= 1.0 {
                self.dust_accum -= 1.0;
                let wheel = 2 + (self.rand() * 2.0) as usize; // rear wheels
                let mount = car.pos + car.rot * Vec3::from(WHEEL_MOUNTS[wheel]);
                let ground = terrain_height(mount.x, mount.z);
                let vel = vec3(self.spread(3.0), 1.5 + self.rand() * 2.5, self.spread(3.0)) - car.vel * 0.05;
                let shade = 0.85 + self.rand() * 0.15;
                self.push(Particle { gravity: 1.0, drag: 1.5,
                    pos: vec3(mount.x, ground + 0.3, mount.z), vel, life: 1.0, decay: 0.8, size: 0.9, growth: 1.8,
                    color: [0.62 * shade, 0.52 * shade, 0.38 * shade], alpha: 0.5,
                });
            }
        }

        // Occasional exhaust puffs under hard acceleration
        if throttle > 0.8 && speed > 5.0 && speed < 90.0 {
            self.exhaust_accum += dt * 6.0;
            while self.exhaust_accum >= 1.0 {
                self.exhaust_accum -= 1.0;
                let at = car.pos + car.rot * vec3(0.8, -0.9, 4.4);
                let vel = car.rot * vec3(self.spread(0.5), 0.8, 3.0) + car.vel * 0.6;
                self.push(Particle { gravity: 1.0, drag: 1.5,
                    pos: at, vel, life: 1.0, decay: 1.6, size: 0.4, growth: 1.0,
                    color: [0.45, 0.45, 0.48], alpha: 0.35,
                });
            }
        }
    }

    /// Clods, grit and stones thrown up behind the rear wheels when they
    /// run off the road at speed, coloured like the ground they came from.
    pub fn rubble(&mut self, car: &CarPhysics, on_road: impl Fn(Vec3) -> f32, ground_color: impl Fn(Vec3) -> [f32; 3], dt: f32) {
        let speed = car.vel.length();
        if speed < 15.0 {
            return;
        }
        for i in 2..4 {
            if !car.wheel_contact[i] {
                continue;
            }
            let at = car.wheel_contact_pos[i];
            if on_road(at) > 0.3 {
                continue;
            }
            // Spinning or sliding wheels throw more
            let rate = (speed * 0.12 + car.wheel_scrub[i] * 8.0).min(30.0);
            self.rubble_accum[i - 2] += dt * rate;
            let base = ground_color(at);
            while self.rubble_accum[i - 2] >= 1.0 {
                self.rubble_accum[i - 2] -= 1.0;
                let up = 4.0 + self.rand() * 7.0;
                let vel = vec3(self.spread(4.0), up, self.spread(4.0)) - car.vel * (0.1 + self.rand() * 0.15);
                let shade = 0.6 + self.rand() * 0.35;
                let (decay, size) = (1.0 + self.rand() * 0.6, 0.25 + self.rand() * 0.35);
                self.push(Particle {
                    pos: at + Vec3::Y * 0.3, vel, life: 1.0, decay, size, growth: 0.0,
                    color: [base[0] * shade, base[1] * shade, base[2] * shade], alpha: 1.0,
                    gravity: 45.0, drag: 0.4,
                });
            }
        }
    }

    /// Spray thrown up off a wet road behind the rear wheels, thicker the
    /// faster and wetter it is: following someone in the rain, you see less.
    pub fn spray(&mut self, car: &CarPhysics, wet: f32, paved: bool, dt: f32) {
        let speed = car.vel.length();
        if wet < 0.15 || !paved || speed < 25.0 || car.grounded_wheels() < 2 {
            return;
        }
        self.spray_accum += dt * (speed * 0.25 * wet).min(35.0);
        while self.spray_accum >= 1.0 {
            self.spray_accum -= 1.0;
            let wheel = 2 + (self.rand() * 2.0) as usize;
            let at = car.wheel_contact_pos[wheel] + Vec3::Y * 0.6;
            let vel = vec3(self.spread(2.5), 1.5 + self.rand() * 2.0, self.spread(2.5)) + car.vel * 0.55;
            let shade = 0.82 + self.rand() * 0.1;
            self.push(Particle {
                pos: at, vel, life: 1.0, decay: 1.7, size: 0.7, growth: 2.6,
                color: [shade, shade * 1.02, shade * 1.05], alpha: 0.16 * wet,
                gravity: 1.0, drag: 2.5,
            });
        }
    }

    /// Sparks off a hard knock: bright, quick, falling.
    pub fn sparks(&mut self, at: Vec3, count: usize, speed: f32) {
        for _ in 0..count {
            let vel = vec3(self.spread(1.0), self.rand() * 0.7 + 0.3, self.spread(1.0)) * speed;
            let heat = self.rand() * 0.4;
            let decay = 2.2 + self.rand();
            self.push(Particle {
                pos: at, vel, life: 1.0, decay, size: 0.18, growth: 0.0,
                color: [2.0, 1.2 + heat, 0.4], alpha: 1.0,
                gravity: 30.0, drag: 0.6,
            });
        }
    }

    /// Splinters of paint and trim knocked off in a collision.
    pub fn shards(&mut self, at: Vec3, color: [f32; 3], count: usize, speed: f32) {
        for _ in 0..count {
            let vel = vec3(self.spread(1.0), self.rand() * 0.8 + 0.4, self.spread(1.0)) * speed;
            let shade = 0.7 + self.rand() * 0.4;
            let (decay, size) = (0.7 + self.rand() * 0.4, 0.3 + self.rand() * 0.2);
            self.push(Particle {
                pos: at, vel, life: 1.0, decay, size, growth: 0.0,
                color: [color[0] * shade, color[1] * shade, color[2] * shade], alpha: 1.0,
                gravity: 45.0, drag: 0.3,
            });
        }
    }

    /// A firework: a big sphere of glowing sparks that drift down as they fade.
    pub fn firework(&mut self, at: Vec3, color: [f32; 3]) {
        for _ in 0..48 {
            let dir = vec3(self.spread(1.0), self.spread(1.0), self.spread(1.0)).normalize_or_zero();
            let vel = dir * (14.0 + self.rand() * 8.0);
            let decay = 0.7 + self.rand() * 0.3;
            self.push(Particle {
                pos: at, vel, life: 1.0, decay, size: 0.9, growth: -0.4,
                color, alpha: 1.0, gravity: 6.0, drag: 1.2,
            });
        }
    }

    /// A burst of coloured sparks (boost pads, pickups).
    pub fn burst(&mut self, at: Vec3, color: [f32; 3], count: usize, speed: f32) {
        for _ in 0..count {
            let vel = vec3(self.spread(1.0), self.rand() * 0.8 + 0.2, self.spread(1.0)) * speed;
            self.push(Particle { gravity: 1.0, drag: 1.5, pos: at, vel, life: 1.0, decay: 1.4, size: 0.5, growth: 0.3, color, alpha: 0.9 });
        }
    }

    /// Leaves drifting by day, fireflies at night, around the camera.
    pub fn emit_ambient(&mut self, camera: Vec3, day: f32, dt: f32) {
        self.ambient_accum += dt * 4.0;
        while self.ambient_accum >= 1.0 {
            self.ambient_accum -= 1.0;
            if day > 0.3 {
                let pos = camera + vec3(self.spread(40.0), 10.0 + self.spread(5.0), self.spread(40.0));
                let vel = vec3(2.0 + self.spread(1.5), -1.5 + self.spread(0.5), self.spread(1.0));
                let tint = self.rand() * 0.2;
                self.push(Particle { gravity: 1.0, drag: 1.5,
                    pos, vel, life: 1.0, decay: 0.25, size: 0.35, growth: 0.0,
                    color: [0.75 + tint, 0.5 + tint * 0.5, 0.15], alpha: 0.9,
                });
            } else {
                let x = camera.x + self.spread(30.0);
                let z = camera.z + self.spread(30.0);
                let pos = vec3(x, terrain_height(x, z) + 1.0 + self.rand() * 2.0, z);
                let vel = vec3(self.spread(1.0), self.spread(0.5), self.spread(1.0));
                self.push(Particle { gravity: 1.0, drag: 1.5,
                    pos, vel, life: 1.0, decay: 0.35, size: 0.25, growth: 0.0,
                    color: [0.9, 1.3, 0.4], alpha: 0.9,
                });
            }
        }
    }

    /// Spray thrown up where the car hits the water, more the faster it goes.
    pub fn splash(&mut self, at: Vec3, speed: f32) {
        let count = (8.0 + speed * 0.4).min(40.0) as usize;
        for _ in 0..count {
            let vel = vec3(self.spread(1.0) * 6.0, 6.0 + self.rand() * (6.0 + speed * 0.12), self.spread(1.0) * 6.0);
            let white = 0.85 + self.rand() * 0.15;
            let pos = at + vec3(self.spread(2.0), 0.0, self.spread(2.0));
            self.push(Particle { gravity: 1.0, drag: 1.5,
                pos,
                vel, life: 1.0, decay: 0.9, size: 0.6, growth: 0.8,
                color: [white * 0.9, white, white], alpha: 0.75,
            });
        }
    }

    /// Bubbles rising from something sinking.
    pub fn bubbles(&mut self, at: Vec3, surface: f32, dt: f32) {
        self.place_accum += dt * 10.0;
        while self.place_accum >= 1.0 {
            self.place_accum -= 1.0;
            let pos = vec3(at.x + self.spread(1.5), surface + 0.2, at.z + self.spread(1.5));
            let size = 0.25 + self.rand() * 0.2;
            self.push(Particle { gravity: 1.0, drag: 1.5,
                pos, vel: vec3(0.0, 0.8, 0.0), life: 1.0, decay: 1.6, size, growth: 0.3,
                color: [0.8, 0.92, 0.95], alpha: 0.7,
            });
        }
    }

    /// Fireflies drifting low over a place after dark.
    pub fn fireflies(&mut self, centre: Vec3, radius: f32, rate: f32, dt: f32) {
        self.place_accum += dt * rate;
        while self.place_accum >= 1.0 {
            self.place_accum -= 1.0;
            let x = centre.x + self.spread(radius);
            let z = centre.z + self.spread(radius);
            let pos = vec3(x, terrain_height(x, z) + 0.8 + self.rand() * 4.0, z);
            let vel = vec3(self.spread(0.8), 0.3 + self.spread(0.3), self.spread(0.8));
            let warm = self.rand() * 0.2;
            self.push(Particle { gravity: 1.0, drag: 1.5,
                pos, vel, life: 1.0, decay: 0.22, size: 0.22, growth: 0.0,
                color: [0.95 + warm, 1.4, 0.45 + warm], alpha: 0.95,
            });
        }
    }

    /// Steam curling up off warm water.
    pub fn steam(&mut self, centre: Vec3, radius: f32, rate: f32, dt: f32) {
        self.place_accum += dt * rate;
        while self.place_accum >= 1.0 {
            self.place_accum -= 1.0;
            let pos = centre + vec3(self.spread(radius), 0.5, self.spread(radius));
            let vel = vec3(1.0 + self.spread(0.6), 2.0 + self.rand() * 1.5, self.spread(0.6));
            let grey = 0.88 + self.rand() * 0.1;
            self.push(Particle { gravity: 1.0, drag: 1.5,
                pos, vel, life: 1.0, decay: 0.18, size: 1.2, growth: 1.6,
                color: [grey, grey, grey * 1.02], alpha: 0.3,
            });
        }
    }

    /// One puff of locomotive smoke from `at`, drifting with `drift`.
    pub fn chimney_smoke(&mut self, at: Vec3, drift: Vec3) {
        let vel = drift + vec3(self.spread(1.5), 7.0 + self.rand() * 3.0, self.spread(1.5));
        let grey = 0.55 + self.rand() * 0.25;
        self.push(Particle { gravity: 1.0, drag: 1.5,
            pos: at, vel, life: 1.0, decay: 0.3, size: 1.4, growth: 2.0,
            color: [grey, grey, grey * 1.03], alpha: 0.45,
        });
    }

    pub fn update(&mut self, dt: f32) {
        for p in &mut self.list {
            p.pos += p.vel * dt;
            p.vel.y -= p.gravity * dt;
            p.vel *= 1.0 - p.drag * dt;
            p.size += p.growth * dt;
            p.life -= p.decay * dt;
        }
        self.list.retain(|p| p.life > 0.0);
    }

    /// Camera-facing hexagons. `right` and `up` are the camera's world axes.
    pub fn build_mesh(&self, right: Vec3, up: Vec3) -> (Vec<Vertex>, Vec<u16>) {
        let mut verts = Vec::with_capacity(self.list.len() * (HEX_SIDES + 1));
        let mut idxs = Vec::with_capacity(self.list.len() * HEX_SIDES * 3);
        let corners: [(f32, f32); HEX_SIDES] = std::array::from_fn(|k| {
            let a = std::f32::consts::TAU * k as f32 / HEX_SIDES as f32 + 0.3;
            (a.cos(), a.sin())
        });
        for p in &self.list {
            // Fade in quickly, out slowly
            let alpha = p.alpha * (p.life * 1.5).min(1.0) * ((1.0 - p.life) * 8.0).min(1.0).max(0.3);
            let color = [p.color[0], p.color[1], p.color[2], alpha];
            let base = verts.len() as u16;
            verts.push(Vertex { pos: p.pos.into(), color });
            for &(cx, cy) in &corners {
                verts.push(Vertex { pos: (p.pos + (right * cx + up * cy) * p.size).into(), color });
            }
            for k in 0..HEX_SIDES as u16 {
                idxs.extend_from_slice(&[base, base + 1 + k, base + 1 + (k + 1) % HEX_SIDES as u16]);
            }
        }
        (verts, idxs)
    }
}
