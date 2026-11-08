//! Small wildlife after dark: foxes trotting the fields at dusk and through
//! the night, pausing to sniff and dashing off if the car comes close;
//! hedgehogs crossing the country roads at night, freezing in the
//! headlights and curling into a ball if the car comes right up; and now and
//! then an owl swooping low across the road ahead.

use std::collections::HashMap;
use std::sync::OnceLock;

use glam::{Vec2, Vec3, vec2, vec3};

use crate::geometry::Vertex;
use crate::props::PERSON_SCALE;
use crate::road::{self, PathKind};
use crate::terrain::{self, terrain_height, WATER_LEVEL};
use crate::townsfolk::Mesh;

const FOXES: usize = 3;
const HEDGEHOGS: usize = 3;
const FORGET: f32 = 400.0;
const DRAW: f32 = 350.0;
const CELL: f32 = 200.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Fox,
    Hedgehog,
    Owl,
}

struct Critter {
    kind: Kind,
    pos: Vec2,
    heading: f32,
    /// Height above the ground (the owl).
    lift: f32,
    /// Paused (sniffing, or frozen) for so long more.
    still: f32,
    /// Running off (fox), or curled up (hedgehog), for so long more.
    alarm: f32,
    phase: f32,
}

/// Road samples by cell, for the hedgehogs to cross at.
fn roads() -> &'static HashMap<(i32, i32), Vec<(usize, usize)>> {
    static GRID: OnceLock<HashMap<(i32, i32), Vec<(usize, usize)>>> = OnceLock::new();
    GRID.get_or_init(|| {
        let mut g: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        if let Some(net) = road::network() {
            for (pi, path) in net.paths.iter().enumerate().filter(|(_, p)| p.kind == PathKind::Road) {
                for (i, q) in path.points.iter().enumerate().step_by(8) {
                    g.entry(((q.x / CELL).floor() as i32, (q.y / CELL).floor() as i32)).or_default().push((pi, i));
                }
            }
        }
        g
    })
}

#[derive(Default)]
pub struct Nightlife {
    critters: Vec<Critter>,
    rng: u32,
    next_owl: f32,
}

impl Nightlife {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223) | 1;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `heading`: which way the car points (its headlights), `night` 0 by day .. 1 dark.
    pub fn update(&mut self, player: Vec3, player_vel: Vec3, heading: Vec2, clock: f32, dt: f32) {
        let me = vec2(player.x, player.z);
        let speed = vec2(player_vel.x, player_vel.z).length();
        let h = clock * 24.0;
        let dusk_on = h >= 18.5 || h < 6.0;
        let dark = h >= 21.0 || h < 5.0;
        self.critters.retain(|c| c.pos.distance(me) < FORGET && !(c.kind == Kind::Owl && c.alarm <= 0.0));
        if !dusk_on {
            self.critters.retain(|c| c.pos.distance(me) < 120.0 && c.kind != Kind::Owl);
        }
        let count = |s: &Self, k: Kind| s.critters.iter().filter(|c| c.kind == k).count();
        // Foxes on open ground round about
        if dusk_on && count(self, Kind::Fox) < FOXES {
            let p = me + Vec2::from_angle(self.rand() * 6.28) * (70.0 + 150.0 * self.rand());
            let raw = terrain::raw_height(p.x, p.y);
            let off_road = road::network().and_then(|n| n.nearest(p.x, p.y)).is_none_or(|s| s.dist > s.half_width() + 8.0);
            if raw > WATER_LEVEL + 2.0 && raw < 120.0 && off_road && !crate::towns::occupied(p, 20.0) {
                let heading = self.rand() * 6.28;
                let phase = self.rand() * 10.0;
                self.critters.push(Critter { kind: Kind::Fox, pos: p, heading, lift: 0.0, still: 0.0, alarm: 0.0, phase });
            }
        }
        // Hedgehogs setting off across the roads
        if dark && count(self, Kind::Hedgehog) < HEDGEHOGS {
            let q = me + Vec2::from_angle(self.rand() * 6.28) * (40.0 + 140.0 * self.rand());
            if let (Some(net), Some(cands)) = (road::network(), roads().get(&((q.x / CELL).floor() as i32, (q.y / CELL).floor() as i32))) {
                let (pi, i) = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
                let path = &net.paths[pi];
                let t = path.tangent(i);
                let side = if self.rand() < 0.5 { 1.0 } else { -1.0 };
                let start = path.points[i] + t.perp() * side * (path.kind.half_width() + 2.0);
                if !crate::towns::occupied(start, 30.0) {
                    let across = -t.perp() * side;
                    self.critters.push(Critter { kind: Kind::Hedgehog, pos: start, heading: across.x.atan2(across.y), lift: 0.0, still: 0.0, alarm: 0.0, phase: 0.0 });
                }
            }
        }
        // An owl, now and then, low across the road ahead
        self.next_owl -= dt;
        if dark && speed > 8.0 && self.next_owl <= 0.0 {
            self.next_owl = 15.0 + 25.0 * self.rand();
            let side = if self.rand() < 0.5 { 1.0 } else { -1.0 };
            let start = me + heading * (45.0 + 20.0 * self.rand()) + heading.perp() * side * 30.0;
            let across = -heading.perp() * side;
            let lift = 7.0 + 4.0 * self.rand();
            self.critters.push(Critter { kind: Kind::Owl, pos: start, heading: across.x.atan2(across.y), lift, still: 0.0, alarm: 6.0, phase: 0.0 });
        }
        self.update_critters(me, speed, heading, dt);
        // Hedgehogs that have made it over trundle off out of mind
        self.critters.retain(|c| c.kind != Kind::Hedgehog || road::network().and_then(|n| n.nearest(c.pos.x, c.pos.y)).is_some_and(|s| s.dist < s.half_width() + 6.0));
    }

    /// Each critter goes about its business, minding the car at `me`.
    fn update_critters(&mut self, me: Vec2, speed: f32, heading: Vec2, dt: f32) {
        for c in &mut self.critters {
            let fwd = vec2(c.heading.sin(), c.heading.cos());
            let to_car = me - c.pos;
            let near = to_car.length();
            let lit = near < 45.0 && heading.dot(-to_car / near.max(0.1)) > 0.85;
            let mut pace = 0.0;
            match c.kind {
                Kind::Fox => {
                    if near < 30.0 && c.alarm <= 0.0 {
                        c.alarm = 3.0;
                        let away = -to_car / near.max(0.1);
                        c.heading = away.x.atan2(away.y);
                    }
                    if c.alarm > 0.0 {
                        c.alarm -= dt;
                        pace = 13.0;
                    } else if c.still > 0.0 {
                        c.still -= dt;
                    } else {
                        pace = 3.0;
                        c.heading += (c.phase * 0.7).sin() * dt * 0.8;
                        if (c.phase * 0.31).sin() > 0.97 {
                            c.still = 2.5;
                        }
                    }
                }
                Kind::Hedgehog => {
                    // Freeze in the headlights; curl up if the car comes right up
                    if near < 6.0 {
                        c.alarm = 3.0;
                    }
                    if c.alarm > 0.0 {
                        c.alarm -= dt * if near > 10.0 { 1.0 } else { 0.0 };
                    } else if lit && speed > 1.0 {
                        c.still = 1.5;
                    }
                    if c.still > 0.0 {
                        c.still -= dt;
                    } else if c.alarm <= 0.0 {
                        pace = 0.7;
                    }
                }
                Kind::Owl => {
                    c.alarm -= dt;
                    pace = 11.0;
                    c.lift += (c.phase * 1.3).sin() * dt * 0.6;
                }
            }
            c.pos += fwd * pace * dt;
            c.phase += dt * (1.0 + pace);
        }
    }

    /// Every critter near enough to see.
    pub fn build_mesh(&self, camera: Vec3) -> (Vec<Vertex>, Vec<u16>) {
        let cam = vec2(camera.x, camera.z);
        let mut m = Mesh::default();
        for c in self.critters.iter().filter(|c| c.pos.distance(cam) < DRAW) {
            let dir = vec2(c.heading.sin(), c.heading.cos());
            let ground = match c.kind {
                Kind::Hedgehog => road::network().and_then(|n| n.nearest(c.pos.x, c.pos.y)).filter(|s| s.dist < s.half_width()).map_or(terrain_height(c.pos.x, c.pos.y), |s| s.height.max(terrain_height(c.pos.x, c.pos.y))),
                _ => terrain_height(c.pos.x, c.pos.y),
            };
            let at = vec3(c.pos.x, ground + c.lift, c.pos.y);
            match c.kind {
                Kind::Fox => fox(&mut m, at, dir, c.phase, c.alarm > 0.0 || c.still <= 0.0),
                Kind::Hedgehog => hedgehog(&mut m, at, dir, c.alarm > 0.0, c.phase),
                Kind::Owl => owl(&mut m, at, dir, c.phase),
            }
        }
        (m.verts, m.idxs)
    }
}

fn frame(dir: Vec2) -> (Vec3, Vec3) {
    (vec3(dir.x, 0.0, dir.y), vec3(dir.y, 0.0, -dir.x))
}

/// A red fox, brush out behind, white-tipped.
fn fox(m: &mut Mesh, at: Vec3, dir: Vec2, phase: f32, moving: bool) {
    let k = PERSON_SCALE;
    let (f, r) = frame(dir);
    let red = [0.8, 0.38, 0.12];
    let black = [0.12, 0.1, 0.1];
    let white = [0.92, 0.9, 0.85];
    for (a, s) in [(0.22, -1.0), (0.22, 1.0), (-0.22, -1.0), (-0.22, 1.0)] {
        let swing = if moving { (phase * 3.0 + a * 12.0 * s).sin() * 0.12 } else { 0.0 };
        let hip = at + (f * a + r * s * 0.07 + Vec3::Y * 0.3) * k;
        m.limb(hip, at + (f * (a + swing) + r * s * 0.07) * k, 0.025 * k, black);
    }
    let body = at + Vec3::Y * 0.36 * k;
    m.box_at(body, r, f, vec3(0.1, 0.09, 0.28) * k, red, red);
    let head = body + f * 0.33 * k + Vec3::Y * 0.06 * k;
    m.box_at(head, r, f, vec3(0.07, 0.07, 0.08) * k, red, red);
    m.box_at(head + f * 0.1 * k - Vec3::Y * 0.02 * k, r, f, vec3(0.035, 0.03, 0.05) * k, white, red);
    for s in [-1.0, 1.0] {
        let ear = head + Vec3::Y * 0.07 * k + r * s * 0.04 * k;
        m.tri(ear - r * 0.02 * k, ear + r * 0.02 * k, ear + Vec3::Y * 0.07 * k, black);
    }
    let tail = body - f * 0.28 * k;
    let tip = tail - f * 0.32 * k - Vec3::Y * 0.05 * k + r * (phase * 2.0).sin() * 0.04 * k;
    m.limb(tail, tip, 0.06 * k, red);
    m.box_at(tip - f * 0.04 * k, r, f, Vec3::splat(0.05) * k, white, white);
}

/// A hedgehog: a dome of spines, or a tight ball when frightened.
fn hedgehog(m: &mut Mesh, at: Vec3, dir: Vec2, curled: bool, phase: f32) {
    let k = PERSON_SCALE;
    let (f, r) = frame(dir);
    let spines = [0.35, 0.27, 0.18];
    let tips = [0.6, 0.52, 0.4];
    if curled {
        m.box_at(at + Vec3::Y * 0.1 * k, r, f, Vec3::splat(0.1) * k, spines, tips);
        m.box_at(at + Vec3::Y * 0.1 * k, f, r, vec3(0.08, 0.11, 0.08) * k, spines, tips);
        return;
    }
    let bob = (phase * 8.0).sin().abs() * 0.01 * k;
    m.box_at(at + Vec3::Y * (0.08 * k + bob), r, f, vec3(0.09, 0.07, 0.13) * k, spines, tips);
    m.box_at(at + Vec3::Y * (0.12 * k + bob), r, f, vec3(0.07, 0.04, 0.1) * k, tips, tips);
    m.box_at(at + f * 0.14 * k + Vec3::Y * 0.05 * k, r, f, vec3(0.04, 0.035, 0.05) * k, [0.6, 0.48, 0.38], [0.6, 0.48, 0.38]);
}

/// A barn owl gliding, pale wings beating slowly.
fn owl(m: &mut Mesh, at: Vec3, dir: Vec2, phase: f32) {
    let k = PERSON_SCALE;
    let (f, r) = frame(dir);
    let pale = [0.9, 0.85, 0.75];
    let buff = [0.8, 0.65, 0.4];
    m.box_at(at, r, f, vec3(0.1, 0.09, 0.18) * k, buff, buff);
    m.box_at(at + f * 0.17 * k + Vec3::Y * 0.04 * k, r, f, Vec3::splat(0.08) * k, pale, buff);
    let flap = (phase * 0.6).sin() * 0.3;
    for s in [-1.0, 1.0] {
        let root = at + Vec3::Y * 0.03 * k;
        let tip = root + (r * s * 0.55 + Vec3::Y * flap * 0.5) * k;
        m.tri(root + f * 0.12 * k, root - f * 0.12 * k, tip, if s > 0.0 { pale } else { buff });
        m.tri(root - f * 0.12 * k, root + f * 0.12 * k, tip, if s > 0.0 { pale } else { buff });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hedgehogs_freeze_in_the_headlights() {
        let hog = || Critter { kind: Kind::Hedgehog, pos: Vec2::ZERO, heading: std::f32::consts::FRAC_PI_2, lift: 0.0, still: 0.0, alarm: 0.0, phase: 0.0 };
        let mut n = Nightlife { critters: vec![hog()], rng: 1, next_owl: 99.0 };
        // At noon (nothing new turns up), a car 20 back, lights on it, coming on
        let step = |n: &mut Nightlife, car: Vec2| {
            let before = n.critters[0].pos;
            n.update_critters(car, 5.0, Vec2::X, 0.1);
            n.critters[0].pos.distance(before)
        };
        assert!(step(&mut n, vec2(-20.0, 0.0)) < 1e-4, "it should freeze in the lights");
        assert!(step(&mut n, vec2(-3.0, 0.0)) < 1e-4);
        assert!(n.critters[0].alarm > 0.0, "it should curl up with the car on it");
        // The car gone: before long it uncurls and goes on
        let mut moved = 0.0;
        for _ in 0..100 {
            moved += step(&mut n, vec2(-300.0, 0.0));
        }
        assert!(moved > 0.5, "it should go on once the car's gone");
    }
}
