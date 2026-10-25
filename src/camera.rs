//! Chase camera.

use glam::{Mat4, Vec3, vec3};

use crate::physics::CarPhysics;
use crate::terrain::terrain_height;

pub struct ChaseCamera {
    pub pos: Vec3,
    pub target: Vec3,
    /// Distance behind the car (mouse wheel adjusts it).
    pub distance: f32,
    /// Smoothed direction the camera looks along (XZ plane).
    look_dir: Vec3,
    /// Extra field of view (degrees) from boosts and slipstreams, smoothed.
    pub fov_kick: f32,
    /// How rough the ride is (kerbs, grass): shakes the view.
    pub rumble: f32,
}

impl ChaseCamera {
    pub fn new(car: &CarPhysics) -> Self {
        let mut cam = Self { pos: Vec3::ZERO, target: Vec3::ZERO, distance: crate::tuning::get().camera_distance, look_dir: car.heading(), fov_kick: 0.0, rumble: 0.0 };
        cam.snap(car);
        cam
    }

    /// Jump straight to the resting position behind the car.
    pub fn snap(&mut self, car: &CarPhysics) {
        self.look_dir = car.heading();
        self.pos = car.pos - self.look_dir * self.distance + vec3(0.0, self.distance * 0.4, 0.0);
        self.target = car.pos + vec3(0.0, 3.0, 0.0);
    }

    /// `zoom` scales the distance (jumps, lap pulses).
    pub fn update(&mut self, car: &CarPhysics, zoom: f32, dt: f32) {
        // Look between where the car points and where it's going, so drifts read
        let heading = car.heading();
        let flat_vel = vec3(car.vel.x, 0.0, car.vel.z);
        let travel = if flat_vel.length() > 15.0 && car.speed > 0.0 { flat_vel.normalize() } else { heading };
        // In a held drift lean further toward the line of travel, so the
        // car's angle shows and the road ahead stays in view
        let w = if car.drifting { 0.65 } else { 0.4 };
        let desired = or(heading * (1.0 - w) + travel * w, heading);
        let turn = 1.0 - (-4.0 * dt).exp();
        self.look_dir = or(self.look_dir + (desired - self.look_dir) * turn, desired);

        // Down lower and in a little closer at speed, so the road rushes by
        let fast = (car.speed.abs() / 170.0).clamp(0.0, 1.0);
        let drop = crate::tuning::get().camera_drop;
        let dist = self.distance * zoom * (1.0 - drop * fast);
        let ideal_pos = car.pos - self.look_dir * dist + vec3(0.0, dist * (0.4 - drop * fast), 0.0);
        let ideal_target = car.pos + vec3(0.0, 3.0, 0.0) + self.look_dir * 6.0;

        let smooth = 1.0 - (-8.0 * dt).exp();
        self.pos += (ideal_pos - self.pos) * smooth;
        self.target += (ideal_target - self.target) * smooth;

        let ground = terrain_height(self.pos.x, self.pos.z) + 3.0;
        self.pos.y = self.pos.y.max(ground);
    }

    /// View-projection matrix. FOV widens with speed; slight shake near top speed.
    pub fn view_proj(&self, aspect: f32, speed: f32, time: f32) -> Mat4 {
        let speed_ratio = (speed.abs() / 170.0).min(1.0);
        let fov = 60.0 + speed_ratio * crate::tuning::get().fov_at_speed + self.fov_kick;
        let far = crate::chunks::DRAW_DISTANCE + 800.0;
        let proj = Mat4::perspective_lh(fov.to_radians(), aspect, 1.0, far);
        let shake_amount = ((speed_ratio - 0.7).max(0.0) / 0.3) * 0.15 + self.rumble * 0.35;
        let t = time * 30.0;
        // Two frequencies, so the judder doesn't look like a clean wobble
        let shake = vec3(t.sin() + (t * 2.7).sin() * 0.5, (t * 1.3).cos() * 0.5 + (t * 3.1).cos() * 0.4, 0.0) * shake_amount;
        proj * Mat4::look_at_lh(self.pos + shake, self.target, Vec3::Y)
    }

    /// Camera right and up axes, for billboards.
    pub fn billboard_axes(&self) -> (Vec3, Vec3) {
        let forward = or(self.target - self.pos, Vec3::NEG_Z);
        let right = or(forward.cross(Vec3::Y), Vec3::X);
        (right, Vec3::Y)
    }
}

/// Normalize `v`, or return `fallback` if it has no length.
fn or(v: Vec3, fallback: Vec3) -> Vec3 {
    let n = v.normalize_or_zero();
    if n == Vec3::ZERO { fallback } else { n }
}
