//! Wood smoke from the chimneys: town houses, farmhouses and the cabins in
//! the woods. Most homes have the fire lit morning and evening, fewer
//! through the day, a few banked overnight; in the cold north the smoke
//! is thicker and every hearth is going, and out in the desert there's
//! none. It drifts off with the wind.

use std::sync::OnceLock;

use glam::{Quat, Vec3, vec3};

use crate::landmarks::{self, Kind};
use crate::particles::Particles;
use crate::props::{ground, hash, BUILDING_TALL};
use crate::terrain;

/// Only chimneys this near the camera smoke (the particles are few).
const NEAR: f32 = 450.0;
/// At most this many at once, the nearest.
const MOST: usize = 36;
/// Which way the wind carries it, and how hard.
const DRIFT: Vec3 = vec3(1.6, 0.0, 1.2);

/// Every chimney top, with a seed for the household.
fn chimneys() -> &'static [(Vec3, f32)] {
    static ALL: OnceLock<Vec<(Vec3, f32)>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut list = crate::towns::chimneys();
        list.extend(crate::farms::chimneys());
        // Cabins, as `props` builds them
        for l in landmarks::all().iter().filter(|l| l.kind == Kind::Cabin) {
            let yaw = hash(l.pos.x, l.pos.y) * std::f32::consts::TAU;
            let base = vec3(l.pos.x, ground(l.pos, 8.0) - 0.5, l.pos.y);
            let top = base + Quat::from_rotation_y(yaw) * vec3(3.0, 0.0, 4.0) + Vec3::Y * 12.5 * BUILDING_TALL;
            list.push((top, l.seed as f32 % 1000.0));
        }
        list
    })
}

/// How many homes have a fire going at `clock` (0..1 from midnight), and
/// how hard it's burning.
fn hearths(clock: f32) -> (f32, f32) {
    match clock * 24.0 {
        h if h < 5.0 => (0.25, 0.3),
        h if h < 9.5 => (0.75, 0.9),
        h if h < 16.0 => (0.3, 0.4),
        h if h < 23.0 => (0.85, 1.0),
        _ => (0.5, 0.6),
    }
}

/// Puffs from the chimneys near `camera` for this frame.
#[derive(Default)]
pub struct Smoke {
    /// Per chimney, the part puff carried over from the last frame.
    owed: Vec<f32>,
}

impl Smoke {
    pub fn update(&mut self, particles: &mut Particles, camera: Vec3, clock: f32, dt: f32) {
        let all = chimneys();
        self.owed.resize(all.len(), 0.0);
        let (share, burn) = hearths(clock);
        let mut near: Vec<(usize, f32)> = all.iter().enumerate()
            .map(|(i, (p, _))| (i, p.distance_squared(camera)))
            .filter(|&(_, d)| d < NEAR * NEAR)
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        for &(i, _) in near.iter().take(MOST) {
            let (top, seed) = all[i];
            let (x, z) = (top.x, top.z);
            let cold = terrain::biome_tundra(x, z) + terrain::snow(x, z) * 0.5 + terrain::biome_mountain(x, z) * 0.3;
            if terrain::biome_desert(x, z) > 0.5 {
                continue;
            }
            // Whether this household has the fire lit (everyone does in the cold)
            if hash(seed, 31.0) > (share + cold).min(1.0) {
                continue;
            }
            let thick = (burn * (0.6 + 0.4 * hash(seed, 32.0)) + cold * 0.5).min(1.0);
            self.owed[i] += dt * (0.5 + 1.0 * thick);
            while self.owed[i] >= 1.0 {
                self.owed[i] -= 1.0;
                particles.house_smoke(top + Vec3::Y * 0.3, DRIFT * (0.6 + 0.4 * hash(seed, 33.0)), thick);
            }
        }
    }
}
