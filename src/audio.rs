//! Everything you hear, synthesised as it plays: the engine and tyres, wind,
//! rain and the crowd underneath it all, and one-shot sounds (knocks,
//! landings, beeps, fireworks) placed left or right and further or nearer.

use rodio::{MixerDeviceSink, DeviceSinkBuilder, Source};
use std::f32::consts::{FRAC_PI_4, TAU};
use std::num::NonZero;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Other cars you can hear at once, nearest first.
pub const OTHER_CARS: usize = 2;
/// One-shot sounds playing at once; any more are dropped.
const MAX_VOICES: usize = 24;
/// Samples between looks at the queue of sounds asked for.
const QUEUE_EVERY: u32 = 256;

/// The one-shot sounds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    /// A car into a post, wall or barrier: a metallic clang.
    Clang,
    /// Car into car: crumpling panels over a thump.
    Crunch,
    /// Into a tree: a hollow knock, then the leaves.
    Thunk,
    /// Into the water.
    Splash,
    /// Touching down after a jump: the springs bottoming out.
    Landing,
    /// A start light coming on.
    Beep,
    /// Lights out: go.
    GoBeep,
    /// A firework bursting.
    Boom,
    /// Over a boost pad.
    Whoosh,
    /// Picking up an orb.
    Pickup,
    /// A lap done, a place found.
    Chime,
    /// Taking the flag.
    Fanfare,
    /// An exhaust backfire.
    Pop,
    /// The steam train's whistle.
    Whistle,
    /// Rushing under a gantry: the air thrown back off it.
    Swish,
}

impl Sound {
    /// How long it lasts (seconds).
    fn length(self) -> f32 {
        match self {
            Sound::Clang => 0.7,
            Sound::Crunch => 0.55,
            Sound::Thunk => 0.9,
            Sound::Splash => 1.3,
            Sound::Landing => 0.4,
            Sound::Beep => 0.28,
            Sound::GoBeep => 0.8,
            Sound::Boom => 2.2,
            Sound::Whoosh => 0.8,
            Sound::Pickup => 0.45,
            Sound::Chime => 0.6,
            Sound::Fanfare => 2.2,
            Sound::Pop => 0.07,
            Sound::Whistle => 2.0,
            Sound::Swish => 0.45,
        }
    }
}

/// A one-shot sound playing: what, how loud, where (-1 left .. 1 right),
/// how long before it's heard, and its own filter and noise state.
struct Voice {
    sound: Sound,
    gain: f32,
    place: Place,
    ear: Spatial,
    delay: f32,
    t: f32,
    seed: u32,
    lp: [f32; 2],
}

impl Voice {
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    /// The next sample, or None once it's over.
    fn next(&mut self, dt: f32) -> Option<f32> {
        if self.delay > 0.0 {
            self.delay -= dt;
            return Some(0.0);
        }
        let t = self.t;
        self.t += dt;
        if t >= self.sound.length() {
            return None;
        }
        let sine = |f: f32| (t * f * TAU).sin();
        // A random detune per voice, so no two knocks are quite alike
        let detune = 1.0 + ((self.seed & 0xff) as f32 / 255.0 - 0.5) * 0.12;
        let v = match self.sound {
            Sound::Clang => {
                // Inharmonic partials ring like struck metal; harder hits
                // are brighter
                let bright = self.gain.min(1.0);
                let ring = sine(180.0 * detune) * 0.5 + sine(267.0 * detune) * 0.35 + sine(412.0 * detune) * 0.25 * bright + sine(731.0 * detune) * 0.12 * bright;
                let body = sine(55.0) * (-t * 22.0).exp();
                let click = self.noise() * (-t * 90.0).exp();
                (ring * (-t * 7.0).exp() * 0.5 + body * 0.6 + click * 0.5) * 0.5
            }
            Sound::Crunch => {
                // Crackling panels: noise, band-limited, chopped into grains
                let n = self.noise();
                self.lp[0] += 0.4 * (n - self.lp[0]);
                self.lp[1] += 0.06 * (self.lp[0] - self.lp[1]);
                let band = self.lp[0] - self.lp[1];
                let grain = if ((t * 180.0) as u32).wrapping_mul(2654435761) >> 30 == 0 { 0.4 } else { 1.0 };
                let thump = sine(48.0) * (-t * 16.0).exp();
                let ring = (sine(330.0 * detune) + sine(517.0 * detune) * 0.6) * (-t * 14.0).exp();
                (band * 2.2 * grain * (-t * 9.0).exp() + thump * 0.6 + ring * 0.15) * 0.28
            }
            Sound::Thunk => {
                // A hollow trunk knocked, then leaves shaken loose
                let knock = sine(92.0 * detune) * (-t * 13.0).exp() + sine(185.0 * detune) * 0.4 * (-t * 22.0).exp();
                let click = self.noise() * (-t * 120.0).exp();
                let n = self.noise();
                self.lp[0] += 0.5 * (n - self.lp[0]);
                let rustle = (n - self.lp[0]) * (t * 10.0).min(1.0) * (-t * 3.5).exp();
                (knock * 0.7 + click * 0.35 + rustle * 0.35) * 0.5
            }
            Sound::Splash => {
                // A whump, then spray falling back: noise whose brightness
                // falls away
                let n = self.noise();
                let open = 0.5 * (-t * 2.2).exp() + 0.03;
                self.lp[0] += open * (n - self.lp[0]);
                let env = (t * 80.0).min(1.0) * (-t * 2.6).exp();
                let whump = sine(42.0) * (-t * 9.0).exp();
                (self.lp[0] * 1.6 * env + whump * 0.6) * 0.32
            }
            Sound::Landing => {
                let thud = sine(46.0) * (-t * 13.0).exp() + sine(94.0) * 0.5 * (-t * 24.0).exp();
                let n = self.noise();
                self.lp[0] += 0.15 * (n - self.lp[0]);
                let clunk = self.lp[0] * (-t * 35.0).exp();
                (thud * 0.7 + clunk * 1.2) * 0.5
            }
            Sound::Beep | Sound::GoBeep => {
                // A clean electronic tone, square-edged, with soft ends
                let (f, len) = if self.sound == Sound::Beep { (660.0, 0.28) } else { (1320.0, 0.8) };
                let tone = sine(f) + sine(f * 3.0) * 0.18 + sine(f * 5.0) * 0.06;
                let ends = (t / 0.005).min(1.0) * ((len - t) / 0.03).clamp(0.0, 1.0);
                tone * ends * 0.1
            }
            Sound::Boom => {
                // The burst: a deep thud and a rush, then a fizz of crackles
                let thud = (t * (38.0 - t * 8.0) * TAU).sin() * (-t * 4.0).exp();
                let n = self.noise();
                self.lp[0] += 0.08 * (n - self.lp[0]);
                let rush = self.lp[0] * 2.0 * (-t * 3.0).exp();
                let crackle = if t > 0.25 && self.noise() > 0.985 - 0.01 * (-t).exp() { self.noise() * (-(t - 0.25) * 1.5).exp() } else { 0.0 };
                (thud * 0.6 + rush * 0.5 + crackle * 0.6) * 0.5
            }
            Sound::Whoosh => {
                // A rush of air that rises and falls, a note lifting under it
                let len = 0.8;
                let swell = (std::f32::consts::PI * t / len).sin();
                let n = self.noise();
                self.lp[0] += (0.03 + 0.4 * swell) * (n - self.lp[0]);
                let lift = (t * (300.0 + 700.0 * t / len) * TAU).sin() * swell;
                (self.lp[0] * 1.4 * swell + lift * 0.12) * 0.28
            }
            Sound::Pickup => {
                // A rising sparkle of three bell notes
                let mut sum = 0.0;
                for (k, f) in [1047.0, 1319.0, 1568.0, 2093.0].into_iter().enumerate() {
                    let start = k as f32 * 0.06;
                    if t >= start {
                        let u = t - start;
                        sum += ((u * f * TAU).sin() + (u * f * 2.0 * TAU).sin() * 0.25) * (-u * 9.0).exp();
                    }
                }
                sum * 0.09
            }
            Sound::Chime => {
                let note = if t < 0.15 { 523.0 } else if t < 0.3 { 659.0 } else { 784.0 };
                let env = ((0.6 - t) / 0.6).powi(2);
                (sine(note) + sine(note * 2.0) * 0.2) * 0.14 * env
            }
            Sound::Fanfare => {
                // Up the chord, then all of it held, brassy and bright
                let mut sum = 0.0;
                let vibrato = 1.0 + 0.004 * (t * 5.5 * TAU).sin();
                for (k, f) in [523.25, 659.25, 783.99, 1046.5].into_iter().enumerate() {
                    let start = k as f32 * 0.14;
                    if t >= start {
                        let u = t - start;
                        let f = f * vibrato;
                        let brass = (u * f * TAU).sin() + (u * f * 2.0 * TAU).sin() * 0.35 + (u * f * 3.0 * TAU).sin() * 0.18;
                        let env = (u / 0.02).min(1.0) * ((2.2 - t) / 1.0).clamp(0.0, 1.0);
                        sum += brass * env;
                    }
                }
                sum * 0.055
            }
            Sound::Pop => {
                let left = POP_LENGTH - t;
                let env = (left / POP_LENGTH).powi(3);
                let crack = self.noise() * (1.0 - t / 0.015).max(0.0);
                (crack * 0.35 + sine(70.0) * 0.3) * env
            }
            Sound::Whistle => {
                // A chord blown through a pipe: breathy at the start, the
                // pitch sagging a touch as the steam runs out
                let sag = 1.0 - 0.03 * (t / 2.0).powi(2);
                let tone = (t * 523.0 * sag * TAU).sin() * 0.5 + (t * 659.0 * sag * TAU).sin() * 0.4 + (t * 784.0 * sag * TAU).sin() * 0.3;
                let n = self.noise();
                self.lp[0] += 0.3 * (n - self.lp[0]);
                let env = (t / 0.08).min(1.0) * ((2.0 - t) / 0.4).clamp(0.0, 1.0);
                (tone + self.lp[0] * (0.25 + 0.5 * (-t * 6.0).exp())) * env * 0.11
            }
            Sound::Swish => {
                // A rush of air, bright as you go under and falling away,
                // with a hollow thud from the structure overhead
                let n = self.noise();
                let open = 0.6 * (-t * 7.0).exp() + 0.04;
                self.lp[0] += open * (n - self.lp[0]);
                self.lp[1] += 0.02 * (self.lp[0] - self.lp[1]);
                let env = (t / 0.03).min(1.0) * (-t * 7.0).exp();
                let hollow = sine(110.0) * (-t * 18.0).exp();
                ((self.lp[0] - self.lp[1]) * 2.2 * env + hollow * 0.25) * 0.4
            }
        };
        Some(v * self.gain)
    }
}

/// What the game tells the sound: levels it sets each frame, and sounds it
/// asks for, read from the audio thread.
pub struct AudioParams {
    speed: AtomicU32,
    sliding: AtomicU32,
    throttle: AtomicU32,
    /// Whether the tyres are on tarmac (squeal) or not (gravel), 0 or 1.
    paved: AtomicU32,
    /// How many wheels are on the ground (0..1).
    grounded: AtomicU32,
    /// Kerb and rough-ground judder, 0..1.
    rumble: AtomicU32,
    /// The nearest other cars: speed, how loud each is here (0..1), its
    /// pitch shift from closing in or pulling away, and where (pan, behind).
    other_speed: [AtomicU32; OTHER_CARS],
    other_level: [AtomicU32; OTHER_CARS],
    other_pitch: [AtomicU32; OTHER_CARS],
    other_place: [[AtomicU32; 2]; OTHER_CARS],
    /// How hard it's raining, and how wet the road is.
    rain: AtomicU32,
    wet: AtomicU32,
    /// How loud the crowd around you is, 0..1.
    crowd: AtomicU32,
    /// The underside grinding on the ground, 0..1.
    scrape: AtomicU32,
    /// The countryside around you: how quiet it is (0 by a road or crowd,
    /// 1 out in the wilds) and how far into the day (0 night .. 1 day).
    quiet: AtomicU32,
    daylight: AtomicU32,
    /// The steam train: how loud its engine is here, chuffs a second, and
    /// where it is (pan, behind); and the nearest crossing's bell
    /// (loudness, pan, behind).
    train: [AtomicU32; 4],
    bell: [AtomicU32; 3],
    /// The space around you: how much it rings (0 dry .. 1), how long
    /// (0 short .. 1 long), and a slap-back echo off the nearest hillside
    /// (loudness, seconds).
    room: [AtomicU32; 4],
    /// One-shot sounds waiting to start.
    queue: Mutex<Vec<Voice>>,
    seed: AtomicU32,
}

fn level(v: f32) -> AtomicU32 {
    AtomicU32::new(v.to_bits())
}

impl AudioParams {
    fn new() -> Self {
        Self {
            speed: level(0.0),
            sliding: level(0.0),
            throttle: level(0.0),
            paved: level(1.0),
            grounded: level(1.0),
            rumble: level(0.0),
            other_speed: std::array::from_fn(|_| level(0.0)),
            other_level: std::array::from_fn(|_| level(0.0)),
            other_pitch: std::array::from_fn(|_| level(1.0)),
            other_place: std::array::from_fn(|_| [level(0.0), level(0.0)]),
            rain: level(0.0),
            wet: level(0.0),
            crowd: level(0.0),
            scrape: level(0.0),
            quiet: level(0.0),
            daylight: level(1.0),
            train: std::array::from_fn(|_| level(0.0)),
            bell: std::array::from_fn(|_| level(0.0)),
            room: std::array::from_fn(|_| level(0.0)),
            queue: Mutex::new(Vec::new()),
            seed: AtomicU32::new(0x2545_f491),
        }
    }

    fn set(v: &AtomicU32, x: f32) {
        v.store(x.to_bits(), Ordering::Relaxed);
    }

    fn get(&self, v: &AtomicU32) -> f32 {
        f32::from_bits(v.load(Ordering::Relaxed))
    }

    /// Play `sound` at `gain` (1 is full), from `place`, after `delay`
    /// seconds.
    pub fn play(&self, sound: Sound, gain: f32, place: Place, delay: f32) {
        if gain < 0.005 {
            return;
        }
        let seed = self.seed.fetch_add(0x9e37_79b9, Ordering::Relaxed) | 1;
        if let Ok(mut queue) = self.queue.lock() {
            if queue.len() < MAX_VOICES {
                queue.push(Voice { sound, gain: gain.min(1.5), place: place.clamped(), ear: Spatial::default(), delay: delay.max(0.0), t: 0.0, seed, lp: [0.0; 2] });
            }
        }
    }

    pub fn trigger_chime(&self) {
        self.play(Sound::Chime, 1.0, Place::CENTRE, 0.0);
    }

    pub fn trigger_pickup(&self) {
        self.play(Sound::Pickup, 1.0, Place::CENTRE, 0.0);
    }

    pub fn trigger_pop(&self) {
        self.play(Sound::Pop, 1.0, Place::CENTRE, 0.0);
    }

    pub fn set_crowd(&self, level: f32) {
        Self::set(&self.crowd, level.clamp(0.0, 1.0));
    }

    pub fn set_scrape(&self, level: f32) {
        Self::set(&self.scrape, level.clamp(0.0, 1.0));
    }

    pub fn set_rain(&self, rain: f32, wet: f32) {
        Self::set(&self.rain, rain);
        Self::set(&self.wet, wet);
    }

    /// The countryside: `quiet` (0 by roads and crowds .. 1 far from
    /// anything) and `daylight` (0 night .. 1 day).
    pub fn set_ambience(&self, quiet: f32, daylight: f32) {
        Self::set(&self.quiet, quiet.clamp(0.0, 1.0));
        Self::set(&self.daylight, daylight.clamp(0.0, 1.0));
    }

    /// Other car `slot` (0 the nearest): its `speed`, `level` (0 silent ..
    /// 1 right there), `pitch` (1 = as is; above while it closes in, below
    /// as it goes) and where it is.
    pub fn set_other(&self, slot: usize, speed: f32, level: f32, pitch: f32, place: Place) {
        Self::set(&self.other_speed[slot], speed);
        Self::set(&self.other_level[slot], level);
        Self::set(&self.other_pitch[slot], pitch);
        Self::set_place(&self.other_place[slot], place);
    }

    /// The train's engine: `level` here, `rate` chuffs a second, and where.
    pub fn set_train(&self, level: f32, rate: f32, place: Place) {
        Self::set(&self.train[0], level.clamp(0.0, 1.0));
        Self::set(&self.train[1], rate);
        Self::set_place(&self.train[2..4], place);
    }

    /// A crossing bell ringing: `level` here, and where.
    pub fn set_bell(&self, level: f32, place: Place) {
        Self::set(&self.bell[0], level.clamp(0.0, 1.0));
        Self::set_place(&self.bell[1..3], place);
    }

    fn set_place(v: &[AtomicU32], place: Place) {
        let place = place.clamped();
        Self::set(&v[0], place.pan);
        Self::set(&v[1], place.behind);
    }

    fn get_place(&self, v: &[AtomicU32]) -> Place {
        Place { pan: self.get(&v[0]), behind: self.get(&v[1]) }
    }

    /// The space around you: `wet` (0 dry .. 1 ringing), `decay` (0 short
    /// .. 1 long), and an `echo` of that loudness `echo_delay` seconds later.
    pub fn set_room(&self, wet: f32, decay: f32, echo: f32, echo_delay: f32) {
        Self::set(&self.room[0], wet.clamp(0.0, 1.0));
        Self::set(&self.room[1], decay.clamp(0.0, 1.0));
        Self::set(&self.room[2], echo.clamp(0.0, 1.0));
        Self::set(&self.room[3], echo_delay.clamp(0.0, ECHO_LONGEST));
    }

    pub fn set_throttle(&self, v: f32) {
        Self::set(&self.throttle, v);
    }

    pub fn set_paved(&self, paved: bool) {
        Self::set(&self.paved, if paved { 1.0 } else { 0.0 });
    }

    pub fn set_grounded(&self, share: f32) {
        Self::set(&self.grounded, share);
    }

    pub fn set_rumble(&self, v: f32) {
        Self::set(&self.rumble, v);
    }

    pub fn set_speed(&self, v: f32) {
        Self::set(&self.speed, v);
    }

    pub fn set_sliding(&self, v: f32) {
        Self::set(&self.sliding, v);
    }
}

/// How long a backfire pop rings (seconds).
const POP_LENGTH: f32 = 0.07;
/// Top speed (units/s) in each gear.
const GEAR_TOPS: [f32; 6] = [38.0, 64.0, 90.0, 115.0, 140.0, 175.0];
/// How long the power is cut through a gear change (seconds).
const SHIFT_CUT: f32 = 0.09;

/// Engine revs (0..1) at `speed`: each gear runs from a little above idle
/// up to the red line, then the next one picks up lower down.
fn engine_revs(speed: f32) -> f32 {
    let gear = GEAR_TOPS.iter().position(|&top| speed < top).unwrap_or(GEAR_TOPS.len() - 1);
    let low = if gear == 0 { 0.0 } else { GEAR_TOPS[gear - 1] * 0.75 };
    let top = GEAR_TOPS[gear];
    0.2 + 0.8 * ((speed - low) / (top - low)).clamp(0.0, 1.0)
}

/// Equal-power gains (left, right) for `pan` (-1 .. 1).
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan + 1.0) * FRAC_PI_4;
    (a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2)
}

/// Where a sound is from the listener: `pan` -1 hard left .. 1 hard right,
/// and `behind` 0 ahead or to the side .. 1 right behind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
    pub pan: f32,
    pub behind: f32,
}

impl Place {
    pub const CENTRE: Place = Place { pan: 0.0, behind: 0.0 };

    fn clamped(self) -> Place {
        Place { pan: self.pan.clamp(-1.0, 1.0), behind: self.behind.clamp(0.0, 1.0) }
    }
}

/// The far ear hears a sound from hard to one side this much later
/// (seconds), about the width of a head; and the samples kept for it.
const EAR_LAG: f32 = 0.00065;
const EAR_RING: usize = 64;
/// How far the far ear's level drops for a sound hard to one side: panned
/// only this much of the way, so it still has something to hear late.
const EAR_SHADOW: f32 = 0.75;

/// Puts a mono sound where it is: louder in the nearer ear and reaching the
/// far one a fraction of a millisecond later, which is most of how we tell
/// left from right; and duller and a little quieter from behind, as the
/// ears' shape shades the highs off sounds that come from there.
#[derive(Clone)]
struct Spatial {
    lp: f32,
    ring: [f32; EAR_RING],
    at: usize,
}

impl Default for Spatial {
    fn default() -> Self {
        Self { lp: 0.0, ring: [0.0; EAR_RING], at: 0 }
    }
}

impl Spatial {
    fn place(&mut self, s: f32, place: Place, rate: f32) -> (f32, f32) {
        let open = 1.0 - 0.8 * place.behind;
        self.lp += open * (s - self.lp);
        let s = self.lp * (1.0 - 0.2 * place.behind);
        self.ring[self.at] = s;
        // Read the far ear's sample from a moment ago, between samples so a
        // car moving across doesn't click
        let lag = (place.pan.abs() * EAR_LAG * rate).min((EAR_RING - 2) as f32);
        let (whole, part) = (lag as usize, lag.fract());
        let back = |k: usize| self.ring[(self.at + EAR_RING - k) % EAR_RING];
        let late = back(whole) * (1.0 - part) + back(whole + 1) * part;
        self.at = (self.at + 1) % EAR_RING;
        let (gl, gr) = pan_gains(place.pan * EAR_SHADOW);
        if place.pan >= 0.0 { (late * gl, s * gr) } else { (s * gl, late * gr) }
    }
}

/// Comb delays for the reverb (samples at 44.1kHz), the right channel's a
/// little longer than the left's so the two ears hear different tails; and
/// the all-pass stages that smear it into a wash.
const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASSES: [usize; 2] = [556, 441];
const STEREO_SPREAD: usize = 23;
/// The longest echo off a hillside (seconds).
pub const ECHO_LONGEST: f32 = 0.6;

/// A feedback delay that rings, losing its highs a little each time round.
struct Comb {
    buf: Vec<f32>,
    at: usize,
    damp: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], at: 0, damp: 0.0 }
    }

    fn run(&mut self, x: f32, feedback: f32, damping: f32) -> f32 {
        let out = self.buf[self.at];
        self.damp = out * (1.0 - damping) + self.damp * damping;
        self.buf[self.at] = x + self.damp * feedback;
        self.at = (self.at + 1) % self.buf.len();
        out
    }
}

/// Spreads a sound out in time without colouring it.
struct Allpass {
    buf: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], at: 0 }
    }

    fn run(&mut self, x: f32) -> f32 {
        let held = self.buf[self.at];
        self.buf[self.at] = x + held * 0.5;
        self.at = (self.at + 1) % self.buf.len();
        held - x
    }
}

/// The space you're in: a reverb (parallel combs into all-passes, one set
/// per ear) for stands and valleys and woods, and a single echo off the
/// nearest hillside. Its settings ease toward what's set so a change of
/// place swells in rather than jumps.
struct Room {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    echo: Vec<f32>,
    echo_at: usize,
    wet: f32,
    decay: f32,
    echo_gain: f32,
    echo_delay: f32,
}

impl Room {
    fn new(rate: u32) -> Self {
        let scale = |n: usize| (n as f32 * rate as f32 / 44100.0) as usize;
        let ear = |spread: usize| COMBS.iter().map(|&n| Comb::new(scale(n + spread))).collect();
        let pass = |spread: usize| ALLPASSES.iter().map(|&n| Allpass::new(scale(n + spread))).collect();
        Self {
            combs: [ear(0), ear(STEREO_SPREAD)],
            allpasses: [pass(0), pass(STEREO_SPREAD)],
            echo: vec![0.0; (ECHO_LONGEST * rate as f32) as usize + 2],
            echo_at: 0,
            wet: 0.0,
            decay: 0.0,
            echo_gain: 0.0,
            echo_delay: 0.1,
        }
    }

    /// What the space sends back of `x` (mono) this sample, per ear.
    fn run(&mut self, x: f32, p: &AudioParams, rate: f32) -> (f32, f32) {
        let ease = 1.0 / (rate * 0.4);
        self.wet += (p.get(&p.room[0]) - self.wet) * ease;
        self.decay += (p.get(&p.room[1]) - self.decay) * ease;
        self.echo_gain += (p.get(&p.room[2]) - self.echo_gain) * ease;
        // The echo's distance changes slowly, so it slides rather than warbles
        self.echo_delay += (p.get(&p.room[3]) - self.echo_delay) * ease * 0.25;

        // The echo: read between samples at the delay, a little dulled
        let len = self.echo.len();
        self.echo[self.echo_at] = x;
        let lag = (self.echo_delay * rate).clamp(1.0, (len - 2) as f32);
        let (whole, part) = (lag as usize, lag.fract());
        let back = |k: usize| self.echo[(self.echo_at + len - k) % len];
        let echo = (back(whole) * (1.0 - part) + back(whole + 1) * part) * self.echo_gain;
        self.echo_at = (self.echo_at + 1) % len;

        if self.wet < 0.002 {
            return (echo, echo);
        }
        let feedback = 0.7 + 0.27 * self.decay;
        let damping = 0.45 - 0.25 * self.decay;
        let input = x * 0.12;
        let mut out = [0.0; 2];
        for (ear, o) in out.iter_mut().enumerate() {
            let mut sum = self.combs[ear].iter_mut().map(|c| c.run(input, feedback, damping)).sum::<f32>();
            for a in &mut self.allpasses[ear] {
                sum = a.run(sum);
            }
            *o = sum * self.wet;
        }
        (out[0] + echo, out[1] + echo)
    }
}

/// One of the other cars you can hear.
#[derive(Default)]
struct OtherVoice {
    rpm: f32,
    phase: f32,
    lp: f32,
    /// Where it's heard from, eased toward where it is so it glides.
    place: Place,
    ear: Spatial,
}

struct EngineSource {
    params: Arc<AudioParams>,
    sample_rate: u32,
    // Engine oscillator phases
    phase_fire: f32,
    phase_exhaust: f32,
    phase_sub: f32,
    // Low-pass filter state
    lp_state: f32,
    // Gravel filter states (one per "wheel")
    gravel_lp: [f32; 4],
    /// Engine revs (0..1), eased toward what the speed and gear give, the
    /// revs a moment ago (a sudden drop is a change up), and how long the
    /// power stays cut for the change.
    rpm: f32,
    last_target: f32,
    shift: f32,
    /// Bouncing off the rev limiter.
    limiter_phase: f32,
    squeal_phase: f32,
    squeal_lp: f32,
    wind_lp: f32,
    rumble_phase: f32,
    others: [OtherVoice; OTHER_CARS],
    rain_lp: f32,
    swish_lp: f32,
    /// Tyres rolling: a road roar on tarmac, grit crackling off it.
    roll_lp: [f32; 2],
    grit_lp: f32,
    /// The crowd: a band of voices, their level easing toward what's set,
    /// and a slow swell through it.
    crowd_lp: [f32; 2],
    crowd_level: f32,
    crowd_time: f32,
    scrape_lp: [f32; 2],
    scrape_level: f32,
    /// Birdsong by day, crickets by night: when the next call comes, and
    /// the one sounding now (time into it, pitch).
    ambient_wait: f32,
    call: f32,
    call_pitch: f32,
    call_phase: f32,
    cricket_phase: f32,
    cricket_time: f32,
    /// The train's chuff (how far through the current one, filter state)
    /// and the crossing bell's strokes.
    chuff: f32,
    chuff_lp: [f32; 2],
    bell_time: f32,
    train_ear: Spatial,
    bell_ear: Spatial,
    room: Room,
    voices: Vec<Voice>,
    queue_count: u32,
    /// The right channel of the frame whose left was just handed out.
    right: Option<f32>,
    // Noise sources (offset for variety)
    noise_states: [u32; 6],
}

impl EngineSource {
    fn new(params: Arc<AudioParams>, sample_rate: u32) -> Self {
        Self {
            params, sample_rate,
            phase_fire: 0.0, phase_exhaust: 0.0, phase_sub: 0.0,
            lp_state: 0.0,
            gravel_lp: [0.0; 4],
            rpm: 0.2,
            last_target: 0.2,
            shift: 0.0,
            limiter_phase: 0.0,
            squeal_phase: 0.0,
            squeal_lp: 0.0,
            wind_lp: 0.0,
            rumble_phase: 0.0,
            others: Default::default(),
            rain_lp: 0.0,
            swish_lp: 0.0,
            roll_lp: [0.0; 2],
            grit_lp: 0.0,
            crowd_lp: [0.0; 2],
            crowd_level: 0.0,
            crowd_time: 0.0,
            scrape_lp: [0.0; 2],
            scrape_level: 0.0,
            ambient_wait: 2.0,
            call: -1.0,
            call_pitch: 1.0,
            call_phase: 0.0,
            cricket_phase: 0.0,
            cricket_time: 0.0,
            chuff: 0.0,
            chuff_lp: [0.0; 2],
            bell_time: 0.0,
            train_ear: Spatial::default(),
            bell_ear: Spatial::default(),
            room: Room::new(sample_rate),
            voices: Vec::with_capacity(MAX_VOICES),
            queue_count: 0,
            right: None,
            noise_states: [12345, 67890, 24680, 13579, 97531, 11111],
        }
    }

    fn noise_n(&mut self, idx: usize) -> f32 {
        self.noise_states[idx] = self.noise_states[idx].wrapping_mul(1103515245).wrapping_add(12345);
        (self.noise_states[idx] >> 16) as f32 / 32768.0 - 1.0
    }

    fn noise(&mut self) -> f32 {
        self.noise_n(0)
    }

    /// 0..1, for scheduling.
    fn chance(&mut self) -> f32 {
        self.noise_n(5) * 0.5 + 0.5
    }

    /// One stereo frame.
    fn frame(&mut self) -> (f32, f32) {
        let dt = 1.0 / self.sample_rate as f32;
        let p = Arc::clone(&self.params);
        let speed = p.get(&p.speed);
        let sliding = p.get(&p.sliding);
        let throttle = p.get(&p.throttle);
        let paved = p.get(&p.paved) > 0.5;
        let grounded = p.get(&p.grounded);
        let rumble = p.get(&p.rumble);

        // ── Engine ──
        // Revs climb through the gears and drop back at each change up;
        // eased so a shift is quick but not a click
        let target_rpm = engine_revs(speed.abs());
        if target_rpm < self.last_target - 0.2 && throttle > 0.3 {
            self.shift = SHIFT_CUT;
        }
        self.last_target = target_rpm;
        self.rpm += (target_rpm - self.rpm) * (dt / 0.06).min(1.0);
        let rpm_t = self.rpm;
        // Firing frequency: 4-cylinder, 4-stroke = 2 fires per rev
        // At idle: 800rpm → 800/60*2 = 26.7Hz
        // At max:  7000rpm → 7000/60*2 = 233Hz
        let fire_freq = 26.0 + rpm_t * 207.0;

        // Power cut for a moment through each change up, and chopped on
        // and off against the limiter flat out in top
        let mut power: f32 = 1.0;
        if self.shift > 0.0 {
            self.shift -= dt;
            power = 0.35;
        }
        if target_rpm > 0.995 && throttle > 0.5 {
            self.limiter_phase = (self.limiter_phase + 14.0 * dt).fract();
            if self.limiter_phase > 0.6 {
                power = power.min(0.45);
            }
        }
        let load = throttle * power;

        // Volume: louder with RPM, and on the power
        let vol = (0.05 + rpm_t * 0.10) * (0.6 + 0.4 * load);

        // 1. Firing pulse — square-ish wave with duty cycle ~30%
        //    This gives the rhythmic "throb" of each cylinder firing
        self.phase_fire += fire_freq * dt;
        if self.phase_fire > 1.0 { self.phase_fire -= 1.0; }
        let pulse = if self.phase_fire < 0.3 { 1.0 } else { -0.3 };

        // 2. Exhaust resonance — sine at 1.5x firing freq
        //    The exhaust pipe resonates, giving the "note" character
        self.phase_exhaust += fire_freq * 1.5 * dt;
        if self.phase_exhaust > 1.0 { self.phase_exhaust -= 1.0; }
        let exhaust = (self.phase_exhaust * TAU).sin() * 0.6;

        // 3. Sub-bass rumble — half the firing frequency
        //    Deep throb you feel in your chest
        self.phase_sub += fire_freq * 0.5 * dt;
        if self.phase_sub > 1.0 { self.phase_sub -= 1.0; }
        let sub = (self.phase_sub * TAU).sin() * 0.4;

        // 4. Mechanical noise — filtered random noise, less at high RPM
        let mech = self.noise() * 0.15 * (1.0 - rpm_t * 0.6);

        // 5. Combine and apply amplitude modulation (throb envelope)
        let throb_env = 0.7 + 0.3 * (self.phase_fire * TAU).sin();
        let raw_engine = (pulse + exhaust + sub + mech) * vol * throb_env;

        // 6. Simple one-pole low-pass filter to warm it up
        //    Cutoff rises with RPM, and opens further on the power: muffled
        //    at idle and on the overrun, a harder edge under load
        let lp_alpha = (0.02 + rpm_t * 0.15 + load * 0.06).min(0.99);
        self.lp_state += lp_alpha * (raw_engine - self.lp_state);
        let engine = self.lp_state;

        // ── Tyre squeal on tarmac: a wavering tone with a scratchy edge ──
        let squeal = if paved && sliding > 6.0 {
            let intensity = ((sliding - 6.0) / 25.0).min(1.0);
            let wobble = self.noise_n(4) * 60.0;
            self.squeal_phase += (980.0 + intensity * 180.0 + wobble) * dt;
            if self.squeal_phase > 1.0 { self.squeal_phase -= 1.0; }
            let tone = (self.squeal_phase * TAU).sin();
            let n = self.noise_n(3);
            self.squeal_lp += 0.35 * (n - self.squeal_lp);
            (tone * 0.7 + (n - self.squeal_lp) * 0.3) * 0.045 * intensity
        } else {
            0.0
        };

        // ── Tire gravel — 4 independent low-passed noise sources ──
        //    Sounds like loose surface rumble, not a screech
        let gravel = if !paved && sliding > 8.0 {
            let intensity = ((sliding - 8.0) / 50.0).min(1.0);
            let mut total = 0.0;
            for i in 0..4 {
                let raw = self.noise_n(i + 1);
                // Low-pass each "wheel" independently — gives texture
                let alpha = 0.08 + (i as f32) * 0.02; // slightly different cutoff per wheel
                self.gravel_lp[i] += alpha * (raw - self.gravel_lp[i]);
                total += self.gravel_lp[i];
            }
            total * 0.05 * intensity
        } else { 0.0 };

        // ── Tyres rolling: a low road roar on tarmac rising with speed;
        //    off it, grit and stones crackling under the tread ──
        let roll_speed = (speed.abs() / 120.0).min(1.3) * grounded;
        let roll = if roll_speed > 0.01 {
            if paved {
                let n = self.noise_n(1);
                self.roll_lp[0] += (0.02 + 0.03 * roll_speed) * (n - self.roll_lp[0]);
                self.roll_lp[1] += 0.004 * (self.roll_lp[0] - self.roll_lp[1]);
                (self.roll_lp[0] - self.roll_lp[1]) * 0.35 * roll_speed
            } else {
                // Crackles come thicker the faster you go
                let hit = if self.chance() < 0.004 + 0.03 * roll_speed { self.noise_n(2) } else { 0.0 };
                self.grit_lp += 0.3 * (hit - self.grit_lp);
                let n = self.noise_n(1);
                self.roll_lp[0] += 0.04 * (n - self.roll_lp[0]);
                (self.grit_lp * 0.5 + self.roll_lp[0] * 0.25) * roll_speed
            }
        } else {
            0.0
        };

        // ── Wind — a rush that builds with speed, brighter as it does ──
        let air = (speed.abs() / 170.0).min(1.2);
        let n = self.noise();
        self.wind_lp += (0.05 + air * 0.25) * (n - self.wind_lp);
        let wind = self.wind_lp * 0.08 * air * air;

        // ── The nearest other cars: thinner, buzzier engines of their own,
        //    each from where it is ──
        let (mut other_l, mut other_r) = (0.0, 0.0);
        for k in 0..OTHER_CARS {
            let level = p.get(&p.other_level[k]);
            if level <= 0.001 {
                continue;
            }
            let car = &mut self.others[k];
            let target = engine_revs(p.get(&p.other_speed[k]).abs());
            car.rpm += (target - car.rpm) * (dt / 0.08).min(1.0);
            let target = p.get_place(&p.other_place[k]);
            let ease = (dt / 0.05).min(1.0);
            car.place.pan += (target.pan - car.place.pan) * ease;
            car.place.behind += (target.behind - car.place.behind) * ease;
            let freq = (30.0 + car.rpm * 220.0) * p.get(&p.other_pitch[k]) * (1.0 + k as f32 * 0.03);
            car.phase += freq * dt;
            if car.phase > 1.0 { car.phase -= 1.0; }
            let raw = if car.phase < 0.35 { 1.0 } else { -0.4 } + (car.phase * 3.0 * TAU).sin() * 0.4;
            car.lp += (0.08 + car.rpm * 0.2) * (raw - car.lp);
            let v = car.lp * 0.09 * level;
            let (l, r) = car.ear.place(v, car.place, self.sample_rate as f32);
            other_l += l;
            other_r += r;
        }

        // ── Rain: a soft hiss, and wet tyres swishing at speed ──
        let rain = p.get(&p.rain);
        let wet = p.get(&p.wet);
        let rain_sound = if rain > 0.01 || wet > 0.01 {
            let n = self.noise_n(2);
            self.rain_lp += 0.45 * (n - self.rain_lp);
            let n2 = self.noise_n(1);
            self.swish_lp += 0.12 * (n2 - self.swish_lp);
            let air = (speed.abs() / 150.0).min(1.0);
            (n - self.rain_lp) * 0.035 * rain + self.swish_lp * 0.06 * wet * air
        } else {
            0.0
        };

        // ── The crowd: a roar of voices, swelling and ebbing ──
        let crowd_target = p.get(&p.crowd);
        self.crowd_level += (crowd_target - self.crowd_level) * (dt / 0.6).min(1.0);
        let crowd = if self.crowd_level > 0.005 {
            self.crowd_time += dt;
            let n = self.noise_n(3);
            self.crowd_lp[0] += 0.3 * (n - self.crowd_lp[0]);
            self.crowd_lp[1] += 0.05 * (self.crowd_lp[0] - self.crowd_lp[1]);
            let t = self.crowd_time;
            let swell = 0.7 + 0.2 * (t * 0.55).sin() + 0.1 * (t * 1.9 + 1.3).sin();
            (self.crowd_lp[0] - self.crowd_lp[1]) * 0.16 * self.crowd_level * swell
        } else {
            0.0
        };

        // ── The underside grinding along: a rasp of high noise on tarmac,
        //    a duller scour on dirt ──
        let scrape_target = p.get(&p.scrape);
        self.scrape_level += (scrape_target - self.scrape_level) * (dt / 0.03).min(1.0);
        let grind = if self.scrape_level > 0.005 {
            let n = self.noise_n(4);
            let (fast, slow) = if paved { (0.7, 0.25) } else { (0.25, 0.04) };
            self.scrape_lp[0] += fast * (n - self.scrape_lp[0]);
            self.scrape_lp[1] += slow * (self.scrape_lp[0] - self.scrape_lp[1]);
            (self.scrape_lp[0] - self.scrape_lp[1]) * 0.3 * self.scrape_level
        } else {
            0.0
        };

        // ── Kerbs and rough ground: a low buzz ──
        self.rumble_phase += (38.0 + speed.abs() * 0.15) * dt;
        if self.rumble_phase > 1.0 { self.rumble_phase -= 1.0; }
        let buzz = if self.rumble_phase < 0.5 { 1.0 } else { -1.0 } * 0.05 * rumble;

        // ── Out in the country: birds calling by day, crickets by night,
        //    heard once the engine's quiet enough ──
        let quiet = p.get(&p.quiet) * (1.0 - (speed.abs() / 60.0).min(1.0) * 0.7);
        let daylight = p.get(&p.daylight);
        let ambient = if quiet > 0.02 {
            let mut v = 0.0;
            // A call: a quick run of chirps, each sweeping down in pitch
            if self.call >= 0.0 {
                let t = self.call;
                self.call += dt;
                let chirp = (t * 9.0).fract();
                let f = self.call_pitch * (4200.0 - 1800.0 * chirp);
                self.call_phase += f * dt;
                if self.call_phase > 1.0 { self.call_phase -= 1.0; }
                let env = (1.0 - chirp).powi(2) * (chirp * 30.0).min(1.0);
                v += (self.call_phase * TAU).sin() * env * 0.045 * daylight;
                if t > 0.35 + self.call_pitch * 0.3 {
                    self.call = -1.0;
                }
            } else {
                self.ambient_wait -= dt;
                if self.ambient_wait <= 0.0 {
                    self.call = 0.0;
                    self.call_pitch = 0.8 + self.chance() * 0.5;
                    self.ambient_wait = 0.6 + self.chance() * 3.0;
                }
            }
            // Crickets: a high trill in bursts, a few a second
            self.cricket_phase += 4600.0 * dt;
            if self.cricket_phase > 1.0 { self.cricket_phase -= 1.0; }
            self.cricket_time += dt;
            let trill = (self.cricket_phase * TAU).sin();
            let pulse = if (self.cricket_time * 22.0).fract() < 0.5 && (self.cricket_time * 1.7).fract() < 0.6 { 1.0 } else { 0.0 };
            v += trill * pulse * 0.013 * (1.0 - daylight);
            v * quiet
        } else {
            0.0
        };

        // ── The steam train: a chuff of exhaust four times a turn of the
        //    wheels, the first of each four hardest ──
        let train_level = p.get(&p.train[0]);
        if train_level > 0.002 {
            let before = self.chuff;
            self.chuff += p.get(&p.train[1]) * dt;
            let accent = if (before as u32) % 4 == 0 { 1.0 } else { 0.65 };
            let u = self.chuff.fract();
            let n = self.noise_n(2);
            self.chuff_lp[0] += 0.25 * (n - self.chuff_lp[0]);
            self.chuff_lp[1] += 0.02 * (self.chuff_lp[0] - self.chuff_lp[1]);
            let v = (self.chuff_lp[0] - self.chuff_lp[1]) * (-u * 7.0).exp() * accent * 0.5 * train_level;
            let (l, r) = self.train_ear.place(v, p.get_place(&p.train[2..4]), self.sample_rate as f32);
            other_l += l;
            other_r += r;
        }
        // ── A crossing bell: struck twice a second, ringing out between ──
        let bell_level = p.get(&p.bell[0]);
        if bell_level > 0.002 {
            self.bell_time = (self.bell_time + dt) % 0.5;
            let u = self.bell_time;
            let ring = (u * 1180.0 * TAU).sin() + (u * 2950.0 * TAU).sin() * 0.5 + (u * 4130.0 * TAU).sin() * 0.25;
            let v = ring * (-u * 8.0).exp() * 0.07 * bell_level;
            let (l, r) = self.bell_ear.place(v, p.get_place(&p.bell[1..3]), self.sample_rate as f32);
            other_l += l;
            other_r += r;
        }

        // What the space around you throws back: the car's own sounds and
        // the knocks, not the weather, the wind or the crowd (already
        // a wash of their own)
        let close = engine + squeal + gravel + roll + grind + buzz;
        let centre = close + wind + rain_sound + crowd + ambient;
        let (mut left, mut right) = (centre + other_l, centre + other_r);
        let mut send = close + (other_l + other_r) * 0.5;

        // ── One-shot sounds, each from where it happened ──
        self.queue_count += 1;
        if self.queue_count >= QUEUE_EVERY {
            self.queue_count = 0;
            if let Ok(mut queue) = p.queue.try_lock() {
                for v in queue.drain(..) {
                    if self.voices.len() < MAX_VOICES {
                        self.voices.push(v);
                    }
                }
            }
        }
        let rate = self.sample_rate as f32;
        self.voices.retain_mut(|v| match v.next(dt) {
            Some(s) => {
                let (l, r) = v.ear.place(s, v.place, rate);
                left += l;
                right += r;
                send += (l + r) * 0.5;
                true
            }
            None => false,
        });
        let (room_l, room_r) = self.room.run(send, &p, rate);
        left += room_l;
        right += room_r;

        // A soft knee rather than a hard clip when everything happens at once
        let limit = |x: f32| 0.8 * (x / 0.8).tanh();
        (limit(left), limit(right))
    }
}

impl Iterator for EngineSource {
    type Item = f32;

    /// Interleaved stereo: left, then right.
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let (l, r) = self.frame();
        self.right = Some(r);
        Some(l)
    }
}

impl Source for EngineSource {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> NonZero<u16> { NonZero::new(2).unwrap() }
    fn sample_rate(&self) -> NonZero<u32> { NonZero::new(self.sample_rate).unwrap() }
    fn total_duration(&self) -> Option<Duration> { None }
}

pub struct GameAudio {
    pub params: Arc<AudioParams>,
    _sink: MixerDeviceSink,
}

impl GameAudio {
    pub fn new() -> Option<Self> {
        let sink = DeviceSinkBuilder::open_default_sink().ok()?;
        let params = Arc::new(AudioParams::new());
        let source = EngineSource::new(Arc::clone(&params), 44100);
        sink.mixer().add(source);
        Some(Self { params, _sink: sink })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pan: f32) -> Place {
        Place { pan, behind: 0.0 }
    }

    const ALL: [Sound; 15] = [
        Sound::Clang, Sound::Crunch, Sound::Thunk, Sound::Splash, Sound::Landing, Sound::Beep, Sound::GoBeep,
        Sound::Boom, Sound::Whoosh, Sound::Pickup, Sound::Chime, Sound::Fanfare, Sound::Pop, Sound::Whistle,
        Sound::Swish,
    ];

    #[test]
    fn revs_climb_through_each_gear_and_drop_at_the_change() {
        let mut last = engine_revs(0.0);
        let mut shifts = 0;
        for k in 1..400 {
            let r = engine_revs(k as f32 * 0.5);
            assert!((0.0..=1.0).contains(&r));
            if r < last - 0.05 {
                shifts += 1;
            }
            last = r;
        }
        assert_eq!(shifts, GEAR_TOPS.len() - 1, "one drop per change up");
    }

    #[test]
    fn mix_stays_in_range_whatever_is_going_on() {
        let params = Arc::new(AudioParams::new());
        let mut source = EngineSource::new(Arc::clone(&params), 44100);
        for (speed, slide, paved, rumble) in [(0.0, 0.0, true, 0.0), (160.0, 40.0, true, 1.0), (90.0, 30.0, false, 0.6)] {
            params.set_speed(speed);
            params.set_sliding(slide);
            params.set_paved(paved);
            params.set_rumble(rumble);
            params.set_throttle(1.0);
            params.set_ambience(1.0, 0.5);
            for k in 0..OTHER_CARS {
                params.set_other(k, speed * 0.9, 1.0, 1.2, at(if k == 0 { -1.0 } else { 1.0 }));
            }
            params.set_rain(1.0, 1.0);
            params.set_train(1.0, 5.0, at(0.5));
            params.set_room(1.0, 1.0, 1.0, 0.3);
            params.set_bell(1.0, at(-0.5));
            for sound in ALL {
                params.play(sound, 1.5, at(0.0), 0.0);
            }
            let mut loudest = 0.0_f32;
            for _ in 0..44100 {
                let v = source.next().unwrap();
                assert!(v.is_finite());
                loudest = loudest.max(v.abs());
            }
            assert!(loudest > 0.01 && loudest <= 0.8, "level {loudest} at speed {speed}");
        }
    }

    /// Render `sound` on its own: (peak, RMS) over its length.
    fn render(sound: Sound, gain: f32) -> (f32, f32) {
        let mut v = Voice { sound, gain, place: Place::CENTRE, ear: Spatial::default(), delay: 0.0, t: 0.0, seed: 0x1234_5679, lp: [0.0; 2] };
        let dt = 1.0 / 44100.0;
        let (mut peak, mut sum, mut n) = (0.0_f32, 0.0, 0);
        while let Some(s) = v.next(dt) {
            assert!(s.is_finite(), "{sound:?} went non-finite");
            peak = peak.max(s.abs());
            sum += s * s;
            n += 1;
        }
        let expected = sound.length() / dt;
        assert!((n as f32 - expected).abs() < expected * 0.01, "{sound:?} ran {n} samples, not {expected}");
        (peak, (sum / n as f32).sqrt())
    }

    #[test]
    fn every_sound_is_heard_and_none_is_deafening() {
        for sound in ALL {
            let (peak, rms) = render(sound, 1.0);
            println!("{sound:?}: peak {peak:.3} rms {rms:.4}");
        }
        for sound in ALL {
            let (peak, rms) = render(sound, 1.0);
            assert!(peak > 0.03, "{sound:?} too quiet: peak {peak}");
            assert!(peak < 0.75, "{sound:?} too loud: peak {peak}");
            assert!(rms > 0.004, "{sound:?} barely there: rms {rms}");
        }
        // A light knock is quieter than a hard one
        assert!(render(Sound::Clang, 0.3).0 < render(Sound::Clang, 1.0).0 * 0.5);
    }

    #[test]
    fn sounds_are_placed_and_delayed() {
        let params = Arc::new(AudioParams::new());
        let mut source = EngineSource::new(Arc::clone(&params), 44100);
        // Let the idling engine settle, then measure it to hear above
        for _ in 0..44100 {
            source.next();
        }
        let loudest = |source: &mut EngineSource, frames: usize| {
            let (mut l, mut r) = (0.0_f32, 0.0_f32);
            for _ in 0..frames {
                l = l.max(source.next().unwrap().abs());
                r = r.max(source.next().unwrap().abs());
            }
            (l, r)
        };
        let (base_l, base_r) = loudest(&mut source, 4410);
        params.play(Sound::Beep, 1.0, at(1.0), 0.1);
        // Not yet: it's a tenth of a second away (allow for the queue)
        let (l, r) = loudest(&mut source, 3900);
        assert!(l <= base_l + 0.01 && r <= base_r + 0.01, "heard too soon");
        // Then mostly in the right ear, but the left still hears it
        let (l, r) = loudest(&mut source, 8000);
        assert!(r - base_r > 2.0 * (l - base_l), "not placed right: left {l}, right {r}");
        assert!(l > base_l + 0.01, "nothing in the far ear: {l}");
    }

    /// Clap once in a space set up as given and listen: the energy that
    /// comes back after `from` seconds, and the loudest single sample.
    fn clap(wet: f32, decay: f32, echo: f32, echo_delay: f32, from: f32) -> (f32, f32) {
        let params = AudioParams::new();
        params.set_room(wet, decay, echo, echo_delay);
        let mut room = Room::new(44100);
        // Settle the eased settings first
        for _ in 0..44100 * 3 {
            room.run(0.0, &params, 44100.0);
        }
        let (mut tail, mut peak) = (0.0, 0.0_f32);
        for k in 0..44100 * 2 {
            let (l, r) = room.run(if k == 0 { 1.0 } else { 0.0 }, &params, 44100.0);
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs()).max(r.abs());
            if k as f32 / 44100.0 > from {
                tail += l * l + r * r;
            }
        }
        (tail, peak)
    }

    #[test]
    fn rooms_ring_on_and_longer_ones_longer() {
        let (dry, _) = clap(0.0, 0.0, 0.0, 0.1, 0.05);
        assert_eq!(dry, 0.0, "open air rang");
        let (short, _) = clap(0.5, 0.0, 0.0, 0.1, 0.3);
        let (long, peak) = clap(0.5, 1.0, 0.0, 0.1, 0.3);
        assert!(short > 0.0, "no tail");
        assert!(long > short * 3.0, "longest no longer: {long} vs {short}");
        assert!(peak < 0.5, "rings too loud: {peak}");
        // At its longest it still dies away
        let (late, _) = clap(1.0, 1.0, 0.0, 0.1, 1.8);
        let (early, _) = clap(1.0, 1.0, 0.0, 0.1, 0.2);
        assert!(late < early * 0.2, "never dies away");
    }

    #[test]
    fn a_hillside_answers_after_the_round_trip() {
        let params = AudioParams::new();
        params.set_room(0.0, 0.0, 0.5, 0.25);
        let mut room = Room::new(44100);
        for _ in 0..44100 * 8 {
            room.run(0.0, &params, 44100.0);
        }
        let mut heard = None;
        for k in 0..44100 / 2 {
            let (l, _) = room.run(if k == 0 { 1.0 } else { 0.0 }, &params, 44100.0);
            if l.abs() > 0.1 && heard.is_none() {
                heard = Some(k as f32 / 44100.0);
            }
        }
        let t = heard.expect("no echo");
        assert!((t - 0.25).abs() < 0.01, "echo at {t}s");
    }

    /// A sound from hard right: the far ear hears it about the width of a
    /// head later than the near one.
    #[test]
    fn the_far_ear_hears_it_later() {
        let mut ear = Spatial::default();
        let rate = 44100.0;
        let (mut first_l, mut first_r) = (None, None);
        for k in 0..200 {
            let s = if k == 0 { 1.0 } else { 0.0 };
            let (l, r) = ear.place(s, at(1.0), rate);
            if l.abs() > 0.01 && first_l.is_none() { first_l = Some(k); }
            if r.abs() > 0.01 && first_r.is_none() { first_r = Some(k); }
        }
        let (l, r) = (first_l.unwrap(), first_r.unwrap());
        let lag = (EAR_LAG * rate) as usize;
        assert_eq!(r, 0, "near ear late");
        assert!(l >= lag && l <= lag + 1, "far ear {l} samples late, not {lag}");
    }

    /// The same sound from behind comes through duller and a little quieter.
    #[test]
    fn sounds_from_behind_are_duller() {
        // Brightness: how much the sound changes sample to sample
        let measure = |behind: f32| {
            let mut v = Voice { sound: Sound::Clang, gain: 1.0, place: Place { pan: 0.0, behind }, ear: Spatial::default(), delay: 0.0, t: 0.0, seed: 77, lp: [0.0; 2] };
            let (mut level, mut edge, mut last) = (0.0, 0.0, 0.0);
            let rate = 44100.0;
            while let Some(s) = v.next(1.0 / rate) {
                let (l, _) = v.ear.place(s, v.place, rate);
                level += l * l;
                edge += (l - last) * (l - last);
                last = l;
            }
            (level, edge)
        };
        let (front_level, front_edge) = measure(0.0);
        let (back_level, back_edge) = measure(1.0);
        assert!(back_level < front_level, "no quieter from behind");
        let (front_bright, back_bright) = (front_edge / front_level, back_edge / back_level);
        assert!(back_bright < front_bright * 0.5, "not duller from behind: {back_bright} vs {front_bright}");
    }

    #[test]
    #[ignore]
    fn audio_demo() {
        let rate = 44100;
        let params = Arc::new(AudioParams::new());
        let mut source = EngineSource::new(Arc::clone(&params), rate);
        let mut samples: Vec<i16> = Vec::new();
        let mut run = |source: &mut EngineSource, seconds: f32, mut each: Box<dyn FnMut(f32) + '_>| {
            let frames = (seconds * rate as f32) as usize;
            for k in 0..frames {
                if k % 64 == 0 {
                    each(k as f32 / frames as f32);
                }
                for _ in 0..2 {
                    samples.push((source.next().unwrap() * 32767.0) as i16);
                }
            }
        };
        let p = Arc::clone(&params);
        // Idling on the grid as the lights come on, by the crowd
        p.set_crowd(0.7);
        for k in 0..3 {
            p.play(Sound::Beep, 1.0, at(0.0), 0.3 + k as f32);
        }
        p.play(Sound::GoBeep, 1.0, at(0.0), 3.3);
        run(&mut source, 3.4, Box::new(|_| p.set_throttle(0.1)));
        // Away through the gears, flat out against the limiter
        let mut speed = 0.0_f32;
        run(&mut source, 9.0, Box::new(|u| {
            p.set_throttle(1.0);
            speed = 180.0 * (u * 1.25).min(1.0).powf(0.8);
            p.set_speed(speed);
            p.set_crowd(0.7 * (1.0 - u * 3.0).max(0.0));
        }));
        // Lift: the exhaust crackles
        for k in 0..4 {
            p.play(Sound::Pop, 1.0, at(0.0), 0.05 + k as f32 * 0.11);
        }
        run(&mut source, 1.0, Box::new(|u| {
            p.set_throttle(0.0);
            p.set_speed(180.0 - 40.0 * u);
        }));
        // A rival comes up from behind on the left, draws alongside and
        // pulls away ahead, its note falling as it goes
        run(&mut source, 4.0, Box::new(|u| {
            p.set_throttle(0.8);
            p.set_speed(130.0);
            // Its angle round from dead behind (0) through the left side to ahead (1)
            let a = u * std::f32::consts::PI;
            let place = Place { pan: -a.sin(), behind: ((0.2 + a.cos()) / 0.8).clamp(0.0, 1.0) };
            let near = 1.0 - (u - 0.5).abs() * 1.6;
            p.set_other(0, 150.0, near.max(0.0).powi(2), if u < 0.5 { 1.08 } else { 0.92 }, place);
        }));
        p.set_other(0, 0.0, 0.0, 1.0, at(0.0));
        // Knocks: a wall on the right, a car on the left, a tree, a jump
        p.play(Sound::Clang, 0.9, at(0.6), 0.2);
        p.play(Sound::Crunch, 0.9, at(-0.6), 1.2);
        p.play(Sound::Thunk, 0.9, at(0.2), 2.2);
        // ...and the same clang again, from behind you
        p.play(Sound::Clang, 0.9, Place { pan: 0.0, behind: 1.0 }, 2.9);
        p.play(Sound::Landing, 1.0, at(0.0), 3.4);
        run(&mut source, 4.2, Box::new(|_| p.set_speed(110.0)));
        // Off the road onto gravel, sliding; then a boost pad
        run(&mut source, 2.0, Box::new(|_| {
            p.set_paved(false);
            p.set_sliding(30.0);
        }));
        p.set_paved(true);
        p.set_sliding(0.0);
        p.play(Sound::Whoosh, 1.0, at(0.0), 0.1);
        p.play(Sound::Pickup, 1.0, at(0.0), 1.2);
        run(&mut source, 2.0, Box::new(|_| ()));
        // Into the lake
        p.play(Sound::Splash, 1.0, at(0.0), 0.1);
        run(&mut source, 1.6, Box::new(|_| {
            p.set_speed(0.0);
            p.set_throttle(0.0);
        }));
        // The flag: the fanfare, the crowd, fireworks going off
        p.play(Sound::Fanfare, 1.0, at(0.0), 0.2);
        for k in 0..6 {
            p.play(Sound::Boom, 0.5, at((k as f32 * 1.7).sin() * 0.6), 0.6 + k as f32 * 0.45);
        }
        run(&mut source, 4.5, Box::new(|_| p.set_crowd(1.0)));
        // The same knock and a blip of revs in three places: open country,
        // a steep valley with a cliff 60 units off, and by the grandstands
        for (wet, decay, echo, delay) in [(0.0, 0.0, 0.0, 0.1), (0.25, 0.8, 0.3, 0.35), (0.35, 0.75, 0.0, 0.1)] {
            p.set_room(wet, decay, echo, delay);
            run(&mut source, 0.6, Box::new(|_| ()));
            p.play(Sound::Clang, 0.8, at(0.3), 0.0);
            run(&mut source, 2.2, Box::new(|u| {
                p.set_throttle(if u > 0.35 && u < 0.6 { 1.0 } else { 0.0 });
                p.set_speed(if u > 0.35 && u < 0.6 { 60.0 } else { 10.0 });
            }));
        }
        p.set_room(0.0, 0.0, 0.0, 0.1);
        p.set_speed(0.0);
        p.play(Sound::Swish, 1.0, at(0.0), 0.3);
        run(&mut source, 1.0, Box::new(|_| p.set_speed(90.0)));
        p.set_speed(0.0);
        // Waiting at a level crossing: the bell, the whistle, the train by
        p.set_crowd(0.0);
        p.play(Sound::Whistle, 0.8, at(0.7), 0.5);
        run(&mut source, 5.0, Box::new(|u| {
            p.set_bell(0.8, at(-0.3));
            let d = (u - 0.6) * 2.0;
            p.set_train((1.0 - d.abs()).max(0.05), 5.0, at(d));
        }));
        p.set_bell(0.0, at(0.0));
        p.set_train(0.0, 0.0, at(0.0));
        // Parked out in the country at dusk: birds, then crickets
        p.set_crowd(0.0);
        run(&mut source, 6.0, Box::new(|u| p.set_ambience(1.0, 1.0 - u)));

        let path = std::env::var("STUMBLER_AUDIO_DEMO").unwrap_or_else(|_| "audio-demo.wav".into());
        let data = (samples.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(&path, wav).unwrap();
        println!("wrote {path}");
    }

    /// How much of one core the mix takes with everything going at once.
    #[test]
    #[ignore]
    fn audio_cost() {
        let params = Arc::new(AudioParams::new());
        let mut source = EngineSource::new(Arc::clone(&params), 44100);
        params.set_speed(120.0);
        params.set_throttle(1.0);
        params.set_sliding(30.0);
        params.set_rain(1.0, 1.0);
        params.set_crowd(1.0);
        params.set_scrape(1.0);
        params.set_ambience(1.0, 0.5);
        params.set_train(1.0, 6.0, at(0.0));
        params.set_bell(1.0, at(0.0));
        for k in 0..OTHER_CARS {
            params.set_other(k, 100.0, 1.0, 1.0, at(0.0));
        }
        let start = std::time::Instant::now();
        let mut sum = 0.0;
        for second in 0..10 {
            for _ in 0..12 {
                params.play(ALL[second % ALL.len()], 1.0, at(0.0), 0.0);
            }
            for _ in 0..44100 * 2 {
                sum += source.next().unwrap();
            }
        }
        let used = start.elapsed().as_secs_f32() / 10.0;
        println!("audio: {:.2}% of a core ({sum})", used * 100.0);
        assert!(used < 0.1);
    }

    #[test]
    fn a_change_up_cuts_the_power_for_a_moment() {
        let params = Arc::new(AudioParams::new());
        let mut source = EngineSource::new(Arc::clone(&params), 44100);
        params.set_throttle(1.0);
        params.set_speed(GEAR_TOPS[0] - 1.0);
        for _ in 0..4410 {
            source.next();
        }
        params.set_speed(GEAR_TOPS[0] + 0.5);
        source.next();
        source.next();
        assert!(source.shift > 0.0, "no cut at the change");
    }
}
