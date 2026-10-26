//! World layout: spreads the circuits evenly over the map, a grid slot each,
//! then links neighbouring circuits with connecting roads.

use glam::{Vec2, vec2};

use crate::road::{ease_tight_corners, resample, ROAD_HALF_WIDTH};
use crate::terrain::{self, WATER_LEVEL, WORLD_SCALE};
use crate::track::Track;
use crate::tracks;

/// Circuits sit on an even grid this far apart, the home circuit in the middle.
pub const GRID_SPACING: f32 = 3500.0 * WORLD_SCALE;
/// Grid steps from the centre to each edge (a 5 x 5 grid).
const GRID_REACH: i32 = 2;
/// Grid slots left for the high mountains in the north-east.
const MOUNTAIN_SLOTS: [(i32, i32); 2] = [(2, 2), (1, 2)];
/// How far a circuit may shift from its slot toward drier, flatter ground.
const NUDGE: f32 = 600.0;
/// Clear ground kept between neighbouring circuits.
const CIRCUIT_MARGIN: f32 = 450.0;
/// Non-adjacent parts of one circuit must stay at least this far apart.
const MIN_SELF_GAP: f32 = ROAD_HALF_WIDTH * 2.0 + 30.0;
/// Roads keep at least this far from circuits they don't connect to.
const ROAD_CLEARANCE: f32 = ROAD_HALF_WIDTH * 2.0 + 60.0;
/// Tightest corner allowed on a circuit. Much below the road's width the
/// inside edge folds over itself.
pub const MIN_CORNER_RADIUS: f32 = ROAD_HALF_WIDTH * 1.75;

pub struct Circuit {
    pub name: &'static str,
    pub track: Track,
}

pub struct Layout {
    pub circuits: Vec<Circuit>,
    pub roads: Vec<Track>,
}

/// Lay out all circuits inside a square map of the given half-size.
pub fn build(half_size: f32) -> Layout {
    let shapes = tracks::circuit_shapes();

    // Each outline about its centre, at its real size unless its own
    // sections would run into each other at the game's road width
    // (each circuit is independent, so they're fitted in parallel)
    let mut placed: Vec<(&'static str, Vec<Vec2>, Vec2, f32)> = parallel_map(shapes.into_iter().enumerate().collect(), |(i, (name, pts))| {
        let centre = pts.iter().copied().sum::<Vec2>() / pts.len() as f32;
        let local: Vec<Vec2> = pts.iter().map(|p| *p - centre).collect();
        // The home circuit is drawn where it belongs, at its final size
        let local = if i == 0 { local } else { fit_outline(&local) };
        let radius = local.iter().map(|p| p.length()).fold(0.0, f32::max) + CIRCUIT_MARGIN * 0.5;
        (name, local, centre, radius)
    });

    // An even grid of slots, each circuit in the one nearest where it suits
    assign_slots(&mut placed);
    let mut placed: Vec<_> = parallel_map(placed.into_iter().enumerate().collect(), |(i, (name, local, slot, radius))| {
        let centre = if i == 0 { slot } else { settle(slot, &local) };
        (name, local, centre, radius)
    });
    relax_positions(&mut placed, half_size);

    let circuits: Vec<Circuit> = parallel_map(placed, |(name, local, centre, _)| {
        let world: Vec<Vec2> = local.iter().map(|p| *p + centre).collect();
        let mut track = Track::from_control_points(&world, true);
        track.points_2d = ease_tight_corners(&track.points_2d, true, MIN_CORNER_RADIUS);
        Circuit { name, track }
    });

    let roads = connect(&circuits);
    Layout { circuits, roads }
}

/// `f` over `items` on a thread each, keeping their order.
pub fn parallel_map<T: Send, U: Send>(items: Vec<T>, f: impl Fn(T) -> U + Sync) -> Vec<U> {
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = items.into_iter().map(|item| scope.spawn(move || f(item))).collect();
        handles.into_iter().map(|h| h.join().expect("layout worker panicked")).collect()
    })
}

/// Each circuit's grid slot (x, z steps from the centre; +z is north),
/// chosen to suit the land there: the desert circuits in the south-west
/// desert, Spa and the Hungaroring in the north-west woods, the cold-weather
/// circuits up in the snow by the mountains, the coastal street circuits
/// along the south, and Baku by the volcano.
fn home_slot(name: &str) -> (i32, i32) {
    match name {
        // North: snow, by the mountains
        "ZANDVOORT" => (-2, 2),
        "MONTREAL" => (-1, 2),
        "RED BULL RING" => (0, 2),
        // Woods and hills
        "SPA-FRANCORCHAMPS" => (-2, 1),
        "HUNGARORING" => (-1, 1),
        "IMOLA" => (0, 1),
        "SUZUKA" => (1, 1),
        "SHANGHAI" => (2, 1),
        // Middle row, either side of the home circuit
        "LAS VEGAS" => (-2, 0),
        "BARCELONA" => (-1, 0),
        "MONZA" => (1, 0),
        "BAKU" => (2, 0),
        // South: desert to the west
        "JEDDAH" => (-2, -1),
        "QATAR" => (-1, -1),
        "COTA" => (0, -1),
        "INTERLAGOS" => (1, -1),
        "MONACO" => (2, -1),
        "BAHRAIN" => (-2, -2),
        "ABU DHABI" => (-1, -2),
        "MEXICO CITY" => (0, -2),
        "MIAMI" => (1, -2),
        "SINGAPORE" => (2, -2),
        _ => panic!("no grid slot for {name}"),
    }
}

/// Put every circuit but the home one (which stays in the centre) in its
/// own grid slot (see `home_slot`).
fn assign_slots(placed: &mut [(&'static str, Vec<Vec2>, Vec2, f32)]) {
    let mut used = vec![(0, 0)];
    for item in placed.iter_mut().skip(1) {
        let slot = home_slot(item.0);
        assert!(!used.contains(&slot) && !MOUNTAIN_SLOTS.contains(&slot), "{} given a taken slot {slot:?}", item.0);
        assert!(slot.0.abs() <= GRID_REACH && slot.1.abs() <= GRID_REACH, "{} off the grid", item.0);
        used.push(slot);
        item.2 = vec2(slot.0 as f32, slot.1 as f32) * GRID_SPACING;
    }
}

/// The spot within `NUDGE` of `slot` where the circuit (outline `local`)
/// has the least water and steep ground under it, staying near the slot.
fn settle(slot: Vec2, local: &[Vec2]) -> Vec2 {
    let samples: Vec<Vec2> = local.iter().step_by(3).copied().collect();
    let score = |centre: Vec2| {
        let (mut wet, mut steep) = (0.0, 0.0);
        for &p in &samples {
            let q = p + centre;
            let h = terrain::raw_height(q.x, q.y);
            if h < WATER_LEVEL + 2.0 {
                wet += 1.0;
            }
            let e = 25.0;
            let dx = terrain::raw_height(q.x + e, q.y) - terrain::raw_height(q.x - e, q.y);
            let dz = terrain::raw_height(q.x, q.y + e) - terrain::raw_height(q.x, q.y - e);
            steep += (dx * dx + dz * dz).sqrt() / (2.0 * e);
        }
        let n = samples.len().max(1) as f32;
        wet / n * 4.0 + steep / n + centre.distance(slot) / NUDGE * 0.2
    };
    let steps = 4;
    let step = NUDGE / steps as f32;
    (-steps..=steps)
        .flat_map(|i| (-steps..=steps).map(move |j| slot + vec2(i as f32, j as f32) * step))
        .filter(|c| c.distance(slot) <= NUDGE)
        .min_by(|a, b| score(*a).total_cmp(&score(*b)))
        .unwrap_or(slot)
}

/// A real circuit may be scaled up to make room for the game's wide roads,
/// but no further than this from its centre to its furthest point.
const MAX_RADIUS: f32 = 1400.0;
/// Sections running closer than this (other than real crossings) are eased apart.
const SPREAD_GAP: f32 = MIN_SELF_GAP + 8.0;
/// Spacing of the points the outline is worked on at.
const SPREAD_STEP: f32 = 8.0;

/// A real outline at a size and shape the game's wide roads fit: grown (as
/// far as `MAX_RADIUS` allows) while it's tight along much of its length,
/// then any sections still running too close are eased apart locally.
fn fit_outline(local: &[Vec2]) -> Vec<Vec2> {
    let curve = |scale: f32| {
        let pts: Vec<Vec2> = local.iter().map(|p| *p * scale).collect();
        resample(&Track::from_control_points(&pts, true).points_2d, true, SPREAD_STEP)
    };
    let radius = local.iter().map(|p| p.length()).fold(0.0, f32::max);
    let mut scale = 1.0;
    while (scale + 0.1) * radius <= MAX_RADIUS && tight_share(&curve(scale)) > 0.12 {
        scale += 0.1;
    }
    // Every few samples is plenty as control points: the spline runs through them
    spread_apart(curve(scale)).into_iter().step_by(4).collect()
}

/// Pairs of samples closer than `gap` in space but much further apart along
/// the path (the two legs of a hairpin, or sections running side by side),
/// leaving out places where the path really crosses itself.
fn close_pairs(samples: &[Vec2], crossings: &[Vec2], gap: f32) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    let n = samples.len();
    let key = |p: Vec2| ((p.x / gap).floor() as i32, (p.y / gap).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, p) in samples.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i);
    }
    let crossing = |p: Vec2| crossings.iter().any(|c| c.distance(p) < 120.0);
    let mut pairs = Vec::new();
    for i in 0..n {
        if crossing(samples[i]) {
            continue;
        }
        let (kx, kz) = key(samples[i]);
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &j in grid.get(&(kx + dx, kz + dz)).into_iter().flatten() {
                    let a = j.abs_diff(i);
                    let along = a.min(n - a) as f32 * SPREAD_STEP;
                    let d = samples[i].distance(samples[j]);
                    // A semicircle's ends are pi/2 times further apart along it
                    // than across, so this widens hairpins to at least `gap` across
                    if j > i && d < gap && along > d * 1.5 && !crossing(samples[j]) {
                        pairs.push((i, j));
                    }
                }
            }
        }
    }
    pairs
}

/// Share of the path with another (non-adjacent, non-crossing) section
/// closer than `SPREAD_GAP`.
fn tight_share(samples: &[Vec2]) -> f32 {
    let crossings = self_crossings(samples);
    let mut tight = vec![false; samples.len()];
    for (i, j) in close_pairs(samples, &crossings, SPREAD_GAP) {
        tight[i] = true;
        tight[j] = true;
    }
    tight.iter().filter(|&&t| t).count() as f32 / samples.len().max(1) as f32
}

/// Ease apart sections running closer than `SPREAD_GAP`: each close pair
/// pushes apart, and the pushes are smoothed along the path so whole
/// stretches shift together rather than kinking.
fn spread_apart(mut samples: Vec<Vec2>) -> Vec<Vec2> {
    let n = samples.len();
    let crossings = self_crossings(&samples);
    const WINDOW: i32 = 25;
    for _ in 0..150 {
        let pairs = close_pairs(&samples, &crossings, SPREAD_GAP);
        if pairs.is_empty() {
            break;
        }
        let mut push = vec![Vec2::ZERO; n];
        for (i, j) in pairs {
            let d = samples[i] - samples[j];
            let dir = d.try_normalize().unwrap_or(Vec2::X);
            let amount = (SPREAD_GAP - d.length()) * 0.15;
            push[i] += dir * amount;
            push[j] -= dir * amount;
        }
        // Spread each push along the path
        let smoothed: Vec<Vec2> = (0..n as i32)
            .map(|i| {
                let mut sum = Vec2::ZERO;
                let mut weight = 0.0;
                for o in -WINDOW..=WINDOW {
                    let w = 1.0 - o.abs() as f32 / (WINDOW + 1) as f32;
                    sum += push[(i + o).rem_euclid(n as i32) as usize] * w;
                    weight += w;
                }
                sum / weight * 2.0
            })
            .collect();
        for (p, d) in samples.iter_mut().zip(smoothed) {
            *p += d;
        }
    }
    samples
}

/// Smallest distance between parts of a closed path that aren't neighbours
/// along it, ignoring places where the path genuinely crosses itself.
#[cfg(test)]
pub fn self_gap(points: &[Vec2]) -> f32 {
    let samples = resample(points, true, 8.0);
    let n = samples.len();
    let near_along = (300.0 / 8.0) as usize;
    let crossings = self_crossings(&samples);
    let mut gap = f32::MAX;
    for i in 0..n {
        if crossings.iter().any(|c| c.distance(samples[i]) < 120.0) {
            continue;
        }
        for j in (i + near_along)..n {
            let along = (j - i).min(n - (j - i));
            if along < near_along {
                continue;
            }
            gap = gap.min(samples[i].distance(samples[j]));
        }
    }
    gap
}

fn self_crossings(samples: &[Vec2]) -> Vec<Vec2> {
    use std::collections::HashMap;
    let n = samples.len();
    // Bucket segments by cell, sized to the longest, so only neighbours are tested
    let cell = (0..n).map(|i| samples[i].distance(samples[(i + 1) % n])).fold(1.0, f32::max) * 2.0;
    let key = |p: Vec2| ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for i in 0..n {
        grid.entry(key(samples[i])).or_default().push(i);
    }
    let mut points = Vec::new();
    for i in 0..n {
        let (a, b) = (samples[i], samples[(i + 1) % n]);
        let (kx, kz) = key(a);
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &j in grid.get(&(kx + dx, kz + dz)).into_iter().flatten() {
                    if j < i + 2 || (j + 1) % n == i {
                        continue;
                    }
                    let (c, d) = (samples[j], samples[(j + 1) % n]);
                    if let Some(p) = segment_intersection(a, b, c, d) {
                        points.push(p);
                    }
                }
            }
        }
    }
    points
}

fn segment_intersection(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    let r = b - a;
    let s = d - c;
    let denom = r.perp_dot(s);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (c - a).perp_dot(s) / denom;
    let u = (c - a).perp_dot(r) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

/// Push circuits apart until their footprints don't overlap, keeping each
/// near its sketched position and inside the map. The home circuit stays put.
fn relax_positions(placed: &mut [(&'static str, Vec<Vec2>, Vec2, f32)], half_size: f32) {
    let n = placed.len();
    let edge = 600.0;
    for _ in 0..2000 {
        let mut moved = false;
        for i in 0..n {
            for j in (i + 1)..n {
                let (ci, ri) = (placed[i].2, placed[i].3);
                let (cj, rj) = (placed[j].2, placed[j].3);
                let delta = cj - ci;
                let dist = delta.length().max(1.0);
                let overlap = ri + rj - dist;
                if overlap <= 0.0 {
                    continue;
                }
                moved = true;
                let dir = if dist > 1.0 { delta / dist } else { vec2(1.0, 0.0) };
                let push = dir * (overlap * 0.5 + 1.0);
                match (i == 0, j == 0) {
                    (true, _) => placed[j].2 += push * 2.0,
                    (_, true) => placed[i].2 -= push * 2.0,
                    _ => {
                        placed[i].2 -= push;
                        placed[j].2 += push;
                    }
                }
            }
        }
        for item in placed.iter_mut().skip(1) {
            let limit = half_size - item.3 - edge;
            item.2 = item.2.clamp(Vec2::splat(-limit), Vec2::splat(limit));
        }
        if !moved {
            return;
        }
    }
}

/// Link circuits with roads: a minimum spanning tree so everything is
/// reachable, plus each circuit's nearest unlinked neighbour for some loops.
fn connect(circuits: &[Circuit]) -> Vec<Track> {
    let n = circuits.len();
    let centres: Vec<Vec2> = circuits.iter()
        .map(|c| c.track.points_2d.iter().copied().sum::<Vec2>() / c.track.points_2d.len() as f32)
        .collect();

    let mut pairs: Vec<(f32, usize, usize)> = (0..n)
        .flat_map(|i| ((i + 1)..n).map(move |j| (i, j)))
        .map(|(i, j)| (centres[i].distance(centres[j]), i, j))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Kruskal
    let mut group: Vec<usize> = (0..n).collect();
    fn root(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    let mut links: Vec<(usize, usize)> = Vec::new();
    for &(_, i, j) in &pairs {
        let (ri, rj) = (root(&mut group, i), root(&mut group, j));
        if ri != rj {
            group[ri] = rj;
            links.push((i, j));
        }
    }
    // Extra links: each circuit to its nearest neighbour it isn't linked to yet
    for i in 0..n {
        let has = |a: usize, b: usize, l: &[(usize, usize)]| l.iter().any(|&(x, y)| (x, y) == (a, b) || (x, y) == (b, a));
        if let Some(&(_, a, b)) = pairs.iter().find(|&&(d, a, b)| (a == i || b == i) && !has(a, b, &links) && d < GRID_SPACING * 1.3) {
            if links.iter().filter(|&&(x, y)| x == i || y == i).count() < 3 {
                links.push((a, b));
            }
        }
    }

    links.iter()
        .filter_map(|&(a, b)| route_road(circuits, a, b))
        .collect()
}

/// A road between the closest points of circuits `a` and `b`, wiggling a
/// little and detouring around any other circuit in the way.
fn route_road(circuits: &[Circuit], a: usize, b: usize) -> Option<Track> {
    let pa_all = &circuits[a].track.points_2d;
    let pb_all = &circuits[b].track.points_2d;
    let (pa, pb) = pa_all.iter().step_by(3)
        .flat_map(|p| pb_all.iter().step_by(3).map(move |q| (*p, *q)))
        .min_by(|x, y| x.0.distance_squared(x.1).total_cmp(&y.0.distance_squared(y.1)))?;

    let dir = (pb - pa).normalize_or_zero();
    let side = vec2(-dir.y, dir.x);
    let length = pa.distance(pb);
    let segments = ((length / 350.0).ceil() as usize).max(2);
    let seed = (a * 31 + b * 17) as f32;

    // Try the direct route first, then bend further and further each way
    for bend in [0.0, 500.0, -500.0, 1000.0, -1000.0, 1600.0, -1600.0, 2400.0, -2400.0].map(|b| b * WORLD_SCALE) {
        let control: Vec<Vec2> = (0..=segments)
            .map(|k| {
                let t = k as f32 / segments as f32;
                let arc = (t * std::f32::consts::PI).sin();
                let wiggle = if k == 0 || k == segments { 0.0 } else { ((seed + k as f32 * 1.7).sin()) * 90.0 };
                pa + (pb - pa) * t + side * (bend * arc + wiggle)
            })
            .collect();
        let track = Track::from_control_points(&control, false);
        let clear = track.points_2d.iter().all(|p| {
            circuits.iter().enumerate().all(|(ci, c)| {
                if ci == a && p.distance(pa) < 250.0 || ci == b && p.distance(pb) < 250.0 {
                    return true; // leaving or joining its own circuits
                }
                c.track.points_2d.iter().all(|q| q.distance_squared(*p) > ROAD_CLEARANCE * ROAD_CLEARANCE)
            })
        });
        if clear {
            return Some(track);
        }
    }
    None
}
