//! Things along the connecting roads: telegraph poles strung with sagging
//! wires, a signpost at each end of every road naming the circuit it leads to
//! (and, on the back, the one you're arriving at), and a wooden fingerpost
//! wherever a trail turns off toward a landmark.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec3};

use crate::hud::{glyph, GLYPH_H, GLYPH_W};
use crate::landmarks;
use crate::props::{ground, hash, Builder};
use crate::road::{PathKind, RoadNetwork, RoadPath, SAMPLE_SPACING, TRAIL_HALF_WIDTH};
use crate::terrain::{self, terrain_height, WATER_LEVEL};

const POLE_WOOD: [f32; 4] = [0.36, 0.27, 0.2, 1.0];
const WIRE: [f32; 4] = [0.12, 0.12, 0.13, 1.0];
const INSULATOR: [f32; 4] = [0.85, 0.88, 0.9, 1.0];
const SIGN_GREEN: [f32; 4] = [0.08, 0.36, 0.2, 1.0];
const SIGN_WHITE: [f32; 4] = [0.95, 0.95, 0.92, 1.0];
const SIGN_POST: [f32; 4] = [0.6, 0.62, 0.64, 1.0];
const FINGER_ARM: [f32; 4] = [0.9, 0.85, 0.7, 1.0];
const FINGER_TEXT: [f32; 4] = [0.25, 0.15, 0.08, 1.0];
const FINGER_POST: [f32; 4] = [0.42, 0.3, 0.2, 1.0];

/// Samples between telegraph poles (about 48 units).
const POLE_EVERY: usize = 12;
/// Poles stand this far from the road's centreline.
const POLE_OFFSET: f32 = crate::road::ROAD_HALF_WIDTH + 10.0;
const POLE_HEIGHT: f32 = 9.5;
/// Half the spread of the two wires on each crossarm.
const ARM_SPREAD: f32 = 1.3;
/// Road signs stand this far along the road from the circuit it leaves.
const SIGN_IN: f32 = 110.0;
/// Widest a road sign's lettering gets.
const SIGN_TEXT_WIDTH: f32 = 28.0;

enum Kind {
    /// Two-sided green road sign: `front` faces traffic leaving the circuit
    /// toward the far end, `back` faces traffic arriving.
    Road { front: [String; 2], back: [String; 2] },
    /// Wooden fingerpost pointing along a trail.
    Finger { name: String },
}

struct Sign {
    pos: Vec2,
    /// Road signs: the way the front faces. Fingerposts: the way they point.
    dir: Vec2,
    kind: Kind,
}

static SIGNS: OnceLock<Vec<Sign>> = OnceLock::new();

/// Site the signs. `circuit_names` are in the same order as the network's
/// circuits; landmarks must already be installed. Later calls are ignored.
pub fn install(circuit_names: &[&'static str], network: &RoadNetwork) {
    let _ = SIGNS.set(site_signs(circuit_names, network));
}

fn site_signs(circuit_names: &[&'static str], network: &RoadNetwork) -> Vec<Sign> {
    let circuits: Vec<&RoadPath> = network.paths.iter().filter(|p| p.circuit()).collect();
    let circuit_at = |p: Vec2| {
        circuits.iter().enumerate()
            .map(|(i, c)| (i, c.points.iter().map(|q| q.distance_squared(p)).fold(f32::MAX, f32::min)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| circuit_names.get(i).copied().unwrap_or("CIRCUIT"))
    };

    let mut signs = Vec::new();
    for road in network.paths.iter().filter(|p| p.kind == PathKind::Road) {
        let n = road.points.len();
        let (Some(a), Some(b)) = (circuit_at(road.points[0]), circuit_at(road.points[n - 1])) else { continue };
        let km = format!("{:.1} KM", (n - 1) as f32 * SAMPLE_SPACING / 1000.0);
        let k = ((SIGN_IN / SAMPLE_SPACING) as usize).min(n / 3);
        // On the right of traffic leaving each end, facing it, set back far
        // enough that the whole board clears the road (and any other path)
        for (leaving, near, far) in [(1.0, a, b), (-1.0, b, a)] {
            let front = [far.to_string(), km.clone()];
            let back = [near.to_string(), "CIRCUIT".to_string()];
            let half_w = board(&front, &back).1;
            let spot = (k..n / 3).step_by(5).find_map(|step| {
                let i = if leaving > 0.0 { step } else { n - 1 - step };
                let ahead = road.tangent(i) * leaving;
                // The driver's right hand side
                let right = Vec2::new(ahead.y, -ahead.x);
                let pos = road.points[i] + right * (road.kind.half_width() + 3.0 + half_w);
                let clear = [-1.0, 0.0, 1.0].iter().all(|&s| {
                    let q = pos + right * s * half_w;
                    network.nearest(q.x, q.y).is_none_or(|p| p.dist > p.half_width() + 2.0)
                });
                clear.then_some((pos, -ahead))
            });
            if let Some((pos, dir)) = spot {
                signs.push(Sign { pos, dir, kind: Kind::Road { front, back } });
            }
        }
    }

    for trail in network.paths.iter().filter(|p| p.kind == PathKind::Trail) {
        let end = *trail.points.last().unwrap();
        let Some(landmark) = landmarks::all().iter()
            .filter(|l| !l.kind.is_secret())
            .min_by(|a, b| a.pos.distance_squared(end).total_cmp(&b.pos.distance_squared(end)))
            .filter(|l| l.pos.distance(end) < 120.0)
        else { continue };
        // First spot clear of whatever the trail turns off from
        let clear = |p: Vec2| {
            network.nearest_matching(p.x, p.y, |k| k != PathKind::Trail)
                .is_none_or(|s| s.dist > s.half_width() + 10.0)
        };
        let Some(i) = (0..trail.points.len() / 2).find(|&i| clear(trail.points[i])) else { continue };
        let dir = trail.tangent(i);
        let side = Vec2::new(-dir.y, dir.x);
        let pos = [1.0, -1.0].iter()
            .map(|&s| trail.points[i] + side * s * (TRAIL_HALF_WIDTH + 3.0))
            .find(|&p| {
                network.nearest(p.x, p.y).is_none_or(|s| s.dist > s.half_width() + 2.0)
                    && clear(p) && terrain::raw_height(p.x, p.y) > WATER_LEVEL + 0.5
            });
        if let Some(pos) = pos {
            signs.push(Sign { pos, dir, kind: Kind::Finger { name: landmark.name.to_uppercase() } });
        }
    }
    signs
}

/// Poles, wires and signs anchored in the chunk `inside` accepts.
pub fn build(b: &mut Builder, network: &RoadNetwork, inside: &impl Fn(Vec2) -> bool) {
    for (r, road) in network.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
        let n = road.points.len();
        for i in (POLE_EVERY..n - POLE_EVERY).step_by(POLE_EVERY) {
            if !inside(road.points[i]) {
                continue;
            }
            let Some((base, across)) = pole_at(network, road, r, i) else { continue };
            telegraph_pole(b, base, across);
            if let Some((next, next_across)) = pole_at(network, road, r, i + POLE_EVERY) {
                for s in [-1.0, 1.0] {
                    let top = |p: Vec3, a: Vec3| p + Vec3::Y * (POLE_HEIGHT - 0.55) + a * s * ARM_SPREAD;
                    wire(b, top(base, across), top(next, next_across));
                }
            }
        }
    }
    for sign in SIGNS.get().map_or(&[][..], Vec::as_slice).iter().filter(|s| inside(s.pos)) {
        match &sign.kind {
            Kind::Road { front, back } => road_sign(b, sign.pos, sign.dir, front, back),
            Kind::Finger { name } => fingerpost(b, sign.pos, sign.dir, name),
        }
    }
}

/// Where the pole by sample `i` of road `r` stands (its foot, and the
/// direction across the road), unless the spot is unsuitable: in water, on a
/// viaduct, or on or beside a circuit, a trail or another stretch of road.
fn pole_at(network: &RoadNetwork, road: &RoadPath, r: usize, i: usize) -> Option<(Vec3, Vec3)> {
    let c = road.points[i];
    if road.heights[i] - terrain::raw_height(c.x, c.y).max(WATER_LEVEL) > 4.0 {
        return None;
    }
    let t = road.tangent(i);
    let side = Vec2::new(-t.y, t.x) * if hash(r as f32, 3.1) < 0.5 { 1.0 } else { -1.0 };
    let p = c + side * POLE_OFFSET;
    if terrain::raw_height(p.x, p.y) < WATER_LEVEL + 0.5 {
        return None;
    }
    let near = |kind: PathKind, gap: f32| network.nearest_matching(p.x, p.y, |k| k == kind).is_some_and(|s| s.dist < gap);
    if network.nearest_matching(p.x, p.y, |k| k == PathKind::Circuit).is_some()
        || near(PathKind::Trail, TRAIL_HALF_WIDTH + 5.0)
        || near(PathKind::Rail, crate::road::RAIL_HALF_WIDTH + 8.0)
        || near(PathKind::Road, POLE_OFFSET - 3.0)
    {
        return None;
    }
    Some((vec3(p.x, terrain_height(p.x, p.y) - 0.5, p.y), vec3(side.x, 0.0, side.y)))
}

fn telegraph_pole(b: &mut Builder, base: Vec3, across: Vec3) {
    let seed = hash(base.x, base.z);
    let lean = vec3(seed - 0.5, 0.0, hash(base.z, base.x) - 0.5) * 0.25;
    let top = base + Vec3::Y * POLE_HEIGHT + lean;
    b.limb(base, top, 0.26, 0.2, 5, seed * 3.0, POLE_WOOD);
    b.post(base, 0.3, POLE_HEIGHT);
    // Crossarm with an insulator at each end
    let arm = top - Vec3::Y * 0.9;
    b.limb(arm - across * (ARM_SPREAD + 0.3), arm + across * (ARM_SPREAD + 0.3), 0.1, 0.1, 4, 0.0, POLE_WOOD);
    for s in [-1.0, 1.0] {
        let at = arm + across * s * ARM_SPREAD;
        b.limb(at, at + Vec3::Y * 0.35, 0.07, 0.05, 4, 0.0, INSULATOR);
    }
}

/// A wire from `a` to `b`, sagging in the middle.
fn wire(b: &mut Builder, from: Vec3, to: Vec3) {
    const SEGMENTS: usize = 6;
    let sag = 0.5 + from.distance(to) * 0.012;
    let at = |k: usize| {
        let t = k as f32 / SEGMENTS as f32;
        from.lerp(to, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t)
    };
    for k in 0..SEGMENTS {
        b.limb(at(k), at(k + 1), 0.05, 0.05, 3, 0.0, WIRE);
    }
}

/// Lettering in the 5x7 pixel font on a plane: `origin` is the top-left of
/// the first glyph, `right` and `down` span the plane, `px` is one font pixel.
pub(crate) fn lettering(b: &mut Builder, origin: Vec3, right: Vec3, down: Vec3, px: f32, text: &str, color: [f32; 4]) {
    for (i, c) in text.chars().enumerate() {
        let gx = i as f32 * (GLYPH_W as f32 + 1.0);
        for (row, bits) in glyph(c).iter().enumerate() {
            let mut col = 0;
            while col < GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < GLYPH_W && bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                    col += 1;
                }
                let at = |x: f32, y: f32| origin + right * (gx + x) * px + down * y * px;
                let (x0, x1, y0, y1) = (start as f32, col as f32, row as f32, row as f32 + 1.0);
                b.quad(at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1), color);
            }
        }
    }
}

pub(crate) fn text_units(text: &str) -> f32 {
    (text.chars().count() as f32 * (GLYPH_W as f32 + 1.0) - 1.0).max(1.0)
}

/// Green two-post road sign with two lines of lettering on each face.
/// Lettering size (one font pixel) and half-width of a road sign's board.
fn board(front: &[String; 2], back: &[String; 2]) -> (f32, f32) {
    let widest = front.iter().chain(back).map(|t| text_units(t)).fold(1.0, f32::max);
    let px = (SIGN_TEXT_WIDTH / widest).min(0.75);
    (px, widest * px * 0.5 + px * 3.0)
}

fn road_sign(b: &mut Builder, pos: Vec2, facing: Vec2, front: &[String; 2], back: &[String; 2]) {
    let (px, half_w) = board(front, back);
    let margin = px * 3.0;
    let height = (GLYPH_H as f32 * 2.0 + 3.0) * px + margin * 2.0;
    let n = vec3(facing.x, 0.0, facing.y);
    // A reader looks along -facing; their right hand
    let right = vec3(-facing.y, 0.0, facing.x);
    let yaw = (-facing.x).atan2(-facing.y);
    let foot = ground(pos, half_w.min(4.0));
    let bottom = vec3(pos.x, foot + 3.2, pos.y);

    for s in [-1.0, 1.0] {
        let post = vec3(pos.x, foot - 0.5, pos.y) + right * s * (half_w - 1.2);
        b.limb(post, post + Vec3::Y * (3.8 + height), 0.18, 0.18, 4, 0.0, SIGN_POST);
        b.post(post, 0.25, 3.8 + height);
    }
    // White border behind a slightly thicker green face
    b.block(bottom - Vec3::Y * 0.25, half_w + 0.25, height + 0.5, 0.1, yaw, SIGN_WHITE, SIGN_WHITE);
    b.block(bottom, half_w, height, 0.16, yaw, SIGN_GREEN, SIGN_GREEN);

    // Lettering catches the headlights after dark
    b.glowing = true;
    for (lines, normal, r) in [(front, n, right), (back, -n, -right)] {
        for (k, line) in lines.iter().enumerate() {
            let w = text_units(line) * px;
            let top = bottom.y + height - margin - k as f32 * (GLYPH_H as f32 + 3.0) * px;
            let origin = vec3(pos.x, top, pos.y) + normal * 0.2 - r * w * 0.5;
            lettering(b, origin, r, -Vec3::Y, px, line, SIGN_WHITE);
        }
    }
    b.glowing = false;
}

/// Weathered wooden post with an arrow-tipped arm pointing along `dir`.
fn fingerpost(b: &mut Builder, pos: Vec2, dir: Vec2, name: &str) {
    let px = (14.0 / text_units(name)).min(0.34);
    let len = text_units(name) * px + 1.4;
    let arm_h = GLYPH_H as f32 * px + 0.5;
    let foot = ground(pos, 0.5);
    let base = vec3(pos.x, foot - 0.4, pos.y);
    let height = 5.2;
    b.limb(base, base + Vec3::Y * height, 0.2, 0.18, 4, 0.4, FINGER_POST);
    b.post(base, 0.25, height);
    b.limb(base + Vec3::Y * height, base + Vec3::Y * (height + 0.2), 0.26, 0.05, 4, 0.4, FINGER_POST);

    // Arm: a thin board from the post out along `dir`, its tip cut to a point
    let d = vec3(dir.x, 0.0, dir.y);
    let side = vec3(-dir.y, 0.0, dir.x);
    let top = base.y + height - 0.4;
    let corner = |along: f32, y: f32, s: f32| vec3(pos.x, y, pos.y) + d * along + side * s * 0.08;
    let (y0, y1, tip) = (top - arm_h, top, len + arm_h * 0.5);
    for s in [-1.0, 1.0] {
        b.quad(corner(-0.3, y0, s), corner(len, y0, s), corner(len, y1, s), corner(-0.3, y1, s), FINGER_ARM);
        b.tri(corner(len, y0, s), corner(tip, (y0 + y1) * 0.5, s), corner(len, y1, s), FINGER_ARM);
    }
    b.quad(corner(-0.3, y1, -1.0), corner(len, y1, -1.0), corner(len, y1, 1.0), corner(-0.3, y1, 1.0), FINGER_ARM);
    b.quad(corner(-0.3, y0, -1.0), corner(len, y0, -1.0), corner(len, y0, 1.0), corner(-0.3, y0, 1.0), FINGER_ARM);

    // The name on both faces, the right way round from either side
    let w = text_units(name) * px;
    let mid = (len - 0.3) * 0.5 - 0.15;
    for s in [1.0, -1.0] {
        let r = -d * s;
        let origin = vec3(pos.x, y1 - 0.25, pos.y) + d * mid - r * w * 0.5 + side * s * 0.12;
        lettering(b, origin, r, -Vec3::Y, px, name, FINGER_TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_poles_stand_off_the_tarmac() {
        let network = crate::sim_tests::network();
        let signs = SIGNS.get().expect("signs installed");
        let roads = network.paths.iter().filter(|p| p.kind == PathKind::Road).count();
        let road_signs = signs.iter().filter(|s| matches!(s.kind, Kind::Road { .. })).count();
        let fingerposts = signs.len() - road_signs;
        println!("{road_signs} road signs on {roads} roads, {fingerposts} fingerposts");
        assert_eq!(road_signs, roads * 2, "a sign at each end of every road");
        assert!(fingerposts * 10 >= network.paths.iter().filter(|p| p.kind == PathKind::Trail).count() * 8, "most trails get a fingerpost");

        let mut poles = 0;
        let on_tarmac = |p: Vec2, gap: f32| network.nearest(p.x, p.y).is_some_and(|s| s.dist < s.half_width() + gap);
        for sign in signs {
            assert!(!on_tarmac(sign.pos, 1.5), "sign at {} stands on a path", sign.pos);
        }
        for (r, road) in network.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
            for i in (POLE_EVERY..road.points.len() - POLE_EVERY).step_by(POLE_EVERY) {
                if let Some((base, _)) = pole_at(network, road, r, i) {
                    poles += 1;
                    assert!(!on_tarmac(Vec2::new(base.x, base.z), 4.0), "pole at {base} stands on a path");
                }
            }
        }
        println!("{poles} telegraph poles");
        assert!(poles > 500);
    }
}
