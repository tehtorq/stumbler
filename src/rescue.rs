//! Fishing sunk cars out of lakes: after a moment fully under water, the car
//! is set back down on firm dry ground near where it went in, facing away
//! from the water, and dropped onto its wheels.

use glam::Vec2;

use crate::physics::{CarPhysics, WorldGround};
use crate::terrain;

/// Fully under water this long, the car is fished out and set down on the shore.
pub const SINK_TIME: f32 = 1.8;
/// It's dropped onto the shore from this high, landing like a recovered flip.
pub const SHORE_DROP: f32 = 5.0;

/// What happened this step.
#[derive(Default)]
pub struct Step {
    /// The car just hit the water at this speed.
    pub splash: Option<f32>,
    /// The car is under water (for bubbles).
    pub sinking: bool,
    /// The car was just set back on the shore.
    pub rescued: bool,
}

#[derive(Default)]
pub struct WaterRescue {
    /// Where the car last had its wheels on dry land.
    last_dry: Option<Vec2>,
    /// How long it has been fully under water.
    sunk_time: f32,
    /// Last step's `submerged`, to catch the moment of the splash.
    was_wet: f32,
}

impl WaterRescue {
    pub fn update(&mut self, car: &mut CarPhysics, dt: f32) -> Step {
        let mut step = Step::default();
        let me = Vec2::new(car.pos.x, car.pos.z);
        let wet = car.submerged;
        if wet > 0.05 && self.was_wet < 0.05 && car.vel.length() > 5.0 {
            step.splash = Some(car.vel.length());
        }
        self.was_wet = wet;
        if wet <= 0.0 && car.grounded_wheels() >= 3 && terrain::raw_height(me.x, me.y) > terrain::WATER_LEVEL + 0.5 {
            self.last_dry = Some(me);
        }
        if wet > 0.6 {
            step.sinking = true;
            self.sunk_time += dt;
        } else {
            self.sunk_time = 0.0;
        }
        if self.sunk_time > SINK_TIME {
            let (spot, facing) = shore_spot(self.last_dry, me);
            car.place(&WorldGround, spot.x, spot.y, (-facing.x).atan2(-facing.y));
            car.pos.y += SHORE_DROP;
            self.sunk_time = 0.0;
            self.was_wet = 0.0;
            step.rescued = true;
        }
        step
    }
}

/// Firm, nearly level dry ground near `last_dry`, heading away from the water
/// at `sunk`; else the nearest such ground round `sunk`. Also the way to face.
pub fn shore_spot(last_dry: Option<Vec2>, sunk: Vec2) -> (Vec2, Vec2) {
    // Well clear of the water and nearly level, so the car doesn't roll back in
    let dry = |p: Vec2| {
        let e = 5.0;
        let h = |x: f32, z: f32| terrain::solid_height(x, z);
        let slope = (h(p.x + e, p.y) - h(p.x - e, p.y)).abs().max((h(p.x, p.y + e) - h(p.x, p.y - e)).abs()) / (2.0 * e);
        terrain::raw_height(p.x, p.y) > terrain::WATER_LEVEL + 4.0 && slope < 0.12
    };
    if let Some(from) = last_dry {
        let away = (from - sunk).try_normalize().unwrap_or(Vec2::Y);
        if let Some(p) = (0..60).map(|k| from + away * (k as f32 * 4.0)).find(|&p| dry(p)) {
            return (p, away);
        }
    }
    for ring in 1..80 {
        let r = ring as f32 * 10.0;
        for k in 0..24 {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let p = sunk + Vec2::new(a.cos(), a.sin()) * r;
            if dry(p) {
                return (p, (p - sunk).normalize_or_zero());
            }
        }
    }
    (sunk, Vec2::Y)
}
