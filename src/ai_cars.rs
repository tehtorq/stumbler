//! AI drivers: follow a looped path using a precomputed speed profile.

use glam::{Vec2, Vec3};

use crate::boosts::Boost;
use crate::physics::{CarInput, CarPhysics, Ground, WHEELBASE};

/// Lateral grip the AI plans corners with (units/s²); below the tyre limit.
const PLAN_GRIP: f32 = 44.0;
/// How far a racing line may run from a circuit's centreline: to within a
/// car's width and a little of the edge.
pub const RACING_ROOM: f32 = crate::road::ROAD_HALF_WIDTH - 7.0;
/// Chance a driver outbrakes themselves going into a corner, and how much
/// more likely with a car right behind them.
const MISTAKE_CHANCE: f32 = 0.05;
const PRESSURE_MISTAKES: f32 = 0.1;
/// How much too fast a mistake carries into the corner, and for how long.
const MISTAKE_OVERSPEED: f32 = 1.12;
const MISTAKE_TIME: f32 = 1.6;
/// Seconds an AI car stays stuck before it's put back on its line.
const STUCK_RESET: f32 = 3.0;
/// A car that hasn't got further than this from where it was is getting
/// nowhere.
const STUCK_RADIUS: f32 = 12.0;
/// Drivers start lining up a pass this far (units) behind a car.
const PASS_REACH: f32 = 60.0;
/// Nose to tail, roughly, and the braking a driver allows for when closing
/// on a car ahead (units/s²).
const CAR_LENGTH: f32 = 10.0;
const FOLLOW_BRAKE: f32 = 25.0;
/// Room (units) another car needs to fit through beside one.
const CAR_WIDTH: f32 = 7.0;
/// Off a standing start drivers keep straight in their lane up to this
/// speed (units/s), then merge onto their line over `LAUNCH_MERGE` seconds.
const LAUNCH_SPEED: f32 = 55.0;
const LAUNCH_MERGE: f32 = 3.0;
/// Corners this far ahead (units) shape how drivers pass and defend.
const CORNER_LOOKAHEAD: f32 = 160.0;
/// Braking deceleration the AI plans with (units/s²).
const PLAN_BRAKE: f32 = 45.0;
/// Corners worth drifting: turning at least this far (radians) within
/// `DRIFT_REACH` units, taken between these speeds (units/s).
const DRIFT_TURN: f32 = 1.2;
const DRIFT_REACH: f32 = 100.0;
const DRIFT_SPEEDS: (f32, f32) = (30.0, 80.0);
/// How long a handbrake flick into a drift lasts at most (seconds).
const DRIFT_KICK: f32 = 0.35;
/// A drift is only started with at least this much turn (radians) in the
/// next second of the corner, so it lasts long enough to pay.
const DRIFT_BEND: f32 = 0.6;
/// The slide angle (radians) held through the heart of a corner, and how
/// much more per unit it's run wide of its line (up to `DRIFT_TRIM`).
const DRIFT_HOLD: f32 = 0.25;
const DRIFT_TRIM_PER_UNIT: f32 = 0.02;
const DRIFT_TRIM: f32 = 0.1;
/// Steering on the handbrake flick: enough to start the slide, not so much
/// it swings past the angle wanted.
const DRIFT_FLICK: f32 = 0.6;

/// How a driver races. Every rival shares one brain; these set how it's
/// tuned. 0.5 is middling for the 0-to-1 traits.
#[derive(Clone, Copy, Debug)]
pub struct Personality {
    /// Multiplies the planned speed profile.
    pub pace: f32,
    /// The side of the line it likes (see `AiDriver::line_offset`).
    pub line: f32,
    /// How hard it races wheel to wheel: how close it follows, how little
    /// room it needs to dive down the inside, how firmly it covers the
    /// inside and how reluctantly it backs out alongside.
    pub aggression: f32,
    /// How rarely it outbraked itself, and how little a car close behind
    /// rattles it.
    pub composure: f32,
    /// How often it throws the car sideways through a slow corner for the
    /// drift boost (0 never, 1 every time).
    pub drift: f32,
    /// Typical reaction to the start signal (seconds).
    pub reaction: f32,
}

impl Personality {
    /// A middling driver at `pace`, keeping to `line`, who never drifts.
    pub const fn plain(pace: f32, line: f32) -> Self {
        Self { pace, line, aggression: 0.5, composure: 0.5, drift: 0.0, reaction: 0.28 }
    }
}

/// Where a driver is with a drift.
#[derive(Clone, Copy, PartialEq)]
enum Drift {
    /// No corner for it coming.
    Off,
    /// A corner coming it's decided to take on the grip, or to drift.
    Grip,
    Ready,
    /// Flicking the tail out with the handbrake for this long so far,
    /// turning toward `dir` (a steering sign).
    Kick(f32, f32),
    /// Holding the slide round the corner toward `dir`.
    Hold(f32),
}

/// A path with a target speed at every point: the line a driver follows,
/// and the road's own centreline it was laid out on. Circuits and roads are
/// loops; a gravel stage runs from its start to its finish.
pub struct AiTrack {
    /// The line driven (a racing line on circuits), one point per centreline point.
    pub points: Vec<Vec2>,
    pub speed: Vec<f32>,
    /// The road's centreline.
    pub centre: Vec<Vec2>,
    /// How far the line may stray from the centreline (0 for road traffic).
    pub room: f32,
    /// A loop (else it ends at its last point).
    pub closed: bool,
}

/// Index `i` on a path of `n` points: round a loop, or held to an open
/// path's ends.
fn wrap(i: isize, n: usize, closed: bool) -> usize {
    if closed { i.rem_euclid(n as isize) as usize } else { i.clamp(0, n as isize - 1) as usize }
}

impl AiTrack {
    /// Drive the centreline itself. `points` must be evenly spaced and form a loop.
    pub fn new(points: Vec<Vec2>, top_speed: f32) -> Self {
        Self::along(points.clone(), points, 0.0, top_speed, true)
    }

    /// Drive a racing line, up to `room` either side of the centreline
    /// `points`: out wide into a corner, clipping the apex, out wide again.
    pub fn racing(points: Vec<Vec2>, top_speed: f32, room: f32) -> Self {
        let line = racing_line(&points, room, true);
        Self::along(line, points, room, top_speed, true)
    }

    /// A gravel stage: a racing line from the first of `points` to the last,
    /// easing off to `end_speed` by the end.
    pub fn stage(points: Vec<Vec2>, top_speed: f32, room: f32, end_speed: f32) -> Self {
        let line = racing_line(&points, room, false);
        let mut track = Self::along(line, points, room, top_speed, false);
        let n = track.len();
        track.speed[n - 1] = end_speed;
        for i in (0..n - 1).rev() {
            let gap = track.points[i].distance(track.points[i + 1]).max(0.1);
            let reachable = (track.speed[i + 1].powi(2) + 2.0 * PLAN_BRAKE * gap).sqrt();
            track.speed[i] = track.speed[i].min(reachable);
        }
        track
    }

    fn along(points: Vec<Vec2>, centre: Vec<Vec2>, room: f32, top_speed: f32, closed: bool) -> Self {
        let n = points.len();
        let at = |i: isize| wrap(i, n, closed);
        let gap = |i: usize| points[i].distance(points[at(i as isize + 1)]).max(0.1);

        // Corner speed from curvature (heading change over a short window)
        let span = 4;
        let mut speed: Vec<f32> = (0..n)
            .map(|i| {
                let a = points[at(i as isize - span)];
                let b = points[i];
                let c = points[at(i as isize + span)];
                let d1 = (b - a).normalize_or_zero();
                let d2 = (c - b).normalize_or_zero();
                let angle = d1.perp_dot(d2).atan2(d1.dot(d2)).abs();
                let curvature = angle / ((a.distance(b) + b.distance(c)) * 0.5).max(0.1);
                (PLAN_GRIP / curvature.max(1e-5)).sqrt().min(top_speed)
            })
            .collect();

        // Backward pass so the car brakes in time for each corner (twice to wrap the loop)
        for _ in 0..2 {
            for i in (0..n).rev() {
                let next = speed[at(i as isize + 1)];
                let reachable = (next * next + 2.0 * PLAN_BRAKE * gap(i)).sqrt();
                speed[i] = speed[i].min(reachable);
            }
        }
        Self { points, speed, centre, room, closed }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Point `i` along the path: round the loop, or held at an open path's ends.
    pub fn at(&self, i: usize) -> usize {
        wrap(i as isize, self.len(), self.closed)
    }

    /// Point `i` along the path, counting back for negative `i`.
    pub fn at_signed(&self, i: isize) -> usize {
        wrap(i, self.len(), self.closed)
    }

    /// The step from point `i` to the next (at an open path's end, the
    /// last step), as a pair of indices.
    fn step(&self, i: usize) -> (usize, usize) {
        let i = self.at(i);
        if self.closed || i + 1 < self.len() { (i, self.at(i + 1)) } else { (i - 1, i) }
    }

    /// Direction of the driven line at point `i`.
    pub fn dir(&self, i: usize) -> Vec2 {
        let (a, b) = self.step(i);
        (self.points[b] - self.points[a]).normalize_or_zero()
    }

    /// Position and heading at centreline point `idx`, offset `lateral`
    /// units sideways.
    pub fn pose(&self, idx: usize, lateral: f32) -> (Vec2, f32) {
        let p = self.centre[self.at(idx)];
        let (a, b) = self.step(idx);
        let dir = (self.centre[b] - self.centre[a]).normalize_or_zero();
        let side = Vec2::new(-dir.y, dir.x);
        ((p + side * lateral), (-dir.x).atan2(-dir.y))
    }
}

/// A line round the loop `centre` that keeps within `room` of it and bends
/// as little as it can: each point is drawn toward the midpoint of its
/// neighbours, first far apart (whole corners) then close (the detail).
fn racing_line(centre: &[Vec2], room: f32, closed: bool) -> Vec<Vec2> {
    let n = centre.len();
    if room <= 0.0 || n < 8 {
        return centre.to_vec();
    }
    let idx = |i: isize| wrap(i, n, closed);
    let normals: Vec<Vec2> = (0..n).map(|i| {
        let t = (centre[idx(i as isize + 1)] - centre[idx(i as isize - 1)]).normalize_or_zero();
        Vec2::new(-t.y, t.x)
    }).collect();
    let mut offset = vec![0.0_f32; n];
    let at = |offset: &[f32], i: usize| centre[i] + normals[i] * offset[i];
    for k in [32, 16, 8, 4, 2, 1] {
        for _ in 0..60 {
            let prev = offset.clone();
            // An open path's ends stay on the centreline
            for i in (0..n).filter(|&i| closed || (i > 0 && i < n - 1)) {
                let mid = (at(&prev, idx(i as isize - k)) + at(&prev, idx(i as isize + k))) * 0.5;
                let want = (mid - centre[i]).dot(normals[i]);
                offset[i] = (prev[i] + (want - prev[i]) * 0.6).clamp(-room, room);
            }
        }
    }
    (0..n).map(|i| at(&offset, i)).collect()
}

/// Drives a car along an `AiTrack`: tracks lap progress and produces inputs.
pub struct AiDriver {
    /// Index of the nearest path point.
    pub progress_idx: usize,
    /// Laps completed (counted when progress wraps past the start).
    pub laps: u32,
    /// Multiplies the planned speed profile: the personality's own pace,
    /// with this race's form.
    pub pace: f32,
    pub style: Personality,
    /// The side of the line this driver likes (in 8-unit steps, + toward
    /// the line's left): on circuits taken mostly on the straights, so
    /// a field doesn't all drive down the one line.
    line_offset: f32,
    drift: Drift,
    wobble_phase: f32,
    /// Temporary lateral offset for getting around a car ahead (-1..1).
    pub(crate) avoid: f32,
    /// Seconds left of an outbraked corner (see `MISTAKE_CHANCE`).
    mistake: f32,
    /// Whether it was braking for a corner last frame.
    braking: bool,
    /// Grip on offer (1 dry, less in the wet): corner speeds come down with it.
    pub grip: f32,
    /// Getting away from a standing start: how firmly it still keeps to its
    /// grid lane (1 straight ahead, down to 0 back on its line).
    launch: f32,
    rng: u32,
}

/// Where another car is and how fast it's going, for racecraft.
#[derive(Clone, Copy)]
pub struct Traffic {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Brake lights on: it's slowing, so give it more room.
    pub braking: bool,
}

impl AiDriver {
    pub fn new(start_idx: usize, style: Personality, seed: f32) -> Self {
        Self {
            progress_idx: start_idx, laps: 0, pace: style.pace, style, line_offset: style.line, drift: Drift::Off, wobble_phase: seed, avoid: 0.0,
            mistake: 0.0, braking: false, grip: 1.0, launch: 1.0, rng: 0x9e37_79b9 ^ (seed * 7919.0) as u32,
        }
    }

    /// Preferred sideways offset from the centreline, in units.
    pub fn lateral(&self) -> f32 {
        self.line_offset * 8.0
    }

    /// Race progress in path points (laps * length + index).
    pub fn progress(&self, track_len: usize) -> f32 {
        self.laps as f32 * track_len as f32 + self.progress_idx as f32
    }

    /// Update lap progress from the car's position.
    pub fn track_progress(&mut self, track: &AiTrack, pos: Vec2) {
        let n = track.len();
        let prev_idx = self.progress_idx;
        self.progress_idx = nearest_in_window(&track.points, pos, prev_idx, 15, 80)
            .filter(|&(_, d2)| d2 < 150.0 * 150.0)
            .map(|(i, _)| i)
            .unwrap_or_else(|| nearest_in_window(&track.points, pos, 0, 0, n).map_or(prev_idx, |(i, _)| i));
        if !track.closed {
            return;
        }
        if prev_idx > n * 3 / 4 && self.progress_idx < n / 4 {
            self.laps += 1;
        } else if prev_idx < n / 4 && self.progress_idx > n * 3 / 4 && self.laps > 0 {
            self.laps -= 1; // went backwards over the line
        }
    }

    pub fn input(&mut self, track: &AiTrack, car: &CarPhysics, traffic: &[Traffic], dt: f32) -> CarInput {
        let pos = Vec2::new(car.pos.x, car.pos.z);
        self.track_progress(track, pos);
        // At the end of a stage's road: pull up
        if !track.closed && self.progress_idx + 3 >= track.len() {
            return CarInput { brake: 1.0, ..Default::default() };
        }

        let speed = car.speed;
        let spacing = track.points[0].distance(track.points[1]).max(0.1);

        // Aim at a point further ahead the faster we go, offset for line variety
        self.wobble_phase += dt * 0.7;
        // Knocked well off the road: aim at a nearer point to head straight back
        let recovering = track.points[self.progress_idx].distance(pos) > 25.0;
        let look_dist = if recovering { 24.0 } else { 18.0 + speed.abs() * 0.4 };
        let look_idx = track.at(self.progress_idx + (look_dist / spacing) as usize);
        let target = track.points[look_idx];
        let dir = track.dir(look_idx);
        let side = Vec2::new(-dir.y, dir.x);

        // Which way the next corner goes, as a side of the line (+1 toward
        // `side`), if there's one coming
        let here = track.points[self.progress_idx];
        let now_dir = (track.points[track.at(self.progress_idx + 2)] - here).try_normalize().unwrap_or(track.dir(self.progress_idx));
        let far = track.at(self.progress_idx + (CORNER_LOOKAHEAD / spacing) as usize);
        let far_dir = track.dir(far);
        let turn = now_dir.perp_dot(far_dir);
        let inside = if turn.abs() > 0.15 { turn.signum() } else { 0.0 };

        // Racecraft, in terms of the line's own sideways direction
        let heading = car.heading();
        let fwd3 = heading;
        let mut follow_speed = f32::MAX;
        // Following so closely it has to brake firmly, not just lift
        let mut urgent = false;
        let mut dodge: f32 = 0.0;
        // Making room for a car alongside comes before any passing or
        // defending (+1/-1 each way it's crowded from)
        let (mut crowded_left, mut crowded_right) = (false, false);
        let mut pressure = 0.0_f32;
        // The lane left between cars overlapping this one, as offsets from
        // the centreline: the line can't cut across a car alongside
        let here_side = Vec2::new(-now_dir.y, now_dir.x);
        let my_across = (pos - track.centre[self.progress_idx]).dot(here_side);
        let (mut lane_lo, mut lane_hi) = (f32::MIN, f32::MAX);
        let aggression = self.style.aggression;
        // A car close by: no time to be throwing it sideways (though the
        // keener a driver is on drifting, the closer it lets them get)
        let mut company = false;
        let elbow = 1.2 - 0.6 * self.style.drift;
        for other in traffic {
            let rel = other.pos - car.pos;
            let ahead = rel.dot(fwd3);
            let lateral = Vec2::new(rel.x, rel.z).dot(side);
            let closing = car.vel.dot(fwd3) - other.vel.dot(fwd3);
            company |= ahead.abs() < 25.0 * elbow && lateral.abs() < 12.0 * elbow && rel.length() > 0.5;
            // Nearly in line: keep going the way it's already pulling out
            // (or, if it isn't yet, to the side this driver likes) rather
            // than flicking from side to side
            let away = if lateral.abs() < 3.0 && self.avoid.abs() > 0.3 {
                self.avoid.signum()
            } else if lateral.abs() < 1.5 && self.line_offset != 0.0 {
                self.line_offset.signum()
            } else if lateral > 0.0 { -1.0 } else { 1.0 };
            if track.room > 0.0 && ahead.abs() < CAR_LENGTH * 1.2 && lateral.abs() < CAR_WIDTH * 1.6 && rel.length() > 0.5 {
                let beside = track.at_signed(self.progress_idx as isize + (ahead / spacing) as isize);
                let its_across = (Vec2::new(other.pos.x, other.pos.z) - track.centre[beside]).dot(here_side);
                if its_across > my_across {
                    lane_hi = lane_hi.min(its_across - CAR_WIDTH);
                } else {
                    lane_lo = lane_lo.max(its_across + CAR_WIDTH);
                }
            }
            if ahead > 1.0 && ahead < PASS_REACH * (0.8 + 0.4 * aggression) && lateral.abs() < 7.0 && closing > -2.0 {
                // Catching a car: dive for the inside of the corner ahead if
                // there's a car's width free there, otherwise go round
                // whichever side is free
                let beside = track.at(self.progress_idx + (ahead / spacing) as usize);
                let its_across = (Vec2::new(other.pos.x, other.pos.z) - track.centre[beside]).dot(side);
                let inside_free = track.room == 0.0 || track.room - its_across * inside > CAR_WIDTH * (1.3 - 0.6 * aggression);
                dodge = if inside != 0.0 && lateral * inside < 3.0 && inside_free { inside } else { away };
                // Still in line with it: keep enough gap to match its speed
                // (and a bit) before running into the back of it
                let gap = ahead - CAR_LENGTH;
                let closing_in = closing.max(0.0);
                let mut needed = closing_in * closing_in / (2.0 * FOLLOW_BRAKE) + (closing_in * 0.6 + 5.0) * (1.5 - aggression);
                if other.braking {
                    // It'll be slower still by the time we get there
                    needed += speed.abs() * 0.25;
                }
                if lateral.abs() < 5.5 && gap < needed {
                    follow_speed = follow_speed.min(other.vel.dot(fwd3) - 1.0);
                    urgent = true;
                }
            } else if ahead.abs() <= 6.0 && lateral.abs() < 9.0 {
                // Alongside: give it room, and whoever is behind backs out
                // rather than leaning on the other car
                if away > 0.0 { crowded_left = true } else { crowded_right = true }
                if ahead > 0.0 {
                    follow_speed = follow_speed.min(other.vel.dot(fwd3) - (6.0 - 4.0 * aggression));
                }
            } else if ahead < -3.0 && ahead > -30.0 && lateral.abs() < 10.0 && closing < 1.0 {
                // Someone right behind and closing: cover the inside into the
                // corner, and feel the pressure
                pressure = pressure.max(1.0 + ahead / 30.0);
                if ahead > -CAR_LENGTH && lateral.abs() > 2.5 {
                    // Its nose is already alongside: too late to close the
                    // door, so leave it a car's width
                    if away > 0.0 { crowded_left = true } else { crowded_right = true }
                } else if inside != 0.0 && dodge == 0.0 {
                    dodge = inside * (0.3 + 0.6 * aggression);
                }
            }
        }
        if crowded_left != crowded_right {
            dodge = if crowded_left { 1.0 } else { -1.0 };
        } else if crowded_left {
            dodge = 0.0; // squeezed both sides: hold the middle
        }
        let rate = 1.0 - (-2.5 * dt).exp();
        self.avoid += (dodge * 1.4 - self.avoid) * rate;
        // On circuits a driver keeps to its side of the line down the
        // straights and comes back to it for the corners
        let preference = if track.room > 0.0 {
            let bend_dir = track.dir(look_idx + 10);
            let straight = 1.0 - (dir.perp_dot(bend_dir).abs() / 0.25).min(1.0);
            self.line_offset * (0.25 + 0.75 * straight)
        } else {
            self.line_offset
        };
        let mut target = target + side * (preference + self.avoid + self.wobble_phase.sin() * 0.15) * 8.0;
        // However it's dodging, stay on the road
        if track.room > 0.0 {
            let centre = track.centre[look_idx];
            if lane_lo <= lane_hi {
                let across = (target - centre).dot(side);
                target += side * (across.clamp(lane_lo, lane_hi) - across);
                // Already closer than that: ease away now, not only by the
                // time it gets to the target
                let squeeze = my_across.clamp(lane_lo, lane_hi) - my_across;
                target += here_side * squeeze * 2.0;
            }
            let across = (target - centre).dot(side);
            let limit = track.room + 2.0;
            if across.abs() > limit {
                target -= side * (across - across.clamp(-limit, limit));
            }
            // Off the line: straight ahead in its own lane for the best
            // launch, easing over to the racing line as the speed comes up
            if self.launch > 0.0 {
                if speed > LAUNCH_SPEED {
                    self.launch = (self.launch - dt / LAUNCH_MERGE).max(0.0);
                }
                let across = (target - centre).dot(side);
                target += side * (my_across - across) * self.launch;
            }
        }

        let to_target = target - pos;
        let dist = to_target.length().max(1.0);
        let to_target = to_target / dist;
        let fwd = Vec2::new(heading.x, heading.z);
        // Pure pursuit: the arc through the target point gives a curvature, the
        // curvature gives a wheel angle, scaled by how far the wheels can turn.
        // Positive angle = target to the right (yaw increases turning right).
        let angle = to_target.perp_dot(fwd).atan2(fwd.dot(to_target));
        let curvature = 2.0 * angle.sin() / dist;
        let wheel_angle = (curvature * WHEELBASE).atan();
        let off_line = track.points[self.progress_idx].distance(pos);
        let steer = if recovering {
            // Knocked off the road: turn hard back toward it rather than
            // taking pure pursuit's long, gentle arc
            (angle * 2.5).clamp(-1.0, 1.0)
        } else {
            (wheel_angle / car.steer_limit()).clamp(-1.0, 1.0)
        };

        // Speed: plan from the profile a little ahead (reaction time)
        let plan_idx = track.at(self.progress_idx + (speed.abs() * 0.25 / spacing) as usize);
        // Corner speeds come down with the grip in the wet
        let mut planned = track.speed[plan_idx] * self.pace * self.grip.sqrt();
        // Into a braking zone: now and then (more often with someone close
        // behind) a driver leaves it too late and runs wide
        let braking = planned < speed - 10.0;
        if braking && !self.braking {
            let nerves = 1.5 - self.style.composure;
            if self.roll() < (MISTAKE_CHANCE + PRESSURE_MISTAKES * pressure * nerves) * nerves {
                self.mistake = MISTAKE_TIME;
            }
        }
        self.braking = braking;
        if self.mistake > 0.0 {
            self.mistake -= dt;
            planned *= MISTAKE_OVERSPEED;
        }
        let mut target_speed = planned.min(follow_speed.max(20.0));
        if off_line > 40.0 {
            target_speed = target_speed.min(50.0); // recovering after a shove
        }
        if recovering && angle.abs() > 0.8 {
            // Facing away from the road: slow to turn round tightly rather
            // than looping out across the fields
            target_speed = target_speed.min(22.0);
        }
        let error = target_speed - speed;
        let (mut throttle, brake) = if urgent && error < 0.0 {
            (0.0, (-error / 8.0).clamp(0.3, 1.0))
        } else if error > 5.0 {
            (1.0, 0.0)
        } else if error > 0.0 {
            (error / 5.0, 0.0)
        } else if error > -8.0 {
            (0.0, 0.0)
        } else {
            (0.0, ((-error - 8.0) / 20.0).clamp(0.2, 1.0))
        };
        // Ease off while sliding or landing, to get the grip back
        let vel_dir = Vec2::new(car.vel.x, car.vel.z).normalize_or_zero();
        let slide = vel_dir.perp_dot(fwd).abs();
        if slide > 0.15 || car.grounded_wheels() < 4 {
            throttle *= 0.4;
        }
        let input = CarInput { throttle, brake, steer, handbrake: false };
        let calm = !company && !recovering && !urgent && self.mistake <= 0.0 && self.launch == 0.0;
        self.drift_input(track, car, input, planned, calm, dt)
    }

    /// Whether it's sliding through a corner on purpose.
    pub fn drifting(&self) -> bool {
        matches!(self.drift, Drift::Kick(..) | Drift::Hold(_))
    }

    /// The next random number, 0 to 1.
    fn roll(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Through a slow corner, drivers who like to (see `Personality::drift`)
    /// flick the tail out with the handbrake, hold the slide round on the
    /// power and straighten up out of it for the drift boost, as the player
    /// can. Otherwise `grip`, the inputs for taking it on the grip.
    fn drift_input(&mut self, track: &AiTrack, car: &CarPhysics, grip: CarInput, planned: f32, calm: bool, dt: f32) -> CarInput {
        let spacing = track.points[0].distance(track.points[1]).max(0.1);
        let speed = car.speed;
        // How far the line turns over the next stretch, and which way (as a
        // steering sign: + right)
        let dir_at = |i: usize| track.dir(i);
        let here = dir_at(self.progress_idx);
        let ahead = dir_at(self.progress_idx + (DRIFT_REACH / spacing) as usize);
        let turn = -here.perp_dot(ahead).atan2(here.dot(ahead));
        let corner = turn.abs() > DRIFT_TURN && (DRIFT_SPEEDS.0..DRIFT_SPEEDS.1).contains(&speed);
        let pos = Vec2::new(car.pos.x, car.pos.z);
        let off_line = track.points[self.progress_idx].distance(pos);
        match self.drift {
            Drift::Off | Drift::Grip | Drift::Ready if !corner => {
                self.drift = Drift::Off;
                grip
            }
            Drift::Off => {
                self.drift = if self.roll() < self.style.drift { Drift::Ready } else { Drift::Grip };
                grip
            }
            Drift::Grip => grip,
            Drift::Ready => {
                // Turning in, done braking, with a second's worth of corner
                // ahead to slide through and no harder braking in it: flick it
                let second = (speed / spacing) as usize;
                let slowest = (0..second).map(|k| track.speed[track.at(self.progress_idx + k)]).fold(f32::MAX, f32::min) * planned / track.speed[self.progress_idx].max(1.0);
                let exit = dir_at(self.progress_idx + second);
                let bends = here.perp_dot(exit).atan2(here.dot(exit)).abs() > DRIFT_BEND;
                if calm && bends && grip.steer * turn.signum() > 0.4 && speed < planned + 4.0 && slowest > speed - 12.0 {
                    self.drift = Drift::Kick(0.0, turn.signum());
                }
                grip
            }
            Drift::Kick(t, dir) => {
                // Flick until the slide takes; if it won't, drive it on the grip
                self.drift = if car.drifting { Drift::Hold(dir) } else if t > DRIFT_KICK { Drift::Grip } else { Drift::Kick(t + dt, dir) };
                if self.drift == Drift::Grip {
                    return grip;
                }
                CarInput { throttle: 1.0, brake: 0.0, steer: dir * DRIFT_FLICK, handbrake: true }
            }
            Drift::Hold(dir) => {
                // Over, or going wrong: back to driving it on the grip, and
                // braking for anything tighter ahead
                if !car.drifting || off_line > 18.0 || !calm && off_line > 10.0 || speed > planned + 10.0 {
                    self.drift = Drift::Grip;
                    return grip;
                }
                // Hold the slide while the corner lasts, a little wider if
                // it's running wide (a sharper angle turns it in tighter),
                // and unwind it as the corner opens out
                let near = dir_at(self.progress_idx + (DRIFT_REACH * 0.5 / spacing) as usize);
                let left = (-here.perp_dot(near).atan2(here.dot(near)) * dir).max(0.0);
                let inward = Vec2::new(-here.y, here.x) * -dir;
                let wide = -(pos - track.points[self.progress_idx]).dot(inward);
                let trim = (wide * DRIFT_TRIM_PER_UNIT).clamp(-DRIFT_TRIM, DRIFT_TRIM);
                let want = DRIFT_HOLD * ((left - 0.15) / 0.35).clamp(0.0, 1.0) + trim;
                // Swinging past it: catch the slide
                let angle = want - (car.sideslip().abs() - want).max(0.0) * 1.5;
                // Enough throttle to hold the slide; all of it once it's
                // dropping below the corner's speed
                let throttle = if speed > planned { 0.4 } else { 1.0 };
                CarInput { throttle, brake: 0.0, steer: car.drift_steer(angle.max(0.0)), handbrake: false }
            }
        }
    }
}

pub struct AiCar {
    pub physics: CarPhysics,
    pub driver: AiDriver,
    pub color_idx: usize,
    pub track_idx: usize,
    /// Driving inputs are ignored while held (race countdown).
    pub held: bool,
    /// Inputs applied on the last update.
    pub last_input: CarInput,
    /// How long it's been stuck: getting nowhere off the road, in the water
    /// or on its side (see `STUCK_RESET`).
    stuck: f32,
    /// Where it was when it last made real progress.
    anchor: Vec2,
    /// Distance carried over between far-mode steps.
    far_travel: f32,
    /// Drift charge and boost, under the player's rules.
    pub boost: Boost,
}

impl AiCar {
    pub fn new(track: &AiTrack, track_idx: usize, start_idx: usize, color_idx: usize, style: Personality) -> Self {
        let n = track.len();
        let start_idx = start_idx % n;
        let mut car = Self {
            physics: CarPhysics::new(Vec3::ZERO, 0.0),
            driver: AiDriver::new(start_idx, style, (track_idx * 7 + color_idx * 3) as f32),
            color_idx,
            track_idx,
            held: false,
            last_input: CarInput::default(),
            stuck: 0.0,
            anchor: Vec2::ZERO,
            far_travel: 0.0,
            boost: Boost::default(),
        };
        car.place_on(track, start_idx, style.line * 8.0);
        car
    }

    /// Put the car at rest on path point `idx`, `lateral` units to the side.
    pub fn place_on(&mut self, track: &AiTrack, idx: usize, lateral: f32) {
        let (pos, yaw) = track.pose(idx, lateral);
        self.physics.place(&crate::physics::WorldGround, pos.x, pos.y, yaw);
        self.driver.progress_idx = idx % track.len();
    }

    /// Cheap update for cars far from the player: glide along the path at
    /// the planned speed without simulating physics.
    pub fn update_far(&mut self, track: &AiTrack, dt: f32) {
        let spacing = track.points[0].distance(track.points[1]).max(0.1);
        let speed = track.speed[self.driver.progress_idx] * self.driver.pace * 0.9;
        self.far_travel += speed * dt;
        let steps = (self.far_travel / spacing) as usize;
        self.far_travel -= steps as f32 * spacing;
        let prev = self.driver.progress_idx;
        self.driver.progress_idx = track.at(prev + steps);
        if self.driver.progress_idx < prev {
            self.driver.laps += 1;
        }
        let (pos, yaw) = track.pose(self.driver.progress_idx, self.driver.lateral());
        self.physics.place(&crate::physics::WorldGround, pos.x, pos.y, yaw);
        self.physics.vel = self.physics.heading() * speed;
        self.physics.speed = speed;
    }

    pub fn update(&mut self, track: &AiTrack, ground: &dyn Ground, traffic: &[Traffic], dt: f32) {
        let input = self.driver.input(track, &self.physics, traffic, dt);
        let input = if self.held { CarInput { handbrake: true, ..Default::default() } } else { input };
        self.last_input = input;
        self.physics.update(ground, dt, input);
        // Only a drift it meant earns a boost: a slide after a knock is
        // something to gather up, not to be fired out of
        if !self.driver.drifting() {
            self.boost.drift_charge = 0.0;
        }
        self.boost.update(&mut self.physics, dt);

        // Stuck (getting nowhere off the road, however it rocks back and
        // forth; sunk; or on its side): after a few seconds it's put back on
        // its line where it left off, as the player can with a reset
        let p = Vec2::new(self.physics.pos.x, self.physics.pos.z);
        let off_line = track.points[self.driver.progress_idx].distance(p);
        let going_nowhere = p.distance(self.anchor) < STUCK_RADIUS;
        if !going_nowhere {
            self.anchor = p;
        }
        let beached = going_nowhere && off_line > 15.0 && !self.held;
        if beached || self.physics.submerged > 0.5 || self.physics.up().y < 0.3 {
            self.stuck += dt;
        } else {
            self.stuck = 0.0;
        }
        if self.stuck > STUCK_RESET {
            self.place_on(track, self.driver.progress_idx, 0.0);
            self.stuck = 0.0;
            self.anchor = Vec2::new(self.physics.pos.x, self.physics.pos.z);
        }
    }
}

/// How far a gravel stage's line may stray from its centreline.
const STAGE_ROOM: f32 = crate::road::GRAVEL_HALF_WIDTH - 4.5;
/// Gravel gives less grip and braking than tarmac: corner and braking
/// speeds are planned for it.
const GRAVEL_PACE: f32 = 0.92;
/// Over a crest, the share of the car's weight left on its wheels.
const CREST_GRIP: f32 = 0.5;
/// Speed a stage's line eases off to at its very end, past the finish.
const STAGE_END_SPEED: f32 = 15.0;

/// The AI's line along gravel stage road `path`, start to finish: slower
/// than on tarmac, and easing off over crests so it stays planted (the
/// jumps it takes flat out).
pub fn stage_track(path: &crate::road::RoadPath) -> AiTrack {
    let mut track = AiTrack::stage(path.points.clone(), crate::AI_TOP_SPEED, STAGE_ROOM, STAGE_END_SPEED);
    let n = track.len();
    let h = &path.heights;
    let jump = |i: usize| path.ramps.iter().any(|&r| i + 3 >= r && i <= r + crate::road::RAMP_SAMPLES + 4);
    let span = 4;
    for i in span..n.saturating_sub(span) {
        track.speed[i] *= GRAVEL_PACE;
        if jump(i) {
            continue;
        }
        // Over a crest the ground falls away: only so fast before the
        // wheels go light
        let gap = crate::road::SAMPLE_SPACING * span as f32;
        let crest = (2.0 * h[i] - h[i - span] - h[i + span]) / (gap * gap);
        if crest > 0.0 {
            track.speed[i] = track.speed[i].min((crate::physics::GRAVITY * CREST_GRIP / crest).sqrt());
        }
    }
    for i in (0..n - 1).rev() {
        let gap = track.points[i].distance(track.points[i + 1]).max(0.1);
        let reachable = (track.speed[i + 1].powi(2) + 2.0 * PLAN_BRAKE * GRAVEL_PACE * gap).sqrt();
        track.speed[i] = track.speed[i].min(reachable);
    }
    track
}

/// Nearest point in `points[center - back ..= center + ahead]` (wrapping).
/// Returns (index, squared distance).
pub fn nearest_in_window(points: &[Vec2], p: Vec2, center: usize, back: usize, ahead: usize) -> Option<(usize, f32)> {
    let n = points.len();
    if n == 0 {
        return None;
    }
    let count = (back + ahead + 1).min(n);
    (0..count)
        .map(|k| (center + n - back.min(n) + k) % n)
        .map(|i| (i, points[i].distance_squared(p)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Ambient AI drivers on every circuit (except `skip`) and connecting road.
pub fn create_traffic(tracks: &[AiTrack], skip: Option<usize>) -> Vec<AiCar> {
    let mut cars = Vec::new();
    for (ti, track) in tracks.iter().enumerate() {
        if Some(ti) == skip || track.len() < 50 || !track.closed {
            continue;
        }
        let count = if track.len() > 600 { 2 } else { 1 };
        for k in 0..count {
            let start = track.len() * (k * 2 + 1) / (count * 2);
            let pace = 0.75 + ((ti * 5 + k * 3) % 4) as f32 * 0.05;
            let mut car = AiCar::new(track, ti, start, 1 + (ti * 3 + k) % 7, Personality::plain(pace, ((ti + k) % 3) as f32 * 0.3 - 0.3));
            car.physics.vel = car.physics.heading() * 30.0;
            cars.push(car);
        }
    }
    cars
}

/// Snapshot of every car for racecraft decisions this frame.
/// How far each car sits in the slipstream of another just ahead of it
/// (0 to 1): close behind, nearly in line and going the same way, fast.
pub fn drafts(cars: &[&CarPhysics]) -> Vec<f32> {
    const REACH: f32 = 70.0;
    const WIDTH: f32 = 6.0;
    cars.iter().map(|car| {
        let speed = car.vel.length();
        if speed < 40.0 {
            return 0.0;
        }
        let dir = car.vel / speed;
        cars.iter()
            .filter(|other| !std::ptr::eq(**other, *car))
            .filter_map(|other| {
                let rel = other.pos - car.pos;
                let ahead = rel.dot(dir);
                let lateral = (rel - dir * ahead).length();
                let along = other.vel.dot(dir) > speed * 0.7;
                (ahead > 4.0 && ahead < REACH && lateral < WIDTH && along)
                    .then(|| (1.0 - ahead / REACH) * (1.0 - lateral / WIDTH * 0.5))
            })
            .fold(0.0, f32::max)
    }).collect()
}

/// Everyone on the road, as other drivers see them: (car, braking).
pub fn traffic<'a>(cars: impl Iterator<Item = (&'a CarPhysics, bool)>) -> Vec<Traffic> {
    cars.map(|(c, braking)| Traffic { pos: c.pos, vel: c.vel, braking }).collect()
}
