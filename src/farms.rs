//! Farmsteads along the connecting roads in the grasslands: a yard with a
//! farmhouse, barn and silo, and behind it a patchwork of fields (wheat,
//! rapeseed, sunflowers, young crops, fresh ploughing and pasture) divided by
//! hedgerows. Terrain colour, trees, ground cover and props all ask `at` what
//! a spot is.

use std::collections::HashMap;
use std::sync::OnceLock;

use glam::{Quat, Vec2, Vec3, vec3};

use crate::landmarks;
use crate::props::{ground, hash, Builder};
use crate::road::{PathKind, RoadNetwork, SAMPLE_SPACING};
use crate::terrain::{self, WATER_LEVEL};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Crop {
    Wheat,
    Rapeseed,
    Sunflowers,
    /// Young green shoots in rows.
    Shoots,
    Ploughed,
    Pasture,
}

impl Crop {
    /// Colour of the ground the crop grows in, as the terrain shows it.
    pub fn ground(self) -> [f32; 3] {
        match self {
            Crop::Wheat => [0.8, 0.66, 0.3],
            Crop::Rapeseed => [0.86, 0.78, 0.18],
            Crop::Sunflowers => [0.36, 0.48, 0.18],
            Crop::Shoots => [0.45, 0.62, 0.22],
            Crop::Ploughed => [0.4, 0.29, 0.19],
            Crop::Pasture => [0.3, 0.56, 0.18],
        }
    }
}

pub struct Field {
    pub centre: Vec2,
    /// Half extents along the farm's road and out from it.
    pub half: Vec2,
    pub crop: Crop,
    pub seed: f32,
}

pub struct Farm {
    /// Middle of the yard.
    pub yard: Vec2,
    /// Unit direction of the road past the farm.
    pub along: Vec2,
    /// Unit direction from the road out across the farm.
    pub out: Vec2,
    /// Where the road's centreline passes the farm gate.
    pub gate: Vec2,
    pub fields: Vec<Field>,
    /// Hedgerows as line segments (gaps for gateways already cut).
    pub hedges: Vec<(Vec2, Vec2)>,
    pub seed: f32,
}

impl Farm {
    /// `p` in the farm's frame: (along the road, out from its centreline).
    pub fn local(&self, p: Vec2) -> Vec2 {
        let d = p - self.gate;
        Vec2::new(d.dot(self.along), d.dot(self.out))
    }

    pub fn world(&self, l: Vec2) -> Vec2 {
        self.gate + self.along * l.x + self.out * l.y
    }
}

impl Field {
    /// `p` relative to the field's centre, in the farm's frame.
    pub fn local(&self, farm: &Farm, p: Vec2) -> Vec2 {
        let d = p - self.centre;
        Vec2::new(d.dot(farm.along), d.dot(farm.out))
    }
}

/// What's at a spot on a farm.
pub enum Spot<'a> {
    Field(&'a Field),
    /// The yard and the track in from the road.
    Yard,
}

// The farm's plan, in its own frame (along the road, out from the centreline):
// four columns of fields in two rows, the yard taking the middle of the row
// nearest the road
const COLUMNS: [f32; 5] = [-184.0, -92.0, 0.0, 92.0, 184.0];
const ROWS: [f32; 3] = [74.0, 155.0, 236.0];
/// Fields stop this far short of the hedge lines.
const INSET: f32 = 3.5;
/// Half-width of the track from the road to the yard.
const TRACK: f32 = 6.0;
/// Farms are at least this far apart.
const SPACING: f32 = 1300.0;
/// Grid cell for `at` lookups.
const CELL: f32 = 500.0;

struct Farms {
    farms: Vec<Farm>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

static FARMS: OnceLock<Farms> = OnceLock::new();

pub fn all() -> &'static [Farm] {
    FARMS.get().map_or(&[], |f| f.farms.as_slice())
}

/// Site farms along the roads. Landmarks must already be installed. Later
/// calls are ignored.
pub fn install(network: &RoadNetwork) {
    FARMS.get_or_init(|| {
        let farms = site(network);
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, farm) in farms.iter().enumerate() {
            let corners = [Vec2::new(COLUMNS[0], 0.0), Vec2::new(COLUMNS[4], 0.0), Vec2::new(COLUMNS[0], ROWS[2]), Vec2::new(COLUMNS[4], ROWS[2])]
                .map(|l| farm.world(l + l.signum() * 10.0));
            let lo = corners.iter().fold(Vec2::MAX, |a, b| a.min(*b));
            let hi = corners.iter().fold(Vec2::MIN, |a, b| a.max(*b));
            for cz in (lo.y / CELL).floor() as i32..=(hi.y / CELL).floor() as i32 {
                for cx in (lo.x / CELL).floor() as i32..=(hi.x / CELL).floor() as i32 {
                    grid.entry((cx, cz)).or_default().push(i);
                }
            }
        }
        Farms { farms, grid }
    });
}

/// Farms whose land might reach `p`.
fn near(p: Vec2) -> impl Iterator<Item = &'static Farm> {
    let farms = FARMS.get();
    let key = ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32);
    farms.and_then(|f| f.grid.get(&key)).into_iter().flatten().map(move |&i| &farms.unwrap().farms[i])
}

/// The field or yard at `p`, if any.
pub fn at(p: Vec2) -> Option<Spot<'static>> {
    for farm in near(p) {
        let l = farm.local(p);
        if l.x.abs() < COLUMNS[3] - INSET && (ROWS[0] + INSET..ROWS[1] - INSET).contains(&l.y)
            || l.x.abs() < TRACK && (30.0..ROWS[0] + INSET).contains(&l.y)
        {
            return Some(Spot::Yard);
        }
        if let Some(field) = farm.fields.iter().find(|f| {
            let fl = f.local(farm, p);
            fl.x.abs() < f.half.x && fl.y.abs() < f.half.y
        }) {
            return Some(Spot::Field(field));
        }
    }
    None
}

/// Whether `p` is within `margin` of farm land (fields, yard or hedges),
/// where trees and boulders shouldn't grow.
pub fn occupied(p: Vec2, margin: f32) -> bool {
    near(p).any(|farm| {
        let l = farm.local(p);
        l.x.abs() < COLUMNS[4] + margin && l.y > ROWS[0] - margin && l.y < ROWS[2] + margin
            || l.x.abs() < TRACK + margin && l.y > 20.0 && l.y < ROWS[0]
    })
}

fn site(network: &RoadNetwork) -> Vec<Farm> {
    let circuit_points: Vec<Vec2> = network.paths.iter()
        .filter(|p| p.circuit())
        .flat_map(|p| p.points.iter().step_by(6).copied())
        .collect();
    let mut farms: Vec<Farm> = Vec::new();
    for (r, road) in network.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
        let n = road.points.len();
        let margin = (300.0 / SAMPLE_SPACING) as usize;
        for i in (margin..n.saturating_sub(margin)).step_by((150.0 / SAMPLE_SPACING) as usize) {
            for s in [1.0, -1.0] {
                let s = if hash(r as f32, i as f32) < 0.5 { s } else { -s };
                let along = road.tangent(i);
                let out = Vec2::new(-along.y, along.x) * s;
                let mut farm = Farm { yard: Vec2::ZERO, along, out, gate: road.points[i], fields: Vec::new(), hedges: Vec::new(), seed: hash(i as f32, r as f32 * 1.7) * 100.0 };
                farm.yard = farm.world(Vec2::new(0.0, (ROWS[0] + ROWS[1]) * 0.5));
                if farms.iter().any(|f| f.yard.distance(farm.yard) < SPACING) || !suitable(&farm, network, &circuit_points) {
                    continue;
                }
                plan(&mut farm);
                farms.push(farm);
                break;
            }
        }
    }
    farms
}

/// Open, gently rolling grassland, clear of water, other paths, circuits and
/// landmarks.
fn suitable(farm: &Farm, network: &RoadNetwork, circuit_points: &[Vec2]) -> bool {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for gz in 0..=8 {
        for gx in 0..=8 {
            let l = Vec2::new(COLUMNS[0] + (COLUMNS[4] - COLUMNS[0]) * gx as f32 / 8.0, ROWS[0] - 8.0 + (ROWS[2] - ROWS[0] + 16.0) * gz as f32 / 8.0);
            let p = farm.world(l);
            let grass = 1.0 - terrain::biome_desert(p.x, p.y) - terrain::biome_forest(p.x, p.y) - terrain::biome_mountain(p.x, p.y) - terrain::biome_tundra(p.x, p.y);
            let volcano = (p - Vec2::new(6500.0, 0.0)).length() < 1000.0;
            if grass < 0.6 || volcano {
                return false;
            }
            let h = terrain::raw_height(p.x, p.y);
            if h < WATER_LEVEL + 2.5 {
                return false;
            }
            lo = lo.min(h);
            hi = hi.max(h);
            if network.nearest(p.x, p.y).is_some_and(|s| s.dist < s.half_width() + s.shoulder() + 4.0) {
                return false;
            }
        }
    }
    hi - lo < 40.0
        && circuit_points.iter().all(|c| c.distance(farm.yard) > 450.0)
        && landmarks::all().iter().all(|l| l.pos.distance(farm.yard) > 500.0)
        && landmarks::cairns().iter().all(|c| c.distance(farm.yard) > 300.0)
}

/// Fields and hedgerows on the farm's plan.
fn plan(farm: &mut Farm) {
    // A mix of crops, at least one pasture for the animals
    let crops = [Crop::Wheat, Crop::Rapeseed, Crop::Sunflowers, Crop::Shoots, Crop::Ploughed, Crop::Wheat, Crop::Pasture];
    let cells = [(0, 0), (3, 0), (0, 1), (1, 1), (2, 1), (3, 1)];
    let pasture = (hash(farm.seed, 1.0) * cells.len() as f32) as usize;
    for (k, &(c, r)) in cells.iter().enumerate() {
        let lo = Vec2::new(COLUMNS[c], ROWS[r]) + INSET;
        let hi = Vec2::new(COLUMNS[c + 1], ROWS[r + 1]) - INSET;
        let seed = farm.seed + k as f32 * 7.3;
        let crop = if k == pasture { Crop::Pasture } else { crops[(hash(seed, 2.0) * crops.len() as f32) as usize] };
        farm.fields.push(Field { centre: farm.world((lo + hi) * 0.5), half: (hi - lo) * 0.5, crop, seed });
    }

    // Hedge lines: along the road (the yard's front gets a fence instead),
    // then across, each with a gateway gap
    let mut lines: Vec<(Vec2, Vec2)> = vec![
        (Vec2::new(COLUMNS[0], ROWS[0]), Vec2::new(COLUMNS[1], ROWS[0])),
        (Vec2::new(COLUMNS[3], ROWS[0]), Vec2::new(COLUMNS[4], ROWS[0])),
    ];
    for &r in &ROWS[1..] {
        for c in 0..4 {
            lines.push((Vec2::new(COLUMNS[c], r), Vec2::new(COLUMNS[c + 1], r)));
        }
    }
    for (c, &x) in COLUMNS.iter().enumerate() {
        if c != 2 {
            lines.push((Vec2::new(x, ROWS[0]), Vec2::new(x, ROWS[1])));
        }
        lines.push((Vec2::new(x, ROWS[1]), Vec2::new(x, ROWS[2])));
    }
    for (k, (a, b)) in lines.into_iter().enumerate() {
        let roll = hash(farm.seed + k as f32, 5.0);
        if roll < 0.55 {
            let t = 0.25 + 0.5 * hash(farm.seed, k as f32);
            let gap = 12.0 / a.distance(b);
            let m0 = a.lerp(b, (t - gap * 0.5).max(0.0));
            let m1 = a.lerp(b, (t + gap * 0.5).min(1.0));
            farm.hedges.push((farm.world(a), farm.world(m0)));
            farm.hedges.push((farm.world(m1), farm.world(b)));
        } else {
            farm.hedges.push((farm.world(a), farm.world(b)));
        }
    }
}

const HOUSE_WALL: [f32; 4] = [0.93, 0.9, 0.82, 1.0];
const HOUSE_ROOF: [f32; 4] = [0.55, 0.22, 0.14, 1.0];
const BARN_RED: [f32; 4] = [0.62, 0.16, 0.12, 1.0];
const BARN_DOOR: [f32; 4] = [0.45, 0.11, 0.08, 1.0];
const BARN_ROOF: [f32; 4] = [0.3, 0.3, 0.32, 1.0];
const TRIM: [f32; 4] = [0.95, 0.94, 0.9, 1.0];
const WINDOW: [f32; 4] = [1.0, 0.8, 0.45, 1.0];
const DOOR: [f32; 4] = [0.32, 0.2, 0.12, 1.0];
const SILO: [f32; 4] = [0.74, 0.76, 0.78, 1.0];
const SILO_BAND: [f32; 4] = [0.55, 0.57, 0.6, 1.0];
const HAY: [f32; 4] = [0.84, 0.7, 0.34, 1.0];
const FENCE: [f32; 4] = [0.5, 0.38, 0.26, 1.0];
const TYRE: [f32; 4] = [0.12, 0.12, 0.12, 1.0];
const DARK: [f32; 4] = [0.2, 0.18, 0.17, 1.0];

/// Buildings, machinery, fences and livestock anchored in the chunk
/// `inside` accepts.
pub fn build(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    for farm in all() {
        farmstead(b, farm, inside);
        for field in farm.fields.iter().filter(|f| f.crop == Crop::Pasture) {
            livestock(b, farm, field, inside);
        }
    }
}

fn farmstead(b: &mut Builder, farm: &Farm, inside: &impl Fn(Vec2) -> bool) {
    // Buildings run along the road: local X out from it, local Z along it
    let yaw = (-farm.along.x).atan2(-farm.along.y);
    let rot = Quat::from_rotation_y(yaw);
    let x_out = rot * Vec3::X;
    let sign = x_out.dot(vec3(farm.out.x, 0.0, farm.out.y)).signum();
    let at = |along: f32, out: f32| farm.world(Vec2::new(along, out));
    // Point on a building's face: `out` from its centre toward the road, `along`, `up`
    let face = |base: Vec3, out: f32, along: f32, up: f32| base + rot * vec3(-sign * out, up, 0.0) + vec3(farm.along.x, 0.0, farm.along.y) * along;
    let window = |b: &mut Builder, base: Vec3, out: f32, along: f32, up: f32, w: f32, h: f32| {
        let o = out + 0.05;
        b.quad(face(base, o, along - w, up), face(base, o, along + w, up), face(base, o, along + w, up + h), face(base, o, along - w, up + h), WINDOW);
    };

    // Farmhouse, facing the road
    let p = at(-50.0, 108.0);
    if inside(p) {
        let base = footing(b, farm, p, Vec2::new(9.0, 6.0), yaw);
        b.solid_block(base, 6.0, 7.5, 9.0, yaw, HOUSE_WALL, HOUSE_WALL);
        b.roof(base + Vec3::Y * 7.5, 6.0, 4.5, 9.0, yaw, HOUSE_ROOF);
        let chimney = base + rot * vec3(0.0, 0.0, 6.0);
        b.block(chimney + Vec3::Y * 9.0, 0.8, 5.0, 0.8, yaw, [0.6, 0.4, 0.32, 1.0], DARK);
        let front = 6.05;
        b.quad(face(base, front, -1.2, 0.5), face(base, front, 1.2, 0.5), face(base, front, 1.2, 4.0), face(base, front, -1.2, 4.0), DOOR);
        b.glowing = true;
        for along in [-5.5, 5.5] {
            window(b, base, 6.0, along, 2.2, 1.0, 1.6);
            window(b, base, 6.0, along, 5.2, 1.0, 1.4);
        }
        window(b, base, -6.0, -3.0, 2.2, 1.0, 1.6);
        window(b, base, -6.0, 4.0, 5.2, 1.0, 1.4);
        b.glowing = false;
    }

    // Red barn with big trimmed doors toward the road
    let p = at(40.0, 120.0);
    if inside(p) {
        let base = footing(b, farm, p, Vec2::new(13.0, 10.0), yaw);
        b.solid_block(base, 10.0, 9.0, 13.0, yaw, BARN_RED, BARN_RED);
        b.roof(base + Vec3::Y * 9.0, 10.0, 6.5, 13.0, yaw, BARN_ROOF);
        let o = 10.06;
        let (w, h) = (4.0, 7.0);
        b.quad(face(base, o, -w, 0.5), face(base, o, w, 0.5), face(base, o, w, h), face(base, o, -w, h), BARN_DOOR);
        let o = 10.1;
        let bar = |b: &mut Builder, a: (f32, f32), c: (f32, f32)| {
            let (p0, p1) = (face(base, o, a.0, a.1), face(base, o, c.0, c.1));
            let d = (p1 - p0).normalize_or_zero();
            let n = rot * vec3(-sign, 0.0, 0.0);
            let s = d.cross(n) * 0.25;
            b.quad(p0 - s, p1 - s, p1 + s, p0 + s, TRIM);
        };
        for (a, c) in [((-w, 0.5), (-w, h)), ((w, 0.5), (w, h)), ((-w, h), (w, h)), ((0.0, 0.5), (0.0, h)), ((-w, 0.5), (0.0, h)), ((0.0, 0.5), (-w, h)), ((0.0, 0.5), (w, h)), ((w, 0.5), (0.0, h))] {
            bar(b, a, c);
        }
        // Hayloft door up in the gable
        let o = 10.06;
        b.quad(face(base, o, -1.5, 9.5), face(base, o, 1.5, 9.5), face(base, o, 1.5, 12.0), face(base, o, -1.5, 12.0), BARN_DOOR);
    }

    // Silo beside the barn
    let p = at(68.0, 143.0);
    if inside(p) {
        let base = vec3(p.x, ground(p, 4.0) - 0.5, p.y);
        b.column(base, 4.0, 4.0, 18.0, 12, SILO);
        for y in [5.0, 10.0, 15.0] {
            b.column(base + Vec3::Y * y, 4.06, 4.06, 0.4, 12, SILO_BAND);
        }
        b.cone(base + Vec3::Y * 18.0, 4.3, 3.2, 12, SILO_BAND);
        b.post(base, 4.0, 21.0);
    }

    // Round hay bales stacked by the barn
    let p = at(14.0, 140.0);
    if inside(p) {
        let y = ground(p, 6.0);
        let axis = vec3(farm.along.x, 0.0, farm.along.y);
        let out = vec3(farm.out.x, 0.0, farm.out.y);
        let base = vec3(p.x, y, p.y);
        for (k, (o, up)) in [(-3.0, 1.35), (0.0, 1.35), (3.0, 1.35), (-1.5, 3.9), (1.5, 3.9), (0.0, 6.4)].into_iter().enumerate() {
            if k == 5 && hash(farm.seed, 9.0) < 0.5 {
                continue;
            }
            let c = base + out * o + Vec3::Y * up;
            b.limb(c - axis * 0.9, c + axis * 0.9, 1.35, 1.35, 9, k as f32, HAY);
        }
        b.post(base, 4.5, 4.0);
    }

    // A tractor parked in the yard, nose to the road
    let p = at(-8.0 + 16.0 * hash(farm.seed, 4.0), 98.0);
    if inside(p) {
        tractor(b, vec3(p.x, ground(p, 3.0), p.y), farm.along, -farm.out, hash(farm.seed, 6.0) < 0.5);
    }

    // Post-and-rail fence across the yard's front, open for the track
    for (from, to) in [(-88.0, -TRACK - 2.0), (TRACK + 2.0, 88.0)] {
        let steps = ((to - from) / 4.0f32).ceil() as usize;
        for k in 0..=steps {
            let along = from + (to - from) * k as f32 / steps as f32;
            let p = at(along, ROWS[0]);
            if !inside(p) {
                continue;
            }
            let base = vec3(p.x, crate::terrain::terrain_height(p.x, p.y) - 0.3, p.y);
            b.block(base, 0.18, 1.7, 0.18, yaw, FENCE, FENCE);
            if k < steps {
                let q = at(along + (to - from) / steps as f32, ROWS[0]);
                let next = vec3(q.x, crate::terrain::terrain_height(q.x, q.y) - 0.3, q.y);
                for h in [0.75, 1.4] {
                    b.limb(base + Vec3::Y * h, next + Vec3::Y * h, 0.08, 0.08, 4, 0.0, FENCE);
                }
            }
        }
    }

    // A mailbox on a post by the road
    let p = at(TRACK + 4.0, 46.0);
    if inside(p) {
        let base = vec3(p.x, crate::terrain::terrain_height(p.x, p.y) - 0.3, p.y);
        b.block(base, 0.12, 1.4, 0.12, yaw, FENCE, FENCE);
        b.block(base + Vec3::Y * 1.4, 0.35, 0.45, 0.55, yaw, [0.2, 0.3, 0.55, 1.0], [0.2, 0.3, 0.55, 1.0]);
    }
}

/// Floor level for a building with footprint `half` about `p` (along the
/// road, out from it): on sloping ground it stands at the uphill corner on a
/// stone plinth down to the lowest, so no wall is buried.
fn footing(b: &mut Builder, farm: &Farm, p: Vec2, half: Vec2, yaw: f32) -> Vec3 {
    let heights = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (0.0, 0.0)]
        .map(|(a, o)| {
            let q = p + farm.along * a * half.x + farm.out * o * half.y;
            crate::terrain::terrain_height(q.x, q.y)
        });
    let lo = heights.iter().copied().fold(f32::MAX, f32::min);
    let hi = heights.iter().copied().fold(f32::MIN, f32::max);
    if hi - lo > 0.4 {
        let stone = [0.55, 0.52, 0.48, 1.0];
        b.block(vec3(p.x, lo - 0.6, p.y), half.y + 0.4, hi - lo + 0.8, half.x + 0.4, yaw, stone, stone);
    }
    vec3(p.x, hi + 0.2, p.y)
}

/// A little tractor: bonnet and cab, big rear wheels, small front ones.
fn tractor(b: &mut Builder, at: Vec3, side: Vec2, facing: Vec2, red: bool) {
    let paint = if red { [0.75, 0.15, 0.1, 1.0] } else { [0.2, 0.5, 0.2, 1.0] };
    let f = vec3(facing.x, 0.0, facing.y);
    let s = vec3(side.x, 0.0, side.y);
    let yaw = (-facing.x).atan2(-facing.y);
    for (along, r, w) in [(-1.6, 1.5, 0.7), (2.0, 0.85, 0.5)] {
        for k in [-1.0, 1.0] {
            let c = at + f * along + s * k * 1.6 + Vec3::Y * r;
            b.limb(c - s * w, c + s * w, r, r, 8, 0.0, TYRE);
            b.limb(c - s * (w + 0.02), c + s * (w + 0.02), r * 0.45, r * 0.45, 8, 0.0, [0.85, 0.75, 0.2, 1.0]);
        }
    }
    b.block(at + f * 1.4 + Vec3::Y * 1.0, 0.75, 1.3, 1.6, yaw, paint, paint);
    b.block(at - f * 1.0 + Vec3::Y * 1.2, 1.2, 0.6, 1.0, yaw, paint, paint);
    // Cab: four posts and a roof
    for (a, k) in [(-1.8, -1.0), (-1.8, 1.0), (-0.2, -1.0), (-0.2, 1.0)] {
        let p = at + f * a + s * k * 1.05 + Vec3::Y * 1.8;
        b.limb(p, p + Vec3::Y * 1.9, 0.08, 0.08, 4, 0.0, DARK);
    }
    b.block(at - f * 1.0 + Vec3::Y * 3.7, 1.3, 0.2, 1.1, yaw, paint, paint);
    let pipe = at + f * 2.2 + s * 0.4 + Vec3::Y * 2.2;
    b.limb(pipe, pipe + Vec3::Y * 1.4, 0.1, 0.1, 5, 0.0, DARK);
    b.post(at, 2.2, 3.8);
}

/// Cows or sheep grazing a pasture, each farm keeping one kind.
fn livestock(b: &mut Builder, farm: &Farm, field: &Field, inside: &impl Fn(Vec2) -> bool) {
    let sheep = hash(farm.seed, 11.0) < 0.5;
    let count = 6 + (hash(field.seed, 12.0) * 5.0) as usize;
    for k in 0..count {
        let seed = field.seed + k as f32 * 3.7;
        let l = Vec2::new((hash(seed, 1.0) - 0.5) * 2.0 * (field.half.x - 6.0), (hash(seed, 2.0) - 0.5) * 2.0 * (field.half.y - 6.0));
        let p = field.centre + farm.along * l.x + farm.out * l.y;
        if !inside(p) {
            continue;
        }
        let a = hash(seed, 3.0) * std::f32::consts::TAU;
        let dir = Vec2::new(a.cos(), a.sin());
        let at = vec3(p.x, ground(p, 1.5), p.y);
        if sheep { sheep_at(b, at, dir, seed) } else { cow_at(b, at, dir, seed) }
    }
}

fn cow_at(b: &mut Builder, at: Vec3, dir: Vec2, seed: f32) {
    const WHITE: [f32; 4] = [0.92, 0.9, 0.86, 1.0];
    const BLACK: [f32; 4] = [0.1, 0.09, 0.09, 1.0];
    const BROWN: [f32; 4] = [0.55, 0.32, 0.18, 1.0];
    let coat = if hash(seed, 5.0) < 0.3 { BROWN } else { WHITE };
    let f = vec3(dir.x, 0.0, dir.y);
    let s = vec3(-dir.y, 0.0, dir.x);
    let yaw = (-dir.x).atan2(-dir.y);
    for (a, k) in [(1.2, -1.0), (1.2, 1.0), (-1.2, -1.0), (-1.2, 1.0)] {
        b.block(at + f * a + s * k * 0.55, 0.18, 1.3, 0.18, yaw, BLACK, BLACK);
    }
    b.block(at + Vec3::Y * 1.2, 0.85, 1.3, 1.7, yaw, coat, coat);
    if coat == WHITE {
        // Patches
        for k in 0..2 {
            let o = f * (hash(seed, k as f32 + 6.0) - 0.5) * 2.0 + Vec3::Y * 1.5;
            b.block(at + o, 0.88, 0.8, 0.5, yaw, BLACK, BLACK);
        }
    }
    // Head down, grazing
    let head = at + f * 2.0 + Vec3::Y * 0.6;
    b.block(head, 0.45, 0.9, 0.55, yaw, if coat == WHITE { BLACK } else { coat }, BLACK);
    b.block(head + f * 0.5, 0.35, 0.4, 0.2, yaw, [0.85, 0.6, 0.6, 1.0], [0.85, 0.6, 0.6, 1.0]);
    b.post(at, 1.4, 2.5);
}

fn sheep_at(b: &mut Builder, at: Vec3, dir: Vec2, seed: f32) {
    const FLEECE: [f32; 4] = [0.93, 0.92, 0.86, 1.0];
    const FACE: [f32; 4] = [0.15, 0.13, 0.12, 1.0];
    let f = vec3(dir.x, 0.0, dir.y);
    let s = vec3(-dir.y, 0.0, dir.x);
    let yaw = (-dir.x).atan2(-dir.y);
    for (a, k) in [(0.6, -1.0), (0.6, 1.0), (-0.6, -1.0), (-0.6, 1.0)] {
        b.block(at + f * a + s * k * 0.35, 0.1, 0.8, 0.1, yaw, FACE, FACE);
    }
    b.blob(at + Vec3::Y * 1.2, 1.0, 0.75, FLEECE, seed);
    b.block(at + f * 1.0 + Vec3::Y * 0.55, 0.25, 0.6, 0.35, yaw, FACE, FACE);
}

#[cfg(test)]
mod tests {
    #[test]
    fn farms_are_sited_in_the_grasslands() {
        let _ = crate::sim_tests::network();
        let farms = super::all();
        println!("{} farms", farms.len());
        for f in farms {
            let d = f.yard - f.gate;
            println!("  yard {:.0},{:.0} gate {:.0},{:.0} yaw {:.0} crops {:?}", f.yard.x, f.yard.y, f.gate.x, f.gate.y, (-d.x).atan2(-d.y).to_degrees(), f.fields.iter().map(|f| f.crop).collect::<Vec<_>>());
        }
        assert!(farms.len() >= 6, "only {} farms", farms.len());
        // No field, yard or hedge on or beside a path
        let network = crate::sim_tests::network();
        for farm in farms {
            for (a, b) in &farm.hedges {
                for k in 0..=10 {
                    let p = a.lerp(*b, k as f32 / 10.0);
                    assert!(network.nearest(p.x, p.y).is_none_or(|s| s.dist > s.half_width() + 4.0), "hedge at {p} on a path");
                }
            }
            for field in &farm.fields {
                for (a, o) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (0.0, 0.0)] {
                    let p = field.centre + farm.along * a * field.half.x + farm.out * o * field.half.y;
                    assert!(network.nearest(p.x, p.y).is_none_or(|s| s.dist > s.half_width() + 4.0), "field corner at {p} on a path");
                }
            }
        }
    }
}
