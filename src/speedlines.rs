//! Wind streaks: faint lines rushing past at speed, out round the edges of
//! the view, stretched along the way the car is going.

use glam::Vec3;

use crate::geometry::Vertex;

const COUNT: usize = 48;
/// Streaks start to show at this speed (units/s) and are at full strength by
/// `FULL_SPEED`.
const FROM_SPEED: f32 = 65.0;
const FULL_SPEED: f32 = 140.0;
/// Where streaks are placed: how far ahead of the camera, and how far out
/// from its line of sight (kept clear of the middle, where the car is).
const AHEAD: (f32, f32) = (15.0, 70.0);
const OUT: (f32, f32) = (7.0, 18.0);
const WIDTH: f32 = 0.02;

pub struct SpeedLines {
    streaks: Vec<Vec3>,
    rng: u32,
}

impl SpeedLines {
    pub fn new() -> Self {
        Self { streaks: Vec::new(), rng: 0x2545_f491 }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn spawn(&mut self, cam: Vec3, fwd: Vec3, near_end: bool) -> Vec3 {
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd);
        // Off to the left or right, from a little below eye level to a
        // little above: never up in the sky or flat on the road
        let left = if self.rand() < 0.5 { -1.0 } else { 1.0 };
        let lift = -0.35 + self.rand() * 0.5;
        let out = OUT.0 + self.rand() * (OUT.1 - OUT.0);
        // New ones appear far ahead; at the very start, fill the whole depth
        let t = if near_end { self.rand() } else { 0.8 + self.rand() * 0.2 };
        let ahead = AHEAD.0 + t * (AHEAD.1 - AHEAD.0);
        cam + fwd * ahead + (right * left + up * lift).normalize() * out
    }

    /// Keep the streaks ahead of the camera, looking along `fwd`.
    pub fn update(&mut self, cam: Vec3, fwd: Vec3) {
        if self.streaks.is_empty() {
            for _ in 0..COUNT {
                let p = self.spawn(cam, fwd, true);
                self.streaks.push(p);
            }
        }
        for i in 0..COUNT {
            let rel = self.streaks[i] - cam;
            let ahead = rel.dot(fwd);
            if ahead < 2.0 || ahead > AHEAD.1 * 1.3 || (rel - fwd * ahead).length() > OUT.1 * 1.5 {
                self.streaks[i] = self.spawn(cam, fwd, false);
            }
        }
    }

    /// Thin camera-facing lines along `vel`, faint below `FROM_SPEED`.
    pub fn mesh(&self, cam: Vec3, vel: Vec3) -> (Vec<Vertex>, Vec<u16>) {
        let speed = vel.length();
        let strength = ((speed - FROM_SPEED) / (FULL_SPEED - FROM_SPEED)).clamp(0.0, 1.0);
        let (mut verts, mut idxs) = (Vec::new(), Vec::new());
        if strength <= 0.0 {
            return (verts, idxs);
        }
        let dir = vel / speed;
        let length = speed * 0.09;
        for &p in &self.streaks {
            let tail = p - dir * length;
            let across = dir.cross(p - cam).normalize_or_zero() * WIDTH * (p - cam).length().sqrt();
            // Fainter close by and far off
            let d = (p - cam).length();
            let alpha = 0.2 * crate::tuning::get().speed_lines * strength * (d / 15.0).min(1.0) * (1.0 - d / (AHEAD.1 * 1.3)).clamp(0.0, 1.0);
            let base = verts.len() as u16;
            for (q, a) in [(p - across, alpha), (p + across, alpha), (tail + across, 0.0), (tail - across, 0.0)] {
                verts.push(Vertex { pos: q.into(), color: [1.0, 1.0, 1.0, a] });
            }
            idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        (verts, idxs)
    }
}
