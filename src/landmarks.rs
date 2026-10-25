//! Landmarks: named places worth driving to, sited where the terrain suits
//! them (windmills on grassland, lighthouses on shores, masts on summits...).

use std::sync::OnceLock;

use glam::Vec2;

use crate::road::{self, RoadNetwork, ROAD_HALF_WIDTH};
use crate::terrain::{self, TERRAIN_HALF_SIZE, WATER_LEVEL};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Windmill,
    Lighthouse,
    RadioMast,
    StoneArch,
    Cabin,
    Village,
    // Secrets: no trail leads to these, only a line of cairns from the road
    StandingStones,
    SunkenTemple,
    OldRacer,
    FairyRing,
    Watcher,
    GreatOak,
    Shipwreck,
    HotSpring,
}

impl Kind {
    pub fn is_secret(self) -> bool {
        matches!(self, Kind::StandingStones | Kind::SunkenTemple | Kind::OldRacer | Kind::FairyRing
            | Kind::Watcher | Kind::GreatOak | Kind::Shipwreck | Kind::HotSpring)
    }

    /// Whether this place comes alive after dark (glows, fireflies).
    pub fn magical(self) -> bool {
        matches!(self, Kind::StandingStones | Kind::FairyRing | Kind::GreatOak | Kind::SunkenTemple)
    }
}

/// A line or two told on finding a secret.
pub fn lore(kind: Kind) -> &'static str {
    match kind {
        Kind::StandingStones => "NINE STONES, AND ONE THAT FELL.\nAFTER DARK, THEY REMEMBER SOMETHING.",
        Kind::SunkenTemple => "ITS STEPS GO DOWN INTO THE LAKE.\nSOMETHING DOWN THERE IS STILL LIT.",
        Kind::OldRacer => "CAR NUMBER 7 NEVER FINISHED ITS LAST LAP.\nSOMEONE STILL LEAVES FLOWERS.",
        Kind::FairyRing => "STEP INSIDE THE RING,\nAND THE FOREST GOES VERY QUIET.",
        Kind::Watcher => "HALF A FACE, STILL WATCHING THE HORIZON.\nWHAT IS IT WAITING FOR?",
        Kind::GreatOak => "OLDER THAN THE ROADS, OLDER THAN THE RACES.\nSOMEONE STILL LIGHTS THE LANTERNS.",
        Kind::Shipwreck => "SHE RAN AGROUND IN A STORM THAT,\nBY EVERY RECORD, NEVER HAPPENED.",
        Kind::HotSpring => "WARM WATER IN THE FROZEN NORTH.\nTHE SNOW NEVER SETTLES HERE.",
        _ => "",
    }
}

#[derive(Clone, Debug)]
pub struct Landmark {
    pub kind: Kind,
    pub pos: Vec2,
    pub name: String,
    /// Seed for small variations in the model.
    pub seed: u32,
}

/// You discover a landmark by driving within this distance of it.
pub const DISCOVER_RADIUS: f32 = 90.0;

static LANDMARKS: OnceLock<Vec<Landmark>> = OnceLock::new();
static CAIRNS: OnceLock<Vec<Vec2>> = OnceLock::new();

/// Make `landmarks` the set returned by `all`. Later calls are ignored.
pub fn install(landmarks: Vec<Landmark>, network: &RoadNetwork) {
    let _ = CAIRNS.set(cairn_trails(&landmarks, network));
    let _ = LANDMARKS.set(landmarks);
}

/// Little stacks of stones every so often from the nearest road toward each
/// secret: a hint for anyone who notices them, nothing more.
pub fn cairns() -> &'static [Vec2] {
    CAIRNS.get().map_or(&[], Vec::as_slice)
}

fn cairn_trails(landmarks: &[Landmark], network: &RoadNetwork) -> Vec<Vec2> {
    let mut out = Vec::new();
    for l in landmarks.iter().filter(|l| l.kind.is_secret()) {
        // Along the way a walker would go from the road, round any water
        if let Some(path) = crate::trails::path_to(network, l.pos) {
            let mut since = -50.0;
            for w in path.windows(2) {
                since += w[0].distance(w[1]);
                if since >= 120.0 && w[1].distance(l.pos) > 70.0 {
                    out.push(w[1]);
                    since = 0.0;
                }
            }
            continue;
        }
        // The nearest road point with dry land all the way (no wading)
        let dry_line = |a: Vec2| (1..20).all(|k| {
            let q = a + (l.pos - a) * (k as f32 / 20.0);
            terrain::raw_height(q.x, q.y) > WATER_LEVEL + 0.5
        });
        let mut starts: Vec<Vec2> = network.paths.iter()
            .filter(|p| p.kind == road::PathKind::Road)
            .flat_map(|p| p.points.iter().step_by(5).copied())
            .collect();
        starts.sort_by(|a, b| a.distance_squared(l.pos).total_cmp(&b.distance_squared(l.pos)));
        let Some(start) = starts.iter().take(400).copied().find(|&a| dry_line(a)).or(starts.first().copied()) else { continue };
        let span = l.pos - start;
        let length = span.length();
        let dir = span / length.max(1.0);
        let side = Vec2::new(-dir.y, dir.x);
        let mut d = ROAD_HALF_WIDTH + 30.0;
        let mut k = 0;
        while d < length - 70.0 {
            // Not quite a straight line: whoever built them wandered a little
            let wobble = ((k as f32 * 1.7 + l.seed as f32 * 0.001).sin()) * 12.0;
            let p = start + dir * d + side * wobble;
            if terrain::raw_height(p.x, p.y) > WATER_LEVEL + 1.0 {
                out.push(p);
            }
            d += 120.0;
            k += 1;
        }
    }
    out
}

/// Every landmark, or none before `install`.
pub fn all() -> &'static [Landmark] {
    LANDMARKS.get().map_or(&[], Vec::as_slice)
}

struct Rule {
    kind: Kind,
    max: usize,
    /// Minimum distance between landmarks of this kind.
    spacing: f32,
    names: &'static [&'static str],
    suits: fn(&RoadNetwork, Vec2) -> bool,
}

const RULES: [Rule; 14] = [
    Rule {
        kind: Kind::Lighthouse, max: 6, spacing: 1800.0,
        names: &["NORTH LIGHT", "GULL POINT", "HARBOUR LIGHT", "STORM BEACON", "LAKESIDE LIGHT", "LAST LIGHT"],
        suits: |n, p| shore(p) && open_ground(n, p, 1.0),
    },
    Rule {
        kind: Kind::Windmill, max: 8, spacing: 1500.0,
        names: &["OLD MILL", "HILLTOP MILL", "MILLER'S REST", "THREE SAILS", "BREEZY MILL", "GRINDSTONE", "MEADOW MILL", "WINDY RIDGE"],
        suits: |n, p| grassland(p) && open_ground(n, p, 0.25) && near_road(n, p, 700.0),
    },
    Rule {
        kind: Kind::RadioMast, max: 6, spacing: 1800.0,
        names: &["SUMMIT MAST", "SKYLINE TOWER", "RELAY PEAK", "HIGH SIGNAL", "STATIC POINT", "CLOUD MAST"],
        suits: |_, p| terrain::biome_mountain(p.x, p.y) > 0.3 && summit(p),
    },
    Rule {
        kind: Kind::StoneArch, max: 6, spacing: 1200.0,
        names: &["SAND ARCH", "SUNGATE", "WIND ARCH", "THE KEYHOLE", "DUNE BRIDGE", "RED ARCH"],
        suits: |n, p| terrain::biome_desert(p.x, p.y) > 0.4 && open_ground(n, p, 0.3),
    },
    Rule {
        kind: Kind::Cabin, max: 6, spacing: 1200.0,
        names: &["HUNTER'S CABIN", "PINE LODGE", "WOODCUTTER'S HUT", "FOX HOLLOW", "MOSSY CABIN", "TRAPPER'S REST"],
        suits: |n, p| (terrain::biome_forest(p.x, p.y) > 0.4 || terrain::biome_tundra(p.x, p.y) > 0.3) && open_ground(n, p, 0.35),
    },
    Rule {
        kind: Kind::Village, max: 8, spacing: 2200.0,
        names: &["LITTLE STUMBLING", "OAKBY", "NETHER CHASE", "PIT LANE END", "GRIDHAM", "APEX GREEN", "CHICANE CROSS", "SLIPSTREAM"],
        suits: |n, p| grassland(p) && open_ground(n, p, 0.2) && near_road(n, p, 260.0),
    },
    // ── Secrets: off the beaten track ──
    Rule {
        kind: Kind::StandingStones, max: 1, spacing: 0.0,
        names: &["THE WHISPERING STONES"],
        suits: |n, p| grassland(p) && open_ground(n, p, 0.25) && rise(p, 120.0, 1.0) && !near_road(n, p, 250.0),
    },
    Rule {
        kind: Kind::SunkenTemple, max: 1, spacing: 0.0,
        names: &["THE DROWNED TEMPLE"],
        suits: |n, p| shore(p) && open_ground(n, p, 0.4) && terrain::biome_tundra(p.x, p.y) < 0.2 && !near_road(n, p, 200.0),
    },
    Rule {
        kind: Kind::OldRacer, max: 1, spacing: 0.0,
        names: &["THE LAST LAP"],
        suits: |n, p| terrain::biome_forest(p.x, p.y) > 0.5 && open_ground(n, p, 0.3) && !near_road(n, p, 250.0),
    },
    Rule {
        kind: Kind::FairyRing, max: 1, spacing: 0.0,
        names: &["THE FAIRY RING"],
        suits: |n, p| terrain::biome_forest(p.x, p.y) > 0.75 && open_ground(n, p, 0.3) && !near_road(n, p, 350.0),
    },
    Rule {
        kind: Kind::Watcher, max: 1, spacing: 0.0,
        names: &["THE WATCHER IN THE SAND"],
        suits: |n, p| terrain::biome_desert(p.x, p.y) > 0.55 && open_ground(n, p, 0.3)
            && terrain::raw_height(p.x, p.y) > WATER_LEVEL + 4.0 && !near_road(n, p, 250.0),
    },
    Rule {
        kind: Kind::GreatOak, max: 1, spacing: 0.0,
        names: &["THE ELDER OAK"],
        suits: |n, p| grassland(p) && open_ground(n, p, 0.2) && rise(p, 100.0, 0.5) && !near_road(n, p, 300.0),
    },
    Rule {
        kind: Kind::Shipwreck, max: 1, spacing: 0.0,
        names: &["THE GREY GULL"],
        suits: |n, p| beach(p) && open_ground(n, p, 0.5) && terrain::biome_tundra(p.x, p.y) < 0.2 && !near_road(n, p, 200.0),
    },
    Rule {
        kind: Kind::HotSpring, max: 1, spacing: 0.0,
        names: &["THE STEAMING POOLS"],
        suits: |n, p| terrain::biome_tundra(p.x, p.y) > 0.5 && open_ground(n, p, 0.3) && !near_road(n, p, 250.0),
    },
];

fn grassland(p: Vec2) -> bool {
    let (x, z) = (p.x, p.y);
    terrain::biome_desert(x, z) + terrain::biome_mountain(x, z) + terrain::biome_forest(x, z) + terrain::biome_tundra(x, z) < 0.3
}

/// Dry, not too steep, and clear of roads.
fn open_ground(network: &RoadNetwork, p: Vec2, max_slope: f32) -> bool {
    let h = |x: f32, z: f32| terrain::raw_height(x, z);
    let e = 20.0;
    let slope = ((h(p.x + e, p.y) - h(p.x - e, p.y)).powi(2) + (h(p.x, p.y + e) - h(p.x, p.y - e)).powi(2)).sqrt() / (2.0 * e);
    h(p.x, p.y) > WATER_LEVEL + 2.0 && slope < max_slope && !near_road(network, p, ROAD_HALF_WIDTH + 90.0)
}

fn near_road(network: &RoadNetwork, p: Vec2, dist: f32) -> bool {
    network.nearest(p.x, p.y).is_some_and(|s| s.dist < dist) || road_within(network, p, dist)
}

/// `nearest` only looks a short way; check further for "near a road" rules.
fn road_within(network: &RoadNetwork, p: Vec2, dist: f32) -> bool {
    if dist <= road::INFLUENCE {
        return false;
    }
    network.paths.iter().any(|path| path.points.iter().step_by(8).any(|q| q.distance_squared(p) < dist * dist))
}

/// On dry land with water close by.
fn shore(p: Vec2) -> bool {
    let h = terrain::raw_height(p.x, p.y);
    if !(WATER_LEVEL + 1.5..WATER_LEVEL + 15.0).contains(&h) {
        return false;
    }
    (0..8).any(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        terrain::raw_height(p.x + a.cos() * 70.0, p.y + a.sin() * 70.0) < WATER_LEVEL - 1.0
    })
}

/// Higher by `by` than the ground `radius` away all round.
fn rise(p: Vec2, radius: f32, by: f32) -> bool {
    let h = terrain::raw_height(p.x, p.y);
    (0..8).all(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        terrain::raw_height(p.x + a.cos() * radius, p.y + a.sin() * radius) < h - by
    })
}

/// Sand just above the waterline, with water close by.
fn beach(p: Vec2) -> bool {
    let h = terrain::raw_height(p.x, p.y);
    (WATER_LEVEL + 0.5..WATER_LEVEL + 3.0).contains(&h) && (0..8).any(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        terrain::raw_height(p.x + a.cos() * 45.0, p.y + a.sin() * 45.0) < WATER_LEVEL - 0.5
    })
}

/// Higher than the ground around it.
fn summit(p: Vec2) -> bool {
    let h = terrain::raw_height(p.x, p.y);
    h > 70.0 && (0..8).all(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        terrain::raw_height(p.x + a.cos() * 150.0, p.y + a.sin() * 150.0) < h - 2.0
    })
}

fn hash(x: f32, z: f32) -> u32 {
    let h = ((x * 0.137 + z * 0.731).sin() * 43758.547).fract().abs();
    (h * u32::MAX as f32) as u32
}

/// Scan a grid of candidate spots and pick sites for each kind of landmark,
/// judged against the circuits and roads in `network`.
pub fn place(network: &RoadNetwork) -> Vec<Landmark> {
    let step = 140.0;
    let edge = TERRAIN_HALF_SIZE - 400.0;
    let mut candidates: Vec<(u32, Vec2)> = Vec::new();
    let mut z = -edge;
    while z < edge {
        let mut x = -edge;
        while x < edge {
            // Jitter so landmarks don't line up on the grid
            let h = hash(x, z);
            let jx = (h & 0xff) as f32 / 255.0 - 0.5;
            let jz = ((h >> 8) & 0xff) as f32 / 255.0 - 0.5;
            candidates.push((h, Vec2::new(x + jx * step, z + jz * step)));
            x += step;
        }
        z += step;
    }
    candidates.sort_by_key(|c| c.0);

    let mut placed: Vec<Landmark> = Vec::new();
    for rule in &RULES {
        let mut count = 0;
        for &(seed, p) in &candidates {
            if count >= rule.max {
                break;
            }
            let crowded = placed.iter().any(|l| {
                let d = l.pos.distance(p);
                d < 500.0 || (l.kind == rule.kind && d < rule.spacing)
            });
            if crowded || !(rule.suits)(network, p) {
                continue;
            }
            placed.push(Landmark { kind: rule.kind, pos: p, name: rule.names[count].to_string(), seed });
            count += 1;
        }
    }
    placed
}
