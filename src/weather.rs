//! Weather: long dry spells with the odd shower blowing through. Rain wets
//! the road quickly and it dries slowly after; while it's wet everything
//! has less grip (and the AI drives to suit), the sky greys over, spray
//! hangs behind the cars and the rain hisses.
//!
//! STUMBLER_WEATHER=rain or =dry holds it one way (dev aid).

use std::sync::atomic::{AtomicU32, Ordering};

use glam::Vec3;

use crate::geometry::Vertex;
use crate::noise::patchy;

/// How much grip a fully wet surface keeps.
pub const WET_GRIP: f32 = 0.78;
/// Rain comes when the weather noise passes this, and is in full flow by
/// `FULL_RAIN`. Higher means rarer.
const RAIN_FROM: f32 = 0.6;
const FULL_RAIN: f32 = 0.72;
/// Seconds per unit of the weather noise: how long spells last.
const SPELL: f32 = 240.0;
/// How fast the road wets in rain, and dries after (per second).
const WETTING: f32 = 0.25;
const DRYING: f32 = 0.012;

/// Road wetness for everyone (physics, AI), as f32 bits.
static WET: AtomicU32 = AtomicU32::new(0);

/// How wet the roads are, 0 dry to 1 soaked.
pub fn wetness() -> f32 {
    f32::from_bits(WET.load(Ordering::Relaxed))
}

/// Grip multiplier for the current wetness.
pub fn grip() -> f32 {
    1.0 - (1.0 - WET_GRIP) * wetness()
}

pub struct Weather {
    /// How hard it's raining, 0..1.
    pub rain: f32,
    /// How wet the roads are, 0..1 (lags the rain).
    pub wet: f32,
    forced: Option<f32>,
    drops: Vec<Vec3>,
    rng: u32,
}

impl Weather {
    pub fn new() -> Self {
        let forced = match std::env::var("STUMBLER_WEATHER").as_deref() {
            Ok("rain") => Some(1.0),
            Ok("dry") => Some(0.0),
            _ => None,
        };
        let wet: f32 = forced.unwrap_or(0.0);
        WET.store(wet.to_bits(), Ordering::Relaxed);
        Self { rain: wet, wet, forced, drops: Vec::new(), rng: 0x1234_5678 }
    }

    /// Move the weather on to `time` (seconds of play).
    pub fn update(&mut self, time: f32, dt: f32) {
        let target = self.forced.unwrap_or_else(|| {
            let n = patchy(time / SPELL, 37.0, 1.0);
            ((n - RAIN_FROM) / (FULL_RAIN - RAIN_FROM)).clamp(0.0, 1.0)
        });
        self.rain += (target - self.rain) * (dt * 0.3).min(1.0);
        let rate = if self.rain > self.wet { WETTING } else { DRYING };
        self.wet += (self.rain - self.wet).clamp(-rate * dt, rate * dt);
        if self.forced.is_some() {
            self.wet = self.rain;
        }
        WET.store(self.wet.to_bits(), Ordering::Relaxed);
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Raindrops round the camera as streaks, slanted by the wind and by how
    /// fast you're driving through them.
    pub fn rain_mesh(&mut self, cam: Vec3, car_vel: Vec3, dt: f32) -> (Vec<Vertex>, Vec<u16>) {
        const COUNT: usize = 700;
        const BOX: f32 = 45.0;
        const FALL: Vec3 = Vec3::new(4.0, -55.0, 2.0);
        let (mut verts, mut idxs) = (Vec::new(), Vec::new());
        if self.rain < 0.02 {
            self.drops.clear();
            return (verts, idxs);
        }
        let count = (COUNT as f32 * self.rain) as usize;
        while self.drops.len() < count {
            let p = cam + Vec3::new((self.rand() - 0.5) * 2.0 * BOX, self.rand() * 40.0 - 8.0, (self.rand() - 0.5) * 2.0 * BOX);
            self.drops.push(p);
        }
        self.drops.truncate(count);
        // The streak shows the drop's motion as you see it
        let seen = (FALL - car_vel * 0.6).normalize_or_zero();
        for k in 0..self.drops.len() {
            let mut p = self.drops[k] + FALL * dt;
            let rel = p - cam;
            if rel.y < -10.0 || rel.x.abs() > BOX || rel.z.abs() > BOX {
                p = cam + Vec3::new((self.rand() - 0.5) * 2.0 * BOX, 25.0 + self.rand() * 10.0, (self.rand() - 0.5) * 2.0 * BOX);
            }
            self.drops[k] = p;
            let tail = p - seen * 2.2;
            // Wider further off, so distant drops still cover a pixel
            let d = (p - cam).length();
            let across = seen.cross(p - cam).normalize_or_zero() * 0.011 * d.sqrt().max(1.0);
            let base = verts.len() as u16;
            // Fine and faint; ones right by the lens fade out rather than
            // smearing across the view
            let a = 0.4 * self.rain * ((d - 4.0) / 10.0).clamp(0.0, 1.0);
            for (q, alpha) in [(p - across, a), (p + across, a), (tail + across, 0.0), (tail - across, 0.0)] {
                verts.push(Vertex { pos: q.into(), color: [0.85, 0.9, 1.0, alpha] });
            }
            idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        (verts, idxs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showers_come_and_go_and_the_road_dries_slowly() {
        let mut w = Weather { rain: 0.0, wet: 0.0, forced: None, drops: Vec::new(), rng: 1 };
        let (mut rainy, mut max_wet) = (0.0_f32, 0.0_f32);
        let total = 3.0 * 3600.0;
        let dt = 0.5;
        let mut t = 0.0;
        let mut dried_after_rain = false;
        while t < total {
            w.update(t, dt);
            if w.rain > 0.3 {
                rainy += dt;
            }
            max_wet = max_wet.max(w.wet);
            dried_after_rain |= max_wet > 0.8 && w.wet < 0.1;
            t += dt;
        }
        let share = rainy / total;
        println!("raining {:.0}% of three hours", share * 100.0);
        assert!((0.05..0.45).contains(&share), "rain {:.0}% of the time", share * 100.0);
        assert!(max_wet > 0.8 && dried_after_rain, "roads should soak and dry again");
    }
}
