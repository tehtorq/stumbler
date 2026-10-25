//! Cheap deterministic noise shared by world generation.

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
