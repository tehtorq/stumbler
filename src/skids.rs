//! Skid marks: black rubber laid on the tarmac (and dark ruts torn in the
//! grass and dirt) wherever tyres slide, lock or spin up, fading over a
//! couple of minutes. Each wheel lays a continuous strip while it slides.

use std::collections::{HashMap, VecDeque};

use glam::{Vec3, vec3};

use crate::geometry::Vertex;
use crate::physics::CarPhysics;
use crate::terrain::terrain_height;

/// Oldest marks give way to new ones past this many strip pieces.
const MAX_PIECES: usize = 3000;
/// Seconds a mark takes to fade away.
const LIFE: f32 = 120.0;
/// A strip grows a new piece each time the wheel has moved this far.
const PIECE_LENGTH: f32 = 1.2;
const HALF_WIDTH: f32 = 0.65;
/// Grip used (see `CarPhysics::wheel_scrub`) at which a tyre starts to mark,
/// and how far past it the mark reaches full darkness.
const MARK_FROM: f32 = 0.98;
const MARK_RANGE: f32 = 0.5;
/// Sit just above the road surface and its painted lines; higher on rough
/// ground, where the terrain mesh's facets stand proud of the true surface.
const LIFT: f32 = 0.11;
const RUT_LIFT: f32 = 0.35;

/// How much of a strip's darkness carries into its next piece once the tyre
/// stops sliding.
const FADE_PER_PIECE: f32 = 0.82;

const RUBBER: [f32; 3] = [0.05, 0.05, 0.06];
const RUT: [f32; 3] = [0.2, 0.15, 0.1];

struct Piece {
    /// Left and right edges at the start and end of the piece.
    corners: [Vec3; 4],
    /// Darkness at each end (0..1).
    strength: [f32; 2],
    color: [f32; 3],
    born: f32,
}

/// Where a wheel's current strip ends.
struct Strip {
    left: Vec3,
    right: Vec3,
    at: Vec3,
    strength: f32,
}

#[derive(Default)]
pub struct Skids {
    pieces: VecDeque<Piece>,
    strips: HashMap<(usize, usize), Strip>,
    /// Whether the marks changed since the mesh was last built.
    pub changed: bool,
}

impl Skids {
    /// Extend or end the strips under car `id`'s wheels. `on_road` says
    /// whether a spot is tarmac (rubber marks) or not (ruts).
    pub fn update(&mut self, id: usize, car: &CarPhysics, on_road: impl Fn(Vec3) -> f32, time: f32) {
        let across = car.rot * Vec3::X;
        let across = vec3(across.x, 0.0, across.z).normalize_or_zero();
        let moving = car.vel.length() > 4.0;
        for wheel in 0..4 {
            let key = (id, wheel);
            let fresh = ((car.wheel_scrub[wheel] - MARK_FROM) / MARK_RANGE).clamp(0.0, 1.0);
            // A tyre hovering at its limit marks on and off; let a strip
            // fade rather than break, so it reads as one continuous mark
            let strength = self.strips.get(&key).map_or(fresh, |s| fresh.max(s.strength * FADE_PER_PIECE));
            if !car.wheel_contact[wheel] || !moving || strength < 0.05 {
                self.strips.remove(&key);
                continue;
            }
            let at = car.wheel_contact_pos[wheel];
            let paved = on_road(at) > 0.5;
            let lift = if paved { LIFT } else { RUT_LIFT };
            let edge = |side: f32| {
                let p = at + across * side * HALF_WIDTH;
                vec3(p.x, terrain_height(p.x, p.z) + lift, p.z)
            };
            match self.strips.get_mut(&key) {
                Some(strip) if strip.at.distance(at) >= PIECE_LENGTH => {
                    // Too far (a jump, a reset): start afresh
                    if strip.at.distance(at) > PIECE_LENGTH * 6.0 {
                        *strip = Strip { left: edge(-1.0), right: edge(1.0), at, strength };
                        continue;
                    }
                    let (left, right) = (edge(-1.0), edge(1.0));
                    let color = if paved { RUBBER } else { RUT };
                    self.pieces.push_back(Piece { corners: [strip.left, strip.right, right, left], strength: [strip.strength, strength], color, born: time });
                    if self.pieces.len() > MAX_PIECES {
                        self.pieces.pop_front();
                    }
                    *strip = Strip { left, right, at, strength };
                    self.changed = true;
                }
                Some(_) => {}
                None => {
                    self.strips.insert(key, Strip { left: edge(-1.0), right: edge(1.0), at, strength });
                }
            }
        }
    }

    /// Forget a car's strips (it left, or was teleported).
    pub fn lift(&mut self, id: usize) {
        self.strips.retain(|&(car, _), _| car != id);
    }

    /// Drop marks that have faded out.
    pub fn expire(&mut self, time: f32) {
        while self.pieces.front().is_some_and(|p| time - p.born > LIFE) {
            self.pieces.pop_front();
            self.changed = true;
        }
    }

    /// The marks within `range` of `centre`, as see-through quads.
    pub fn mesh(&self, centre: Vec3, range: f32, time: f32) -> (Vec<Vertex>, Vec<u16>) {
        let mut verts = Vec::new();
        let mut idxs = Vec::new();
        for piece in &self.pieces {
            if piece.corners[0].distance_squared(centre) > range * range || verts.len() + 4 > u16::MAX as usize {
                continue;
            }
            let fade = 1.0 - ((time - piece.born) / LIFE).clamp(0.0, 1.0);
            let base = verts.len() as u16;
            for (k, corner) in piece.corners.iter().enumerate() {
                let alpha = 0.6 * fade * piece.strength[k / 2];
                let c = piece.color;
                verts.push(Vertex { pos: (*corner).into(), color: [c[0], c[1], c[2], alpha] });
            }
            idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        (verts, idxs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{CarInput, Ground};

    struct Flat;
    impl Ground for Flat {
        fn height(&self, _x: f32, _z: f32) -> f32 { 0.0 }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn normal(&self, _x: f32, _z: f32) -> Vec3 { Vec3::Y }
    }

    fn marks_laid(input: CarInput) -> usize {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.vel = car.forward() * 70.0;
        let mut skids = Skids::default();
        for k in 0..120 {
            car.update(&Flat, 1.0 / 60.0, input);
            skids.update(0, &car, |_| 1.0, k as f32 / 60.0);
        }
        skids.pieces.len()
    }

    #[test]
    fn sliding_tyres_mark_the_road_and_cruising_ones_dont() {
        let cruising = marks_laid(CarInput { throttle: 0.5, ..Default::default() });
        let sliding = marks_laid(CarInput { throttle: 1.0, steer: 1.0, handbrake: true, ..Default::default() });
        let braking = marks_laid(CarInput { brake: 1.0, ..Default::default() });
        assert_eq!(cruising, 0, "marks from just driving along");
        assert!(sliding > 40, "a handbrake slide left only {sliding} pieces");
        assert!(braking > 10, "stamping on the brakes left only {braking} pieces");
    }
}
