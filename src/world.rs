//! Static world: layout, road network (with landmarks and roadside signs),
//! start gates and boost pads, plus
//! simplified outlines for the minimap. Terrain, trees and props stream in
//! through `chunks`.

use glam::{Vec2, Vec3};

use crate::boosts::{self, BoostPad};
use crate::geometry::Vertex;
use crate::gpu::{Gpu, Mesh};
use crate::road::{self, PathKind, RoadNetwork, ROAD_HALF_WIDTH, KERB_WIDTH};
use crate::terrain::{self, terrain_height};
use crate::{farms, landmarks, layout, railway, roadside, stages, trails};

/// A place to race: a circuit, or a gravel stage from one place to another.
pub struct CircuitInfo {
    pub name: &'static str,
    /// Evenly spaced points: a closed loop for a circuit, start to end for a
    /// stage.
    pub points: Vec<Vec2>,
    /// Outline for maps (every few points).
    pub outline: Vec<Vec2>,
    pub centre: Vec2,
    /// A gravel stage (else a circuit).
    pub stage: bool,
    /// Where the start line is (0 on a circuit; a stage's grid is behind it).
    pub start_idx: usize,
}

impl CircuitInfo {
    pub fn start(&self) -> Vec2 {
        self.points[self.start_idx]
    }

    /// Which way the race sets off over the start line.
    pub fn start_dir(&self) -> Vec2 {
        (self.points[self.start_idx + 1] - self.points[self.start_idx]).normalize_or_zero()
    }

    /// A stage's finish line (a circuit finishes where it starts).
    pub fn finish_idx(&self) -> usize {
        if self.stage { self.points.len() - 1 - crate::race::STAGE_RUNOFF } else { 0 }
    }

    /// Half the width a gantry over the start or finish must span.
    fn gate_half(&self) -> f32 {
        if self.stage { road::GRAVEL_HALF_WIDTH + 5.0 } else { GATE_HALF }
    }
}

pub struct World {
    pub circuits: Vec<CircuitInfo>,
    /// Connecting roads as out-and-back loops for cruising traffic.
    pub road_loops: Vec<Vec<Vec2>>,
    /// Connecting road outlines for maps.
    pub road_outlines: Vec<Vec<Vec2>>,
    /// Dirt trail outlines for maps.
    pub trail_outlines: Vec<Vec<Vec2>>,
    pub boost_pads: Vec<BoostPad>,
    pub gate_mesh: Mesh,
    pub boost_mesh: Mesh,
}

impl World {
    pub fn build(gpu: &Gpu) -> Self {
        // The road network must exist before anything samples terrain_height
        let t0 = std::time::Instant::now();
        let layout = layout::build(terrain::TERRAIN_HALF_SIZE);
        eprintln!("[timing] layout {:.2}s", t0.elapsed().as_secs_f32());
        let names: Vec<&'static str> = layout.circuits.iter().map(|c| c.name).collect();
        let network = road::install(|| build_network(layout));
        eprintln!("[timing] roads, railway, landmarks and trails {:.2}s", t0.elapsed().as_secs_f32());

        // Circuits, then the gravel stages
        let circuits: Vec<CircuitInfo> = network.paths.iter()
            .filter(|p| p.circuit())
            .zip(names)
            .chain(network.paths.iter().filter(|p| p.kind == PathKind::Gravel).zip(stages::names().iter().copied()))
            .map(|(p, name)| CircuitInfo {
                name,
                outline: p.points.iter().step_by(8).chain(p.points.last().filter(|_| !p.closed)).copied().collect(),
                centre: p.points.iter().copied().sum::<Vec2>() / p.points.len() as f32,
                points: p.points.clone(),
                stage: !p.closed,
                start_idx: if p.closed { 0 } else { crate::race::STAGE_START },
            })
            .collect();
        let roads: Vec<&road::RoadPath> = network.paths.iter().filter(|p| p.kind == PathKind::Road).collect();
        let road_loops = roads.iter()
            .map(|p| {
                let mut points = p.points.clone();
                points.extend(p.points.iter().rev().skip(1).take(p.points.len().saturating_sub(2)));
                points
            })
            .collect();
        let road_outlines = roads.iter().map(|p| p.points.iter().step_by(10).copied().collect()).collect();
        let trail_outlines = network.paths.iter()
            .filter(|p| p.kind == PathKind::Trail)
            .map(|p| p.points.iter().step_by(6).chain(p.points.last()).copied().collect())
            .collect();

        let (gate_verts, gate_idxs) = build_gates(&circuits);
        let boost_pads = boosts::create_boost_pads(network);
        let (boost_verts, boost_idxs) = boosts::build_boost_mesh(&boost_pads);

        Self {
            circuits,
            road_loops,
            road_outlines,
            trail_outlines,
            boost_pads,
            gate_mesh: Mesh::new(gpu, &gate_verts, &gate_idxs),
            boost_mesh: Mesh::new(gpu, &boost_verts, &boost_idxs),
        }
    }

    /// The circuit or stage whose start line is nearest `p`, with the distance.
    pub fn nearest_start(&self, p: Vec2) -> Option<(usize, f32)> {
        self.circuits.iter().enumerate()
            .map(|(i, c)| (i, c.start().distance(p)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Circuits and connecting roads from `layout`, the railway routed between
/// them, then landmarks sited against them all (installed for
/// `landmarks::all`) and a dirt trail out to each that isn't a secret.
pub fn build_network(layout: layout::Layout) -> RoadNetwork {
    let names: Vec<&'static str> = layout.circuits.iter().map(|c| c.name).collect();
    let circuits: Vec<_> = layout.circuits.into_iter().map(|c| c.track).collect();
    let mut network = RoadNetwork::build(&circuits, &layout.roads, terrain::TERRAIN_HALF_SIZE, terrain::natural_height);
    network.add_rail(&railway::route(&network), terrain::natural_height);
    network.add_gravel(&stages::route_all(&network), terrain::natural_height);
    let sites = landmarks::place(&network);
    // Secrets get no trail; finding them is the point
    let positions: Vec<Vec2> = sites.iter().filter(|l| !l.kind.is_secret()).map(|l| l.pos).collect();
    network.add_trails(&trails::route_all(&network, &positions), terrain::natural_height);
    landmarks::install(sites, &network);
    farms::install(&network);
    roadside::install(&names, &network);
    railway::install(&network);
    network
}

/// Where the five start lights hang on `c`'s start gantry: in a row
/// under the banner, on the side facing the grid.
pub fn start_lamps(c: &CircuitInfo) -> [Vec3; 5] {
    let p0 = c.start();
    let dir = c.start_dir();
    let side = Vec2::new(-dir.y, dir.x);
    let (left, right) = (p0 + side * c.gate_half(), p0 - side * c.gate_half());
    let top = terrain_height(left.x, left.y).max(terrain_height(right.x, right.y)) + POLE_HEIGHT;
    std::array::from_fn(|k| {
        let p = p0 + side * (k as f32 - 2.0) * 3.8 - dir * 0.9;
        Vec3::new(p.x, top - 6.4, p.y)
    })
}

const GATE_HALF: f32 = ROAD_HALF_WIDTH + KERB_WIDTH + 4.0;
/// Tall enough for the chase camera to pass under.
const POLE_HEIGHT: f32 = 26.0;

/// Start/finish gantry over each circuit's first point (and over each
/// stage's start and its finish): two poles, a banner, and at a start the
/// lights' housings.
fn build_gates(circuits: &[CircuitInfo]) -> (Vec<Vertex>, Vec<u16>) {
    let mut verts = Vec::new();
    let mut idxs = Vec::new();
    let pole_color: [f32; 4] = [0.85, 0.85, 0.85, 1.0];
    let banner_color: [f32; 4] = [0.9, 0.1, 0.1, 1.0];
    let banner_stripe: [f32; 4] = [0.95, 0.95, 0.95, 1.0];

    let gates = circuits.iter().flat_map(|c| {
        let finish = c.stage.then(|| (c, c.points[c.finish_idx()], (c.points[c.finish_idx() + 1] - c.points[c.finish_idx()]).normalize_or_zero(), false));
        std::iter::once((c, c.start(), c.start_dir(), true)).chain(finish)
    });
    for (c, p0, dir, start) in gates {
        let side = Vec2::new(-dir.y, dir.x);
        let gate_half = c.gate_half();
        let pole_h = POLE_HEIGHT;
        let banner_h = 4.0;

        let left = p0 + side * gate_half;
        let right = p0 - side * gate_half;
        let ly = terrain_height(left.x, left.y);
        let ry = terrain_height(right.x, right.y);
        let top = ly.max(ry) + pole_h;

        for (pos, ground) in [(left, ly), (right, ry)] {
            crate::geometry::push_box(
                &mut verts, &mut idxs,
                pos.x, (ground + top) * 0.5, pos.y,
                0.8, (top - ground) * 0.5, 0.8,
                [pole_color; 6],
            );
        }

        // Banner: a thin slab spanning the poles, with a white stripe along the bottom
        let depth = dir * 0.6;
        for (y0, y1, color) in [(top - banner_h, top, banner_color), (top - banner_h - 0.6, top - banner_h, banner_stripe)] {
            let b = verts.len() as u16;
            for (p, y) in [(left - depth, y0), (right - depth, y0), (right - depth, y1), (left - depth, y1),
                           (left + depth, y0), (right + depth, y0), (right + depth, y1), (left + depth, y1)] {
                verts.push(Vertex { pos: [p.x, y, p.y], color });
            }
            // front, back, top, bottom
            for q in [[0, 1, 2, 3], [5, 4, 7, 6], [3, 2, 6, 7], [4, 5, 1, 0]] {
                idxs.extend_from_slice(&[b + q[0], b + q[1], b + q[2], b + q[0], b + q[2], b + q[3]]);
            }
        }
        // Start light housings, dark until the lamps in them come on
        for lamp in start_lamps(c).into_iter().filter(|_| start) {
            crate::geometry::push_box(&mut verts, &mut idxs, lamp.x, lamp.y, lamp.z, 1.3, 1.3, 1.3, [[0.08, 0.08, 0.09, 1.0]; 6]);
        }
    }
    (verts, idxs)
}
