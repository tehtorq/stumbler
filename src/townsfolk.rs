//! People out and about in the towns: strolling the pavements and stopping
//! at shop windows, waiting at the zebra crossing until the road is clear,
//! sitting on the benches round the fountain, chatting in the square, and
//! jumping out of the way of a car that comes at them.
//!
//! A town is peopled while you're near it. Everyone is kept in the town's
//! own frame (along the street, out from its centreline), which makes
//! pavements, the crossing and the square simple to stay on.

use glam::{Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::props::PERSON_SCALE;
use crate::road::ROAD_HALF_WIDTH;
use crate::terrain::terrain_height;
use crate::towns::{self, Town, BENCHES, CHAIR_OFF, CROSSING, FRONT, HALF_LEN, KERB, PAVE, SQUARE_DEPTH, SQUARE_HALF, STROLL_OUT};
use crate::venue::{HAIR, SHIRTS, SKINS};

/// A town fills with people inside this distance, and empties beyond the next.
const PEOPLE_NEAR: f32 = 520.0;
const PEOPLE_GONE: f32 = 700.0;
/// Drawn out to this far.
const DRAW: f32 = 420.0;
/// How many stroll, sit, and gather in the square, in the daytime.
const WALKERS: usize = 26;
const SQUARE_FOLK: usize = 8;
/// Walking and running pace (a unit is half a metre).
const WALK: (f32, f32) = (2.2, 3.2);
const RUN: f32 = 11.0;
/// A car this near, or this near and coming on, sends people running.
const TOO_CLOSE: f32 = 9.0;
const COMING: f32 = 20.0;
/// Waiting at the crossing until no car is this near it.
const CLEAR: f32 = 38.0;

#[derive(Clone, Copy, PartialEq)]
enum Doing {
    /// Strolling along the pavement, this way along the street.
    Strolling,
    /// Looking in a shop window for a while.
    Browsing(f32),
    /// At the kerb by the crossing, waiting for the road to clear.
    Waiting,
    /// Over the stripes to the far pavement (this side of the street).
    Crossing(f32),
    /// Wandering the square toward a spot, then stopping to chat.
    Wandering(Vec2),
    Chatting(f32),
    /// Sat on a bench, facing the fountain.
    Sitting,
    /// A child after the ball, running to where it's got to.
    Football(Vec2),
    /// A child on swing `seat` in the playground.
    Swinging(u8),
    /// Outside the pub with a pint, now and then taking a sip (the clock
    /// runs on from this).
    Drinking(f32),
    /// Running from a car (away from it) for a while.
    Fleeing(f32, Vec2),
    /// Then standing and staring after it.
    Staring(f32),
}

struct Person {
    town: usize,
    /// Along the street, and out from its centreline (left positive).
    at: Vec2,
    /// Which way along the street they're strolling (+1 or -1).
    way: f32,
    /// Facing, in the town's frame.
    facing: Vec2,
    doing: Doing,
    pace: f32,
    /// Step clock.
    phase: f32,
    /// Moving this fast just now (drives the legs).
    speed: f32,
    seed: f32,
    /// What they last stared after.
    stare_at: Vec2,
    /// A child (drawn small; with a school bag when out walking).
    child: bool,
}

#[derive(Default)]
pub struct Townsfolk {
    people: Vec<Person>,
    /// The towns holding their market just now.
    markets: Vec<usize>,
    /// Each town's football (town, where, how it's rolling), while the kids play.
    balls: Vec<(usize, Vec2, Vec2)>,
    /// The clock, for the swings.
    time: f32,
    /// How hard it's raining (0..1): umbrellas up, and fewer out.
    rain: f32,
    /// Towns peopled just now.
    peopled: Vec<usize>,
    rng: u32,
}

impl Townsfolk {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `cars` are where every car near enough to matter is, and how it's moving.
    /// `clock` is the time of day (0..1 from midnight): the streets are
    /// busiest by day, the pubs in the evening, and empty in the small hours.
    /// `rain` (0..1): in the wet the umbrellas go up, folk hurry, the
    /// square and the pub fronts empty, and fewer come out.
    pub fn update(&mut self, player: Vec3, cars: &[(Vec3, Vec3)], day: u32, clock: f32, rain: f32, dt: f32) {
        self.rain = rain;
        // A market drawing a crowd to the square
        let markets: Vec<usize> = self.peopled.iter().copied().filter(|&t| crate::market::market_on(t, day, clock)).collect();
        let opening: Vec<usize> = markets.iter().copied().filter(|t| !self.markets.contains(t)).collect();
        for t in opening {
            for _ in 0..14 {
                let spot = square_spot(&towns::all()[t], [self.rand(), self.rand()]);
                let wait = 2.0 + 8.0 * self.rand();
                let p = self.person(t, spot, 1.0, Doing::Chatting(wait));
                self.people.push(p);
            }
        }
        self.markets = markets;
        self.time += dt;
        self.children(day, clock, dt);
        let me = vec2(player.x, player.z);
        let all = towns::all();
        // Empty the towns left behind, fill the ones arrived at
        self.peopled.retain(|&t| all[t].centre.distance(me) < PEOPLE_GONE);
        let peopled = self.peopled.clone();
        self.people.retain(|p| peopled.contains(&p.town));
        for (t, town) in all.iter().enumerate() {
            if town.centre.distance(me) < PEOPLE_NEAR && !self.peopled.contains(&t) {
                self.peopled.push(t);
                self.populate(t, clock);
            }
        }
        self.keep_hours(clock);
        if rain > 0.2 {
            // Off the benches and out of the square, in out of the rain
            let leave = dt * 0.15;
            let rolls: Vec<f32> = (0..self.people.len()).map(|_| self.rand()).collect();
            let mut k = 0;
            self.people.retain(|p| {
                k += 1;
                !(matches!(p.doing, Doing::Sitting | Doing::Chatting(_) | Doing::Wandering(_) | Doing::Drinking(_)) && rolls[k - 1] < leave)
            });
        }
        let hurry = 1.0 + 0.5 * (rain > 0.2) as u8 as f32;
        for i in 0..self.people.len() {
            let r = [self.rand(), self.rand(), self.rand()];
            let town = &all[self.people[i].town];
            // The cars as this town sees them
            let near: Vec<(Vec2, Vec2)> = cars.iter()
                .filter(|(p, _)| vec2(p.x, p.z).distance(town.centre) < HALF_LEN + 80.0)
                .filter_map(|&(p, v)| {
                    let at = town.local(vec2(p.x, p.z))?;
                    let ahead = town.local(vec2(p.x + v.x, p.z + v.z))?;
                    Some((at, ahead - at))
                })
                .collect();
            self.people[i].update(town, &near, dt * hurry, r);
        }
    }

    /// Whether anyone's on town `town`'s zebra crossing, or waiting at it.
    pub fn crossing_busy(&self, town: usize) -> bool {
        self.people.iter().any(|p| p.town == town && matches!(p.doing, Doing::Crossing(_) | Doing::Waiting) && (p.at.x - CROSSING).abs() < 3.0)
    }

    /// The folk near `at`: how many (pub crowds count double, they're
    /// louder), and the middle of them.
    pub fn crowd_near(&self, at: Vec3, reach: f32) -> Option<(f32, Vec3)> {
        let all = towns::all();
        let (mut n, mut sum) = (0.0, Vec2::ZERO);
        for p in &self.people {
            let pos = all[p.town].world(p.at.x, p.at.y);
            if pos.distance(vec2(at.x, at.z)) < reach {
                let w = if matches!(p.doing, Doing::Drinking(_) | Doing::Chatting(_)) { 2.0 } else { 1.0 };
                n += w;
                sum += pos * w;
            }
        }
        (n > 0.0).then(|| {
            let c = sum / n;
            (n, vec3(c.x, terrain_height(c.x, c.y) + 2.0, c.y))
        })
    }

    /// Everyone's place in their town's frame, whether they're on the
    /// crossing (or waiting at it), and whether they're running.
    #[cfg(test)]
    pub fn report(&self) -> Vec<(Vec2, bool, bool, bool)> {
        let all = towns::all();
        self.people.iter().map(|p| (p.at, matches!(p.doing, Doing::Crossing(_) | Doing::Waiting), matches!(p.doing, Doing::Fleeing(..)), walkable(&all[p.town], p.at) || matches!(p.doing, Doing::Sitting | Doing::Drinking(_) | Doing::Swinging(_) | Doing::Football(_)))).collect()
    }

    /// As the hours pass: the pubs fill at opening time and empty at
    /// closing, and the streets fill or empty toward how busy they should be.
    fn keep_hours(&mut self, clock: f32) {
        // Up from the tables at closing: the cafés' in the evening, the pubs' late
        let h = clock * 24.0;
        let cafes_open = (8.5..18.5).contains(&h);
        let all = towns::all();
        self.people.retain(|p| {
            if p.doing != Doing::Sitting || (p.at.y.abs() - towns::TABLE_OUT).abs() > 0.5 {
                return true;
            }
            let table = all[p.town].tables().into_iter().find(|(l, ..)| (l.x - p.at.x).abs() < CHAIR_OFF + 0.5 && l.y.signum() == p.at.y.signum());
            match table {
                Some((_, true, _)) => pubs_open(clock),
                Some((_, false, _)) => cafes_open,
                None => true,
            }
        });
        for t in self.peopled.clone() {
            let drinking = self.people.iter().any(|p| p.town == t && matches!(p.doing, Doing::Drinking(_)));
            if pubs_open(clock) && !drinking && self.rain < 0.2 {
                self.fill_pubs(t);
            } else if !pubs_open(clock) && drinking {
                // Time, gentlemen: off home along the street
                for p in self.people.iter_mut().filter(|p| p.town == t && matches!(p.doing, Doing::Drinking(_))) {
                    p.doing = Doing::Strolling;
                    p.way = if p.at.x > 0.0 { 1.0 } else { -1.0 };
                }
            }
            let walkers = self.people.iter().filter(|p| p.town == t && p.doing == Doing::Strolling).count();
            let want = (WALKERS as f32 * busy(clock) * (1.0 - 0.6 * self.rain.min(1.0))) as usize;
            if walkers > want + 2 {
                // Too many about: whoever reaches the end of the street goes on out of town
                self.people.retain(|p| !(p.town == t && p.doing == Doing::Strolling && p.at.x.abs() > HALF_LEN - 7.0 && p.at.x * p.way > 0.0));
            } else if walkers + 2 < want && self.rand() < 0.01 {
                // More arriving, in from one end
                let way = if self.rand() < 0.5 { 1.0 } else { -1.0 };
                let side = if self.rand() < 0.5 { 1.0 } else { -1.0 };
                let o = side * (KERB + 1.5 + (STROLL_OUT - KERB - 2.0) * self.rand());
                let p = self.person(t, vec2(-way * (HALF_LEN - 8.0), o), way, Doing::Strolling);
                self.people.push(p);
            }
        }
    }

    /// Folk sat at the café tables (by day) or the pub's (while it's open).
    fn fill_tables(&mut self, t: usize, pubs: bool, share: f32) {
        for (l, pub_, _) in towns::all()[t].tables() {
            if pub_ != pubs {
                continue;
            }
            for s in [-1.0, 1.0] {
                if self.rand() < share {
                    let at = l + vec2(s * CHAIR_OFF, 0.0);
                    let mut p = self.person(t, at, 1.0, Doing::Sitting);
                    p.facing = vec2(-s, 0.0);
                    self.people.push(p);
                }
            }
        }
    }

    /// The children: out to play in the square through the day and after
    /// school (football, the swings), and walking home with their school
    /// bags at half past three. Indoors in the rain, the dark, or market days
    /// (for the football).
    fn children(&mut self, day: u32, clock: f32, dt: f32) {
        let h = clock * 24.0;
        let out = self.rain < 0.2 && (9.0..19.5).contains(&h);
        let home_time = (15.3..16.5).contains(&h) && self.rain < 0.5;
        let all = towns::all();
        for t in self.peopled.clone() {
            let playing = out && !crate::market::market_on(t, day, clock);
            let has = |f: &Self, d: fn(&Doing) -> bool| f.people.iter().any(|p| p.town == t && p.child && d(&p.doing));
            if playing && !has(self, |d| matches!(d, Doing::Football(_))) {
                let ((s0, s1), (o0, o1)) = PITCH;
                let centre = vec2((s0 + s1) * 0.5, all[t].square * (FRONT + (o0 + o1) * 0.5));
                self.balls.retain(|b| b.0 != t);
                self.balls.push((t, centre, Vec2::ZERO));
                for _ in 0..4 + (self.rand() * 3.0) as usize {
                    let at = centre + vec2(self.rand() - 0.5, self.rand() - 0.5) * 10.0;
                    let mut p = self.person(t, at, 1.0, Doing::Football(centre));
                    p.child = true;
                    self.people.push(p);
                }
            }
            if playing && !has(self, |d| matches!(d, Doing::Swinging(_))) {
                for seat in 0..2u8 {
                    if self.rand() < 0.8 {
                        let mut p = self.person(t, vec2(SWINGS.0, all[t].square * (FRONT + SWINGS.1)), 1.0, Doing::Swinging(seat));
                        p.child = true;
                        self.people.push(p);
                    }
                }
            }
            if !playing {
                self.people.retain(|p| !(p.town == t && p.child && matches!(p.doing, Doing::Football(_) | Doing::Swinging(_))));
                self.balls.retain(|b| b.0 != t);
            }
            // Home from school: a few at a time along the pavements
            if home_time && self.people.iter().filter(|p| p.town == t && p.child && p.doing == Doing::Strolling).count() < 6 && self.rand() < dt * 0.4 {
                let way = if self.rand() < 0.5 { 1.0 } else { -1.0 };
                let side = if self.rand() < 0.5 { 1.0 } else { -1.0 };
                let o = side * (KERB + 1.5 + (STROLL_OUT - KERB - 2.0) * self.rand());
                for k in 0..1 + (self.rand() * 2.5) as usize {
                    let mut p = self.person(t, vec2(-way * (HALF_LEN - 8.0) - way * k as f32 * 2.0, o), way, Doing::Strolling);
                    p.child = true;
                    p.pace *= 0.9;
                    self.people.push(p);
                }
            }
        }
        // The ball: rolling to a stop, booted on by whoever reaches it
        let kicks: Vec<f32> = (0..self.balls.len()).map(|_| self.rand()).collect();
        for (n, ball) in self.balls.iter_mut().enumerate() {
            let ((s0, s1), (o0, o1)) = PITCH;
            let sq = all[ball.0].square;
            ball.1 += ball.2 * dt;
            ball.2 *= 1.0 - (dt * 1.2).min(1.0);
            // Kept to the pitch, bouncing off its edges
            let (lo, hi) = (vec2(s0, (FRONT + o0).min(FRONT + o1) * sq), vec2(s1, (FRONT + o1) * sq));
            let (ylo, yhi) = (lo.y.min(hi.y), lo.y.max(hi.y));
            if ball.1.x < s0 || ball.1.x > s1 { ball.2.x = -ball.2.x; ball.1.x = ball.1.x.clamp(s0, s1); }
            if ball.1.y < ylo || ball.1.y > yhi { ball.2.y = -ball.2.y; ball.1.y = ball.1.y.clamp(ylo, yhi); }
            let near = self.people.iter().any(|p| p.town == ball.0 && matches!(p.doing, Doing::Football(_)) && p.at.distance(ball.1) < 1.6);
            if near && ball.2.length() < 3.0 {
                ball.2 = Vec2::from_angle(kicks[n] * std::f32::consts::TAU) * (9.0 + 6.0 * kicks[n]);
            }
        }
        // The kids chase it (each with its own idea of where it's going)
        // (the nearest goes for it, the rest hang about in a loose ring)
        let balls = self.balls.clone();
        let nearest: Vec<(usize, Option<usize>)> = balls.iter().map(|b| (b.0, self.people.iter().enumerate()
            .filter(|(_, p)| p.town == b.0 && matches!(p.doing, Doing::Football(_)))
            .min_by(|x, y| x.1.at.distance(b.1).total_cmp(&y.1.at.distance(b.1))).map(|(i, _)| i))).collect();
        for (i, p) in self.people.iter_mut().enumerate().filter(|(_, p)| matches!(p.doing, Doing::Football(_))) {
            if let Some(&(_, at, vel)) = balls.iter().find(|b| b.0 == p.town) {
                let chaser = nearest.iter().any(|&(t, n)| t == p.town && n == Some(i));
                let ring = if chaser { Vec2::ZERO } else { Vec2::from_angle(p.seed) * (3.0 + (p.seed * 0.37) % 3.0) };
                p.doing = Doing::Football(at + vel * 0.3 + ring);
            }
        }
    }

    /// A knot of drinkers outside each pub, pints in hand.
    fn fill_pubs(&mut self, t: usize) {
        self.fill_tables(t, true, 0.8);
        for (s, side, half_w) in towns::all()[t].pubs() {
            let groups = 1 + (self.rand() * 2.0) as usize;
            for g in 0..groups {
                let centre = vec2(s + (g as f32 - (groups - 1) as f32 * 0.5) * half_w * 0.9, side * (FRONT - 2.6));
                let n = 2 + (self.rand() * 3.0) as usize;
                for k in 0..n {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU + self.rand();
                    let at = centre + Vec2::from_angle(a) * 1.6;
                    let sip = self.rand() * 10.0;
                    let mut p = self.person(t, at, 1.0, Doing::Drinking(sip));
                    p.facing = (centre - at).normalize_or_zero();
                    self.people.push(p);
                }
            }
        }
    }

    fn populate(&mut self, t: usize, clock: f32) {
        let town = &towns::all()[t];
        let share = busy(clock) * (1.0 - 0.6 * self.rain.min(1.0));
        let wet = self.rain > 0.2;
        // Coffee and cake at the café tables through the day
        let h = clock * 24.0;
        if !wet && (8.5..18.5).contains(&h) {
            self.fill_tables(t, false, 0.55 * share);
        }
        let walkers = (WALKERS as f32 * share) as usize;
        for _ in 0..walkers {
            let side = if self.rand() < 0.5 { 1.0 } else { -1.0 };
            let s = (self.rand() * 2.0 - 1.0) * (HALF_LEN - 8.0);
            let o = side * (KERB + 1.2 + (STROLL_OUT - KERB - 1.6) * self.rand());
            let way = if self.rand() < 0.5 { 1.0 } else { -1.0 };
            let person = self.person(t, vec2(s, o), way, Doing::Strolling);
            self.people.push(person);
        }
        // Folk in the square: some on the benches, the rest about
        let fountain = vec2(0.0, town.square * (FRONT + SQUARE_DEPTH * 0.5));
        for (k, &(s, d)) in BENCHES.iter().enumerate() {
            if !wet && self.rand() < 0.65 * share {
                let at = vec2(s, town.square * (FRONT + SQUARE_DEPTH * d));
                let mut p = self.person(t, at, 1.0, Doing::Sitting);
                p.facing = (fountain - at).normalize_or_zero();
                // Shuffled along a little, now and then two to a bench
                p.at += p.facing.perp() * (self.rand() - 0.5) * 2.4;
                let facing = p.facing;
                self.people.push(p);
                if k % 2 == 0 && self.rand() < 0.35 {
                    let mut q = self.person(t, at - facing.perp() * 2.4, 1.0, Doing::Sitting);
                    q.facing = facing;
                    self.people.push(q);
                }
            }
        }
        for _ in 0..if wet { 0 } else { (SQUARE_FOLK as f32 * share) as usize } {
            let spot = square_spot(town, [self.rand(), self.rand()]);
            let chat = Doing::Chatting(2.0 + 8.0 * self.rand());
            let p = self.person(t, spot, 1.0, chat);
            self.people.push(p);
        }
    }

    fn person(&mut self, town: usize, at: Vec2, way: f32, doing: Doing) -> Person {
        let pace = WALK.0 + (WALK.1 - WALK.0) * self.rand();
        let seed = self.rand() * 1000.0;
        let facing = Vec2::from_angle(self.rand() * std::f32::consts::TAU);
        Person { town, at, way, facing: if doing == Doing::Strolling { vec2(way, 0.0) } else { facing }, doing, pace, phase: self.rand() * 10.0, speed: 0.0, seed, stare_at: at, child: false }
    }

    /// Everyone near enough to see.
    pub fn build_mesh(&self, camera: Vec3) -> (Vec<Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let all = towns::all();
        let mut m = Mesh::default();
        for p in &self.people {
            let town = &all[p.town];
            let pos = town.world(p.at.x, p.at.y);
            if pos.distance(cam) > DRAW {
                continue;
            }
            let ahead = town.world(p.at.x + p.facing.x, p.at.y + p.facing.y);
            let facing = (ahead - pos).normalize_or_zero();
            let y = terrain_height(pos.x, pos.y) + if p.at.y.abs() > KERB { PAVE } else { 0.03 };
            let pose = match p.doing {
                Doing::Sitting => Pose::Sitting,
                // A sip every so often
                Doing::Drinking(t) => Pose::Drinking((t * 0.7).sin() > 0.85),
                Doing::Fleeing(..) => Pose::Running,
                _ if p.speed > 0.2 => Pose::Walking,
                _ => Pose::Standing,
            };
            m.umbrella = self.rain > 0.2;
            let square = p.at.y * town.square > FRONT && p.at.x.abs() < SQUARE_HALF;
            m.shopping = self.markets.contains(&p.town) && square && !m.umbrella;
            if let Doing::Swinging(seat) = p.doing {
                swinger(&mut m, town, seat, self.time + p.seed, p.seed);
                continue;
            }
            let pose = if matches!(p.doing, Doing::Football(_)) && p.speed > 0.2 { Pose::Running } else { pose };
            let first = m.verts.len();
            let feet = vec3(pos.x, y, pos.y);
            m.figure(feet, facing, p.phase, pose, p.seed);
            if p.child {
                if p.doing == Doing::Strolling {
                    // The school bag on their back
                    let f3 = vec3(facing.x, 0.0, facing.y);
                    let r3 = vec3(facing.y, 0.0, -facing.x);
                    m.box_at(feet + Vec3::Y * 1.2 * PERSON_SCALE - f3 * 0.24 * PERSON_SCALE, r3, f3, vec3(0.2, 0.22, 0.09) * PERSON_SCALE, [0.15, 0.3, 0.65], [0.15, 0.3, 0.65]);
                }
                for v in &mut m.verts[first..] {
                    v.pos = (feet + (Vec3::from(v.pos) - feet) * CHILD).into();
                }
            }
        }
        // The swings' frames' seats and chains, and the balls
        for &t in &self.peopled {
            let town = &all[t];
            if town.centre.distance(cam) > DRAW {
                continue;
            }
            for seat in 0..2u8 {
                if !self.people.iter().any(|p| p.town == t && p.doing == Doing::Swinging(seat)) {
                    swing_seat(&mut m, town, seat, 0.0);
                }
            }
        }
        for &(t, at, _) in &self.balls {
            let town = &all[t];
            let p = town.world(at.x, at.y);
            let y = terrain_height(p.x, p.y) + PAVE + 0.22 * PERSON_SCALE;
            m.box_at(vec3(p.x, y, p.y), Vec3::X, Vec3::Z, Vec3::splat(0.11 * PERSON_SCALE), [0.95, 0.95, 0.95], [0.1, 0.1, 0.1]);
        }
        (m.verts, m.idxs)
    }
}

/// Children are drawn this much smaller than grown-ups.
const CHILD: f32 = 0.62;
/// The swings: where the frame stands in the square (along the street, and
/// in from its front edge), and how long the chains are (metres).
const SWINGS: (f32, f32) = (19.0, 7.0);
const CHAIN: f32 = 1.6;
const SWING_TOP: f32 = 2.2;
/// The kids' football pitch: a corner of the square (along, and in from its front).
const PITCH: ((f32, f32), (f32, f32)) = ((-SQUARE_HALF + 3.0, -8.0), (4.0, 18.0));

/// Where swing `seat`'s pivot is, in the world, and which way it swings.
fn swing_pivot(town: &Town, seat: u8) -> (Vec3, Vec3) {
    let s = SWINGS.0 + if seat == 0 { -1.0 } else { 1.0 } * 0.9 * PERSON_SCALE;
    let o = town.square * (FRONT + SWINGS.1);
    let p = town.world(s, o);
    let out = (town.world(s, o + town.square) - p).normalize_or_zero();
    (vec3(p.x, terrain_height(p.x, p.y) + PAVE + SWING_TOP * PERSON_SCALE, p.y), vec3(out.x, 0.0, out.y))
}

/// A swing's seat hanging from its chains, swung `angle` out.
fn swing_seat(m: &mut Mesh, town: &Town, seat: u8, angle: f32) -> Vec3 {
    let (pivot, out) = swing_pivot(town, seat);
    let side = out.cross(Vec3::Y);
    let hang = (-Vec3::Y * angle.cos() + out * angle.sin()) * CHAIN * PERSON_SCALE;
    let seat_at = pivot + hang;
    for s in [-1.0, 1.0] {
        m.limb(pivot + side * s * 0.25 * PERSON_SCALE, seat_at + side * s * 0.25 * PERSON_SCALE, 0.015 * PERSON_SCALE, [0.55, 0.55, 0.58]);
    }
    m.box_at(seat_at, side, out, vec3(0.28, 0.025, 0.12) * PERSON_SCALE, [0.15, 0.15, 0.15], [0.8, 0.15, 0.1]);
    seat_at
}

/// A child swinging: the seat on its chains, the child sat on it.
fn swinger(m: &mut Mesh, town: &Town, seat: u8, t: f32, seed: f32) {
    let angle = (t * 2.1).sin() * 0.7;
    let seat_at = swing_seat(m, town, seat, angle);
    let (_, out) = swing_pivot(town, seat);
    let first = m.verts.len();
    let feet = seat_at - Vec3::Y * 0.62 * PERSON_SCALE;
    m.figure(feet, vec2(out.x, out.z), 0.0, Pose::Sitting, seed);
    for v in &mut m.verts[first..] {
        v.pos = (seat_at + (Vec3::from(v.pos) - seat_at) * CHILD).into();
    }
}

/// How busy the streets are at `clock` (0..1 from midnight), as a share of
/// the daytime crowd.
fn busy(clock: f32) -> f32 {
    let h = clock * 24.0;
    match h {
        h if h < 5.0 => 0.05,
        h if h < 7.0 => 0.25,
        h if h < 9.0 => 0.7,
        h if h < 17.0 => 1.0,
        h if h < 20.0 => 0.8,
        h if h < 23.0 => 0.4,
        _ => 0.15,
    }
}

/// The pubs are open (and busy outside) from six till half eleven.
fn pubs_open(clock: f32) -> bool {
    (18.0..23.5).contains(&(clock * 24.0))
}

/// Somewhere in the square, clear of the fountain.
fn square_spot(town: &Town, r: [f32; 2]) -> Vec2 {
    let s = (r[0] * 2.0 - 1.0) * (SQUARE_HALF - 3.0);
    let d = 3.0 + (SQUARE_DEPTH - 6.0) * r[1];
    let mut p = vec2(s, town.square * (FRONT + d));
    let fountain = vec2(0.0, town.square * (FRONT + SQUARE_DEPTH * 0.5));
    if p.distance(fountain) < 6.0 {
        p = fountain + (p - fountain).try_normalize().unwrap_or(Vec2::X) * 6.5;
    }
    p
}

/// Whether `at` is somewhere people may walk without crossing: the
/// pavements and the square.
fn walkable(town: &Town, at: Vec2) -> bool {
    let o = at.y.abs();
    let pavement = at.x.abs() < HALF_LEN && o > KERB + 0.8 && o < STROLL_OUT;
    let square = at.x.abs() < SQUARE_HALF - 1.0 && at.y * town.square > FRONT - 1.0 && o < FRONT + SQUARE_DEPTH - 1.5;
    pavement || square
}

impl Person {
    fn update(&mut self, town: &Town, cars: &[(Vec2, Vec2)], dt: f32, r: [f32; 3]) {
        // A car coming at them: run
        if !matches!(self.doing, Doing::Fleeing(..)) {
            let threat = cars.iter().find(|(c, step)| {
                let to_me = self.at - *c;
                let d = to_me.length();
                let fast = step.length() > 3.0;
                fast && (d < TOO_CLOSE || d < COMING && step.normalize_or_zero().dot(to_me / d.max(0.1)) > 0.8)
            });
            if let Some((c, step)) = threat {
                // Away from the car and off its line, whichever side is nearer
                let along = step.normalize_or_zero();
                let off = (self.at - *c).perp_dot(along).signum();
                let away = (along.perp() * -off + (self.at - *c).normalize_or_zero()).try_normalize().unwrap_or(Vec2::Y);
                self.doing = Doing::Fleeing(1.2 + 0.6 * r[0], away);
                self.stare_at = *c;
            }
        }

        let mut goal: Option<(Vec2, f32)> = None;
        self.doing = match self.doing {
            Doing::Strolling => {
                goal = Some((self.at + vec2(self.way, 0.0) * 4.0, self.pace));
                let near_crossing = (self.at.x - CROSSING).abs() < 0.6 * self.pace && self.way * (CROSSING - self.at.x) >= 0.0;
                if self.at.x.abs() > HALF_LEN - 6.0 && self.at.x * self.way > 0.0 {
                    // The end of the street: turn back
                    self.way = -self.way;
                    Doing::Strolling
                } else if near_crossing && r[0] < dt * 0.6 {
                    Doing::Waiting
                } else if r[1] < dt * 0.06 {
                    Doing::Browsing(3.0 + 6.0 * r[2])
                } else {
                    Doing::Strolling
                }
            }
            Doing::Browsing(t) => {
                // Turned to the shop front
                self.facing = vec2(0.0, self.at.y.signum());
                if t <= 0.0 { Doing::Strolling } else { Doing::Browsing(t - dt) }
            }
            Doing::Waiting => {
                // Step to the kerb at the stripes, look up and down the road
                let side = self.at.y.signum();
                let kerb = vec2(CROSSING, side * (KERB + 1.2));
                goal = Some((kerb, self.pace));
                let clear = cars.iter().all(|(c, step)| c.distance(vec2(CROSSING, 0.0)) > CLEAR || step.length() < 0.5);
                if self.at.distance(kerb) < 0.4 {
                    self.facing = vec2((r[0] * 4.0).sin(), -side).normalize();
                    if clear { Doing::Crossing(-side) } else { Doing::Waiting }
                } else {
                    Doing::Waiting
                }
            }
            Doing::Crossing(to) => {
                let target = vec2(CROSSING, to * (KERB + 2.5));
                // Over the road while not yet past the middle; hurry if a car comes
                let hurry = cars.iter().any(|(c, step)| c.distance(self.at) < 30.0 && step.length() > 2.0);
                goal = Some((target, self.pace * if hurry { 2.2 } else { 1.0 }));
                if (self.at - target).length() < 0.4 { Doing::Strolling } else { Doing::Crossing(to) }
            }
            Doing::Wandering(to) => {
                goal = Some((to, self.pace * 0.8));
                if self.at.distance(to) < 0.8 { Doing::Chatting(3.0 + 9.0 * r[0]) } else { Doing::Wandering(to) }
            }
            Doing::Chatting(t) => {
                if t <= 0.0 { Doing::Wandering(square_spot(town, [r[0], r[1]])) } else { Doing::Chatting(t - dt) }
            }
            Doing::Sitting => Doing::Sitting,
            Doing::Football(to) => {
                goal = Some((to, self.pace * 1.6));
                Doing::Football(to)
            }
            Doing::Swinging(seat) => Doing::Swinging(seat),
            Doing::Drinking(t) => Doing::Drinking(t + dt),
            Doing::Fleeing(t, away) => {
                goal = Some((self.at + away * 4.0, RUN));
                if t <= 0.0 { Doing::Staring(2.0 + 2.0 * r[0]) } else { Doing::Fleeing(t - dt, away) }
            }
            Doing::Staring(t) => {
                self.facing = (self.stare_at - self.at).try_normalize().unwrap_or(self.facing);
                if t > 0.0 {
                    Doing::Staring(t - dt)
                } else if self.at.y * town.square > FRONT && self.at.x.abs() < SQUARE_HALF {
                    Doing::Wandering(square_spot(town, [r[1], r[2]]))
                } else {
                    Doing::Strolling
                }
            }
        };

        // Step toward the goal, keeping to where they may walk
        self.speed = 0.0;
        if let Some((goal, pace)) = goal {
            let to = goal - self.at;
            if to.length() > 0.05 {
                let dir = to.normalize();
                let step = dir * (pace * dt).min(to.length());
                let next = self.at + step;
                let on_road = matches!(self.doing, Doing::Crossing(_) | Doing::Waiting);
                let fleeing = matches!(self.doing, Doing::Fleeing(..));
                if on_road || walkable(town, next) || fleeing && next.y.abs() > KERB + 0.3 {
                    self.at = next;
                    self.speed = pace;
                    self.facing = dir;
                } else if matches!(self.doing, Doing::Strolling) {
                    // Nudged back onto the pavement
                    self.at.y = self.at.y.signum() * self.at.y.abs().clamp(KERB + 0.9, STROLL_OUT - 0.1);
                    self.at.x += self.way * pace * dt;
                    self.speed = pace;
                }
            }
        }
        // Keep out of the fountain
        let fountain = vec2(0.0, town.square * (FRONT + SQUARE_DEPTH * 0.5));
        if self.at.distance(fountain) < 5.4 {
            self.at = fountain + (self.at - fountain).try_normalize().unwrap_or(Vec2::X) * 5.4;
        }
        // Not left stranded in the road
        if !matches!(self.doing, Doing::Crossing(_) | Doing::Waiting) && self.at.y.abs() < ROAD_HALF_WIDTH {
            self.at.y = self.at.y.signum() * (KERB + 0.9);
        }
        // A stride is about a body's height, so the legs keep pace with the ground
        self.phase += dt * self.speed * 2.2 / PERSON_SCALE;
    }
}

// ── Model ──

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Pose {
    Standing,
    Walking,
    Running,
    Sitting,
    /// A pint in hand: raised to drink, or held at the chest.
    Drinking(bool),
    /// Sat on the ground, legs out in front, leaning back on the hands.
    OnGround,
    /// On a bicycle's saddle, pedalling (the cranks at this angle), hands
    /// on the bars.
    Riding(f32),
}

#[derive(Default)]
pub(crate) struct Mesh {
    pub verts: Vec<Vertex>,
    pub idxs: Vec<u16>,
    /// While set, standing and walking figures hold an umbrella up.
    pub umbrella: bool,
    /// While set, figures wear hi-vis and a hard hat, and dig.
    pub workwear: bool,
    /// While set, standing and walking figures carry a shopping bag.
    pub shopping: bool,
}

impl Mesh {
    pub(crate) fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: [f32; 3]) {
        if self.verts.len() > 65000 {
            return;
        }
        let base = self.verts.len() as u16;
        for p in [a, b, c] {
            self.verts.push(Vertex { pos: p.into(), color: [col[0], col[1], col[2], 1.0] });
        }
        self.idxs.extend_from_slice(&[base, base + 1, base + 2]);
    }

    pub(crate) fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, col: [f32; 3]) {
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    /// A box centred on `c` with half sizes along `right`, up and `front`.
    pub(crate) fn box_at(&mut self, c: Vec3, right: Vec3, front: Vec3, half: Vec3, side: [f32; 3], top: [f32; 3]) {
        let p = |x: f32, y: f32, z: f32| c + right * (x * half.x) + Vec3::Y * (y * half.y) + front * (z * half.z);
        let v = [p(-1.0, -1.0, -1.0), p(1.0, -1.0, -1.0), p(1.0, -1.0, 1.0), p(-1.0, -1.0, 1.0), p(-1.0, 1.0, -1.0), p(1.0, 1.0, -1.0), p(1.0, 1.0, 1.0), p(-1.0, 1.0, 1.0)];
        self.quad(v[4], v[5], v[6], v[7], top);
        for (i, j, k, l) in [(0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)] {
            self.quad(v[i], v[j], v[k], v[l], side);
        }
    }

    /// A square-section limb from `a` to `b`, `t` thick.
    pub(crate) fn limb(&mut self, a: Vec3, b: Vec3, t: f32, col: [f32; 3]) {
        let axis = (b - a).normalize_or_zero();
        let side = axis.cross(if axis.y.abs() > 0.9 { Vec3::X } else { Vec3::Y }).normalize_or_zero() * t;
        let up = axis.cross(side).normalize_or_zero() * t;
        let c = [side + up, side - up, -side - up, -side + up];
        for k in 0..4 {
            self.quad(a + c[k], a + c[(k + 1) % 4], b + c[(k + 1) % 4], b + c[k], col);
        }
    }

    /// Someone standing at `feet` facing `facing`, dressed by `seed`.
    pub(crate) fn figure(&mut self, feet: Vec3, facing: Vec2, phase: f32, pose: Pose, seed: f32) {
        let first = self.verts.len();
        self.figure_unscaled(feet, facing, phase, pose, seed);
        // Modelled a unit to the metre; the world is two units to it
        for v in &mut self.verts[first..] {
            v.pos = (feet + (Vec3::from(v.pos) - feet) * PERSON_SCALE).into();
        }
    }

    fn figure_unscaled(&mut self, feet: Vec3, facing: Vec2, phase: f32, pose: Pose, seed: f32) {
        let pick = |n: usize, salt: f32| (((seed * 0.37 + salt * 12.9898).sin().abs() * 43758.547).fract() * n as f32) as usize % n;
        let shirt = if self.workwear { [1.0, 0.5, 0.05] } else { SHIRTS[pick(SHIRTS.len(), 1.0)] };
        let skin = SKINS[pick(SKINS.len(), 2.0)];
        let hair = HAIR[pick(HAIR.len(), 3.0)];
        const LEGS: [[f32; 3]; 4] = [[0.2, 0.22, 0.32], [0.15, 0.15, 0.17], [0.45, 0.4, 0.32], [0.3, 0.32, 0.4]];
        let legs = LEGS[pick(4, 4.0)];
        let tall = 0.92 + 0.14 * (pick(100, 5.0) as f32 / 100.0);
        let front = vec3(facing.x, 0.0, facing.y);
        let right = vec3(facing.y, 0.0, -facing.x);
        let hip_h = 0.85 * tall;
        let (swing, arms_up, lean) = match pose {
            Pose::Walking => (phase.sin() * 0.45, false, 0.03),
            Pose::Running => (phase.sin() * 0.9, true, 0.18),
            _ => (0.0, false, 0.0),
        };
        let shoe = [0.12, 0.1, 0.1];
        let hips;
        if let Pose::Riding(crank) = pose {
            hips = feet + Vec3::Y * 0.95 - front * 0.12;
            let bracket = feet + Vec3::Y * 0.3 + front * 0.05;
            for s in [-1.0, 1.0] {
                let a = crank + if s > 0.0 { 0.0 } else { std::f32::consts::PI };
                let pedal = bracket + front * (a.cos() * 0.17) + Vec3::Y * (a.sin() * 0.17) + right * s * 0.13;
                // The knee out in front, between hip and pedal
                let knee = (hips + pedal) * 0.5 + front * 0.22 + Vec3::Y * 0.08;
                self.limb(hips + right * s * 0.12, knee, 0.1, legs);
                self.limb(knee, pedal + Vec3::Y * 0.05, 0.09, legs);
                self.box_at(pedal + Vec3::Y * 0.03, right, front, vec3(0.07, 0.04, 0.12), shoe, shoe);
            }
        } else if pose == Pose::OnGround {
            hips = feet + Vec3::Y * 0.15;
            for s in [-1.0, 1.0] {
                let foot = hips + front * 0.95 + right * s * 0.15 - Vec3::Y * 0.05;
                self.limb(hips + right * s * 0.13, foot, 0.1, legs);
                self.box_at(foot + Vec3::Y * 0.08, right, front, vec3(0.08, 0.14, 0.05), shoe, shoe);
            }
        } else if pose == Pose::Sitting {
            hips = feet + Vec3::Y * 0.62 - front * 0.05;
            for s in [-1.0, 1.0] {
                let knee = hips + front * 0.48 + right * s * 0.13 + Vec3::Y * 0.02;
                self.limb(hips + right * s * 0.13, knee, 0.1, legs);
                let foot = feet + front * 0.5 + right * s * 0.14;
                self.limb(knee, foot + Vec3::Y * 0.05, 0.09, legs);
                self.box_at(foot + Vec3::Y * 0.05 + front * 0.05, right, front, vec3(0.08, 0.05, 0.14), shoe, shoe);
            }
        } else {
            hips = feet + Vec3::Y * hip_h;
            for s in [-1.0, 1.0] {
                // Legs swing from the hip, opposite each other
                let a = swing * s;
                let foot = feet + front * (a.sin() * hip_h) + right * s * 0.13 + Vec3::Y * (1.0 - a.cos()) * hip_h * 0.5;
                self.limb(hips + right * s * 0.13, foot + Vec3::Y * 0.08, 0.1, legs);
                self.box_at(foot + Vec3::Y * 0.05 + front * 0.05, right, front, vec3(0.08, 0.05, 0.14), shoe, shoe);
            }
        }
        let body = 0.6 * tall;
        let lean = match pose { Pose::OnGround => -0.15, Pose::Riding(_) => 0.35, _ => lean };
        let chest = hips + Vec3::Y * body + front * lean;
        self.box_at((hips + chest) * 0.5, right, front, vec3(0.24, body * 0.5, 0.14), shirt, shirt);
        let head = chest + Vec3::Y * 0.2;
        self.box_at(head, right, front, vec3(0.14, 0.16, 0.14), skin, hair);
        self.box_at(head + Vec3::Y * 0.12 - front * 0.02, right, front, vec3(0.15, 0.05, 0.15), hair, hair);
        if self.workwear {
            // A hard hat, and the hi-vis stripes
            self.box_at(head + Vec3::Y * 0.17, right, front, vec3(0.18, 0.06, 0.18), [0.95, 0.95, 0.9], [0.98, 0.98, 0.95]);
            self.box_at((hips + chest) * 0.5 + Vec3::Y * 0.05, right, front, vec3(0.245, 0.03, 0.145), [0.85, 0.85, 0.8], [0.85, 0.85, 0.8]);
        }
        for s in [-1.0, 1.0] {
            let shoulder = chest - Vec3::Y * 0.05 + right * s * 0.3;
            let hand = if let (Pose::Drinking(sip), true) = (pose, s > 0.0) {
                let hand = if sip { head + front * 0.2 - Vec3::Y * 0.05 } else { chest - Vec3::Y * 0.2 + front * 0.3 };
                // The pint
                let beer = [0.85, 0.55, 0.15];
                self.box_at(hand + Vec3::Y * 0.1, right, front, vec3(0.05, 0.09, 0.05), beer, [0.95, 0.93, 0.85]);
                hand
            } else if self.umbrella && s > 0.0 && matches!(pose, Pose::Standing | Pose::Walking) {
                // An umbrella held up over the head
                let hand = chest + front * 0.28 + right * 0.12 + Vec3::Y * 0.02;
                const CANOPIES: [[f32; 3]; 6] = [[0.08, 0.08, 0.1], [0.75, 0.12, 0.12], [0.15, 0.3, 0.7], [0.95, 0.8, 0.15], [0.2, 0.5, 0.3], [0.85, 0.85, 0.82]];
                let canopy = CANOPIES[pick(6, 7.0)];
                let top = hand + Vec3::Y * 0.95 - front * 0.12;
                self.limb(hand - Vec3::Y * 0.1, top + Vec3::Y * 0.1, 0.02, [0.2, 0.2, 0.2]);
                let ribs = 8;
                for k in 0..ribs {
                    let rim = |k: usize| {
                        let a = k as f32 / ribs as f32 * std::f32::consts::TAU;
                        top + (right * a.cos() + front * a.sin()) * 0.62 - Vec3::Y * 0.18
                    };
                    let shade = if k % 2 == 0 { canopy } else { canopy.map(|c| c * 0.85) };
                    self.tri(top + Vec3::Y * 0.08, rim(k), rim(k + 1), shade);
                }
                hand
            } else if self.shopping && s < 0.0 && matches!(pose, Pose::Standing | Pose::Walking) {
                // A shopping bag in the hand, hanging
                let hand = shoulder - Vec3::Y * 0.6 + right * s * 0.06;
                const BAGS: [[f32; 3]; 4] = [[0.85, 0.75, 0.55], [0.3, 0.5, 0.35], [0.8, 0.2, 0.2], [0.25, 0.3, 0.55]];
                let bag = BAGS[pick(4, 8.0)];
                self.box_at(hand - Vec3::Y * 0.2, right, front, vec3(0.06, 0.17, 0.16), bag, bag);
                hand
            } else if arms_up {
                shoulder + Vec3::Y * 0.5 + right * s * 0.25 + front * 0.1
            } else if pose == Pose::Sitting {
                shoulder - Vec3::Y * 0.4 + front * 0.3
            } else if let Pose::Riding(_) = pose {
                // Reaching forward and down to the bars
                feet + Vec3::Y * 1.0 + front * 0.5 + right * s * 0.22
            } else if pose == Pose::OnGround {
                // Leaning back on the hands
                hips - front * 0.3 + right * s * 0.3 - Vec3::Y * 0.12
            } else {
                // Arms swing against the legs
                let a = -swing * s * 0.8;
                shoulder + front * (a.sin() * 0.55) - Vec3::Y * (a.cos() * 0.55) + right * s * 0.04
            };
            self.limb(shoulder, hand, 0.06, shirt);
            self.box_at(hand, right, front, vec3(0.05, 0.05, 0.05), skin, skin);
        }
    }
}
