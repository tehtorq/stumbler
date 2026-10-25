//! The race: grid start, countdown, laps, positions, results.

use glam::{Quat, Vec2, Vec3};

use crate::ai_cars::{AiCar, AiDriver, AiTrack};
use crate::physics::{CarPhysics, WorldGround};

pub const RACE_LAPS: u32 = 3;
pub const COUNTDOWN: f32 = 3.0;
/// Path points between grid slots (4 units each): the cars stagger left and
/// right, so each is two of these behind the one in front on its side.
const GRID_SPACING: usize = 5;
/// A lap only counts if the player passed through every sector.
const SECTORS: usize = 8;
/// Further than this from the circuit and the HUD nags you to return.
const OFF_TRACK_WARNING: f32 = 250.0;
/// Rivals well behind the player find a little extra pace, and those well
/// ahead ease off a touch, so the pack stays in contention: the change per
/// unit of track between them, and its limits either way.
const CATCH_UP_PER_UNIT: f32 = 0.00004;
const CATCH_UP_MAX: f32 = 0.05;
const EASE_OFF_MAX: f32 = 0.03;
/// How long a sector split stays on screen.
const SPLIT_SHOW: f32 = 3.0;
/// Rivals react to the start signal after somewhere in this range (seconds).
const REACTION: (f32, f32) = (0.12, 0.45);
/// Each race, each rival's pace varies by up to this share either way, so
/// the same car doesn't always lead.
const FORM: f32 = 0.02;
/// Seconds between recorded frames of the player's lap.
const GHOST_STEP: f32 = 0.05;

/// One moment of a recorded lap.
#[derive(Clone, Copy)]
pub struct GhostFrame {
    /// Seconds into the lap.
    pub t: f32,
    pub pos: Vec3,
    pub rot: Quat,
}

/// Where a recorded lap had the car `t` seconds in, if it lasted that long.
pub fn ghost_pose(frames: &[GhostFrame], t: f32) -> Option<(Vec3, Quat)> {
    let i = frames.partition_point(|f| f.t <= t);
    if i == 0 || i >= frames.len() {
        return None;
    }
    let (a, b) = (frames[i - 1], frames[i]);
    let k = ((t - a.t) / (b.t - a.t).max(1e-4)).clamp(0.0, 1.0);
    Some((a.pos.lerp(b.pos, k), a.rot.slerp(b.rot, k)))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    Countdown { remaining: f32 },
    Racing,
    Finished { position: usize, total_time: f32 },
}

pub enum RaceEvent {
    Go,
    Lap { time: f32 },
    /// Crossed the line without passing every sector.
    LapInvalid,
    Finished,
}

pub struct Race {
    /// Index of the race circuit in the AI track list.
    pub circuit: usize,
    /// Indices of the rival cars in the car list.
    pub rivals: Vec<usize>,
    pub phase: Phase,
    /// Seconds since the start signal.
    pub clock: f32,
    pub best_lap: Option<f32>,
    pub last_lap: Option<f32>,
    pub position: usize,
    /// Completed, valid laps.
    pub laps_done: u32,
    /// Distance from the circuit centreline.
    pub off_track: f32,
    /// Seconds spent driving the wrong way round.
    pub wrong_way: f32,
    progress: AiDriver,
    lap_start: f32,
    sectors: [bool; SECTORS],
    /// The first crossing of the line (from the grid) starts lap 1.
    started_lap: bool,
    /// Rivals' paces as they lined up (with the day's form), before any catch-up.
    base_pace: Vec<f32>,
    /// How long after the start signal each rival gets going.
    reactions: Vec<f32>,
    /// Seeds the rivals' reactions and form; from the clock if not set.
    pub seed: Option<u32>,
    /// Time into the lap at each sector boundary: this lap, and the best.
    splits: [Option<f32>; SECTORS],
    best_splits: Option<[Option<f32>; SECTORS]>,
    /// The latest sector split against the best lap's, and how long ago.
    pub split: Option<(f32, f32)>,
    /// A time trial: alone, lap after lap, against your own ghost.
    pub trial: bool,
    /// The player's lap so far, and the last valid lap once it's done
    /// (for the game to keep as a ghost if it's the best).
    recording: Vec<GhostFrame>,
    pub completed_lap: Option<(f32, Vec<GhostFrame>)>,
}

impl Race {
    pub fn new(circuit: usize, rivals: Vec<usize>) -> Self {
        Self {
            circuit,
            rivals,
            phase: Phase::Countdown { remaining: COUNTDOWN },
            clock: 0.0,
            best_lap: None,
            last_lap: None,
            position: 1,
            laps_done: 0,
            off_track: 0.0,
            wrong_way: 0.0,
            progress: AiDriver::new(0, 1.0, 0.0, 0.0),
            lap_start: 0.0,
            sectors: [false; SECTORS],
            started_lap: false,
            base_pace: Vec::new(),
            reactions: Vec::new(),
            seed: None,
            splits: [None; SECTORS],
            best_splits: None,
            split: None,
            trial: false,
            recording: Vec::new(),
            completed_lap: None,
        }
    }

    /// Note where the player is, for the lap's ghost.
    pub fn record(&mut self, pos: Vec3, rot: Quat) {
        if !self.started_lap || self.phase != Phase::Racing {
            return;
        }
        let t = self.lap_time();
        if self.recording.last().is_none_or(|f| t - f.t >= GHOST_STEP) {
            self.recording.push(GhostFrame { t, pos, rot });
        }
    }

    pub fn racer_count(&self) -> usize {
        self.rivals.len() + 1
    }

    /// Current lap number for display (1-based, capped at the race length).
    pub fn display_lap(&self) -> u32 {
        if self.trial { self.laps_done + 1 } else { (self.laps_done + 1).min(RACE_LAPS) }
    }

    /// A time trial on circuit `circuit`: no rivals, no lap limit.
    pub fn time_trial(circuit: usize) -> Self {
        Self { trial: true, ..Self::new(circuit, Vec::new()) }
    }

    pub fn lap_time(&self) -> f32 {
        if self.started_lap { self.clock - self.lap_start } else { 0.0 }
    }

    /// Whether drivers must wait for the start signal.
    pub fn holding(&self) -> bool {
        matches!(self.phase, Phase::Countdown { .. })
    }

    pub fn is_off_track(&self) -> bool {
        self.off_track > OFF_TRACK_WARNING && self.phase == Phase::Racing
    }

    /// Line everyone up on the grid behind the start line and begin the countdown.
    /// The player starts at the back.
    pub fn start(&mut self, player: &mut CarPhysics, cars: &mut [AiCar], track: &AiTrack) {
        let n = track.len();
        let slot = |k: usize| (n - 3 - k * GRID_SPACING, if k % 2 == 0 { -8.0 } else { 8.0 });

        // Slowest at the front, so the quicker rivals have to come through
        // the field and there's racing ahead of the player, not a procession
        let mut grid = self.rivals.clone();
        grid.sort_by(|&a, &b| cars[a].driver.pace.total_cmp(&cars[b].driver.pace));
        for (k, &ci) in grid.iter().enumerate() {
            let (idx, lateral) = slot(k);
            let car = &mut cars[ci];
            car.place_on(track, idx, lateral);
            car.driver.laps = 0;
            car.held = true;
        }
        let (idx, lateral) = slot(self.rivals.len());
        let (pos, yaw) = track.pose(idx, lateral);
        player.place(&WorldGround, pos.x, pos.y, yaw);

        *self = Self { rivals: std::mem::take(&mut self.rivals), trial: self.trial, seed: self.seed, ..Self::new(self.circuit, Vec::new()) };
        self.progress.progress_idx = idx;
        // Every race a little different: reactions off the line, and form
        let mut seed = self.seed.unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos())
        }) | 1;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        self.base_pace = self.rivals.iter().map(|&ci| cars[ci].driver.pace * (1.0 + FORM * (rand() * 2.0 - 1.0))).collect();
        self.reactions = self.rivals.iter().map(|_| REACTION.0 + rand() * (REACTION.1 - REACTION.0)).collect();
    }

    /// Note which way the player is heading (`velocity` on the ground), to
    /// warn them if it's the wrong way round.
    pub fn check_direction(&mut self, velocity: Vec2, track: &AiTrack, dt: f32) {
        let n = track.len();
        let i = self.progress.progress_idx;
        let along = (track.points[(i + 2) % n] - track.points[i]).normalize_or_zero();
        let backwards = velocity.length() > 10.0 && velocity.normalize_or_zero().dot(along) < -0.5;
        self.wrong_way = if backwards && self.phase == Phase::Racing { self.wrong_way + dt } else { 0.0 };
    }

    /// The circuit point the player has got to.
    pub fn progress_idx(&self) -> usize {
        self.progress.progress_idx
    }

    /// Whether to tell the player they're going the wrong way.
    pub fn is_wrong_way(&self) -> bool {
        self.wrong_way > 1.5
    }

    pub fn update(&mut self, dt: f32, player_pos: Vec2, cars: &mut [AiCar], track: &AiTrack) -> Vec<RaceEvent> {
        let mut events = Vec::new();
        let n = track.len();
        self.update_position(cars, n);

        match self.phase {
            Phase::Countdown { remaining } => {
                let remaining = remaining - dt;
                if remaining <= 0.0 {
                    self.phase = Phase::Racing;
                    self.clock = 0.0;
                    events.push(RaceEvent::Go);
                } else {
                    self.phase = Phase::Countdown { remaining };
                }
                return events;
            }
            Phase::Racing | Phase::Finished { .. } => self.clock += dt,
        }
        // Each rival gets away once they've reacted to the signal
        for (k, &ci) in self.rivals.iter().enumerate() {
            if cars[ci].held && self.clock >= self.reactions.get(k).copied().unwrap_or(0.0) {
                cars[ci].held = false;
            }
        }

        let laps_before = self.progress.laps;
        let sector_before = self.progress.progress_idx * SECTORS / n;
        self.progress.track_progress(track, player_pos);
        self.off_track = track.points[self.progress.progress_idx].distance(player_pos);
        let sector = self.progress.progress_idx * SECTORS / n;
        self.sectors[sector] = true;
        self.pace_rivals(cars, n);
        if let Some((_, age)) = &mut self.split {
            *age += dt;
        }
        // Into the next sector: note the split and how it compares
        if self.started_lap && sector == sector_before + 1 && self.phase == Phase::Racing {
            let at = self.clock - self.lap_start;
            self.splits[sector] = Some(at);
            if let Some(best) = self.best_splits.and_then(|b| b[sector]) {
                self.split = Some((at - best, 0.0));
            }
        }

        if self.progress.laps > laps_before && self.phase == Phase::Racing {
            if !self.started_lap {
                // Left the grid and crossed the line: lap 1 begins
                self.started_lap = true;
                self.lap_start = self.clock;
            } else if self.sectors.iter().all(|&s| s) {
                let time = self.clock - self.lap_start;
                if let Some(best) = self.best_lap {
                    self.split = Some((time - best, 0.0));
                }
                let best = self.best_lap.is_none_or(|b| time < b);
                if best {
                    self.best_lap = Some(time);
                    self.best_splits = Some(self.splits);
                }
                self.last_lap = Some(time);
                self.completed_lap = Some((time, std::mem::take(&mut self.recording)));
                self.laps_done += 1;
                self.lap_start = self.clock;
                events.push(RaceEvent::Lap { time });
            } else {
                self.lap_start = self.clock;
                events.push(RaceEvent::LapInvalid);
            }
            self.sectors = [false; SECTORS];
            self.splits = [None; SECTORS];
            self.recording.clear();
        }

        if self.phase == Phase::Racing && !self.trial && self.laps_done >= RACE_LAPS {
            self.update_position(cars, n);
            self.phase = Phase::Finished { position: self.position, total_time: self.clock };
            events.push(RaceEvent::Finished);
        }
        events
    }

    /// Catch-up and ease-off for the rivals (see `CATCH_UP_PER_UNIT`).
    fn pace_rivals(&mut self, cars: &mut [AiCar], track_len: usize) {
        let me = self.progress.progress(track_len);
        for (k, &ci) in self.rivals.iter().enumerate() {
            let driver = &mut cars[ci].driver;
            let behind = (me - driver.progress(track_len)) * 4.0;
            let adjust = (behind * CATCH_UP_PER_UNIT).clamp(-EASE_OFF_MAX, CATCH_UP_MAX);
            driver.pace = self.base_pace[k] * (1.0 + adjust);
        }
    }

    /// The latest split while it's still on show.
    pub fn recent_split(&self) -> Option<f32> {
        self.split.filter(|&(_, age)| age < SPLIT_SHOW).map(|(delta, _)| delta)
    }

    fn update_position(&mut self, cars: &[AiCar], track_len: usize) {
        if matches!(self.phase, Phase::Finished { .. }) {
            return;
        }
        let me = self.progress.progress(track_len);
        self.position = 1 + self.rivals.iter().filter(|&&ci| cars[ci].driver.progress(track_len) > me).count();
    }

    /// Countdown text, if one is showing: "3", "2", "1", then "GO!" briefly.
    pub fn countdown_text(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Countdown { remaining } if remaining > 2.0 => Some("3"),
            Phase::Countdown { remaining } if remaining > 1.0 => Some("2"),
            Phase::Countdown { .. } => Some("1"),
            Phase::Racing if self.clock < 1.0 => Some("GO!"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghost_replays_between_recorded_frames() {
        let frames: Vec<GhostFrame> = (0..5)
            .map(|k| GhostFrame { t: k as f32 * 0.5, pos: Vec3::new(k as f32 * 10.0, 0.0, 0.0), rot: Quat::IDENTITY })
            .collect();
        assert!(ghost_pose(&frames, -0.1).is_none(), "nothing before the lap starts");
        let (pos, _) = ghost_pose(&frames, 0.75).unwrap();
        assert!((pos.x - 15.0).abs() < 1e-3, "halfway between frames: {pos}");
        assert!(ghost_pose(&frames, 3.0).is_none(), "nothing once the recording runs out");
    }
}
