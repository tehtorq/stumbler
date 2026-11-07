//! A biplane touring the island: it flies unhurriedly from one place you
//! haven't found yet to the next, circles over each for a while, and moves
//! on once you've caught up (or it's tired of waiting). Follow it, and it
//! shows you the way to somewhere new.

use glam::{Mat4, Quat, Vec2, Vec3, vec3};

use crate::geometry::Vertex;
use crate::props::Builder;
use crate::terrain::{terrain_height, WATER_LEVEL};

/// Cruising speed: a good deal slower than a car at full tilt, so it can be
/// kept up with across country.
const SPEED: f32 = 55.0;
/// How fast it can come round, in radians a second.
const TURN_RATE: f32 = 0.32;
/// Height kept above the highest ground ahead.
const CLEARANCE: f32 = 85.0;
const CLIMB_RATE: f32 = 14.0;
/// The circle it flies over a place it's arrived at.
const ORBIT_RADIUS: f32 = 190.0;
/// It moves on once you're this close to the place it's circling...
const CAUGHT_UP: f32 = 300.0;
/// ...after giving you at least this long to look about...
const MIN_CIRCLE: f32 = 6.0;
/// ...or once it has waited this long regardless.
const MAX_CIRCLE: f32 = 55.0;
/// Further from you than this, it heads for somewhere near you next rather
/// than near itself, so it never strays far.
const STRAY: f32 = 2600.0;
/// The next place is at least this far on, so it's a journey worth following.
const MIN_HOP: f32 = 450.0;

/// A place the plane might fly to.
pub struct Destination {
    pub pos: Vec2,
    pub name: String,
}

enum Phase {
    /// On the way to the target.
    Cruise,
    /// Circling over it, for this long so far.
    Circle(f32),
}

pub struct Biplane {
    pub pos: Vec3,
    /// Unit heading over the ground.
    heading: Vec2,
    /// Current climb (+) or descent (-), units a second.
    climb: f32,
    /// Smoothed turn rate, for banking into turns.
    turning: f32,
    target: Option<Destination>,
    phase: Phase,
    /// Places already shown this session, so it doesn't go back.
    visited: Vec<String>,
    /// Propeller angle.
    prop: f32,
}

impl Biplane {
    /// Starts out ahead of `player` (facing `forward`), crossing their view,
    /// so it's seen from the start.
    pub fn new(player: Vec2, forward: Vec2) -> Self {
        let forward = forward.try_normalize().unwrap_or(Vec2::NEG_Y);
        let right = Vec2::new(-forward.y, forward.x);
        let start = player + forward * 420.0 - right * 260.0;
        let ground = terrain_height(start.x, start.y).max(WATER_LEVEL);
        Self {
            pos: vec3(start.x, ground + CLEARANCE, start.y),
            heading: right,
            climb: 0.0,
            turning: 0.0,
            target: None,
            phase: Phase::Cruise,
            visited: Vec::new(),
            prop: 0.0,
        }
    }

    pub fn xz(&self) -> Vec2 {
        Vec2::new(self.pos.x, self.pos.z)
    }

    /// Fly on. `unfound` lists the places not yet discovered (asked for only
    /// when choosing where next); `fallback` is somewhere to go once
    /// everything's been found.
    pub fn update(&mut self, dt: f32, player: Vec2, unfound: impl FnOnce() -> Vec<Destination>, fallback: impl FnOnce() -> Vec<Destination>) {
        self.prop = (self.prop + dt * 40.0) % std::f32::consts::TAU;
        let here = self.xz();

        // Where next, when it has nowhere to go or has finished circling
        let leave = match (&self.phase, &self.target) {
            (_, None) => true,
            (Phase::Circle(t), Some(target)) => {
                (*t > MIN_CIRCLE && player.distance(target.pos) < CAUGHT_UP) || *t > MAX_CIRCLE
            }
            _ => false,
        };
        if leave {
            if let Some(done) = self.target.take() {
                self.visited.push(done.name);
            }
            let anchor = if here.distance(player) > STRAY { player } else { here };
            let mut choices: Vec<Destination> = unfound()
                .into_iter()
                .filter(|d| !self.visited.contains(&d.name) && d.pos.distance(here) > MIN_HOP)
                .collect();
            if choices.is_empty() {
                // Everything found: tour somewhere at a sensible distance, in turn
                choices = fallback().into_iter().filter(|d| !self.visited.contains(&d.name) && (MIN_HOP..STRAY).contains(&d.pos.distance(here))).collect();
                if choices.is_empty() {
                    self.visited.clear();
                }
            }
            self.target = choices.into_iter().min_by(|a, b| a.pos.distance_squared(anchor).total_cmp(&b.pos.distance_squared(anchor)));
            self.phase = Phase::Cruise;
        }

        // Steer: straight for the target, then round it
        let desired = match (&self.target, &self.phase) {
            (Some(target), Phase::Cruise) => {
                let to = target.pos - here;
                if to.length() < ORBIT_RADIUS + 40.0 {
                    self.phase = Phase::Circle(0.0);
                }
                to.normalize_or_zero()
            }
            (Some(target), Phase::Circle(t)) => {
                self.phase = Phase::Circle(t + dt);
                let out = here - target.pos;
                let d = out.length().max(1.0);
                let radial = out / d;
                // Anticlockwise round it, drawn in or out to the circle
                let tangent = Vec2::new(-radial.y, radial.x);
                (tangent - radial * ((d - ORBIT_RADIUS) / ORBIT_RADIUS).clamp(-1.0, 1.0)).normalize_or_zero()
            }
            _ => self.heading,
        };
        let right = Vec2::new(-self.heading.y, self.heading.x);
        let off = right.dot(desired).atan2(self.heading.dot(desired));
        let rate = (off * 1.5).clamp(-TURN_RATE, TURN_RATE);
        let angle = rate * dt;
        let (s, c) = angle.sin_cos();
        self.heading = Vec2::new(self.heading.x * c - self.heading.y * s, self.heading.x * s + self.heading.y * c).normalize();
        self.turning += (rate - self.turning) * (dt * 2.0).min(1.0);

        // Hold height over the highest ground coming up
        let ahead = [0.0, 120.0, 260.0, 420.0]
            .iter()
            .map(|&k| {
                let p = here + self.heading * k;
                terrain_height(p.x, p.y).max(WATER_LEVEL)
            })
            .fold(f32::MIN, f32::max);
        let want = ((ahead + CLEARANCE - self.pos.y) * 0.4).clamp(-CLIMB_RATE * 0.6, CLIMB_RATE);
        self.climb += (want - self.climb) * (dt * 1.2).min(1.0);

        let step = self.heading * SPEED * dt;
        self.pos += vec3(step.x, self.climb * dt, step.y);
    }

    /// Model matrix: nose along the heading, banked into turns, pitched with the climb.
    pub fn transform(&self) -> Mat4 {
        let yaw = (-self.heading.x).atan2(-self.heading.y);
        let pitch = (self.climb / SPEED).atan();
        // Turning toward +X (its right) dips the right wing
        let bank = (self.turning * 1.6).clamp(-0.6, 0.6);
        let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch) * Quat::from_rotation_z(-bank);
        Mat4::from_scale_rotation_translation(Vec3::splat(SCALE), rot, self.pos)
    }

    /// The propeller's place on the nose, spinning.
    pub fn prop_transform(&self) -> Mat4 {
        self.transform() * Mat4::from_translation(vec3(0.0, 0.0, NOSE - 0.5)) * Mat4::from_rotation_z(self.prop)
    }
}

/// The model is built to about real size; the world is drawn larger than
/// life, so the plane is too.
const SCALE: f32 = 1.5;

/// Where the nose ends, along -Z.
const NOSE: f32 = -5.6;

const CREAM: [f32; 4] = [0.93, 0.88, 0.74, 1.0];
const RED: [f32; 4] = [0.78, 0.12, 0.1, 1.0];
const DARK: [f32; 4] = [0.14, 0.14, 0.16, 1.0];
const WOOD: [f32; 4] = [0.45, 0.3, 0.18, 1.0];

/// The meshes: the plane itself (front at -Z, right wing at +X), its
/// wingtip lights (glowing after dark), and the propeller.
pub struct Meshes {
    pub body: (Vec<Vertex>, Vec<u32>),
    pub lights: (Vec<Vertex>, Vec<u32>),
    pub prop: (Vec<Vertex>, Vec<u32>),
}

pub fn build_meshes() -> Meshes {
    let mut b = Builder::default();
    // Fuselage, tapering to the tail, with the engine cowling and a spinner
    b.limb(vec3(0.0, 0.0, -4.8), vec3(0.0, 0.2, 6.2), 1.15, 0.35, 8, 0.0, CREAM);
    b.limb(vec3(0.0, 0.0, NOSE), vec3(0.0, 0.0, -4.8), 1.05, 1.2, 8, 0.0, DARK);
    b.limb(vec3(0.0, 0.0, NOSE - 0.7), vec3(0.0, 0.0, NOSE), 0.15, 0.45, 6, 0.0, RED);
    // A red stripe down each side, and the open cockpit with the pilot's head
    b.block(vec3(0.0, -0.15, 0.5), 1.08, 0.3, 4.0, 0.0, RED, RED);
    b.block(vec3(0.0, 0.95, 0.9), 0.55, 0.12, 0.8, 0.0, DARK, DARK);
    b.blob(vec3(0.0, 1.35, 1.0), 0.42, 1.0, [0.45, 0.32, 0.22, 1.0], 3.0);
    b.block(vec3(0.0, 0.95, -0.2), 0.6, 0.5, 0.06, 0.0, [0.6, 0.7, 0.8, 1.0], [0.6, 0.7, 0.8, 1.0]);
    // Two wings, the upper set a little forward of the lower
    b.block(vec3(0.0, -1.0, -1.6), 8.0, 0.24, 1.35, 0.0, RED, RED);
    b.block(vec3(0.0, 2.1, -2.0), 8.6, 0.24, 1.4, 0.0, RED, RED);
    // Struts between them, and the cabane over the fuselage
    for x in [-5.6, 5.6] {
        for z in [-2.6, -0.8] {
            b.limb(vec3(x, -0.8, z), vec3(x, 2.1, z - 0.4), 0.1, 0.1, 4, 0.0, WOOD);
        }
    }
    for x in [-0.8, 0.8] {
        b.limb(vec3(x * 0.9, 0.8, -2.4), vec3(x, 2.1, -2.6), 0.08, 0.08, 4, 0.0, WOOD);
    }
    // Tailplane and fin
    b.block(vec3(0.0, 0.15, 5.6), 3.0, 0.16, 0.85, 0.0, RED, RED);
    b.block(vec3(0.0, 0.3, 5.8), 0.12, 2.3, 0.75, 0.0, CREAM, CREAM);
    // Undercarriage: legs and two wheels
    for x in [-1.3, 1.3] {
        b.limb(vec3(x * 0.6, -0.8, -3.2), vec3(x, -2.5, -3.0), 0.09, 0.09, 4, 0.0, DARK);
        b.limb(vec3(x - 0.15, -2.6, -3.0), vec3(x + 0.15, -2.6, -3.0), 0.65, 0.65, 8, 0.0, DARK);
    }
    let body = (b.verts, b.idxs);

    // Wingtip lights: red to port, green to starboard
    let mut l = Builder::default();
    l.block(vec3(-8.65, 2.08, -2.0), 0.12, 0.28, 0.25, 0.0, [1.4, 0.15, 0.1, 1.0], [1.4, 0.15, 0.1, 1.0]);
    l.block(vec3(8.65, 2.08, -2.0), 0.12, 0.28, 0.25, 0.0, [0.15, 1.4, 0.3, 1.0], [0.15, 1.4, 0.3, 1.0]);
    let lights = (l.verts, l.idxs);

    // Two-bladed propeller, about its own centre
    let mut p = Builder::default();
    p.block(vec3(0.0, -2.1, 0.0), 0.22, 4.2, 0.07, 0.0, WOOD, WOOD);
    p.block(vec3(0.0, -0.25, 0.0), 0.3, 0.5, 0.12, 0.0, DARK, DARK);
    let prop = (p.verts, p.idxs);

    Meshes { body, lights, prop }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(x: f32, z: f32, name: &str) -> Destination {
        Destination { pos: Vec2::new(x, z), name: name.into() }
    }

    #[test]
    fn flies_to_the_nearest_unfound_place_circles_it_and_moves_on_when_you_arrive() {
        let mut plane = Biplane::new(Vec2::ZERO, Vec2::NEG_Y);
        let places = || vec![place(3000.0, 0.0, "FAR"), place(1200.0, 900.0, "NEAR")];
        let start = plane.xz();
        let mut player = Vec2::ZERO;
        // Out to the nearer place, without the player
        let mut circled = 0.0;
        for _ in 0..(120 * 60) {
            plane.update(1.0 / 60.0, player, places, Vec::new);
            if let Phase::Circle(t) = plane.phase {
                circled = t;
                break;
            }
        }
        assert!(circled >= 0.0 && plane.target.as_ref().is_some_and(|t| t.name == "NEAR"), "never reached NEAR from {start}");
        let over = plane.xz().distance(Vec2::new(1200.0, 900.0));
        assert!(over < ORBIT_RADIUS + 60.0, "circling {over} away");
        // It waits while you're away...
        for _ in 0..(20 * 60) {
            plane.update(1.0 / 60.0, player, places, Vec::new);
        }
        assert!(matches!(plane.phase, Phase::Circle(_)), "left before you came");
        let d = plane.xz().distance(Vec2::new(1200.0, 900.0));
        assert!((ORBIT_RADIUS * 0.6..ORBIT_RADIUS * 1.5).contains(&d), "orbit drifted to {d}");
        // ...and once you turn up, it heads on to the next
        player = Vec2::new(1150.0, 880.0);
        for _ in 0..60 {
            plane.update(1.0 / 60.0, player, places, Vec::new);
        }
        assert!(plane.target.as_ref().is_some_and(|t| t.name == "FAR"), "didn't move on");
    }

    #[test]
    fn keeps_clear_of_the_ground() {
        let mut plane = Biplane::new(Vec2::ZERO, Vec2::NEG_Y);
        let places = || vec![place(4000.0, 3000.0, "A"), place(-3000.0, 2500.0, "B")];
        for _ in 0..(150 * 60) {
            plane.update(1.0 / 60.0, Vec2::ZERO, places, Vec::new);
            let ground = terrain_height(plane.pos.x, plane.pos.z).max(WATER_LEVEL);
            assert!(plane.pos.y > ground + 20.0, "down to {} over ground at {ground}", plane.pos.y);
        }
    }
}
