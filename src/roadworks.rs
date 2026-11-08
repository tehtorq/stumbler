//! Road works: each day a handful of stretches of country road are dug
//! up, somewhere new every day. One lane is coned off behind red-and-white
//! barriers round a hole and its spoil heap; a digger works its arm, and
//! workers in hi-vis dig and lean on their shovels. Temporary traffic
//! lights at either end let one way through at a time, and the everyday
//! traffic waits its turn and squeezes by in the open lane.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::collisions::Solid;
use crate::geometry::Vertex;
use crate::props::{hash, PERSON_SCALE};
use crate::road::{self, PathKind, SAMPLE_SPACING};
use crate::townsfolk::{Mesh, Pose};

/// How long the coned-off stretch is (samples), and the lights' cycle.
const LENGTH: f32 = 26.0;
const CYCLE: f32 = 24.0;
/// Of the candidate stretches, about this share are dug up on a given day.
const SHARE: f32 = 0.2;
const DRAW: f32 = 600.0;

/// A stretch of road being worked on: the path, where along it the works
/// start and end (samples), which way's lane is closed (+1 or -1).
#[derive(Clone, Copy)]
pub struct Works {
    pub path: usize,
    pub from: f32,
    pub to: f32,
    pub closed: f32,
    seed: f32,
}

/// Straight, level stretches of country road away from the towns, where
/// works may be.
fn candidates() -> &'static [Works] {
    static ALL: OnceLock<Vec<Works>> = OnceLock::new();
    let Some(net) = road::network() else { return &[] };
    ALL.get_or_init(|| {
        let mut list: Vec<Works> = Vec::new();
        for (pi, path) in net.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
            let n = path.points.len();
            let mut i = 60;
            while i + 60 < n {
                let here = i;
                i += 350;
                let straight = path.tangent(here).dot(path.tangent(here + LENGTH as usize)) > 0.995;
                let p = path.points[here];
                let clear = !crate::towns::occupied(p, 150.0) && path.structure[here] == road::Structure::Ground
                    && net.paths.iter().filter(|q| !std::ptr::eq(*q, path)).all(|q| q.points.iter().step_by(6).all(|r| r.distance(p) > 140.0));
                if straight && clear {
                    let seed = hash(p.x, p.y) * 1000.0;
                    list.push(Works { path: pi, from: here as f32, to: here as f32 + LENGTH, closed: if hash(seed, 1.0) < 0.5 { 1.0 } else { -1.0 }, seed });
                }
            }
        }
        list
    })
}

/// The works in hand on `day`.
pub fn today(day: u32) -> Vec<Works> {
    candidates().iter().filter(|w| hash(w.seed, day as f32 * 7.31 + 3.0) < SHARE).copied().collect()
}

impl Works {
    /// Whether the lights let traffic going `way` through just now (each
    /// way gets green in turn, with an all-red gap to clear the stretch).
    pub fn green_for(&self, way: f32, time: f32) -> bool {
        let u = ((time + self.seed) / CYCLE).fract();
        if way > 0.0 { u < 0.4 } else { (0.5..0.9).contains(&u) }
    }

    /// Where along the path the lights for traffic going `way` stand.
    pub fn stop_line(&self, way: f32) -> f32 {
        if way > 0.0 { self.from - 6.0 } else { self.to + 6.0 }
    }
}

/// A point `along` the path (samples) and `out` to the left of its direction.
fn spot(net: &road::RoadNetwork, path: usize, along: f32, out: f32) -> (Vec3, Vec2) {
    let p = &net.paths[path];
    let n = p.points.len();
    let i = (along.max(0.0) as usize).min(n - 2);
    let f = (along - i as f32).clamp(0.0, 1.0);
    let c = p.points[i].lerp(p.points[i + 1], f);
    let t = p.tangent(i);
    let at = c + t.perp() * out;
    (vec3(at.x, p.heights[i].max(crate::terrain::terrain_height(at.x, at.y)), at.y), t)
}

/// The digger and barriers, for cars to bump into.
pub fn solids(day: u32, near: Vec3) -> Vec<(Solid, Vec3)> {
    let Some(net) = road::network() else { return Vec::new() };
    let mut list = Vec::new();
    for w in today(day) {
        let hw = net.paths[w.path].kind.half_width();
        let side = -w.closed;
        let (mid, t) = spot(net, w.path, (w.from + w.to) * 0.5, side * hw * 0.5);
        if mid.distance(near) > 400.0 {
            continue;
        }
        // The whole closed lane, behind its barriers
        let len = (w.to - w.from) * SAMPLE_SPACING * 0.5;
        list.push((Solid { centre: vec2(mid.x, mid.z), half: vec2(hw * 0.45, len), yaw: t.x.atan2(t.y), top: mid.y + 3.0, round: false }, Vec3::ZERO));
    }
    list
}

/// Everything at the works near the camera.
pub fn build_mesh(day: u32, camera: Vec3, time: f32) -> ((Vec<Vertex>, Vec<u16>), (Vec<Vertex>, Vec<u16>)) {
    let mut m = Mesh::default();
    let mut lamps = Mesh::default();
    let Some(net) = road::network() else { return ((m.verts, m.idxs), (lamps.verts, lamps.idxs)) };
    let k = PERSON_SCALE;
    for w in today(day) {
        let hw = net.paths[w.path].kind.half_width();
        // The closed lane is to the right of the way it carries: left of the path's own direction when that's -1
        let side = -w.closed;
        let (mid, t) = spot(net, w.path, (w.from + w.to) * 0.5, side * hw * 0.5);
        if mid.distance(camera) > DRAW {
            continue;
        }
        let f = vec3(t.x, 0.0, t.y);
        let r = vec3(-t.y, 0.0, t.x) * side;
        let orange = [0.98, 0.45, 0.08];
        let white = [0.95, 0.95, 0.92];
        // Cones along the lane's edge, tapering in at either end
        let mut a = w.from - 8.0;
        while a <= w.to + 8.0 {
            let taper = ((a - (w.from - 8.0)).min(w.to + 8.0 - a) / 8.0).clamp(0.0, 1.0);
            let (c, _) = spot(net, w.path, a, side * (hw - 2.0 - (hw * 0.95 - 2.0) * taper));
            m.box_at(c + Vec3::Y * 0.02 * k, r, f, vec3(0.2, 0.02, 0.2) * k, [0.1, 0.1, 0.1], [0.1, 0.1, 0.1]);
            m.limb(c, c + Vec3::Y * 0.7 * k, 0.12 * k, orange);
            m.box_at(c + Vec3::Y * 0.4 * k, r, f, vec3(0.1, 0.06, 0.1) * k, white, white);
            a += 2.0;
        }
        // Red-and-white barriers along the works, the hole and its heap
        let mut a = w.from;
        let mut stripe = 0;
        while a < w.to {
            let (c, _) = spot(net, w.path, a, side * 1.5);
            m.box_at(c + Vec3::Y * 0.6 * k, r, f, vec3(0.04, 0.12, 0.9) * k, if stripe % 2 == 0 { [0.85, 0.1, 0.08] } else { white }, white);
            for s in [-0.85, 0.85] {
                m.limb(c + f * s * k, c + f * s * k + Vec3::Y * 0.75 * k, 0.04 * k, [0.3, 0.3, 0.3]);
            }
            a += 2.0 * 0.95;
            stripe += 1;
        }
        let hole = mid + Vec3::Y * 0.05;
        m.quad(hole - f * 3.0 * k - r * 1.2 * k, hole + f * 3.0 * k - r * 1.2 * k, hole + f * 3.0 * k + r * 1.2 * k, hole - f * 3.0 * k + r * 1.2 * k, [0.15, 0.11, 0.08]);
        let heap = mid + f * 6.0 * k + r * 1.5 * k;
        for (o, s) in [(0.0, 1.0), (1.2, 0.7), (-1.0, 0.8)] {
            m.box_at(heap + f * o * k + Vec3::Y * 0.35 * s * k, r, f, vec3(0.9, 0.35, 1.0) * s * k, [0.45, 0.32, 0.2], [0.5, 0.36, 0.22]);
        }
        // The digger, working its arm
        digger(&mut m, mid - f * 7.0 * k + r * 0.4 * k, f, r, time + w.seed);
        // Workers: one digging in the hole, one leaning on a shovel, one watching
        m.workwear = true;
        let dig = (time * 2.2 + w.seed).sin();
        m.figure(hole - Vec3::Y * 0.3 + f * 1.0 * k, -t, dig * 0.6, Pose::Walking, w.seed);
        m.figure(hole + f * 3.6 * k + r * 1.6 * k, (r.truncate_y()).normalize_or_zero(), 0.0, Pose::Standing, w.seed + 1.0);
        m.figure(mid - f * 2.0 * k - r * 0.8 * k, -(r.truncate_y()).normalize_or_zero(), 0.0, Pose::Standing, w.seed + 2.0);
        m.workwear = false;
        let spade = hole + f * 1.5 * k;
        m.limb(spade + Vec3::Y * (0.2 + 0.2 * dig) * k, spade + Vec3::Y * (1.2 + 0.2 * dig) * k + f * 0.3 * k, 0.03 * k, [0.4, 0.3, 0.2]);
        // Signs, and the lights at each end
        for way in [1.0f32, -1.0] {
            let line = w.stop_line(way);
            let (light, _) = spot(net, w.path, line, -way * (hw + 2.0));
            let up = Vec3::Y * 2.6 * k;
            m.limb(light, light + up, 0.06 * k, [0.25, 0.25, 0.25]);
            m.box_at(light + up + Vec3::Y * 0.3 * k, r, f, vec3(0.18, 0.4, 0.18) * k, [0.1, 0.1, 0.1], [0.1, 0.1, 0.1]);
            let go = w.green_for(way, time);
            let face = f * -way * 0.19 * k;
            lamps.box_at(light + up + Vec3::Y * 0.5 * k + face, r, f, vec3(0.1, 0.1, 0.02) * k, if go { [0.25, 0.08, 0.06] } else { [1.0, 0.12, 0.08] }, [0.1, 0.1, 0.1]);
            lamps.box_at(light + up + Vec3::Y * 0.1 * k + face, r, f, vec3(0.1, 0.1, 0.02) * k, if go { [0.15, 1.0, 0.35] } else { [0.05, 0.2, 0.08] }, [0.1, 0.1, 0.1]);
            // A "road works" triangle further back
            let (sign, _) = spot(net, w.path, line - way * 15.0, -way * (hw + 3.0));
            m.limb(sign, sign + Vec3::Y * 1.6 * k, 0.04 * k, [0.4, 0.4, 0.42]);
            let top = sign + Vec3::Y * 2.5 * k;
            let s = r * 0.55 * k;
            m.tri(top - s - Vec3::Y * 0.9 * k, top + s - Vec3::Y * 0.9 * k, top, [0.85, 0.1, 0.08]);
            m.tri(top - s * 0.7 - Vec3::Y * 0.75 * k, top + s * 0.7 - Vec3::Y * 0.75 * k, top - Vec3::Y * 0.2 * k, white);
        }
    }
    ((m.verts, m.idxs), (lamps.verts, lamps.idxs))
}

trait Flat {
    fn truncate_y(self) -> Vec2;
}

impl Flat for Vec3 {
    fn truncate_y(self) -> Vec2 {
        vec2(self.x, self.z)
    }
}

/// A yellow tracked digger, its arm swinging down to scoop and up again.
fn digger(m: &mut Mesh, at: Vec3, f: Vec3, r: Vec3, time: f32) {
    let k = PERSON_SCALE;
    let yellow = [0.95, 0.7, 0.1];
    let dark = [0.15, 0.15, 0.15];
    for s in [-1.0, 1.0] {
        m.box_at(at + r * s * 0.9 * k + Vec3::Y * 0.4 * k, r, f, vec3(0.3, 0.4, 1.6) * k, dark, dark);
    }
    m.box_at(at + Vec3::Y * 1.2 * k, r, f, vec3(1.1, 0.4, 1.3) * k, yellow, yellow);
    m.box_at(at + Vec3::Y * 2.1 * k - r * 0.4 * k - f * 0.2 * k, r, f, vec3(0.5, 0.55, 0.55) * k, [0.3, 0.42, 0.5], yellow);
    // The boom and dipper, rising and falling
    let swing = (time * 0.5).sin();
    let shoulder = at + Vec3::Y * 1.6 * k + f * 0.9 * k + r * 0.3 * k;
    let elbow = shoulder + f * 2.2 * k + Vec3::Y * (1.6 + 0.6 * swing) * k;
    let bucket = elbow + f * 1.2 * k - Vec3::Y * (2.0 + 0.6 * swing) * k;
    m.limb(shoulder, elbow, 0.2 * k, yellow);
    m.limb(elbow, bucket, 0.15 * k, yellow);
    m.box_at(bucket - Vec3::Y * 0.2 * k, r, f, vec3(0.45, 0.25, 0.3) * k, dark, dark);
}
