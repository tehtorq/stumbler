//! Trackside furniture on the circuits: tyre walls round the outside of the
//! corners, braking boards counting down to the tight ones, and sponsor
//! hoardings along the straights. Things close by the track are what make
//! speed feel like speed.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec3};

use crate::props::{hash, Builder};
use crate::road::{PathKind, RoadNetwork, RoadPath, CIRCUIT_SHOULDER, ROAD_HALF_WIDTH};
use crate::roadside::{lettering, text_units};
use crate::terrain::terrain_height;

/// Bends tighter than this get tyre walls.
const CORNER_RADIUS: f32 = 260.0;
/// Corners tighter than this get braking boards before them.
const HAIRPIN_RADIUS: f32 = 130.0;
/// Bends gentler than this count as straight (for hoardings).
const STRAIGHT_RADIUS: f32 = 1200.0;
/// Distance from the centreline: past the flat run-off for walls and
/// hoardings, just off the kerb for boards.
const WALL_OFFSET: f32 = ROAD_HALF_WIDTH + CIRCUIT_SHOULDER + 4.0;
const HOARDING_OFFSET: f32 = ROAD_HALF_WIDTH + CIRCUIT_SHOULDER + 2.0;
const BOARD_OFFSET: f32 = ROAD_HALF_WIDTH + 9.0;
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
const HOARDING_BACK: [f32; 4] = [0.45, 0.45, 0.48, 1.0];
/// Made-up sponsors: name, background, lettering.
const SPONSORS: [(&str, [f32; 4], [f32; 4]); 8] = [
    ("STUMBLER", [0.1, 0.25, 0.6, 1.0], [1.0, 0.85, 0.2, 1.0]),
    ("TURBO OIL", [0.75, 0.1, 0.08, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ("GRIP TYRES", [0.08, 0.08, 0.1, 1.0], [1.0, 0.85, 0.2, 1.0]),
    ("BOOST COLA", [0.85, 0.15, 0.35, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ("FAST LANE", [0.1, 0.5, 0.25, 1.0], [1.0, 1.0, 1.0, 1.0]),
    ("APEX", [1.0, 0.75, 0.1, 1.0], [0.1, 0.1, 0.12, 1.0]),
    ("LOW POLY", [0.3, 0.15, 0.55, 1.0], [0.9, 0.95, 1.0, 1.0]),
    ("FULL THROTTLE", [0.95, 0.95, 0.92, 1.0], [0.8, 0.1, 0.1, 1.0]),
];

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

fn side(path: &RoadPath, i: usize) -> Vec2 {
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
        let mut lights = Vec::new();
        for path in network.paths.iter().filter(|p| p.circuit()) {
            for (k, i) in (0..path.points.len()).step_by(FLOODLIGHT_EVERY).enumerate() {
                let s = if k % 2 == 0 { 1.0 } else { -1.0 };
                let side = side(path, i) * s;
                let foot = path.points[i] + side * WALL_OFFSET;
                if !clear(network, foot, WALL_OFFSET) {
                    continue;
                }
                let base = vec3(foot.x, terrain_height(foot.x, foot.y) - 0.3, foot.y);
                let arm = path.points[i] + side * (WALL_OFFSET - 5.0);
                let head = vec3(arm.x, base.y + FLOODLIGHT_HEIGHT, arm.y);
                let pool = path.points[i] + side * 6.0;
                let rings = POOL_RINGS.iter().flat_map(|&r| (0..POOL_SIDES).map(move |k| {
                    let a = std::f32::consts::TAU * k as f32 / POOL_SIDES as f32;
                    let p = pool + Vec2::new(a.cos(), a.sin()) * r * POOL_RADIUS;
                    vec3(p.x, terrain_height(p.x, p.y) + 0.2, p.y)
                })).collect();
                lights.push(Floodlight { base, head, pool, rings });
            }
        }
        lights
    })
}

/// Everything trackside anchored in the chunk `inside` accepts.
pub fn build(b: &mut Builder, network: &RoadNetwork, inside: &impl Fn(Vec2) -> bool) {
    for light in floodlights(network).iter().filter(|l| inside(Vec2::new(l.base.x, l.base.z))) {
        floodlight(b, light);
    }
    let layouts = layouts(network);
    for (path, layout) in network.paths.iter().filter(|p| p.circuit()).zip(layouts) {
        let n = path.points.len();
        for corner in &layout.corners {
            tyre_wall(b, network, path, corner, inside);
            if corner.tight {
                for (distance, label) in BOARD_DISTANCES {
                    let i = (corner.start + n - (distance / crate::road::SAMPLE_SPACING) as usize) % n;
                    let p = path.points[i] + side(path, i) * corner.outside * BOARD_OFFSET;
                    if inside(path.points[i]) && !near_start(path, i) && clear(network, p, BOARD_OFFSET) {
                        braking_board(b, p, path.tangent(i), label);
                    }
                }
            }
        }
        for &(first, last) in &layout.straights {
            for i in (first + 6..last.saturating_sub(6)).step_by(15) {
                if !inside(path.points[i]) || near_start(path, i) {
                    continue;
                }
                for s in [-1.0, 1.0] {
                    let p = path.points[i] + side(path, i) * s * HOARDING_OFFSET;
                    if clear(network, p, HOARDING_OFFSET) {
                        let pick = (hash(p.x * 0.13, p.y * 0.29) * SPONSORS.len() as f32) as usize % SPONSORS.len();
                        hoarding(b, p, -side(path, i) * s, SPONSORS[pick]);
                    }
                }
            }
        }
    }
}

/// A slim grey mast with an arm out toward the track and a lamp head that
/// shines after dark.
fn floodlight(b: &mut Builder, light: &Floodlight) {
    let top = vec3(light.base.x, light.head.y, light.base.z);
    b.limb(light.base, top, 0.35, 0.25, 6, 0.0, POST);
    b.post(light.base, 0.5, FLOODLIGHT_HEIGHT);
    b.limb(top, light.head, 0.15, 0.15, 4, 0.0, POST);
    let toward = (light.head - top).normalize_or_zero();
    let yaw = (-toward.x).atan2(-toward.z);
    b.block(light.head - Vec3::Y * 0.6, 1.4, 0.9, 0.9, yaw, [0.2, 0.2, 0.22, 1.0], [0.2, 0.2, 0.22, 1.0]);
    b.glowing = true;
    b.block(light.head - Vec3::Y * 0.75, 1.2, 0.15, 0.75, yaw, [1.0, 0.96, 0.85, 1.0], [1.0, 0.96, 0.85, 1.0]);
    b.glowing = false;
}

/// Stacked tyres round the outside of a corner, a red-and-white band on top.
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
        let (ya, yc) = (terrain_height(a.x, a.y) - 0.3, terrain_height(c.x, c.y) - 0.3);
        let along = (c - a).normalize_or_zero();
        let across = Vec2::new(-along.y, along.x) * 0.8;
        let corner3 = |p: Vec2, y: f32| vec3(p.x, y, p.y);
        // Three tyres to a 4-unit stretch: alternate shades read as stacks
        for t in 0..3 {
            let (f0, f1) = (t as f32 / 3.0, (t + 1) as f32 / 3.0);
            let (p0, p1) = (a.lerp(c, f0), a.lerp(c, f1));
            let (y0, y1) = (ya + (yc - ya) * f0, ya + (yc - ya) * f1);
            let col = if (k * 3 + t) % 2 == 0 { TYRE } else { TYRE_DARK };
            for s in [-1.0, 1.0] {
                let (q0, q1) = (p0 + across * s, p1 + across * s);
                b.quad(corner3(q0, y0), corner3(q1, y1), corner3(q1, y1 + WALL_HEIGHT * 0.65), corner3(q0, y0 + WALL_HEIGHT * 0.65), col);
                let band = BAND[(k / 2) % 2];
                b.quad(corner3(q0, y0 + WALL_HEIGHT * 0.65), corner3(q1, y1 + WALL_HEIGHT * 0.65), corner3(q1, y1 + WALL_HEIGHT), corner3(q0, y0 + WALL_HEIGHT), band);
            }
            b.quad(corner3(p0 - across, y0 + WALL_HEIGHT), corner3(p1 - across, y1 + WALL_HEIGHT), corner3(p1 + across, y1 + WALL_HEIGHT), corner3(p0 + across, y0 + WALL_HEIGHT), TYRE);
        }
        let mid = (a + c) * 0.5;
        let yaw = (-along.x).atan2(-along.y);
        b.solid_block(vec3(mid.x, ya.min(yc), mid.y), 0.8, WALL_HEIGHT + (ya - yc).abs(), a.distance(c) * 0.5, yaw, TYRE, TYRE);
    }
}

/// A countdown board on a post beside the approach to a tight corner.
fn braking_board(b: &mut Builder, p: Vec2, ahead: Vec2, label: &str) {
    let ground = terrain_height(p.x, p.y);
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

/// A sponsor's hoarding on two legs, facing the track (`facing` points at it).
fn hoarding(b: &mut Builder, p: Vec2, facing: Vec2, (name, back, ink): (&str, [f32; 4], [f32; 4])) {
    let ground = terrain_height(p.x, p.y);
    let f = vec3(facing.x, 0.0, facing.y);
    let right = vec3(-facing.y, 0.0, facing.x);
    let yaw = (-facing.x).atan2(-facing.y);
    let (half_len, low, high) = (7.0, ground + 0.5, ground + 3.2);
    for s in [-1.0, 1.0] {
        let leg = vec3(p.x, ground - 0.3, p.y) + right * s * (half_len - 0.8);
        b.limb(leg, leg + Vec3::Y * 0.9, 0.1, 0.1, 4, 0.0, POST);
    }
    b.solid_block(vec3(p.x, low, p.y), half_len, high - low, 0.15, yaw, HOARDING_BACK, HOARDING_BACK);
    let at = |x: f32, y: f32| vec3(p.x, y, p.y) + right * x + f * 0.17;
    b.quad(at(-half_len, low), at(half_len, low), at(half_len, high), at(-half_len, high), back);
    let px = (12.0 / text_units(name)).min(0.3);
    let width = text_units(name) * px;
    let origin = vec3(p.x, (low + high) * 0.5 + 3.5 * px, p.y) + f * 0.2 - right * width * 0.5;
    lettering(b, origin, right, -Vec3::Y, px, name, ink);
}
