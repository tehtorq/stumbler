//! Breakdowns: now and then a car gives up on a country road and sits on
//! the verge, bonnet up and hazards going, a warning triangle out on the
//! road behind and its driver waiting beside it (often on the phone).
//! Later on the recovery truck turns up, its beacon going round. Which
//! cars break down, where and when, is new each day.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::collisions::Solid;
use crate::geometry::Vertex;
use crate::props::{hash, PERSON_SCALE};
use crate::road::{self, PathKind};
use crate::traffic::{vehicle, Kind};
use crate::townsfolk::{Mesh, Pose};

/// Of the candidate spots, about this share see a breakdown on a given day.
const SHARE: f32 = 0.04;
const DRAW: f32 = 600.0;

/// A spot on a verge: path, sample, which side (+1 left of the path's way).
#[derive(Clone, Copy)]
struct Spot {
    path: usize,
    at: usize,
    side: f32,
    seed: f32,
}

fn spots() -> &'static [Spot] {
    static ALL: OnceLock<Vec<Spot>> = OnceLock::new();
    let Some(net) = road::network() else { return &[] };
    ALL.get_or_init(|| {
        let mut list = Vec::new();
        for (pi, path) in net.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
            let n = path.points.len();
            let mut i = 120;
            while i + 40 < n {
                let p = path.points[i];
                // Not up on an embankment, behind its guard rail
                let level = (i.saturating_sub(4)..i + 4).all(|j| path.heights[j] - crate::terrain::raw_height(path.points[j].x, path.points[j].y) < 2.5);
                if level && !crate::towns::occupied(p, 120.0) && path.structure[i] == road::Structure::Ground {
                    let seed = hash(p.y, p.x) * 1000.0;
                    list.push(Spot { path: pi, at: i, side: if hash(seed, 1.0) < 0.5 { 1.0 } else { -1.0 }, seed });
                }
                i += 280;
            }
        }
        list
    })
}

/// The breakdowns on now (`day`, `clock` 0..1 from midnight): each spot,
/// and how far through its wait it is (0 just stopped .. 1 towed away).
fn now(day: u32, clock: f32) -> Vec<(Spot, f32)> {
    let h = clock * 24.0;
    spots().iter().filter_map(|s| {
        let roll = |k: f32| hash(s.seed, day as f32 * 5.17 + k);
        if roll(2.0) > SHARE {
            return None;
        }
        let start = 7.0 + 10.0 * roll(3.0);
        let wait = 1.5 + 2.5 * roll(4.0);
        let u = (h - start) / wait;
        (0.0..1.0).contains(&u).then_some((*s, u))
    }).collect()
}

/// Where the broken-down car stands and faces (with the traffic's flow on its side).
fn place(net: &road::RoadNetwork, s: &Spot) -> (Vec3, Vec2) {
    let path = &net.paths[s.path];
    let t = path.tangent(s.at);
    let dir = if s.side > 0.0 { -t } else { t };
    let hw = path.kind.half_width();
    let p = path.points[s.at] + t.perp() * s.side * (hw + 3.5);
    (vec3(p.x, crate::terrain::terrain_height(p.x, p.y).max(path.heights[s.at] - 0.5), p.y), dir)
}

/// The broken-down cars (and tow trucks), for cars to bump into.
pub fn solids(day: u32, clock: f32, near: Vec3) -> Vec<(Solid, Vec3)> {
    let Some(net) = road::network() else { return Vec::new() };
    now(day, clock).into_iter().filter_map(|(s, _)| {
        let (p, dir) = place(net, &s);
        (p.distance(near) < 300.0).then(|| (Solid { centre: vec2(p.x, p.z), half: vec2(0.9, 2.15) * PERSON_SCALE, yaw: dir.x.atan2(dir.y), top: p.y + 3.0, round: false }, Vec3::ZERO))
    }).collect()
}

/// Everything at the breakdowns near the camera, and the lamps to glow.
pub fn build_mesh(day: u32, clock: f32, camera: Vec3, time: f32, night: f32) -> ((Vec<Vertex>, Vec<u16>), (Vec<Vertex>, Vec<u16>)) {
    let mut m = Mesh::default();
    let mut lamps = Mesh::default();
    let Some(net) = road::network() else { return ((m.verts, m.idxs), (lamps.verts, lamps.idxs)) };
    let k = PERSON_SCALE;
    for (s, u) in now(day, clock) {
        let (p, dir) = place(net, &s);
        if p.distance(camera) > DRAW {
            continue;
        }
        let blink = (time * 1.6).fract() < 0.5;
        vehicle(&mut m, &mut lamps, p, dir, Kind::Car, s.seed, false, night > 0.3, blink);
        let f = vec3(dir.x, 0.0, dir.y);
        let r = vec3(dir.y, 0.0, -dir.x);
        let at = |a: f32, up: f32, c: f32| p + (f * a + Vec3::Y * up + r * c) * k;
        // The bonnet propped up
        let (hinge, front) = (1.1, 2.1);
        let paint = [[0.75, 0.1, 0.1], [0.12, 0.25, 0.55], [0.85, 0.85, 0.82], [0.15, 0.15, 0.16], [0.55, 0.57, 0.6], [0.2, 0.42, 0.28], [0.85, 0.62, 0.2], [0.45, 0.2, 0.35]][(hash(s.seed, 1.0) * 8.0) as usize % 8];
        let (h0, h1, f0, f1) = (at(hinge, 0.96, -0.85), at(hinge, 0.96, 0.85), at(front - 0.3, 1.75, -0.85), at(front - 0.3, 1.75, 0.85));
        m.quad(h0, h1, f1, f0, paint);
        m.quad(h1, h0, f0, f1, [0.2, 0.2, 0.2]);
        m.limb(at(front - 0.2, 0.95, 0.6), at(front - 0.32, 1.72, 0.6), 0.02 * k, [0.6, 0.6, 0.6]);
        // The driver: peering under the bonnet at first, then waiting on the phone
        let looking = u < 0.3;
        let driver = if looking { at(front + 0.7, 0.0, 0.2) } else { at(-0.5, 0.0, 2.1) };
        let facing = if looking { -dir } else { dir.perp() * -1.0 };
        m.figure(driver, facing, 0.0, Pose::Standing, s.seed + 4.0);
        if !looking {
            // A phone held to the ear
            let ear = driver + Vec3::Y * 1.62 * k + vec3(facing.y, 0.0, -facing.x) * 0.17 * k;
            m.box_at(ear, r, f, vec3(0.03, 0.07, 0.04) * k, [0.1, 0.1, 0.12], [0.1, 0.1, 0.12]);
        }
        // The warning triangle out behind, on the road's edge
        let tri = at(-14.0, 0.0, -2.4);
        let s2 = r * 0.35 * k;
        m.tri(tri - s2, tri + s2, tri + Vec3::Y * 0.6 * k, [0.85, 0.1, 0.08]);
        m.tri(tri + s2, tri - s2, tri + Vec3::Y * 0.6 * k, [0.85, 0.1, 0.08]);
        // The recovery truck, once it's come, beacon going round
        if u > 0.65 {
            let truck = at(7.5, 0.0, 0.0);
            vehicle(&mut m, &mut lamps, truck, dir, Kind::Van, s.seed * 3.0, false, night > 0.3, false);
            let tf = |a: f32, up: f32| truck + (f * a + Vec3::Y * up) * k;
            m.box_at(tf(-1.2, 1.3), r, f, vec3(1.0, 0.08, 1.4) * k, [0.95, 0.75, 0.1], [0.95, 0.75, 0.1]);
            let spin = (time * 5.0).sin() > 0.0;
            let beacon = if spin { [1.0, 0.65, 0.1] } else { [0.5, 0.3, 0.05] };
            lamps.box_at(tf(1.2, 2.55), r, f, vec3(0.6, 0.08, 0.12) * k, beacon, beacon);
            // The recovery driver having a look
            m.workwear = true;
            m.figure(at(front + 1.2, 0.0, -1.0), (-dir + dir.perp() * 0.5).normalize_or_zero(), 0.0, Pose::Standing, s.seed + 9.0);
            m.workwear = false;
        }
    }
    ((m.verts, m.idxs), (lamps.verts, lamps.idxs))
}

/// Where the breakdowns are at `clock` on `day` (for tests and visits).
#[cfg(test)]
pub fn report(day: u32, clock: f32) -> Vec<(Vec3, Vec2, f32)> {
    let net = road::network().unwrap();
    now(day, clock).into_iter().map(|(s, u)| { let (p, d) = place(net, &s); (p, d, u) }).collect()
}
