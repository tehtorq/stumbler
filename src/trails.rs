//! Dirt trails from the road network out to landmarks, routed across the
//! terrain around water, steep ground and circuits. Trails can branch off
//! earlier trails, so remote landmarks end up on a network of their own.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use glam::Vec2;

use crate::road::{ease_tight_corners, resample, PathKind, RoadNetwork, CIRCUIT_SHOULDER, ROAD_HALF_WIDTH, TRAIL_HALF_WIDTH};
use crate::terrain::{self, TERRAIN_HALF_SIZE, WATER_LEVEL, WORLD_SCALE};
use crate::track::Track;

/// Grid spacing for route finding.
const CELL: f32 = 30.0;
/// How far from a landmark to look for somewhere to join: a quick search
/// first, then a wide one for whatever is left.
const SEARCH_RADII: [f32; 2] = [3000.0, 7000.0 * WORLD_SCALE / 2.0];
/// Trails stop this far short of the landmark itself.
const STOP_SHORT: f32 = 40.0;
/// Trails keep this far from circuit centrelines, so they never cut a lap.
const CIRCUIT_CLEARANCE: f32 = ROAD_HALF_WIDTH + CIRCUIT_SHOULDER + TRAIL_HALF_WIDTH + 20.0;
/// Extra route cost for joining a circuit rather than a road or trail, so a
/// trail only does it when the landmark is boxed in by the circuit.
const CIRCUIT_JOIN_COST: f32 = 1500.0;
/// Tightest bend on a trail, so switchback legs stay clear of each other.
const MIN_CORNER_RADIUS: f32 = 28.0;
/// Gradients well above this get expensive, so trails wind up hills.
const COMFORTABLE_SLOPE: f32 = 0.12;

/// A trail to each site that can reach a road, a circuit or another trail.
/// Sites nearest the roads are routed first so further ones can branch off.
pub fn route_all(network: &RoadNetwork, sites: &[Vec2]) -> Vec<Track> {
    let network_points: Vec<Vec2> = network.paths.iter().flat_map(|p| p.points.iter().step_by(4).copied()).collect();
    let reach = |p: Vec2| network_points.iter().map(|q| q.distance_squared(p)).fold(f32::MAX, f32::min);
    let mut order: Vec<usize> = (0..sites.len()).collect();
    order.sort_by(|&a, &b| reach(sites[a]).total_cmp(&reach(sites[b])));

    // Routed in waves, a wave's worth at once: each can branch off the
    // trails of the waves before it
    let wave = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut existing = TrailPoints::default();
    let mut trails = Vec::new();
    for radius in SEARCH_RADII {
        let mut unreached = Vec::new();
        for batch in order.chunks(wave) {
            let routed = crate::layout::parallel_map(batch.to_vec(), |i| (i, route(network, &existing, sites[i], radius)));
            let mut this_wave = TrailPoints::default();
            for (i, mut trail) in routed {
                // One that runs into another of this wave (which it couldn't
                // see) is routed again, now able to join it
                if trail.as_ref().is_some_and(|t| t.points_2d.iter().any(|&p| this_wave.within(p, CELL * 1.5))) {
                    trail = route(network, &existing, sites[i], radius);
                }
                if let Some(t) = &trail {
                    this_wave.add(&t.points_2d);
                }
                match trail {
                    Some(trail) => {
                        existing.add(&trail.points_2d);
                        trails.push(trail);
                    }
                    None => unreached.push(i),
                }
            }
        }
        order = unreached;
    }
    trails
}

/// A walkable route from the nearest road to `site` (round water, off the
/// circuits), as trail routing would lay it, but without building anything.
pub fn path_to(network: &RoadNetwork, site: Vec2) -> Option<Vec<Vec2>> {
    let none = TrailPoints::default();
    SEARCH_RADII.iter().find_map(|&r| route(network, &none, site, r)).map(|t| t.points_2d)
}

/// Points along trails routed so far, bucketed by grid cell.
#[derive(Default)]
struct TrailPoints(HashMap<(i32, i32), Vec<Vec2>>);

impl TrailPoints {
    fn key(p: Vec2) -> (i32, i32) {
        ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
    }

    fn add(&mut self, points: &[Vec2]) {
        for &p in points {
            self.0.entry(Self::key(p)).or_default().push(p);
        }
    }

    fn within(&self, p: Vec2, dist: f32) -> bool {
        let (kx, kz) = Self::key(p);
        (-1..=1).any(|dz| (-1..=1).any(|dx| {
            self.0.get(&(kx + dx, kz + dz)).is_some_and(|v| v.iter().any(|q| q.distance(p) < dist))
        }))
    }
}

#[derive(Clone, Copy)]
enum Cell {
    Open(f32),
    NearCircuit,
    Blocked,
}

/// Cheapest path over a grid from `site` to the first road or trail it meets
/// (or failing that, a circuit), as a smooth trail starting at the join.
fn route(network: &RoadNetwork, existing: &TrailPoints, site: Vec2, radius: f32) -> Option<Track> {
    let r = (radius / CELL) as i32;
    let dim = 2 * r + 1;
    let pos = |i: usize| site + Vec2::new((i as i32 % dim - r) as f32, (i as i32 / dim - r) as f32) * CELL;
    let start = (r * dim + r) as usize;

    let mut cells: Vec<Option<Cell>> = vec![None; (dim * dim) as usize];
    let mut cell = |i: usize| {
        *cells[i].get_or_insert_with(|| {
            let p = pos(i);
            if p.x.abs().max(p.y.abs()) > TERRAIN_HALF_SIZE - 300.0 || terrain::raw_height(p.x, p.y) < WATER_LEVEL + 1.5 {
                Cell::Blocked
            } else if network.nearest_matching(p.x, p.y, |k| k == PathKind::Circuit).is_some_and(|s| s.dist < CIRCUIT_CLEARANCE) {
                Cell::NearCircuit
            } else {
                Cell::Open(terrain::natural_height(p.x, p.y))
            }
        })
    };
    let joins = |p: Vec2| {
        network.nearest_matching(p.x, p.y, |k| k == PathKind::Road).is_some_and(|s| s.dist < CELL * 0.75)
            || existing.within(p, CELL * 0.75)
    };

    // Entries are (cost bits, cell, whether this is a circuit join)
    let mut cost = vec![f32::INFINITY; (dim * dim) as usize];
    let mut from = vec![usize::MAX; (dim * dim) as usize];
    let mut open = BinaryHeap::new();
    cost[start] = 0.0;
    open.push(Reverse((0u32, start, false)));
    let mut goal = None;
    while let Some(Reverse((bits, i, circuit_join))) = open.pop() {
        let c = f32::from_bits(bits);
        if circuit_join {
            goal = Some((i, true));
            break;
        }
        if c > cost[i] {
            continue;
        }
        if i != start && joins(pos(i)) {
            goal = Some((i, false));
            break;
        }
        let Cell::Open(h) = cell(i) else { continue };
        let (cx, cz) = (i as i32 % dim, i as i32 / dim);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (cx + dx, cz + dz);
            if nx < 0 || nz < 0 || nx >= dim || nz >= dim || (nx - r).pow(2) + (nz - r).pow(2) > r * r {
                continue;
            }
            let j = (nz * dim + nx) as usize;
            let step = CELL * if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            // Positive floats order the same as their bit patterns
            let hj = match cell(j) {
                Cell::Open(hj) => hj,
                Cell::NearCircuit => {
                    if from[j] == usize::MAX {
                        from[j] = i;
                        open.push(Reverse(((c + step + CIRCUIT_JOIN_COST).to_bits(), j, true)));
                    }
                    continue;
                }
                Cell::Blocked => continue,
            };
            let slope = (hj - h).abs() / step;
            let next = c + step * (1.0 + 3.0 * (slope / COMFORTABLE_SLOPE).powi(2));
            if next < cost[j] {
                cost[j] = next;
                from[j] = i;
                open.push(Reverse((next.to_bits(), j, false)));
            }
        }
    }

    // Walk back from the join to the landmark
    let (goal, circuit_join) = goal?;
    let mut path = vec![goal];
    while let Some(&last) = path.last() {
        if last == start {
            break;
        }
        path.push(from[last]);
    }
    let mut control: Vec<Vec2> = path.iter().map(|&i| pos(i)).step_by(2).chain(std::iter::once(site)).collect();
    if circuit_join {
        // Run on until it meets the circuit's asphalt
        let p = control[0];
        let edge = network.paths.iter()
            .filter(|path| path.circuit())
            .flat_map(|path| path.points.iter().copied())
            .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))?;
        control.insert(0, edge + (p - edge).normalize_or_zero() * ROAD_HALF_WIDTH * 0.5);
    }
    if control.len() < 3 {
        return None;
    }
    // Iron out the grid's staircase, keeping both ends where they are
    for _ in 0..4 {
        let prev = control.clone();
        for i in 1..control.len() - 1 {
            control[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }

    // Round off the grid's zigzags and any tight switchbacks, then stop short of the landmark
    let mut points = resample(&Track::from_control_points(&control, false).points_2d, false, 4.0);
    for _ in 0..40 {
        let prev = points.clone();
        for i in 1..points.len() - 1 {
            points[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }
    let mut points = ease_tight_corners(&points, false, MIN_CORNER_RADIUS);
    points.retain(|p| p.distance(site) > STOP_SHORT);
    (points.len() >= 8).then(|| Track { points_2d: points, closed: false })
}
