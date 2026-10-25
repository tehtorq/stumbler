use rodio::{MixerDeviceSink, DeviceSinkBuilder, Source};
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub struct AudioParams {
    speed: AtomicU32,
    sliding: AtomicU32,
    impact: AtomicU32,
    chime: AtomicU32,  // lap complete chime timer
    pickup: AtomicU32, // collectible pickup ping
    throttle: AtomicU32,
    /// Whether the tyres are on tarmac (squeal) or not (gravel), 0 or 1.
    paved: AtomicU32,
    /// Kerb and rough-ground judder, 0..1.
    rumble: AtomicU32,
    /// The nearest other car: its speed, how loud it is here (0..1) and
    /// its pitch shift from closing in or pulling away.
    other_speed: AtomicU32,
    other_level: AtomicU32,
    other_pitch: AtomicU32,
    /// How hard it's raining, and how wet the road is.
    rain: AtomicU32,
    wet: AtomicU32,
}

impl AudioParams {
    fn new() -> Self {
        Self {
            speed: AtomicU32::new(0),
            sliding: AtomicU32::new(0),
            impact: AtomicU32::new(0),
            chime: AtomicU32::new(0),
            pickup: AtomicU32::new(0),
            throttle: AtomicU32::new(0),
            paved: AtomicU32::new(1.0_f32.to_bits()),
            rumble: AtomicU32::new(0),
            other_speed: AtomicU32::new(0),
            other_level: AtomicU32::new(0),
            other_pitch: AtomicU32::new(1.0_f32.to_bits()),
            rain: AtomicU32::new(0),
            wet: AtomicU32::new(0),
        }
    }

    pub fn set_rain(&self, rain: f32, wet: f32) {
        self.rain.store(rain.to_bits(), Ordering::Relaxed);
        self.wet.store(wet.to_bits(), Ordering::Relaxed);
    }

    /// The nearest other car: `speed`, `level` (0 silent .. 1 right there)
    /// and `pitch` (1 = as is; above while it closes in, below as it goes).
    pub fn set_other(&self, speed: f32, level: f32, pitch: f32) {
        self.other_speed.store(speed.to_bits(), Ordering::Relaxed);
        self.other_level.store(level.to_bits(), Ordering::Relaxed);
        self.other_pitch.store(pitch.to_bits(), Ordering::Relaxed);
    }

    pub fn set_throttle(&self, v: f32) {
        self.throttle.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn set_paved(&self, paved: bool) {
        self.paved.store(if paved { 1.0_f32 } else { 0.0 }.to_bits(), Ordering::Relaxed);
    }

    pub fn set_rumble(&self, v: f32) {
        self.rumble.store(v.to_bits(), Ordering::Relaxed);
    }

    fn get(&self, v: &AtomicU32) -> f32 {
        f32::from_bits(v.load(Ordering::Relaxed))
    }

    pub fn set_speed(&self, v: f32) {
        self.speed.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn set_sliding(&self, v: f32) {
        self.sliding.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn trigger_impact(&self) {
        self.impact.store((0.15_f32).to_bits(), Ordering::Relaxed);
    }

    pub fn trigger_chime(&self) {
        self.chime.store((0.6_f32).to_bits(), Ordering::Relaxed);
    }

    pub fn trigger_pickup(&self) {
        self.pickup.store((0.15_f32).to_bits(), Ordering::Relaxed);
    }

    fn get_speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }

    fn get_sliding(&self) -> f32 {
        f32::from_bits(self.sliding.load(Ordering::Relaxed))
    }

    fn get_impact(&self) -> f32 {
        f32::from_bits(self.impact.load(Ordering::Relaxed))
    }

    fn get_chime(&self) -> f32 {
        f32::from_bits(self.chime.load(Ordering::Relaxed))
    }

    fn get_pickup(&self) -> f32 {
        f32::from_bits(self.pickup.load(Ordering::Relaxed))
    }

    fn decay_impact(&self, dt: f32) {
        let v = self.get_impact();
        if v > 0.0 {
            self.impact.store((v - dt).max(0.0).to_bits(), Ordering::Relaxed);
        }
        let c = self.get_chime();
        if c > 0.0 {
            self.chime.store((c - dt).max(0.0).to_bits(), Ordering::Relaxed);
        }
        let p = self.get_pickup();
        if p > 0.0 {
            self.pickup.store((p - dt).max(0.0).to_bits(), Ordering::Relaxed);
        }
    }
}

/// Top speed (units/s) in each gear.
const GEAR_TOPS: [f32; 6] = [38.0, 64.0, 90.0, 115.0, 140.0, 175.0];

/// Engine revs (0..1) at `speed`: each gear runs from a little above idle
/// up to the red line, then the next one picks up lower down.
fn engine_revs(speed: f32) -> f32 {
    let gear = GEAR_TOPS.iter().position(|&top| speed < top).unwrap_or(GEAR_TOPS.len() - 1);
    let low = if gear == 0 { 0.0 } else { GEAR_TOPS[gear - 1] * 0.75 };
    let top = GEAR_TOPS[gear];
    0.2 + 0.8 * ((speed - low) / (top - low)).clamp(0.0, 1.0)
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
    // Impact resonance
    impact_phase: f32,
    /// Engine revs (0..1), eased toward what the speed and gear give.
    rpm: f32,
    squeal_phase: f32,
    squeal_lp: f32,
    wind_lp: f32,
    rumble_phase: f32,
    other_rpm: f32,
    other_phase: f32,
    other_lp: f32,
    rain_lp: f32,
    swish_lp: f32,
    // Noise sources (offset for variety)
    noise_states: [u32; 5],
}

impl EngineSource {
    fn new(params: Arc<AudioParams>, sample_rate: u32) -> Self {
        Self {
            params, sample_rate,
            phase_fire: 0.0, phase_exhaust: 0.0, phase_sub: 0.0,
            lp_state: 0.0,
            gravel_lp: [0.0; 4],
            impact_phase: 0.0,
            rpm: 0.2,
            squeal_phase: 0.0,
            squeal_lp: 0.0,
            wind_lp: 0.0,
            rumble_phase: 0.0,
            other_rpm: 0.2,
            other_phase: 0.0,
            other_lp: 0.0,
            rain_lp: 0.0,
            swish_lp: 0.0,
            noise_states: [12345, 67890, 24680, 13579, 97531],
        }
    }

    fn noise_n(&mut self, idx: usize) -> f32 {
        self.noise_states[idx] = self.noise_states[idx].wrapping_mul(1103515245).wrapping_add(12345);
        (self.noise_states[idx] >> 16) as f32 / 32768.0 - 1.0
    }

    fn noise(&mut self) -> f32 {
        self.noise_n(0)
    }
}

impl Iterator for EngineSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let dt = 1.0 / self.sample_rate as f32;
        let speed = self.params.get_speed();
        let sliding = self.params.get_sliding();
        let impact = self.params.get_impact();
        self.params.decay_impact(dt);

        let throttle = self.params.get(&self.params.throttle);
        let paved = self.params.get(&self.params.paved) > 0.5;
        let rumble = self.params.get(&self.params.rumble);

        // ── Engine ──
        // Revs climb through the gears and drop back at each change up;
        // eased so a shift is quick but not a click
        let target_rpm = engine_revs(speed.abs());
        self.rpm += (target_rpm - self.rpm) * (dt / 0.06).min(1.0);
        let rpm_t = self.rpm;
        // Firing frequency: 4-cylinder, 4-stroke = 2 fires per rev
        // At idle: 800rpm → 800/60*2 = 26.7Hz
        // At max:  7000rpm → 7000/60*2 = 233Hz
        let fire_freq = 26.0 + rpm_t * 207.0;

        // Volume: louder with RPM, and on the power
        let vol = (0.05 + rpm_t * 0.10) * (0.65 + 0.35 * throttle);

        // 1. Firing pulse — square-ish wave with duty cycle ~30%
        //    This gives the rhythmic "throb" of each cylinder firing
        self.phase_fire += fire_freq * dt;
        if self.phase_fire > 1.0 { self.phase_fire -= 1.0; }
        let pulse = if self.phase_fire < 0.3 { 1.0 } else { -0.3 };

        // 2. Exhaust resonance — sine at 1.5x firing freq
        //    The exhaust pipe resonates, giving the "note" character
        self.phase_exhaust += fire_freq * 1.5 * dt;
        if self.phase_exhaust > 1.0 { self.phase_exhaust -= 1.0; }
        let exhaust = (self.phase_exhaust * std::f32::consts::TAU).sin() * 0.6;

        // 3. Sub-bass rumble — half the firing frequency
        //    Deep throb you feel in your chest
        self.phase_sub += fire_freq * 0.5 * dt;
        if self.phase_sub > 1.0 { self.phase_sub -= 1.0; }
        let sub = (self.phase_sub * std::f32::consts::TAU).sin() * 0.4;

        // 4. Mechanical noise — filtered random noise, less at high RPM
        let mech = self.noise() * 0.15 * (1.0 - rpm_t * 0.6);

        // 5. Combine and apply amplitude modulation (throb envelope)
        let throb_env = 0.7 + 0.3 * (self.phase_fire * std::f32::consts::TAU).sin();
        let raw_engine = (pulse + exhaust + sub + mech) * vol * throb_env;

        // 6. Simple one-pole low-pass filter to warm it up
        //    Cutoff rises with RPM: muffled at idle, opens up at speed
        let lp_alpha = (0.02 + rpm_t * 0.15).min(0.99);
        self.lp_state += lp_alpha * (raw_engine - self.lp_state);
        let engine = self.lp_state;

        // ── Tyre squeal on tarmac: a wavering tone with a scratchy edge ──
        let squeal = if paved && sliding > 6.0 {
            let intensity = ((sliding - 6.0) / 25.0).min(1.0);
            let wobble = self.noise_n(4) * 60.0;
            self.squeal_phase += (980.0 + intensity * 180.0 + wobble) * dt;
            if self.squeal_phase > 1.0 { self.squeal_phase -= 1.0; }
            let tone = (self.squeal_phase * std::f32::consts::TAU).sin();
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

        // ── Wind — a rush that builds with speed, brighter as it does ──
        let air = (speed.abs() / 170.0).min(1.2);
        let n = self.noise();
        self.wind_lp += (0.05 + air * 0.25) * (n - self.wind_lp);
        let wind = self.wind_lp * 0.08 * air * air;

        // ── The nearest other car: a thinner, buzzier engine of its own ──
        let other_level = self.params.get(&self.params.other_level);
        let other = if other_level > 0.001 {
            let target = engine_revs(self.params.get(&self.params.other_speed).abs());
            self.other_rpm += (target - self.other_rpm) * (dt / 0.08).min(1.0);
            let freq = (30.0 + self.other_rpm * 220.0) * self.params.get(&self.params.other_pitch);
            self.other_phase += freq * dt;
            if self.other_phase > 1.0 { self.other_phase -= 1.0; }
            let raw = if self.other_phase < 0.35 { 1.0 } else { -0.4 } + (self.other_phase * 3.0 * std::f32::consts::TAU).sin() * 0.4;
            self.other_lp += (0.08 + self.other_rpm * 0.2) * (raw - self.other_lp);
            self.other_lp * 0.09 * other_level
        } else {
            0.0
        };

        // ── Rain: a soft hiss, and wet tyres swishing at speed ──
        let rain = self.params.get(&self.params.rain);
        let wet = self.params.get(&self.params.wet);
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

        // ── Kerbs and rough ground: a low buzz ──
        self.rumble_phase += (38.0 + speed.abs() * 0.15) * dt;
        if self.rumble_phase > 1.0 { self.rumble_phase -= 1.0; }
        let buzz = if self.rumble_phase < 0.5 { 1.0 } else { -1.0 } * 0.05 * rumble;

        // ── Impact — metallic clang ──
        //    Two decaying sine tones at inharmonic frequencies = metallic ring
        let thud = if impact > 0.0 {
            let env = impact / 0.15;
            self.impact_phase += dt;
            let t = self.impact_phase;
            // Inharmonic frequencies give metallic character
            // (non-integer ratios = bell/metal, integer ratios = musical)
            let ring1 = (t * 180.0 * std::f32::consts::TAU).sin() * 0.15;
            let ring2 = (t * 267.0 * std::f32::consts::TAU).sin() * 0.10;
            let ring3 = (t * 412.0 * std::f32::consts::TAU).sin() * 0.06;
            // Low body thump underneath
            let body = (t * 25.0 * std::f32::consts::TAU).sin() * 0.15;
            (ring1 + ring2 + ring3 + body) * env * env
        } else {
            self.impact_phase = 0.0;
            0.0
        };

        // ── Lap chime — ascending major triad ──
        let chime_val = self.params.get_chime();
        let chime = if chime_val > 0.0 {
            let t = 0.6 - chime_val; // time since trigger
            let env = (chime_val / 0.6).powi(2); // fade out
            // Three notes: C5, E5, G5 (523, 659, 784 Hz)
            let note = if t < 0.15 { 523.0 }
                       else if t < 0.3 { 659.0 }
                       else { 784.0 };
            (t * note * std::f32::consts::TAU).sin() * 0.15 * env
        } else { 0.0 };

        // ── Pickup ping — short bright note ──
        let pickup_val = self.params.get_pickup();
        let pickup = if pickup_val > 0.0 {
            let t = 0.15 - pickup_val;
            let env = (pickup_val / 0.15).powi(2);
            (t * 1047.0 * std::f32::consts::TAU).sin() * 0.2 * env // C6 = 1047 Hz
        } else { 0.0 };

        Some((engine + other + squeal + gravel + wind + rain_sound + buzz + thud + chime + pickup).clamp(-0.8, 0.8))
    }
}

impl Source for EngineSource {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> NonZero<u16> { NonZero::new(1).unwrap() }
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
            params.set_other(speed * 0.9, 1.0, 1.2);
            params.set_rain(1.0, 1.0);
            params.trigger_impact();
            let mut loudest = 0.0_f32;
            for _ in 0..44100 / 2 {
                let v = source.next().unwrap();
                assert!(v.is_finite());
                loudest = loudest.max(v.abs());
            }
            assert!(loudest > 0.01 && loudest <= 0.8, "level {loudest} at speed {speed}");
        }
    }
}
