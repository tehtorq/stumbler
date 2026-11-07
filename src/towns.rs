//! Market towns the connecting roads run straight through: a main street of
//! shops, pubs and houses behind pavements, street lamps strung with bunting,
//! a zebra crossing, and off one side a square with a fountain, benches and a
//! town hall under a clock tower. A sign at each end gives the town's name.
//! Trees, ground cover, boulders and telegraph poles ask `occupied` to keep
//! out of the way.

use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec3};

use crate::landmarks::{self, Kind};
use crate::props::{hash, Builder};
use crate::road::{PathKind, RoadNetwork, RoadPath, Structure, ROAD_HALF_WIDTH, SAMPLE_SPACING};
use crate::roadside::{lettering, text_units};
use crate::terrain::{self, terrain_height, WATER_LEVEL};

/// Half the length of a town's main street.
pub const HALF_LEN: f32 = 150.0;
/// The pavement starts here, out from the street's centreline...
const KERB: f32 = ROAD_HALF_WIDTH + 0.5;
/// ...and the buildings stand here, behind it.
const FRONT: f32 = ROAD_HALF_WIDTH + 8.0;
/// Deepest a building goes back from its front.
const DEEPEST: f32 = 18.0;
/// The square: half its width along the street, and how far back it goes.
const SQUARE_HALF: f32 = 26.0;
const SQUARE_DEPTH: f32 = 46.0;
/// The town hall's half width and depth, behind the square.
const HALL_HALF: Vec2 = Vec2::new(18.0, 9.0);
/// Centreline kept either side of the middle: the street, then the signs.
const REACH: f32 = HALF_LEN + 40.0;
/// Pavements and the square stand this far above the ground.
const PAVE: f32 = 0.25;
/// Lamps stand this far apart along each side.
const LAMP_EVERY: f32 = 32.0;
const LAMP_HEIGHT: f32 = 8.0;
/// Towns are at least this far apart.
const SPACING: f32 = 3200.0;
const MAX_TOWNS: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// Pastel render and slate roofs.
    Plaster,
    /// Red brick, white trim, tiled roofs.
    Brick,
    /// Black-and-white timber framing.
    Timber,
    /// Flat-roofed adobe, out in the desert.
    Adobe,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Use {
    Shop(&'static str),
    Pub(&'static str),
    House,
}

/// One building along the main street.
struct Lot {
    /// Middle of its front, along the street.
    s: f32,
    /// Half its width along the street.
    half_w: f32,
    depth: f32,
    /// Which side of the street: +1 the left as the samples run, -1 the right.
    side: f32,
    storeys: usize,
    usage: Use,
    seed: f32,
}

pub struct Town {
    pub name: String,
    pub centre: Vec2,
    pub style: Style,
    /// The side of the street the square opens off (as `Lot::side`).
    pub square: f32,
    pub seed: f32,
    /// The street's centreline every `SAMPLE_SPACING`, the middle one at `centre`.
    points: Vec<Vec2>,
    lots: Vec<Lot>,
}

impl Town {
    fn mid(&self) -> f32 {
        (self.points.len() / 2) as f32
    }

    fn tangent(&self, k: usize) -> Vec2 {
        let n = self.points.len();
        (self.points[(k + 1).min(n - 1)] - self.points[k.saturating_sub(1)]).normalize_or_zero()
    }

    /// The centreline `s` along the street from the middle, and the unit
    /// normal to its left there.
    fn frame(&self, s: f32) -> (Vec2, Vec2) {
        let f = (s / SAMPLE_SPACING + self.mid()).clamp(0.0, (self.points.len() - 1) as f32);
        let k = (f.floor() as usize).min(self.points.len() - 2);
        let t = f - k as f32;
        let p = self.points[k].lerp(self.points[k + 1], t);
        let d = self.tangent(k).lerp(self.tangent(k + 1), t).normalize_or_zero();
        (p, Vec2::new(-d.y, d.x))
    }

    /// How sharply the street bends toward its left at `s` (1/radius;
    /// negative bending right).
    fn bend(&self, s: f32) -> f32 {
        let (_, n0) = self.frame(s - 8.0);
        let (_, n1) = self.frame(s + 8.0);
        n0.perp_dot(n1) / 16.0
    }

    /// The point `s` along the street and `o` out to its left.
    pub fn world(&self, s: f32, o: f32) -> Vec2 {
        let (p, n) = self.frame(s);
        p + n * o
    }

    /// `p` as (along the street, out to its left), if it's anywhere near.
    pub fn local(&self, p: Vec2) -> Option<Vec2> {
        if p.distance_squared(self.centre) > (REACH + 120.0) * (REACH + 120.0) {
            return None;
        }
        let k = (0..self.points.len()).min_by(|&a, &b| self.points[a].distance_squared(p).total_cmp(&self.points[b].distance_squared(p)))?;
        let t = self.tangent(k);
        let d = p - self.points[k];
        Some(Vec2::new((k as f32 - self.mid()) * SAMPLE_SPACING + d.dot(t), d.dot(Vec2::new(-t.y, t.x))))
    }

    /// Whether `l` (as `local`) is within `margin` of the town's buildings,
    /// pavements or square.
    fn covers(&self, l: Vec2, margin: f32) -> bool {
        let street = l.x.abs() < HALF_LEN + margin && l.y.abs() < FRONT + DEEPEST + 4.0 + margin;
        let back = l.y * self.square;
        let square = l.x.abs() < SQUARE_HALF.max(HALL_HALF.x) + 4.0 + margin && back > 0.0 && back < FRONT + SQUARE_DEPTH + HALL_HALF.y * 2.0 + 6.0 + margin;
        street || square
    }

    /// Trees standing in the square.
    pub fn trees(&self) -> [Vec2; 4] {
        let o = |d: f32| self.square * (FRONT + d);
        [(-1.0, 20.0), (1.0, 20.0), (-1.0, 40.0), (1.0, 40.0)].map(|(k, d)| self.world(k * (SQUARE_HALF - 4.0), o(d)))
    }
}

static TOWNS: OnceLock<Vec<Town>> = OnceLock::new();

pub fn all() -> &'static [Town] {
    TOWNS.get().map_or(&[], Vec::as_slice)
}

/// Site towns along the roads, before the farms (which keep clear of them).
/// Landmarks must already be installed. Later calls are ignored.
pub fn install(network: &RoadNetwork) {
    TOWNS.get_or_init(|| site(network));
}

/// Whether `p` is within `margin` of a town's buildings, pavements or square.
pub fn occupied(p: Vec2, margin: f32) -> bool {
    all().iter().any(|t| t.local(p).is_some_and(|l| t.covers(l, margin)))
}

/// The town whose main street `p` is on or beside.
pub fn at(p: Vec2) -> Option<&'static Town> {
    all().iter().find(|t| t.local(p).is_some_and(|l| l.x.abs() < HALF_LEN && l.y.abs() < FRONT + DEEPEST))
}

/// How far `p` is from the nearer row of shop fronts, if it's on a main street.
pub fn street_walls(p: Vec2) -> Option<f32> {
    all().iter().find_map(|t| {
        let l = t.local(p)?;
        (l.x.abs() < HALF_LEN - 10.0 && l.y.abs() < KERB).then(|| FRONT - l.y.abs())
    })
}

/// Dev aid: a spot at the end of the named town's main street, facing up it.
pub fn street_start(name: &str) -> Option<(Vec2, f32)> {
    let town = all().iter().find(|t| t.name.eq_ignore_ascii_case(name))?;
    let s = -HALF_LEN - 30.0;
    let (_, n) = town.frame(s);
    let dir = Vec2::new(n.y, -n.x);
    Some((town.world(s, -10.0), (-dir.x).atan2(-dir.y)))
}

fn site(network: &RoadNetwork) -> Vec<Town> {
    let circuit_points: Vec<Vec2> = network.paths.iter()
        .filter(|p| p.circuit())
        .flat_map(|p| p.points.iter().step_by(6).copied())
        .collect();
    let reach = (REACH / SAMPLE_SPACING).ceil() as usize;
    // Keep well in from the ends, where roads meet circuits
    let margin = reach + (300.0 / SAMPLE_SPACING) as usize;
    let mut towns: Vec<Town> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (r, road) in network.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
        let n = road.points.len();
        for i in (margin..n.saturating_sub(margin)).step_by((100.0 / SAMPLE_SPACING) as usize) {
            if towns.len() >= MAX_TOWNS {
                return towns;
            }
            let centre = road.points[i];
            if towns.iter().any(|t| t.centre.distance(centre) < SPACING) || !straight_and_level(road, i, reach) {
                continue;
            }
            let seed = hash(i as f32 * 0.37, r as f32 * 2.3) * 100.0;
            let style = if terrain::biome_desert(centre.x, centre.y) > 0.5 {
                Style::Adobe
            } else {
                [Style::Plaster, Style::Brick, Style::Timber][(hash(seed, 1.0) * 3.0) as usize % 3]
            };
            let mut town = Town {
                name: String::new(),
                centre,
                style,
                square: if hash(seed, 2.0) < 0.5 { 1.0 } else { -1.0 },
                seed,
                points: road.points[i - reach..=i + reach].to_vec(),
                lots: Vec::new(),
            };
            if !suitable(&town, network, &circuit_points) {
                continue;
            }
            town.name = name(seed, style, &names);
            names.push(town.name.clone());
            plan(&mut town);
            towns.push(town);
        }
    }
    towns
}

/// A gentle sweep at most, on the ground the whole way.
fn straight_and_level(road: &RoadPath, i: usize, reach: usize) -> bool {
    (i - reach..=i + reach).all(|k| road.curvature[k] < 1.0 / 220.0 && road.structure[k] == Structure::Ground)
}

/// Gently rolling land clear of water, cliffs, other paths, circuits,
/// landmarks and farms, with nothing but this road through it.
fn suitable(town: &Town, network: &RoadNetwork, circuit_points: &[Vec2]) -> bool {
    unsuitable(town, network, circuit_points).is_none()
}

/// Why a town can't go here, if it can't.
fn unsuitable(town: &Town, network: &RoadNetwork, circuit_points: &[Vec2]) -> Option<&'static str> {
    let c = town.centre;
    if (c - Vec2::from(terrain::VOLCANO)).length() < 1300.0 {
        return Some("volcano");
    }
    if circuit_points.iter().any(|q| q.distance(c) < 700.0) {
        return Some("circuit");
    }
    if landmarks::all().iter().any(|l| l.pos.distance(c) < if l.kind == Kind::Village { 1000.0 } else { 450.0 })
        || landmarks::cairns().iter().any(|q| q.distance(c) < 300.0)
    {
        return Some("landmark");
    }
    let road_height = |s: f32| { let p = town.world(s, 0.0); terrain::raw_height(p.x, p.y) };
    let deep = FRONT + DEEPEST + 8.0;
    let mut s = -REACH;
    while s <= REACH {
        let base = road_height(s);
        let mut o = -deep;
        while o <= deep {
            let p = town.world(s, o);
            let raw = terrain::raw_height(p.x, p.y);
            if raw < WATER_LEVEL + 2.5 {
                return Some("water");
            }
            if (raw - base).abs() > 7.0 {
                return Some("slope");
            }
            if terrain::biome_mountain(p.x, p.y) > 0.5 || terrain::biome_tundra(p.x, p.y) > 0.5 {
                return Some("biome");
            }
            if network.nearest_matching(p.x, p.y, |k| k != PathKind::Road).is_some_and(|n| n.dist < n.half_width() + 12.0) {
                return Some("other path");
            }
            if network.nearest_matching(p.x, p.y, |k| k == PathKind::Road).is_some_and(|n| n.dist < o.abs() - 3.0) {
                return Some("other road");
            }
            o += 8.0;
        }
        s += 16.0;
    }
    // And room for the square and the hall behind it
    for s in [-HALL_HALF.x, 0.0, HALL_HALF.x] {
        for d in [SQUARE_DEPTH * 0.5, SQUARE_DEPTH, SQUARE_DEPTH + HALL_HALF.y * 2.0 + 4.0] {
            let p = town.world(s, town.square * (FRONT + d));
            let raw = terrain::raw_height(p.x, p.y);
            if raw < WATER_LEVEL + 2.5
                || (raw - road_height(0.0)).abs() > 7.0
                || network.nearest(p.x, p.y).is_some_and(|n| n.dist < (FRONT + d) - 3.0)
            {
                return Some("square");
            }
        }
    }
    None
}

/// A name for the town that no other has.
fn name(seed: f32, style: Style, taken: &[String]) -> String {
    const HEADS: [&str; 20] = ["ASH", "BRAM", "CALDER", "DUN", "ELM", "FERN", "GALLOW", "HART", "IVY", "KEL",
        "LARK", "MARL", "NETHER", "OAK", "PEN", "ROOK", "STAN", "THORN", "WICK", "WILLOW"];
    const TAILS: [&str; 12] = ["FORD", "BURY", "TON", "WICK", "HAM", "BY", "STEAD", "MERE", "FIELD", "BRIDGE", "LEY", "DALE"];
    const DESERT_HEADS: [&str; 10] = ["MESA", "SOL", "DUST", "RED", "COPPER", "SUN", "SAGE", "DRY", "CINDER", "AMBER"];
    const DESERT_TAILS: [&str; 6] = [" WELLS", " SPRINGS", " FLATS", " ROCK", "VILLE", " CREEK"];
    let (heads, tails): (&[&str], &[&str]) = if style == Style::Adobe { (&DESERT_HEADS, &DESERT_TAILS) } else { (&HEADS, &TAILS) };
    for k in 0..50 {
        let h = heads[(hash(seed, 10.0 + k as f32) * heads.len() as f32) as usize % heads.len()];
        let t = tails[(hash(seed, 60.0 + k as f32) * tails.len() as f32) as usize % tails.len()];
        let name = format!("{h}{t}");
        if !taken.contains(&name) {
            return name;
        }
    }
    format!("TOWN {}", taken.len() + 1)
}

/// Buildings shoulder to shoulder along both sides of the street, an alley
/// here and there, shops and pubs toward the middle and houses at the ends.
fn plan(town: &mut Town) {
    const SHOPS: [&str; 22] = ["BAKERY", "CAFE", "BUTCHER", "BOOKS", "GROCER", "HARDWARE", "FLOWERS", "CHEMIST",
        "BANK", "POST OFFICE", "TAILOR", "HOTEL", "DINER", "TOYS", "BIKES", "SWEETS", "BARBER", "TYRES",
        "MOTOR SPARES", "SPEED SHOP", "ICE CREAM", "RECORDS"];
    const PUBS: [&str; 8] = ["THE CROWN", "THE PLOUGH", "THE SWAN", "THE FOX", "THE BELL", "THE ANCHOR", "THE STAG", "THE OWL"];
    let mut used: Vec<&str> = Vec::new();
    let mut pick = |list: &[&'static str], roll: f32| -> &'static str {
        let start = (roll * list.len() as f32) as usize;
        let name = (0..list.len()).map(|k| list[(start + k) % list.len()]).find(|n| !used.contains(n)).unwrap_or(list[start % list.len()]);
        used.push(name);
        name
    };
    for side in [1.0, -1.0] {
        let mut s = -HALF_LEN;
        let mut k = 0;
        while s < HALF_LEN - 7.0 {
            let seed = town.seed + side * 31.0 + k as f32 * 3.17;
            k += 1;
            let w = (9.0 + 8.0 * hash(seed, 1.0)).min(HALF_LEN - s);
            if side == town.square && s < SQUARE_HALF + 3.0 && s + w > -SQUARE_HALF - 3.0 {
                s = SQUARE_HALF + 3.0;
                continue;
            }
            let depth = 10.0 + 8.0 * hash(seed, 4.0);
            // On the inside of a bend the backs crowd together: space the
            // fronts out so they don't meet
            let squeeze = (town.bend(s + w * 0.5) * side * (FRONT + depth)).clamp(0.0, 0.5);
            let w = w / (1.0 - squeeze);
            let mid = s + w * 0.5;
            let central = 1.0 - mid.abs() / HALF_LEN;
            let roll = hash(seed, 3.0);
            let usage = if roll < 0.07 + 0.08 * central {
                Use::Pub(pick(&PUBS, hash(seed, 6.0)))
            } else if roll < 0.2 + 0.65 * central {
                Use::Shop(pick(&SHOPS, hash(seed, 7.0)))
            } else {
                Use::House
            };
            let storeys = (1 + (hash(seed, 2.0) * (1.2 + 2.2 * central)) as usize).min(3).max(if usage == Use::House { 1 } else { 2 });
            town.lots.push(Lot { s: mid, half_w: w * 0.5 * (1.0 - squeeze) - 0.1, depth, side, storeys, usage, seed });
            s += w + if hash(seed, 5.0) < 0.15 { 4.0 } else { 0.2 };
        }
    }
}

const PAVING: [f32; 4] = [0.6, 0.59, 0.56, 1.0];
const PAVING_DARK: [f32; 4] = [0.54, 0.53, 0.5, 1.0];
const KERB_STONE: [f32; 4] = [0.74, 0.73, 0.7, 1.0];
const SQUARE_STONE: [f32; 4] = [0.66, 0.62, 0.55, 1.0];
const SQUARE_DARK: [f32; 4] = [0.58, 0.54, 0.48, 1.0];
const WINDOW: [f32; 4] = [1.0, 0.82, 0.5, 1.0];
const SHOP_LIGHT: [f32; 4] = [1.0, 0.9, 0.65, 1.0];
const GLASS: [f32; 4] = [0.22, 0.27, 0.33, 1.0];
const FRAME: [f32; 4] = [0.92, 0.91, 0.87, 1.0];
const DOOR: [f32; 4] = [0.3, 0.2, 0.14, 1.0];
const DARK: [f32; 4] = [0.16, 0.15, 0.15, 1.0];
const IRON: [f32; 4] = [0.14, 0.16, 0.17, 1.0];
const STONE: [f32; 4] = [0.72, 0.69, 0.63, 1.0];
const LAMP: [f32; 4] = [1.0, 0.86, 0.55, 1.0];
const WATER: [f32; 4] = [0.3, 0.52, 0.66, 1.0];
const WHITE: [f32; 4] = [0.95, 0.95, 0.92, 1.0];
const SIGN_RED: [f32; 4] = [0.75, 0.12, 0.1, 1.0];
const BEACON: [f32; 4] = [1.0, 0.6, 0.15, 1.0];
const BENCH: [f32; 4] = [0.5, 0.34, 0.2, 1.0];

const PLASTER: [[f32; 4]; 6] = [[0.93, 0.87, 0.72, 1.0], [0.8, 0.88, 0.85, 1.0], [0.92, 0.78, 0.74, 1.0], [0.86, 0.84, 0.93, 1.0], [0.95, 0.93, 0.88, 1.0], [0.88, 0.9, 0.72, 1.0]];
const BRICK: [[f32; 4]; 4] = [[0.62, 0.28, 0.2, 1.0], [0.55, 0.25, 0.19, 1.0], [0.68, 0.38, 0.26, 1.0], [0.5, 0.3, 0.24, 1.0]];
const TIMBER_FILL: [[f32; 4]; 3] = [[0.95, 0.94, 0.88, 1.0], [0.93, 0.9, 0.8, 1.0], [0.96, 0.95, 0.92, 1.0]];
const ADOBE: [[f32; 4]; 4] = [[0.82, 0.6, 0.42, 1.0], [0.88, 0.7, 0.5, 1.0], [0.76, 0.52, 0.38, 1.0], [0.9, 0.78, 0.6, 1.0]];
const SLATE: [[f32; 4]; 3] = [[0.32, 0.34, 0.38, 1.0], [0.38, 0.38, 0.4, 1.0], [0.28, 0.3, 0.33, 1.0]];
const TILE: [[f32; 4]; 3] = [[0.6, 0.26, 0.16, 1.0], [0.5, 0.24, 0.18, 1.0], [0.66, 0.34, 0.2, 1.0]];
/// Shop signs, awnings and doors.
const LIVERY: [[f32; 4]; 7] = [[0.12, 0.3, 0.2, 1.0], [0.5, 0.1, 0.12, 1.0], [0.12, 0.2, 0.42, 1.0], [0.15, 0.15, 0.17, 1.0],
    [0.55, 0.38, 0.1, 1.0], [0.28, 0.16, 0.36, 1.0], [0.1, 0.4, 0.42, 1.0]];
const BUNTING: [[f32; 3]; 5] = [[0.85, 0.15, 0.12], [0.15, 0.35, 0.8], [0.95, 0.8, 0.15], [0.2, 0.65, 0.3], [0.95, 0.95, 0.95]];

fn choose<T: Copy>(list: &[T], roll: f32) -> T {
    list[(roll * list.len() as f32) as usize % list.len()]
}

/// Everything in the towns anchored in the chunk `inside` accepts.
pub fn build(b: &mut Builder, inside: &impl Fn(Vec2) -> bool) {
    for town in all() {
        if !inside_town(town, inside) {
            continue;
        }
        b.tag("towns");
        pavements(b, town, inside);
        for lot in &town.lots {
            building(b, town, lot, inside);
        }
        square(b, town, inside);
        street_furniture(b, town, inside);
        townsfolk(b, town, inside);
        for end in [-1.0, 1.0] {
            entry_sign(b, town, end, inside);
        }
    }
}

/// Whether the chunk could hold any of the town: tried every so often along it.
fn inside_town(town: &Town, inside: &impl Fn(Vec2) -> bool) -> bool {
    let mut s = -REACH;
    while s <= REACH {
        for o in [-80.0, -40.0, 0.0, 40.0, 80.0] {
            if inside(town.world(s, o)) {
                return true;
            }
        }
        s += 20.0;
    }
    false
}

/// Height of the pavement (or square) at `p`.
fn paved(p: Vec2) -> f32 {
    terrain_height(p.x, p.y) + PAVE
}

fn lift(p: Vec2, y: f32) -> Vec3 {
    vec3(p.x, y, p.y)
}

/// Flagstones from the kerb back to the shop fronts, both sides, with a
/// kerbstone edge down to the road.
fn pavements(b: &mut Builder, town: &Town, inside: &impl Fn(Vec2) -> bool) {
    let step = SAMPLE_SPACING;
    let reach = HALF_LEN + 6.0;
    let middle = (KERB + FRONT) * 0.5;
    for side in [1.0, -1.0] {
        let mut s = -reach;
        let mut k = 0;
        while s < reach {
            let s1 = (s + step).min(reach);
            if inside(town.world(s, side * KERB)) {
                let at = |s: f32, o: f32| { let p = town.world(s, side * o); lift(p, paved(p)) };
                for (j, (o0, o1)) in [(KERB + 0.6, middle), (middle, FRONT + 1.0)].into_iter().enumerate() {
                    let tone = if (k + j) % 2 == 0 { PAVING } else { PAVING_DARK };
                    b.quad(at(s, o0), at(s1, o0), at(s1, o1), at(s, o1), tone);
                }
                let down = |s: f32| { let p = town.world(s, side * KERB); lift(p, terrain::settled(p.x, p.y) - 0.1) };
                b.quad(at(s, KERB), at(s1, KERB), down(s1), down(s), KERB_STONE);
                b.quad(at(s, KERB), at(s1, KERB), at(s1, KERB + 0.6), at(s, KERB + 0.6), KERB_STONE);
            }
            s = s1;
            k += 1;
        }
    }
}

/// Floor level for a building of half extents `half` (along, back) whose
/// middle is `centre`: on sloping ground it stands at the highest corner on a
/// stone plinth down to the lowest.
fn footing(b: &mut Builder, centre: Vec2, along: Vec2, back: Vec2, half: Vec2, yaw: f32) -> f32 {
    let heights = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (0.0, 0.0)]
        .map(|(a, o)| { let q = centre + along * a * half.x + back * o * half.y; terrain_height(q.x, q.y) });
    let lo = heights.iter().copied().fold(f32::MAX, f32::min);
    let hi = heights.iter().copied().fold(f32::MIN, f32::max) + PAVE;
    if hi - lo > 0.4 {
        b.block(lift(centre, lo - 0.6), half.y + 0.3, hi - lo + 0.6, half.x + 0.3, yaw, STONE, STONE);
    }
    hi
}

/// Where to draw on a building's front: `a` along it (to the right as you
/// face it), `y` up from the floor, standing `out` proud of the wall.
struct Facade {
    origin: Vec3,
    right: Vec3,
    out: Vec3,
}

impl Facade {
    fn at(&self, a: f32, y: f32, out: f32) -> Vec3 {
        self.origin + self.right * a + Vec3::Y * y + self.out * (0.06 + out)
    }

    fn panel(&self, b: &mut Builder, a0: f32, a1: f32, y0: f32, y1: f32, out: f32, color: [f32; 4]) {
        b.quad(self.at(a0, y0, out), self.at(a1, y0, out), self.at(a1, y1, out), self.at(a0, y1, out), color);
    }

    /// A framed window: lit (glowing after dark) or dark glass.
    fn window(&self, b: &mut Builder, a: f32, y: f32, w: f32, h: f32, lit: bool, frame: [f32; 4]) {
        self.panel(b, a - w - 0.15, a + w + 0.15, y - 0.25, y + h + 0.15, 0.0, frame);
        b.glowing = lit;
        self.panel(b, a - w, a + w, y, y + h, 0.04, if lit { WINDOW } else { GLASS });
        b.glowing = false;
        // A sill to catch the light
        self.panel(b, a - w - 0.3, a + w + 0.3, y - 0.25, y - 0.1, 0.12, frame);
    }

    /// Lettering centred on `(a, y)`, `px` a font pixel.
    fn text(&self, b: &mut Builder, a: f32, y: f32, px: f32, out: f32, text: &str, color: [f32; 4]) {
        let w = text_units(text) * px;
        let origin = self.at(a - w * 0.5, y + crate::hud::GLYPH_H as f32 * px * 0.5, out);
        lettering(b, origin, self.right, -Vec3::Y, px, text, color);
    }
}

/// The frame a building stands in: its yaw (local X back from the street,
/// local Z along it), and unit vectors along and back from the street.
fn lot_frame(town: &Town, s: f32, side: f32) -> (Vec2, Vec2, Vec2, f32) {
    let (p, n) = town.frame(s);
    let back = n * side;
    let along = Vec2::new(n.y, -n.x);
    (p, along, back, (-back.y).atan2(back.x))
}

fn building(b: &mut Builder, town: &Town, lot: &Lot, inside: &impl Fn(Vec2) -> bool) {
    let (p, along, back, yaw) = lot_frame(town, lot.s, lot.side);
    let front = p + back * FRONT;
    let centre = front + back * lot.depth * 0.5;
    if !inside(centre) {
        return;
    }
    let seed = lot.seed;
    let (hw, hd) = (lot.half_w, lot.depth * 0.5);
    let floor = footing(b, centre, along, back, Vec2::new(hw, hd), yaw);
    // Everything from here stands on the footing (planted, or on its plinth)
    b.tag("deck");
    let ground_floor = if lot.usage == Use::House { 4.0 } else { 4.6 };
    let height = ground_floor + (lot.storeys - 1) as f32 * 3.8 + 0.6;
    let (wall, roof, trim) = match town.style {
        Style::Plaster => (choose(&PLASTER, hash(seed, 11.0)), choose(&SLATE, hash(seed, 12.0)), FRAME),
        Style::Brick => (choose(&BRICK, hash(seed, 11.0)), choose(&TILE, hash(seed, 12.0)), FRAME),
        Style::Timber => (choose(&TIMBER_FILL, hash(seed, 11.0)), choose(&TILE, hash(seed, 12.0)), DARK),
        Style::Adobe => (choose(&ADOBE, hash(seed, 11.0)), choose(&ADOBE, hash(seed, 12.0)), [0.45, 0.3, 0.2, 1.0]),
    };
    let base = lift(centre, floor);
    b.solid_block(base, hd, height, hw, yaw, wall, wall);

    // As you face the front from the street, your right is along -along
    // when the building is on the left, so follow the roadside signs' rule
    let out = -back;
    let face = Facade { origin: lift(front, floor), right: vec3(-out.y, 0.0, out.x), out: vec3(out.x, 0.0, out.y) };
    let livery = choose(&LIVERY, hash(seed, 13.0));

    // The roof: pitched with a chimney, or flat behind a parapet in the desert
    if town.style == Style::Adobe {
        b.block(base + Vec3::Y * (height - 0.5), hd + 0.2, 0.5, hw + 0.2, yaw, trim, roof);
        // Beam ends poking out under the roofline
        let beams = (hw / 1.6) as i32;
        for k in -beams..=beams {
            face.panel(b, k as f32 * 1.6 - 0.18, k as f32 * 1.6 + 0.18, height - 1.3, height - 0.95, 0.4, [0.4, 0.27, 0.17, 1.0]);
        }
        if hash(seed, 14.0) < 0.35 {
            let tank = base + Vec3::Y * height + vec3(back.x, 0.0, back.y) * hd * 0.4;
            b.column(tank, 1.3, 1.3, 2.2, 8, [0.6, 0.62, 0.64, 1.0]);
        }
    } else {
        let rise = 2.6 + hash(seed, 14.0) * 2.0;
        b.roof(base + Vec3::Y * height, hd, rise, hw, yaw, roof);
        let chimney = base + vec3(along.x, 0.0, along.y) * (hw - 1.2) * if hash(seed, 15.0) < 0.5 { 1.0 } else { -1.0 };
        b.block(chimney + Vec3::Y * (height + rise * 0.5), 0.7, rise * 0.5 + 1.6, 0.7, yaw, [0.55, 0.36, 0.3, 1.0], DARK);
        // A cornice along the top of the front
        face.panel(b, -hw - 0.1, hw + 0.1, height - 0.45, height - 0.05, 0.15, trim);
    }

    // Timber framing over the upper floors
    if town.style == Style::Timber && lot.storeys > 1 {
        let beam = |b: &mut Builder, a0: f32, a1: f32, y0: f32, y1: f32| face.panel(b, a0, a1, y0, y1, 0.02, DARK);
        for k in 0..lot.storeys {
            let y = ground_floor + k as f32 * 3.8;
            beam(b, -hw, hw, y - 0.2, y + 0.1);
        }
        let posts = ((hw * 2.0) / 2.6).round().max(2.0) as i32;
        for k in 0..=posts {
            let a = -hw + 2.0 * hw * k as f32 / posts as f32;
            beam(b, a - 0.15, a + 0.15, ground_floor, height - 0.4);
        }
    }

    // Upper floors: a row of windows each, some lit
    let across = ((hw * 2.0 - 1.0) / 3.4).floor().max(1.0) as i32;
    for storey in 1..lot.storeys {
        let y = ground_floor + (storey - 1) as f32 * 3.8 + 1.0;
        for k in 0..across {
            let a = -hw + (k as f32 + 0.5) * 2.0 * hw / across as f32;
            let lit = hash(seed + storey as f32, k as f32 + 20.0) < 0.55;
            face.window(b, a, y, 0.55, 1.7, lit, trim);
        }
    }

    match lot.usage {
        Use::Shop(name) => {
            // Lit shop window with the door to one end, a sign above, and
            // maybe a striped awning
            let door = if hash(seed, 16.0) < 0.5 { 1.0 } else { -1.0 };
            let (w0, w1) = if door > 0.0 { (-hw + 0.8, hw - 3.0) } else { (-hw + 3.0, hw - 0.8) };
            face.panel(b, w0 - 0.2, w1 + 0.2, 0.0, 0.8, 0.05, livery);
            b.glowing = true;
            face.panel(b, w0, w1, 0.8, 3.3, 0.04, SHOP_LIGHT);
            b.glowing = false;
            face.panel(b, w0 - 0.2, w1 + 0.2, 3.3, 3.5, 0.06, livery);
            let d = door * (hw - 1.9);
            face.panel(b, d - 0.9, d + 0.9, 0.0, 3.1, 0.04, livery);
            face.panel(b, d - 0.6, d + 0.6, 1.6, 2.8, 0.06, GLASS);
            // Sign board, the name lit after dark
            face.panel(b, -hw + 0.5, hw - 0.5, 3.6, ground_floor - 0.05, 0.12, livery);
            let px = ((hw * 2.0 - 2.0) / text_units(name)).min(0.14);
            b.glowing = true;
            face.text(b, 0.0, (3.6 + ground_floor) * 0.5, px, 0.16, name, WHITE);
            b.glowing = false;
            if hash(seed, 17.0) < 0.6 {
                awning(b, &face, w0 - 0.3, w1 + 0.3, livery);
            }
        }
        Use::Pub(name) => {
            face.panel(b, -hw, hw, 0.0, ground_floor - 0.3, 0.02, livery);
            let d = hash(seed, 16.0) * (hw - 2.0) * 2.0 - (hw - 2.0);
            face.panel(b, d - 0.9, d + 0.9, 0.0, 3.0, 0.05, DOOR);
            for a in [-hw * 0.6, hw * 0.6] {
                if (a - d).abs() > 2.4 {
                    b.glowing = true;
                    face.panel(b, a - 1.3, a + 1.3, 1.0, 2.9, 0.05, WINDOW);
                    b.glowing = false;
                }
            }
            let px = ((hw * 2.0 - 1.5) / text_units(name)).min(0.13);
            face.text(b, 0.0, ground_floor - 0.75, px, 0.06, name, [0.95, 0.85, 0.5, 1.0]);
            hanging_sign(b, &face, if d > 0.0 { -hw + 0.8 } else { hw - 0.8 }, ground_floor + 0.4, livery);
        }
        Use::House => {
            let d = (hash(seed, 16.0) - 0.5) * hw;
            face.panel(b, d - 0.75, d + 0.75, 0.0, 2.8, 0.04, livery);
            face.panel(b, d - 1.0, d + 1.0, -0.05, 0.25, 0.3, STONE);
            for a in [-hw * 0.55, hw * 0.55] {
                if (a - d).abs() > 2.0 {
                    face.window(b, a, 0.9, 0.6, 1.6, hash(seed, a + 30.0) < 0.5, trim);
                }
            }
        }
    }
    b.tag("towns");
}

/// A striped canvas awning over a shop window.
fn awning(b: &mut Builder, face: &Facade, a0: f32, a1: f32, color: [f32; 4]) {
    let stripes = ((a1 - a0) / 1.1).round().max(2.0) as usize;
    let (top, low, reach) = (3.45, 2.55, 2.2);
    for k in 0..stripes {
        let x0 = a0 + (a1 - a0) * k as f32 / stripes as f32;
        let x1 = a0 + (a1 - a0) * (k + 1) as f32 / stripes as f32;
        let c = if k % 2 == 0 { color } else { WHITE };
        b.quad(face.at(x0, top, 0.05), face.at(x1, top, 0.05), face.at(x1, low, reach), face.at(x0, low, reach), c);
        // A scalloped valance along the front edge
        b.quad(face.at(x0, low, reach), face.at(x1, low, reach), face.at(x1, low - 0.35, reach), face.at(x0, low - 0.35, reach), c);
    }
    for a in [a0, a1] {
        b.tri(face.at(a, top, 0.05), face.at(a, low, reach), face.at(a, low, 0.05), color);
    }
}

/// A pub's painted sign hanging from an iron bracket, edge on to the street.
fn hanging_sign(b: &mut Builder, face: &Facade, a: f32, y: f32, color: [f32; 4]) {
    b.limb(face.at(a, y, 0.0), face.at(a, y, 1.9), 0.06, 0.06, 4, 0.0, IRON);
    let (r, o) = (face.right * 0.06, |d: f32| face.out * d);
    let top = face.at(a, y - 0.15, 0.0);
    let corners = [top + o(0.4), top + o(1.7), top + o(1.7) - Vec3::Y * 1.3, top + o(0.4) - Vec3::Y * 1.3];
    for s in [-1.0, 1.0] {
        b.quad(corners[0] + r * s, corners[1] + r * s, corners[2] + r * s, corners[3] + r * s, color);
    }
    // A gilt border
    let gold = [0.85, 0.7, 0.3, 1.0];
    b.quad(corners[0] + o(0.05) + r * 1.3, corners[1] - o(0.05) + r * 1.3, corners[1] - o(0.05) + r * 1.3 - Vec3::Y * 0.12, corners[0] + o(0.05) + r * 1.3 - Vec3::Y * 0.12, gold);
    b.quad(corners[3] + o(0.05) + r * 1.3, corners[2] - o(0.05) + r * 1.3, corners[2] - o(0.05) + r * 1.3 + Vec3::Y * 0.12, corners[3] + o(0.05) + r * 1.3 + Vec3::Y * 0.12, gold);
}

/// The square: paved, with trees, benches round a fountain, lamps at its
/// corners, and the town hall across the back under its clock tower.
fn square(b: &mut Builder, town: &Town, inside: &impl Fn(Vec2) -> bool) {
    let side = town.square;
    let at = |s: f32, d: f32| town.world(s, side * (FRONT + d));
    let cell = 4.0;
    let (cols, rows) = ((SQUARE_HALF * 2.0 / cell) as i32, ((SQUARE_DEPTH - 1.0) / cell) as i32);
    for j in 0..rows {
        for i in 0..cols {
            let (s0, d0) = (-SQUARE_HALF + i as f32 * cell, 1.0 + j as f32 * cell);
            if !inside(at(s0, d0)) {
                continue;
            }
            let c = |s: f32, d: f32| { let p = at(s, d); lift(p, paved(p)) };
            let tone = if (i + j) % 2 == 0 { SQUARE_STONE } else { SQUARE_DARK };
            b.quad(c(s0, d0), c(s0 + cell, d0), c(s0 + cell, d0 + cell), c(s0, d0 + cell), tone);
        }
    }
    // Kerbs down from its open sides
    for (s0, s1, d0, d1) in [(-SQUARE_HALF, -SQUARE_HALF, 1.0, SQUARE_DEPTH), (SQUARE_HALF, SQUARE_HALF, 1.0, SQUARE_DEPTH)] {
        let steps = ((d1 - d0) / cell).floor() as i32;
        for k in 0..steps {
            let (da, db) = (d0 + k as f32 * cell, d0 + (k + 1) as f32 * cell);
            let (pa, pb) = (at(s0, da), at(s1, db));
            if inside(pa) {
                b.quad(lift(pa, paved(pa)), lift(pb, paved(pb)), lift(pb, terrain::settled(pb.x, pb.y) - 0.1), lift(pa, terrain::settled(pa.x, pa.y) - 0.1), KERB_STONE);
            }
        }
    }

    // The fountain in the middle
    let f = at(0.0, SQUARE_DEPTH * 0.5);
    if inside(f) {
        let y = paved(f);
        let base = lift(f, y - 0.1);
        b.column(base, 4.4, 4.4, 1.0, 10, STONE);
        let water = base + Vec3::Y * 0.85;
        for k in 0..10 {
            let a0 = std::f32::consts::TAU * k as f32 / 10.0;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / 10.0;
            b.tri(water, water + vec3(a0.cos(), 0.0, a0.sin()) * 4.0, water + vec3(a1.cos(), 0.0, a1.sin()) * 4.0, WATER);
        }
        b.column(water, 0.6, 0.45, 2.4, 8, STONE);
        b.column(water + Vec3::Y * 2.4, 0.5, 1.8, 0.5, 8, STONE);
        b.column(water + Vec3::Y * 2.9, 0.25, 0.2, 1.0, 6, STONE);
        b.blob(water + Vec3::Y * 4.0, 0.35, 1.0, STONE, town.seed);
        b.post(base, 4.4, 1.0);
    }

    // Benches facing it
    for (s, d) in [(-12.0, 0.3), (12.0, 0.3), (-12.0, 0.7), (12.0, 0.7)] {
        let p = at(s, SQUARE_DEPTH * d);
        if !inside(p) {
            continue;
        }
        let facing = (f - p).normalize_or_zero();
        bench(b, lift(p, paved(p)), facing);
    }

    // The town hall: stone, columns across its front, a clock tower and a flag
    let hall_d = SQUARE_DEPTH + HALL_HALF.y + 1.0;
    let (p, along, back, yaw) = lot_frame(town, 0.0, side);
    let centre = p + back * (FRONT + hall_d);
    if inside(centre) {
        let floor = footing(b, centre, along, back, HALL_HALF, yaw);
        b.tag("deck");
        let base = lift(centre, floor);
        let height = 11.0;
        b.solid_block(base, HALL_HALF.y, height, HALL_HALF.x, yaw, STONE, STONE);
        b.roof(base + Vec3::Y * height, HALL_HALF.y, 3.0, HALL_HALF.x, yaw, choose(&SLATE, hash(town.seed, 21.0)));
        let front = centre - back * HALL_HALF.y;
        let out = -back;
        let face = Facade { origin: lift(front, floor), right: vec3(-out.y, 0.0, out.x), out: vec3(out.x, 0.0, out.y) };
        face.panel(b, -HALL_HALF.x - 0.1, HALL_HALF.x + 0.1, height - 0.7, height - 0.1, 0.2, WHITE);
        for k in 0..6 {
            let a = -HALL_HALF.x + 2.5 + k as f32 * (HALL_HALF.x * 2.0 - 5.0) / 5.0;
            if a.abs() < 4.0 {
                continue;
            }
            face.window(b, a, 1.4, 0.8, 2.4, true, WHITE);
            face.window(b, a, 6.4, 0.8, 2.4, hash(town.seed, a) < 0.6, WHITE);
            // A column standing proud between each pair
            let c = face.at(a + 1.75, 0.0, 1.6);
            if a + 1.75 < HALL_HALF.x - 1.0 && (a + 1.75).abs() > 4.0 {
                b.column(c, 0.45, 0.4, height - 0.7, 8, WHITE);
            }
        }
        // Steps up to the doors, under the tower
        b.block(face.at(0.0, -0.3, 2.0), 2.0, 0.5, 4.5, yaw, STONE, STONE);
        b.glowing = true;
        face.panel(b, -1.4, 1.4, 0.2, 3.8, 0.05, SHOP_LIGHT);
        b.glowing = false;

        let tower = lift(front + back * 3.5, floor);
        let top = 25.0;
        b.solid_block(tower, 3.5, top, 3.5, yaw, STONE, STONE);
        b.block(tower + Vec3::Y * (height - 0.2), 3.8, 0.6, 3.8, yaw, WHITE, WHITE);
        b.block(tower + Vec3::Y * (top - 0.4), 3.9, 0.6, 3.9, yaw, WHITE, WHITE);
        // A pyramid cap
        let r = glam::Quat::from_rotation_y(yaw);
        let apex = tower + Vec3::Y * (top + 6.0);
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, z)| tower + Vec3::Y * (top + 0.2) + r * vec3(x * 4.0, 0.0, z * 4.0));
        for k in 0..4 {
            b.tri(corners[k], corners[(k + 1) % 4], apex, choose(&SLATE, hash(town.seed, 22.0)));
        }
        crate::venue::flagpole(b, apex - Vec3::Y * 0.5, 5.0, along, choose(&BUNTING, hash(town.seed, 23.0)));
        b.tag("towns");
        // A clock face on each side, lit after dark, all showing the same time
        let hour = hash(town.seed, 24.0) * std::f32::consts::TAU;
        let minute = hash(town.seed, 25.0) * std::f32::consts::TAU;
        for k in 0..4 {
            let a = yaw + k as f32 * std::f32::consts::FRAC_PI_2;
            let n = glam::Quat::from_rotation_y(a) * Vec3::X;
            let right = vec3(-n.z, 0.0, n.x);
            let c = tower + Vec3::Y * (top - 4.0) + n * 3.56;
            b.glowing = true;
            for j in 0..12 {
                let t0 = std::f32::consts::TAU * j as f32 / 12.0;
                let t1 = std::f32::consts::TAU * (j + 1) as f32 / 12.0;
                let rim = |t: f32| c + (right * t.cos() + Vec3::Y * t.sin()) * 2.3;
                b.tri(c, rim(t0), rim(t1), [1.0, 0.96, 0.85, 1.0]);
            }
            b.glowing = false;
            for (angle, len) in [(hour, 1.2), (minute, 1.9)] {
                let dir = right * angle.sin() + Vec3::Y * angle.cos();
                let w = n.cross(dir).normalize_or_zero() * 0.1;
                let tip = c + n * 0.05 + dir * len;
                let foot = c + n * 0.05;
                b.quad(foot - w, foot + w, tip + w, tip - w, DARK);
            }
        }
    }
}

fn bench(b: &mut Builder, at: Vec3, facing: Vec2) {
    let f = vec3(facing.x, 0.0, facing.y);
    let s = vec3(-facing.y, 0.0, facing.x);
    let yaw = (-facing.y).atan2(facing.x);
    for k in [-1.0, 1.0] {
        b.block(at + s * k * 1.2, 0.15, 0.5, 0.25, yaw, IRON, IRON);
    }
    // Seat and back rest on the legs
    b.tag("deck");
    b.block(at + Vec3::Y * 0.5, 0.25, 0.12, 1.5, yaw, BENCH, BENCH);
    b.block(at - f * 0.3 + Vec3::Y * 0.6, 0.06, 0.6, 1.5, yaw, BENCH, BENCH);
    b.tag("towns");
}

/// Lamps in pairs along the street (bunting strung between those in the
/// middle), a zebra crossing by the square with its beacons, and lamps at
/// the square's corners.
fn street_furniture(b: &mut Builder, town: &Town, inside: &impl Fn(Vec2) -> bool) {
    let pairs = (HALF_LEN / LAMP_EVERY) as i32;
    for k in -pairs..pairs {
        let s = (k as f32 + 0.5) * LAMP_EVERY;
        if s.abs() > HALF_LEN {
            continue;
        }
        let tops: Vec<Vec3> = [1.0, -1.0].iter().map(|&side| {
            let p = town.world(s, side * (KERB + 1.2));
            let n = town.frame(s).1 * -side;
            let foot = lift(p, paved(p) - 0.1);
            if inside(p) {
                street_lamp(b, foot, n);
            }
            foot + Vec3::Y * (LAMP_HEIGHT - 0.6)
        }).collect();
        let anchor = town.world(s, 0.0);
        if s.abs() < 90.0 && inside(anchor) {
            bunting(b, tops[0], tops[1], k);
        }
    }

    // The crossing, just past the square
    let s = SQUARE_HALF + 9.0;
    let p = town.world(s, 0.0);
    if inside(p) {
        let stripes = ((ROAD_HALF_WIDTH * 2.0 - 2.0) / 3.0) as i32;
        for k in 0..stripes {
            let o0 = -ROAD_HALF_WIDTH + 1.0 + k as f32 * 3.0;
            let c = |s: f32, o: f32| { let q = town.world(s, o); lift(q, terrain_height(q.x, q.y) + 0.06) };
            b.quad(c(s - 3.0, o0), c(s + 3.0, o0), c(s + 3.0, o0 + 1.6), c(s - 3.0, o0 + 1.6), WHITE);
        }
        for side in [1.0, -1.0] {
            let q = town.world(s, side * (KERB + 1.0));
            let foot = lift(q, paved(q) - 0.1);
            b.column(foot, 0.12, 0.12, 0.6, 6, DARK);
            // Stripes up the pole, each sitting on the one below
            b.tag("deck");
            for k in 1..6 {
                let c = if k % 2 == 0 { DARK } else { WHITE };
                b.column(foot + Vec3::Y * k as f32 * 0.6, 0.12, 0.12, 0.6, 6, c);
            }
            b.tag("towns");
            b.glowing = true;
            b.blob(foot + Vec3::Y * 4.0, 0.42, 1.0, BEACON, side);
            b.glowing = false;
            b.post(foot, 0.25, 4.0);
        }
    }

    // Lamps at the square's open corners
    for s in [-SQUARE_HALF + 1.5, SQUARE_HALF - 1.5] {
        let p = town.world(s, town.square * (FRONT + SQUARE_DEPTH - 2.0));
        if inside(p) {
            street_lamp(b, lift(p, paved(p) - 0.1), town.frame(s).1 * -town.square);
        }
    }
}

/// A black iron lamp post, its lantern hanging out over `toward`.
fn street_lamp(b: &mut Builder, foot: Vec3, toward: Vec2) {
    let t = vec3(toward.x, 0.0, toward.y);
    b.column(foot, 0.22, 0.13, LAMP_HEIGHT, 6, IRON);
    b.column(foot, 0.32, 0.28, 1.0, 6, IRON);
    b.post(foot, 0.3, LAMP_HEIGHT);
    let top = foot + Vec3::Y * LAMP_HEIGHT;
    b.limb(top - Vec3::Y * 0.3, top + t * 1.6 - Vec3::Y * 0.1, 0.06, 0.06, 4, 0.0, IRON);
    let lantern = top + t * 1.6 - Vec3::Y * 0.4;
    let yaw = (-toward.y).atan2(toward.x);
    b.block(lantern + Vec3::Y * 0.3, 0.4, 0.12, 0.4, yaw, IRON, IRON);
    b.glowing = true;
    b.block(lantern - Vec3::Y * 0.5, 0.28, 0.8, 0.28, yaw, LAMP, LAMP);
    b.glowing = false;
}

/// A string of pennants sagging across the street between two lamp posts.
fn bunting(b: &mut Builder, from: Vec3, to: Vec3, seed: i32) {
    let span = from.distance(to);
    let flags = (span / 1.6) as usize;
    let sag = 1.6 + span * 0.02;
    let at = |t: f32| from.lerp(to, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t);
    const SEGMENTS: usize = 8;
    for k in 0..SEGMENTS {
        let (t0, t1) = (k as f32 / SEGMENTS as f32, (k + 1) as f32 / SEGMENTS as f32);
        b.limb(at(t0), at(t1), 0.03, 0.03, 3, 0.0, IRON);
    }
    let layer = std::mem::replace(&mut b.layer, crate::props::Layer::Flag);
    for k in 1..flags {
        let t0 = (k as f32 - 0.35) / flags as f32;
        let t1 = (k as f32 + 0.35) / flags as f32;
        let (a, c) = (at(t0), at(t1));
        let tip = (a + c) * 0.5 - Vec3::Y * 0.9;
        let color = BUNTING[(k + seed.unsigned_abs() as usize) % BUNTING.len()];
        // A triangle as a cloth quad with its tip doubled up, free to flutter
        b.cloth([a, c, tip, tip], [0.0, 0.0, 0.6, 0.6], color);
    }
    b.layer = layer;
}

/// People out on the pavements and in the square.
fn townsfolk(b: &mut Builder, town: &Town, inside: &impl Fn(Vec2) -> bool) {
    for side in [1.0, -1.0] {
        let mut s = -HALF_LEN + 6.0;
        while s < HALF_LEN - 6.0 {
            let seed = town.seed + s * 0.13 + side * 7.0;
            if hash(seed, 1.0) < 0.45 {
                let o = KERB + 1.8 + (FRONT - KERB - 3.0) * hash(seed, 2.0);
                let p = town.world(s, side * o);
                if inside(p) {
                    // Most watch the road; some look in the shop windows
                    let n = town.frame(s).1 * side;
                    let facing = if hash(seed, 3.0) < 0.75 { -n } else { n };
                    crate::venue::person(b, lift(p, paved(p)), facing, seed, 0.15 * hash(seed, 4.0));
                }
            }
            s += 7.0 + 6.0 * hash(seed, 5.0);
        }
    }
    let f = town.world(0.0, town.square * (FRONT + SQUARE_DEPTH * 0.5));
    for k in 0..8 {
        let seed = town.seed + k as f32 * 5.3;
        let a = hash(seed, 6.0) * std::f32::consts::TAU;
        let p = f + Vec2::from_angle(a) * (6.0 + 12.0 * hash(seed, 7.0));
        if inside(p) {
            crate::venue::person(b, lift(p, paved(p)), Vec2::from_angle(hash(seed, 8.0) * std::f32::consts::TAU), seed, 0.0);
        }
    }
}

/// The town's name on a white board at the roadside as you arrive, and
/// on the back, as you leave, the name struck through in red.
fn entry_sign(b: &mut Builder, town: &Town, end: f32, inside: &impl Fn(Vec2) -> bool) {
    let s = end * (HALF_LEN + 25.0);
    let (_, n) = town.frame(s);
    // Keep to the right of traffic arriving from this end
    let side = end;
    let p = town.world(s, side * (KERB + 5.0));
    if !inside(p) {
        return;
    }
    let t = Vec2::new(n.y, -n.x);
    let facing = t * end;
    let px = (9.0 / text_units(&town.name)).min(0.42);
    let half_w = text_units(&town.name) * px * 0.5 + 1.0;
    let height = crate::hud::GLYPH_H as f32 * px + 2.6;
    let normal = vec3(facing.x, 0.0, facing.y);
    let right = vec3(-facing.y, 0.0, facing.x);
    let yaw = (-facing.x).atan2(-facing.y);
    let foot = crate::props::ground(p, 1.0);
    let bottom = vec3(p.x, foot + 1.8, p.y);
    for k in [-1.0, 1.0] {
        let post = vec3(p.x, foot - 0.4, p.y) + right * k * (half_w - 0.6);
        b.limb(post, post + Vec3::Y * (2.2 + height), 0.1, 0.1, 4, 0.0, [0.6, 0.62, 0.64, 1.0]);
        b.post(post, 0.2, 2.2 + height);
    }
    // The board, up on its posts
    b.tag("deck");
    b.block(bottom - Vec3::Y * 0.12, half_w + 0.12, height + 0.24, 0.06, yaw, DARK, DARK);
    b.block(bottom, half_w, height, 0.1, yaw, WHITE, WHITE);
    b.tag("towns");
    let w = text_units(&town.name) * px;
    let welcome = "WELCOME TO";
    let small = 0.14;
    for (normal, r, leaving) in [(normal, right, false), (-normal, -right, true)] {
        let centre = vec3(p.x, bottom.y, p.y) + normal * 0.12;
        let name_top = bottom.y + height - 1.6;
        lettering(b, vec3(centre.x, name_top, centre.z) - r * w * 0.5, r, -Vec3::Y, px, &town.name, DARK);
        if leaving {
            // Struck through
            let (l, rr) = (centre - r * (w * 0.5 + 0.3), centre + r * (w * 0.5 + 0.3));
            let mid = name_top - crate::hud::GLYPH_H as f32 * px * 0.5;
            let (a, c) = (vec3(l.x, mid - 0.9, l.z) + normal * 0.03, vec3(rr.x, mid + 0.9, rr.z) + normal * 0.03);
            let up = Vec3::Y * 0.18;
            b.quad(a - up, c - up, c + up, a + up, SIGN_RED);
        } else {
            let ww = text_units(welcome) * small;
            lettering(b, vec3(centre.x, bottom.y + height - 0.3, centre.z) - r * ww * 0.5, r, -Vec3::Y, small, welcome, DARK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towns_line_the_roads() {
        let network = crate::sim_tests::network();
        let towns = all();
        for t in towns {
            println!("{} ({:?}) at {:.0},{:.0}: {} buildings", t.name, t.style, t.centre.x, t.centre.y, t.lots.len());
        }
        assert!(towns.len() >= 4, "only {} towns", towns.len());
        for (i, t) in towns.iter().enumerate() {
            assert!(towns[i + 1..].iter().all(|u| u.name != t.name), "two towns called {}", t.name);
            assert!(t.lots.len() >= 20, "{} has only {} buildings", t.name, t.lots.len());
            assert!(t.lots.iter().any(|l| matches!(l.usage, Use::Shop(_))), "{} has no shops", t.name);
            // Every building stands clear of every path: fronts and backs
            for lot in &t.lots {
                for (a, d) in [(-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let p = t.world(lot.s + a * lot.half_w, lot.side * (FRONT + d * lot.depth));
                    let near = network.nearest(p.x, p.y);
                    assert!(near.is_none_or(|n| n.dist > n.half_width() + 6.0), "{} building corner at {p} on a path", t.name);
                }
                assert!(occupied(t.world(lot.s, lot.side * (FRONT + lot.depth * 0.5)), 0.0));
            }
            // No two buildings on a side overlap
            for (k, a) in t.lots.iter().enumerate() {
                for c in &t.lots[k + 1..] {
                    if a.side == c.side {
                        assert!((a.s - c.s).abs() >= a.half_w + c.half_w, "{}: buildings overlap at {}", t.name, a.s);
                    }
                }
            }
            assert!(at(t.centre).is_some_and(|u| u.name == t.name));
            assert!(street_walls(t.centre).is_some_and(|d| (d - FRONT).abs() < 1.0));
        }
        assert!(!occupied(Vec2::new(1.0e6, 1.0e6), 10.0));
    }
}

