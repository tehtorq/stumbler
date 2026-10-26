//! Gravel stages: point-to-point roads through the woods, raced from a
//! start somewhere out in the country to a finish somewhere else. Each is
//! routed across the terrain through its waypoints, round steep ground and
//! the circuits, preferring the trees, and over water on a causeway only
//! where there's no way round.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::sync::OnceLock;

use glam::Vec2;

use crate::road::{ease_tight_corners, resample, PathKind, RoadNetwork, CIRCUIT_SHOULDER, GRAVEL_HALF_WIDTH, ROAD_HALF_WIDTH};
use crate::terrain::{self, TERRAIN_HALF_SIZE, WATER_LEVEL};
use crate::track::Track;

/// A stage: its name and the places it winds through, start to finish, in
/// the original layout (see `StageDef::spread`).
pub struct StageDef {
    pub name: &'static str,
    pub waypoints: &'static [[f32; 2]],
}

pub const STAGES: [StageDef; 3] = [
    // Back and forth down through the deep forest between Spa, the
    // Hungaroring, Zandvoort and Montreal
    StageDef {
        name: "WILDWOOD RUN",
        waypoints: &[[-5200.0, 4100.0], [-6200.0, 4600.0], [-4200.0, 4400.0], [-6000.0, 5300.0], [-3900.0, 5800.0], [-5700.0, 6000.0], [-4900.0, 7200.0], [-5600.0, 7900.0], [-4300.0, 8300.0], [-5000.0, 8800.0]],
    },
    // Weaving round the back of Las Vegas through the western woods, then
    // north past Spa, over the lake on a causeway, to the deep forest's edge
    StageDef {
        name: "PINE RIDGE",
        waypoints: &[[-5400.0, -1450.0], [-6600.0, -1700.0], [-7200.0, -2600.0], [-8300.0, -2300.0], [-7800.0, -1400.0], [-8700.0, -300.0], [-7900.0, 300.0], [-8800.0, 1200.0], [-7400.0, 1700.0], [-6200.0, 2000.0], [-6100.0, 3200.0], [-5700.0, 3800.0]],
    },
    // East from Imola's woods over the railway into the foothills, round
    // and back toward the Red Bull Ring
    StageDef {
        name: "HIGHLAND TIMBER",
        waypoints: &[[-900.0, 4700.0], [1500.0, 5000.0], [3500.0, 5600.0], [5500.0, 5000.0], [6400.0, 5800.0], [4500.0, 6300.0], [2800.0, 6000.0], [1200.0, 6400.0], [-300.0, 5600.0], [-1200.0, 6200.0]],
    },
];

impl StageDef {
    /// The waypoints in the world: the stage moves out with the layout to
    /// stay among the same circuits, but keeps its own length and shape.
    pub fn spread(&self) -> Vec<[f32; 2]> {
        let n = self.waypoints.len() as f32;
        let centre = self.waypoints.iter().fold(Vec2::ZERO, |sum, w| sum + Vec2::from(*w)) / n;
        let shift = centre * (terrain::WORLD_SCALE - 1.0);
        self.waypoints.iter().map(|w| (Vec2::from(*w) + shift).to_array()).collect()
    }
}

/// Grid spacing for route finding.
const CELL: f32 = 25.0;
/// How far a leg may stray outside the box round its two ends.
const MARGIN: f32 = 1200.0;
/// Stages keep this far from circuit centrelines.
const CIRCUIT_CLEARANCE: f32 = ROAD_HALF_WIDTH + CIRCUIT_SHOULDER + GRAVEL_HALF_WIDTH + 60.0;
/// A stage keeps this far from its own earlier stretches, and from other
/// stages.
const SELF_GAP: f32 = 120.0;
/// Tightest bend on a stage.
const MIN_CORNER_RADIUS: f32 = 45.0;
/// How much dearer a stretch across water (on a causeway) is than on land.
const WATER_COST: f32 = 8.0;
/// Gradients well above this get expensive, so stages wind up hills.
const COMFORTABLE_SLOPE: f32 = 0.14;

static ROUTED: OnceLock<Vec<&'static str>> = OnceLock::new();

/// Every stage that can be routed, as open paths from start to finish, in
/// the order of `names`.
pub fn route_all(network: &RoadNetwork) -> Vec<Track> {
    // Each stage keeps clear of those laid before it
    let mut taken = HashSet::new();
    let routed: Vec<(&'static str, Track)> = STAGES.iter().filter_map(|s| {
        let track = route(network, &taken, &s.spread())?;
        mark(&mut taken, &track.points_2d);
        Some((s.name, track))
    }).collect();
    let _ = ROUTED.set(routed.iter().map(|(name, _)| *name).collect());
    routed.into_iter().map(|(_, t)| t).collect()
}

/// The names of the stages `route_all` laid out, in order.
pub fn names() -> &'static [&'static str] {
    ROUTED.get().map_or(&[], |v| v.as_slice())
}

/// Whether a stage road may go at `p`: on the map, clear of the circuits.
pub(crate) fn open(network: &RoadNetwork, p: Vec2) -> bool {
    p.x.abs().max(p.y.abs()) < TERRAIN_HALF_SIZE - 300.0
        && !network.nearest_matching(p.x, p.y, |k| k == PathKind::Circuit).is_some_and(|s| s.dist < CIRCUIT_CLEARANCE)
}

/// Mark the grid cells within `SELF_GAP` of `points` as taken.
fn mark(taken: &mut HashSet<(i32, i32)>, points: &[Vec2]) {
    let r = (SELF_GAP / CELL).ceil() as i32;
    for p in points {
        let c = (*p / CELL).floor();
        for dz in -r..=r {
            for dx in -r..=r {
                taken.insert((c.x as i32 + dx, c.y as i32 + dz));
            }
        }
    }
}

/// A smooth stage road through `waypoints`, each leg the cheapest way
/// across the ground, clear of the `taken` cells.
fn route(network: &RoadNetwork, taken: &HashSet<(i32, i32)>, waypoints: &[[f32; 2]]) -> Option<Track> {
    let mut control: Vec<Vec2> = Vec::new();
    // Ground near the legs laid so far (bar the end this leg starts from),
    // so the road never runs alongside or across itself
    let mut taken = taken.clone();
    let mut marked = 0;
    for leg in waypoints.windows(2) {
        let keep = control.len().saturating_sub((SELF_GAP * 1.2 / CELL) as usize);
        if keep > marked {
            mark(&mut taken, &control[marked..keep]);
            marked = keep;
        }
        let cells = leg_route(network, &taken, Vec2::from(leg[0]), Vec2::from(leg[1]))?;
        if !control.is_empty() {
            control.pop();
        }
        control.extend(cells.into_iter().step_by(2));
    }
    control.push(Vec2::from(*waypoints.last()?));
    // Iron out the grid's staircase, keeping both ends where they are
    for _ in 0..4 {
        let prev = control.clone();
        for i in 1..control.len() - 1 {
            control[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }
    let mut points = resample(&Track::from_control_points(&control, false).points_2d, false, 4.0);
    for _ in 0..12 {
        let prev = points.clone();
        for i in 1..points.len() - 1 {
            points[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }
    let points = ease_tight_corners(&points, false, MIN_CORNER_RADIUS);
    Some(Track { points_2d: resample(&points, false, 4.0), closed: false })
}

/// The cheapest path over a grid from `a` to `b` (both nudged onto open
/// ground), as the cells' centres in order.
fn leg_route(network: &RoadNetwork, taken: &HashSet<(i32, i32)>, a: Vec2, b: Vec2) -> Option<Vec<Vec2>> {
    let lo = a.min(b) - MARGIN;
    let hi = a.max(b) + MARGIN;
    let (w, h) = (((hi.x - lo.x) / CELL) as i32 + 1, ((hi.y - lo.y) / CELL) as i32 + 1);
    let pos = |i: usize| lo + Vec2::new((i as i32 % w) as f32, (i as i32 / w) as f32) * CELL;
    let index = |p: Vec2| {
        let c = ((p - lo) / CELL).round();
        (c.x >= 0.0 && c.y >= 0.0 && (c.x as i32) < w && (c.y as i32) < h).then(|| (c.y as i32 * w + c.x as i32) as usize)
    };
    let mut cells: Vec<Option<Option<f32>>> = vec![None; (w * h) as usize];
    let mut cell = |i: usize| {
        *cells[i].get_or_insert_with(|| {
            let p = pos(i);
            let k = (p / CELL).floor();
            (open(network, p) && !taken.contains(&(k.x as i32, k.y as i32))).then(|| terrain::natural_height(p.x, p.y))
        })
    };
    // The nearest open cell to a point, if it's in water or too near a circuit
    let settle = |p: Vec2, cell: &mut dyn FnMut(usize) -> Option<f32>| {
        (0..80).map(|r| r as f32 * CELL * 0.5)
            .flat_map(|r| (0..16).map(move |k| p + Vec2::from_angle(k as f32 * std::f32::consts::TAU / 16.0) * r))
            .filter_map(index)
            .find(|&j| cell(j).is_some())
    };
    let start = settle(a, &mut cell)?;
    let goal = settle(b, &mut cell)?;

    let mut cost = vec![f32::INFINITY; (w * h) as usize];
    let mut from = vec![usize::MAX; (w * h) as usize];
    let mut queue = BinaryHeap::new();
    cost[start] = 0.0;
    queue.push(Reverse((0u32, start)));
    let target = pos(goal);
    while let Some(Reverse((_, i))) = queue.pop() {
        if i == goal {
            break;
        }
        let Some(hi_) = cell(i) else { continue };
        let (cx, cz) = (i as i32 % w, i as i32 / w);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (cx + dx, cz + dz);
            if nx < 0 || nz < 0 || nx >= w || nz >= h {
                continue;
            }
            let j = (nz * w + nx) as usize;
            let Some(hj) = cell(j) else { continue };
            let p = pos(j);
            let step = CELL * if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            let slope = (hj - hi_).abs() / step;
            // Through the woods if it can: open ground costs a little more
            let wood = (crate::trees::density(p.x, p.y, hj) * 1.5).min(1.0);
            // Across a road or the railway, but not along beside one
            let beside = network.nearest_matching(p.x, p.y, |k| k > PathKind::Gravel).is_some_and(|s| s.dist < s.half_width() + s.shoulder() + 50.0);
            // Over water on a causeway only when there's no way round
            let wet = terrain::raw_height(p.x, p.y) < WATER_LEVEL + 1.5;
            let next = cost[i] + step * (1.0 + 3.0 * (slope / COMFORTABLE_SLOPE).powi(2)) * (1.3 - 0.4 * wood)
                * if beside { 4.0 } else { 1.0 } * if wet { WATER_COST } else { 1.0 };
            if next < cost[j] {
                cost[j] = next;
                from[j] = i;
                // A* toward the goal (positive floats order as their bits)
                queue.push(Reverse(((next + p.distance(target)).to_bits(), j)));
            }
        }
    }
    if from[goal] == usize::MAX && goal != start {
        return None;
    }
    let mut path = vec![goal];
    while let Some(&last) = path.last() {
        if last == start {
            break;
        }
        path.push(from[last]);
    }
    path.reverse();
    Some(path.into_iter().map(pos).collect())
}
