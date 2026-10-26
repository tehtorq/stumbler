//! Collisions. Trees are soft: the car brushes through the branches, losing
//! a little speed and twitching on hard hits, while the tree whips about. Scenery
//! and other cars are solid, resolved with impulses at the contact point so
//! glancing hits spin the car naturally.

use glam::{Quat, Vec2, Vec3};

use crate::physics::CarPhysics;

/// Each car is approximated by two circles along its length.
const CAR_CIRCLE_OFFSET: f32 = 2.4;
const CAR_CIRCLE_RADIUS: f32 = 2.5;
const TREE_RADIUS: f32 = 0.8;
/// Speed lost per second while in a tree: a fifth or so of it going
/// straight through one at speed, so forests slow you rather than stop you.
const TREE_DRAG: f32 = 1.2;
/// Above this speed a tree hit starts to twist the car.
const TREE_SPIN_SPEED: f32 = 70.0;
/// Yaw rate gained per second in a tree, per unit of speed above `TREE_SPIN_SPEED`.
const TREE_SPIN: f32 = 0.4;
const RESTITUTION: f32 = 0.25;
const FRICTION: f32 = 0.3;

/// Scenery cars can't drive through: an upright box footprint (or a round
/// post of radius `half.x` when `round`) from the ground up to `top`.
#[derive(Clone, Copy, Debug)]
pub struct Solid {
    pub centre: Vec2,
    pub half: Vec2,
    pub yaw: f32,
    pub top: f32,
    pub round: bool,
}

impl Solid {
    /// Push-out for a circle at `c` with radius `r`: (normal, depth, contact
    /// point on the footprint edge), if they overlap.
    fn contact(&self, c: Vec2, r: f32) -> Option<(Vec2, f32, Vec2)> {
        if self.round {
            let delta = c - self.centre;
            let dist = delta.length();
            let depth = self.half.x + r - dist;
            let n = if dist > 1e-4 { delta / dist } else { Vec2::X };
            return (depth > 0.0).then(|| (n, depth, self.centre + n * self.half.x));
        }
        // Work in the box's own frame (blocks are rotated by `yaw` about Y)
        let to_local = Quat::from_rotation_y(-self.yaw);
        let to_world = Quat::from_rotation_y(self.yaw);
        let d = to_local * Vec3::new(c.x - self.centre.x, 0.0, c.y - self.centre.y);
        let local = Vec2::new(d.x, d.z);
        let clamped = local.clamp(-self.half, self.half);
        let (n_local, depth, edge) = if clamped != local {
            let delta = local - clamped;
            let dist = delta.length();
            (delta / dist, r - dist, clamped)
        } else {
            // Centre inside the box: out through the nearest face
            let gap = self.half - local.abs();
            if gap.x < gap.y {
                let s = local.x.signum();
                (Vec2::new(s, 0.0), gap.x + r, Vec2::new(s * self.half.x, local.y))
            } else {
                let s = local.y.signum();
                (Vec2::new(0.0, s), gap.y + r, Vec2::new(local.x, s * self.half.y))
            }
        };
        if depth <= 0.0 {
            return None;
        }
        let n = to_world * Vec3::new(n_local.x, 0.0, n_local.y);
        let e = to_world * Vec3::new(edge.x, 0.0, edge.y);
        Some((Vec2::new(n.x, n.z), depth, self.centre + Vec2::new(e.x, e.z)))
    }

    /// Rough reach from the centre, for skipping far-off solids quickly.
    fn reach(&self) -> f32 {
        if self.round { self.half.x } else { self.half.length() }
    }
}

pub struct Hit {
    /// Closing speed along the contact normal.
    pub impact_speed: f32,
    pub point: Vec3,
}

fn circles(car: &CarPhysics) -> [Vec2; 2] {
    let f = car.heading();
    let c = Vec2::new(car.pos.x, car.pos.z);
    let f = Vec2::new(f.x, f.z) * CAR_CIRCLE_OFFSET;
    [c + f, c - f]
}

/// Drive through any trees the car overlaps: drag it down hard and, on fast
/// hits, spin it toward the side the tree caught. Returns the index and hit
/// of the fastest tree contact this step.
pub fn car_vs_trees(car: &mut CarPhysics, trees: &[[f32; 2]], dt: f32) -> Option<(usize, Hit)> {
    let reach = CAR_CIRCLE_OFFSET + CAR_CIRCLE_RADIUS + TREE_RADIUS;
    let centre = Vec2::new(car.pos.x, car.pos.z);
    let mut fastest: Option<(usize, Hit)> = None;

    for (ti, &[tx, tz]) in trees.iter().enumerate() {
        let tree = Vec2::new(tx, tz);
        if tree.distance_squared(centre) > reach * reach {
            continue;
        }
        let touching = circles(car).iter().any(|c| c.distance(tree) < CAR_CIRCLE_RADIUS + TREE_RADIUS);
        if !touching {
            continue;
        }
        let speed = Vec2::new(car.vel.x, car.vel.z).length();
        let keep = (-TREE_DRAG * dt).exp();
        car.vel.x *= keep;
        car.vel.z *= keep;
        if speed > TREE_SPIN_SPEED {
            // The side that catches the tree is held back, swinging the nose toward it
            let across = car.rot * Vec3::X;
            let side = if (tree - centre).dot(Vec2::new(across.x, across.z)) > 0.0 { -1.0 } else { 1.0 };
            car.ang_vel += car.up() * side * (speed - TREE_SPIN_SPEED) * TREE_SPIN * dt;
        }
        let point = Vec3::new(tx, car.pos.y, tz);
        if fastest.as_ref().is_none_or(|(_, h)| speed > h.impact_speed) {
            fastest = Some((ti, Hit { impact_speed: speed, point }));
        }
    }
    fastest
}

/// Collide the car with buildings, boulders and other scenery it isn't
/// above. Returns the hardest impact.
pub fn car_vs_solids(car: &mut CarPhysics, solids: &[Solid]) -> Option<Hit> {
    /// The underside of the car, relative to its centre.
    const CLEARANCE: f32 = 1.2;
    let mut hardest: Option<Hit> = None;
    for solid in solids {
        let centre = Vec2::new(car.pos.x, car.pos.z);
        let reach = solid.reach() + CAR_CIRCLE_OFFSET + CAR_CIRCLE_RADIUS;
        if car.pos.y - CLEARANCE > solid.top || solid.centre.distance_squared(centre) > reach * reach {
            continue;
        }
        for c in circles(car) {
            let Some((n, depth, edge)) = solid.contact(c, CAR_CIRCLE_RADIUS) else { continue };
            let normal = Vec3::new(n.x, 0.0, n.y);
            car.pos += normal * depth;
            let point = Vec3::new(edge.x, car.pos.y, edge.y);
            if let Some(impact_speed) = resolve_static(car, normal, point) {
                if hardest.as_ref().is_none_or(|h| impact_speed > h.impact_speed) {
                    hardest = Some(Hit { impact_speed, point });
                }
            }
        }
    }
    hardest
}

/// Collide the car with solids moving at their own velocity (the train's
/// cars): it gets shoved along with them. Returns the hardest impact.
pub fn car_vs_moving(car: &mut CarPhysics, solids: &[(Solid, Vec3)]) -> Option<Hit> {
    let mut hardest: Option<Hit> = None;
    for (solid, vel) in solids {
        let centre = Vec2::new(car.pos.x, car.pos.z);
        let reach = solid.reach() + CAR_CIRCLE_OFFSET + CAR_CIRCLE_RADIUS;
        // Not when passing over or under it
        if (car.pos.y - (solid.top - 5.0)).abs() > 7.0 || solid.centre.distance_squared(centre) > reach * reach {
            continue;
        }
        for c in circles(car) {
            let Some((n, depth, edge)) = solid.contact(c, CAR_CIRCLE_RADIUS) else { continue };
            let normal = Vec3::new(n.x, 0.0, n.y);
            car.pos += normal * depth;
            let point = Vec3::new(edge.x, car.pos.y, edge.y);
            if let Some(impact_speed) = resolve_against(car, normal, point, *vel) {
                if hardest.as_ref().is_none_or(|h| impact_speed > h.impact_speed) {
                    hardest = Some(Hit { impact_speed, point });
                }
            }
        }
    }
    hardest
}

/// Impulse against an immovable object. Returns closing speed if approaching.
fn resolve_static(car: &mut CarPhysics, normal: Vec3, point: Vec3) -> Option<f32> {
    resolve_against(car, normal, point, Vec3::ZERO)
}

/// Impulse against an unstoppable object moving at `surface`. Returns
/// closing speed if approaching.
fn resolve_against(car: &mut CarPhysics, normal: Vec3, point: Vec3, surface: Vec3) -> Option<f32> {
    let v = car.point_velocity(point) - surface;
    let vn = v.dot(normal);
    if vn >= 0.0 {
        return None;
    }
    let jn = -(1.0 + RESTITUTION) * vn / car.inverse_mass_at(point, normal);
    let vt = v - normal * vn;
    let t = vt.normalize_or_zero();
    let jt_max = vt.length() / car.inverse_mass_at(point, t).max(1e-6);
    car.apply_impulse(normal * jn - t * jt_max.min(FRICTION * jn), point);
    Some(-vn)
}

/// Collide two cars. Returns the hit if they were closing.
pub fn car_vs_car(a: &mut CarPhysics, b: &mut CarPhysics) -> Option<Hit> {
    let reach = 2.0 * (CAR_CIRCLE_OFFSET + CAR_CIRCLE_RADIUS);
    let ca = Vec2::new(a.pos.x, a.pos.z);
    let cb = Vec2::new(b.pos.x, b.pos.z);
    if ca.distance_squared(cb) > reach * reach || (a.pos.y - b.pos.y).abs() > 4.0 {
        return None;
    }

    // Deepest overlapping circle pair
    let mut best: Option<(f32, Vec2, Vec2)> = None;
    for pa in circles(a) {
        for pb in circles(b) {
            let delta = pa - pb;
            let dist = delta.length();
            let pen = 2.0 * CAR_CIRCLE_RADIUS - dist;
            if pen > 0.0 && dist > 1e-4 && best.is_none_or(|(p, _, _)| pen > p) {
                best = Some((pen, delta / dist, pb + delta * 0.5));
            }
        }
    }
    let (pen, n, mid) = best?;
    let normal = Vec3::new(n.x, 0.0, n.y);
    a.pos += normal * pen * 0.5;
    b.pos -= normal * pen * 0.5;

    let point = Vec3::new(mid.x, (a.pos.y + b.pos.y) * 0.5, mid.y);
    let rel = a.point_velocity(point) - b.point_velocity(point);
    let vn = rel.dot(normal);
    if vn >= 0.0 {
        return None;
    }
    let jn = -(1.0 + RESTITUTION) * vn / (a.inverse_mass_at(point, normal) + b.inverse_mass_at(point, normal));
    let vt = rel - normal * vn;
    let t = vt.normalize_or_zero();
    let jt_max = vt.length() / (a.inverse_mass_at(point, t) + b.inverse_mass_at(point, t)).max(1e-6);
    let impulse = normal * jn - t * jt_max.min(FRICTION * jn);
    a.apply_impulse(impulse, point);
    b.apply_impulse(-impulse, point);
    Some(Hit { impact_speed: -vn, point })
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

    /// Drive at `speed` toward a tree `lateral` units along the car's local X
    /// (positive = the side steering left turns toward). Returns the car after
    /// two seconds, its speed just after the tree, and the contact speed.
    fn crash(lateral: f32, speed: f32) -> (CarPhysics, f32, f32) {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.vel = Vec3::new(0.0, 0.0, -speed); // facing -Z
        let trees = [[lateral, -30.0]];
        let (mut contact, mut after) = (0.0_f32, None);
        for _ in 0..120 {
            car.update(&Flat, 1.0 / 60.0, CarInput::default());
            match car_vs_trees(&mut car, &trees, 1.0 / 60.0) {
                Some((_, hit)) => contact = contact.max(hit.impact_speed),
                None if contact > 0.0 && after.is_none() => after = Some(car.vel.length()),
                None => {}
            }
        }
        (car, after.unwrap_or(0.0), contact)
    }

    #[test]
    fn tree_slows_the_car_without_bouncing_it_back() {
        let (car, after, contact) = crash(0.0, 60.0);
        assert!(contact > 40.0, "no contact registered: {contact}");
        assert!(after < contact * 0.9 && after > contact * 0.6, "should lose some speed, not most: {contact} -> {after}");
        assert!(car.vel.z <= 0.5, "bounced back off the tree: {}", car.vel);
        assert!(car.up().y > 0.9, "tree hit flipped the car");
    }

    #[test]
    fn slow_bump_into_a_tree_isnt_a_wall() {
        // Creep into a trunk: the car pushes through, slowed, and isn't flung back
        let (car, after, contact) = crash(0.0, 15.0);
        assert!(contact > 0.0, "missed the tree");
        assert!(car.vel.z <= 0.0 && after < contact, "unexpected: {} ({contact} -> {after})", car.vel);
    }

    #[test]
    fn fast_tree_hit_twists_the_car_toward_the_tree() {
        for side in [-1.0_f32, 1.0] {
            let (car, _, contact) = crash(side * 3.0, 110.0);
            assert!(contact > 90.0, "missed the tree");
            // Clipping it on local +X swings the nose that way, which is a left turn (yaw down)
            let turn = -car.yaw() * side;
            println!("tree at {side}: turned {:.2} rad, speed after {:.0}", turn, car.vel.length());
            // A twitch toward it, not a spin
            assert!(turn > 0.05 && turn < 0.6, "tree at {side}: yaw {} should turn a little toward it", car.yaw());
            assert!(car.up().y > 0.9);
        }
    }

    /// Drive at 60 toward a solid 30 units ahead; returns the car and hardest impact.
    fn crash_into(solid: Solid, height: f32) -> (CarPhysics, f32) {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.pos.y += height;
        car.vel = Vec3::new(0.0, 0.0, -60.0);
        let mut hardest = 0.0_f32;
        for _ in 0..120 {
            car.update(&Flat, 1.0 / 60.0, CarInput::default());
            if let Some(hit) = car_vs_solids(&mut car, &[solid]) {
                hardest = hardest.max(hit.impact_speed);
            }
        }
        (car, hardest)
    }

    #[test]
    fn house_wall_stops_the_car() {
        // A house turned 30 degrees, square across the car's path
        let house = Solid { centre: Vec2::new(0.0, -36.0), half: Vec2::new(6.0, 8.0), yaw: 0.5, top: 8.0, round: false };
        let (car, impact) = crash_into(house, 0.0);
        assert!(impact > 30.0, "no impact registered: {impact}");
        assert!(house.contact(Vec2::new(car.pos.x, car.pos.z), 0.0).is_none(), "ended up inside the house");
        assert!(car.vel.z > -10.0, "still charging forward: {}", car.vel);
    }

    #[test]
    fn round_post_deflects_the_car() {
        let mast = Solid { centre: Vec2::new(2.5, -30.0), half: Vec2::splat(3.0), yaw: 0.0, top: 70.0, round: true };
        let (car, impact) = crash_into(mast, 0.0);
        assert!(impact > 5.0, "missed the mast");
        assert!(car.pos.z.abs() < 30.0 || car.pos.x < -3.0, "drove through the mast: {}", car.pos);
    }

    #[test]
    fn jumps_clear_low_scenery() {
        let boulder = Solid { centre: Vec2::new(0.0, -10.0), half: Vec2::splat(3.0), yaw: 0.0, top: 2.0, round: true };
        let (_, impact) = crash_into(boulder, 6.0);
        assert_eq!(impact, 0.0, "hit a boulder it flew over");
    }

    #[test]
    fn rear_end_shares_momentum() {
        let mut a = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        let mut b = CarPhysics::new_on(&Flat, Vec3::new(0.0, 0.0, -20.0), 0.0);
        a.vel = Vec3::new(0.0, 0.0, -50.0);
        let mut hit = false;
        for _ in 0..60 {
            a.update(&Flat, 1.0 / 60.0, CarInput::default());
            b.update(&Flat, 1.0 / 60.0, CarInput::default());
            hit |= car_vs_car(&mut a, &mut b).is_some();
        }
        assert!(hit);
        assert!(b.vel.z < -15.0, "car in front should be shoved forward: {}", b.vel);
        assert!(a.pos.z > b.pos.z, "cars passed through each other");
    }
}
