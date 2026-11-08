//! Life along the water's edge: wooden piers reaching out from the shore
//! (handrails, lamps, a bait hut, folk fishing off the end), anglers on the
//! banks under their umbrellas, and the moorings where boats tie up beside
//! the piers (which `boats` keeps afloat).
//!
//! Sites are found once, the first time they're asked for, by walking the
//! shoreline: piers go where the water deepens quickly, preferring spots near
//! a town and within reach of a road; anglers anywhere a road passes close
//! to a bank.

use std::collections::HashSet;
use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::props::{hash, Builder};
use crate::road;
use crate::terrain::{self, TERRAIN_HALF_SIZE, WATER_LEVEL};

const WOOD: [f32; 4] = [0.5, 0.36, 0.23, 1.0];
const WOOD_DARK: [f32; 4] = [0.36, 0.25, 0.16, 1.0];
const WOOD_PALE: [f32; 4] = [0.6, 0.47, 0.32, 1.0];
const TARRED: [f32; 4] = [0.16, 0.14, 0.13, 1.0];
const RAIL: [f32; 4] = [0.88, 0.88, 0.84, 1.0];
const RED: [f32; 4] = [0.82, 0.16, 0.12, 1.0];
const WHITE: [f32; 4] = [0.94, 0.93, 0.9, 1.0];
const LAMP: [f32; 4] = [1.0, 0.86, 0.55, 1.0];
const LINE: [f32; 4] = [0.85, 0.85, 0.8, 1.0];

/// The deck stands this far above the water.
const DECK: f32 = 1.7;
/// Half the deck's width, and the T-head's half width and depth.
const DECK_HALF: f32 = 3.4;
const HEAD_HALF: (f32, f32) = (12.0, 7.0);
/// Pier lengths, from where the water starts (it walks out until deep enough).
const PIER_LENGTH: (f32, f32) = (34.0, 96.0);
/// Water at least this deep off the end, for the boats.
const PIER_DEPTH: f32 = 3.5;
/// Fewest units between piers, and between anglers.
const PIER_SPACING: f32 = 2000.0;
const ANGLER_SPACING: f32 = 550.0;
const MAX_PIERS: usize = 48;
const MAX_ANGLERS: usize = 180;
/// Size of the cells used to tell what's near a road or a circuit.
const CELL: f32 = 200.0;

pub struct Pier {
    /// Where it leaves the land, and the way it heads out.
    pub root: Vec2,
    pub dir: Vec2,
    /// From the root to the far side of the T-head.
    pub length: f32,
    pub seed: f32,
}

pub struct Angler {
    pub pos: Vec2,
    /// Toward the water.
    pub facing: Vec2,
    pub seed: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Craft {
    /// A rowing boat, oars shipped.
    Dinghy,
    /// A fishing boat with a wheelhouse.
    Trawler,
    /// A small yacht, sails furled.
    Yacht,
}

#[derive(Clone, Copy)]
pub struct Mooring {
    pub pos: Vec2,
    /// As `boats` measures it: forward is (sin, cos).
    pub heading: f32,
    pub craft: Craft,
    pub seed: f32,
}

#[derive(Default)]
pub struct Harbours {
    pub piers: Vec<Pier>,
    pub anglers: Vec<Angler>,
    pub moorings: Vec<Mooring>,
}

static HARBOURS: OnceLock<Harbours> = OnceLock::new();
static NONE: Harbours = Harbours { piers: Vec::new(), anglers: Vec::new(), moorings: Vec::new() };

/// Every pier, angler and mooring (none until the road network is in).
pub fn get() -> &'static Harbours {
    match road::network() {
        Some(network) => HARBOURS.get_or_init(|| site(network)),
        None => &NONE,
    }
}

fn wet(p: Vec2, depth: f32) -> bool {
    terrain::raw_height(p.x, p.y) < WATER_LEVEL - depth && terrain::solid_height(p.x, p.y) < WATER_LEVEL - depth
}

fn cell(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

/// Cells near any point of `points`, out to `reach` cells.
fn cells_near<'a>(points: impl Iterator<Item = &'a Vec2>, reach: i32) -> HashSet<(i32, i32)> {
    let mut set = HashSet::new();
    for p in points {
        let (cx, cz) = cell(*p);
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                set.insert((cx + dx, cz + dz));
            }
        }
    }
    set
}

fn site(network: &road::RoadNetwork) -> Harbours {
    let roads = cells_near(network.paths.iter().filter(|p| !p.circuit()).flat_map(|p| p.points.iter().step_by(3)), 1);
    let circuits = cells_near(network.paths.iter().filter(|p| p.circuit()).flat_map(|p| p.points.iter().step_by(3)), 2);
    let clear_of_roads = |q: Vec2, margin: f32| network.nearest(q.x, q.y).is_none_or(|s| s.dist > s.half_width() + margin);
    let landmarks = crate::landmarks::all();
    let clear_of_landmarks = |q: Vec2| landmarks.iter().all(|l| l.pos.distance_squared(q) > 220.0 * 220.0);
    let towns = crate::towns::all();
    let town_distance = |q: Vec2| towns.iter().map(|t| t.centre.distance(q)).fold(6000.0, f32::min);

    // Dry ground just above the water, every so often round every shore
    let step = 90.0;
    let edge = TERRAIN_HALF_SIZE - 300.0;
    let mut shore: Vec<(Vec2, Vec2, f32)> = Vec::new();
    let mut z = -edge;
    while z < edge {
        let mut x = -edge;
        while x < edge {
            let h = hash(x, z);
            let p = vec2(x + (h - 0.5) * step, z + (hash(z, x) - 0.5) * step);
            x += step;
            let height = terrain::raw_height(p.x, p.y);
            if !(WATER_LEVEL + 0.8..WATER_LEVEL + 5.0).contains(&height) || circuits.contains(&cell(p)) {
                continue;
            }
            // Out toward the deepest water nearby
            let Some(out) = (0..16)
                .map(|k| Vec2::from_angle(k as f32 * std::f32::consts::TAU / 16.0))
                .min_by(|a, b| {
                    let d = |v: Vec2| terrain::raw_height(p.x + v.x * 45.0, p.y + v.y * 45.0);
                    d(*a).total_cmp(&d(*b))
                })
            else {
                continue;
            };
            if wet(p + out * 30.0, 0.5) {
                shore.push((p, out, h));
            }
        }
        z += step;
    }

    // Piers: nearest the towns first, a little shuffled
    let mut by_town: Vec<&(Vec2, Vec2, f32)> = shore.iter().filter(|(p, ..)| roads.contains(&cell(*p))).collect();
    by_town.sort_by(|a, b| (town_distance(a.0) + a.2 * 900.0).total_cmp(&(town_distance(b.0) + b.2 * 900.0)));
    let mut piers: Vec<Pier> = Vec::new();
    for &&(p, out, h) in &by_town {
        if piers.len() >= MAX_PIERS {
            break;
        }
        if piers.iter().any(|q| q.root.distance(p) < PIER_SPACING) || !clear_of_landmarks(p) || crate::towns::occupied(p, 25.0) {
            continue;
        }
        if let Some(length) = pier_length(p, out, &clear_of_roads) {
            piers.push(Pier { root: p, dir: out, length, seed: h * 1000.0 });
        }
    }

    // Anglers: where a road passes close enough to see them from
    let mut anglers: Vec<Angler> = Vec::new();
    let mut spots: Vec<&(Vec2, Vec2, f32)> = shore.iter().collect();
    spots.sort_by(|a, b| a.2.total_cmp(&b.2));
    for &&(p, out, h) in &spots {
        if anglers.len() >= MAX_ANGLERS {
            break;
        }
        let near_road = network.nearest(p.x, p.y).is_some_and(|s| s.dist > s.half_width() + 14.0 && s.dist < road::INFLUENCE + 60.0)
            || roads.contains(&cell(p)) && clear_of_roads(p, 14.0);
        let tundra = terrain::biome_tundra(p.x, p.y) > 0.5;
        if !near_road || tundra || !wet(p + out * 10.0, 0.6) || crate::towns::occupied(p, 10.0) || !clear_of_landmarks(p) {
            continue;
        }
        if anglers.iter().any(|a| a.pos.distance(p) < ANGLER_SPACING) || piers.iter().any(|q| q.root.distance(p) < 150.0) {
            continue;
        }
        // Stood a little back from the very edge
        let pos = p - out * 3.0;
        anglers.push(Angler { pos, facing: out, seed: h * 1000.0 });
    }

    let moorings = piers.iter().flat_map(moorings_of).collect();
    Harbours { piers, anglers, moorings }
}

/// How long a pier from `root` heading `out` would be, if one fits there:
/// over water all the way, deep enough at the end, crossing no road.
fn pier_length(root: Vec2, out: Vec2, clear_of_roads: &impl Fn(Vec2, f32) -> bool) -> Option<f32> {
    if !clear_of_roads(root, 20.0) {
        return None;
    }
    let side = out.perp();
    let mut s = 4.0;
    // Find the water's edge
    while !wet(root + out * s, 0.0) {
        s += 2.0;
        if s > 18.0 {
            return None;
        }
    }
    let start = s;
    loop {
        let q = root + out * s;
        if !wet(q, 0.3) || !clear_of_roads(q, 10.0) {
            return None;
        }
        if s - start >= PIER_LENGTH.0 && wet(q, PIER_DEPTH) {
            break;
        }
        s += 4.0;
        if s - start > PIER_LENGTH.1 {
            return None;
        }
    }
    // Room for the T-head and boats round it
    let end = s + HEAD_HALF.1 * 2.0;
    let around = [out * (end + 34.0), out * end + side * (HEAD_HALF.0 + 14.0), out * end - side * (HEAD_HALF.0 + 14.0)];
    around.iter().all(|&v| wet(root + v, 2.0) && clear_of_roads(root + v, 10.0)).then_some(end)
}

/// Boats tied up beside a pier: a fishing boat at the end, rowing boats
/// along it, now and then a yacht.
fn moorings_of(pier: &Pier) -> Vec<Mooring> {
    let heading = pier.dir.x.atan2(pier.dir.y);
    let side = pier.dir.perp();
    let at = |s: f32, o: f32| pier.root + pier.dir * s + side * o;
    let r = |k: f32| hash(pier.seed, k);
    let mut list = Vec::new();
    let flip = if r(1.0) < 0.5 { 1.0 } else { -1.0 };
    let head = pier.length - HEAD_HALF.1;
    list.push(Mooring { pos: at(head - 1.0, flip * (HEAD_HALF.0 + 6.0)), heading, craft: Craft::Trawler, seed: r(2.0) });
    let dinghies = 1 + (r(3.0) * 3.0) as usize;
    for k in 0..dinghies {
        let s = pier.length * (0.3 + 0.2 * k as f32);
        let o = if k % 2 == 0 { -flip } else { flip } * (DECK_HALF + 2.6);
        list.push(Mooring { pos: at(s, o), heading: heading + (r(4.0 + k as f32) - 0.5) * 0.3, craft: Craft::Dinghy, seed: r(5.0 + k as f32) });
    }
    if r(9.0) < 0.6 {
        list.push(Mooring { pos: at(head + 1.0, -flip * (HEAD_HALF.0 + 5.0)), heading: heading + std::f32::consts::PI, craft: Craft::Yacht, seed: r(10.0) });
    }
    list.retain(|m| wet(m.pos, if m.craft == Craft::Dinghy { 1.0 } else { 1.2 }));
    list
}

/// Piers and anglers whose footing lies inside this chunk.
pub fn build(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    let harbours = get();
    b.tag("harbour");
    for pier in harbours.piers.iter().filter(|p| inside(p.root)) {
        build_pier(b, pier);
    }
}

/// Folk fishing off a pier: the far rail and the sides of the head, rods
/// out. Each: where they stand on the deck, facing, seed, sat or standing.
fn pier_fishers(pier: &Pier) -> Vec<(Vec3, Vec2, f32, bool)> {
    let (root, dir) = (pier.root, pier.dir);
    let side = dir.perp();
    let head = pier.length - HEAD_HALF.1;
    let at = |s: f32, o: f32| root + dir * s + side * o;
    let deck = WATER_LEVEL + DECK;
    let fishers = 2 + (hash(pier.seed, 6.0) * 3.0) as usize;
    let spots = [(head + HEAD_HALF.1 - 1.4, -2.4, dir), (head + HEAD_HALF.1 - 1.4, 3.0, dir), (head + 1.0, HEAD_HALF.0 - 1.4, side),
        (head - 2.5, -(HEAD_HALF.0 - 1.4), -side), (head * 0.6, DECK_HALF - 1.2, side)];
    spots.iter().take(fishers).enumerate().map(|(k, &(s, o, facing))| {
        let q = at(s, o);
        (vec3(q.x, deck, q.y), facing, pier.seed + k as f32 * 17.0, k % 3 == 1)
    }).collect()
}

/// Anglers are drawn this near the camera.
const ANGLERS_DRAW: f32 = 450.0;

/// The anglers out just now near the camera, on the banks and the piers.
/// They're out from dawn to dusk; in the rain those on the piers pack up,
/// as do bank anglers without an umbrella. Their mesh is rebuilt only when
/// who's out changes.
#[derive(Default)]
pub struct Anglers {
    /// Who was drawn last: (bank anglers, piers).
    out: (Vec<usize>, Vec<usize>),
}

impl Anglers {
    /// A new mesh if who's out has changed since the last.
    pub fn refresh(&mut self, camera: Vec3, clock: f32, rain: f32) -> Option<(Vec<crate::geometry::Vertex>, Vec<u16>)> {
        let cam = vec2(camera.x, camera.z);
        let h = clock * 24.0;
        let daylight = (5.0..21.0).contains(&h);
        let wet = rain > 0.2;
        let harbours = get();
        let bank: Vec<usize> = harbours.anglers.iter().enumerate()
            .filter(|(_, a)| daylight && a.pos.distance(cam) < ANGLERS_DRAW)
            // Only the hardy ones under their big umbrellas stay out in the wet
            .filter(|(_, a)| !wet || (has_umbrella(a.seed) && rain < 0.7))
            .map(|(i, _)| i)
            .collect();
        let piers: Vec<usize> = harbours.piers.iter().enumerate()
            .filter(|(_, p)| daylight && !wet && p.root.distance(cam) < ANGLERS_DRAW)
            .map(|(i, _)| i)
            .collect();
        if (bank.clone(), piers.clone()) == self.out {
            return None;
        }
        let mut b = Builder::default();
        for &i in &bank {
            build_angler(&mut b, &harbours.anglers[i]);
        }
        for &i in &piers {
            for (foot, facing, seed, seated) in pier_fishers(&harbours.piers[i]) {
                angler(&mut b, foot, facing, seed, seated, DECK);
            }
        }
        self.out = (bank, piers);
        let verts = b.verts;
        let idxs = b.idxs.iter().filter(|&&i| i < 65535).map(|&i| i as u16).collect();
        Some((verts, idxs))
    }
}

/// Whether a bank angler has the big umbrella up (as `build_angler` decides).
fn has_umbrella(seed: f32) -> bool {
    hash(seed, 2.0) < 0.7
}

/// The yaw that turns a `Builder::block`'s depth (its z) along `dir`.
fn yaw_of(dir: Vec2) -> f32 {
    dir.x.atan2(dir.y)
}

fn build_pier(b: &mut Builder, pier: &Pier) {
    let (root, dir) = (pier.root, pier.dir);
    let side = dir.perp();
    let yaw = yaw_of(dir);
    let deck = WATER_LEVEL + DECK;
    let at = |s: f32, o: f32| root + dir * s + side * o;
    let p3 = |q: Vec2, y: f32| vec3(q.x, y, q.y);
    let bed = |q: Vec2| terrain::raw_height(q.x, q.y).min(terrain::terrain_height(q.x, q.y));
    let r = |k: f32| hash(pier.seed, k);
    let head = pier.length - HEAD_HALF.1;

    // From the land: a short ramp down from wherever the ground is
    let land = terrain::terrain_height(root.x, root.y);
    let neck = 6.0;
    let ramp = [p3(at(-2.0, -DECK_HALF), land + 0.15), p3(at(-2.0, DECK_HALF), land + 0.15), p3(at(neck, DECK_HALF), deck), p3(at(neck, -DECK_HALF), deck)];
    b.quad(ramp[0], ramp[1], ramp[2], ramp[3], WOOD);

    // Planks in runs of a few, in slightly different woods, on stout piles
    let mut s = neck;
    let mut k = 0;
    while s < head {
        let len = 3.0_f32.min(head - s);
        let c = at(s + len * 0.5, 0.0);
        let tone = [WOOD, WOOD_PALE, WOOD, WOOD_DARK][(k + (r(0.5) * 4.0) as usize) % 4];
        b.block(p3(c, deck - 0.35), DECK_HALF, 0.35, len * 0.5, yaw, WOOD_DARK, tone);
        s += len;
        k += 1;
    }
    let mut s = neck;
    while s <= head + 0.1 {
        for o in [-1.0, 1.0] {
            let q = at(s, o * (DECK_HALF - 0.3));
            let foot = bed(q);
            if foot < deck {
                b.column(p3(q, foot - 0.4), 0.32, 0.3, deck - foot + 0.1, 5, TARRED);
            }
        }
        // A cross brace under the deck
        b.block(p3(at(s, 0.0), deck - 0.9), DECK_HALF, 0.35, 0.18, yaw, WOOD_DARK, WOOD_DARK);
        s += 6.0;
    }

    // The T-head: a broad platform at the end
    let head_c = at(head, 0.0);
    b.block(p3(head_c, deck - 0.35), HEAD_HALF.0, 0.35, HEAD_HALF.1, yaw, WOOD_DARK, WOOD_PALE);
    for (s, o) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0), (1.0, 0.0), (-1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        let q = at(head + s * (HEAD_HALF.1 - 0.4), o * (HEAD_HALF.0 - 0.4));
        let foot = bed(q);
        b.column(p3(q, foot - 0.4), 0.36, 0.34, deck - foot + 0.1, 5, TARRED);
    }

    // Handrails along both sides and round the head, gaps at the steps
    let rail_h = 2.2;
    let post = |b: &mut Builder, q: Vec2| b.column(p3(q, deck), 0.13, 0.13, rail_h, 4, RAIL);
    let rail = |b: &mut Builder, a: Vec2, c: Vec2| b.limb(p3(a, deck + rail_h), p3(c, deck + rail_h), 0.1, 0.1, 4, 0.0, RAIL);
    for o in [-1.0, 1.0] {
        let x = o * (DECK_HALF - 0.12);
        let mut s = neck;
        while s < head - HEAD_HALF.1 - 0.5 {
            let next = (s + 3.0).min(head - HEAD_HALF.1);
            post(b, at(s, x));
            rail(b, at(s, x), at(next, x));
            s = next;
        }
        // Round the head, leaving a gap for steps down to the boats on each side
        let y = o * (HEAD_HALF.0 - 0.12);
        let corners = [at(head - HEAD_HALF.1 + 0.12, o * DECK_HALF), at(head - HEAD_HALF.1 + 0.12, y), at(head + HEAD_HALF.1 - 0.12, y)];
        for w in corners.windows(2) {
            post(b, w[0]);
            rail(b, w[0], w[1]);
        }
        post(b, corners[2]);
        let gap = at(head + HEAD_HALF.1 - 0.12, o * 3.0);
        rail(b, corners[2], gap);
        post(b, gap);
    }
    // The far rail, where the anglers lean
    let far = |o: f32| at(head + HEAD_HALF.1 - 0.12, o);
    rail(b, far(-3.0), far(3.0));

    // Lamps along the pier, and a pair on the head
    let mut lamps: Vec<Vec2> = Vec::new();
    let mut s = neck + 10.0;
    let mut o = 1.0;
    while s < head - HEAD_HALF.1 - 4.0 {
        lamps.push(at(s, o * (DECK_HALF - 0.12)));
        s += 18.0;
        o = -o;
    }
    lamps.push(at(head + HEAD_HALF.1 - 0.2, HEAD_HALF.0 - 0.2));
    lamps.push(at(head + HEAD_HALF.1 - 0.2, -HEAD_HALF.0 + 0.2));
    for q in lamps {
        b.column(p3(q, deck), 0.2, 0.15, 7.2, 5, TARRED);
        b.block(p3(q, deck + 7.1), 0.6, 0.2, 0.6, yaw, TARRED, TARRED);
        b.glowing = true;
        b.block(p3(q, deck + 7.3), 0.45, 0.9, 0.45, yaw, LAMP, LAMP);
        b.glowing = false;
        b.cone(p3(q, deck + 8.2), 0.65, 0.6, 4, TARRED);
    }

    // A lifebuoy on its post, a bench, bollards, crates and lobster pots
    let buoy = at(head - HEAD_HALF.1 + 1.0, (HEAD_HALF.0 - 2.4) * if r(1.5) < 0.5 { 1.0 } else { -1.0 });
    b.column(p3(buoy, deck), 0.16, 0.16, 3.2, 4, WOOD_DARK);
    let ring_c = p3(buoy, deck + 2.2) + vec3(dir.x, 0.0, dir.y) * 0.3;
    for k in 0..8 {
        let a0 = k as f32 * std::f32::consts::TAU / 8.0;
        let a1 = a0 + std::f32::consts::TAU / 8.0;
        let pt = |a: f32| ring_c + vec3(side.x, 0.0, side.y) * a.cos() * 0.9 + Vec3::Y * a.sin() * 0.9;
        b.limb(pt(a0), pt(a1), 0.2, 0.2, 4, 0.0, if k % 2 == 0 { RED } else { WHITE });
    }
    let bench = at(head - 2.5, -(HEAD_HALF.0 - 4.0) * if r(1.5) < 0.5 { 1.0 } else { -1.0 });
    b.block(p3(bench, deck), 2.8, 0.9, 0.7, yaw + std::f32::consts::FRAC_PI_2, WOOD_DARK, WOOD_PALE);
    for o in [-1.0, 1.0] {
        let q = at(head + o * (HEAD_HALF.1 - 1.2), (HEAD_HALF.0 - 1.0) * if r(1.0) < 0.5 { 1.0 } else { -1.0 });
        b.column(p3(q, deck), 0.5, 0.4, 1.1, 6, TARRED);
        b.column(p3(q, deck + 1.1), 0.62, 0.62, 0.22, 6, TARRED);
    }
    let crates = 1 + (r(2.5) * 3.0) as usize;
    for k in 0..crates {
        let q = at(head - 3.0 + k as f32 * 2.2, (HEAD_HALF.0 - 3.0) * if r(1.0) < 0.5 { -1.0 } else { 1.0 } + (r(k as f32) - 0.5));
        let tone = if k % 2 == 0 { [0.25, 0.42, 0.6, 1.0] } else { [0.7, 0.55, 0.3, 1.0] };
        b.block(p3(q, deck), 0.9, 1.0, 0.7, yaw + r(k as f32 + 3.0) * 0.5, tone, tone);
        if k == 0 {
            // A pot on top: a domed net cage
            b.column(p3(q, deck + 1.0), 0.7, 0.5, 0.7, 6, [0.28, 0.3, 0.26, 1.0]);
        }
    }

    // (The folk fishing off it come and go: see `Anglers`)

    // A bait hut by the land end, with nets hung out to dry
    let hut = at(-14.0, if r(7.0) < 0.5 { 12.0 } else { -12.0 });
    let y = crate::props::ground(hut, 4.0);
    let paint = [[0.3, 0.5, 0.62, 1.0], [0.75, 0.3, 0.22, 1.0], [0.35, 0.55, 0.38, 1.0], [0.92, 0.88, 0.75, 1.0]][(r(8.0) * 4.0) as usize % 4];
    b.solid_block(vec3(hut.x, y - 0.4, hut.y), 5.2, 6.0, 4.0, yaw, paint, paint);
    b.roof(vec3(hut.x, y + 5.6, hut.y), 5.2, 2.8, 4.0, yaw, [0.28, 0.24, 0.22, 1.0]);
    // A door facing the pier
    let toward = (root - hut).normalize_or_zero();
    let door = hut + toward * 4.05;
    b.block(vec3(door.x, y - 0.3, door.y), 0.15, 4.2, 1.0, yaw_of(toward) + std::f32::consts::FRAC_PI_2, WOOD_DARK, WOOD_DARK);
    let net_a = hut + side * 6.5 - dir * 3.0;
    let net_b = hut + side * 6.5 + dir * 5.0;
    for q in [net_a, net_b] {
        b.column(vec3(q.x, crate::props::ground(q, 0.5) - 0.3, q.y), 0.14, 0.14, 4.6, 4, WOOD_DARK);
    }
    let ya = crate::props::ground(net_a, 0.5) + 4.0;
    let net = [vec3(net_a.x, ya, net_a.y), vec3(net_b.x, ya, net_b.y), vec3(net_b.x, ya - 2.6, net_b.y), vec3(net_a.x, ya - 2.8, net_a.y)];
    b.quad(net[0], net[1], net[2], net[3], [0.33, 0.4, 0.36, 1.0]);

    // A rowing boat pulled up on the shore, upside down
    let up = at(-5.0, if r(7.0) < 0.5 { -10.0 } else { 10.0 });
    let yb = crate::props::ground(up, 2.0);
    let hull = [[0.85, 0.85, 0.8, 1.0], [0.25, 0.4, 0.6, 1.0], [0.7, 0.25, 0.2, 1.0]][(r(9.5) * 3.0) as usize % 3];
    upturned(b, vec3(up.x, yb - 0.05, up.y), Vec2::from_angle(r(9.7) * 6.28), hull);
}

/// A rowing boat lying keel up on the shore at `at`, bow toward `dir`.
fn upturned(b: &mut Builder, at: Vec3, dir: Vec2, hull: [f32; 4]) {
    let side = dir.perp();
    // Laid out a unit to the metre
    let k = crate::props::PERSON_SCALE;
    let p = |along: f32, across: f32, up: f32| at + (vec3(dir.x, 0.0, dir.y) * along + vec3(side.x, 0.0, side.y) * across + Vec3::Y * up) * k;
    // The gunwale on the ground, round from the transom to the bow
    let rim = [(-1.9, 0.62), (-1.0, 0.78), (0.2, 0.8), (1.2, 0.6), (2.0, 0.0)];
    // The keel along the top, highest amidships
    let keel = [(-1.9, 0.55), (-1.0, 0.62), (0.2, 0.62), (1.2, 0.55), (2.0, 0.1)];
    let dark = [hull[0] * 0.75, hull[1] * 0.75, hull[2] * 0.75, 1.0];
    for s in [-1.0, 1.0] {
        for k in 0..4 {
            let (r0, r1, k0, k1) = (rim[k], rim[k + 1], keel[k], keel[k + 1]);
            b.quad(p(r0.0, s * r0.1, 0.0), p(r1.0, s * r1.1, 0.0), p(k1.0, 0.0, k1.1), p(k0.0, 0.0, k0.1), hull);
        }
    }
    // The transom
    b.tri(p(-1.9, -0.62, 0.0), p(-1.9, 0.62, 0.0), p(-1.9, 0.0, 0.55), dark);
    // A strip of darker keel along the top
    b.limb(p(-1.95, 0.0, 0.57), p(1.25, 0.0, 0.58), 0.1, 0.1, 4, 0.0, dark);
}

fn build_angler(b: &mut Builder, a: &Angler) {
    let y = terrain::terrain_height(a.pos.x, a.pos.y).max(WATER_LEVEL + 0.2);
    let foot = vec3(a.pos.x, y, a.pos.y);
    let r = |k: f32| hash(a.seed, k);
    let seated = r(1.0) < 0.6;
    angler(b, foot, a.facing, a.seed, seated, y - WATER_LEVEL);
    // Their kit, laid out a unit to the metre and scaled up
    let k = crate::props::PERSON_SCALE;
    let facing = a.facing;
    let fwd = vec3(facing.x, 0.0, facing.y);
    let right = vec2(facing.y, -facing.x);
    let yaw = yaw_of(facing);
    let place = |along: f32, across: f32| {
        let q = a.pos + (facing * along + right * across) * k;
        vec3(q.x, terrain::terrain_height(q.x, q.y).max(WATER_LEVEL + 0.1) - 0.05, q.y)
    };
    // A bucket and a tackle box beside them
    b.column(place(0.2, 1.0), 0.26 * k, 0.3 * k, 0.5 * k, 6, [0.85, 0.85, 0.82, 1.0]);
    b.block(place(-0.6, -0.9), 0.4 * k, 0.35 * k, 0.25 * k, yaw + 0.3, [0.25, 0.45, 0.3, 1.0], [0.3, 0.5, 0.35, 1.0]);
    // Most have the big green umbrella up behind them
    if has_umbrella(a.seed) {
        let pole = place(-1.3, 0.4);
        let tilt = fwd * 0.9 * k;
        let top = pole + Vec3::Y * 2.5 * k + tilt;
        b.limb(pole, top, 0.05 * k, 0.05 * k, 4, 0.0, TARRED);
        let canopy = [[0.22, 0.36, 0.22, 1.0], [0.2, 0.25, 0.4, 1.0], [0.42, 0.36, 0.24, 1.0]][(r(3.0) * 3.0) as usize % 3];
        let ribs = 8;
        let lip = |n: usize| {
            let ang = n as f32 * std::f32::consts::TAU / ribs as f32;
            let d = vec3(ang.cos(), 0.0, ang.sin());
            // Tipped toward the water like the pole
            top + (d * 1.7 - Vec3::Y * (0.7 - 0.35 * d.dot(fwd))) * k
        };
        for n in 0..ribs {
            b.tri(top + Vec3::Y * 0.15 * k, lip(n), lip(n + 1), canopy);
        }
    }
    // A second rod propped on a rest
    if r(4.0) < 0.5 {
        let rest = place(0.9, -0.8);
        b.column(rest, 0.03 * k, 0.03 * k, 0.7 * k, 4, TARRED);
        let butt = rest + (Vec3::Y * 0.1 - fwd * 1.2) * k;
        let tip = rest + (Vec3::Y * 1.3 + fwd * 2.8) * k;
        b.limb(butt, tip, 0.03 * k, 0.015 * k, 3, 0.0, TARRED);
        let water = tip + fwd * 2.5 * k;
        line(b, tip, vec3(water.x, WATER_LEVEL + 0.35, water.z));
    }
}

/// A thin fishing line from `a` to `c`.
fn line(b: &mut Builder, a: Vec3, c: Vec3) {
    let across = (c - a).cross(Vec3::Y).normalize_or_zero() * 0.015;
    b.quad(a - across, a + across, c + across, c - across, LINE);
}

/// Someone fishing from `foot`, facing `facing`, standing or sat on a stool,
/// with the rod out and the line down to a float on the water `above` below.
fn angler(b: &mut Builder, foot: Vec3, facing: Vec2, seed: f32, seated: bool, above: f32) {
    let r = |k: f32| hash(seed, k);
    // Modelled a unit to the metre, then scaled up about the feet (all but
    // the line, which has to reach the water)
    let first = b.verts.len();
    const COATS: [[f32; 4]; 6] = [[0.2, 0.32, 0.22, 1.0], [0.85, 0.7, 0.15, 1.0], [0.16, 0.2, 0.34, 1.0], [0.5, 0.38, 0.24, 1.0], [0.62, 0.18, 0.15, 1.0], [0.3, 0.32, 0.3, 1.0]];
    const SKIN: [[f32; 4]; 4] = [[0.92, 0.75, 0.62, 1.0], [0.78, 0.58, 0.42, 1.0], [0.55, 0.38, 0.26, 1.0], [0.38, 0.26, 0.18, 1.0]];
    const HATS: [[f32; 4]; 4] = [[0.3, 0.35, 0.25, 1.0], [0.15, 0.15, 0.18, 1.0], [0.75, 0.6, 0.2, 1.0], [0.45, 0.3, 0.2, 1.0]];
    let coat = COATS[(r(1.5) * 6.0) as usize % 6];
    let skin = SKIN[(r(2.5) * 4.0) as usize % 4];
    let hat = HATS[(r(3.5) * 4.0) as usize % 4];
    let trousers = [0.2, 0.2, 0.22, 1.0];
    let boots = [0.12, 0.14, 0.12, 1.0];
    let yaw = yaw_of(facing);
    let fwd = vec3(facing.x, 0.0, facing.y);
    let right = vec3(facing.y, 0.0, -facing.x);

    let hips;
    if seated {
        // A folding stool, knees out front
        b.block(foot - fwd * 0.3, 0.3, 0.5, 0.3, yaw, TARRED, [0.25, 0.35, 0.25, 1.0]);
        hips = foot - fwd * 0.3 + Vec3::Y * 0.5;
        for s in [-1.0, 1.0] {
            let knee = hips + fwd * 0.5 + right * s * 0.16 + Vec3::Y * 0.05;
            b.limb(hips + right * s * 0.16, knee, 0.12, 0.11, 4, 0.0, trousers);
            b.limb(knee, foot + fwd * 0.35 + right * s * 0.18, 0.1, 0.1, 4, 0.0, trousers);
            b.block(foot + fwd * 0.42 + right * s * 0.18, 0.1, 0.14, 0.17, yaw, boots, boots);
        }
    } else {
        hips = foot + Vec3::Y * 0.85;
        for s in [-1.0, 1.0] {
            b.limb(foot + right * s * 0.15 + Vec3::Y * 0.1, hips + right * s * 0.14, 0.11, 0.12, 4, 0.0, trousers);
            b.block(foot + right * s * 0.15 + fwd * 0.05, 0.1, 0.15, 0.17, yaw, boots, boots);
        }
    }
    // Body, leaning a little toward the water
    let lean = if seated { 0.12 } else { 0.06 };
    let shoulders = hips + Vec3::Y * 0.62 + fwd * lean;
    b.limb(hips, shoulders, 0.24, 0.27, 4, 0.785, coat);
    let neck = shoulders + Vec3::Y * 0.08;
    b.block(neck, 0.15, 0.3, 0.15, yaw, skin, skin);
    // A hat: a woolly beanie or a brimmed one
    let crown = neck + Vec3::Y * 0.3;
    if r(4.5) < 0.5 {
        b.block(crown - Vec3::Y * 0.04, 0.17, 0.16, 0.17, yaw, hat, hat);
    } else {
        b.block(crown - Vec3::Y * 0.02, 0.28, 0.04, 0.28, yaw, hat, hat);
        b.block(crown, 0.16, 0.14, 0.16, yaw, hat, hat);
    }
    // Arms forward to the rod, held up at an angle
    let hands = shoulders + fwd * 0.45 - Vec3::Y * 0.3;
    for s in [-1.0, 1.0] {
        b.limb(shoulders + right * s * 0.27 - Vec3::Y * 0.05, hands + right * s * 0.06, 0.08, 0.07, 4, 0.0, coat);
    }
    let rod_dir = (fwd * 0.75 + Vec3::Y * 0.55 + right * (r(5.5) - 0.5) * 0.4).normalize();
    let butt = hands - rod_dir * 0.6;
    let tip = hands + rod_dir * (3.2 + r(6.5) * 1.2);
    b.limb(butt, tip, 0.035, 0.015, 3, 0.0, TARRED);
    let k = crate::props::PERSON_SCALE;
    for v in &mut b.verts[first..] {
        v.pos = (foot + (Vec3::from(v.pos) - foot) * k).into();
    }
    let tip = foot + (tip - foot) * k;
    // The line down to a float
    let out = tip + fwd * (1.0 + r(7.5) * 2.0) * k;
    let float = vec3(out.x, foot.y - above + 0.3, out.z);
    line(b, tip, float);
    b.block(float - Vec3::Y * 0.2, 0.14, 0.5, 0.14, yaw, RED, [0.95, 0.85, 0.2, 1.0]);
}
