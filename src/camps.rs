//! Campsites out in the wilds, a little way off the trails: tents round a
//! ring of stones, logs to sit on, a woodpile, now and then a camper van.
//! The fire's lit in the evening and again for breakfast, crackling, its
//! smoke going up and its light flickering over the camp; at night the
//! campers sit round it, by day they potter about the tents.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::lights::Light;
use crate::particles::Particles;
use crate::props::{hash, Builder, PERSON_SCALE};
use crate::road::{self, PathKind};
use crate::terrain::{self, terrain_height, WATER_LEVEL};
use crate::townsfolk::{Mesh, Pose};

const MAX_CAMPS: usize = 30;
const SPACING: f32 = 1500.0;
/// Out from the trail.
const OFF_TRAIL: f32 = 40.0;
/// The ring of logs round the fire, and where the tents pitch (metres).
const LOGS: f32 = 2.4;
const TENTS: f32 = 6.0;
/// Fires burn and campers show inside these distances.
const NEAR: f32 = 400.0;

pub struct Camp {
    pub centre: Vec2,
    /// Toward the trail.
    pub facing: Vec2,
    pub seed: f32,
}

/// Every campsite (none until the roads and trails are in).
pub fn all() -> &'static [Camp] {
    static CAMPS: OnceLock<Vec<Camp>> = OnceLock::new();
    match road::network() {
        Some(n) => CAMPS.get_or_init(|| site(n)),
        None => &[],
    }
}

fn site(net: &road::RoadNetwork) -> Vec<Camp> {
    let mut camps: Vec<Camp> = Vec::new();
    let flat = |q: Vec2| {
        let h = terrain_height(q.x, q.y);
        (0..6).all(|k| { let d = Vec2::from_angle(k as f32 * 1.05) * 18.0; (terrain_height(q.x + d.x, q.y + d.y) - h).abs() < 2.5 })
    };
    for path in net.paths.iter().filter(|p| p.kind == PathKind::Trail) {
        let n = path.points.len();
        for i in (n / 4..n * 3 / 4).step_by(40) {
            if camps.len() >= MAX_CAMPS {
                return camps;
            }
            let t = path.tangent(i);
            let side = if hash(path.points[i].x, path.points[i].y) < 0.5 { 1.0 } else { -1.0 };
            let c = path.points[i] + t.perp() * side * OFF_TRAIL;
            let ok = terrain::raw_height(c.x, c.y) > WATER_LEVEL + 2.0 && terrain_height(c.x, c.y) < 150.0 && flat(c)
                && terrain::biome_desert(c.x, c.y) < 0.4 && terrain::biome_tundra(c.x, c.y) < 0.4
                && net.nearest(c.x, c.y).is_none_or(|s| s.dist > s.half_width() + 22.0)
                && camps.iter().all(|o| o.centre.distance(c) > SPACING)
                && crate::landmarks::all().iter().all(|l| l.pos.distance(c) > 160.0)
                && !crate::towns::occupied(c, 60.0) && crate::farms::at(c).is_none();
            if ok {
                camps.push(Camp { centre: c, facing: -t.perp() * side, seed: hash(c.x, c.y) * 1000.0 });
            }
        }
    }
    camps
}

fn ground(p: Vec2) -> Vec3 {
    vec3(p.x, terrain_height(p.x, p.y), p.y)
}

/// The log seats round the fire: where each sits and which way along it runs.
fn logs(camp: &Camp) -> Vec<(Vec2, Vec2)> {
    (0..3).map(|k| {
        let a = camp.seed + k as f32 * 2.09;
        let out = Vec2::from_angle(a);
        (camp.centre + out * LOGS * PERSON_SCALE, out.perp())
    }).collect()
}

/// The tents, fire ring, logs and woodpile for the chunk.
pub fn build(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    b.tag("campsites");
    let k = PERSON_SCALE;
    for camp in all().iter().filter(|c| inside(c.centre)) {
        let r = |n: f32| hash(camp.seed, n);
        // The ring of stones, and the charred logs in it
        let c = ground(camp.centre);
        for s in 0..9 {
            let a = s as f32 / 9.0 * std::f32::consts::TAU;
            b.boulder(c + vec3(a.cos(), 0.08, a.sin()) * 0.55 * k, 0.18 * k, camp.seed + s as f32, [0.45, 0.44, 0.42, 1.0]);
        }
        for s in 0..3 {
            let a = s as f32 * 2.1 + camp.seed;
            let d = vec3(a.cos(), 0.0, a.sin()) * 0.35 * k;
            b.limb(c - d + Vec3::Y * 0.1, c + d + Vec3::Y * 0.25 * k, 0.06 * k, 0.06 * k, 5, 0.0, [0.12, 0.1, 0.08, 1.0]);
        }
        // Logs to sit on
        for (at, along) in logs(camp) {
            let g = ground(at) + Vec3::Y * 0.3 * k;
            b.limb(g - vec3(along.x, 0.0, along.y) * 0.9 * k, g + vec3(along.x, 0.0, along.y) * 0.9 * k, 0.3 * k, 0.28 * k, 6, 0.3, [0.42, 0.3, 0.2, 1.0]);
        }
        // Two or three tents round about, doors toward the fire
        const CANVAS: [[f32; 4]; 5] = [[0.9, 0.45, 0.1, 1.0], [0.2, 0.45, 0.25, 1.0], [0.2, 0.35, 0.7, 1.0], [0.85, 0.75, 0.2, 1.0], [0.6, 0.15, 0.15, 1.0]];
        let tents = 2 + (r(1.0) * 2.0) as usize;
        for t in 0..tents {
            let a = camp.seed + 1.0 + t as f32 * 2.4;
            let out = Vec2::from_angle(a);
            let at = camp.centre + out * TENTS * k;
            let colour = CANVAS[(r(2.0 + t as f32) * 5.0) as usize % 5];
            tent(b, ground(at), -out, colour);
        }
        // A woodpile, and sometimes a camper van parked up
        let pile = camp.centre + camp.facing.perp() * 4.0 * k;
        b.block(ground(pile), 0.6 * k, 0.5 * k, 0.4 * k, camp.facing.x.atan2(camp.facing.y), [0.5, 0.36, 0.22, 1.0], [0.62, 0.48, 0.32, 1.0]);
        if r(9.0) < 0.5 {
            let van = camp.centre + camp.facing * 10.0 * k;
            let yaw = camp.facing.perp().x.atan2(camp.facing.perp().y);
            let paint = [[0.85, 0.82, 0.7, 1.0], [0.3, 0.55, 0.6, 1.0], [0.9, 0.6, 0.2, 1.0]][(r(10.0) * 3.0) as usize % 3];
            let g = ground(van);
            b.solid_block(g + Vec3::Y * 0.3 * k, 1.0 * k, 2.2 * k, 2.6 * k, yaw, paint, [0.95, 0.95, 0.92, 1.0]);
            for s in [-1.0, 1.0] {
                for e in [-1.0, 1.0] {
                    let w = van + camp.facing.perp() * e * 1.6 * k + camp.facing * s * 0.95 * k;
                    b.column(ground(w), 0.34 * k, 0.34 * k, 0.6 * k, 8, [0.1, 0.1, 0.1, 1.0]);
                }
            }
        }
    }
}

/// A ridge tent pitched at `at`, its door facing `door`.
fn tent(b: &mut Builder, at: Vec3, door: Vec2, colour: [f32; 4]) {
    let k = PERSON_SCALE;
    let f = vec3(door.x, 0.0, door.y);
    let s = vec3(door.y, 0.0, -door.x);
    let (half_len, half_w, height) = (1.2 * k, 0.9 * k, 1.1 * k);
    let ridge = [at + Vec3::Y * height + f * half_len, at + Vec3::Y * height - f * half_len];
    let base = |e: f32, w: f32| at + f * e * half_len + s * w * half_w;
    let dark = [colour[0] * 0.75, colour[1] * 0.75, colour[2] * 0.75, 1.0];
    b.quad(base(1.0, 1.0), base(-1.0, 1.0), ridge[1], ridge[0], colour);
    b.quad(base(-1.0, -1.0), base(1.0, -1.0), ridge[0], ridge[1], dark);
    // The back closed, the front with its door open a crack
    b.tri(base(-1.0, 1.0), base(-1.0, -1.0), ridge[1], dark);
    b.tri(base(1.0, -1.0), base(1.0, 1.0), ridge[0], colour);
    let door_c = at + f * (half_len + 0.01);
    b.tri(door_c + s * 0.25 * k, door_c - s * 0.25 * k, door_c + Vec3::Y * 0.8 * k, [0.1, 0.08, 0.06, 1.0]);
}

/// Whether the fire's going at `clock` (0..1 from midnight): evenings into
/// the night, and for breakfast.
fn fire_lit(clock: f32) -> bool {
    let h = clock * 24.0;
    !(1.5..6.5).contains(&h) && !(9.5..18.0).contains(&h)
}

#[derive(Default)]
pub struct Camps;

impl Camps {
    /// Flames, embers and smoke from the fires near the camera.
    pub fn update(&self, particles: &mut Particles, camera: Vec3, clock: f32, rain: f32, dt: f32) {
        if !fire_lit(clock) || rain > 0.5 {
            return;
        }
        let cam = vec2(camera.x, camera.z);
        for camp in all().iter().filter(|c| c.centre.distance(cam) < NEAR) {
            let c = ground(camp.centre);
            particles.campfire(c + Vec3::Y * 0.2, 14.0, dt);
            if hash(camp.seed, (clock * 3000.0).floor()) < dt * 2.5 {
                particles.house_smoke(c + Vec3::Y * 1.5, vec3(1.0, 0.0, 0.8), 0.6);
            }
        }
    }

    /// The warm light of the nearest fire going at night, flickering.
    pub fn fire_light(&self, camera: Vec3, clock: f32, night: f32, time: f32) -> Option<Light> {
        if !fire_lit(clock) || night < 0.2 {
            return None;
        }
        let cam = vec2(camera.x, camera.z);
        let camp = all().iter().filter(|c| c.centre.distance(cam) < 250.0).min_by(|a, b| a.centre.distance(cam).total_cmp(&b.centre.distance(cam)))?;
        let flicker = 0.8 + 0.12 * (time * 13.0).sin() + 0.08 * (time * 29.0 + 1.3).sin();
        Some(Light { pos: ground(camp.centre) + Vec3::Y * 2.0, radius: 32.0, dir: Vec3::NEG_Y, cos_half_angle: 0.0, color: vec3(1.0, 0.6, 0.25), intensity: 2.6 * night * flicker })
    }

    /// Where the fires near `at` are going (for their crackle and the voices).
    pub fn fires_near(&self, at: Vec3, clock: f32, reach: f32) -> Vec<Vec3> {
        if !fire_lit(clock) {
            return Vec::new();
        }
        let p = vec2(at.x, at.z);
        all().iter().filter(|c| c.centre.distance(p) < reach).map(|c| ground(c.centre)).collect()
    }

    /// The campers: round the fire when it's lit, about the tents otherwise.
    pub fn build_mesh(&self, camera: Vec3, clock: f32, rain: f32, time: f32) -> (Vec<Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let mut m = Mesh::default();
        let k = PERSON_SCALE;
        let h = clock * 24.0;
        for camp in all().iter().filter(|c| c.centre.distance(cam) < NEAR) {
            let r = |n: f32| hash(camp.seed, n);
            let campers = 2 + (r(20.0) * 3.0) as usize;
            if rain > 0.5 {
                // Everyone in the tents
                continue;
            }
            if fire_lit(clock) {
                // Sat on the logs, facing the fire
                let seats = logs(camp);
                for p in 0..campers {
                    let (log, along) = seats[p % seats.len()];
                    let at = log + along * (if p / seats.len() == 0 { -0.4 } else { 0.5 }) * k;
                    let facing = (camp.centre - at).normalize_or_zero();
                    m.figure(ground(at) - Vec3::Y * 0.02, facing, 0.0, Pose::Sitting, camp.seed + p as f32);
                }
            } else if (7.0..21.0).contains(&h) {
                // Pottering about the camp
                for p in 0..campers.min(3) {
                    let a = camp.seed + p as f32 * 2.0 + (time * 0.05 + r(p as f32)).sin() * 0.6;
                    let at = camp.centre + Vec2::from_angle(a) * (3.5 + 1.5 * r(30.0 + p as f32)) * k;
                    let facing = Vec2::from_angle(a + 1.8);
                    let pose = if p == 0 { Pose::OnGround } else { Pose::Standing };
                    m.figure(ground(at), facing, 0.0, pose, camp.seed + p as f32);
                }
            }
        }
        (m.verts, m.idxs)
    }
}
