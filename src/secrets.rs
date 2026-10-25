//! The secret places: hand-built scenes with no trail leading to them, each
//! with its own small story. Some come alive after dark (see `Kind::magical`):
//! their carvings, mushrooms and lanterns go in the glow buffer.

use glam::{Quat, Vec2, Vec3, vec3};

use crate::landmarks::{Kind, Landmark};
use crate::props::{ground, hash, Builder};
use crate::terrain::{self, terrain_height, WATER_LEVEL};

type Col = [f32; 4];

const STONE: Col = [0.52, 0.51, 0.48, 1.0];
const STONE_DARK: Col = [0.4, 0.39, 0.37, 1.0];
const LICHEN: Col = [0.62, 0.65, 0.35, 1.0];
const MOSS: Col = [0.3, 0.46, 0.18, 1.0];
const MARBLE: Col = [0.86, 0.84, 0.78, 1.0];
const SANDSTONE: Col = [0.88, 0.73, 0.52, 1.0];
const BARK: Col = [0.3, 0.22, 0.14, 1.0];
const WOOD: Col = [0.42, 0.3, 0.19, 1.0];
const OLD_WOOD: Col = [0.36, 0.32, 0.27, 1.0];
const RUNE: Col = [0.45, 0.85, 1.0, 1.0];
const LANTERN: Col = [1.0, 0.78, 0.38, 1.0];

fn shade(c: Col, s: f32) -> Col {
    [c[0] * s, c[1] * s, c[2] * s, c[3]]
}

pub fn build(b: &mut Builder, l: &Landmark) {
    let seed = (l.seed % 1000) as f32;
    match l.kind {
        Kind::StandingStones => standing_stones(b, l.pos, seed),
        Kind::SunkenTemple => temple(b, l.pos, seed),
        Kind::OldRacer => old_racer(b, l.pos, seed),
        Kind::FairyRing => fairy_ring(b, l.pos, seed),
        Kind::Watcher => watcher(b, l.pos, seed),
        Kind::GreatOak => great_oak(b, l.pos, seed),
        Kind::Shipwreck => shipwreck(b, l.pos, seed),
        Kind::HotSpring => hot_spring(b, l.pos, seed),
        _ => {}
    }
}

/// Unit vector from `p` toward the nearest open water.
fn toward_water(p: Vec2) -> Vec2 {
    (0..24)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let d = Vec2::new(a.cos(), a.sin());
            let depth: f32 = [25.0, 45.0, 70.0].iter().map(|r| terrain::raw_height(p.x + d.x * r, p.y + d.y * r)).sum();
            (d, depth)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(Vec2::X, |(d, _)| d)
}

/// Unit vector from `p` toward the nearest road, the way travellers come.
fn toward_road(p: Vec2) -> Vec2 {
    crate::road::network()
        .and_then(|n| n.paths.iter()
            .filter(|path| path.kind == crate::road::PathKind::Road)
            .flat_map(|path| path.points.iter().step_by(4).copied())
            .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p))))
        .map_or(Vec2::Y, |q| (q - p).normalize_or_zero())
}

/// Yaw that turns local +Z toward `dir`.
fn yaw_toward(dir: Vec2) -> f32 {
    dir.x.atan2(dir.y)
}

/// A frame at `origin` with local +Z along `yaw`: local → world.
fn frame(origin: Vec3, yaw: f32) -> impl Fn(Vec3) -> Vec3 {
    let r = Quat::from_rotation_y(yaw);
    move |p| origin + r * p
}

/// A ring of ten standing stones on a rise, one fallen, with spiral carvings
/// on their inner faces and a flat altar stone in the middle.
fn standing_stones(b: &mut Builder, p: Vec2, seed: f32) {
    let radius = 17.0;
    let stones = 10;
    let fallen = (seed as usize) % stones;
    for k in 0..stones {
        let a = k as f32 * std::f32::consts::TAU / stones as f32 + seed;
        let out = Vec2::new(a.cos(), a.sin());
        let q = p + out * radius;
        let y = ground(q, 2.0) - 0.8;
        let h = 7.0 + 3.0 * hash(seed, k as f32);
        let w = 1.3 + 0.5 * hash(k as f32, seed);
        let col = shade(STONE, 0.9 + 0.2 * hash(seed + 3.0, k as f32));
        if k == fallen {
            // Toppled outward long ago, half sunk in the turf
            let base = vec3(q.x, y + 1.0, q.y);
            let tip = base + vec3(out.x, 0.0, out.y) * h + Vec3::Y * 0.4;
            b.limb(base, tip, w, w * 0.8, 4, a, col);
            b.blob(base + vec3(out.x, 0.0, out.y) * h * 0.5 + Vec3::Y * w * 0.8, 0.9, 0.4, LICHEN, seed + k as f32);
            continue;
        }
        let lean = vec3(out.x, 0.0, out.y) * 0.4 * (hash(seed, k as f32 + 9.0) - 0.3);
        let base = vec3(q.x, y, q.y);
        let top = base + Vec3::Y * h + lean;
        b.limb(base, top, w, w * 0.75, 4, a + std::f32::consts::FRAC_PI_4, col);
        b.blob(top - Vec3::Y * h * 0.25 + vec3(out.x, 0.0, out.y) * w * 0.5, 0.7, 0.5, LICHEN, seed + k as f32);
        b.post(base, w, h);
        // Carved spirals on both broad faces, faint by day, glowing after dark
        b.glowing = true;
        let side = vec3(-out.y, 0.0, out.x);
        for facing in [-1.0, 1.0] {
            let normal = vec3(out.x, 0.0, out.y) * facing;
            let face = base + normal * (w * 0.74) + Vec3::Y * h * 0.55 + lean * 0.5;
            for t in 0..9 {
                let ang = t as f32 * 0.95;
                let r0 = 0.2 + t as f32 * 0.11;
                let c0 = face + (side * ang.cos() + Vec3::Y * ang.sin()) * r0;
                let c1 = face + (side * (ang + 0.95).cos() + Vec3::Y * (ang + 0.95).sin()) * (r0 + 0.11);
                let thick = Vec3::Y * 0.15;
                b.quad(c0 - thick, c1 - thick, c1 + thick, c0 + thick, RUNE);
            }
        }
        b.glowing = false;
    }
    // Altar stone, with a ring of runes cut into the ground round it
    let y = ground(p, 3.0);
    b.block(vec3(p.x, y - 0.3, p.y), 2.6, 1.3, 1.6, seed, STONE_DARK, shade(STONE, 1.05));
    b.glowing = true;
    for k in 0..16 {
        let a = k as f32 * std::f32::consts::TAU / 16.0;
        let c = vec3(p.x + a.cos() * 6.0, terrain_height(p.x + a.cos() * 6.0, p.y + a.sin() * 6.0) + 0.06, p.y + a.sin() * 6.0);
        let t = vec3(-a.sin(), 0.0, a.cos()) * 0.9;
        let n = vec3(a.cos(), 0.0, a.sin()) * 0.25;
        b.quad(c - t - n, c + t - n, c + t + n, c - t + n, RUNE);
    }
    b.glowing = false;
}

/// A ruined temple on the shore: a stepped platform, columns (two broken,
/// one fallen), a headless statue, and steps running down under the water to
/// a doorway that is still lit.
fn temple(b: &mut Builder, p: Vec2, seed: f32) {
    let dir = toward_water(p);
    let yaw = yaw_toward(dir);
    let y = ground(p, 12.0) - 0.5;
    let w = frame(vec3(p.x, y, p.y), yaw);
    let at = |x: f32, z: f32| w(vec3(x, 0.0, z));
    // Platform in three steps
    for (k, (hx, hz)) in [(14.0, 16.0), (12.0, 14.0), (10.0, 12.0)].into_iter().enumerate() {
        b.block(at(0.0, -2.0) + Vec3::Y * k as f32 * 0.8, hx, 0.8, hz, yaw, shade(MARBLE, 0.85), if k == 2 { MARBLE } else { shade(MARBLE, 0.95) });
    }
    let floor = 2.4;
    // Two rows of columns
    for (k, z) in [-10.0, -4.5, 1.0, 6.5].into_iter().enumerate() {
        for x in [-7.5, 7.5] {
            let i = k * 2 + (x > 0.0) as usize;
            let base = at(x, z) + Vec3::Y * floor;
            let height = match i { 3 => 4.5, 6 => 7.0, _ => 13.0 };
            b.limb(base, base + Vec3::Y * 0.6, 1.5, 1.5, 4, std::f32::consts::FRAC_PI_4 + yaw, MARBLE);
            b.limb(base + Vec3::Y * 0.6, base + Vec3::Y * height, 1.05, 0.95, 8, yaw, MARBLE);
            if height > 10.0 {
                b.limb(base + Vec3::Y * height, base + Vec3::Y * (height + 0.8), 1.5, 1.6, 4, std::f32::consts::FRAC_PI_4 + yaw, MARBLE);
            } else {
                // Broken off: moss on the stump
                b.blob(base + Vec3::Y * height, 1.0, 0.4, MOSS, seed + i as f32);
            }
            b.post(base, 1.1, height);
        }
    }
    // One lies where it fell, in drums
    for k in 0..4 {
        let c = at(13.0 + k as f32 * 0.4, -6.0 + k as f32 * 2.4) + Vec3::Y * 1.0;
        let d = w(vec3(0.15, 0.0, 1.0)) - w(Vec3::ZERO);
        b.limb(c, c + d * 2.1, 1.0, 1.0, 8, seed + k as f32, shade(MARBLE, 0.92));
        if k % 2 == 0 {
            b.blob(c + Vec3::Y * 0.9, 0.6, 0.4, MOSS, seed + k as f32 * 3.0);
        }
    }
    // Headless statue on a plinth, one arm raised toward the water
    let plinth = at(0.0, -11.5) + Vec3::Y * floor;
    b.block(plinth, 1.8, 2.5, 1.8, yaw, shade(MARBLE, 0.9), MARBLE);
    let hips = plinth + Vec3::Y * 2.5;
    b.limb(hips, hips + Vec3::Y * 3.5, 1.2, 1.0, 6, yaw, MARBLE);
    let shoulder = hips + Vec3::Y * 3.3;
    b.limb(shoulder + w(vec3(1.0, 0.0, 0.0)) - w(Vec3::ZERO), shoulder + w(vec3(1.8, 2.5, 1.8)) - w(Vec3::ZERO), 0.4, 0.3, 5, yaw, MARBLE);
    b.limb(shoulder - (w(vec3(1.0, 0.0, 0.0)) - w(Vec3::ZERO)), shoulder + w(vec3(-1.3, -1.8, 0.3)) - w(Vec3::ZERO), 0.4, 0.3, 5, yaw, MARBLE);
    b.blob(shoulder + Vec3::Y * 0.4, 0.5, 0.4, MOSS, seed);
    b.post(plinth, 1.8, 7.0);
    // Steps running down into the water
    let mut z = 14.0;
    let mut step_y = floor;
    while step_y > (WATER_LEVEL - 6.0) - y {
        b.block(at(0.0, z), 6.0, 0.7, 1.2, yaw, shade(MARBLE, 0.8), shade(MARBLE, 0.9));
        z += 2.4;
        step_y -= 0.9;
        let c = at(0.0, z);
        b.block(vec3(c.x, y + step_y - 0.7, c.z), 6.0, 0.7, 1.2, yaw, shade(MARBLE, 0.8), shade(MARBLE, 0.9));
        z += 0.01;
    }
    // A doorway at the bottom, under the water, with a light inside
    let door = at(0.0, z + 2.0);
    let dy = y + step_y - 0.5;
    for x in [-3.0, 3.0] {
        let c = at(x, z + 2.0);
        b.block(vec3(c.x, dy, c.z), 0.9, 6.0, 0.9, yaw, shade(MARBLE, 0.75), MARBLE);
    }
    b.block(vec3(door.x, dy + 6.0, door.z), 3.9, 1.0, 1.0, yaw, shade(MARBLE, 0.75), MARBLE);
    b.glowing = true;
    let (l, r) = (at(-2.1, z + 2.6), at(2.1, z + 2.6));
    b.quad(vec3(l.x, dy, l.z), vec3(r.x, dy, r.z), vec3(r.x, dy + 6.0, r.z), vec3(l.x, dy + 6.0, l.z), [0.35, 0.95, 0.9, 1.0]);
    // Its light spills up the last steps
    for k in 0..4 {
        let c = at(0.0, z - k as f32 * 2.4);
        let sy = dy + 0.75 + k as f32 * 0.9;
        b.quad(vec3(c.x, sy, c.z) + (at(-4.0, 0.0) - at(0.0, 0.0)), vec3(c.x, sy, c.z) + (at(4.0, 0.0) - at(0.0, 0.0)),
            vec3(c.x, sy, c.z) + (at(4.0, 0.6) - at(0.0, 0.0)), vec3(c.x, sy, c.z) + (at(-4.0, 0.6) - at(0.0, 0.0)), [0.25, 0.75, 0.75, 1.0]);
    }
    b.glowing = false;
}

/// A vintage race car, number 7, rusting in a clearing: flat tyres sunk in
/// the moss, ivy climbing it, a chequered flag on a stick, and fresh flowers
/// on the nose.
fn old_racer(b: &mut Builder, p: Vec2, seed: f32) {
    let yaw = seed * 0.7;
    // Resting on the forest floor, tilted to its slope and settled in a little
    let h = |dx: f32, dz: f32| terrain_height(p.x + dx, p.y + dz);
    let normal = vec3(h(-4.0, 0.0) - h(4.0, 0.0), 8.0, h(0.0, -4.0) - h(0.0, 4.0)).normalize();
    let tilt = Quat::from_rotation_arc(Vec3::Y, normal) * Quat::from_rotation_y(yaw);
    let origin = vec3(p.x, h(0.0, 0.0) - 0.35, p.y);
    // A little larger than life, so it reads among the trees
    let w = |q: Vec3| origin + tilt * (q * 1.3);
    let paint: Col = [0.62, 0.2, 0.13, 1.0];
    let rust: Col = [0.45, 0.25, 0.14, 1.0];
    // Cigar body in sections, nose to tail, faded and rusting through
    let sections = [(-4.6, 0.25), (-3.4, 0.75), (-1.8, 1.0), (0.0, 1.1), (1.8, 1.05), (3.2, 0.85), (4.4, 0.5)];
    for k in 0..sections.len() - 1 {
        let (z0, r0) = sections[k];
        let (z1, r1) = sections[k + 1];
        let col = if (k + seed as usize) % 3 == 1 { rust } else { paint };
        b.limb(w(vec3(0.0, 1.25, z0)), w(vec3(0.0, 1.25, z1)), r0, r1, 8, std::f32::consts::FRAC_PI_8, col);
    }
    // Number roundels on both flanks
    for s in [-1.0_f32, 1.0] {
        let c = w(vec3(s * 1.08, 1.35, 0.6));
        let n = w(vec3(s, 0.0, 0.0)) - w(Vec3::ZERO);
        let t = w(vec3(0.0, 0.0, 1.0)) - w(Vec3::ZERO);
        for k in 0..8 {
            let a0 = k as f32 * std::f32::consts::TAU / 8.0;
            let a1 = (k + 1) as f32 * std::f32::consts::TAU / 8.0;
            b.tri(c + n * 0.02, c + n * 0.02 + (t * a0.cos() + Vec3::Y * a0.sin()) * 0.6, c + n * 0.02 + (t * a1.cos() + Vec3::Y * a1.sin()) * 0.6, [0.92, 0.9, 0.82, 1.0]);
        }
        // A blocky 7
        let ink: Col = [0.08, 0.08, 0.08, 1.0];
        let o = c + n * 0.05;
        b.quad(o + t * -0.25 + Vec3::Y * 0.3, o + t * 0.25 + Vec3::Y * 0.3, o + t * 0.25 + Vec3::Y * 0.42, o + t * -0.25 + Vec3::Y * 0.42, ink);
        b.quad(o + t * 0.13 + Vec3::Y * 0.3, o + t * 0.25 + Vec3::Y * 0.3, o + t * -0.05 + Vec3::Y * -0.4, o + t * -0.17 + Vec3::Y * -0.4, ink);
    }
    // Cockpit, windscreen and a little headrest
    b.limb(w(vec3(0.0, 2.1, 0.4)), w(vec3(0.0, 2.1, 1.5)), 0.55, 0.55, 6, 0.0, [0.08, 0.08, 0.08, 1.0]);
    b.quad(w(vec3(-0.55, 2.2, -0.2)), w(vec3(0.55, 2.2, -0.2)), w(vec3(0.45, 2.8, 0.05)), w(vec3(-0.45, 2.8, 0.05)), [0.55, 0.62, 0.62, 1.0]);
    b.limb(w(vec3(0.0, 2.2, 1.6)), w(vec3(0.0, 2.9, 2.0)), 0.35, 0.3, 6, 0.0, paint);
    // Wire wheels with perished tyres, flat and sunk to the rims
    for (x, z) in [(-1.6, -2.8), (1.6, -2.8), (-1.65, 2.6), (1.65, 2.6)] {
        let s: f32 = if x < 0.0 { -1.0 } else { 1.0 };
        let hub = w(vec3(x, 0.75, z));
        let axle = w(vec3(s * 0.35, 0.0, 0.0)) - w(Vec3::ZERO);
        b.limb(hub - axle, hub + axle, 1.05, 1.05, 10, 0.0, [0.12, 0.12, 0.11, 1.0]);
        b.limb(hub + axle * 1.02, hub + axle * 1.1, 0.55, 0.55, 8, 0.0, [0.6, 0.58, 0.52, 1.0]);
    }
    // Ivy creeping over the tail
    for k in 0..5 {
        let c = w(vec3(0.6 * (hash(seed, k as f32) - 0.5), 0.9 + k as f32 * 0.25, 3.0 + k as f32 * 0.3));
        b.blob(c, 0.55, 0.6, shade(MOSS, 0.9 + 0.2 * hash(k as f32, seed)), seed + k as f32);
    }
    // Fresh flowers on the nose: someone still visits
    for k in 0..5 {
        let a = k as f32 * 1.3;
        let c = w(vec3(a.cos() * 0.3, 1.6, -3.6 + a.sin() * 0.3));
        let bloom: Col = if k % 2 == 0 { [0.95, 0.95, 0.9, 1.0] } else { [0.95, 0.35, 0.45, 1.0] };
        b.limb(c - Vec3::Y * 0.2, c + vec3(0.0, 0.25, -0.3), 0.04, 0.04, 3, 0.0, MOSS);
        b.blob(c + vec3(0.0, 0.3, -0.35), 0.18, 0.6, bloom, seed + k as f32 * 2.0);
    }
    // A chequered flag on a stick, stuck in the ground beside it
    let pole = w(vec3(3.2, 0.0, -3.5));
    b.limb(pole, pole + Vec3::Y * 4.5, 0.07, 0.06, 4, 0.0, OLD_WOOD);
    let fx = w(vec3(0.0, 0.0, 1.0)) - w(Vec3::ZERO);
    for i in 0..4 {
        for j in 0..3 {
            let c = pole + Vec3::Y * (4.4 - j as f32 * 0.35) + fx * (0.05 + i as f32 * 0.35);
            let col: Col = if (i + j) % 2 == 0 { [0.95, 0.95, 0.92, 1.0] } else { [0.08, 0.08, 0.08, 1.0] };
            b.quad(c, c + fx * 0.35, c + fx * 0.35 - Vec3::Y * 0.35, c - Vec3::Y * 0.35, col);
        }
    }
    b.post(origin, 3.5, 3.0);
}

/// Pale mushrooms in a ring, glowing after dark, round a mossy stone.
fn fairy_ring(b: &mut Builder, p: Vec2, seed: f32) {
    let count = 18;
    for k in 0..count {
        let a = k as f32 * std::f32::consts::TAU / count as f32 + 0.2 * hash(seed, k as f32);
        let r = 8.0 + 0.8 * hash(k as f32, seed);
        let q = p + Vec2::new(a.cos(), a.sin()) * r;
        let y = terrain_height(q.x, q.y) - 0.1;
        let size = 0.8 + 0.7 * hash(seed + 2.0, k as f32);
        let base = vec3(q.x, y, q.y);
        b.limb(base, base + Vec3::Y * size * 1.3, size * 0.16, size * 0.12, 6, 0.0, [0.9, 0.88, 0.8, 1.0]);
        b.glowing = true;
        let cap: Col = if k % 3 == 0 { [0.7, 0.55, 1.0, 1.0] } else { [0.55, 0.95, 0.9, 1.0] };
        b.cone(base + Vec3::Y * size * 1.2, size * 0.6, size * 0.55, 7, cap);
        b.glowing = false;
    }
    let y = terrain_height(p.x, p.y);
    b.boulder(vec3(p.x, y + 0.4, p.y), 1.4, seed, STONE);
    b.blob(vec3(p.x, y + 1.2, p.y), 0.9, 0.4, MOSS, seed);
    b.glowing = true;
    b.quad(vec3(p.x - 0.4, y + 1.0, p.y + 1.25), vec3(p.x + 0.4, y + 1.0, p.y + 1.25), vec3(p.x + 0.4, y + 1.6, p.y + 1.15), vec3(p.x - 0.4, y + 1.6, p.y + 1.15), RUNE);
    b.glowing = false;
}

/// A colossal stone face buried to the chin, staring at the horizon, with a
/// broken hand reaching out of the dunes nearby.
fn watcher(b: &mut Builder, p: Vec2, seed: f32) {
    // It gazes out toward the road, so those who follow the cairns meet its eyes
    let yaw = yaw_toward(toward_road(p)) + (hash(seed, 1.0) - 0.5) * 0.5;
    let y = ground(p, 10.0) - 3.0;
    let w = frame(vec3(p.x, y, p.y), yaw);
    let stone = SANDSTONE;
    let worn = shade(SANDSTONE, 0.88);
    let deep: Col = [0.28, 0.2, 0.14, 1.0];
    let crack: Col = [0.45, 0.34, 0.24, 1.0];
    // Built up in tiers, tapering like a head: chin, cheeks, mid-face, brow, forehead
    let tiers = [(5.0, 0.0, 4.5, 5.0), (7.2, 4.0, 5.0, 6.2), (7.0, 8.5, 4.5, 6.0), (7.4, 12.8, 1.9, 6.6), (6.2, 14.6, 4.2, 5.8)];
    for (k, &(half_x, y0, h, half_z)) in tiers.iter().enumerate() {
        b.block(w(vec3(0.0, y0, 0.0)), half_x, h, half_z, yaw, if k % 2 == 0 { worn } else { stone }, stone);
    }
    // Ears
    for s in [-1.0, 1.0] {
        b.block(w(vec3(s * 7.6, 7.5, -0.5)), 0.9, 6.0, 1.6, yaw, worn, stone);
    }
    // Deep eye sockets under the heavy brow, with heavy-lidded eyes inside
    for s in [-1.0, 1.0] {
        b.block(w(vec3(s * 3.4, 10.0, 6.05)), 2.3, 2.8, 0.25, yaw, deep, deep);
        b.block(w(vec3(s * 3.4, 10.6, 6.2)), 1.5, 1.4, 0.3, yaw, worn, stone);
        b.block(w(vec3(s * 3.4, 11.8, 6.25)), 1.8, 0.6, 0.35, yaw, stone, stone);
    }
    // Nose: a long wedge with flared nostrils
    b.limb(w(vec3(0.0, 12.6, 6.6)), w(vec3(0.0, 7.4, 8.6)), 1.1, 2.0, 3, std::f32::consts::FRAC_PI_2, stone);
    for s in [-1.0_f32, 1.0] {
        b.block(w(vec3(s * 0.9, 7.0, 8.0)), 0.5, 0.4, 0.4, yaw, deep, deep);
    }
    // Full lips, the lower one half in the sand
    b.block(w(vec3(0.0, 5.0, 6.0)), 3.2, 1.0, 0.8, yaw, worn, stone);
    b.block(w(vec3(0.0, 3.6, 5.6)), 2.8, 1.1, 0.9, yaw, worn, stone);
    b.block(w(vec3(0.0, 4.95, 6.7)), 3.0, 0.12, 0.2, yaw, deep, deep);
    // Weathering: cracks across the cheek and forehead
    for (x0, y0, x1, y1) in [(-6.0, 6.0, -3.5, 8.5), (4.5, 15.0, 6.0, 18.0), (1.0, 16.5, 3.0, 15.0)] {
        let (a, c) = (w(vec3(x0, y0, 6.25)), w(vec3(x1, y1, 6.25)));
        let t = vec3(0.0, 0.12, 0.0);
        b.quad(a - t, c - t, c + t, a + t, crack);
    }
    // The crown is broken: jagged stumps on top, one piece fallen beside
    for k in 0..4 {
        let x = -4.5 + k as f32 * 3.0;
        let h = 1.5 + 3.5 * hash(seed, k as f32);
        b.block(w(vec3(x, 18.8, 0.0)), 1.3, h, 5.0, yaw + 0.05 * k as f32, worn, stone);
    }
    b.block(w(vec3(13.0, 2.0, -3.0)), 3.0, 3.0, 5.0, yaw + 0.6, worn, stone);
    b.post(vec3(p.x, y, p.y), 9.0, 22.0);
    // Sand drifted against the sides
    for s in [-1.0, 1.0] {
        b.blob(w(vec3(s * 8.0, 2.5, 0.0)), 6.0, 0.4, [0.82, 0.68, 0.46, 1.0], seed + s);
    }
    // The hand, some way off, fingers curled up out of the sand
    let hand = w(vec3(-26.0, 0.0, 14.0));
    let hy = terrain_height(hand.x, hand.z) - 1.0;
    let hw = frame(vec3(hand.x, hy, hand.z), yaw + 0.8);
    b.block(hw(Vec3::ZERO), 3.0, 4.5, 1.6, yaw + 0.8, worn, stone);
    for (k, x) in [-2.2, -0.75, 0.75, 2.2].into_iter().enumerate() {
        let h = [5.0, 6.5, 6.0, 4.5][k];
        let knuckle = hw(vec3(x, 4.5, 0.0));
        let tip = hw(vec3(x, 4.5 + h, 1.5 + h * 0.3));
        b.limb(knuckle, tip, 0.7, 0.6, 5, 0.0, stone);
    }
    b.limb(hw(vec3(3.2, 2.5, 0.5)), hw(vec3(5.0, 6.0, 2.5)), 0.8, 0.65, 5, 0.0, stone);
    b.post(vec3(hand.x, hy, hand.z), 4.0, 10.0);
}

/// A vast old oak alone on a hill: spreading roots, a crown wider than a
/// house, a rope swing, a bench, and lanterns hung in its branches.
fn great_oak(b: &mut Builder, p: Vec2, seed: f32) {
    let y = ground(p, 4.0) - 0.5;
    let base = vec3(p.x, y, p.y);
    let top = base + Vec3::Y * 16.0;
    b.limb(base, top, 3.6, 2.4, 9, seed, BARK);
    b.post(base, 3.6, 16.0);
    // Roots spreading out over the ground
    for k in 0..6 {
        let a = seed + k as f32 * 1.05;
        let out = vec3(a.cos(), 0.0, a.sin());
        b.limb(base + out * 2.0 + Vec3::Y * 1.2, base + out * (7.0 + 2.0 * hash(seed, k as f32)) - Vec3::Y * 0.3, 1.1, 0.3, 5, a, shade(BARK, 0.9));
    }
    // Great limbs, each ending in a clump of leaves
    let mut ends = Vec::new();
    for k in 0..7 {
        let a = seed * 2.0 + k as f32 * 0.9;
        let out = vec3(a.cos(), 0.0, a.sin());
        let from = top - Vec3::Y * (2.0 + 4.0 * hash(seed, k as f32));
        let to = from + out * (10.0 + 6.0 * hash(k as f32, seed)) + Vec3::Y * (5.0 + 5.0 * hash(seed + 3.0, k as f32));
        b.limb(from, to, 1.3, 0.5, 6, a, BARK);
        ends.push(to);
    }
    for (k, &e) in ends.iter().enumerate() {
        for j in 0..3 {
            let a = seed + (k * 3 + j) as f32 * 2.1;
            let c = e + vec3(a.cos() * 3.0, 2.0 + j as f32 * 1.2, a.sin() * 3.0);
            let green: Col = [0.2 + 0.08 * hash(a, 1.0), 0.42 + 0.1 * hash(a, 2.0), 0.13, 1.0];
            b.blob(c, 5.5 + 2.0 * hash(a, 3.0), 0.75, green, a);
        }
    }
    b.blob(top + Vec3::Y * 10.0, 9.0, 0.7, [0.22, 0.46, 0.14, 1.0], seed);
    // A rope swing from the lowest limb
    let low = ends.iter().copied().min_by(|a, b| a.y.total_cmp(&b.y)).unwrap_or(top);
    let hang = base + (low - base) * 0.75;
    let hang = vec3(hang.x, low.y - 1.5, hang.z);
    let seat_y = y + 1.6;
    for s in [-0.9, 0.9] {
        let off = vec3(s, 0.0, 0.0);
        b.limb(hang + off, vec3(hang.x + s, seat_y, hang.z), 0.05, 0.05, 3, 0.0, [0.75, 0.68, 0.5, 1.0]);
    }
    b.block(vec3(hang.x, seat_y - 0.15, hang.z), 1.1, 0.25, 0.45, 0.0, WOOD, WOOD);
    // A bench under the tree, facing the view down the hill
    let bench = base + vec3(-7.0, 0.0, 3.0);
    let by = terrain_height(bench.x, bench.z);
    b.block(vec3(bench.x, by + 1.1, bench.z), 2.6, 0.3, 0.7, 0.4, WOOD, WOOD);
    for s in [-2.0_f32, 2.0] {
        let leg = bench + Quat::from_rotation_y(0.4) * vec3(s, 0.0, 0.0);
        b.block(vec3(leg.x, by, leg.z), 0.2, 1.1, 0.6, 0.4, OLD_WOOD, OLD_WOOD);
    }
    b.block(vec3(bench.x, by + 1.4, bench.z) + Quat::from_rotation_y(0.4) * vec3(0.0, 0.0, -0.65), 2.6, 1.2, 0.12, 0.4, WOOD, WOOD);
    // Initials carved in the bark (a heart, of course)
    let carve = base + vec3(2.6, 4.0, 2.6).normalize() * 3.2 + Vec3::Y * 4.0;
    b.quad(carve, carve + vec3(0.4, 0.5, -0.4), carve + Vec3::Y * 0.9, carve + vec3(-0.4, 0.5, 0.4), [0.55, 0.42, 0.3, 1.0]);
    // Lanterns hanging from every limb
    for (k, &e) in ends.iter().enumerate() {
        let at = base + (e - base) * (0.55 + 0.2 * hash(seed, k as f32));
        let lamp = vec3(at.x, at.y - 2.5, at.z);
        b.limb(lamp + Vec3::Y * 0.9, lamp + Vec3::Y * 2.5, 0.04, 0.04, 3, 0.0, [0.2, 0.2, 0.2, 1.0]);
        b.block(lamp + Vec3::Y * 0.8, 0.55, 0.15, 0.55, k as f32, [0.15, 0.15, 0.15, 1.0], [0.15, 0.15, 0.15, 1.0]);
        b.glowing = true;
        b.block(lamp, 0.45, 0.8, 0.45, k as f32, LANTERN, LANTERN);
        b.glowing = false;
    }
    // A string of little lights draped between two limbs
    if let (Some(&a), Some(&c)) = (ends.first(), ends.get(3)) {
        let (a, c) = (base + (a - base) * 0.5, base + (c - base) * 0.5);
        b.glowing = true;
        for k in 0..=16 {
            let t = k as f32 / 16.0;
            let sag = 4.0 * t * (1.0 - t) * 3.0;
            let q = a + (c - a) * t - Vec3::Y * sag;
            let col: Col = if k % 3 == 0 { [1.0, 0.55, 0.45, 1.0] } else { [1.0, 0.9, 0.6, 1.0] };
            b.block(q, 0.16, 0.3, 0.16, 0.0, col, col);
        }
        b.glowing = false;
    }
}

/// The ribs of a wrecked ship on the beach, heeled over, its bow in the water,
/// with a broken mast and the rags of a sail.
fn shipwreck(b: &mut Builder, p: Vec2, seed: f32) {
    let dir = toward_water(p);
    let yaw = yaw_toward(dir) + 0.4;
    let y = ground(p, 6.0) - 1.5;
    let heel = Quat::from_rotation_z(0.35) * Quat::from_rotation_x(0.08);
    let rot = Quat::from_rotation_y(yaw) * heel;
    let o = vec3(p.x, y, p.y);
    let w = |q: Vec3| o + rot * q;
    // Keel, bow (local +Z) toward the water
    b.limb(w(vec3(0.0, 0.0, -14.0)), w(vec3(0.0, 0.8, 15.0)), 0.6, 0.5, 4, 0.0, OLD_WOOD);
    b.limb(w(vec3(0.0, 0.8, 15.0)), w(vec3(0.0, 7.0, 17.5)), 0.5, 0.4, 4, 0.0, OLD_WOOD);
    // Ribs, curving up from the keel, narrower toward bow and stern; some broken
    for k in 0..11 {
        let z = -12.5 + k as f32 * 2.6;
        let f = 1.0 - (z / 15.0).powi(2);
        let beam = 5.5 * f + 0.8;
        for s in [-1.0_f32, 1.0] {
            let broken = hash(seed + s, k as f32) < 0.3;
            let pts = [vec3(0.0, 0.3, z), vec3(s * beam * 0.7, 1.2, z), vec3(s * beam, 3.5, z), vec3(s * beam * 0.95, 6.0, z)];
            let n = if broken { 2 } else { 3 };
            for j in 0..n {
                b.limb(w(pts[j]), w(pts[j + 1]), 0.3, 0.28, 4, 0.0, shade(OLD_WOOD, 0.9 + 0.15 * hash(k as f32, s)));
            }
            // A few planks still clinging on the low side
            if s < 0.0 && k > 1 && k < 9 && hash(seed, k as f32 + 20.0) < 0.6 {
                let z1 = z + 2.6;
                let beam1 = 5.5 * (1.0 - (z1 / 15.0).powi(2)) + 0.8;
                b.quad(w(vec3(s * beam * 0.7, 1.2, z)), w(vec3(s * beam1 * 0.7, 1.2, z1)), w(vec3(s * beam1, 3.3, z1)), w(vec3(s * beam, 3.3, z)), shade(OLD_WOOD, 0.8));
            }
        }
    }
    b.post(vec3(p.x, y, p.y), 6.0, 7.0);
    // The mast snapped off, its top lying across the sand with the sail
    b.limb(w(vec3(0.0, 0.5, 2.0)), w(vec3(0.0, 9.0, 2.0)), 0.45, 0.35, 6, 0.0, OLD_WOOD);
    let fallen = o + Quat::from_rotation_y(yaw) * vec3(-9.0, 0.0, 0.0);
    let fy = terrain_height(fallen.x, fallen.z) + 0.4;
    let along = Quat::from_rotation_y(yaw + 0.3) * Vec3::Z;
    let a = vec3(fallen.x, fy, fallen.z) - along * 8.0;
    let c = vec3(fallen.x, fy, fallen.z) + along * 8.0;
    b.limb(a, c, 0.35, 0.3, 6, 0.0, OLD_WOOD);
    let sail: Col = [0.85, 0.82, 0.72, 1.0];
    let side = Quat::from_rotation_y(yaw + 0.3) * Vec3::X;
    b.tri(a + along * 2.0, c - along * 3.0, a + along * 5.0 + side * 4.0 + Vec3::Y * 0.3, sail);
    b.tri(c - along * 3.0, c - along * 1.0 + side * 3.0 + Vec3::Y * 0.2, a + along * 5.0 + side * 4.0 + Vec3::Y * 0.3, shade(sail, 0.92));
    // An anchor half buried beside her
    let anchor = o + Quat::from_rotation_y(yaw) * vec3(7.0, 0.0, -6.0);
    let ay = terrain_height(anchor.x, anchor.z);
    let iron: Col = [0.3, 0.22, 0.18, 1.0];
    b.limb(vec3(anchor.x, ay - 0.5, anchor.z), vec3(anchor.x, ay + 3.5, anchor.z) + along * 0.8, 0.25, 0.2, 5, 0.0, iron);
    b.limb(vec3(anchor.x, ay + 3.0, anchor.z) - side * 1.2, vec3(anchor.x, ay + 3.0, anchor.z) + side * 1.2, 0.18, 0.18, 5, 0.0, iron);
}

/// Hot springs in the snow: three mineral-rimmed pools stepping down a slope,
/// turquoise water, and warm rocks round about (steam is added at run time).
fn hot_spring(b: &mut Builder, p: Vec2, seed: f32) {
    let rim: Col = [0.86, 0.72, 0.5, 1.0];
    let crust: Col = [0.78, 0.5, 0.28, 1.0];
    let pools = [(Vec2::ZERO, 7.0), (Vec2::new(9.0, 5.0), 5.0), (Vec2::new(14.0, -3.0), 4.0)];
    for &(off, r) in &pools {
        let c = p + Quat::from_rotation_y(seed).mul_vec3(vec3(off.x, 0.0, off.y)).truncate_xz();
        // Level water just above the highest ground it covers
        let y = (0..9).map(|j| {
            let a = j as f32 * std::f32::consts::TAU / 8.0;
            let q = if j == 8 { c } else { c + Vec2::new(a.cos(), a.sin()) * r };
            terrain_height(q.x, q.y)
        }).fold(f32::MIN, f32::max) + 0.4;
        let sides = 14;
        for j in 0..sides {
            let a0 = j as f32 * std::f32::consts::TAU / sides as f32;
            let a1 = (j + 1) as f32 * std::f32::consts::TAU / sides as f32;
            let e0 = vec3(c.x + a0.cos() * r, y, c.y + a0.sin() * r);
            let e1 = vec3(c.x + a1.cos() * r, y, c.y + a1.sin() * r);
            let mid = vec3(c.x, y, c.y);
            // Water surface
            b.tri(mid, e1, e0, [0.3, 0.78, 0.82, 1.0]);
            // Mineral rim, ochre crust spilling over the outside
            b.limb(e0 - Vec3::Y * 0.3, e1 - Vec3::Y * 0.3, 0.55, 0.55, 5, 0.0, rim);
            let (q0, q1) = (Vec2::new(c.x + a0.cos() * (r + 2.0), c.y + a0.sin() * (r + 2.0)), Vec2::new(c.x + a1.cos() * (r + 2.0), c.y + a1.sin() * (r + 2.0)));
            let o0 = vec3(q0.x, terrain_height(q0.x, q0.y) - 0.3, q0.y);
            let o1 = vec3(q1.x, terrain_height(q1.x, q1.y) - 0.3, q1.y);
            b.quad(e0 + Vec3::Y * 0.1, e1 + Vec3::Y * 0.1, o1, o0, if j % 2 == 0 { crust } else { shade(crust, 0.9) });
        }
    }
    // Dark warm rocks, bare of snow
    for k in 0..7 {
        let a = seed + k as f32 * 0.9;
        let q = p + Vec2::new(a.cos(), a.sin()) * (11.0 + 5.0 * hash(seed, k as f32));
        b.boulder(vec3(q.x, terrain_height(q.x, q.y) + 0.5, q.y), 1.2 + hash(k as f32, seed) * 1.8, a, STONE_DARK);
    }
}

trait TruncateXz {
    fn truncate_xz(self) -> Vec2;
}

impl TruncateXz for Vec3 {
    fn truncate_xz(self) -> Vec2 {
        Vec2::new(self.x, self.z)
    }
}
