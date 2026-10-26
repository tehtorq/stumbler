//! Arcade rigid-body car physics.
//!
//! The body is a rigid body (position, velocity, orientation, angular velocity).
//! Each wheel is a ray-cast spring/damper; tyre forces are applied at the
//! contact patches, so pitch, roll, yaw and drifting all come out of the same
//! forces rather than being animated. Rear-wheel drive, friction circle per tyre.
//!
//! Car-local axes: -Z forward, +Y up, X across (mounts with -X are on the
//! left as seen from the driver's seat... of a left-handed world).

use glam::{Mat3, Mat4, Quat, Vec3, vec3};

use crate::car::{WHEEL_MOUNTS, WHEEL_RADIUS};

// --- Chassis ---
pub(crate) const GRAVITY: f32 = 45.0;
const CAR_MASS: f32 = 40.0;
/// Inertia about local X (pitch), Y (yaw), Z (roll). Inflated vs a solid box for stability.
const INERTIA: Vec3 = vec3(450.0, 420.0, 220.0);
const ANGULAR_DAMPING: f32 = 0.6;
const DRAG: f32 = 0.06;
const DOWNFORCE: f32 = 0.03;

// --- Suspension ---
const SPRING_REST: f32 = 1.5;
const SPRING_K: f32 = 900.0;
const SPRING_DAMP: f32 = 90.0;
const BUMP_STOP_K: f32 = 9000.0;
const ANTI_ROLL: f32 = 500.0;

// --- Tyres ---
const TYRE_MU: f32 = 1.3;
/// Lateral force per unit of sideways slip velocity, per unit of load share.
const CORNER_STIFFNESS: f32 = 0.6;
/// Grip retained once a tyre is sliding (drift).
const SLIDE_FACTOR: f32 = 0.8;
const HANDBRAKE_GRIP: f32 = 0.5;
/// Share of available grip traction control lets the engine use.
const TRACTION_LIMIT: f32 = 0.95;
const ROLL_RESIST: f32 = 0.3;
/// Fraction of the way from contact patch up to the centre of mass at which
/// tyre forces act (0 = realistic). Keeps body roll and squat lively but sane.
const LATERAL_LIFT: f32 = 0.6;
const LONGITUDINAL_LIFT: f32 = 0.85;

// --- Drivetrain ---
const ENGINE_FORCE: f32 = 2400.0;
const BRAKE_FORCE: f32 = 3200.0;
/// Share of the braking done by each front wheel (the rest split between
/// the rears): biased forward so the tail stays put under heavy braking.
const FRONT_BRAKE_SHARE: f32 = 0.3;
const REVERSE_MAX: f32 = 40.0;
const MAX_STEER: f32 = 0.55;
const STEER_RATE: f32 = 3.0;
/// Below this speed with no pedals pressed the brakes hold the car still.
const AUTO_HOLD_SPEED: f32 = 2.5;
/// Extra steering angle allowed beyond the grip-limited angle (front slip),
/// at low speed; it tapers off above `MARGIN_SPEED` so full lock at speed
/// sits at the tyres' peak rather than scrubbing past it.
const STEER_SLIP_MARGIN: f32 = 0.07;
const MARGIN_SPEED: f32 = 30.0;
/// A drift starts once the body is this far sideways (radians) with the
/// throttle down, and ends when it straightens up past `DRIFT_EXIT`.
const DRIFT_ENTRY: f32 = 0.26;
const DRIFT_EXIT: f32 = 0.1;
/// The drift angle held with the steering turned fully into the slide is
/// `Tuning::drift_angle`. Let go and it unwinds toward `DRIFT_UNWIND` (and
/// so ends, past `DRIFT_EXIT`); counter-steer and it closes right up.
const DRIFT_UNWIND: f32 = 0.06;
/// Share of the sliding tyres' drag along the direction of travel let off
/// while drifting: they still swing the car round, but don't stop it dead.
const DRIFT_GLIDE: f32 = 0.6;
/// Rear tyres' sideways grip while drifting short of the angle the steering
/// asks for is `Tuning::drift_grip`, so the tail can swing out; it firms up
/// to full grip as the angle passes it by `DRIFT_FIRM` (radians), so the
/// car can straighten.
const DRIFT_FIRM: f32 = 0.12;
/// How quickly the drift angle is brought to what the steering asks (1/s).
const DRIFT_ANGLE_RATE: f32 = 3.0;
/// Extra yaw rate a handbrake flick asks for, to swing the tail out (rad/s).
const HANDBRAKE_KICK: f32 = 1.4;
/// Share of the tyres' grip a held-down turn uses, leaving a little for the
/// engine to keep the speed up.
const CORNER_SHARE: f32 = 0.85;
/// Share of the rear tyres' grip the engine may use while drifting: power
/// keeps the tail out and the speed up instead of traction control
/// cutting it.
const DRIFT_DRIVE: f32 = 0.4;
/// Torque pulling yaw rate toward what the steering asks for (drift assist).
const YAW_ASSIST: f32 = 500.0;
/// Extra assist once sideslip passes this angle, to catch spins.
const SPIN_SLIP: f32 = 0.5;
/// Spring and damper around the car's long axis keeping it upright relative
/// to the ground while it has wheels down, so bumps and kerbs can't flip it.
const ROLL_ASSIST: f32 = 1500.0;
const ROLL_ASSIST_DAMP: f32 = 150.0;
/// In the air: spring and damper levelling pitch and roll, so jumps off
/// crests land on the wheels.
const AIR_LEVEL: f32 = 2600.0;
const AIR_LEVEL_DAMP: f32 = 700.0;
/// Cap on pitch and roll rates (rad/s), so ramps and crests can bounce the
/// car but rarely throw it over.
const MAX_TIP_RATE: f32 = 3.0;
/// On its side or roof and nearly still for this long, the car hops back
/// onto its wheels.
const RECOVER_AFTER: f32 = 1.0;
const RECOVER_HOP: f32 = 18.0;
/// A hop that hasn't landed upright by now has failed; allow another.
const RECOVER_GIVE_UP: f32 = 2.5;

// --- Water ---
/// Share of gravity water holds up once the car is fully under: it sinks, slowly.
const BUOYANCY: f32 = 0.7;
/// Water resistance on the body (per unit of mass and speed).
const WATER_DRAG: f32 = 2.2;
const WATER_ANGULAR_DRAG: f32 = 450.0;
/// Depth over which the car goes from just wet to fully under.
const SUBMERGE_DEPTH: f32 = 2.6;

// --- Body contacts (landings, flips) ---
const BODY_K: f32 = 6000.0;
const BODY_DAMP: f32 = 300.0;
const BODY_MU: f32 = 0.6;
const BODY_CORNERS: [[f32; 3]; 8] = [
    [-2.1, -1.2, -4.6], [2.1, -1.2, -4.6], [-2.1, -1.2, 4.1], [2.1, -1.2, 4.1],
    [-1.4, 1.25, -0.3], [1.4, 1.25, -0.3], [-1.4, 1.25, 1.6], [1.4, 1.25, 1.6],
];

const MAX_SUBSTEP: f32 = 1.0 / 240.0;
pub const WHEELBASE: f32 = WHEEL_MOUNTS[2][2] - WHEEL_MOUNTS[0][2];

/// What the car drives on. Lets tests run on flat ground.
pub trait Ground {
    fn height(&self, x: f32, z: f32) -> f32;
    /// Surface friction multiplier (1 = normal, <1 = sand/ice).
    fn grip(&self, x: f32, z: f32) -> f32;
    /// 1 on the road, 0 off it.
    fn on_road(&self, x: f32, z: f32) -> f32;
    /// Water surface height here, if (x, z) is over water.
    fn water(&self, _x: f32, _z: f32) -> Option<f32> {
        None
    }
    fn normal(&self, x: f32, z: f32) -> Vec3 {
        let e = 0.5;
        let dx = self.height(x + e, z) - self.height(x - e, z);
        let dz = self.height(x, z + e) - self.height(x, z - e);
        vec3(-dx, 2.0 * e, -dz).normalize()
    }
}

/// The real world: terrain flattened by roads.
pub struct WorldGround;

impl Ground for WorldGround {
    fn height(&self, x: f32, z: f32) -> f32 {
        crate::terrain::solid_height(x, z)
    }
    fn water(&self, x: f32, z: f32) -> Option<f32> {
        let level = crate::terrain::WATER_LEVEL;
        (crate::terrain::raw_height(x, z) < level && crate::terrain::solid_height(x, z) < level).then_some(level)
    }
    fn grip(&self, x: f32, z: f32) -> f32 {
        // Less of it in the wet
        crate::terrain::wet_grip(x, z, crate::weather::grip())
    }
    fn on_road(&self, x: f32, z: f32) -> f32 {
        crate::road::on_road(x, z)
    }
}

/// The steepest grade the car can drive up from a standstill (dry, balanced
/// setup), on ground of `grip` (`terrain::surface_grip`) at `road` (as
/// `Ground::on_road` gives it). Measured: `steepest_climb_matches_the_physics`
/// keeps it honest. World building keeps drivable slopes under this.
pub fn steepest_climb(grip: f32, road: f32) -> f32 {
    const GRIPS: [f32; 4] = [0.5, 0.6, 0.8, 1.0];
    // Rows: off the road, half on it, fully on it
    const CLIMB: [[f32; 4]; 3] = [[0.175, 0.20, 0.30, 0.40], [0.20, 0.275, 0.375, 0.475], [0.25, 0.325, 0.45, 0.525]];
    let along = |row: &[f32; 4]| {
        let g = grip.clamp(0.4, 1.0);
        let k = GRIPS.iter().position(|&x| x >= g).unwrap_or(3).max(1);
        let t = (g - GRIPS[k - 1]) / (GRIPS[k] - GRIPS[k - 1]);
        row[k - 1] + (row[k] - row[k - 1]) * t
    };
    let r = road.clamp(0.0, 1.0) * 2.0;
    let k = (r.floor() as usize).min(1);
    along(&CLIMB[k]) + (along(&CLIMB[k + 1]) - along(&CLIMB[k])) * (r - k as f32)
}

/// How the car is set up: more wing pressing it into the road for grip in
/// fast corners at the cost of drag on the straights, or the other way
/// round. Slow corners are much the same either way.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Setup {
    pub name: &'static str,
    /// Multiplies the tyres' own grip (softer or harder settings).
    pub grip: f32,
    /// Multiplies downforce.
    pub wing: f32,
    /// Multiplies aerodynamic drag.
    pub drag: f32,
}

pub const SETUPS: [Setup; 3] = [
    Setup { name: "BALANCED", grip: 1.0, wing: 1.0, drag: 1.0 },
    Setup { name: "GRIP", grip: 1.06, wing: 2.0, drag: 1.6 },
    Setup { name: "SPEED", grip: 0.96, wing: 0.8, drag: 0.8 },
];

#[derive(Clone, Copy, Default)]
pub struct CarInput {
    pub throttle: f32,
    pub brake: f32,
    /// -1 (left) to 1 (right)
    pub steer: f32,
    pub handbrake: bool,
}

pub struct CarPhysics {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub ang_vel: Vec3,
    /// Forward speed (signed).
    pub speed: f32,
    /// Current front wheel angle in radians (positive = right).
    pub steer_angle: f32,
    /// Permanent top-speed bonus from collectibles (adds engine force).
    pub speed_bonus: f32,
    pub wheel_compression: [f32; 4],
    pub wheel_spin: [f32; 4],
    pub wheel_contact: [bool; 4],
    /// Sideways slip speed per wheel (for smoke and tyre squeal).
    pub wheel_slip: [f32; 4],
    /// Whether each wheel exceeded its grip this step.
    pub wheel_sliding: [bool; 4],
    /// Where each wheel last touched the ground.
    pub wheel_contact_pos: [Vec3; 4],
    /// Each wheel's grip used, 0..1 and beyond when sliding; spinning under
    /// power or locked under braking counts too (for skid marks).
    pub wheel_scrub: [f32; 4],
    /// How long the car has lain tipped over; negative while a recovery hop is under way.
    tipped_time: f32,
    /// How far under water the body is: 0 dry, 1 fully submerged.
    pub submerged: f32,
    /// Held in a power slide (see `DRIFT_ENTRY`).
    pub drifting: bool,
    /// How fast the direction of travel is turning (rad/s, same sense as yaw).
    travel_turn: f32,
    /// How far into another car's slipstream (0 none, 1 right on its tail).
    /// Set from outside each frame.
    pub draft: f32,
    pub setup: Setup,
}

impl CarPhysics {
    pub fn new(start_pos: Vec3, start_yaw: f32) -> Self {
        Self::new_on(&WorldGround, start_pos, start_yaw)
    }

    pub fn new_on(ground: &dyn Ground, start_pos: Vec3, start_yaw: f32) -> Self {
        let mut car = Self {
            pos: start_pos,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            ang_vel: Vec3::ZERO,
            speed: 0.0,
            steer_angle: 0.0,
            speed_bonus: 0.0,
            wheel_compression: [0.0; 4],
            wheel_spin: [0.0; 4],
            wheel_contact: [false; 4],
            wheel_slip: [0.0; 4],
            wheel_sliding: [false; 4],
            wheel_contact_pos: [Vec3::ZERO; 4],
            wheel_scrub: [0.0; 4],
            tipped_time: 0.0,
            submerged: 0.0,
            drifting: false,
            travel_turn: 0.0,
            draft: 0.0,
            setup: SETUPS[0],
        };
        car.place(ground, start_pos.x, start_pos.z, start_yaw);
        car
    }

    /// Teleport to (x, z) facing `yaw`, resting on the ground, at rest.
    pub fn place(&mut self, ground: &dyn Ground, x: f32, z: f32, yaw: f32) {
        // Mount height above ground when springs carry the static load
        let static_comp = CAR_MASS * GRAVITY / 4.0 / SPRING_K;
        let ride = -WHEEL_MOUNTS[0][1] + WHEEL_RADIUS + SPRING_REST - static_comp;
        self.pos = vec3(x, ground.height(x, z) + ride, z);
        self.vel = Vec3::ZERO;
        self.ang_vel = Vec3::ZERO;
        self.rot = Quat::from_rotation_y(yaw);
        self.steer_angle = 0.0;
        self.wheel_compression = [static_comp; 4];
        self.tipped_time = 0.0;
        self.submerged = 0.0;
    }

    pub fn forward(&self) -> Vec3 {
        self.rot * Vec3::NEG_Z
    }

    pub fn up(&self) -> Vec3 {
        self.rot * Vec3::Y
    }

    /// Heading angle (rotation about world Y; 0 = facing -Z).
    pub fn yaw(&self) -> f32 {
        let f = self.forward();
        (-f.x).atan2(-f.z)
    }

    /// Flat forward direction on the XZ plane.
    pub fn heading(&self) -> Vec3 {
        let yaw = self.yaw();
        vec3(-yaw.sin(), 0.0, -yaw.cos())
    }

    /// Front wheel angle at full lock for the current speed (radians).
    pub fn steer_limit(&self) -> f32 {
        let grip_limit = (WHEELBASE * TYRE_MU * self.setup.grip * CORNER_SHARE * GRAVITY / (self.speed * self.speed).max(1.0)).atan();
        let margin = STEER_SLIP_MARGIN * (MARGIN_SPEED / self.speed.abs().max(MARGIN_SPEED)).powi(2);
        MAX_STEER.min(grip_limit + margin)
    }

    /// How far sideways the body is to its travel (radians, + with the
    /// velocity toward the car's right).
    pub fn sideslip(&self) -> f32 {
        self.vel.dot(self.rot * Vec3::X).atan2(self.speed.abs().max(1.0))
    }

    /// The steering that asks a drift under way to settle at `angle`
    /// (radians): the drift's own response to the wheel, run backwards.
    pub fn drift_steer(&self, angle: f32) -> f32 {
        let full = crate::tuning::get().drift_angle;
        let into = if angle >= DRIFT_UNWIND { (angle - DRIFT_UNWIND) / (full - DRIFT_UNWIND) } else { angle / DRIFT_UNWIND - 1.0 };
        into.clamp(-1.0, 1.0) * self.sideslip().signum()
    }

    pub fn grounded_wheels(&self) -> usize {
        self.wheel_contact.iter().filter(|&&c| c).count()
    }

    pub fn update(&mut self, ground: &dyn Ground, dt: f32, input: CarInput) {
        let dt = dt.min(0.05);
        let steps = ((dt / MAX_SUBSTEP).ceil() as u32).max(1);
        let sub_dt = dt / steps as f32;
        for _ in 0..steps {
            self.step(ground, sub_dt, input);
        }
    }

    fn step(&mut self, ground: &dyn Ground, dt: f32, input: CarInput) {
        let up = self.up();
        let fwd = self.forward();
        self.speed = self.vel.dot(fwd);

        // --- Steering: speed-sensitive limit so the front tyres stay near peak grip ---
        let wheelbase = WHEELBASE;
        let v2 = self.speed * self.speed;
        let target = input.steer.clamp(-1.0, 1.0) * self.steer_limit();
        let max_delta = STEER_RATE * dt;
        self.steer_angle += (target - self.steer_angle).clamp(-max_delta, max_delta);

        // How deep the body is in water (measured from its underside)
        self.submerged = ground.water(self.pos.x, self.pos.z)
            .map_or(0.0, |level| ((level - (self.pos.y - 1.2)) / SUBMERGE_DEPTH).clamp(0.0, 1.0));
        let wet = self.submerged;

        let mut force = vec3(0.0, -GRAVITY * CAR_MASS, 0.0);
        let mut torque = Vec3::ZERO;
        let com = self.pos;
        let apply = |f: Vec3, at: Vec3, force: &mut Vec3, torque: &mut Vec3| {
            *force += f;
            *torque += (at - com).cross(f);
        };

        // --- Suspension ---
        let mut loads = [0.0_f32; 4];
        let mut contacts = [Vec3::ZERO; 4];
        let mut normals = [Vec3::Y; 4];
        for i in 0..4 {
            let mount = self.pos + self.rot * Vec3::from(WHEEL_MOUNTS[i]);
            let n = ground.normal(mount.x, mount.z);
            let h = ground.height(mount.x, mount.z);
            // Distance along -up from the mount to the ground plane
            let denom = up.dot(n).max(0.2);
            let dist = (mount.y - h) * n.y / denom;
            let compression = SPRING_REST - (dist - WHEEL_RADIUS);

            if compression <= 0.0 {
                self.wheel_contact[i] = false;
                self.wheel_compression[i] = 0.0;
                continue;
            }
            let comp_vel = (compression - self.wheel_compression[i]) / dt;
            self.wheel_compression[i] = compression.min(SPRING_REST);
            self.wheel_contact[i] = true;

            let mut f = SPRING_K * compression + SPRING_DAMP * comp_vel;
            if compression > SPRING_REST * 0.9 {
                f += BUMP_STOP_K * (compression - SPRING_REST * 0.9);
            }
            loads[i] = f.max(0.0);
            contacts[i] = mount - up * dist;
            normals[i] = n;
        }

        // Anti-roll bars move load from the compressed side to the extended side
        for (l, r) in [(0, 1), (2, 3)] {
            if self.wheel_contact[l] && self.wheel_contact[r] {
                let shift = ANTI_ROLL * (self.wheel_compression[l] - self.wheel_compression[r]);
                loads[l] = (loads[l] + shift).max(0.0);
                loads[r] = (loads[r] - shift).max(0.0);
            }
        }

        // --- Drift: sideways with the power on (a handbrake flick, or lifting
        // and turning in hard) and the engine helps hold it there ---
        let throttle = input.throttle.clamp(0.0, 1.0);
        let sideslip = self.vel.dot(self.rot * Vec3::X).atan2(self.speed.abs().max(1.0));
        if self.speed < 20.0 || throttle < 0.3 || sideslip.abs() < DRIFT_EXIT {
            self.drifting = false;
        } else if sideslip.abs() > DRIFT_ENTRY {
            self.drifting = true;
        }
        // The angle the steering asks for (steering into the slide opens it)
        let into = input.steer.clamp(-1.0, 1.0) * sideslip.signum();
        let tune = crate::tuning::get();
        let drift_target = if into >= 0.0 { DRIFT_UNWIND + (tune.drift_angle - DRIFT_UNWIND) * into } else { DRIFT_UNWIND * (1.0 + into) };
        let rear_drift_grip = (tune.drift_grip + (1.0 - tune.drift_grip) * (sideslip.abs() - drift_target) / DRIFT_FIRM).clamp(tune.drift_grip, 1.0);

        // --- Tyres ---
        let brake = input.brake.clamp(0.0, 1.0);
        let nominal_load = CAR_MASS * GRAVITY / 4.0;
        let engine = ENGINE_FORCE + self.speed_bonus * 15.0;
        for i in 0..4 {
            self.wheel_sliding[i] = false;
            self.wheel_slip[i] = 0.0;
            self.wheel_scrub[i] = 0.0;
            if !self.wheel_contact[i] {
                continue;
            }
            self.wheel_contact_pos[i] = contacts[i];
            let n = normals[i];
            let contact = contacts[i];
            let mount = self.pos + self.rot * Vec3::from(WHEEL_MOUNTS[i]);
            apply(n * loads[i], mount, &mut force, &mut torque);

            let front = i < 2;
            let wheel_fwd = if front { Quat::from_axis_angle(up, self.steer_angle) * fwd } else { fwd };
            let wheel_fwd = (wheel_fwd - n * wheel_fwd.dot(n)).normalize_or_zero();
            let wheel_side = n.cross(wheel_fwd);

            let v = self.vel + self.ang_vel.cross(contact - self.pos);
            let v_long = v.dot(wheel_fwd);
            let v_lat = v.dot(wheel_side);

            let surface = ground.grip(contact.x, contact.z);
            let road = ground.on_road(contact.x, contact.z);
            // Lake-bed mud gives the tyres little to bite on
            let mu = TYRE_MU * self.setup.grip * surface * (0.75 + 0.25 * road) * (1.0 - 0.75 * wet);
            let max_force = mu * loads[i];
            let load_share = loads[i] / nominal_load;

            // Lateral: stiff at low speed so a parked car doesn't creep sideways
            let low_speed_boost = 1.0 + 4.0 / (1.0 + v_long.abs());
            let mut f_lat = -v_lat * CORNER_STIFFNESS * nominal_load * load_share * low_speed_boost / 4.0;
            if self.drifting && !front {
                f_lat = f_lat.clamp(-max_force * rear_drift_grip, max_force * rear_drift_grip);
            }

            // Longitudinal
            let mut f_long = -v_long * ROLL_RESIST * (2.0 - road);
            if !front && throttle > 0.0 {
                // Traction control: only use the grip cornering leaves spare,
                // except in a drift, where the wheels are let spin
                let spare = ((max_force * TRACTION_LIMIT).powi(2) - f_lat * f_lat).max(0.0).sqrt();
                let allowed = if self.drifting { spare.max(max_force * DRIFT_DRIVE) } else { spare };
                f_long += (engine * throttle * 0.5).min(allowed);
            }
            if brake > 0.0 {
                if self.speed > 2.0 {
                    // Taper near standstill so the brake doesn't chatter
                    let taper = (v_long.abs() / 2.0).min(1.0);
                    let share = if front { FRONT_BRAKE_SHARE } else { 0.5 - FRONT_BRAKE_SHARE };
                    f_long -= v_long.signum() * BRAKE_FORCE * brake * share * taper;
                } else if !front && self.speed > -REVERSE_MAX {
                    f_long -= engine * 0.25 * brake;
                }
            }
            // Parked (no pedals, barely moving): hold still, even on a slope
            if throttle == 0.0 && brake == 0.0 && self.speed.abs() < AUTO_HOLD_SPEED {
                f_long -= v_long.clamp(-1.0, 1.0) * BRAKE_FORCE * 0.25;
            }
            let mut grip_cap = max_force;
            if input.handbrake && !front {
                f_long -= v_long.clamp(-1.0, 1.0) * BRAKE_FORCE * 0.25;
                grip_cap *= HANDBRAKE_GRIP;
            }

            // Friction circle
            let total = (f_long * f_long + f_lat * f_lat).sqrt();
            self.wheel_scrub[i] = total / grip_cap.max(1e-3);
            if total > grip_cap {
                let scale = grip_cap * SLIDE_FACTOR / total;
                f_long *= scale;
                f_lat *= scale;
                self.wheel_sliding[i] = true;
            }
            self.wheel_slip[i] = v_lat.abs();

            let mut long_force = wheel_fwd * f_long;
            let mut lat_force = wheel_side * f_lat;
            if self.drifting {
                let travel = self.vel.normalize_or_zero();
                for f in [&mut long_force, &mut lat_force] {
                    let along = f.dot(travel);
                    if along < 0.0 {
                        *f -= travel * along * DRIFT_GLIDE;
                    }
                }
            }
            // Act partway up toward the centre of mass to tame roll and squat
            let height = (self.pos - contact).dot(up);
            apply(long_force, contact + up * height * LONGITUDINAL_LIFT, &mut force, &mut torque);
            apply(lat_force, contact + up * height * LATERAL_LIFT, &mut force, &mut torque);

            self.wheel_spin[i] += v_long / WHEEL_RADIUS * dt;
        }
        for i in 0..4 {
            if !self.wheel_contact[i] {
                // Free-spinning wheels slowly follow the body speed
                self.wheel_spin[i] += self.speed / WHEEL_RADIUS * dt;
            }
        }

        // --- Water: holds the car up a little, drags at it, damps any tumbling ---
        if wet > 0.0 {
            force += Vec3::Y * GRAVITY * CAR_MASS * BUOYANCY * wet;
            force -= self.vel * CAR_MASS * WATER_DRAG * wet;
            torque -= self.ang_vel * WATER_ANGULAR_DRAG * wet;
        }

        // --- Aero ---
        force -= self.vel * self.vel.length() * DRAG * self.setup.drag * (1.0 - crate::tuning::get().slipstream * self.draft.clamp(0.0, 1.0));
        force -= up * DOWNFORCE * self.setup.wing * v2;

        // --- Body contacts: landings on the chassis, roofs after flips ---
        let mut deepest = 0.0_f32;
        for corner in BODY_CORNERS {
            let p = self.pos + self.rot * Vec3::from(corner);
            let h = ground.height(p.x, p.z);
            let pen = h - p.y;
            if pen <= 0.0 {
                continue;
            }
            deepest = deepest.max(pen);
            let n = ground.normal(p.x, p.z);
            let v = self.vel + self.ang_vel.cross(p - self.pos);
            let vn = v.dot(n);
            let fn_mag = (BODY_K * pen - BODY_DAMP * vn).max(0.0);
            let vt = v - n * vn;
            let friction = -vt.normalize_or_zero() * (BODY_MU * fn_mag).min(vt.length() * CAR_MASS / dt * 0.25);
            apply(n * fn_mag + friction, p, &mut force, &mut torque);
        }

        // --- Arcade helpers ---
        let grounded = self.grounded_wheels();
        let local_ang = self.rot.inverse() * self.ang_vel;
        if grounded >= 2 && self.vel.length() > 3.0 {
            // Drift assist: steer the yaw rate toward what the front wheels ask
            // for, capped at what the tyres could deliver. Weaker on the
            // handbrake so you can still kick the tail out.
            let max_rate = TYRE_MU * self.setup.grip * CORNER_SHARE * GRAVITY / self.speed.abs().max(5.0);
            let mut desired = (self.speed * self.steer_angle.tan() / wheelbase).clamp(-max_rate, max_rate);
            let side_speed = self.vel.dot(self.rot * Vec3::X);
            let slip = side_speed.atan2(self.speed.abs());
            let spin = ((slip.abs() - SPIN_SLIP) * 4.0).clamp(0.0, 2.0);
            let mut strength = 1.0 + spin;
            if input.handbrake {
                // Swing the tail out the way the wheel is turned
                desired += input.steer.clamp(-1.0, 1.0) * HANDBRAKE_KICK;
                strength = 1.5;
            } else if self.drifting {
                // Hold the drift at the angle the steering asks for: the
                // slip grows as the car yaws faster than its path turns
                desired = self.travel_turn + (slip.signum() * drift_target - slip) * DRIFT_ANGLE_RATE;
                strength = 2.0;
            }
            let assist = -(local_ang.y - desired) * YAW_ASSIST * strength * (grounded as f32 / 4.0);
            torque += up * assist;
        }
        if grounded >= 1 {
            let mut n = Vec3::ZERO;
            for i in (0..4).filter(|&i| self.wheel_contact[i]) {
                n += normals[i];
            }
            let n = n.normalize_or_zero();
            let roll_err = up.cross(n).dot(fwd);
            torque += fwd * (roll_err * ROLL_ASSIST - self.ang_vel.dot(fwd) * ROLL_ASSIST_DAMP);
        }
        if grounded == 0 {
            // In the air: level out (by angle, so it works upside down too),
            // and allow a little yaw control
            let tilt = up.y.clamp(-1.0, 1.0).acos();
            let axis = up.cross(Vec3::Y).try_normalize().unwrap_or(fwd);
            let tumble = self.ang_vel - up * self.ang_vel.dot(up);
            torque += axis * tilt * AIR_LEVEL - tumble * AIR_LEVEL_DAMP;
            torque += up * (input.steer * 250.0 - local_ang.y * 60.0);
        }

        // --- Integrate ---
        let before = self.vel;
        self.vel += force / CAR_MASS * dt;
        let flat_speed2 = self.vel.x * self.vel.x + self.vel.z * self.vel.z;
        self.travel_turn = if flat_speed2 > 25.0 { before.cross(self.vel).y / flat_speed2 / dt } else { 0.0 };
        self.pos += self.vel * dt;

        self.ang_vel += self.inv_inertia_world() * torque * dt;
        self.ang_vel *= 1.0 - ANGULAR_DAMPING * dt;
        if self.tipped_time >= 0.0 {
            let local = self.rot.inverse() * self.ang_vel;
            let capped = vec3(local.x.clamp(-MAX_TIP_RATE, MAX_TIP_RATE), local.y, local.z.clamp(-MAX_TIP_RATE, MAX_TIP_RATE));
            self.ang_vel = self.rot * capped;
        }
        self.rot = (Quat::from_scaled_axis(self.ang_vel * dt) * self.rot).normalize();
        self.recover_if_tipped(grounded, dt);

        // Last-resort anti-tunnelling if something drove deep into the ground
        if deepest > 1.5 {
            self.pos.y += deepest - 1.5;
            let n = ground.normal(self.pos.x, self.pos.z);
            let vn = self.vel.dot(n);
            if vn < 0.0 {
                self.vel -= n * vn;
            }
        }
    }

    /// Lying on its side or roof: after a moment, hop up with a roll that
    /// brings it down on its wheels (the air levelling finishes the job).
    fn recover_if_tipped(&mut self, grounded: usize, dt: f32) {
        let up = self.up();
        if self.tipped_time < 0.0 {
            // Hop under way: done once it's back on its wheels, or give up
            // after a while so a failed hop can be tried again
            self.tipped_time -= dt;
            if (grounded >= 3 && up.y > 0.8) || self.tipped_time < -RECOVER_GIVE_UP {
                self.tipped_time = 0.0;
            }
            return;
        }
        if up.y < 0.5 && grounded <= 2 && self.vel.length() < 8.0 {
            self.tipped_time += dt;
        } else {
            self.tipped_time = 0.0;
        }
        if self.tipped_time > RECOVER_AFTER {
            let fwd = self.forward();
            // Roll the short way back upright; straight upside down, either way will do
            let roll_err = up.cross(Vec3::Y).dot(fwd);
            let dir = if roll_err.abs() > 0.05 { roll_err.signum() } else { 1.0 };
            let tilt = up.y.clamp(-1.0, 1.0).acos();
            let airtime = 2.0 * RECOVER_HOP / GRAVITY;
            self.vel = vec3(self.vel.x * 0.5, RECOVER_HOP, self.vel.z * 0.5);
            self.ang_vel = fwd * dir * (tilt / airtime * 1.2);
            self.tipped_time = -dt;
        }
    }

    /// Apply an impulse at a world-space point (collisions).
    pub fn apply_impulse(&mut self, impulse: Vec3, at: Vec3) {
        self.vel += impulse / CAR_MASS;
        self.ang_vel += self.inv_inertia_world() * (at - self.pos).cross(impulse);
    }

    fn inv_inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.rot);
        r * Mat3::from_diagonal(INERTIA.recip()) * r.transpose()
    }

    /// Inverse of the effective mass for an impulse along `dir` at `at`.
    pub fn inverse_mass_at(&self, at: Vec3, dir: Vec3) -> f32 {
        let r = at - self.pos;
        1.0 / CAR_MASS + dir.dot((self.inv_inertia_world() * r.cross(dir)).cross(r))
    }

    /// Velocity of a world-space point on the body.
    pub fn point_velocity(&self, at: Vec3) -> Vec3 {
        self.vel + self.ang_vel.cross(at - self.pos)
    }

    pub fn body_transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rot, self.pos)
    }

    pub fn wheel_transform(&self, i: usize) -> Mat4 {
        let mount = WHEEL_MOUNTS[i];
        let drop = SPRING_REST - self.wheel_compression[i];
        let local = vec3(mount[0], mount[1] - drop, mount[2]);
        let steer = if i < 2 { Mat4::from_rotation_y(self.steer_angle) } else { Mat4::IDENTITY };
        self.body_transform() * Mat4::from_translation(local) * steer * Mat4::from_rotation_x(self.wheel_spin[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat;
    impl Ground for Flat {
        fn height(&self, _x: f32, _z: f32) -> f32 { 0.0 }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn normal(&self, _x: f32, _z: f32) -> Vec3 { Vec3::Y }
    }

    /// A 15% slope rising toward -Z.
    struct Slope;
    impl Ground for Slope {
        fn height(&self, _x: f32, z: f32) -> f32 { -z * 0.15 }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 1.0 }
    }

    fn run(car: &mut CarPhysics, ground: &dyn Ground, seconds: f32, input: CarInput) {
        let dt = 1.0 / 60.0;
        for _ in 0..(seconds / dt) as usize {
            car.update(ground, dt, input);
            assert!(car.pos.is_finite() && car.vel.is_finite() && car.rot.is_finite(), "physics blew up");
        }
    }

    fn throttle(t: f32) -> CarInput {
        CarInput { throttle: t, ..Default::default() }
    }

    #[test]
    fn settles_at_rest() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        let start = car.pos;
        run(&mut car, &Flat, 3.0, CarInput::default());
        assert!(car.vel.length() < 0.1, "still moving: {}", car.vel);
        assert!((car.pos - start).length() < 0.5, "drifted: {}", car.pos - start);
        assert!(car.up().y > 0.999);
        assert_eq!(car.grounded_wheels(), 4);
    }

    #[test]
    fn accelerates_straight_to_top_speed() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        run(&mut car, &Flat, 2.0, throttle(1.0));
        let after_2s = car.speed;
        run(&mut car, &Flat, 18.0, throttle(1.0));
        assert!(after_2s > 50.0, "sluggish launch: {after_2s}");
        assert!((140.0..200.0).contains(&car.speed), "top speed {}", car.speed);
        assert!(car.yaw().abs() < 0.02, "veered: {}", car.yaw());
        assert!(car.vel.x.abs() < 1.0);
    }

    #[test]
    fn brakes_to_a_stop() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        run(&mut car, &Flat, 6.0, throttle(1.0));
        let mut t = 0.0;
        while car.speed > 2.0 && t < 10.0 {
            car.update(&Flat, 1.0 / 60.0, CarInput { brake: 1.0, ..Default::default() });
            t += 1.0 / 60.0;
        }
        assert!(t < 4.0, "took {t}s to stop");
        assert!(car.up().y > 0.99);
        assert!(car.yaw().abs() < 0.05, "swerved under braking: {}", car.yaw());
    }

    #[test]
    fn corners_without_spinning() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        run(&mut car, &Flat, 3.0, throttle(0.6));
        run(&mut car, &Flat, 8.0, CarInput { throttle: 0.4, steer: 1.0, ..Default::default() });
        assert!(car.up().y > 0.95, "rolled over: up={}", car.up());
        // Turning right means yaw increasing; check it goes round steadily
        let yaw_rate = (self::local_yaw_rate(&car)).abs();
        assert!(yaw_rate > 0.2 && yaw_rate < 4.0, "yaw rate {yaw_rate}");
        // Moving mostly forwards, not sideways
        assert!(car.speed > 0.0 && car.vel.dot(car.forward()) > car.vel.length() * 0.8);
    }

    #[test]
    fn steering_right_turns_right() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        run(&mut car, &Flat, 2.0, throttle(0.6));
        let yaw0 = car.yaw();
        run(&mut car, &Flat, 0.5, CarInput { throttle: 0.6, steer: 1.0, ..Default::default() });
        assert!(car.yaw() > yaw0 + 0.05, "yaw {} -> {}", yaw0, car.yaw());
    }

    #[test]
    fn handbrake_turn_rotates_car() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        run(&mut car, &Flat, 3.0, throttle(1.0));
        let yaw0 = car.yaw();
        run(&mut car, &Flat, 1.0, CarInput { steer: 1.0, handbrake: true, ..Default::default() });
        assert!((car.yaw() - yaw0).abs() > 0.5, "handbrake barely turned: {}", car.yaw() - yaw0);
        assert!(car.up().y > 0.95);
    }

    #[test]
    fn lands_from_a_drop() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.pos.y += 20.0;
        car.vel = vec3(0.0, 0.0, -80.0);
        run(&mut car, &Flat, 4.0, CarInput::default());
        assert!(car.up().y > 0.99, "landed crooked: {}", car.up());
        assert_eq!(car.grounded_wheels(), 4);
        assert!(car.vel.y.abs() < 0.5);
    }

    #[test]
    fn holds_on_a_slope_with_handbrake() {
        let mut car = CarPhysics::new_on(&Slope, Vec3::ZERO, 0.0);
        let start = car.pos;
        run(&mut car, &Slope, 3.0, CarInput { handbrake: true, ..Default::default() });
        assert!((car.pos - start).length() < 6.0, "slid {}", (car.pos - start).length());
    }

    #[test]
    fn stays_parked_on_a_slope() {
        let mut car = CarPhysics::new_on(&Slope, Vec3::ZERO, 0.0);
        let start = car.pos;
        run(&mut car, &Slope, 5.0, CarInput::default());
        assert!((car.pos - start).length() < 2.0, "rolled {}", (car.pos - start).length());
    }

    #[test]
    fn climbs_a_slope() {
        let mut car = CarPhysics::new_on(&Slope, Vec3::ZERO, 0.0);
        run(&mut car, &Slope, 5.0, throttle(1.0));
        assert!(car.speed > 40.0, "can't climb: {}", car.speed);
    }

    #[test]
    fn recovers_from_a_handbrake_slide() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        run(&mut car, &Flat, 4.0, throttle(1.0));
        run(&mut car, &Flat, 0.5, CarInput { throttle: 0.5, steer: 1.0, handbrake: true, ..Default::default() });
        run(&mut car, &Flat, 2.0, throttle(0.5));
        let side = car.vel.dot(car.rot * Vec3::X);
        let slip = side.atan2(car.speed.abs().max(0.1)).abs();
        assert!(slip < 0.17, "still sliding {:.1} degrees after letting go", slip.to_degrees());
        assert!(car.speed > 20.0, "slide killed all the speed: {}", car.speed);
    }

    /// Upside down (or on its side) at rest: the car must get back on its wheels.
    fn recovers_from(roll: f32) {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.rot = Quat::from_rotation_z(roll);
        car.pos.y = 3.0;
        let mut t = 0.0;
        while t < 8.0 && !(car.up().y > 0.95 && car.grounded_wheels() == 4) {
            run(&mut car, &Flat, 0.1, CarInput { throttle: 1.0, ..Default::default() });
            t += 0.1;
        }
        assert!(car.up().y > 0.95 && car.grounded_wheels() == 4, "still stuck after {t:.1}s, up={}", car.up());
    }

    /// A 40% side slope: the first hop may not stick, but it must keep trying.
    struct SideSlope;
    impl Ground for SideSlope {
        fn height(&self, x: f32, _z: f32) -> f32 { x * 0.4 }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 0.0 }
    }

    #[test]
    fn rights_itself_on_a_side_slope() {
        let mut car = CarPhysics::new_on(&SideSlope, Vec3::ZERO, 0.0);
        car.rot = Quat::from_rotation_z(std::f32::consts::PI);
        car.pos.y += 3.0;
        let mut upright_for = 0.0;
        for _ in 0..(15.0 * 60.0) as usize {
            car.update(&SideSlope, 1.0 / 60.0, CarInput::default());
            upright_for = if car.up().y > 0.8 && car.grounded_wheels() >= 3 { upright_for + 1.0 / 60.0 } else { 0.0 };
            if upright_for > 1.0 {
                return;
            }
        }
        panic!("never got back on its wheels: up={}", car.up());
    }

    /// A plain slope rising toward -Z (the way a car faces at yaw 0).
    struct Incline { grade: f32, grip: f32, road: f32 }
    impl Ground for Incline {
        fn height(&self, _x: f32, z: f32) -> f32 { -z * self.grade }
        fn grip(&self, _x: f32, _z: f32) -> f32 { self.grip }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { self.road }
    }

    /// Whether a car can drive up `grade` from a standstill on ground of `grip`.
    pub(crate) fn climbs(grade: f32, grip: f32, road: f32) -> bool {
        let ground = Incline { grade, grip, road };
        let mut car = CarPhysics::new_on(&ground, vec3(0.0, 0.0, 0.0), 0.0);
        for _ in 0..(12.0 * 120.0) as usize {
            car.update(&ground, 1.0 / 120.0, CarInput { throttle: 1.0, ..Default::default() });
        }
        -car.pos.z > 150.0
    }

    #[test]
    fn steepest_climb_matches_the_physics() {
        for road in [0.0, 0.25, 0.5, 1.0] {
            for grip in [0.5, 0.55, 0.7, 1.0] {
                let limit = steepest_climb(grip, road);
                assert!(climbs(limit - 0.03, grip, road), "can't climb {:.0}% at grip {grip} road {road}", (limit - 0.03) * 100.0);
                assert!(!climbs(limit + 0.05, grip, road), "climbs well over {:.0}% at grip {grip} road {road}", limit * 100.0);
            }
        }
    }

    /// Dev aid: the steepest grade the car climbs on each kind of ground.
    #[test]
    #[ignore]
    fn steepest_climbs() {
        for road in [0.0, 0.5, 1.0] {
            for grip in [1.0, 0.8, 0.6, 0.5] {
                let steepest = (1..40).map(|k| k as f32 * 0.025).take_while(|&g| climbs(g, grip, road)).last().unwrap_or(0.0);
                println!("road {road} grip {grip}: climbs {:.1}%", steepest * 100.0);
            }
        }
    }

    /// A beach shelving into a lake toward -Z: dry for z > 0, water (level 0) beyond.
    struct Lake;
    impl Ground for Lake {
        fn height(&self, _x: f32, z: f32) -> f32 { (z * 0.15).max(-12.0) }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 0.0 }
        fn water(&self, _x: f32, z: f32) -> Option<f32> { (z < 0.0).then_some(0.0) }
    }

    #[test]
    fn drives_into_a_lake_and_sinks() {
        let mut car = CarPhysics::new_on(&Lake, vec3(0.0, 0.0, 40.0), 0.0);
        car.vel = vec3(0.0, 0.0, -60.0);
        let mut deepest_rate = 0.0_f32;
        for _ in 0..(6.0 * 60.0) as usize {
            car.update(&Lake, 1.0 / 60.0, CarInput { throttle: 1.0, ..Default::default() });
            if car.submerged > 0.95 {
                deepest_rate = deepest_rate.min(car.vel.y);
            }
        }
        assert!(car.submerged > 0.95, "still afloat: submerged {}", car.submerged);
        assert!(car.pos.y < -3.0, "not under: y={}", car.pos.y);
        assert!(car.vel.length() < 8.0, "still charging along under water: {}", car.vel);
        assert!(deepest_rate > -12.0, "sank like a stone: {deepest_rate}");
    }

    #[test]
    fn rights_itself_from_its_roof() {
        recovers_from(std::f32::consts::PI);
    }

    #[test]
    fn rights_itself_from_its_side() {
        recovers_from(std::f32::consts::FRAC_PI_2);
    }

    fn local_yaw_rate(car: &CarPhysics) -> f32 {
        (car.rot.inverse() * car.ang_vel).y
    }

    #[test]
    fn braking_hard_mid_corner_keeps_the_tail_in_line() {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.vel = car.forward() * 70.0;
        run(&mut car, &Flat, 1.0, CarInput { steer: 1.0, throttle: 1.0, ..Default::default() });
        let mut worst = 0.0_f32;
        while car.vel.length() > 30.0 {
            car.update(&Flat, 1.0 / 60.0, CarInput { steer: 1.0, brake: 1.0, ..Default::default() });
            let side = car.vel.dot(car.rot * Vec3::X);
            worst = worst.max(side.atan2(car.speed.abs().max(1.0)).abs());
        }
        assert!(worst < 0.16, "tail stepped out {:.1} degrees", worst.to_degrees());
    }

    /// Flick into a drift with the handbrake at `speed`, then hold steer
    /// and throttle for `hold` seconds. Returns (sideslip at the end in
    /// radians, speed kept as a share of the entry speed, still drifting).
    fn held_drift(speed: f32, hold: f32) -> (CarPhysics, f32, f32) {
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.vel = car.forward() * speed;
        run(&mut car, &Flat, 0.4, CarInput { steer: 1.0, throttle: 1.0, handbrake: true, ..Default::default() });
        run(&mut car, &Flat, hold, CarInput { steer: 1.0, throttle: 1.0, ..Default::default() });
        let side = car.vel.dot(car.rot * Vec3::X);
        let slip = side.atan2(car.speed.abs().max(1.0)).abs();
        let kept = car.vel.length() / speed;
        (car, slip, kept)
    }

    #[test]
    fn a_drift_holds_with_the_power_on_and_keeps_its_speed() {
        let (car, slip, kept) = held_drift(75.0, 2.0);
        assert!(car.drifting, "drift ended while held");
        assert!((0.3..0.8).contains(&slip), "drift angle {:.0} degrees", slip.to_degrees());
        assert!(kept > 0.6, "drift scrubbed off too much speed: kept {:.0}%", kept * 100.0);
        assert!(car.up().y > 0.95);
    }

    #[test]
    fn letting_go_straightens_a_drift_and_countersteer_ends_it_sooner() {
        let straighten = |steer: f32| {
            let (mut car, _, _) = held_drift(75.0, 1.0);
            let mut t = 0.0;
            while car.drifting && t < 3.0 {
                car.update(&Flat, 1.0 / 60.0, CarInput { steer: -steer, throttle: 1.0, ..Default::default() });
                t += 1.0 / 60.0;
            }
            t
        };
        let (released, countered) = (straighten(0.0), straighten(1.0));
        assert!(released < 1.2, "still drifting {released:.1}s after letting go");
        assert!(countered < released, "counter-steering ({countered:.2}s) should be quicker than letting go ({released:.2}s)");
    }

    /// Dev aid: handling numbers on flat tarmac.
    #[test]
    #[ignore]
    fn handling_telemetry() {
        let dt = 1.0 / 60.0;
        let kmh = 1.8;
        // Acceleration
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        let (mut t, mut t100, mut t200) = (0.0, None, None);
        while t < 30.0 {
            car.update(&Flat, dt, throttle(1.0));
            t += dt;
            if t100.is_none() && car.speed * kmh >= 100.0 { t100 = Some(t); }
            if t200.is_none() && car.speed * kmh >= 200.0 { t200 = Some(t); }
        }
        println!("0-100 km/h {:.2}s, 0-200 {:.2}s, top {:.0} km/h", t100.unwrap_or(-1.0), t200.unwrap_or(-1.0), car.speed * kmh);
        let top = car.speed;
        // Braking from top speed
        let start = car.pos;
        let mut t = 0.0;
        while car.speed > 1.0 && t < 20.0 {
            car.update(&Flat, dt, CarInput { brake: 1.0, ..Default::default() });
            t += dt;
        }
        println!("brake from {:.0} km/h: {:.1}s, {:.0} units", top * kmh, t, (car.pos - start).length());

        // Steady cornering at speed: full lock, throttle to hold speed
        for v in [30.0, 60.0, 90.0, 120.0, 150.0] {
            let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
            car.vel = car.forward() * v;
            run(&mut car, &Flat, 0.5, CarInput::default());
            car.vel = car.forward() * v;
            let mut t = 0.0;
            let mut t90 = None;
            let mut rates = Vec::new();
            while t < 4.0 {
                let err = v - car.speed;
                let input = CarInput { steer: 1.0, throttle: (err * 0.2).clamp(0.0, 1.0), brake: (-err * 0.05).clamp(0.0, 1.0), handbrake: false };
                car.update(&Flat, dt, input);
                t += dt;
                rates.push((t, car.ang_vel.y.abs()));
            }
            let steady = rates.last().unwrap().1;
            for &(tt, r) in &rates { if t90.is_none() && r >= steady * 0.9 { t90 = Some(tt); } }
            let side = car.vel.dot(car.rot * Vec3::X);
            let slip = side.atan2(car.speed.abs()).to_degrees();
            let lat_g = steady * car.vel.length() / GRAVITY;
            println!("v {:3.0} km/h: yaw {:.2} rad/s, radius {:4.0}, lateral {:.2} g, slip {:5.1} deg, 90% in {:.2}s, held {:.0} km/h, steer {:.3}",
                v * kmh, steady, car.vel.length() / steady.max(1e-3), lat_g, slip, t90.unwrap_or(-1.0), car.speed * kmh, car.steer_angle);
        }

        // Running wide onto the grass flat out, and onto it while turning
        struct Grass;
        impl Ground for Grass {
            fn height(&self, _x: f32, _z: f32) -> f32 { 0.0 }
            fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
            fn on_road(&self, _x: f32, _z: f32) -> f32 { 0.0 }
            fn normal(&self, _x: f32, _z: f32) -> Vec3 { Vec3::Y }
        }
        for (name, ground) in [("tarmac", &Flat as &dyn Ground), ("grass", &Grass as &dyn Ground)] {
            let mut car = CarPhysics::new_on(ground, Vec3::ZERO, 0.0);
            car.vel = car.forward() * 140.0;
            run(&mut car, ground, 2.0, throttle(1.0));
            let straight = car.speed;
            let mut car = CarPhysics::new_on(ground, Vec3::ZERO, 0.0);
            car.vel = car.forward() * 100.0;
            run(&mut car, ground, 2.0, CarInput { throttle: 1.0, steer: 1.0, ..Default::default() });
            println!("{name:6}: 252 km/h flat out for 2s -> {:.0} km/h; 180 km/h turning -> {:.0} km/h", straight * kmh, car.speed * kmh);
        }

        // Mid-corner upsets: full lock at speed, then stamp on the brake or lift off
        for (what, brake, gas) in [("brake", 1.0, 0.0), ("lift", 0.0, 0.0), ("brake+steer half", 1.0, 0.0)] {
            for v in [70.0, 110.0] {
                let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
                car.vel = car.forward() * v;
                run(&mut car, &Flat, 0.3, throttle(1.0));
                car.vel = car.forward() * v;
                run(&mut car, &Flat, 1.0, CarInput { steer: 1.0, throttle: 1.0, ..Default::default() });
                let steer = if what.contains("half") { 0.5 } else { 1.0 };
                let mut worst = 0.0_f32;
                let mut t = 0.0;
                while t < 2.0 && car.vel.length() > 30.0 {
                    car.update(&Flat, dt, CarInput { steer, brake, throttle: gas, handbrake: false });
                    let side = car.vel.dot(car.rot * Vec3::X);
                    worst = worst.max(side.atan2(car.speed.abs().max(1.0)).abs());
                    t += dt;
                }
                println!("{what:17} at {:3.0} km/h: worst slip {:4.1} deg", v * kmh, worst.to_degrees());
            }
        }

        // Turn-in at a held speed: full lock one way, then the other
        for v in [40.0, 70.0, 100.0, 130.0] {
            let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
            car.vel = car.forward() * v;
            let mut t = 0.0;
            let mut rates = Vec::new();
            while t < 4.0 {
                let steer = if t < 2.0 { 1.0 } else { -1.0 };
                car.update(&Flat, dt, CarInput { steer, ..Default::default() });
                car.vel = car.vel.normalize() * v;
                t += dt;
                rates.push((t, (car.rot.inverse() * car.ang_vel).y));
            }
            let steady = rates.iter().find(|r| r.0 >= 1.95).unwrap().1;
            let t63 = rates.iter().find(|r| r.1.abs() >= steady.abs() * 0.63).map_or(-1.0, |r| r.0);
            let back = rates.iter().find(|r| r.0 > 2.0 && r.1 * steady.signum() <= -steady.abs() * 0.63).map_or(-1.0, |r| r.0 - 2.0);
            println!("turn-in at {:3.0} km/h: steady {:.2} rad/s ({:.2} of grip), 63% in {:.2}s, full reversal 63% in {:.2}s", v * kmh, steady, steady.abs() * v / (TYRE_MU * GRAVITY), t63, back);
        }

        // Handbrake flick into a held drift: steer in, then neutral, then out
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.vel = car.forward() * 80.0;
        let mut t = 0.0;
        let mut log = String::new();
        while t < 6.0 {
            let steer = if t < 2.5 { 1.0 } else if t < 4.0 { 0.0 } else { -1.0 };
            let input = CarInput { steer, throttle: 1.0, brake: 0.0, handbrake: t < 0.4 };
            car.update(&Flat, dt, input);
            t += dt;
            if (t * 4.0).fract() < dt * 4.0 {
                let side = car.vel.dot(car.rot * Vec3::X);
                log += &format!(" {:.0}deg/{:.0}{}[{:.2},{:.2}]", side.atan2(car.speed.abs()).to_degrees(), car.vel.length() * kmh, if car.drifting { "d" } else { "" }, car.travel_turn, (car.rot.inverse() * car.ang_vel).y);
            }
        }
        println!("handbrake drift (slip/kmh every 0.25s):{log}");
    }
}
