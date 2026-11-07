//! What moves over a circuit on race day: the sponsors' airship circling the
//! venue, hot-air balloons drifting higher up, a television helicopter
//! keeping up with the race, the funfair's big wheel turning, and the
//! starter on the line waving the flag.

use glam::{Mat4, Quat, Vec2, Vec3, vec3};

use crate::gpu::{Draw, GpuContext, Mesh};
use crate::props::Builder;
use crate::venue::WHEEL_RADIUS;

/// Things in the sky are drawn within this distance of the camera.
const SKY_DRAW_DISTANCE: f32 = 3600.0;
/// The airship's circle round the venue: how far out, how high, how fast.
const BLIMP_ORBIT: f32 = 430.0;
const BLIMP_HEIGHT: f32 = 135.0;
const BLIMP_SPEED: f32 = 0.032;
/// Where the helicopter hangs, from the car it's following.
const HELI_BEHIND: f32 = 80.0;
const HELI_SIDE: f32 = 45.0;
const HELI_ABOVE: f32 = 42.0;
/// Turns a minute of the big wheel.
const WHEEL_TURNS: f32 = 1.5;
const GONDOLAS: usize = 16;
/// The starter's rostrum: how far out past the gantry it stands, how high
/// its deck is, and where on the starter the flag arm swings from.
const ROSTRUM_OUT: f32 = 4.5;
const ROSTRUM_HEIGHT: f32 = 2.4;
const SHOULDER: Vec3 = vec3(0.42, 1.42, 0.0);

const SILVER: [f32; 4] = [0.82, 0.84, 0.86, 1.0];
const RED: [f32; 4] = [0.82, 0.12, 0.1, 1.0];
const DARK: [f32; 4] = [0.15, 0.16, 0.18, 1.0];
const WHITE: [f32; 4] = [0.95, 0.95, 0.92, 1.0];
const MARSHAL: [f32; 4] = [0.95, 0.42, 0.08, 1.0];
const BALLOONS: [[[f32; 4]; 2]; 3] = [
    [[0.92, 0.2, 0.14, 1.0], [0.98, 0.8, 0.16, 1.0]],
    [[0.14, 0.36, 0.84, 1.0], [0.95, 0.95, 0.92, 1.0]],
    [[0.12, 0.6, 0.36, 1.0], [0.96, 0.52, 0.12, 1.0]],
];

pub struct RaceDay {
    blimp: Mesh,
    balloons: Vec<Mesh>,
    heli: Mesh,
    rotor: Mesh,
    wheel: Mesh,
    gondola: Mesh,
    /// The big wheel's hub and axle, if the home circuit has its funfair.
    wheel_site: Option<(Vec3, Vec3)>,
    heli_pos: Vec3,
    heli_vel: Vec3,
    starter: Mesh,
    starter_arm: Mesh,
    starter_flag: Mesh,
    /// How hard the starter is waving (0 holding the flag still, 1 flat
    /// out), easing toward the cue, and how far through the wave.
    wave: f32,
    wave_phase: f32,
}

fn mesh(gpu: &GpuContext, build: impl FnOnce(&mut Builder)) -> Mesh {
    let mut b = Builder::default();
    build(&mut b);
    Mesh::new(gpu, &b.verts, &b.idxs)
}

impl RaceDay {
    pub fn new(gpu: &GpuContext) -> Self {
        Self {
            blimp: mesh(gpu, blimp),
            balloons: BALLOONS.iter().map(|&tones| mesh(gpu, |b| balloon(b, tones))).collect(),
            heli: mesh(gpu, helicopter),
            rotor: mesh(gpu, |b| {
                for yaw in [0.0, std::f32::consts::FRAC_PI_2] {
                    b.block(Vec3::ZERO, 0.35, 0.08, 7.5, yaw, DARK, DARK);
                }
            }),
            wheel: mesh(gpu, big_wheel),
            gondola: mesh(gpu, gondola),
            wheel_site: crate::road::network().and_then(crate::venue::wheel_site),
            heli_pos: Vec3::ZERO,
            heli_vel: Vec3::ZERO,
            starter: mesh(gpu, starter),
            starter_arm: mesh(gpu, |b| {
                b.limb(Vec3::ZERO, vec3(0.0, 0.75, 0.0), 0.11, 0.09, 4, 0.0, MARSHAL);
                b.limb(vec3(0.0, 0.6, 0.0), vec3(0.0, 2.6, 0.0), 0.05, 0.045, 4, 0.0, DARK);
            }),
            starter_flag: mesh(gpu, chequered_flag),
            wave: 0.0,
            wave_phase: 0.0,
        }
    }

    /// Cue the starter: `wave` 1 to wave the flag, 0 to hold it still.
    pub fn cue_starter(&mut self, wave: f32, dt: f32) {
        self.wave += (wave - self.wave) * (dt * 3.0).min(1.0);
        self.wave_phase += dt * (1.0 + 4.5 * self.wave);
    }

    /// Keep the helicopter on station by `car` (position, heading).
    pub fn follow(&mut self, car: Vec3, heading: Vec3, dt: f32) {
        let side = vec3(-heading.z, 0.0, heading.x);
        let ground = crate::terrain::terrain_height(car.x, car.z);
        let station = car - heading * HELI_BEHIND + side * HELI_SIDE + Vec3::Y * HELI_ABOVE;
        let station = vec3(station.x, station.y.max(ground + HELI_ABOVE), station.z);
        if self.heli_pos.distance(station) > 600.0 {
            self.heli_pos = station;
            self.heli_vel = Vec3::ZERO;
        }
        // A damped spring: it lags behind through corners and catches up
        let pull = (station - self.heli_pos) * 2.2 - self.heli_vel * 2.4;
        self.heli_vel += pull * dt;
        self.heli_pos += self.heli_vel * dt;
    }

    /// The airship and balloons round the venue at `venue` (the middle of the
    /// circuit, at ground level), the helicopter if `race` is on, and the
    /// big wheel, all at `time` seconds.
    pub fn draws<'a>(&'a self, out: &mut Vec<Draw<'a>>, camera: Vec3, venue: Option<Vec3>, race: bool, time: f32) {
        let near = |p: Vec3| p.distance(camera) < SKY_DRAW_DISTANCE;
        if let Some(centre) = venue {
            let a = time * BLIMP_SPEED;
            let at = centre + vec3(a.cos() * BLIMP_ORBIT, BLIMP_HEIGHT, a.sin() * BLIMP_ORBIT);
            // Nose along its circle, leaning a little into the turn
            let heading = Vec2::new(-a.sin(), a.cos());
            let yaw = (-heading.x).atan2(-heading.y);
            let pose = Quat::from_rotation_y(yaw) * Quat::from_rotation_z(-0.06);
            if near(at) {
                out.push(Draw::mesh(&self.blimp, Mat4::from_rotation_translation(pose, at + Vec3::Y * (time * 0.4).sin() * 2.0)));
            }
            for (k, mesh) in self.balloons.iter().enumerate() {
                let a = time * 0.009 + k as f32 * 2.1;
                let reach = 620.0 + k as f32 * 140.0;
                let at = centre + vec3(a.cos() * reach, 190.0 + k as f32 * 35.0 + (time * 0.2 + k as f32).sin() * 6.0, a.sin() * reach);
                if near(at) {
                    out.push(Draw::mesh(mesh, Mat4::from_rotation_translation(Quat::from_rotation_y(time * 0.05 + k as f32), at)));
                }
            }
        }
        if race && near(self.heli_pos) {
            let flat = vec3(self.heli_vel.x, 0.0, self.heli_vel.z);
            let yaw = if flat.length() > 2.0 { (-flat.x).atan2(-flat.z) } else { 0.0 };
            // Nose down as it speeds up, banking into its turns
            let pitch = -(flat.length() / 120.0).min(0.3);
            let pose = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
            let body = Mat4::from_rotation_translation(pose, self.heli_pos);
            out.push(Draw::mesh(&self.heli, body));
            out.push(Draw::mesh(&self.rotor, body * Mat4::from_translation(vec3(0.0, 3.4, 0.0)) * Mat4::from_rotation_y(time * 31.0)));
        }
        if let Some((hub, axle)) = self.wheel_site.filter(|(hub, _)| near(*hub)) {
            let face = Quat::from_rotation_arc(Vec3::Z, axle);
            let turn = time * WHEEL_TURNS / 60.0 * std::f32::consts::TAU;
            out.push(Draw::mesh(&self.wheel, Mat4::from_rotation_translation(face * Quat::from_rotation_z(turn), hub)));
            for k in 0..GONDOLAS {
                let a = turn + k as f32 / GONDOLAS as f32 * std::f32::consts::TAU;
                let rim = face * vec3(a.cos() * WHEEL_RADIUS, a.sin() * WHEEL_RADIUS, 0.0);
                out.push(Draw::mesh(&self.gondola, Mat4::from_rotation_translation(face, hub + rim)));
            }
        }
    }

    /// The starter on a rostrum beside the line at `start` (heading `dir`,
    /// the gantry `gate_half` wide), waving as cued.
    pub fn starter_draws<'a>(&'a self, out: &mut Vec<Draw<'a>>, camera: Vec3, start: Vec2, dir: Vec2, gate_half: f32) {
        let Some((deck, face)) = rostrum_site(start, dir, gate_half) else { return };
        if deck.distance(camera) > 900.0 {
            return;
        }
        let body = Mat4::from_rotation_translation(Quat::from_rotation_y(face.x.atan2(face.y)), deck);
        out.push(Draw::mesh(&self.starter, body));
        // Held up and leaning out while it waits; swept overhead side to
        // side when it's waved
        let swing = -0.35 * (1.0 - self.wave) + self.wave * 1.15 * (self.wave_phase * 1.3).sin();
        let arm = body * Mat4::from_translation(SHOULDER) * Mat4::from_rotation_z(swing);
        out.push(Draw::mesh(&self.starter_arm, arm));
        out.push(Draw::flags_at(&self.starter_flag, arm));
    }
}

/// Where the starter's rostrum stands for the line at `start`: its deck,
/// and which way the starter faces. On whichever side of the road is clear
/// of the pits and the road itself, just short of the line.
fn rostrum_site(start: Vec2, dir: Vec2, gate_half: f32) -> Option<(Vec3, Vec2)> {
    let side = Vec2::new(-dir.y, dir.x);
    [1.0, -1.0].into_iter().find_map(|s| {
        let p = start + side * s * (gate_half + ROSTRUM_OUT) - dir * 3.0;
        if crate::venue::occupied(p, 3.0) || crate::road::paved(p.x, p.y) {
            return None;
        }
        let ground = [(-1.3, -1.3), (1.3, -1.3), (1.3, 1.3), (-1.3, 1.3)].iter()
            .map(|&(x, z)| crate::terrain::settled(p.x + x, p.y + z))
            .fold(f32::MAX, f32::min);
        Some((vec3(p.x, ground + ROSTRUM_HEIGHT, p.y), -side * s))
    })
}

/// The starter's rostrum and the starter on it, facing +Z, the deck's top
/// at the origin. The flag arm is drawn on its own so it can swing.
fn starter(b: &mut Builder) {
    let orange = [0.92, 0.48, 0.1, 1.0];
    b.block(vec3(0.0, -0.3, 0.0), 1.3, 0.3, 1.3, 0.0, orange, [0.55, 0.55, 0.56, 1.0]);
    // Legs long enough to reach the ground however it falls away
    for (x, z) in [(-1.15, -1.15), (1.15, -1.15), (1.15, 1.15), (-1.15, 1.15)] {
        b.limb(vec3(x, -ROSTRUM_HEIGHT - 3.0, z), vec3(x, -0.3, z), 0.08, 0.08, 4, 0.0, SILVER);
    }
    // A rail round the back and sides
    for (a, c) in [((-1.2, -1.2), (1.2, -1.2)), ((-1.2, -1.2), (-1.2, 1.2)), ((1.2, -1.2), (1.2, 1.2))] {
        let (a, c) = (vec3(a.0, 1.0, a.1), vec3(c.0, 1.0, c.1));
        b.limb(a, c, 0.04, 0.04, 4, 0.0, SILVER);
        for p in [a, c] {
            b.limb(vec3(p.x, 0.0, p.z), p, 0.04, 0.04, 4, 0.0, SILVER);
        }
    }
    // The starter: dark trousers, a bright overall, a cap
    let skin = [0.86, 0.66, 0.5, 1.0];
    for x in [-0.17, 0.17] {
        b.block(vec3(x, 0.0, 0.0), 0.13, 0.8, 0.14, 0.0, DARK, DARK);
    }
    b.block(vec3(0.0, 0.8, 0.0), 0.36, 0.7, 0.22, 0.0, MARSHAL, MARSHAL);
    b.block(vec3(0.0, 1.5, 0.0), 0.2, 0.4, 0.2, 0.0, skin, skin);
    b.block(vec3(0.0, 1.88, 0.03), 0.22, 0.1, 0.25, 0.0, WHITE, WHITE);
    b.limb(vec3(-SHOULDER.x, SHOULDER.y, 0.0), vec3(-0.5, 0.75, 0.05), 0.1, 0.09, 4, 0.0, MARSHAL);
}

/// Black and white squares on a cloth from the top of the stick, free to
/// ripple toward its far edge (drawn as `MeshKind::Flag`).
fn chequered_flag(b: &mut Builder) {
    const COLS: usize = 5;
    const ROWS: usize = 4;
    let (length, height, top) = (2.4, 1.6, 2.6);
    for c in 0..COLS {
        for r in 0..ROWS {
            let (x0, x1) = (-length * c as f32 / COLS as f32, -length * (c + 1) as f32 / COLS as f32);
            let (y0, y1) = (top - height * r as f32 / ROWS as f32, top - height * (r + 1) as f32 / ROWS as f32);
            let (f0, f1) = (c as f32 / COLS as f32, (c + 1) as f32 / COLS as f32);
            let tone = if (c + r) % 2 == 0 { [0.95, 0.95, 0.93] } else { [0.06, 0.06, 0.07] };
            // Both faces, so it shows from the road and the stands
            b.cloth([vec3(x0, y0, 0.0), vec3(x1, y0, 0.0), vec3(x1, y1, 0.0), vec3(x0, y1, 0.0)], [f0, f1, f1, f0], tone);
            b.cloth([vec3(x0, y0, 0.0), vec3(x0, y1, 0.0), vec3(x1, y1, 0.0), vec3(x1, y0, 0.0)], [f0, f0, f1, f1], tone);
        }
    }
}

/// A round body along Z from `-length` to `length`, its radius `profile` of
/// the way along (0 at the nose, 1 at the tail) times `radius`.
fn body_of_revolution(b: &mut Builder, length: f32, radius: f32, sides: usize, rings: usize, profile: impl Fn(f32) -> f32, tone: impl Fn(usize, usize) -> [f32; 4]) {
    let point = |ring: usize, k: usize| {
        let f = ring as f32 / rings as f32;
        let a = std::f32::consts::TAU * k as f32 / sides as f32;
        let r = radius * profile(f);
        vec3(a.cos() * r, a.sin() * r, -length + 2.0 * length * f)
    };
    for ring in 0..rings {
        for k in 0..sides {
            b.quad(point(ring, k), point(ring, k + 1), point(ring + 1, k + 1), point(ring + 1, k), tone(ring, k));
        }
    }
}

fn blimp(b: &mut Builder) {
    let (length, radius) = (34.0, 8.0);
    body_of_revolution(b, length, radius, 14, 14, |f| (1.0 - (2.0 * f - 1.0).powi(2)).max(0.0).powf(0.6).max(0.04), |ring, _| if ring == 4 || ring == 5 { RED } else { SILVER });
    // Fins at the tail
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let out = vec3(dx, dy, 0.0);
        let (root0, root1) = (out * 2.6 + Vec3::Z * 20.0, out * 0.8 + Vec3::Z * 31.0);
        let tip = out * 10.0 + Vec3::Z * 31.0;
        b.tri(root0, root1, tip, RED);
        b.tri(root0, tip, root1, RED);
    }
    // The gondola slung underneath
    b.block(vec3(0.0, -radius - 2.0, -4.0), 1.8, 2.2, 5.0, 0.0, WHITE, WHITE);
    b.block(vec3(0.0, -radius - 1.2, -4.0), 1.86, 0.7, 4.0, 0.0, DARK, DARK);
    // Its name down both sides
    for s in [-1.0_f32, 1.0] {
        let name = "STUMBLER";
        let px = 1.15;
        let width = crate::roadside::text_units(name) * px;
        let along = vec3(0.0, 0.0, -s);
        let origin = vec3(s * (radius + 0.05), 3.5 * px, width * 0.5 * s);
        crate::roadside::lettering(b, origin, along, -Vec3::Y, px, name, [0.12, 0.22, 0.62, 1.0]);
    }
}

fn balloon(b: &mut Builder, tones: [[f32; 4]; 2]) {
    // Built along Z, then stood up: Z becomes the height
    let mut envelope = Builder::default();
    let height = 13.0;
    body_of_revolution(&mut envelope, height, 9.5, 12, 10, |f| {
        let f = 1.0 - f;
        if f < 0.7 { (f / 0.7).powf(0.55) } else { 1.0 - ((f - 0.7) / 0.3).powi(2) * 0.85 }
    }.max(0.15), |_, k| tones[k % 2]);
    let stand = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    for v in &mut envelope.verts {
        let p = stand * Vec3::from(v.pos) + Vec3::Y * (height + 6.0);
        v.pos = p.into();
    }
    let base = b.verts.len() as u32;
    b.verts.extend(envelope.verts);
    b.idxs.extend(envelope.idxs.iter().map(|i| i + base));
    b.block(Vec3::ZERO, 1.4, 1.6, 1.4, 0.0, [0.5, 0.34, 0.2, 1.0], [0.4, 0.28, 0.16, 1.0]);
    for (x, z) in [(-1.3, -1.3), (1.3, -1.3), (1.3, 1.3), (-1.3, 1.3)] {
        b.limb(vec3(x, 1.6, z), vec3(x * 2.2, 6.6, z * 2.2), 0.04, 0.04, 3, 0.0, DARK);
    }
}

fn helicopter(b: &mut Builder) {
    let blue = [0.12, 0.3, 0.7, 1.0];
    body_of_revolution(b, 4.2, 1.9, 8, 6, |f| (1.0 - (2.0 * f - 1.0).powi(2)).max(0.0).sqrt().max(0.3), |ring, _| if ring == 1 { [0.3, 0.5, 0.65, 1.0] } else { blue });
    b.limb(vec3(0.0, 0.4, 3.6), vec3(0.0, 1.0, 12.0), 0.5, 0.25, 5, 0.0, blue);
    b.block(vec3(0.0, 0.6, 11.6), 0.1, 2.4, 0.8, 0.0, WHITE, WHITE);
    b.limb(vec3(0.0, 1.9, 0.0), vec3(0.0, 3.4, 0.0), 0.25, 0.2, 4, 0.0, DARK);
    for s in [-1.0, 1.0] {
        b.limb(vec3(s * 1.5, -2.4, -2.6), vec3(s * 1.5, -2.4, 2.6), 0.1, 0.1, 4, 0.0, DARK);
        for z in [-1.5, 1.5] {
            b.limb(vec3(s * 1.5, -2.4, z), vec3(s * 0.9, -1.2, z), 0.08, 0.08, 4, 0.0, DARK);
        }
    }
    // The camera ball under the nose
    b.blob(vec3(0.0, -1.6, -3.0), 0.55, 1.0, DARK, 3.0);
}

fn big_wheel(b: &mut Builder) {
    const SEGMENTS: usize = 32;
    let rim = |k: usize, z: f32, r: f32| {
        let a = std::f32::consts::TAU * k as f32 / SEGMENTS as f32;
        vec3(a.cos() * r, a.sin() * r, z)
    };
    for z in [-3.0, 3.0] {
        for k in 0..SEGMENTS {
            b.limb(rim(k, z, WHEEL_RADIUS), rim(k + 1, z, WHEEL_RADIUS), 0.35, 0.35, 4, 0.0, WHITE);
            b.limb(rim(k, z, WHEEL_RADIUS * 0.7), rim(k + 1, z, WHEEL_RADIUS * 0.7), 0.2, 0.2, 4, 0.0, WHITE);
            if k % 2 == 0 {
                b.limb(vec3(0.0, 0.0, z), rim(k, z, WHEEL_RADIUS), 0.12, 0.12, 3, 0.0, WHITE);
            }
        }
    }
    for k in (0..SEGMENTS).step_by(SEGMENTS / GONDOLAS) {
        b.limb(rim(k, -3.0, WHEEL_RADIUS), rim(k, 3.0, WHEEL_RADIUS), 0.15, 0.15, 4, 0.0, WHITE);
    }
    b.limb(vec3(0.0, 0.0, -4.6), vec3(0.0, 0.0, 4.6), 1.4, 1.4, 8, 0.0, RED);
    b.glowing = true;
    for k in (0..SEGMENTS).step_by(2) {
        b.blob(rim(k, 3.4, WHEEL_RADIUS), 0.4, 1.0, [1.0, 0.85, 0.4, 1.0], k as f32);
    }
    b.glowing = false;
}

fn gondola(b: &mut Builder) {
    b.limb(Vec3::ZERO, vec3(0.0, -1.6, 0.0), 0.08, 0.08, 3, 0.0, DARK);
    b.block(vec3(0.0, -4.6, 0.0), 1.6, 3.0, 1.4, 0.0, [0.92, 0.3, 0.2, 1.0], WHITE);
    b.block(vec3(0.0, -2.9, 0.0), 1.62, 0.9, 1.42, 0.0, [0.25, 0.4, 0.55, 1.0], [0.25, 0.4, 0.55, 1.0]);
    b.roof(vec3(0.0, -1.6, 0.0), 1.6, 0.8, 1.4, 0.0, RED);
}
