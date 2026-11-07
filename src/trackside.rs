//! Trackside furniture on the circuits: gravel traps and tyre walls round the
//! outside of the corners, a barrier line (catch fencing where people stand,
//! Armco elsewhere) with sponsors' boards along it, braking boards counting
//! down to the tight corners, painted grid boxes, and a sponsors' bridge over
//! the longest straight. Things close by the track are what make speed feel
//! like speed.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec3};

use crate::collisions::Solid;
use crate::props::{hash, Builder, Layer};
use crate::road::{PathKind, RoadNetwork, RoadPath, CIRCUIT_SHOULDER, KERB_WIDTH, ROAD_HALF_WIDTH, SAMPLE_SPACING};
use crate::roadside::{lettering, text_units};
use crate::terrain::{self, terrain_height};

/// Bends tighter than this get tyre walls.
const CORNER_RADIUS: f32 = 260.0;
/// Corners tighter than this get braking boards before them.
const HAIRPIN_RADIUS: f32 = 130.0;
/// Bends gentler than this count as straight (for hoardings).
const STRAIGHT_RADIUS: f32 = 1200.0;
/// Distance from the centreline: the barrier line stands on the flat
/// shoulder, walls and fences along it; run-off lies between it and the kerb,
/// and floodlights stand behind it. Boards are just off the kerb.
pub const BARRIER_OFFSET: f32 = ROAD_HALF_WIDTH + 26.0;
const WALL_OFFSET: f32 = BARRIER_OFFSET;
const RUNOFF_FROM: f32 = ROAD_HALF_WIDTH + KERB_WIDTH + 0.5;
const RUNOFF_TO: f32 = BARRIER_OFFSET - 1.5;
const FLOODLIGHT_OFFSET: f32 = ROAD_HALF_WIDTH + CIRCUIT_SHOULDER;
const BOARD_OFFSET: f32 = ROAD_HALF_WIDTH + 9.0;
/// Samples of barrier in a run, a gap for getting through left at the end of each.
const BARRIER_RUN: usize = 70;
const BARRIER_GAP: usize = 4;
const FENCE_HEIGHT: f32 = 6.5;
/// Samples either side of the start line kept clear for the grandstands.
const START_CLEAR: usize = 45;
/// Braking boards: distances before the corner.
const BOARD_DISTANCES: [(f32, &str); 3] = [(150.0, "150"), (100.0, "100"), (50.0, "50")];
const WALL_HEIGHT: f32 = 1.8;

const TYRE: [f32; 4] = [0.1, 0.1, 0.11, 1.0];
const TYRE_DARK: [f32; 4] = [0.06, 0.06, 0.07, 1.0];
const BAND: [[f32; 4]; 2] = [[0.85, 0.12, 0.1, 1.0], [0.95, 0.95, 0.92, 1.0]];
const BOARD_WHITE: [f32; 4] = [0.95, 0.95, 0.92, 1.0];
const BOARD_TEXT: [f32; 4] = [0.08, 0.08, 0.1, 1.0];
const POST: [f32; 4] = [0.6, 0.6, 0.62, 1.0];
/// Made-up sponsors: name, background, lettering.
pub const SPONSORS: [Sponsor; 8] = [
    ("STUMBLER", [0.1, 0.25, 0.6, 1.0], [1.0, 0.85, 0.2, 1.0]),
    ("TURBO OIL", [0.75, 0.1, 0.08, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ("GRIP TYRES", [0.08, 0.08, 0.1, 1.0], [1.0, 0.85, 0.2, 1.0]),
    ("BOOST COLA", [0.85, 0.15, 0.35, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ("FAST LANE", [0.1, 0.5, 0.25, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ("APEX", [1.0, 0.75, 0.1, 1.0], [0.1, 0.1, 0.12, 1.0]),
    ("LOW POLY", [0.3, 0.15, 0.55, 1.0], [0.9, 0.95, 1.0, 1.0]),
    ("FULL THROTTLE", [0.95, 0.95, 0.92, 1.0], [0.8, 0.1, 0.1, 1.0]),
];

/// A name, its background and its lettering.
pub type Sponsor = (&'static str, [f32; 4], [f32; 4]);

/// Mown grass, light and dark stripes, laid along the circuit.
const MOWN: [[f32; 4]; 2] = [[0.36, 0.6, 0.2, 1.0], [0.3, 0.52, 0.17, 1.0]];
const GRAVEL: [[f32; 4]; 3] = [[0.74, 0.66, 0.5, 1.0], [0.69, 0.61, 0.46, 1.0], [0.78, 0.71, 0.56, 1.0]];
const PAINT: [f32; 4] = [0.95, 0.95, 0.92, 1.0];
const RAIL: [f32; 4] = [0.74, 0.76, 0.78, 1.0];
const FENCE_POST: [f32; 4] = [0.38, 0.4, 0.44, 1.0];
const JERSEY: [f32; 4] = [0.78, 0.78, 0.75, 1.0];
/// Bunting along the fences, colour after colour.
const BUNTING: [[f32; 3]; 4] = [[0.9, 0.16, 0.12], [0.96, 0.96, 0.92], [0.12, 0.36, 0.82], [0.98, 0.8, 0.14]];

struct Corner {
    start: usize,
    end: usize,
    /// Which side is the outside, in units of the path's side vector.
    outside: f32,
    tight: bool,
}

struct Layout {
    corners: Vec<Corner>,
    /// Runs of straight: (first, last) sample.
    straights: Vec<(usize, usize)>,
}

static LAYOUTS: OnceLock<Vec<Layout>> = OnceLock::new();

pub fn side(path: &RoadPath, i: usize) -> Vec2 {
    let t = path.tangent(i);
    Vec2::new(-t.y, t.x)
}

fn near_start(path: &RoadPath, i: usize) -> bool {
    let n = path.points.len();
    i.min(n - i) < START_CLEAR
}

/// Corners and straights round each circuit, in network order.
fn layouts(network: &RoadNetwork) -> &'static [Layout] {
    LAYOUTS.get_or_init(|| {
        network.paths.iter().filter(|p| p.circuit()).map(|path| {
            let n = path.points.len();
            let runs = |want: &dyn Fn(f32) -> bool, shortest: usize| {
                let mut runs = Vec::new();
                let mut i = 0;
                while i < n {
                    if !want(path.curvature[i]) {
                        i += 1;
                        continue;
                    }
                    let start = i;
                    while i < n && want(path.curvature[i]) {
                        i += 1;
                    }
                    if i - start >= shortest {
                        runs.push((start, i - 1));
                    }
                }
                runs
            };
            let corners = runs(&|c| c > 1.0 / CORNER_RADIUS, 5).into_iter().map(|(start, end)| {
                let turn = path.tangent(start).perp_dot(path.tangent(end));
                let tightest = (start..=end).map(|i| path.curvature[i]).fold(0.0, f32::max);
                Corner { start, end, outside: -turn.signum(), tight: tightest > 1.0 / HAIRPIN_RADIUS }
            }).collect();
            let straights = runs(&|c| c < 1.0 / STRAIGHT_RADIUS, 40);
            Layout { corners, straights }
        }).collect()
    })
}

/// The corners of `path`: first sample, last, and which side is the outside.
pub fn corners(network: &RoadNetwork, path: &RoadPath) -> Vec<(usize, usize, f32)> {
    let k = network.paths.iter().filter(|p| p.circuit()).position(|p| std::ptr::eq(p, path)).expect("a circuit");
    layouts(network)[k].corners.iter().map(|c| (c.start, c.end, c.outside)).collect()
}

/// Height to lay something flat at `p`, on top of the ground however it's drawn.
pub fn surface(p: Vec2) -> f32 {
    terrain_height(p.x, p.y).max(terrain::mesh_surface(p.x, p.y, 12.5)).max(terrain::mesh_surface(p.x, p.y, 25.0)) + 0.08
}

/// `surface` for the flat ground beside a circuit's tarmac, where the true
/// surface is what's drawn (and much quicker to find).
fn flat_surface(p: Vec2) -> f32 {
    terrain_height(p.x, p.y) + 0.1
}

/// Whether a spot this far out from circuit `path`'s centreline is dry
/// land, clear of junctions, other paths and other parts of the circuit.
fn clear(network: &RoadNetwork, p: Vec2, offset: f32) -> bool {
    if crate::terrain::raw_height(p.x, p.y) < crate::terrain::WATER_LEVEL + 0.5 {
        return false;
    }
    match network.nearest(p.x, p.y) {
        Some(s) if s.kind == PathKind::Circuit => s.dist > offset - 3.0,
        Some(s) => s.dist > s.half_width() + 8.0,
        None => true,
    }
}

/// A floodlight on a tall pole beside a circuit, angled down onto the track.
pub struct Floodlight {
    /// Foot of the pole.
    pub base: Vec3,
    /// The lamp head, out over the run-off.
    pub head: Vec3,
    /// Middle of the pool of light it throws on the track.
    pub pool: Vec2,
    /// The pool's rings laid on the ground: `POOL_RINGS[r]` round, `POOL_SIDES`
    /// points each, ring by ring.
    pub rings: Vec<Vec3>,
}

/// Floodlights stand this far apart round each circuit, on alternate sides.
const FLOODLIGHT_EVERY: usize = 30;
const FLOODLIGHT_HEIGHT: f32 = 18.0;
/// Radius of a floodlight's pool of light, the rings it falls off in (as a
/// share of that), and the sides of each ring.
pub const POOL_RADIUS: f32 = 26.0;
pub const POOL_RINGS: [f32; 4] = [0.0, 0.45, 0.75, 1.0];
pub const POOL_SIDES: usize = 14;

static FLOODLIGHTS: OnceLock<Vec<Floodlight>> = OnceLock::new();

/// Every floodlight round every circuit.
pub fn floodlights(network: &RoadNetwork) -> &'static [Floodlight] {
    FLOODLIGHTS.get_or_init(|| {
        let circuits = network.paths.iter().filter(|p| p.circuit()).collect();
        crate::layout::parallel_map(circuits, |path| {
            let mut lights = Vec::new();
            for (k, i) in (0..path.points.len()).step_by(FLOODLIGHT_EVERY).enumerate() {
                let s = if k % 2 == 0 { 1.0 } else { -1.0 };
                let side = side(path, i) * s;
                let foot = path.points[i] + side * FLOODLIGHT_OFFSET;
                // Not on the lip of a drop, where the mast would stand on nothing
                let lip = terrain_height(foot.x, foot.y) - crate::props::ground(foot, 2.0) > 1.5;
                if lip || !clear(network, foot, FLOODLIGHT_OFFSET) || crate::venue::pit_lane(foot) {
                    continue;
                }
                let base = vec3(foot.x, crate::props::ground(foot, 0.5) - 0.3, foot.y);
                let arm = path.points[i] + side * (FLOODLIGHT_OFFSET - 5.0);
                let head = vec3(arm.x, base.y + FLOODLIGHT_HEIGHT, arm.y);
                let pool = path.points[i] + side * 6.0;
                let rings = POOL_RINGS.iter().flat_map(|&r| (0..POOL_SIDES).map(move |k| {
                    let a = std::f32::consts::TAU * k as f32 / POOL_SIDES as f32;
                    let p = pool + Vec2::new(a.cos(), a.sin()) * r * POOL_RADIUS;
                    vec3(p.x, terrain_height(p.x, p.y) + 0.2, p.y)
                })).collect();
                lights.push(Floodlight { base, head, pool, rings });
            }
            lights
        }).into_iter().flatten().collect()
    })
}

/// Everything trackside anchored in the chunk `inside` accepts.
pub fn build(b: &mut Builder, network: &RoadNetwork, inside: &impl Fn(Vec2) -> bool) {
    b.tag("floodlights");
    for light in floodlights(network).iter().filter(|l| inside(Vec2::new(l.base.x, l.base.z))) {
        floodlight(b, light);
    }
    let layouts = layouts(network);
    for (path, layout) in network.paths.iter().filter(|p| p.circuit()).zip(layouts) {
        let n = path.points.len();
        let mut walled = vec![[false; 2]; n];
        for corner in &layout.corners {
            let s = (corner.outside > 0.0) as usize;
            for k in corner.start + n - 8..corner.end + n + 8 {
                walled[k % n][s] = true;
            }
            gravel_trap(b, network, path, corner, inside);
            b.tag("tyres");
            tyre_wall(b, network, path, corner, inside);
            b.tag("boards");
            if corner.tight {
                for (distance, label) in BOARD_DISTANCES {
                    let i = (corner.start + n - (distance / SAMPLE_SPACING) as usize) % n;
                    let p = path.points[i] + side(path, i) * corner.outside * BOARD_OFFSET;
                    if inside(path.points[i]) && !near_start(path, i) && clear(network, p, BOARD_OFFSET) {
                        braking_board(b, p, path.tangent(i), label);
                    }
                }
            }
        }
        b.tag("barriers");
        barriers(b, network, path, &walled, inside);
        b.tag("trackside");
        mown_stripes(b, network, path, &walled, inside);
        grid_boxes(b, path, inside);
        if let Some(&(first, last)) = layout.straights.iter().filter(|s| !near_start(path, (s.0 + s.1) / 2)).max_by_key(|s| s.1 - s.0) {
            let i = (first + last) / 2;
            if inside(path.points[i]) {
                b.tag("bridge");
                bridge(b, network, path, i);
            }
        }
    }
}

/// The barrier line down each side: catch fencing with sponsors' boards and
/// bunting where people stand behind it, Armco elsewhere, gaps where other
/// paths meet the circuit and every so often for getting through.
fn barriers(b: &mut Builder, network: &RoadNetwork, path: &RoadPath, walled: &[[bool; 2]], inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    let stride = 2;
    for (slot, s) in [(0, -1.0), (1, 1.0)] {
        let at = |i: usize| path.points[i % n] + side(path, i % n) * s * BARRIER_OFFSET;
        for i in (0..n).step_by(stride) {
            let (a, c) = (at(i), at(i + stride));
            if !inside(a) || i % (BARRIER_RUN + BARRIER_GAP) >= BARRIER_RUN || walled[i][slot] || walled[(i + stride) % n][slot] {
                continue;
            }
            if !clear(network, a, BARRIER_OFFSET) || !clear(network, c, BARRIER_OFFSET) || crate::venue::pit_lane(a) || crate::venue::pit_lane(c) {
                continue;
            }
            let out = side(path, i) * s;
            let peopled = crate::venue::occupied(a + out * 14.0, 0.0);
            let (ya, yc) = (crate::props::ground(a, 0.2) - 0.2, crate::props::ground(c, 0.2) - 0.2);
            let (a3, c3) = (vec3(a.x, ya, a.y), vec3(c.x, yc, c.y));
            let along = c - a;
            let yaw = (-along.x).atan2(-along.y);
            let mid = (a + c) * 0.5;
            b.solids.push(Solid { centre: mid, half: Vec2::new(0.5, along.length() * 0.5 + 0.4), yaw, top: ya.max(yc) + 3.0, round: false });
            if peopled {
                fence_panel(b, a3, c3, out, i);
            } else {
                armco(b, a3, c3, s);
                // Sponsors' boards behind the Armco every so often
                if i % 16 == 8 {
                    let p = a + out * 4.0;
                    if clear(network, p, BARRIER_OFFSET) {
                        let pick = (hash(p.x * 0.13, p.y * 0.29) * SPONSORS.len() as f32) as usize % SPONSORS.len();
                        hoarding(b, p, -out, SPONSORS[pick]);
                    }
                }
            }
        }
    }
}

/// A span of W-beam on a post at its start.
fn armco(b: &mut Builder, a: Vec3, c: Vec3, s: f32) {
    b.limb(a, a + Vec3::Y * 1.3, 0.12, 0.1, 4, 0.0, FENCE_POST);
    let lift = |p: Vec3, y: f32| p + Vec3::Y * y;
    let along = (c - a).normalize_or_zero();
    let toward = vec3(along.z, 0.0, -along.x) * s * 0.14;
    for y in [0.72, 1.08] {
        let (p, q) = (lift(a, y) + toward, lift(c, y) + toward);
        let (up, down) = (Vec3::Y * 0.2, -Vec3::Y * 0.2);
        b.quad(p + down, q + down, q + up, p + up, RAIL);
        b.quad(p + up, q + up, q + up - toward * 1.6, p + up - toward * 1.6, RAIL);
    }
}

/// A span of catch fence: a concrete footing, a post with its arm leaning
/// over the track, wires, boards, and bunting on top.
fn fence_panel(b: &mut Builder, a: Vec3, c: Vec3, out: Vec2, i: usize) {
    let up = Vec3::Y;
    let across = vec3(out.x, 0.0, out.y);
    let along = c - a;
    let length = along.length();
    let right = along / length.max(0.01);
    let toe = across * 0.4;
    // The footing, sloped like a road barrier
    b.quad(a - toe, c - toe, c - toe * 0.4 + up * 1.2, a - toe * 0.4 + up * 1.2, JERSEY);
    b.quad(a + toe, c + toe, c + toe * 0.4 + up * 1.2, a + toe * 0.4 + up * 1.2, JERSEY);
    b.quad(a - toe * 0.4 + up * 1.2, c - toe * 0.4 + up * 1.2, c + toe * 0.4 + up * 1.2, a + toe * 0.4 + up * 1.2, JERSEY);

    let top = a + up * FENCE_HEIGHT;
    // The post stands on the footing
    b.tag("deck");
    b.limb(a + up * 1.2, top, 0.1, 0.08, 4, 0.0, FENCE_POST);
    b.tag("barriers");
    b.limb(top, top - across * 1.1 + up * 0.5, 0.06, 0.05, 4, 0.0, FENCE_POST);
    for y in [2.2, 3.4, 4.6, FENCE_HEIGHT - 0.1] {
        b.limb(a + up * y, c + up * y, 0.03, 0.03, 3, 0.0, FENCE_POST);
    }
    let sponsor = SPONSORS[(i / 4 + (i / 40) * 3) % SPONSORS.len()];
    if length > 1.0 {
        let middle = (a + c) * 0.5 + up * 1.3 - toe * 0.8;
        panel_tilted(b, middle, (c.y - a.y) / length * right_sign(right, out), -out, length * 0.5 - 0.05, 0.62, sponsor);
    }
    // Bunting along the top wire
    let layer = std::mem::replace(&mut b.layer, Layer::Flag);
    let wire = up * (FENCE_HEIGHT - 0.1);
    for k in 0..3 {
        let (p, q) = (a + wire + right * length * k as f32 / 3.0, a + wire + right * length * (k + 1) as f32 / 3.0);
        let tip = (p + q) * 0.5 - up * 1.1;
        b.cloth([p, q, tip, tip], [0.0, 0.0, 0.7, 0.7], BUNTING[(i / 2 + k) % BUNTING.len()]);
    }
    b.layer = layer;
}

/// Which way a board facing back along `out` reads compared with `along`.
fn right_sign(along: Vec3, out: Vec2) -> f32 {
    along.dot(vec3(out.y, 0.0, -out.x)).signum()
}

/// Gravel out from the kerb round the outside of a corner, easing in and out
/// at its ends.
fn gravel_trap(b: &mut Builder, network: &RoadNetwork, path: &RoadPath, corner: &Corner, inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    let lead = 8;
    let (first, last) = (corner.start + n - lead, corner.end + n + lead);
    let reach = |k: usize| {
        let ends = (k - first).min(last - k) as f32 / lead as f32;
        RUNOFF_FROM + (RUNOFF_TO - RUNOFF_FROM) * ends.clamp(0.0, 1.0).sqrt()
    };
    let spot = |k: usize, offset: f32| {
        let p = path.points[k % n] + side(path, k % n) * corner.outside * offset;
        vec3(p.x, flat_surface(p), p.y)
    };
    for k in (first..last).step_by(2) {
        if !inside(path.points[k % n]) || near_start(path, k % n) {
            continue;
        }
        let (end0, end1) = (reach(k), reach((k + 2).min(last)));
        if !clear(network, path.points[k % n] + side(path, k % n) * corner.outside * end0, BARRIER_OFFSET * 0.5) {
            continue;
        }
        for r in 0..3 {
            let (f0, f1) = (r as f32 / 3.0, (r + 1) as f32 / 3.0);
            let lerp = |e: f32, f: f32| RUNOFF_FROM + (e - RUNOFF_FROM) * f;
            let tone = GRAVEL[(hash(k as f32, r as f32) * 3.0) as usize % 3];
            let next = (k + 2).min(last);
            b.quad(spot(k, lerp(end0, f0)), spot(next, lerp(end1, f0)), spot(next, lerp(end1, f1)), spot(k, lerp(end0, f1)), tone);
        }
    }
}

/// Stripes of mown grass between the kerbs and the barriers, where there's
/// no gravel.
fn mown_stripes(b: &mut Builder, network: &RoadNetwork, path: &RoadPath, walled: &[[bool; 2]], inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    let stride = 2;
    const BANDS: usize = 4;
    for (slot, s) in [(0, -1.0), (1, 1.0)] {
        let spot = |i: usize, offset: f32| {
            let p = path.points[i % n] + side(path, i % n) * s * offset;
            vec3(p.x, flat_surface(p) - 0.03, p.y)
        };
        for i in (0..n).step_by(stride) {
            let j = i + stride;
            if !inside(path.points[i]) || walled[i][slot] || walled[j % n][slot] {
                continue;
            }
            let (a, c) = (path.points[i] + side(path, i) * s * RUNOFF_TO, path.points[j % n] + side(path, j % n) * s * RUNOFF_TO);
            if !clear(network, a, RUNOFF_TO) || !clear(network, c, RUNOFF_TO) || crate::venue::pit_lane(a) {
                continue;
            }
            for k in 0..BANDS {
                let (f0, f1) = (k as f32 / BANDS as f32, (k + 1) as f32 / BANDS as f32);
                let off = |f: f32| RUNOFF_FROM + (RUNOFF_TO - RUNOFF_FROM) * f;
                b.quad(spot(i, off(f0)), spot(j, off(f0)), spot(j, off(f1)), spot(i, off(f1)), MOWN[k % 2]);
            }
        }
    }
}

/// The white boxes the cars line up in, behind the start line.
fn grid_boxes(b: &mut Builder, path: &RoadPath, inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    if !inside(path.points[0]) {
        return;
    }
    let spot = |p: Vec2| vec3(p.x, terrain_height(p.x, p.y) + 0.09, p.y);
    for k in 0..8 {
        let i = n - 3 - k * crate::race::GRID_STEP;
        let wide = if k % 2 == 0 { -crate::race::GRID_WIDE } else { crate::race::GRID_WIDE };
        let (t, centre) = (path.tangent(i), path.points[i] + side(path, i) * wide);
        let across = Vec2::new(-t.y, t.x);
        let front = centre + t * 5.5;
        let quad = |b: &mut Builder, a: Vec2, c: Vec2, w: Vec2| b.quad(spot(a - w), spot(c - w), spot(c + w), spot(a + w), PAINT);
        quad(b, front - across * 4.0, front + across * 4.0, t * 0.22);
        for s in [-4.0, 4.0] {
            quad(b, front + across * s, front + across * s - t * 3.2, across * 0.2);
        }
    }
}

/// A sponsors' bridge over the track on two lattice towers, boards facing
/// each way along it.
fn bridge(b: &mut Builder, network: &RoadNetwork, path: &RoadPath, i: usize) {
    let (p, t) = (path.points[i], path.tangent(i));
    let across = side(path, i);
    if [-1.0, 1.0].iter().any(|s| !clear(network, p + across * *s * 36.0, 30.0)) {
        return;
    }
    let span = ROAD_HALF_WIDTH + KERB_WIDTH + 6.0;
    let (low, high) = (path.heights[i] + 15.0, path.heights[i] + 21.0);
    let (a3, t3) = (vec3(across.x, 0.0, across.y), vec3(t.x, 0.0, t.y));
    let yaw = (-t.x).atan2(-t.y);
    for s in [-1.0, 1.0] {
        let foot = p + across * s * span;
        let base = crate::props::ground(foot, 3.0);
        b.solid_block(vec3(foot.x, base - 0.5, foot.y), 2.6, 1.6, 2.6, yaw, JERSEY, JERSEY);
        for (dx, dz) in [(-1.6, -1.6), (1.6, -1.6), (1.6, 1.6), (-1.6, 1.6)] {
            let c = vec3(foot.x, 0.0, foot.y) + a3 * dx + t3 * dz;
            b.limb(vec3(c.x, base, c.z), vec3(c.x, high, c.z), 0.2, 0.2, 4, 0.0, FENCE_POST);
        }
        let mut y = base + 1.0;
        while y < high - 1.0 {
            let c = vec3(foot.x, y, foot.y);
            b.limb(c - a3 * 1.6 - t3 * 1.6, c + a3 * 1.6 + t3 * 1.6 + Vec3::Y * 3.0, 0.08, 0.08, 4, 0.0, FENCE_POST);
            b.limb(c + a3 * 1.6 - t3 * 1.6, c - a3 * 1.6 + t3 * 1.6 + Vec3::Y * 3.0, 0.08, 0.08, 4, 0.0, FENCE_POST);
            y += 3.0;
        }
    }
    b.block(vec3(p.x, low, p.y), span + 1.0, high - low, 1.1, yaw, [0.16, 0.17, 0.2, 1.0], [0.16, 0.17, 0.2, 1.0]);
    let pick = SPONSORS[(i * 7) % SPONSORS.len()];
    for s in [-1.0, 1.0] {
        let face = t * s;
        panel(b, vec3(p.x, (low + high) * 0.5, p.y) + vec3(face.x, 0.0, face.y) * 1.16, face, span - 2.0, (high - low) * 0.5 - 0.4, pick);
    }
}

/// A slim grey mast with an arm out toward the track and a lamp head that
/// shines after dark.
fn floodlight(b: &mut Builder, light: &Floodlight) {
    let top = vec3(light.base.x, light.head.y, light.base.z);
    b.limb(light.base, top, 0.35, 0.25, 6, 0.0, POST);
    b.post(light.base, 0.5, FLOODLIGHT_HEIGHT);
    // The arm and lamp hang from the top of the mast
    b.tag("deck");
    b.limb(top, light.head, 0.15, 0.15, 4, 0.0, POST);
    let toward = (light.head - top).normalize_or_zero();
    let yaw = (-toward.x).atan2(-toward.z);
    b.block(light.head - Vec3::Y * 0.6, 1.4, 0.9, 0.9, yaw, [0.2, 0.2, 0.22, 1.0], [0.2, 0.2, 0.22, 1.0]);
    b.glowing = true;
    b.block(light.head - Vec3::Y * 0.75, 1.2, 0.15, 0.75, yaw, [1.0, 0.96, 0.85, 1.0], [1.0, 0.96, 0.85, 1.0]);
    b.glowing = false;
    b.tag("floodlights");
}

/// Stacked tyres round the outside of a corner, every third pile painted,
/// roped together with a band of conveyor belt along the front.
fn tyre_wall(b: &mut Builder, network: &RoadNetwork, path: &RoadPath, corner: &Corner, inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    let at = |i: usize| path.points[i % n] + side(path, i % n) * corner.outside * WALL_OFFSET;
    let lead = 8;
    for k in corner.start + n - lead..corner.end + n + lead {
        let (i, j) = (k % n, (k + 1) % n);
        if !inside(path.points[i]) || near_start(path, i) {
            continue;
        }
        let (a, c) = (at(i), at(j));
        if !clear(network, a, WALL_OFFSET) || !clear(network, c, WALL_OFFSET) {
            continue;
        }
        let (ya, yc) = (crate::props::ground(a, 0.8) - 0.3, crate::props::ground(c, 0.8) - 0.3);
        // Three piles to a 4-unit stretch, red and white every so often
        for t in 0..3 {
            let f = (t as f32 + 0.5) / 3.0;
            let p = a.lerp(c, f);
            let y = ya + (yc - ya) * f;
            let count = k * 3 + t;
            let tone = match count % 12 {
                0..=2 => BAND[0],
                6..=8 => BAND[1],
                _ if count % 2 == 0 => TYRE,
                _ => TYRE_DARK,
            };
            b.column(vec3(p.x, y, p.y), 0.72, 0.72, WALL_HEIGHT, 7, tone);
            let hole = vec3(p.x, y + WALL_HEIGHT + 0.01, p.y);
            b.column(hole, 0.4, 0.4, 0.01, 7, TYRE_DARK);
        }
        let along = (c - a).normalize_or_zero();
        let mid = (a + c) * 0.5;
        let yaw = (-along.x).atan2(-along.y);
        b.solid_block(vec3(mid.x, ya.min(yc), mid.y), 0.8, WALL_HEIGHT + (ya - yc).abs(), a.distance(c) * 0.5, yaw, TYRE, TYRE);
    }
}

/// A countdown board on a post beside the approach to a tight corner.
fn braking_board(b: &mut Builder, p: Vec2, ahead: Vec2, label: &str) {
    let ground = crate::props::ground(p, 0.2);
    let base = vec3(p.x, ground - 0.3, p.y);
    b.limb(base, base + Vec3::Y * 3.6, 0.12, 0.12, 4, 0.0, POST);
    // The board faces the cars coming toward it
    let facing = vec3(-ahead.x, 0.0, -ahead.y);
    let right = vec3(ahead.y, 0.0, -ahead.x);
    let centre = base + Vec3::Y * 3.6;
    let (w, h) = (2.6, 1.6);
    let corner = |x: f32, y: f32| centre + right * x + Vec3::Y * y;
    b.quad(corner(-w, -h), corner(w, -h), corner(w, h), corner(-w, h), BOARD_WHITE);
    let px = (4.2 / text_units(label)).min(0.38);
    let width = text_units(label) * px;
    let origin = centre + facing * 0.05 - right * width * 0.5 + Vec3::Y * (7.0 * px * 0.5);
    lettering(b, origin, right, -Vec3::Y, px, label, BOARD_TEXT);
}

/// A sponsor's hoarding on two legs, facing the track (`facing` points at
/// it). The board sits level, its legs each reaching down to the ground.
fn hoarding(b: &mut Builder, p: Vec2, facing: Vec2, sponsor: Sponsor) {
    let right = Vec2::new(-facing.y, facing.x);
    let half_len = 7.0;
    let legs = [p - right * (half_len - 0.8), p + right * (half_len - 0.8)];
    let feet = legs.map(|q| terrain::settled(q.x, q.y));
    let low = feet[0].max(feet[1]) + 0.6;
    for (q, y) in legs.into_iter().zip(feet) {
        b.limb(vec3(q.x, y - 0.3, q.y), vec3(q.x, low + 0.4, q.y), 0.1, 0.1, 4, 0.0, POST);
    }
    let yaw = (-facing.x).atan2(-facing.y);
    let height = 2.7;
    // The board itself is up on its legs
    #[cfg(test)]
    let source = std::mem::replace(&mut b.source, "deck");
    b.solid_block(vec3(p.x, low, p.y), half_len, height, 0.15, yaw, POST, POST);
    #[cfg(test)]
    b.tag(source);
    let f = vec3(facing.x, 0.0, facing.y);
    panel(b, vec3(p.x, low + height * 0.5, p.y) + f * 0.17, facing, half_len, height * 0.5, sponsor);
}

/// A sponsor's board facing along `facing`, its middle at `centre`, level.
pub fn panel(b: &mut Builder, centre: Vec3, facing: Vec2, half: f32, half_height: f32, sponsor: Sponsor) {
    panel_tilted(b, centre, 0.0, facing, half, half_height, sponsor);
}

/// A sponsor's board rising `slope` along its length, to follow the ground.
fn panel_tilted(b: &mut Builder, centre: Vec3, slope: f32, facing: Vec2, half: f32, half_height: f32, (name, back, ink): Sponsor) {
    let f = vec3(facing.x, 0.0, facing.y);
    let right = vec3(-facing.y, 0.0, facing.x);
    let at = |x: f32, y: f32| centre + right * x + Vec3::Y * (y + x * slope);
    b.quad(at(-half, -half_height), at(half, -half_height), at(half, half_height), at(-half, half_height), back);
    let px = (half * 1.8 / text_units(name)).min(half_height * 2.0 * 0.75 / 7.0);
    let width = text_units(name) * px;
    let origin = at(-width * 0.5, 3.5 * px) + f * 0.03;
    lettering(b, origin, right, -Vec3::Y, px, name, ink);
}
