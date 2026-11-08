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

/// How many diesels can be heard at once.
pub const DIESELS: usize = 2;

/// The one-shot sounds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    /// A car into a post, wall or barrier: a heavy thud and crumpling panels.
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
    /// A cow lowing in the field.
    Moo,
    /// A sheep bleating.
    Baa,
    /// One stroke of a church or town-hall bell.
    Toll,
    /// A halyard slapping a mast, down at the moorings.
    Tink,
    /// A dog's bark.
    Bark,
    /// A balloon's burner roaring.
    Burner,
    /// A campfire's crackle: a log popping.
    Crackle,
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
            Sound::Pop => 0.12,
            Sound::Whistle => 2.0,
            Sound::Moo => 1.7,
            Sound::Baa => 0.8,
            Sound::Toll => 4.0,
            Sound::Tink => 0.5,
            Sound::Bark => 0.3,
            Sound::Burner => 2.6,
            Sound::Crackle => 0.25,
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
    /// Oscillator phases and odd bits of state (bubbles, ticks).
    st: [f32; 4],
    filters: [Svf; 3],
}

impl Voice {
    fn new(sound: Sound, gain: f32, place: Place, delay: f32, seed: u32) -> Self {
        Self { sound, gain, place, ear: Spatial::default(), delay, t: 0.0, seed, lp: [0.0; 2], st: [0.0; 4], filters: [Svf::default(); 3] }
    }

    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    /// 0..1.
    fn chance(&mut self) -> f32 {
        self.noise() * 0.5 + 0.5
    }

    /// A low note whose pitch drops from `from` to `to` (Hz) as it dies, at
    /// rate `drop`: the thud of a heavy body, on oscillator `k`.
    fn thud(&mut self, k: usize, t: f32, dt: f32, from: f32, to: f32, drop: f32) -> f32 {
        let f = to + (from - to) * (-t * drop).exp();
        self.st[k] = (self.st[k] + f * dt).fract();
        (self.st[k] * TAU).sin()
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
        let rate = 1.0 / dt;
        let sine = |f: f32| (t * f * TAU).sin();
        // A random detune per voice, so no two knocks are quite alike
        let detune = 1.0 + ((self.seed & 0xff) as f32 / 255.0 - 0.5) * 0.12;
        // The first sample: what strikes the resonators
        let strike = if t < dt { 1.0 } else { 0.0 };
        let g = self.gain.min(1.5);
        let v = match self.sound {
            Sound::Clang => {
                // Into a wall or barrier: the car's weight landing as a low
                // thud, panels crumpling in a crackle that thins out, a dull
                // knock from the barrier and bits rattling down after
                let thud = self.thud(0, t, dt, 115.0, 40.0, 26.0) * (-t * 12.0).exp();
                let density = 0.4 * (-t * 15.0).exp() * (0.4 + g);
                let grain = if self.chance() < density { self.noise() } else { 0.0 };
                let bed = self.noise() * (-t * 35.0).exp() * 0.35;
                let crunch = self.filters[0].band(grain + bed, (900.0 + 500.0 * g) * detune, rate, 1.0);
                let knock = self.filters[1].band(strike * 3.0 + grain * 0.15, 165.0 * detune, rate, 0.25);
                if t > 0.05 && self.st[2] <= 0.0 && self.chance() < 0.0012 * g * (-t * 3.0).exp() {
                    self.st[2] = 0.004;
                }
                let tick = if self.st[2] > 0.0 { self.st[2] -= dt; self.noise() } else { 0.0 };
                let debris = self.filters[2].band(tick, 2400.0 * detune, rate, 0.7);
                (thud * 0.85 + crunch * 1.3 + knock * 0.45 + debris * 0.35) * 0.55
            }
            Sound::Crunch => {
                // Car into car: a lighter thud, a longer crumple of panels
                // and plastic, and the tyres scuffing against each other
                let thud = self.thud(0, t, dt, 120.0, 55.0, 30.0) * (-t * 17.0).exp();
                let density = 0.5 * (-t * 8.0).exp() * (0.4 + g);
                let grain = if self.chance() < density { self.noise() } else { 0.0 };
                let panels = self.filters[0].band(grain, 1500.0 * detune, rate, 1.0);
                let plastic = self.filters[1].band(grain, 650.0 * detune, rate, 0.7);
                let n = self.noise();
                self.lp[0] += 0.05 * (n - self.lp[0]);
                let scuff = self.lp[0] * (t * 30.0).min(1.0) * (-t * 6.0).exp();
                if t > 0.08 && self.st[2] <= 0.0 && self.chance() < 0.001 * g * (-t * 3.0).exp() {
                    self.st[2] = 0.004;
                }
                let tick = if self.st[2] > 0.0 { self.st[2] -= dt; self.noise() } else { 0.0 };
                let debris = self.filters[2].band(tick, 3000.0 * detune, rate, 0.7);
                (thud * 0.6 + panels * 1.2 + plastic * 0.7 + scuff * 0.8 + debris * 0.3) * 0.5
            }
            Sound::Thunk => {
                // A trunk knocked: two woody notes that die almost at once,
                // the bark cracking, then the leaves shaken loose
                let k1 = self.filters[0].band(strike * 4.0, 140.0 * detune, rate, 0.12);
                let k2 = self.filters[1].band(strike * 3.0, 380.0 * detune, rate, 0.18);
                let grain = if self.chance() < 0.35 * (-t * 40.0).exp() { self.noise() } else { 0.0 };
                let crack = self.filters[2].band(grain, 2200.0, rate, 1.0);
                let thud = self.thud(0, t, dt, 90.0, 50.0, 30.0) * (-t * 20.0).exp();
                let n = self.noise();
                self.lp[0] += 0.5 * (n - self.lp[0]);
                // Leaves: bright noise that flutters as they shake
                self.st[1] += (self.noise() * 0.5 - self.st[1]) * 0.002;
                let flutter = 0.6 + 4.0 * self.st[1].abs();
                let rustle = (n - self.lp[0]) * flutter * (t * 8.0).min(1.0) * (-t * 2.8).exp();
                (k1 * 0.5 + k2 * 0.35 + crack * 0.8 + thud * 0.5 + rustle * 0.3) * 0.5
            }
            Sound::Splash => {
                // A whump, then spray falling back (noise whose brightness
                // falls away), and bubbles glooping up after
                let n = self.noise();
                let open = 0.5 * (-t * 2.2).exp() + 0.03;
                self.lp[0] += open * (n - self.lp[0]);
                let env = (t * 80.0).min(1.0) * (-t * 2.6).exp();
                let whump = self.thud(0, t, dt, 70.0, 30.0, 15.0) * (-t * 8.0).exp();
                // A bubble: a short note that rises as it goes
                if self.st[2] <= 0.0 && t > 0.15 && self.chance() < 0.0025 * (-t * 1.2).exp() {
                    self.st[2] = 0.001;
                    self.st[3] = 250.0 + self.chance() * 600.0;
                }
                let mut bubble = 0.0;
                if self.st[2] > 0.0 {
                    let age = self.st[2];
                    self.st[2] += dt;
                    self.st[1] = (self.st[1] + self.st[3] * (1.0 + age * 25.0) * dt).fract();
                    bubble = (self.st[1] * TAU).sin() * (-age * 45.0).exp();
                    if age > 0.08 {
                        self.st[2] = 0.0;
                    }
                }
                (self.lp[0] * 1.6 * env + whump * 0.7 + bubble * 0.25) * 0.32
            }
            Sound::Landing => {
                // The springs bottoming out: a deep thud, a clunk in the
                // suspension and a scuff of the tyres taking the weight
                let thud = self.thud(0, t, dt, 80.0, 34.0, 18.0) * (-t * 10.0).exp();
                let clunk = self.filters[0].band(strike * 4.0, 240.0 * detune, rate, 0.3);
                let n = self.noise();
                self.lp[0] += 0.12 * (n - self.lp[0]);
                let scuff = self.lp[0] * (-t * 22.0).exp();
                (thud * 0.85 + clunk * 0.4 + scuff * 0.9) * 0.55
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
                // A backfire: a sharp crack out of the pipe over a low boom
                let n = self.noise();
                let crack = self.filters[0].band(n * (-t * 90.0).exp(), 1700.0 * detune, rate, 1.2);
                let boom = self.thud(0, t, dt, 110.0, 55.0, 40.0) * (-t * 38.0).exp();
                (crack * 1.4 + boom * 0.6) * 0.5
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
            Sound::Moo => {
                // A low buzz of a voice through an opening mouth: "mmm-oooo",
                // the pitch lifting then sagging
                let f0 = (105.0 + 45.0 * (t / 0.5).min(1.0) - 30.0 * ((t - 0.6) / 1.0).clamp(0.0, 1.0)) * detune;
                self.st[0] = (self.st[0] + f0 * dt).fract();
                let saw = self.st[0] * 2.0 - 1.0;
                let open = (t / 0.5).min(1.0);
                let f1 = self.filters[0].band(saw, 280.0 + 420.0 * open, rate, 0.18);
                let f2 = self.filters[1].band(saw, 800.0 + 300.0 * open, rate, 0.22);
                let env = (t / 0.15).min(1.0) * ((1.7 - t) / 0.35).clamp(0.0, 1.0);
                (f1 + f2 * 0.6) * env * 0.2
            }
            Sound::Baa => {
                // A bleat: a reedy voice wavering hard, "meh-eh-eh"
                let f0 = (360.0 - 40.0 * t) * detune;
                self.st[0] = (self.st[0] + f0 * dt).fract();
                let saw = self.st[0] * 2.0 - 1.0;
                let waver = 0.55 + 0.45 * (t * 19.0 * TAU).sin();
                let f1 = self.filters[0].band(saw, 750.0, rate, 0.25);
                let f2 = self.filters[1].band(saw, 1850.0, rate, 0.3);
                let env = (t / 0.04).min(1.0) * ((0.8 - t) / 0.25).clamp(0.0, 1.0);
                (f1 + f2 * 0.7) * waver * env * 0.2
            }
            Sound::Toll => {
                // A big bronze bell: a hum below the strike note and the
                // bell's own minor-third tierce, the high partials dying first
                // (A fixed pitch: `detune` drifts once noise has been drawn)
                let base = 196.0;
                let partials = [(0.5, 0.5, 0.35), (1.0, 0.8, 0.6), (1.19, 0.6, 0.8), (1.5, 0.35, 1.1), (2.0, 0.45, 1.4), (2.51, 0.2, 2.2), (2.99, 0.15, 3.0)];
                let mut v = 0.0;
                for (ratio, amp, decay) in partials {
                    v += (t * base * ratio * TAU).sin() * amp * (-t * decay).exp();
                }
                let clang = if t < 0.12 { self.noise() * (-t * 40.0).exp() * 0.3 } else { 0.0 };
                (v + clang) * (t / 0.004).min(1.0) * 0.09
            }
            Sound::Tink => {
                // A rope slapping an aluminium mast: a knock and a short ring
                let n = self.noise();
                let knock = self.filters[0].band(n, 900.0, rate, 0.8) * (-t * 60.0).exp();
                let ring = (sine(1840.0) + sine(2950.0) * 0.6 + sine(4100.0) * 0.3) * (-t * 14.0).exp();
                (knock * 0.5 + ring * 0.08) * (t / 0.002).min(1.0)
            }
            Sound::Bark => {
                // A sharp "ruff": a rough voice dropping in pitch, a puff of breath
                let f0 = 520.0 - 500.0 * t;
                self.st[0] = (self.st[0] + f0 * dt).fract();
                let src = (self.st[0] * 2.0 - 1.0) * 0.7 + self.noise() * 0.3;
                let f1 = self.filters[0].band(src, 900.0, rate, 0.35);
                let f2 = self.filters[1].band(src, 1700.0, rate, 0.4);
                let env = (t / 0.01).min(1.0) * (-t * 11.0).exp();
                (f1 + f2 * 0.6) * env * 0.45
            }
            Sound::Burner => {
                // A gas burner's roar: rushing flame with a low flutter
                let n = self.noise();
                let roar = self.filters[0].band(n, 420.0, rate, 1.1);
                let body = self.filters[1].run(n, 130.0, rate, 0.9).0;
                let flutter = 0.75 + 0.25 * (t * 37.0 * TAU).sin() * self.chance();
                let env = (t / 0.08).min(1.0) * ((2.6 - t) / 0.3).clamp(0.0, 1.0);
                (roar * 0.6 + body * 0.8) * flutter * env * 0.8
            }
            Sound::Crackle => {
                // A sap pocket bursting: a sharp tick, then a few tiny ones
                let n = self.noise();
                let tick = self.filters[0].band(n, 2600.0, rate, 0.7) * (-t * 90.0).exp();
                let after = if (t * 37.0).fract() < 0.08 && t > 0.03 { n * 0.4 * (-t * 14.0).exp() } else { 0.0 };
                (tick * 0.9 + after * 0.3) * (t / 0.001).min(1.0)
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
    /// What's around you: woods, water and town (each 0..1).
    scene: [AtomicU32; 3],
    /// How loud your own engine is in the mix (1 as designed).
    engine_level: AtomicU32,
    /// The steam train: how loud its engine is here, chuffs a second, and
    /// where it is (pan, behind); and the nearest crossing's bell
    /// (loudness, pan, behind).
    train: [AtomicU32; 4],
    bell: [AtomicU32; 3],
    /// Diesels near you (fishing boats, tractors, combines): per slot, how
    /// loud here, firing strokes a second, and where (pan, behind).
    diesel: [[AtomicU32; 4]; DIESELS],
    /// The murmur of folk about: how loud here, and where.
    chatter: [AtomicU32; 3],
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
            scene: std::array::from_fn(|_| level(0.0)),
            engine_level: level(1.0),
            train: std::array::from_fn(|_| level(0.0)),
            bell: std::array::from_fn(|_| level(0.0)),
            diesel: std::array::from_fn(|_| std::array::from_fn(|_| level(0.0))),
            chatter: std::array::from_fn(|_| level(0.0)),
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
                queue.push(Voice::new(sound, gain.min(1.5), place.clamped(), delay.max(0.0), seed));
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

    /// Your own engine's loudness in the mix (1 as designed, 0 silent).
    #[allow(dead_code)]
    pub fn set_engine_level(&self, v: f32) {
        Self::set(&self.engine_level, v.clamp(0.0, 2.0));
    }

    /// What's around you, each 0..1: `forest` (trees close by), `water`
    /// (a lake or the sea in earshot) and `town`.
    pub fn set_scene(&self, forest: f32, water: f32, town: f32) {
        Self::set(&self.scene[0], forest.clamp(0.0, 1.0));
        Self::set(&self.scene[1], water.clamp(0.0, 1.0));
        Self::set(&self.scene[2], town.clamp(0.0, 1.0));
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

    /// Diesel `slot`: `level` here, `rate` firing strokes a second, and where.
    pub fn set_diesel(&self, slot: usize, level: f32, rate: f32, place: Place) {
        Self::set(&self.diesel[slot][0], level.clamp(0.0, 1.0));
        Self::set(&self.diesel[slot][1], rate);
        Self::set_place(&self.diesel[slot][2..4], place);
    }

    /// The chatter of a busy street: `level` here, and where.
    pub fn set_chatter(&self, level: f32, place: Place) {
        Self::set(&self.chatter[0], level.clamp(0.0, 1.0));
        Self::set_place(&self.chatter[1..3], place);
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

/// Top speed (units/s) in each gear.
const GEAR_TOPS: [f32; 6] = [38.0, 64.0, 90.0, 115.0, 140.0, 175.0];
/// How long the power is cut through a gear change (seconds).
const SHIFT_CUT: f32 = 0.09;
/// How long the throttle is blipped through a change down (seconds).
const BLIP: f32 = 0.12;

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

/// A resonant filter (state-variable): low, band and high pass from one
/// pair of states. Cheap, and stable up to a sixth of the sample rate.
#[derive(Clone, Copy, Default)]
struct Svf {
    low: f32,
    band: f32,
}

impl Svf {
    /// One sample through at centre `freq` (Hz), damping `damp` (about
    /// 1/Q: small rings long and narrow, 1 or more is broad): (low, band, high).
    fn run(&mut self, x: f32, freq: f32, rate: f32, damp: f32) -> (f32, f32, f32) {
        let f = 2.0 * (std::f32::consts::PI * (freq / rate).min(0.16)).sin();
        let high = x - self.low - damp * self.band;
        self.band += f * high;
        self.low += f * self.band;
        (self.low, self.band, high)
    }

    fn band(&mut self, x: f32, freq: f32, rate: f32, damp: f32) -> f32 {
        self.run(x, freq, rate, damp).1
    }
}

/// A delay line, read between samples.
struct Delay {
    buf: Vec<f32>,
    at: usize,
}

impl Delay {
    fn new(longest: f32, rate: f32) -> Self {
        Self { buf: vec![0.0; (longest * rate) as usize + 3], at: 0 }
    }

    /// What went in `lag` seconds ago.
    fn read(&self, lag: f32, rate: f32) -> f32 {
        let len = self.buf.len();
        let lag = (lag * rate).clamp(1.0, (len - 2) as f32);
        let (whole, part) = (lag as usize, lag.fract());
        let back = |k: usize| self.buf[(self.at + len - k) % len];
        back(whole) * (1.0 - part) + back(whole + 1) * part
    }

    fn write(&mut self, x: f32) {
        self.buf[self.at] = x;
        self.at = (self.at + 1) % self.buf.len();
    }
}

/// Engine speed at idle and at the red line (rpm).
const IDLE_RPM: f32 = 850.0;
const RED_LINE: f32 = 8200.0;
/// Which bank of a cross-plane V8 each firing in turn goes out of. Each
/// bank's own pulses come unevenly spaced, and that is the V8's burble.
const BANK_ORDER: [usize; 8] = [0, 1, 0, 0, 1, 0, 1, 1];
/// Each bank's exhaust: the round trip of a pressure wave down the pipe
/// and back (seconds), a touch different side to side so they don't ring
/// as one.
const PIPE_TRIPS: [f32; 2] = [0.0047, 0.0054];
/// How much of each wave the open end of the pipe throws back (inverted).
const PIPE_REFLECT: f32 = 0.55;
/// Pressure pulses in flight at once, at most.
const PULSES: usize = 10;

/// One cylinder's exhaust pulse as it leaves the valve: how long ago, how
/// strong, how wide (seconds) and which bank.
#[derive(Clone, Copy, Default)]
struct Pulse {
    age: f32,
    amp: f32,
    width: f32,
    bank: usize,
}

/// An engine, built up from what makes the noise: each cylinder firing a
/// pulse of pressure into its bank's exhaust, the pipes ringing at their
/// own lengths, the silencer's body, the intake gasping air, and all of it
/// pushed harder (and grittier) the more throttle it's given.
struct Engine {
    /// Through the four-stroke cycle (two turns of the crank), 0..1, and the
    /// next firing due.
    cycle: f32,
    next: usize,
    next_at: f32,
    pulses: [Pulse; PULSES],
    pipes: [Delay; 2],
    pipe_lp: [f32; 2],
    body: Svf,
    tone: f32,
    intake: Svf,
    intake_env: f32,
    dc: [f32; 2],
    rng: u32,
    /// Scales the pipes' lengths: another car's exhaust sounds its own.
    pipe_scale: f32,
}

impl Engine {
    fn new(rate: f32, pipe_scale: f32, seed: u32) -> Self {
        let longest = PIPE_TRIPS[1] * pipe_scale * 1.2;
        Self {
            cycle: 0.0,
            next: 0,
            next_at: 0.0,
            pulses: [Pulse::default(); PULSES],
            pipes: [Delay::new(longest, rate), Delay::new(longest, rate)],
            pipe_lp: [0.0; 2],
            body: Svf::default(),
            tone: 0.0,
            intake: Svf::default(),
            intake_env: 0.0,
            dc: [0.0; 2],
            rng: seed | 1,
            pipe_scale,
        }
    }

    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    /// The next sample at `revs` (0..1, as `engine_revs`), `load` (0 off the
    /// throttle .. 1 flat out) and `overrun` (0..1: high revs and no
    /// throttle, when it misfires and crackles).
    fn sample(&mut self, revs: f32, load: f32, overrun: f32, rate: f32) -> f32 {
        let dt = 1.0 / rate;
        let rpm = IDLE_RPM + ((revs - 0.2) / 0.8).max(0.0) * (RED_LINE - IDLE_RPM);
        // Firing: eight a cycle, each a touch early or late
        self.cycle += rpm / 120.0 * dt;
        while self.cycle >= self.next_at {
            let k = self.next % 8;
            self.next += 1;
            let jitter = self.noise() * (0.002 + 0.006 * (1.0 - revs));
            self.next_at = self.next as f32 / 8.0 + jitter;
            // Combustion: strong and steady on the power; weak, lumpy and
            // now and then missing altogether off it
            let lumpy = 0.07 + 0.25 * (1.0 - load) * (1.0 - revs);
            let mut amp = (0.3 + 0.7 * load) * (1.0 + self.noise() * lumpy);
            let mut width = 28.0 / (6.0 * rpm) * (1.25 - 0.35 * load);
            if overrun > 0.0 && self.noise() > 1.0 - 0.3 * overrun {
                amp *= 0.15;
            }
            // Overrun crackle: unburnt fuel going off in the pipe, a sharp
            // narrow spike
            if overrun > 0.0 && self.noise() > 1.0 - 0.07 * overrun {
                amp = 1.6 + self.noise() * 0.5;
                width = 0.00025;
            }
            if let Some(slot) = self.pulses.iter_mut().find(|p| p.amp == 0.0) {
                *slot = Pulse { age: 0.0, amp, width, bank: BANK_ORDER[k] };
            }
        }
        if self.cycle >= 1.0 {
            self.cycle -= 1.0;
            self.next_at -= 1.0;
            self.next -= 8;
        }
        // Each pulse: a quick rise and a slower fall
        let mut banks = [0.0_f32; 2];
        for p in &mut self.pulses {
            if p.amp == 0.0 {
                continue;
            }
            let x = p.age / p.width;
            banks[p.bank] += p.amp * x * (1.0 - x).exp();
            p.age += dt;
            if x > 9.0 {
                p.amp = 0.0;
            }
        }
        // The pipes: what comes back off the open end, inverted and dulled,
        // reinforces some notes and cancels others as the revs sweep through
        let mut out = 0.0;
        for b in 0..2 {
            let back = self.pipes[b].read(PIPE_TRIPS[b] * self.pipe_scale, rate);
            self.pipe_lp[b] += 0.45 * (back - self.pipe_lp[b]);
            let y = banks[b] - PIPE_REFLECT * self.pipe_lp[b];
            self.pipes[b].write(y);
            out += y;
        }
        // The silencer's body: a deep resonance under it all
        let body = self.body.band(out, 105.0, rate, 0.6);
        out += body * 0.5;
        // Brighter the harder it's pushed and the faster it spins
        let cutoff = 700.0 + 3500.0 * load + 1800.0 * revs;
        self.tone += (1.0 - (-TAU * cutoff * dt).exp()) * (out - self.tone);
        // The intake: air roaring in, swelling with each gulp
        self.intake_env += ((banks[0] + banks[1]).abs() - self.intake_env) * 0.01;
        let gasp = self.noise() * self.intake_env;
        let intake = self.intake.band(gasp, 500.0 + 1700.0 * revs, rate, 0.7) * (0.15 + 0.85 * load) * revs;
        // Pushed into the edge of distortion: grit under load
        let drive = 1.2 + 2.8 * load;
        let mut y = (self.tone * drive).tanh() / drive.sqrt() + intake * 0.5;
        // Keep it centred
        let x = y;
        y = x - self.dc[0] + 0.995 * self.dc[1];
        self.dc = [x, y];
        y
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

/// Creatures calling at once, at most.
const MAX_CALLS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CallKind {
    None,
    /// A quick run of chirps, each sweeping down.
    Chirps,
    /// A songbird's warble: a fluttering high tune in short phrases.
    Warble,
    /// A wood pigeon: soft low coos in a lilting rhythm.
    Coo,
    /// A gull's cry, falling away.
    Gull,
    /// An owl: hoo... hoo-hoo.
    Owl,
    /// A frog croaking.
    Frog,
}

/// A creature calling: what, how far in, its own pitch, how loud and from where.
struct Call {
    kind: CallKind,
    t: f32,
    pitch: f32,
    phase: f32,
    level: f32,
    place: Place,
    ear: Spatial,
    seed: u32,
}

impl Call {
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    /// Advance the oscillator at `f` Hz and give its sine.
    fn osc(&mut self, f: f32, dt: f32) -> f32 {
        self.phase = (self.phase + f * dt).fract();
        (self.phase * TAU).sin()
    }

    fn next(&mut self, dt: f32) -> Option<f32> {
        let t = self.t;
        self.t += dt;
        let p = self.pitch;
        let v = match self.kind {
            CallKind::None => return None,
            CallKind::Chirps => {
                if t > 0.3 + p * 0.35 { return None; }
                let chirp = (t * 9.0 * p).fract();
                let f = p * (4300.0 - 1900.0 * chirp);
                let env = (1.0 - chirp).powi(2) * (chirp * 30.0).min(1.0);
                self.osc(f, dt) * env * 0.05
            }
            CallKind::Warble => {
                if t > 1.1 { return None; }
                // Phrases of notes hopping about, with a fast trill on each
                let note = (t * 7.0) as u32;
                let hop = [0.0, 0.25, -0.1, 0.4, 0.15, -0.2, 0.3, 0.05][(note.wrapping_mul(2654435761) >> 29) as usize];
                let trill = (t * 32.0 * TAU).sin() * 0.06;
                let f = p * 3100.0 * (1.0 + hop + trill);
                let u = (t * 7.0).fract();
                let env = (u * 12.0).min(1.0) * (1.0 - u).powf(0.7) * (1.0 - t / 1.1);
                self.osc(f, dt) * env * 0.04
            }
            CallKind::Coo => {
                if t > 2.2 { return None; }
                // coo-COO-coo, coo-coo
                let starts = [0.0, 0.35, 0.9, 1.45, 1.75];
                let lens = [0.3, 0.5, 0.3, 0.25, 0.3];
                let mut env = 0.0;
                for (s, l) in starts.iter().zip(lens) {
                    if t >= *s && t < s + l {
                        let u = (t - s) / l;
                        env = (u * std::f32::consts::PI).sin().powf(1.5) * if l > 0.4 { 1.0 } else { 0.75 };
                    }
                }
                let f = p * 480.0 * (1.0 + 0.05 * env);
                let s = self.osc(f, dt);
                (s + (2.0 * self.phase * TAU).sin() * 0.15) * env * 0.05
            }
            CallKind::Gull => {
                if t > 1.2 { return None; }
                // Two or three falling cries, nasal (rich in harmonics)
                let u = (t / 0.4).fract();
                let f = p * (1250.0 - 500.0 * u);
                let s = self.osc(f, dt);
                let nasal = (s * 3.0).tanh() + (3.0 * self.phase * TAU).sin() * 0.3;
                let env = (u * 15.0).min(1.0) * (1.0 - u).powf(1.5);
                nasal * env * 0.03
            }
            CallKind::Owl => {
                if t > 1.6 { return None; }
                // hoo ... hoo-hoo, soft and breathy
                let starts = [0.0, 0.85, 1.15];
                let mut env = 0.0;
                for s in starts {
                    if t >= s && t < s + 0.35 {
                        env = ((t - s) / 0.35 * std::f32::consts::PI).sin();
                    }
                }
                let breath = self.noise() * 0.15;
                (self.osc(p * 370.0, dt) + breath) * env * 0.05
            }
            CallKind::Frog => {
                if t > 0.45 { return None; }
                // A croak: a rapid rattle of pulses through a throaty note
                let rattle = if (t * 34.0 * p).fract() < 0.3 { 1.0 } else { 0.0 };
                let s = self.osc(p * 330.0, dt);
                let env = (t * 20.0).min(1.0) * (1.0 - t / 0.45);
                (s * 2.5).tanh() * rattle * env * 0.04
            }
        };
        Some(v)
    }
}

/// A cricket: its pitch, chirps a second, how long a burst of them runs,
/// where it is, and its own clock and phase.
struct Cricket {
    pitch: f32,
    rate: f32,
    burst: f32,
    pan: f32,
    time: f32,
    phase: f32,
}

/// One of the other cars you can hear.
struct OtherVoice {
    rpm: f32,
    engine: Engine,
    lp: f32,
    /// Where it's heard from, eased toward where it is so it glides.
    place: Place,
    ear: Spatial,
}

struct EngineSource {
    params: Arc<AudioParams>,
    sample_rate: u32,
    engine: Engine,
    /// A blip of throttle through a change down (seconds left), and how
    /// long since the throttle was last lifted.
    blip: f32,
    lifted: f32,
    // Gravel filter states (one per "wheel"), and the stones' crunch
    gravel_lp: [f32; 4],
    gravel_crunch: Svf,
    /// Engine revs (0..1), eased toward what the speed and gear give, the
    /// revs a moment ago (a sudden drop is a change up), and how long the
    /// power stays cut for the change.
    rpm: f32,
    last_target: f32,
    shift: f32,
    /// Bouncing off the rev limiter.
    limiter_phase: f32,
    /// Tyre squeal: rubber resonances rung by the slip, a flutter as it
    /// sticks and lets go, and a scrub of rubber before it really squeals.
    squeal: [Svf; 3],
    squeal_wander: f32,
    squeal_flutter: f32,
    scrub: Svf,
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
    /// The countryside's bed: wind gusting (its level and where it's
    /// heading), leaves, waves and a town's hum.
    gust: f32,
    gust_target: f32,
    gust_wait: f32,
    wind_bed: [f32; 2],
    wind_svf: Svf,
    leaves_lp: f32,
    leaves_flutter: f32,
    waves: [f32; 2],
    wave_time: f32,
    hum: Svf,
    /// Creatures calling, each from its own direction, and when the next
    /// one starts; crickets chirping, each its own pitch and pace.
    calls: Vec<Call>,
    call_wait: f32,
    crickets: [Cricket; 3],
    forest: f32,
    water: f32,
    town: f32,
    /// The train's chuff (how far through the current one, filter state)
    /// and the crossing bell's strokes.
    chuff: f32,
    chuff_lp: [f32; 2],
    bell_time: f32,
    train_ear: Spatial,
    bell_ear: Spatial,
    /// Each diesel: where in its firing stroke, filter state, and its ear.
    diesels: [(f32, [f32; 2], Svf, Spatial); DIESELS],
    /// The chatter: three voices, each with its syllable clock, pace,
    /// loudness, pitch and eased level, and formant filters; then the ear.
    babble: ([[f32; 5]; 3], [[Svf; 2]; 3], Spatial),
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
            engine: Engine::new(sample_rate as f32, 1.0, 0x5eed_1234),
            blip: 0.0,
            lifted: 10.0,
            gravel_lp: [0.0; 4],
            gravel_crunch: Svf::default(),
            rpm: 0.2,
            last_target: 0.2,
            shift: 0.0,
            limiter_phase: 0.0,
            squeal: [Svf::default(); 3],
            squeal_wander: 0.0,
            squeal_flutter: 0.0,
            scrub: Svf::default(),
            wind_lp: 0.0,
            rumble_phase: 0.0,
            others: std::array::from_fn(|k| OtherVoice {
                rpm: 0.2,
                engine: Engine::new(sample_rate as f32, 0.85 + 0.2 * k as f32, 0x0dd_ba11 + k as u32 * 7919),
                lp: 0.0,
                place: Place::CENTRE,
                ear: Spatial::default(),
            }),
            rain_lp: 0.0,
            swish_lp: 0.0,
            roll_lp: [0.0; 2],
            grit_lp: 0.0,
            crowd_lp: [0.0; 2],
            crowd_level: 0.0,
            crowd_time: 0.0,
            scrape_lp: [0.0; 2],
            scrape_level: 0.0,
            gust: 0.5,
            gust_target: 0.5,
            gust_wait: 0.0,
            wind_bed: [0.0; 2],
            wind_svf: Svf::default(),
            leaves_lp: 0.0,
            leaves_flutter: 0.0,
            waves: [0.0; 2],
            wave_time: 0.0,
            hum: Svf::default(),
            calls: Vec::with_capacity(MAX_CALLS),
            call_wait: 1.5,
            crickets: [
                Cricket { pitch: 4300.0, rate: 21.0, burst: 1.6, pan: -0.6, time: 0.0, phase: 0.0 },
                Cricket { pitch: 4750.0, rate: 26.0, burst: 2.1, pan: 0.5, time: 0.4, phase: 0.0 },
                Cricket { pitch: 3900.0, rate: 17.0, burst: 1.1, pan: 0.1, time: 0.9, phase: 0.0 },
            ],
            forest: 0.0,
            water: 0.0,
            town: 0.0,
            chuff: 0.0,
            chuff_lp: [0.0; 2],
            bell_time: 0.0,
            train_ear: Spatial::default(),
            diesels: std::array::from_fn(|_| (0.0, [0.0; 2], Svf::default(), Spatial::default())),
            babble: ([[0.0, 4.0, 0.5, 500.0, 0.0], [0.3, 5.3, 0.5, 650.0, 0.0], [0.6, 3.7, 0.5, 420.0, 0.0]], [[Svf::default(); 2]; 3], Spatial::default()),
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
        // held still (or nearly), the throttle revs it freely
        let geared = engine_revs(speed.abs());
        let free = (0.2 + 0.75 * throttle) * (1.0 - speed.abs() / 25.0).clamp(0.0, 1.0);
        let target_rpm = geared.max(free);
        if geared < self.last_target - 0.2 && throttle > 0.3 {
            self.shift = SHIFT_CUT;
        }
        // A change down off the throttle: a blip to match the revs
        if geared > self.last_target + 0.15 && throttle < 0.3 {
            self.blip = BLIP;
        }
        self.last_target = geared;
        // Revs rise quickly under power, fall more slowly
        let ease = if target_rpm > self.rpm { 0.05 } else { 0.09 };
        self.rpm += (target_rpm - self.rpm) * (dt / ease).min(1.0);

        // Power cut for a moment through each change up, and chopped on
        // and off against the limiter flat out in top
        let mut power: f32 = 1.0;
        if self.shift > 0.0 {
            self.shift -= dt;
            power = 0.4;
        }
        if target_rpm > 0.995 && throttle > 0.5 {
            self.limiter_phase = (self.limiter_phase + 14.0 * dt).fract();
            if self.limiter_phase > 0.6 {
                power = power.min(0.3);
            }
        }
        let mut load = throttle * power;
        if self.blip > 0.0 {
            self.blip -= dt;
            load = load.max(0.7);
        }
        // Off the throttle at high revs it misfires and crackles, most just
        // after the lift
        if load > 0.3 {
            self.lifted = 0.0;
        } else {
            self.lifted += dt;
        }
        let overrun = if load < 0.15 { ((self.rpm - 0.45) / 0.4).clamp(0.0, 1.0) * (0.03 + 0.97 * (-self.lifted / 1.2).exp()) } else { 0.0 };
        let rate = self.sample_rate as f32;
        // Louder up the revs and on the power
        let vol = (0.2 + 0.57 * self.rpm) * (0.55 + 0.45 * load);
        let engine = self.engine.sample(self.rpm, load, overrun, rate) * vol * p.get(&p.engine_level);

        // ── Tyres on tarmac: a scrub of rubber as they start to slip, then
        //    a squeal, rubber resonances rung by the slip, wandering in
        //    pitch and fluttering as the tread sticks and lets go ──
        let squeal = if paved && sliding > 3.0 {
            let n = self.noise_n(3);
            let scrub = self.scrub.band(n, 420.0, rate, 1.2) * ((sliding - 3.0) / 10.0).min(1.0) * 0.05;
            let mut sq = 0.0;
            if sliding > 7.0 {
                let intensity = ((sliding - 7.0) / 22.0).min(1.0);
                self.squeal_wander += (self.noise_n(4) * 0.5 - self.squeal_wander) * 0.0004;
                let flutter_target = if self.chance() < 0.004 { self.chance() } else { self.squeal_flutter };
                self.squeal_flutter += (flutter_target - self.squeal_flutter) * 0.02;
                let shift = 1.0 + self.squeal_wander * 0.25 + intensity * 0.12;
                let excite = n * (0.55 + 0.45 * self.squeal_flutter);
                for (k, (f, amp)) in [(820.0, 1.0), (1270.0, 0.7), (1890.0, 0.35)].into_iter().enumerate() {
                    sq += self.squeal[k].band(excite, f * shift, rate, 0.025) * amp;
                }
                sq *= 0.12 * intensity;
            }
            scrub + sq
        } else {
            0.0
        };

        // ── Sliding on gravel: a rumble of loose ground under the tyres and
        //    the stones crunching and spitting ──
        let gravel = if !paved && sliding > 8.0 {
            let intensity = ((sliding - 8.0) / 40.0).min(1.0);
            let mut total = 0.0;
            for i in 0..4 {
                let raw = self.noise_n(i + 1);
                let alpha = 0.08 + (i as f32) * 0.02;
                self.gravel_lp[i] += alpha * (raw - self.gravel_lp[i]);
                total += self.gravel_lp[i];
            }
            let stone = if self.chance() < 0.08 + 0.2 * intensity { self.noise_n(2) } else { 0.0 };
            let crunch = self.gravel_crunch.band(stone, 1900.0, rate, 1.1);
            (total * 0.05 + crunch * 0.12) * intensity
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
            // Further off, duller
            let pitch = p.get(&p.other_pitch[k]);
            let raw = car.engine.sample((car.rpm * pitch).min(1.1), 0.75, 0.0, rate);
            car.lp += (0.12 + 0.88 * level) * (raw - car.lp);
            let v = car.lp * (0.25 + 0.35 * car.rpm) * level;
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

        // ── The world around you: wind always, gusting; leaves in the
        //    woods, waves by the water, a hum in town; creatures calling
        //    by day and night. The engine and the speed drown most of it ──
        let daylight = p.get(&p.daylight);
        let quiet = p.get(&p.quiet);
        let ease = (dt / 1.5).min(1.0);
        self.forest += (p.get(&p.scene[0]) - self.forest) * ease;
        self.water += (p.get(&p.scene[1]) - self.water) * ease;
        self.town += (p.get(&p.scene[2]) - self.town) * ease;
        let hush = 1.0 - (speed.abs() / 70.0).min(1.0) * 0.6;
        // Gusts: the wind wanders toward a new strength every few seconds
        self.gust_wait -= dt;
        if self.gust_wait <= 0.0 {
            self.gust_target = 0.2 + self.chance() * 0.8;
            self.gust_wait = 1.5 + self.chance() * 4.0;
        }
        self.gust += (self.gust_target - self.gust) * (dt / 1.2).min(1.0);
        let n = self.noise_n(5);
        let whoosh = self.wind_svf.band(n, 220.0 + 380.0 * self.gust, rate, 1.3);
        self.wind_bed[0] += 0.01 * (n - self.wind_bed[0]);
        let open_air = 1.0 - 0.5 * self.town;
        let mut bed = (whoosh * 0.22 + self.wind_bed[0] * 0.3) * (0.2 + 0.8 * self.gust) * open_air;
        // Leaves: bright noise fluttering in the gusts
        let n = self.noise_n(1);
        self.leaves_lp += 0.6 * (n - self.leaves_lp);
        self.leaves_flutter += (self.chance() - self.leaves_flutter) * 0.004;
        bed += (n - self.leaves_lp) * self.gust * self.gust * (0.4 + 0.6 * self.leaves_flutter) * 0.07 * self.forest;
        // Waves: a swell of surf every several seconds, hiss as each breaks
        if self.water > 0.01 {
            self.wave_time += dt;
            let swell = (0.5 - 0.5 * (self.wave_time * TAU / 6.5).cos()).powi(2);
            let n = self.noise_n(2);
            self.waves[0] += (0.02 + 0.1 * swell) * (n - self.waves[0]);
            self.waves[1] += 0.01 * (self.waves[0] - self.waves[1]);
            bed += (self.waves[0] - self.waves[1] * 0.5) * (0.15 + swell) * 0.14 * self.water;
        }
        // A town's hum: traffic and life a street or two away
        if self.town > 0.01 {
            let n = self.noise_n(4);
            bed += self.hum.band(n, 160.0, rate, 0.8) * 0.05 * self.town;
        }
        let ambient = bed * hush;
        let (mut amb_l, mut amb_r) = (0.0, 0.0);
        // Calls: birds by day (and gulls by the water), owls and frogs at
        // night; more of them out in the wilds than by the road
        let lively = (0.4 + 0.6 * quiet) * hush;
        self.call_wait -= dt * (0.6 + 0.8 * self.forest + 0.4 * quiet);
        if self.call_wait <= 0.0 && self.calls.len() < MAX_CALLS && lively > 0.05 {
            let night = 1.0 - daylight;
            let pick = self.chance();
            let kind = if night > 0.6 {
                if self.water > 0.3 && pick < 0.6 { CallKind::Frog } else if self.forest > 0.2 && pick < 0.35 { CallKind::Owl } else { CallKind::None }
            } else if self.water > 0.4 && pick < 0.3 {
                CallKind::Gull
            } else if pick < 0.45 {
                CallKind::Chirps
            } else if pick < 0.8 {
                CallKind::Warble
            } else {
                CallKind::Coo
            };
            if kind != CallKind::None {
                let place = Place { pan: self.chance() * 2.0 - 1.0, behind: self.chance() * self.chance() };
                let level = 0.4 + 0.6 * self.chance();
                let pitch = 0.85 + 0.3 * self.chance();
                let seed = self.noise_states[5] | 1;
                self.calls.push(Call { kind, t: 0.0, pitch, phase: 0.0, level, place, ear: Spatial::default(), seed });
            }
            self.call_wait = 0.4 + self.chance() * 2.5;
        }
        self.calls.retain_mut(|c| match c.next(dt) {
            Some(v) => {
                let (l, r) = c.ear.place(v * c.level * lively * 1.6, c.place, rate);
                amb_l += l;
                amb_r += r;
                true
            }
            None => false,
        });
        // Crickets at night, each chirping its own pitch and pace
        let night = (1.0 - daylight * 1.5).max(0.0);
        if night > 0.01 {
            for c in &mut self.crickets {
                c.time += dt;
                c.phase = (c.phase + c.pitch * dt).fract();
                let pulse = (c.time * c.rate).fract();
                let on = (c.time / c.burst).fract() < 0.55;
                let env = if on { (1.0 - pulse * 2.0).max(0.0).powi(2) } else { 0.0 };
                let v = (c.phase * TAU).sin() * env * 0.012 * night * lively;
                let (gl, gr) = pan_gains(c.pan);
                amb_l += v * gl;
                amb_r += v * gr;
            }
        }
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

        // ── Diesels: a thump each firing stroke, a rattle over it ──
        for k in 0..DIESELS {
            let level = p.get(&p.diesel[k][0]);
            if level < 0.002 {
                continue;
            }
            let rate = p.get(&p.diesel[k][1]);
            let n = self.noise_n(3);
            let (phase, lp, rattle, ear) = &mut self.diesels[k];
            *phase += rate * dt;
            let u = phase.fract();
            lp[0] += 0.12 * (n - lp[0]);
            lp[1] += 0.01 * (lp[0] - lp[1]);
            let thump = ((*phase * 0.5).fract() * TAU).sin() * 0.3 + (lp[0] - lp[1]) * 3.0;
            let clatter = rattle.band(n, 1100.0, self.sample_rate as f32, 0.6) * 0.25;
            let v = (thump * (-u * 9.0).exp() + clatter * (0.4 + 0.6 * (-u * 5.0).exp())) * 0.35 * level;
            let (l, r) = ear.place(v, p.get_place(&p.diesel[k][2..4]), self.sample_rate as f32);
            other_l += l;
            other_r += r;
        }
        // ── Chatter: several voices' worth of murmured syllables ──
        let chatter = p.get(&p.chatter[0]);
        if chatter > 0.002 {
            let rate = self.sample_rate as f32;
            let n = [self.noise_n(4), self.noise_n(5), self.noise_n(1)];
            let picks = [self.noise_n(5) * 0.5 + 0.5, self.noise_n(4) * 0.5 + 0.5, self.noise_n(1) * 0.5 + 0.5];
            let (voices, formants, ear) = &mut self.babble;
            let mut v = 0.0;
            for k in 0..3 {
                let st = &mut voices[k];
                // Syllables at each voice's own pace, now and then a pause
                st[0] += dt * st[1];
                if st[0] >= 1.0 {
                    st[0] -= 1.0;
                    let pick = picks[k];
                    st[1] = 3.0 + 4.0 * pick;
                    st[2] = if pick < 0.15 { 0.0 } else { 0.3 + 0.7 * (pick * 7.3).fract() };
                    st[3] = 380.0 + 450.0 * (pick * 3.7).fract();
                }
                st[4] += (st[2] * (st[0] * std::f32::consts::PI).sin() - st[4]) * 0.004;
                let f1 = formants[k][0].band(n[k], st[3], rate, 0.3);
                let f2 = formants[k][1].band(n[k], st[3] * 2.7, rate, 0.35);
                v += (f1 + f2 * 0.45) * st[4];
            }
            let v = v * 0.15 * chatter;
            let (l, r) = ear.place(v, p.get_place(&p.chatter[1..3]), rate);
            other_l += l;
            other_r += r;
        }

        // What the space around you throws back: the car's own sounds and
        // the knocks, not the weather, the wind or the crowd (already
        // a wash of their own)
        let close = engine + squeal + gravel + roll + grind + buzz;
        let centre = close + wind + rain_sound + crowd + ambient;
        let (mut left, mut right) = (centre + other_l + amb_l, centre + other_r + amb_r);
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
        let mut v = Voice::new(sound, gain, Place::CENTRE, 0.0, 0x1234_5679);
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
            let mut v = Voice::new(Sound::Clang, 1.0, Place { pan: 0.0, behind }, 0.0, 77);
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
        write_wav(&path, rate, &samples);
        println!("wrote {path}");
    }

    /// 16-bit stereo `samples` (interleaved) as a WAV file at `path`.
    fn write_wav(path: &str, rate: u32, samples: &[i16]) {
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
        std::fs::write(path, wav).unwrap();
    }

    /// Render `seconds` of the mix to a WAV in `dir`, calling `each` with how
    /// far through (0..1) every 64 frames to drive it.
    fn clip(dir: &str, name: &str, seconds: f32, setup: impl Fn(&AudioParams), mut each: impl FnMut(&AudioParams, f32)) {
        let rate = 44100;
        let params = Arc::new(AudioParams::new());
        let mut source = EngineSource::new(Arc::clone(&params), rate);
        setup(&params);
        // Let eased levels settle before listening, then start the clip's
        // own sounds afresh
        for _ in 0..2 * rate as usize {
            each(&params, 0.0);
            source.next();
            source.next();
        }
        source.voices.clear();
        params.queue.lock().unwrap().clear();
        setup(&params);
        let frames = (seconds * rate as f32) as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for k in 0..frames {
            if k % 64 == 0 {
                each(&params, k as f32 / frames as f32);
            }
            for _ in 0..2 {
                samples.push((source.next().unwrap() * 32767.0) as i16);
            }
        }
        write_wav(&format!("{dir}/{name}.wav"), rate, &samples);
    }

    /// Dev aid: short labelled clips of each kind of sound, to listen to
    /// (and turn into spectrograms) when changing them. Written to
    /// `$STUMBLER_AUDIO_OUT` (default `target/audio-clips`).
    #[test]
    #[ignore]
    fn audio_clips() {
        let dir = std::env::var("STUMBLER_AUDIO_OUT").unwrap_or_else(|_| "target/audio-clips".into());
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.as_str();
        // Engine
        clip(d, "01-engine-idle", 3.0, |p| p.set_throttle(0.0), |_, _| ());
        clip(d, "02-engine-full-throttle-through-gears", 8.0, |_| (), |p, u| {
            p.set_throttle(1.0);
            p.set_speed(175.0 * (u * 1.1).min(1.0).powf(0.85));
        });
        clip(d, "03-engine-lift-and-coast", 4.0, |_| (), |p, u| {
            p.set_throttle(if u < 0.25 { 1.0 } else { 0.0 });
            p.set_speed(if u < 0.25 { 110.0 } else { 110.0 - (u - 0.25) * 80.0 });
            if (u * 4000.0) as u32 == 1001 { p.trigger_pop(); }
        });
        clip(d, "04-engine-part-throttle-cruise", 4.0, |_| (), |p, u| {
            p.set_throttle(0.35 + 0.1 * (u * 12.0).sin());
            p.set_speed(80.0);
        });
        clip(d, "05-engine-brake-and-downshift-into-corner", 4.0, |_| (), |p, u| {
            p.set_throttle(if u < 0.55 { 0.0 } else { 1.0 });
            p.set_speed(if u < 0.55 { 150.0 - u / 0.55 * 100.0 } else { 50.0 + (u - 0.55) * 120.0 });
        });
        // Impacts, light then hard
        for (k, (sound, name)) in [(Sound::Clang, "wall"), (Sound::Crunch, "car"), (Sound::Thunk, "tree"), (Sound::Landing, "landing"), (Sound::Splash, "splash"), (Sound::Pop, "backfire")].into_iter().enumerate() {
            clip(d, &format!("1{k}-hit-{name}-light-then-hard"), 2.6, move |p| {
                p.play(sound, 0.3, Place::CENTRE, 0.2);
                p.play(sound, 1.1, Place::CENTRE, 1.3);
            }, |p, _| p.set_speed(60.0));
        }
        // Tyres and surfaces
        clip(d, "20-skid-tarmac", 3.0, |_| (), |p, u| {
            p.set_speed(90.0 - u * 50.0);
            p.set_sliding(30.0 * (1.0 - u));
            p.set_throttle(0.2);
        });
        clip(d, "21-slide-gravel", 3.0, |p| p.set_paved(false), |p, u| {
            p.set_speed(70.0);
            p.set_sliding(35.0 * (1.0 - u * 0.5));
            p.set_throttle(0.8);
        });
        clip(d, "22-kerb", 2.0, |_| (), |p, u| {
            p.set_speed(80.0);
            p.set_throttle(0.6);
            p.set_rumble(if u > 0.25 && u < 0.7 { 1.0 } else { 0.0 });
        });
        clip(d, "23-rolling-gravel-road", 3.0, |p| p.set_paved(false), |p, _| {
            p.set_speed(60.0);
            p.set_throttle(0.5);
        });
        clip(d, "24-scrape", 2.0, |_| (), |p, _| {
            p.set_speed(50.0);
            p.set_scrape(0.8);
        });
        // Ambience, parked and then driving slowly
        clip(d, "30-ambience-country-day-parked", 10.0, |p| p.set_ambience(1.0, 1.0), |_, _| ());
        clip(d, "31-ambience-country-night-parked", 10.0, |p| p.set_ambience(1.0, 0.0), |_, _| ());
        clip(d, "32-ambience-country-day-driving", 6.0, |p| p.set_ambience(1.0, 1.0), |p, _| {
            p.set_speed(45.0);
            p.set_throttle(0.4);
        });
        clip(d, "33-ambience-by-the-road-day", 6.0, |p| p.set_ambience(0.3, 1.0), |_, _| ());
        clip(d, "37-ambience-woods-day", 10.0, |p| { p.set_ambience(1.0, 1.0); p.set_scene(1.0, 0.0, 0.0); }, |_, _| ());
        clip(d, "38-ambience-woods-night", 10.0, |p| { p.set_ambience(1.0, 0.0); p.set_scene(1.0, 0.0, 0.0); }, |_, _| ());
        clip(d, "39-ambience-shore-day", 10.0, |p| { p.set_ambience(1.0, 1.0); p.set_scene(0.0, 1.0, 0.0); }, |_, _| ());
        clip(d, "3a-ambience-lake-night", 10.0, |p| { p.set_ambience(1.0, 0.0); p.set_scene(0.2, 1.0, 0.0); }, |_, _| ());
        clip(d, "3b-ambience-town", 8.0, |p| { p.set_ambience(0.2, 1.0); p.set_scene(0.0, 0.0, 1.0); }, |_, _| ());
        clip(d, "34-rain", 5.0, |p| p.set_rain(1.0, 1.0), |p, _| {
            p.set_speed(70.0);
            p.set_throttle(0.5);
        });
        clip(d, "35-crowd", 5.0, |p| p.set_crowd(0.8), |_, _| ());
        clip(d, "36-wind-at-top-speed", 4.0, |_| (), |p, _| {
            p.set_speed(170.0);
            p.set_throttle(1.0);
        });
        // Others
        clip(d, "40-rival-passes-on-the-left", 4.0, |_| (), |p, u| {
            p.set_throttle(0.8);
            p.set_speed(130.0);
            let a = u * std::f32::consts::PI;
            let place = Place { pan: -a.sin(), behind: ((0.2 + a.cos()) / 0.8).clamp(0.0, 1.0) };
            let near = 1.0 - (u - 0.5).abs() * 1.6;
            p.set_other(0, 150.0, near.max(0.0).powi(2), if u < 0.5 { 1.08 } else { 0.92 }, place);
        });
        // The world on its own, engine off, to judge the ambience by
        for (name, quiet, day, scene) in [("day", 1.0, 1.0, (0.0, 0.0, 0.0)), ("woods-day", 1.0, 1.0, (1.0, 0.0, 0.0)), ("woods-night", 1.0, 0.0, (1.0, 0.0, 0.0)), ("shore-day", 1.0, 1.0, (0.0, 1.0, 0.0)), ("lake-night", 1.0, 0.0, (0.2, 1.0, 0.0)), ("town", 0.2, 1.0, (0.0, 0.0, 1.0))] {
            clip(d, &format!("50-stem-ambience-{name}"), 10.0, move |p| {
                p.set_engine_level(0.0);
                p.set_ambience(quiet, day);
                p.set_scene(scene.0, scene.1, scene.2);
            }, |_, _| ());
        }
        // The world going about its day, engine off
        let quiet = |p: &AudioParams| { p.set_engine_level(0.0); p.set_ambience(1.0, 1.0); };
        for (k, (name, sound, times)) in [("cow", Sound::Moo, 3), ("sheep", Sound::Baa, 4), ("clock-striking-three", Sound::Toll, 3), ("halyards-at-moorings", Sound::Tink, 8), ("dog", Sound::Bark, 4), ("balloon-burner", Sound::Burner, 1)].into_iter().enumerate() {
            clip(d, &format!("6{k}-{name}"), 7.0, move |p| {
                quiet(p);
                for n in 0..times {
                    let gap = if sound == Sound::Tink { 0.7 } else if sound == Sound::Toll { 2.2 } else { 7.0 / times as f32 * 0.8 };
                    p.play(sound, 0.9, Place { pan: (n as f32 * 0.7).sin() * 0.5, behind: 0.0 }, 0.3 + n as f32 * gap);
                }
            }, |_, _| ());
        }
        clip(d, "66-fishing-boat-passing", 8.0, quiet, |p, u| {
            let near = 1.0 - (u - 0.5).abs() * 1.6;
            p.set_diesel(0, near.max(0.05), 10.0, Place { pan: u * 2.0 - 1.0, behind: 0.0 });
        });
        clip(d, "67-tractor-ploughing", 6.0, quiet, |p, _| p.set_diesel(0, 0.8, 13.0, Place::CENTRE));
        clip(d, "68-busy-street-chatter", 6.0, quiet, |p, _| p.set_chatter(0.8, Place::CENTRE));
        println!("wrote clips to {dir}");
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

    /// The engine on its own at `revs` and `load`: (RMS, brightness as the
    /// share of the signal's change sample to sample).
    fn engine_at(revs: f32, load: f32) -> (f32, f32) {
        let mut e = Engine::new(44100.0, 1.0, 99);
        let (mut sum, mut edge, mut last) = (0.0, 0.0, 0.0);
        for k in 0..44100 {
            let v = e.sample(revs, load, 0.0, 44100.0);
            assert!(v.is_finite());
            if k > 4410 {
                sum += v * v;
                edge += (v - last) * (v - last);
            }
            last = v;
        }
        ((sum / 39690.0).sqrt(), (edge / sum).sqrt())
    }

    #[test]
    fn the_engine_hardens_on_the_power() {
        for revs in [0.3, 0.6, 0.9] {
            let (off_level, off_bright) = engine_at(revs, 0.0);
            let (on_level, on_bright) = engine_at(revs, 1.0);
            assert!(on_level > off_level * 1.5, "revs {revs}: no louder on the power ({on_level} vs {off_level})");
            assert!(on_bright > off_bright, "revs {revs}: no brighter on the power");
        }
    }

    /// A hard knock into a wall is over quickly: no bell-like ring after it.
    #[test]
    fn a_wall_hit_thuds_rather_than_rings() {
        let mut v = Voice::new(Sound::Clang, 1.2, Place::CENTRE, 0.0, 4242);
        let dt = 1.0 / 44100.0;
        let (mut early, mut late) = (0.0, 0.0);
        while let Some(s) = v.next(dt) {
            if v.t < 0.25 { early += s * s } else { late += s * s }
        }
        assert!(late < early * 0.05, "rings on: {late} after vs {early} at first");
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
