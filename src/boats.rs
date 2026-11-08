//! Boats on the open water: yachts beating and reaching across the wind,
//! fishing boats steaming out to their grounds and lying to with their nets
//! down (gulls wheeling over them), and the craft tied up at the piers (see
//! `harbour`). Like the wildlife they come and go in a ring round the player;
//! every one rides the same swell the water shader draws.

use glam::{Quat, Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::harbour::{self, Craft};
use crate::props::PERSON_SCALE;
use crate::terrain::{self, WATER_LEVEL};

/// How many of each are kept sailing round the player.
const YACHTS: usize = 7;
const TRAWLERS: usize = 3;
/// Spawn this far off (nearer only while first filling the water), and
/// forget beyond the last.
const SPAWN: (f32, f32) = (1100.0, 1700.0);
const FIRST_SPAWN: f32 = 150.0;
const FORGET: f32 = 2100.0;
/// Drawn out to this far (the haze has them by then).
const DRAW: f32 = 1900.0;
/// Moored boats are drawn out to this far.
const MOORED_DRAW: f32 = 1200.0;
/// Which way the wind blows (toward), for the sails.
const WIND: Vec2 = vec2(0.8, 0.6);
/// Matches `WATER_DEEP_AT` in `chunks` and the swell in `mesh.wgsl`.
const DEEP_AT: f32 = 8.0;

/// How many yachts and fishing boats are out at `clock` (0..1 from midnight).
fn afloat_at(clock: f32) -> (usize, usize) {
    let h = clock * 24.0;
    let yachts = match h {
        h if (8.0..19.0).contains(&h) => YACHTS,
        h if (6.5..8.0).contains(&h) || (19.0..21.0).contains(&h) => YACHTS / 2,
        _ => 0,
    };
    let trawlers = match h {
        // Out at dawn in force, a few through the day, one fishing the night
        h if (4.5..10.0).contains(&h) => TRAWLERS + 2,
        h if (10.0..20.0).contains(&h) => TRAWLERS,
        _ => 1,
    };
    (yachts, trawlers)
}

/// The swell's height at `p` at time `t`, as `wave_height` in `mesh.wgsl`.
fn wave(p: Vec2, t: f32) -> f32 {
    (p.x * 0.13 + t * 1.1).sin() * 0.5 + (p.y * 0.17 - t * 0.9).sin() * 0.35 + ((p.x - p.y) * 0.09 + t * 0.7).sin() * 0.45
}

/// How much of the swell reaches `p` (none at the shore, all of it in deep water).
fn swell(p: Vec2) -> f32 {
    ((WATER_LEVEL - terrain::raw_height(p.x, p.y)) / DEEP_AT).clamp(0.0, 1.0)
}

/// The water surface at `p` at time `t`, swell and all.
pub(crate) fn surface_at(p: Vec2, t: f32) -> f32 {
    surface(p, t, swell(p))
}

/// The water surface at `p`, `swell` of the way to the full swell.
fn surface(p: Vec2, t: f32, swell: f32) -> f32 {
    WATER_LEVEL + 0.3 + wave(p, t) * 0.7 * swell
}

/// Open water at least `depth` deep (not under a causeway).
fn wet(p: Vec2, depth: f32) -> bool {
    terrain::raw_height(p.x, p.y) < WATER_LEVEL - depth && terrain::solid_height(p.x, p.y) < WATER_LEVEL - depth
}

#[derive(Clone, Copy, PartialEq)]
enum Doing {
    Sailing,
    /// Steaming out for this long more.
    Steaming(f32),
    /// Nets down, drifting, for this long more.
    Fishing(f32),
}

struct Boat {
    craft: Craft,
    pos: Vec2,
    heading: f32,
    /// Where it's trying to point.
    want: f32,
    speed: f32,
    doing: Doing,
    /// Until the next look ahead for shallows.
    look: f32,
    seed: f32,
    /// Making for this pier's end, out of the weather.
    home: Option<Vec2>,
}

pub struct Boats {
    boats: Vec<Boat>,
    /// How many of each should be out at this hour.
    want: (usize, usize),
    rng: u32,
    filled: bool,
    /// Spawn ring (dev aid STUMBLER_BOATS_CLOSE=1 brings it in for screenshots).
    ring: (f32, f32),
}

impl Boats {
    pub fn new(seed: u32) -> Self {
        let close = std::env::var_os("STUMBLER_BOATS_CLOSE").is_some();
        Self { boats: Vec::new(), want: (YACHTS, TRAWLERS), rng: seed | 1, filled: false, ring: if close { (50.0, 160.0) } else { SPAWN } }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng & 0xffffff) as f32 / 0x1000000 as f32
    }

    /// `clock` is the time of day (0..1 from midnight): yachts are out by
    /// day, and the fishing boats put out from the piers at dawn.
    /// `rain` (0..1): in the wet the boats make for the nearest pier.
    pub fn update(&mut self, player: Vec3, clock: f32, rain: f32, dt: f32) {
        let me = vec2(player.x, player.z);
        self.boats.retain(|b| b.pos.distance(me) < FORGET);
        // Tied up at the pier they made for
        self.boats.retain(|b| b.home.is_none_or(|h| b.pos.distance(h) > 30.0));
        self.want = afloat_at(clock);
        if rain > 0.3 {
            self.want = (0, 0);
            for b in self.boats.iter_mut().filter(|b| b.home.is_none()) {
                b.home = harbour::get().piers.iter()
                    .map(|p| p.root + p.dir * (p.length + 20.0))
                    .filter(|h| h.distance(b.pos) < 3000.0)
                    .min_by(|x, y| x.distance(b.pos).total_cmp(&y.distance(b.pos)));
                if b.craft == Craft::Trawler {
                    b.doing = Doing::Steaming(600.0);
                }
            }
        } else {
            for b in &mut self.boats {
                b.home = None;
            }
        }
        // More out than there should be: those well out of sight go home
        for craft in [Craft::Yacht, Craft::Trawler] {
            let want = if craft == Craft::Yacht { self.want.0 } else { self.want.1 };
            let out = self.boats.iter().filter(|b| b.craft == craft).count();
            if out > want {
                if let Some(i) = self.boats.iter().position(|b| b.craft == craft && b.pos.distance(me) > SPAWN.0 * 0.9) {
                    self.boats.swap_remove(i);
                }
            }
        }
        let h = clock * 24.0;
        if (4.5..8.0).contains(&h) {
            self.put_out(me);
        }
        // On a long jump across the map, fill the water afresh
        if self.boats.is_empty() {
            self.filled = false;
        }
        self.spawn(me);
        for i in 0..self.boats.len() {
            let r = [self.rand(), self.rand()];
            // Give way to any boat close ahead
            let (pos, heading) = (self.boats[i].pos, self.boats[i].heading);
            let fwd = vec2(heading.sin(), heading.cos());
            let crowd = self.boats.iter().enumerate()
                .filter(|&(j, o)| j != i && o.pos.distance(pos) < 100.0 && (o.pos - pos).dot(fwd) > -20.0)
                .map(|(_, o)| (o.pos - pos).perp_dot(fwd))
                .next();
            let b = &mut self.boats[i];
            if let Some(side) = crowd {
                // It's off to one side: bear away from it
                b.want = heading + if side > 0.0 { 0.6 } else { -0.6 };
            }
            b.update(dt, r);
        }
    }

    /// At dawn, a fishing boat now and then leaves a pier near the player,
    /// steaming out to sea.
    fn put_out(&mut self, me: Vec2) {
        let trawlers = self.boats.iter().filter(|b| b.craft == Craft::Trawler).count();
        if trawlers >= self.want.1 || self.rand() > 0.004 {
            return;
        }
        let piers = &harbour::get().piers;
        let Some(pier) = piers.iter().filter(|p| p.root.distance(me) < 1400.0).min_by(|a, b| a.root.distance(me).total_cmp(&b.root.distance(me))) else { return };
        let pos = pier.root + pier.dir * (pier.length + 24.0);
        if !wet(pos, 2.0) || self.boats.iter().any(|b| b.pos.distance(pos) < 60.0) {
            return;
        }
        let heading = pier.dir.x.atan2(pier.dir.y);
        let seed = self.rand();
        let steam = 60.0 + 60.0 * self.rand();
        self.boats.push(Boat { craft: Craft::Trawler, pos, heading, want: heading, speed: 2.0, doing: Doing::Steaming(steam), look: 0.0, seed, home: None });
    }

    fn spawn(&mut self, me: Vec2) {
        let tries = if self.filled { 2 } else { 60 };
        let near = if self.filled { self.ring.0 } else { FIRST_SPAWN.min(self.ring.0) };
        for _ in 0..tries {
            let yachts = self.boats.iter().filter(|b| b.craft == Craft::Yacht).count();
            let trawlers = self.boats.iter().filter(|b| b.craft == Craft::Trawler).count();
            let (want_yachts, want_trawlers) = self.want;
            if yachts >= want_yachts && trawlers >= want_trawlers {
                break;
            }
            let a = self.rand() * std::f32::consts::TAU;
            let d = near + (self.ring.1 - near) * self.rand();
            let p = me + Vec2::from_angle(a) * d;
            // Plenty of open water round about, so they aren't boxed in
            let room = (self.ring.0 * 0.5).min(90.0);
            let open = wet(p, 4.0) && (0..8).all(|k| wet(p + Vec2::from_angle(k as f32 * std::f32::consts::FRAC_PI_4) * room, 2.5));
            if !open || self.boats.iter().any(|b| b.pos.distance(p) < room * 1.7) {
                continue;
            }
            let craft = if trawlers < want_trawlers && (yachts >= want_yachts || self.rand() < 0.3) { Craft::Trawler } else { Craft::Yacht };
            let heading = self.rand() * std::f32::consts::TAU;
            let doing = match craft {
                Craft::Trawler if self.rand() < 0.5 => Doing::Fishing(20.0 + 40.0 * self.rand()),
                Craft::Trawler => Doing::Steaming(30.0 + 60.0 * self.rand()),
                _ => Doing::Sailing,
            };
            let seed = self.rand();
            self.boats.push(Boat { craft, pos: p, heading, want: heading, speed: 0.0, doing, look: 0.0, seed, home: None });
        }
        self.filled = true;
    }

    /// The fishing boats' engines: where, and firing strokes a second
    /// (ticking over while fishing, working hard steaming).
    pub fn engines(&self) -> impl Iterator<Item = (Vec3, f32)> + '_ {
        self.boats.iter().filter(|b| b.craft == Craft::Trawler)
            .map(|b| (vec3(b.pos.x, WATER_LEVEL + 1.5, b.pos.y), if matches!(b.doing, Doing::Fishing(_)) { 6.0 } else { 10.0 }))
    }

    /// Where each boat is, on the water.
    pub fn afloat(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.boats.iter().map(|b| vec3(b.pos.x, WATER_LEVEL + 1.5, b.pos.y))
    }

    /// How many boats are making for a pier.
    #[cfg(test)]
    pub fn homeward(&self) -> usize {
        self.boats.iter().filter(|b| b.home.is_some()).count()
    }

    /// Where each boat under way is.
    #[cfg(test)]
    pub fn positions(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.boats.iter().map(|b| b.pos)
    }

    /// The boats near enough to see: their hulls and rigging, and the
    /// lights that glow after dark.
    pub fn build_mesh(&self, camera: Vec3, time: f32) -> ((Vec<Vertex>, Vec<u16>), (Vec<Vertex>, Vec<u16>)) {
        let cam = vec2(camera.x, camera.z);
        let mut m = Mesh::default();
        let mut glow = Mesh::default();
        for b in self.boats.iter().filter(|b| b.pos.distance(cam) < DRAW) {
            let fishing = matches!(b.doing, Doing::Fishing(_));
            let going = b.speed > 1.5;
            let sw = swell(b.pos);
            let pose = Pose::afloat(b.craft, b.pos, b.heading, time, sw, if b.craft == Craft::Yacht { heel(b.heading) } else { 0.0 });
            if going {
                wake(&mut m, b.craft, b.pos, b.heading, b.speed, time, sw);
            }
            build(&mut m, &mut glow, &pose, b.craft, b.seed, if b.craft == Craft::Yacht { Rig::Sailing(boom(b.heading)) } else { Rig::Working(fishing) });
            if b.craft == Craft::Trawler {
                gulls(&mut m, pose.at(Vec3::ZERO), if fishing { 7 } else { 3 }, time, b.seed);
            }
        }
        for moor in harbour::get().moorings.iter().filter(|m| m.pos.distance(cam) < MOORED_DRAW) {
            // Swinging a little on the ropes
            let heading = moor.heading + (time * 0.25 + moor.seed * 20.0).sin() * 0.06;
            let pose = Pose::afloat(moor.craft, moor.pos, heading, time, swell(moor.pos), 0.0);
            build(&mut m, &mut glow, &pose, moor.craft, moor.seed, Rig::Moored);
        }
        ((m.verts, m.idxs), (glow.verts, glow.idxs))
    }
}

/// How far a yacht heeling on `heading` leans over: most with the wind on
/// the beam, little running before it or close-hauled.
fn heel(heading: f32) -> f32 {
    let fwd = Vec2::from_angle(std::f32::consts::FRAC_PI_2 - heading);
    let across = fwd.perp_dot(WIND.normalize());
    // Leans away from the wind (negative leans to starboard)
    across * 0.22
}

/// How far the boom swings out (and which side, + to starboard).
fn boom(heading: f32) -> f32 {
    let fwd = Vec2::from_angle(std::f32::consts::FRAC_PI_2 - heading);
    let w = WIND.normalize();
    // 0 head to wind, 1 dead downwind
    let off = (1.0 + fwd.dot(w)) * 0.5;
    let side = if fwd.perp_dot(w) > 0.0 { -1.0 } else { 1.0 };
    side * (0.2 + 1.1 * off)
}

impl Boat {
    fn update(&mut self, dt: f32, r: [f32; 2]) {
        let fwd = |h: f32| vec2(h.sin(), h.cos());
        // Now and then look ahead, and steer for clear water
        self.look -= dt;
        if self.look <= 0.0 {
            self.look = 0.4;
            let reach = 80.0 + self.speed * 5.0;
            let clear = |h: f32| wet(self.pos + fwd(h) * reach, 2.5) && wet(self.pos + fwd(h) * reach * 0.5, 2.5);
            if !clear(self.heading) {
                let way = [0.5, -0.5, 1.0, -1.0, 1.6, -1.6, 2.4, -2.4, std::f32::consts::PI].into_iter()
                    .map(|d| self.heading + d)
                    .find(|&h| clear(h));
                self.want = way.unwrap_or(self.heading + std::f32::consts::PI);
            } else if let Some(home) = self.home {
                // Making for the pier
                let to = home - self.pos;
                self.want = to.x.atan2(to.y);
            } else {
                // Wander a little
                self.want += (r[0] - 0.5) * 0.25;
            }
            if self.craft == Craft::Yacht {
                // Can't sail within 45 degrees of the wind: bear away
                let into = fwd(self.want).dot(-WIND.normalize());
                if into > 0.7 {
                    let side = if fwd(self.want).perp_dot(-WIND) > 0.0 { -1.0 } else { 1.0 };
                    self.want += side * 0.4;
                }
            }
        }
        let mut diff = self.want - self.heading;
        while diff > std::f32::consts::PI { diff -= std::f32::consts::TAU; }
        while diff < -std::f32::consts::PI { diff += std::f32::consts::TAU; }
        let turn_rate = if self.craft == Craft::Yacht { 0.22 } else { 0.3 };
        self.heading += diff.clamp(-turn_rate * dt, turn_rate * dt);

        let target = match (self.craft, self.doing) {
            (Craft::Yacht, _) => {
                // Fastest across the wind, slow close-hauled
                let across = fwd(self.heading).perp_dot(WIND.normalize()).abs();
                let behind = fwd(self.heading).dot(WIND.normalize()).max(0.0);
                5.0 + 7.0 * across + 3.0 * behind
            }
            (_, Doing::Fishing(_)) => 0.6,
            _ => 6.5,
        } * (1.0 - diff.abs().min(1.0) * 0.4);
        self.speed += (target - self.speed) * (dt * 0.3).min(1.0);
        let next = self.pos + fwd(self.heading) * self.speed * dt;
        if wet(next, 1.5) {
            self.pos = next;
        } else {
            self.speed = 0.0;
            self.want = self.heading + std::f32::consts::PI;
        }
        self.doing = match self.doing {
            Doing::Steaming(t) if t <= 0.0 => Doing::Fishing(30.0 + 50.0 * r[1]),
            Doing::Steaming(t) => Doing::Steaming(t - dt),
            Doing::Fishing(t) if t <= 0.0 => Doing::Steaming(40.0 + 60.0 * r[1]),
            Doing::Fishing(t) => Doing::Fishing(t - dt),
            Doing::Sailing => Doing::Sailing,
        };
    }
}

// ── Models ──

#[derive(Default)]
struct Mesh {
    verts: Vec<Vertex>,
    idxs: Vec<u16>,
}

impl Mesh {
    fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: [f32; 3]) {
        if self.verts.len() > 65000 {
            return;
        }
        let base = self.verts.len() as u16;
        for p in [a, b, c] {
            self.verts.push(Vertex { pos: p.into(), color: [col[0], col[1], col[2], 1.0] });
        }
        self.idxs.extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, col: [f32; 3]) {
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }
}

/// Where a boat floats and how it's tipped by the swell under it.
struct Pose {
    origin: Vec3,
    rot: Quat,
}

impl Pose {
    fn afloat(craft: Craft, pos: Vec2, heading: f32, t: f32, swell: f32, heel: f32) -> Self {
        let (len, beam) = size(craft);
        let fwd = vec2(heading.sin(), heading.cos());
        let right = vec2(fwd.y, -fwd.x);
        let bow = surface(pos + fwd * len * 0.4, t, swell);
        let stern = surface(pos - fwd * len * 0.4, t, swell);
        let port = surface(pos - right * beam * 0.5, t, swell);
        let starboard = surface(pos + right * beam * 0.5, t, swell);
        let y = (bow + stern + port + starboard) * 0.25;
        let pitch = ((bow - stern) / (len * 0.8)).atan();
        // Positive roll tips the mast to port
        let roll = -((port - starboard) / beam).atan() * 0.7 + heel;
        let rot = Quat::from_rotation_y(heading) * Quat::from_rotation_x(-pitch) * Quat::from_rotation_z(roll);
        Self { origin: vec3(pos.x, y, pos.y), rot }
    }

    /// A point in the boat's frame (+z the bow, +x starboard, y up from the
    /// waterline) out in the world.
    fn at(&self, p: Vec3) -> Vec3 {
        // Modelled a unit to the metre
        self.origin + self.rot * (p * PERSON_SCALE)
    }
}

/// Length and beam, in the world's units.
pub(crate) fn size(craft: Craft) -> (f32, f32) {
    let (len, beam) = match craft {
        Craft::Dinghy => (4.2, 1.6),
        Craft::Trawler => (13.0, 4.4),
        Craft::Yacht => (10.0, 3.2),
    };
    (len * PERSON_SCALE, beam * PERSON_SCALE)
}

enum Rig {
    /// Under sail, the boom swung out this far.
    Sailing(f32),
    /// A fishing boat, nets down or not.
    Working(bool),
    /// Tied up: sails furled, nobody at work.
    Moored,
}

const HULLS: [[f32; 3]; 6] = [[0.92, 0.92, 0.9], [0.16, 0.24, 0.42], [0.62, 0.14, 0.12], [0.18, 0.36, 0.26], [0.1, 0.1, 0.12], [0.85, 0.62, 0.18]];
const ANTIFOUL: [f32; 3] = [0.5, 0.14, 0.12];
const DECK_WOOD: [f32; 3] = [0.62, 0.5, 0.36];
const WHITE: [f32; 3] = [0.93, 0.93, 0.9];
const SAIL: [f32; 3] = [0.96, 0.95, 0.9];
const DARK: [f32; 3] = [0.14, 0.14, 0.16];
const METAL: [f32; 3] = [0.7, 0.71, 0.72];
const GLASS: [f32; 3] = [0.2, 0.26, 0.3];

fn build(m: &mut Mesh, glow: &mut Mesh, pose: &Pose, craft: Craft, seed: f32, rig: Rig) {
    let pick = |n: usize, salt: f32| ((seed * 977.0 + salt * 131.0).sin().abs() * 1000.0) as usize % n;
    let f = |p: Vec3| pose.at(p);
    match craft {
        Craft::Yacht => {
            let hull = if pick(3, 1.0) == 0 { HULLS[1 + pick(4, 2.0)] } else { WHITE };
            let stripe = HULLS[1 + pick(3, 3.0)];
            let deck = hull_shape(m, &f, 10.0, 3.2, 1.0, 0.6, hull, stripe, WHITE, false);
            // The cabin top, with its windows
            block(m, &f, vec3(-1.0, deck, -2.2), vec3(1.0, deck + 0.7, 1.2), WHITE, WHITE);
            for s in [-1.0, 1.0] {
                let x = s * 1.01;
                m.quad(f(vec3(x, deck + 0.3, -1.6)), f(vec3(x, deck + 0.3, 0.8)), f(vec3(x, deck + 0.55, 0.8)), f(vec3(x, deck + 0.55, -1.6)), GLASS);
            }
            // Cockpit coaming and a tiller
            block(m, &f, vec3(-0.9, deck, -4.3), vec3(0.9, deck + 0.35, -2.2), DECK_WOOD, DECK_WOOD);
            let mast_foot = vec3(0.0, deck + 0.7, 1.1);
            let mast_top = vec3(0.0, deck + 13.0, 1.1);
            limb(m, &f, mast_foot, mast_top, 0.09, METAL);
            limb(m, &f, mast_top, vec3(0.0, deck + 0.2, 4.9), 0.02, DARK);
            limb(m, &f, mast_top, vec3(0.0, deck + 0.2, -4.9), 0.02, DARK);
            let gooseneck = vec3(0.0, deck + 1.6, 1.1);
            match rig {
                Rig::Sailing(swing) => {
                    // The main on its boom, filled and curved to leeward
                    let boom_end = gooseneck + vec3(swing.sin(), 0.0, -swing.cos()) * 4.6;
                    limb(m, &f, gooseneck, boom_end, 0.06, METAL);
                    let belly = vec3(swing.signum() * 0.6, 0.0, 0.0);
                    let mid_luff = gooseneck.lerp(mast_top, 0.45);
                    let mid_leech = boom_end.lerp(mast_top, 0.4) + belly;
                    let sail = if pick(5, 4.0) == 0 { [0.95, 0.75, 0.3] } else { SAIL };
                    m.quad(f(gooseneck), f(boom_end), f(mid_leech), f(mid_luff), sail);
                    m.tri(f(mid_luff), f(mid_leech), f(mast_top - Vec3::Y * 0.3), sail);
                    // The jib, sheeted in the same side
                    let head = mast_top.lerp(mast_foot, 0.12);
                    let tack = vec3(0.0, deck + 0.3, 4.8);
                    let clew = vec3(swing.signum() * (0.6 + swing.abs() * 0.9), deck + 1.0, 0.0);
                    let mid = tack.lerp(head, 0.4).lerp(clew, 0.3) + belly * 0.8;
                    m.tri(f(tack), f(mid), f(head), SAIL);
                    m.tri(f(tack), f(clew), f(mid), SAIL);
                    m.tri(f(mid), f(clew), f(head), SAIL);
                }
                _ => {
                    // Sails furled under a blue cover along the boom
                    let boom_end = gooseneck + vec3(0.0, 0.0, -4.6);
                    block(m, &f, gooseneck + vec3(-0.25, -0.15, -4.4), boom_end + vec3(0.25, 0.35, 4.3), [0.18, 0.3, 0.5], [0.18, 0.3, 0.5]);
                }
            }
            // Masthead, port and starboard lights
            nav_lights(glow, &f, mast_top + Vec3::Y * 0.1, deck + 0.4, 4.0, 0.9);
        }
        Craft::Trawler => {
            let hull = HULLS[1 + pick(5, 1.0)];
            let deck = hull_shape(m, &f, 13.0, 4.4, 1.6, 1.1, hull, WHITE, [0.4, 0.38, 0.35], false);
            // Wheelhouse forward, windows all round, a lamp at the windows
            let (lo, hi) = (vec3(-1.4, deck, 0.4), vec3(1.4, deck + 2.3, 3.6));
            block(m, &f, lo, hi, WHITE, [0.85, 0.3, 0.2]);
            let band = |y0: f32, y1: f32| [(vec3(-1.41, y0, 0.6), vec3(-1.41, y0, 3.4), vec3(-1.41, y1, 3.4), vec3(-1.41, y1, 0.6)),
                (vec3(1.41, y0, 3.4), vec3(1.41, y0, 0.6), vec3(1.41, y1, 0.6), vec3(1.41, y1, 3.4)),
                (vec3(-1.2, y0, 3.61), vec3(1.2, y0, 3.61), vec3(1.2, y1, 3.61), vec3(-1.2, y1, 3.61))];
            for (a, b, c, d) in band(deck + 1.3, deck + 2.0) {
                glow.quad(f(a), f(b), f(c), f(d), [0.95, 0.8, 0.45]);
            }
            // Mast with a crosstree, a gantry astern, the net drum
            let mast_top = vec3(0.0, deck + 7.5, 2.0);
            limb(m, &f, vec3(0.0, deck + 2.3, 2.0), mast_top, 0.1, METAL);
            limb(m, &f, mast_top - vec3(1.2, 1.5, 0.0), mast_top - vec3(-1.2, 1.5, 0.0), 0.06, METAL);
            limb(m, &f, mast_top, vec3(0.0, deck + 3.0, -4.8), 0.03, DARK);
            for s in [-1.0, 1.0] {
                limb(m, &f, vec3(s * 1.7, deck, -5.6), vec3(s * 1.2, deck + 4.2, -5.9), 0.12, [0.85, 0.5, 0.15]);
            }
            limb(m, &f, vec3(-1.25, deck + 4.2, -5.9), vec3(1.25, deck + 4.2, -5.9), 0.12, [0.85, 0.5, 0.15]);
            limb(m, &f, vec3(-1.1, deck + 0.7, -2.8), vec3(1.1, deck + 0.7, -2.8), 0.6, [0.3, 0.38, 0.32]);
            // Fish boxes and buoys on deck
            for k in 0..3 {
                let x = -1.2 + k as f32 * 0.9;
                block(m, &f, vec3(x - 0.35, deck, -4.6), vec3(x + 0.35, deck + 0.4, -4.0), [0.25, 0.45, 0.65], [0.3, 0.5, 0.7]);
            }
            block(m, &f, vec3(1.6, deck + 0.3, -1.0), vec3(1.9, deck + 0.7, -0.6), [0.95, 0.45, 0.1], [0.95, 0.45, 0.1]);
            if let Rig::Working(true) = rig {
                // Warps out over the stern, down into the water
                for s in [-1.0, 1.0] {
                    let top = vec3(s * 1.2, deck + 4.2, -5.9);
                    let wet = vec3(s * 3.0, -0.2, -22.0);
                    limb(m, &f, top, wet, 0.04, DARK);
                }
                // Someone at work on deck
                figure(m, &f, vec3(0.6, deck, -3.6), 3.0, pick(4, 9.0));
            }
            nav_lights(glow, &f, mast_top + Vec3::Y * 0.1, deck + 0.5, 4.5, 1.4);
        }
        Craft::Dinghy => {
            let hull = HULLS[pick(6, 1.0)];
            let floor = hull_shape(m, &f, 4.2, 1.6, 0.55, 0.3, hull, hull, DECK_WOOD, true);
            for z in [-0.9, 0.4] {
                block(m, &f, vec3(-0.68, floor + 0.25, z - 0.15), vec3(0.68, floor + 0.32, z + 0.15), DECK_WOOD, DECK_WOOD);
            }
            // Oars shipped along the thwarts
            for s in [-1.0, 1.0] {
                limb(m, &f, vec3(s * 0.45, floor + 0.36, -1.6), vec3(s * 0.4, floor + 0.36, 1.4), 0.04, [0.75, 0.6, 0.4]);
            }
        }
    }
}

/// Port (red) and starboard (green) on the beam, a white light up the mast.
fn nav_lights(glow: &mut Mesh, f: &impl Fn(Vec3) -> Vec3, top: Vec3, deck: f32, z: f32, half_beam: f32) {
    let lamp = |glow: &mut Mesh, c: Vec3, col: [f32; 3]| block(glow, f, c - Vec3::splat(0.12), c + Vec3::splat(0.12), col, col);
    lamp(glow, top, [1.0, 0.97, 0.9]);
    lamp(glow, vec3(-half_beam, deck, z * 0.5), [1.0, 0.2, 0.15]);
    lamp(glow, vec3(half_beam, deck, z * 0.5), [0.2, 1.0, 0.4]);
}

/// A hull from stern (-z) to bow (+z): topsides in `side` above a stripe at
/// the waterline, antifouling below, and a deck (or for an open boat, the
/// inside and the boards on its floor). Returns the deck's height.
#[allow(clippy::too_many_arguments)]
fn hull_shape(m: &mut Mesh, f: &impl Fn(Vec3) -> Vec3, len: f32, beam: f32, freeboard: f32, draft: f32,
    side: [f32; 3], stripe: [f32; 3], deck: [f32; 3], open: bool) -> f32 {
    const STATIONS: usize = 8;
    let width = |t: f32| {
        let w = if t < 0.35 { 0.78 + 0.22 * (t / 0.35).sqrt() } else { 1.0 - ((t - 0.35) / 0.65).powf(1.8) };
        w * beam * 0.5
    };
    let sheer = |t: f32| freeboard * (1.0 + 0.3 * t * t);
    // Each station's side, top down: gunwale, stripe top, waterline, bilge, keel
    let ring = |k: usize| -> [Vec3; 5] {
        let t = k as f32 / STATIONS as f32;
        let z = (t - 0.5) * len;
        let w = width(t);
        let bottom = draft * (1.0 - 0.6 * (2.0 * t - 1.0).abs().powi(3));
        [vec3(w, sheer(t), z), vec3(w * 0.99, 0.28, z), vec3(w * 0.93, -0.05, z), vec3(w * 0.62, -bottom * 0.7, z), vec3(0.0, -bottom, z)]
    };
    let colours = [side, stripe, ANTIFOUL, ANTIFOUL];
    for k in 0..STATIONS {
        let (a, b) = (ring(k), ring(k + 1));
        for s in [-1.0, 1.0] {
            let m_ = |p: Vec3| vec3(p.x * s, p.y, p.z);
            for band in 0..4 {
                m.quad(f(m_(a[band])), f(m_(b[band])), f(m_(b[band + 1])), f(m_(a[band + 1])), colours[band]);
            }
        }
        if !open {
            let deck_y = |r: &[Vec3; 5]| r[0].y - 0.08;
            m.quad(f(vec3(-a[0].x, deck_y(&a), a[0].z)), f(vec3(a[0].x, deck_y(&a), a[0].z)), f(vec3(b[0].x, deck_y(&b), b[0].z)), f(vec3(-b[0].x, deck_y(&b), b[0].z)), deck);
        }
    }
    // The transom across the stern
    let s = ring(0);
    for band in 0..4 {
        m.quad(f(s[band]), f(vec3(-s[band].x, s[band].y, s[band].z)), f(vec3(-s[band + 1].x, s[band + 1].y, s[band + 1].z)), f(s[band + 1]), colours[band.min(3)]);
    }
    if open {
        // Inside: the hull's skin seen from within, boards on the floor
        let floor = 0.12;
        let inner = [side[0] * 0.8, side[1] * 0.8, side[2] * 0.8];
        for k in 0..STATIONS {
            let (a, b) = (ring(k), ring(k + 1));
            for s in [-1.0, 1.0] {
                let top = |r: &[Vec3; 5]| vec3(r[0].x * s * 0.92, r[0].y, r[0].z);
                let bot = |r: &[Vec3; 5]| vec3(r[0].x * s * 0.7, floor, r[0].z);
                m.quad(f(top(&a)), f(top(&b)), f(bot(&b)), f(bot(&a)), inner);
            }
            m.quad(f(vec3(-a[0].x * 0.7, floor, a[0].z)), f(vec3(a[0].x * 0.7, floor, a[0].z)), f(vec3(b[0].x * 0.7, floor, b[0].z)), f(vec3(-b[0].x * 0.7, floor, b[0].z)), deck);
        }
        return floor;
    }
    freeboard - 0.08
}

/// A box from `lo` to `hi` in the boat's frame.
fn block(m: &mut Mesh, f: &impl Fn(Vec3) -> Vec3, lo: Vec3, hi: Vec3, side: [f32; 3], top: [f32; 3]) {
    let c = |x: f32, y: f32, z: f32| f(vec3(x, y, z));
    let (a, b) = (lo, hi);
    let p = [c(a.x, a.y, a.z), c(b.x, a.y, a.z), c(b.x, a.y, b.z), c(a.x, a.y, b.z), c(a.x, b.y, a.z), c(b.x, b.y, a.z), c(b.x, b.y, b.z), c(a.x, b.y, b.z)];
    m.quad(p[4], p[5], p[6], p[7], top);
    for (i, j, k, l) in [(0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)] {
        m.quad(p[i], p[j], p[k], p[l], side);
    }
}

/// A square-section rod from `a` to `b` in the boat's frame, `r` thick.
fn limb(m: &mut Mesh, f: &impl Fn(Vec3) -> Vec3, a: Vec3, b: Vec3, r: f32, col: [f32; 3]) {
    let axis = (b - a).normalize_or_zero();
    let side = axis.cross(if axis.y.abs() > 0.9 { Vec3::X } else { Vec3::Y }).normalize_or_zero() * r;
    let up = axis.cross(side).normalize_or_zero() * r;
    let corners = [side + up, side - up, -side - up, -side + up];
    for k in 0..4 {
        let (c0, c1) = (corners[k], corners[(k + 1) % 4]);
        m.quad(f(a + c0), f(a + c1), f(b + c1), f(b + c0), col);
    }
}

/// A deckhand in oilskins at `at`, facing `yaw` round from the bow.
fn figure(m: &mut Mesh, f: &impl Fn(Vec3) -> Vec3, at: Vec3, yaw: f32, tone: usize) {
    let coat = [[0.95, 0.75, 0.1], [0.95, 0.45, 0.1], [0.2, 0.3, 0.5], [0.3, 0.45, 0.3]][tone % 4];
    let r = Quat::from_rotation_y(yaw);
    let g = |p: Vec3| f(at + r * p);
    block(m, &g, vec3(-0.25, 0.0, -0.15), vec3(0.25, 0.8, 0.15), [0.2, 0.2, 0.22], [0.2, 0.2, 0.22]);
    block(m, &g, vec3(-0.28, 0.8, -0.18), vec3(0.28, 1.45, 0.18), coat, coat);
    block(m, &g, vec3(-0.15, 1.45, -0.15), vec3(0.15, 1.75, 0.15), [0.85, 0.68, 0.55], coat);
}

/// White water off the bow and a widening V astern, riding the swell.
fn wake(m: &mut Mesh, craft: Craft, pos: Vec2, heading: f32, speed: f32, t: f32, swell: f32) {
    let (len, beam) = size(craft);
    let fwd = vec2(heading.sin(), heading.cos());
    let right = vec2(fwd.y, -fwd.x);
    let foam = [0.86, 0.91, 0.93];
    let lift = 0.14;
    let on = |q: Vec2| vec3(q.x, surface(q, t, swell) + lift, q.y);
    let reach = (speed * 3.0).min(40.0);
    let stern = pos - fwd * len * 0.45;
    let steps = 6;
    for s in [-1.0, 1.0] {
        for k in 0..steps {
            let d0 = reach * k as f32 / steps as f32;
            let d1 = reach * (k + 1) as f32 / steps as f32;
            // The arms spread at about 20 degrees, thinning as they go
            let spread = |d: f32| beam * 0.5 + d * 0.36;
            let thick = |d: f32| (0.9 - d / reach * 0.7) * (speed / 10.0).min(1.0);
            let a = stern - fwd * d0 + right * s * spread(d0);
            let b = stern - fwd * d1 + right * s * spread(d1);
            let (ta, tb) = (thick(d0), thick(d1));
            m.quad(on(a), on(b), on(b + right * s * tb), on(a + right * s * ta), foam);
        }
    }
    // Churned water right behind
    let wash = (speed * 1.2).min(12.0);
    m.quad(on(stern - right * beam * 0.35), on(stern + right * beam * 0.35), on(stern - fwd * wash + right * beam * 0.2), on(stern - fwd * wash - right * beam * 0.2), foam);
    // A bow wave each side
    let bow = pos + fwd * len * 0.48;
    for s in [-1.0, 1.0] {
        let back = pos + fwd * len * 0.1 + right * s * (beam * 0.55);
        m.quad(on(bow), on(back), on(back + right * s * 0.5 - fwd * 0.6), on(bow + right * s * 0.25), foam);
    }
}

/// Gulls wheeling over a fishing boat at `over`.
fn gulls(m: &mut Mesh, over: Vec3, count: usize, t: f32, seed: f32) {
    for k in 0..count {
        let s = seed * 31.0 + k as f32 * 1.7;
        let speed = 0.35 + 0.2 * (s * 3.1).sin().abs();
        let a = t * speed * if k % 2 == 0 { 1.0 } else { -1.0 } + s;
        let r = 6.0 + 5.0 * (s * 1.3).sin().abs();
        let c = over + vec3(a.cos() * r, 9.0 + 4.0 * (s * 2.7).sin().abs() + (t * 0.6 + s).sin() * 1.2, a.sin() * r);
        // Facing along the circle
        let along = vec3(-a.sin(), 0.0, a.cos()) * if k % 2 == 0 { 1.0 } else { -1.0 };
        let side = vec3(along.z, 0.0, -along.x);
        let flap = (t * 4.0 + s * 5.0).sin() * 0.35;
        let span = 1.3;
        let body = [0.95, 0.95, 0.93];
        let wing = [0.8, 0.82, 0.85];
        for w in [-1.0, 1.0] {
            let tip = c + side * w * span + Vec3::Y * flap;
            m.tri(c + along * 0.35, c - along * 0.3, tip - along * 0.15, wing);
        }
        m.tri(c + along * 0.55, c - along * 0.6 + side * 0.14, c - along * 0.6 - side * 0.14, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wind_fills_the_sails_from_behind() {
        // Running before the wind the boom is right out; heading into it, hardly
        let downwind = WIND.x.atan2(WIND.y);
        assert!(boom(downwind).abs() > 1.2);
        let reaching = downwind + std::f32::consts::FRAC_PI_2;
        assert!(boom(reaching).abs() > 0.6 && boom(reaching).abs() < 0.9);
        assert!(heel(reaching).abs() > 0.2);
    }

    #[test]
    fn boats_float_on_the_swell_as_drawn() {
        // Out in deep water a boat sits at the shader's surface
        let p = vec2(0.0, 0.0);
        let t = 3.7;
        let expected = WATER_LEVEL + 0.3 + wave(p, t) * 0.7;
        let deep = (WATER_LEVEL - terrain::raw_height(p.x, p.y) >= DEEP_AT).then_some(());
        if deep.is_some() {
            assert!((surface(p, t, swell(p)) - expected).abs() < 1e-4);
        }
    }
}
