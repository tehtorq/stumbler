//! Cheap deterministic noise shared by world generation.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A map keyed by grid cell, for lookups too frequent to pay for the
/// standard hasher.
pub type CellMap<V> = HashMap<(i32, i32), V, BuildHasherDefault<CellHasher>>;

#[derive(Default)]
pub struct CellHasher(u64);

impl Hasher for CellHasher {
    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&b| self.write_u32(b as u32));
    }

    fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }

    fn write_u32(&mut self, v: u32) {
        self.0 = (self.0.rotate_left(5) ^ v as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Pseudo-random 0..1 from a position.
pub fn hash(x: f32, z: f32) -> f32 {
    let h = (x * 127.1 + z * 311.7).sin() * 43758.547;
    h - h.floor()
}

/// Smooth value noise, roughly 0..1, with features about `scale` across.
pub fn noise(x: f32, z: f32, scale: f32) -> f32 {
    let (u, v) = (x / scale, z / scale);
    let (iu, iv) = (u.floor(), v.floor());
    let (fu, fv) = (u - iu, v - iv);
    let (su, sv) = (fu * fu * (3.0 - 2.0 * fu), fv * fv * (3.0 - 2.0 * fv));
    let a = hash(iu, iv);
    let b = hash(iu + 1.0, iv);
    let c = hash(iu, iv + 1.0);
    let d = hash(iu + 1.0, iv + 1.0);
    let top = a + (b - a) * su;
    let bottom = c + (d - c) * su;
    top + (bottom - top) * sv
}

/// Two octaves of `noise`, for patches with ragged edges.
pub fn patchy(x: f32, z: f32, scale: f32) -> f32 {
    noise(x, z, scale) * 0.7 + noise(x + 517.0, z - 263.0, scale * 0.35) * 0.3
}
