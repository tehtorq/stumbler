//! Rivers and canyons cut into the land. A river rises as a dry gully in
//! the hills and runs down to sea level, where it fills with water between
//! sloping banks, with shallow fords to splash across now and then. Where the
//! ground around is high and hard (mountains, desert rock) its valley sides
//! steepen into a gorge. A canyon is dry: a flat, drivable floor sunk between
//! steep walls, ramping up to the land at either end.
//!
//! Courses are sketched in the world's original layout (like the big
//! landforms in `terrain`) and spread out with it, then meander. Each is cut
//! as a profile across its centreline: the terrain takes whichever is lower,
//! its own height or the cut's, blended so the edge is never a crease.

use std::collections::HashMap;
use std::sync::OnceLock;

use glam::Vec2;

use crate::noise::noise;
use crate::terrain::{self, WATER_LEVEL, WORLD_SCALE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    River,
    Canyon,
}

struct Def {
    name: &'static str,
    kind: Kind,
    /// Rises from a source (a dry gully at first); else wet all the way,
    /// like a river that leaves another.
    source: bool,
    /// Through the land, in the original layout.
    points: &'static [[f32; 2]],
}

/// Laid between the circuits' grid slots, so no circuit is in the way.
const DEFS: [Def; 9] = [
    // Down from the northern mountains through the middle of the map to the sea
    Def { name: "WINDING WATER", kind: Kind::River, source: true,
        points: &[[-1750.0, 7900.0], [-1500.0, 6600.0], [-2000.0, 4600.0], [-1700.0, 2500.0], [-1450.0, 600.0], [-2000.0, -1500.0], [-1700.0, -3500.0], [-1550.0, -6000.0], [-1750.0, -10400.0]] },
    // Out of the alps in a gorge, past the volcano and south
    Def { name: "EASTWATER", kind: Kind::River, source: true,
        points: &[[5350.0, 7900.0], [5100.0, 6700.0], [5400.0, 4800.0], [5050.0, 2600.0], [5300.0, 600.0], [5150.0, -1700.0], [5400.0, -4200.0], [5200.0, -6600.0], [5450.0, -10400.0]] },
    // From the deep forest, down the west and through the desert
    Def { name: "WEST FORK", kind: Kind::River, source: true,
        points: &[[-5300.0, 6200.0], [-5050.0, 4300.0], [-5400.0, 2300.0], [-5200.0, 300.0], [-5450.0, -1800.0], [-5150.0, -4000.0], [-5350.0, -6400.0], [-5200.0, -10400.0]] },
    // Across the south, leaving the Winding Water for the east coast
    Def { name: "LONG REACH", kind: Kind::River, source: false,
        points: &[[-1700.0, -5250.0], [300.0, -5050.0], [1900.0, -5400.0], [3600.0, -5200.0], [5250.0, -5300.0], [7400.0, -5150.0], [10400.0, -5300.0]] },
    // A short run off the tundra into the west fork
    Def { name: "COLDBROOK", kind: Kind::River, source: true,
        points: &[[-7900.0, 7700.0], [-7200.0, 7450.0], [-6500.0, 7750.0], [-5900.0, 7300.0], [-5300.0, 6600.0]] },
    // Dry canyons: through the desert between its circuits
    Def { name: "RED GORGE", kind: Kind::Canyon, source: false,
        points: &[[-9100.0, -5000.0], [-8200.0, -5400.0], [-7200.0, -5100.0], [-6200.0, -5450.0]] },
    Def { name: "BONE CANYON", kind: Kind::Canyon, source: false,
        points: &[[-4500.0, -5400.0], [-3800.0, -5000.0], [-3000.0, -5450.0], [-2400.0, -5150.0]] },
    // Through the southern hills
    Def { name: "DEVIL'S CUT", kind: Kind::Canyon, source: false,
        points: &[[2300.0, -4300.0], [2900.0, -4500.0], [3300.0, -3950.0], [4000.0, -4250.0], [4500.0, -3800.0]] },
    // Along the frozen north, between the circuits and the coast
    Def { name: "ICEFALL CANYON", kind: Kind::Canyon, source: false,
        points: &[[-6600.0, 7950.0], [-5600.0, 7750.0], [-4500.0, 8000.0], [-3500.0, 7700.0], [-2600.0, 8050.0]] },
];

/// Spacing of the samples along a course.
const STEP: f32 = 40.0;
/// How far a course swings either way as it meanders, and over what distance.
const MEANDER: f32 = 1800.0;
const MEANDER_SCALE: f32 = 9000.0;
/// Water depth mid-river, and at a ford.
const RIVER_DEPTH: f32 = 5.0;
const FORD_DEPTH: f32 = 0.9;
/// A ford every so often along the wet stretch, this long.
const FORD_EVERY: f32 = 3000.0;
const FORD_LENGTH: f32 = 90.0;
/// The banks either side of a ford are no steeper than this, for this many
/// samples up and down the river.
const FORD_BANK: f32 = 0.2;
const FORD_NOTCH: usize = 10;
/// River half-widths at the source and the mouth.
const RIVER_WIDTH: (f32, f32) = (14.0, 46.0);
/// Share of a river's length over which its bed falls from the source to the sea.
const DESCENT: f32 = 0.22;
/// Canyon floor half-width, and how high its walls stand: the floor sinks a
/// little and the land either side rises into a tableland.
const CANYON_FLOOR: f32 = 45.0;
const CANYON_DEPTH: f32 = 70.0;
const CANYON_SINK: f32 = 8.0;
/// The tableland round a canyon is at full height out to the first distance,
/// easing down to the land by the second (gently enough to drive up).
const TABLE: (f32, f32) = (450.0, 1250.0);
/// Share of a canyon's length over which it ramps down at each end.
const CANYON_RAMP: f32 = 0.18;
/// Steepest the floor of either may run, so it can be driven.
const FLOOR_GRADE: f32 = 0.12;
/// No cut reaches further than this from the centreline.
const REACH: f32 = 1300.0;
/// Grid cell for finding the courses near a point.
const CELL: f32 = 256.0;

/// One course, sampled every `STEP` along its centreline.
pub struct Course {
    pub name: &'static str,
    pub kind: Kind,
    pub points: Vec<Vec2>,
    /// Floor (bed) height along it.
    floor: Vec<f32>,
    half_width: Vec<f32>,
    /// Water depth mid-channel (0 on dry stretches).
    depth: Vec<f32>,
    /// Gradient of the sides above the floor.
    wall: Vec<f32>,
    /// How much a canyon raises the land either side.
    rise: Vec<f32>,
    /// The samples on the island (see `on_land`).
    land: std::ops::Range<usize>,
}

impl Course {
    /// The stretch of it on the island, leaving out where a river runs on
    /// out into the sea.
    pub fn on_land(&self) -> &[Vec2] {
        &self.points[self.land.clone()]
    }
}

struct Waterways {
    courses: Vec<Course>,
    /// Segments (course, index) within `REACH` of each cell.
    grid: HashMap<(i32, i32), Vec<(u16, u32)>>,
}

static WATERWAYS: OnceLock<Waterways> = OnceLock::new();

fn get() -> &'static Waterways {
    WATERWAYS.get_or_init(build)
}

/// Every course, for maps.
pub fn all() -> &'static [Course] {
    &get().courses
}

/// The land at (x, z) once the courses are cut into `natural`.
pub fn carve(x: f32, z: f32, natural: f32) -> f32 {
    let ways = get();
    let p = Vec2::new(x, z);
    let Some(segments) = ways.grid.get(&cell_of(p)) else { return natural };
    // Each course nearby, at its nearest point: (course, segment, t, distance)
    let mut nearest: Vec<(usize, usize, f32, f32)> = Vec::new();
    for &(c, i) in segments {
        let course = &ways.courses[c as usize];
        let (a, b) = (course.points[i as usize], course.points[i as usize + 1]);
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        let d = p.distance(a + ab * t);
        if d > REACH {
            continue;
        }
        match nearest.iter_mut().find(|n| n.0 == c as usize) {
            Some(n) if n.3 <= d => {}
            Some(n) => *n = (c as usize, i as usize, t, d),
            None => nearest.push((c as usize, i as usize, t, d)),
        }
    }
    // Canyons cut through a tableland that they raise around themselves
    let profile = |&(c, i, t, d): &(usize, usize, f32, f32)| {
        let course = &ways.courses[c];
        let lerp = |v: &[f32]| v[i] + (v[i + 1] - v[i]) * t;
        let (floor, w, depth, wall, rise) =
            (lerp(&course.floor), lerp(&course.half_width), lerp(&course.depth), lerp(&course.wall), lerp(&course.rise));
        let cut = if d < w {
            // A channel, shallowing to the banks
            let across = d / w;
            floor - depth * (1.0 - across * across)
        } else {
            floor + (d - w) * wall
        };
        (cut, rise * smoothstep(TABLE.1, TABLE.0, d), d)
    };
    let cuts: Vec<(f32, f32, f32)> = nearest.iter().map(profile).collect();
    let base = natural + cuts.iter().map(|c| c.1).sum::<f32>();
    let mut h = base;
    for (cut, _, d) in cuts {
        // Fade out toward the reach, so a cut never ends in a step
        let fade = smoothstep(REACH, REACH * 0.75, d);
        let cut = cut + (1.0 - fade) * (base - cut).max(0.0);
        h = smooth_min(h, cut, 6.0);
    }
    h
}

/// Every ford on the island: its middle, and which way is across the river.
pub fn fords() -> &'static [(Vec2, Vec2)] {
    static FORDS: OnceLock<Vec<(Vec2, Vec2)>> = OnceLock::new();
    FORDS.get_or_init(|| {
        all().iter()
            .flat_map(|c| {
                // The shallowest sample of each run of shallow water
                let shallow = |i: usize| c.depth[i] > 0.0 && c.depth[i] < RIVER_DEPTH * 0.5;
                let mut fords = Vec::new();
                let mut i = c.land.start;
                while i < c.land.end {
                    if !shallow(i) {
                        i += 1;
                        continue;
                    }
                    let start = i;
                    while i < c.land.end && shallow(i) {
                        i += 1;
                    }
                    let mid = (start..i).min_by(|&a, &b| c.depth[a].total_cmp(&c.depth[b])).unwrap();
                    let along = (c.points[(mid + 1).min(c.points.len() - 1)] - c.points[mid.saturating_sub(1)]).normalize_or_zero();
                    fords.push((c.points[mid], along.perp()));
                }
                fords
            })
            .collect()
    })
}

/// The course (if any) at (x, z): what to call the place, for banners.
pub fn at(x: f32, z: f32) -> Option<(&'static str, Kind)> {
    let ways = get();
    let p = Vec2::new(x, z);
    ways.grid.get(&cell_of(p))?.iter().find_map(|&(c, i)| {
        let course = &ways.courses[c as usize];
        let (a, b) = (course.points[i as usize], course.points[i as usize + 1]);
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        (p.distance(a + ab * t) < course.half_width[i as usize] + 30.0).then_some((course.name, course.kind))
    })
}

fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

fn smoothstep(edge0: f32, edge1: f32, v: f32) -> f32 {
    let t = ((v - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Polynomial smooth minimum: like `min`, rounded over a band `k` wide.
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

fn build() -> Waterways {
    let courses: Vec<Course> = DEFS.iter().enumerate().map(|(n, def)| lay(n, def)).collect();
    let mut grid: HashMap<(i32, i32), Vec<(u16, u32)>> = HashMap::new();
    let r = (REACH / CELL).ceil() as i32 + 1;
    for (c, course) in courses.iter().enumerate() {
        for i in 0..course.points.len() - 1 {
            let (a, b) = (course.points[i], course.points[i + 1]);
            let (lo, hi) = (cell_of(a.min(b)), cell_of(a.max(b)));
            for cz in lo.1 - r..=hi.1 + r {
                for cx in lo.0 - r..=hi.0 + r {
                    // Only cells the segment's reach can touch
                    let centre = (Vec2::new(cx as f32, cz as f32) + 0.5) * CELL;
                    let ab = b - a;
                    let t = ((centre - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                    if centre.distance(a + ab * t) < REACH + CELL * 0.75 {
                        grid.entry((cx, cz)).or_default().push((c as u16, i as u32));
                    }
                }
            }
        }
    }
    Waterways { courses, grid }
}

/// A course's centreline, meandering through its sketch, and its profile.
fn lay(n: usize, def: &Def) -> Course {
    let sketch: Vec<Vec2> = def.points.iter().map(|p| Vec2::from(*p) * WORLD_SCALE).collect();
    // Straight between the sketch's points, sampled finely
    let mut line = Vec::new();
    for w in sketch.windows(2) {
        let steps = (w[0].distance(w[1]) / STEP).ceil().max(1.0) as usize;
        line.extend((0..steps).map(|k| w[0].lerp(w[1], k as f32 / steps as f32)));
    }
    line.push(*sketch.last().unwrap());
    // Swing it sideways: broad meanders and a smaller wander. Ends stay put
    // so a course meets what it leaves or joins.
    let seed = n as f32 * 1931.0;
    let len = line.len();
    let along: Vec<f32> = (0..len).map(|i| i as f32 * STEP).collect();
    let total = along[len - 1];
    let amplitude = if def.kind == Kind::Canyon { 0.35 } else { 1.0 };
    let mut points: Vec<Vec2> = (0..len)
        .map(|i| {
            let prev = line[i.saturating_sub(1)];
            let next = line[(i + 1).min(len - 1)];
            let dir = (next - prev).normalize_or_zero();
            let side = Vec2::new(-dir.y, dir.x);
            let s = along[i];
            let ends = smoothstep(0.0, 2500.0, s) * smoothstep(total, total - 2500.0, s);
            let swing = (noise(s, seed, MEANDER_SCALE) - 0.5) * 2.0 * MEANDER + (noise(s, seed + 77.0, 1400.0) - 0.5) * 2.0 * 220.0;
            line[i] + side * swing * amplitude * ends
        })
        .collect();
    // Round off the corners left by the sketch
    for _ in 0..30 {
        let prev = points.clone();
        for i in 1..len - 1 {
            points[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }

    // The land along it, evened out over a few hundred units
    let land: Vec<f32> = points.iter().map(|p| terrain::uncarved_height(p.x, p.y)).collect();
    let window = 8;
    let land: Vec<f32> = (0..len)
        .map(|i| {
            let (lo, hi) = (i.saturating_sub(window), (i + window).min(len - 1));
            land[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
        })
        .collect();
    // Hard ground makes steep sides: rock in the mountains and desert
    let wall: Vec<f32> = points.iter().map(|p| {
        let rock = terrain::biome_mountain(p.x, p.y).max(terrain::alpine(p.x, p.y)).max(terrain::biome_desert(p.x, p.y) * 0.8);
        match def.kind {
            Kind::River => 0.28 + 1.2 * rock,
            Kind::Canyon => 2.4,
        }
    }).collect();

    let sea = WATER_LEVEL - 0.5;
    let mut floor: Vec<f32> = match def.kind {
        Kind::River => (0..len)
            .map(|i| {
                if !def.source {
                    return sea;
                }
                // Down from the source to the sea, never above the land
                let t = smoothstep(0.0, DESCENT, along[i] / total);
                (land[0] - 4.0 + (sea - land[0] + 4.0) * t).min(land[i] - 4.0).max(sea)
            })
            .collect(),
        Kind::Canyon => (0..len)
            .map(|i| (land[i] - CANYON_SINK * full_depth(along[i] / total)).max(WATER_LEVEL + 3.0))
            .collect(),
    };
    let rise: Vec<f32> = (0..len)
        .map(|i| if def.kind == Kind::Canyon { CANYON_DEPTH * full_depth(along[i] / total) } else { 0.0 })
        .collect();
    // Rivers only ever run downhill
    if def.kind == Kind::River {
        for i in 1..len {
            floor[i] = floor[i].min(floor[i - 1]);
        }
    }
    limit_grade(&mut floor, FLOOR_GRADE * STEP);

    let half_width: Vec<f32> = (0..len)
        .map(|i| match def.kind {
            Kind::River => {
                let t = if def.source { (along[i] / total / DESCENT).min(1.0) } else { 1.0 };
                RIVER_WIDTH.0 + (RIVER_WIDTH.1 - RIVER_WIDTH.0) * t
            }
            Kind::Canyon => CANYON_FLOOR,
        })
        .collect();
    let raw_depth: Vec<f32> = (0..len)
        .map(|i| {
            if def.kind == Kind::Canyon || floor[i] > sea + 0.01 {
                return 0.0;
            }
            // Shallow fords now and then: a crossing for anyone without a
            // bridge, where there's dry land either side to cross between
            let s = along[i] + seed;
            let dir = (points[(i + 1).min(len - 1)] - points[i.saturating_sub(1)]).normalize_or_zero();
            let banks = [-1.0, 1.0].map(|k| points[i] + dir.perp() * k * (half_width[i] + 90.0));
            let dry = banks.iter().all(|b| terrain::uncarved_height(b.x, b.y) > WATER_LEVEL + 2.0);
            let ford = dry && (s % FORD_EVERY) < FORD_LENGTH;
            if ford { FORD_DEPTH } else { RIVER_DEPTH }
        })
        .collect();
    // Ease the depth in and out of each ford
    let mut eased = raw_depth.clone();
    for _ in 0..3 {
        let prev = eased.clone();
        for i in 1..len - 1 {
            eased[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }
    let depth = eased.iter().zip(&raw_depth).map(|(&e, &d)| if d == 0.0 { 0.0 } else { e.min(d.max(FORD_DEPTH)) }).collect();
    // Gentle banks down to each ford, notched through a gorge if need be,
    // so a ford can be reached as well as crossed
    let wall: Vec<f32> = (0..len)
        .map(|i| {
            let near = (i.saturating_sub(FORD_NOTCH)..(i + FORD_NOTCH + 1).min(len))
                .filter(|&k| raw_depth[k] == FORD_DEPTH)
                .map(|k| 1.0 - k.abs_diff(i) as f32 / (FORD_NOTCH + 1) as f32)
                .fold(0.0, f32::max);
            let wet = terrain::wet_grip(points[i].x, points[i].y, crate::weather::WET_GRIP);
            let bank = FORD_BANK.min(0.9 * crate::physics::steepest_climb(wet, 0.0));
            wall[i] + (wall[i].min(bank) - wall[i]) * smoothstep(0.0, 0.6, near)
        })
        .collect();

    let ashore = |p: &Vec2| terrain::uncarved_height(p.x, p.y) > WATER_LEVEL - 4.0;
    let first = points.iter().position(ashore).unwrap_or(0);
    let last = points.iter().rposition(ashore).map_or(len, |i| i + 1).max(first);
    Course { name: def.name, kind: def.kind, points, floor, half_width, depth, wall, rise, land: first..last }
}

/// How far into its full depth a canyon is, `f` of the way along it: it
/// ramps in from the land at each end.
fn full_depth(f: f32) -> f32 {
    smoothstep(0.0, CANYON_RAMP, f) * smoothstep(1.0, 1.0 - CANYON_RAMP, f)
}

/// Keep `heights` within `max_rise` of their neighbours.
fn limit_grade(heights: &mut [f32], max_rise: f32) {
    let n = heights.len();
    for _ in 0..4 {
        for i in 1..n {
            heights[i] = heights[i].clamp(heights[i - 1] - max_rise, heights[i - 1] + max_rise);
        }
        for i in (0..n - 1).rev() {
            heights[i] = heights[i].clamp(heights[i + 1] - max_rise, heights[i + 1] + max_rise);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dev aid: print spots along each course to look at it from (STUMBLER_AT).
    #[test]
    #[ignore]
    fn course_spots() {
        for c in all() {
            let n = c.points.len();
            println!("{} ({:?}), {:.0} units", c.name, c.kind, n as f32 * STEP);
            for k in 1..6 {
                let i = n * k / 6;
                let p = c.points[i];
                let dir = (c.points[i + 1] - p).normalize();
                println!("  {:.0},{:.0} heading {:.0} floor {:.1} depth {:.1} land {:.1}", p.x, p.y, (-dir.x).atan2(-dir.y).to_degrees(), c.floor[i], c.depth[i], terrain::uncarved_height(p.x, p.y));
            }
            let fords: Vec<String> = (1..n).filter(|&i| c.depth[i] > 0.0 && c.depth[i] <= FORD_DEPTH + 0.05 && c.depth[i - 1] > FORD_DEPTH + 0.05)
                .map(|i| format!("{:.0},{:.0}", c.points[i].x, c.points[i].y)).collect();
            if !fords.is_empty() {
                println!("  fords at {}", fords.join("  "));
            }
        }
    }

    #[test]
    fn rivers_reach_the_sea_and_canyons_sink() {
        for c in all() {
            match c.kind {
                Kind::River => assert!(c.depth.iter().filter(|&&d| d > 0.0).count() > c.points.len() / 2, "{} is mostly dry", c.name),
                Kind::Canyon => {
                    let mid = c.points.len() / 2;
                    let p = c.points[mid];
                    let side = p + (c.points[mid + 1] - p).normalize().perp() * TABLE.0;
                    assert!(terrain::raw_height(side.x, side.y) - terrain::raw_height(p.x, p.y) > 40.0, "{} barely sinks", c.name);
                }
            }
            for w in c.floor.windows(2) {
                assert!((w[1] - w[0]).abs() <= FLOOR_GRADE * STEP + 0.01, "{} floor too steep to drive", c.name);
            }
        }
    }
}
