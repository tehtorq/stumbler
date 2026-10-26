//! Scenery built per chunk: boulders, viaduct pillars and guard rails under
//! raised roads, grandstands at circuits, landmark models, and (through
//! `roadside`, `trackside` and `farms`) signs, telegraph poles, tyre walls,
//! hoardings and farmsteads.

use glam::{Quat, Vec2, Vec3, vec3};

use crate::collisions::Solid;
use crate::geometry::Vertex;
use crate::landmarks::{self, Kind, Landmark};
use crate::road::{self, KERB_WIDTH, ROAD_HALF_WIDTH};
use crate::terrain::{self, terrain_height, TERRAIN_HALF_SIZE, WATER_LEVEL};

const STONE: [f32; 4] = [0.55, 0.53, 0.5, 1.0];
const CONCRETE: [f32; 4] = [0.68, 0.67, 0.64, 1.0];
const RAIL: [f32; 4] = [0.8, 0.8, 0.82, 1.0];
const WHITE: [f32; 4] = [0.93, 0.92, 0.88, 1.0];
const RED: [f32; 4] = [0.8, 0.15, 0.12, 1.0];
const DARK: [f32; 4] = [0.2, 0.18, 0.17, 1.0];
const WOOD: [f32; 4] = [0.45, 0.3, 0.18, 1.0];
const ROOF_COLORS: [[f32; 4]; 4] = [[0.7, 0.25, 0.15, 1.0], [0.45, 0.25, 0.2, 1.0], [0.35, 0.37, 0.42, 1.0], [0.6, 0.35, 0.15, 1.0]];
const WALL_COLORS: [[f32; 4]; 4] = [[0.92, 0.88, 0.78, 1.0], [0.85, 0.82, 0.75, 1.0], [0.95, 0.93, 0.9, 1.0], [0.82, 0.72, 0.6, 1.0]];

/// Boulders smaller than this are left for cars to drive through.
const SOLID_BOULDER: f32 = 3.0;

/// Accumulates props geometry with simple primitives, plus the colliders
/// for anything a car shouldn't drive through.
#[derive(Default)]
pub struct Builder {
    pub verts: Vec<Vertex>,
    pub idxs: Vec<u32>,
    pub solids: Vec<Solid>,
    /// Geometry that glows after dark (drawn as `MeshKind::Glow`).
    pub glow_verts: Vec<Vertex>,
    pub glow_idxs: Vec<u32>,
    /// While set, new geometry goes to the glow buffers.
    pub glowing: bool,
}

impl Builder {
    fn buffers(&mut self) -> (&mut Vec<Vertex>, &mut Vec<u32>) {
        if self.glowing { (&mut self.glow_verts, &mut self.glow_idxs) } else { (&mut self.verts, &mut self.idxs) }
    }

    pub fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: [f32; 4]) {
        let (verts, idxs) = self.buffers();
        let base = verts.len() as u32;
        for p in [a, b, c, d] {
            verts.push(Vertex { pos: p.into(), color });
        }
        idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, color: [f32; 4]) {
        let (verts, idxs) = self.buffers();
        let base = verts.len() as u32;
        for p in [a, b, c] {
            verts.push(Vertex { pos: p.into(), color });
        }
        idxs.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Tapered prism from `a` to `b` with radii `r0`, `r1`, capped both ends.
    #[allow(clippy::too_many_arguments)]
    pub fn limb(&mut self, a: Vec3, b: Vec3, r0: f32, r1: f32, sides: usize, twist: f32, color: [f32; 4]) {
        let axis = (b - a).normalize_or_zero();
        let rot = Quat::from_rotation_arc(Vec3::Y, axis);
        let ring = |c: Vec3, r: f32, k: usize| {
            let t = std::f32::consts::TAU * k as f32 / sides as f32 + twist;
            c + rot * vec3(t.cos() * r, 0.0, t.sin() * r)
        };
        for k in 0..sides {
            let (p0, p1) = (ring(a, r0, k), ring(a, r0, k + 1));
            let (q0, q1) = (ring(b, r1, k), ring(b, r1, k + 1));
            self.quad(p0, q0, q1, p1, color);
            self.tri(b, q1, q0, color);
            self.tri(a, p0, p1, color);
        }
    }

    /// Faceted lump (a jittered icosahedron), squashed vertically by `squash`.
    pub fn blob(&mut self, c: Vec3, r: f32, squash: f32, color: [f32; 4], seed: f32) {
        let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
        let raw = [
            vec3(-1.0, t, 0.0), vec3(1.0, t, 0.0), vec3(-1.0, -t, 0.0), vec3(1.0, -t, 0.0),
            vec3(0.0, -1.0, t), vec3(0.0, 1.0, t), vec3(0.0, -1.0, -t), vec3(0.0, 1.0, -t),
            vec3(t, 0.0, -1.0), vec3(t, 0.0, 1.0), vec3(-t, 0.0, -1.0), vec3(-t, 0.0, 1.0),
        ];
        let spin = Quat::from_rotation_y(seed * 5.0);
        let pts: Vec<Vec3> = raw.iter().enumerate().map(|(k, v)| {
            let j = 0.82 + 0.36 * hash(seed + k as f32, seed * 1.3);
            let d = spin * v.normalize() * r * j;
            c + vec3(d.x, d.y * squash, d.z)
        }).collect();
        const FACES: [[usize; 3]; 20] = [
            [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
            [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
            [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
            [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
        ];
        for (k, f) in FACES.iter().enumerate() {
            let shade = 0.88 + 0.2 * hash(seed, k as f32);
            let col = [color[0] * shade, color[1] * shade, color[2] * shade, color[3]];
            self.tri(pts[f[0]], pts[f[1]], pts[f[2]], col);
        }
    }

    /// A round collider (for the secrets module).
    pub fn post(&mut self, bottom: Vec3, radius: f32, height: f32) {
        self.solid_post(bottom, radius, height);
    }

    /// Box from `bottom` (centre of base) up, rotated about Y by `yaw`.
    pub fn block(&mut self, bottom: Vec3, half_x: f32, height: f32, half_z: f32, yaw: f32, side: [f32; 4], top: [f32; 4]) {
        let r = Quat::from_rotation_y(yaw);
        let c = |x: f32, y: f32, z: f32| bottom + r * vec3(x, y, z);
        let (x, z, h) = (half_x, half_z, height);
        let corners = [c(-x, 0.0, -z), c(x, 0.0, -z), c(x, 0.0, z), c(-x, 0.0, z)];
        let tops = [c(-x, h, -z), c(x, h, -z), c(x, h, z), c(-x, h, z)];
        for k in 0..4 {
            let n = (k + 1) % 4;
            self.quad(corners[k], corners[n], tops[n], tops[k], side);
        }
        self.quad(tops[0], tops[1], tops[2], tops[3], top);
    }

    /// A `block` cars collide with.
    pub fn solid_block(&mut self, bottom: Vec3, half_x: f32, height: f32, half_z: f32, yaw: f32, side: [f32; 4], top: [f32; 4]) {
        self.block(bottom, half_x, height, half_z, yaw, side, top);
        self.solids.push(Solid { centre: vec2(bottom.x, bottom.z), half: vec2(half_x, half_z), yaw, top: bottom.y + height, round: false });
    }

    /// A round collider of `radius` from `bottom` up `height`, for columns and boulders.
    fn solid_post(&mut self, bottom: Vec3, radius: f32, height: f32) {
        self.solids.push(Solid { centre: vec2(bottom.x, bottom.z), half: Vec2::splat(radius), yaw: 0.0, top: bottom.y + height, round: true });
    }

    /// Pitched roof sitting on a block of the same footprint.
    pub fn roof(&mut self, base: Vec3, half_x: f32, rise: f32, half_z: f32, yaw: f32, color: [f32; 4]) {
        let r = Quat::from_rotation_y(yaw);
        let c = |x: f32, y: f32, z: f32| base + r * vec3(x, y, z);
        let (x, z) = (half_x + 0.6, half_z + 0.6);
        let ridge = [c(0.0, rise, -z), c(0.0, rise, z)];
        self.quad(c(-x, 0.0, -z), c(-x, 0.0, z), ridge[1], ridge[0], color);
        self.quad(c(x, 0.0, z), c(x, 0.0, -z), ridge[0], ridge[1], color);
        self.tri(c(-x, 0.0, -z), ridge[0], c(x, 0.0, -z), color);
        self.tri(c(x, 0.0, z), ridge[1], c(-x, 0.0, z), color);
    }

    /// Tapered prism (cylinder-ish) from `bottom` up.
    pub fn column(&mut self, bottom: Vec3, r0: f32, r1: f32, height: f32, sides: usize, color: [f32; 4]) {
        let ring = |r: f32, y: f32, k: usize| {
            let a = std::f32::consts::TAU * (k % sides) as f32 / sides as f32;
            bottom + vec3(a.cos() * r, y, a.sin() * r)
        };
        for k in 0..sides {
            self.quad(ring(r0, 0.0, k), ring(r0, 0.0, k + 1), ring(r1, height, k + 1), ring(r1, height, k), color);
        }
        let top = bottom + Vec3::Y * height;
        for k in 0..sides {
            self.tri(top, ring(r1, height, k + 1), ring(r1, height, k), color);
        }
    }

    pub fn cone(&mut self, bottom: Vec3, radius: f32, height: f32, sides: usize, color: [f32; 4]) {
        let apex = bottom + Vec3::Y * height;
        for k in 0..sides {
            let a0 = std::f32::consts::TAU * k as f32 / sides as f32;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / sides as f32;
            self.tri(bottom + vec3(a0.cos(), 0.0, a0.sin()) * radius, bottom + vec3(a1.cos(), 0.0, a1.sin()) * radius, apex, color);
        }
    }

    /// Irregular low-poly boulder: a jittered octahedron. Big ones are solid.
    pub fn boulder(&mut self, centre: Vec3, size: f32, seed: f32, color: [f32; 4]) {
        if size >= SOLID_BOULDER {
            self.solid_post(centre - Vec3::Y * size * 0.5, size * 0.8, size * 1.3);
        }
        let j = |k: f32| 0.75 + 0.5 * ((seed * 12.9898 + k * 78.233).sin() * 43758.547).fract().abs();
        let r = Quat::from_rotation_y(seed * 7.0);
        let pts = [
            vec3(size * j(1.0), 0.0, 0.0), vec3(-size * j(2.0), 0.0, 0.0),
            vec3(0.0, size * 0.8 * j(3.0), 0.0), vec3(0.0, -size * 0.5, 0.0),
            vec3(0.0, 0.0, size * j(4.0)), vec3(0.0, 0.0, -size * j(5.0)),
        ].map(|p| centre + r * p);
        let shade = |k: f32| {
            let s = 0.9 + 0.2 * j(k + 10.0) - 0.15;
            [color[0] * s, color[1] * s, color[2] * s, 1.0]
        };
        let faces = [(0, 2, 4), (4, 2, 1), (1, 2, 5), (5, 2, 0), (4, 3, 0), (1, 3, 4), (5, 3, 1), (0, 3, 5)];
        for (k, (a, b, c)) in faces.into_iter().enumerate() {
            self.tri(pts[a], pts[b], pts[c], shade(k as f32));
        }
    }
}

pub fn hash(x: f32, z: f32) -> f32 {
    ((x * 12.9898 + z * 78.233).sin() * 43758.547).fract().abs()
}

/// Ground height under a footprint, using the lowest corner so nothing floats.
pub fn ground(p: Vec2, radius: f32) -> f32 {
    [vec2(radius, radius), vec2(-radius, radius), vec2(radius, -radius), vec2(-radius, -radius), Vec2::ZERO]
        .iter()
        .map(|o| terrain_height(p.x + o.x, p.y + o.y))
        .fold(f32::MAX, f32::min)
}

fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

/// Props for one chunk: the ordinary mesh, what glows after dark, and colliders.
pub struct Props {
    pub mesh: (Vec<Vertex>, Vec<u32>),
    pub glow: (Vec<Vertex>, Vec<u32>),
    pub solids: Vec<Solid>,
}

/// All props whose anchor point lies in [lo, hi).
pub fn generate_props(lo: Vec2, hi: Vec2) -> Props {
    let mut b = Builder::default();
    let inside = |p: Vec2| p.x >= lo.x && p.y >= lo.y && p.x < hi.x && p.y < hi.y;

    boulders(&mut b, lo, hi);
    if let Some(network) = road::network() {
        crate::roadside::build(&mut b, network, &inside);
        crate::trackside::build(&mut b, network, &inside);
        crate::farms::build(&mut b, &inside);
        crate::railway::build_props(&mut b, &inside);
        for path in &network.paths {
            raised_road_supports(&mut b, path, &inside);
            if path.circuit() && inside(path.points[0]) {
                grandstands(&mut b, path);
            }
        }
    }
    for landmark in landmarks::all().iter().filter(|l| inside(l.pos)) {
        if landmark.kind.is_secret() {
            crate::secrets::build(&mut b, landmark);
        } else {
            build_landmark(&mut b, landmark);
        }
    }
    for &c in landmarks::cairns().iter().filter(|&&c| inside(c)) {
        cairn(&mut b, c);
    }
    Props { mesh: (b.verts, b.idxs), glow: (b.glow_verts, b.glow_idxs), solids: b.solids }
}

/// A little stack of flat stones, largest at the bottom.
fn cairn(b: &mut Builder, p: Vec2) {
    let seed = hash(p.x, p.y) * 100.0;
    let mut y = ground(p, 1.5) - 0.3;
    let stones = 4 + (seed as usize % 3);
    for k in 0..stones {
        let r = 1.3 - k as f32 * 0.22;
        let off = vec3(hash(seed, k as f32) - 0.5, 0.0, hash(k as f32, seed) - 0.5) * 0.3;
        b.boulder(vec3(p.x, y + r * 0.35, p.y) + off, r, seed + k as f32, if k % 2 == 0 { STONE } else { [0.5, 0.49, 0.45, 1.0] });
        y += r * 0.6;
    }
}

fn boulders(b: &mut Builder, lo: Vec2, hi: Vec2) {
    let cell = 60.0;
    let first = |v: f32| ((v + TERRAIN_HALF_SIZE) / cell).ceil() as i32;
    for cz in first(lo.y)..first(hi.y) {
        for cx in first(lo.x)..first(hi.x) {
            let bx = cx as f32 * cell - TERRAIN_HALF_SIZE;
            let bz = cz as f32 * cell - TERRAIN_HALF_SIZE;
            let p = vec2(bx + hash(bx, bz) * cell, bz + hash(bz, bx) * cell);
            let (x, z) = (p.x, p.y);
            let rocky = terrain::biome_mountain(x, z) * 0.5 + terrain::biome_desert(x, z) * 0.25 + terrain::biome_tundra(x, z) * 0.3 + 0.04;
            if hash(x * 1.3, z * 0.7) > rocky {
                continue;
            }
            if terrain::raw_height(x, z) < WATER_LEVEL + 0.5 || crate::farms::occupied(p, 8.0) {
                continue;
            }
            if road::network().and_then(|n| n.nearest(x, z)).is_some_and(|s| s.dist < s.half_width() + 12.0) {
                continue;
            }
            let color = if terrain::biome_desert(x, z) > 0.4 {
                [0.72, 0.5, 0.33, 1.0]
            } else if terrain::biome_tundra(x, z) > 0.4 {
                [0.8, 0.83, 0.88, 1.0]
            } else {
                STONE
            };
            let size = 1.5 + hash(z, x * 2.0) * 5.0;
            b.boulder(vec3(x, terrain_height(x, z) + size * 0.2, z), size, hash(x, z * 3.0) * 100.0, color);
        }
    }
}

/// Pillars and rails where a road runs well above the ground (over water or
/// across a dip), so it reads as a viaduct rather than floating tarmac.
fn raised_road_supports(b: &mut Builder, path: &road::RoadPath, inside: &impl Fn(Vec2) -> bool) {
    let n = path.points.len();
    let network = road::network();
    let hw = path.kind.half_width();
    // Guard rails stand along both edges (behind the kerbs on a circuit)
    let rail_at = if path.circuit() { hw + KERB_WIDTH + 2.0 } else { hw + 1.0 };
    // How far below sample `i`'s deck the ground is, unless a wider path's
    // surface takes over there (a road running onto a circuit)
    let drop_at = |i: usize| {
        let p = path.points[i];
        let covered = network.is_some_and(|net| net.nearest_matching(p.x, p.y, |k| k > path.kind)
            .is_some_and(|s| s.dist < s.half_width() + s.shoulder() + hw));
        let below = terrain::raw_height(p.x, p.y).max(WATER_LEVEL - 4.0);
        (!covered).then(|| (path.heights[i] - below, below))
    };
    // Where the rail post by sample `i` on side `s` stands, if there's one
    let post = |i: usize, s: f32| -> Option<Vec3> {
        let i = i % n;
        if drop_at(i).is_none_or(|(drop, _)| drop < 4.0) {
            return None;
        }
        let p = path.points[i];
        let side = path.smooth_side(i);
        // None on the inside of a bend so tight the rail would crowd its middle
        let turn = path.tangent((i + n - 2) % n).perp_dot(path.tangent((i + 2) % n));
        if path.curvature[i] * rail_at > 0.6 && turn * s > 0.0 {
            return None;
        }
        let edge = p + side * s * rail_at;
        // A gap where another path joins or crosses
        let crossing = network.is_some_and(|net| net.nearest_matching(edge.x, edge.y, |k| k != path.kind)
            .is_some_and(|o| o.dist < o.half_width() + 6.0));
        (!crossing).then(|| vec3(edge.x, path.heights[i], edge.y))
    };
    for i in (0..n).step_by(2) {
        let p = path.points[i];
        if !inside(p) {
            continue;
        }
        let Some((drop, below)) = drop_at(i).filter(|&(d, _)| d >= 4.0) else { continue };
        let t = path.tangent(i);
        let side = vec2(-t.y, t.x);
        let yaw = (-t.x).atan2(-t.y);
        for s in [-1.0, 1.0] {
            let Some(a) = post(i, s) else { continue };
            b.block(a, 0.25, 1.4, 0.25, yaw, RAIL, RAIL);
            // The rail runs post to post, following the bend
            if i + 2 < n || path.closed {
                if let Some(c) = post(i + 2, s) {
                    b.limb(a + Vec3::Y * 1.2, c + Vec3::Y * 1.2, 0.18, 0.18, 4, 0.785, RAIL);
                }
            }
        }
        // Piers every 40 units
        if i % 10 == 0 {
            for s in [-1.0, 1.0] {
                let foot = p + side * s * (hw - 6.0).max(hw * 0.6);
                b.block(vec3(foot.x, below - 2.0, foot.y), 2.2, drop + 1.5, 2.2, yaw, CONCRETE, CONCRETE);
            }
            // Pier cap, kept well under the deck so it can't poke through on a slope
            b.block(vec3(p.x, path.heights[i] - 4.0, p.y), hw - 3.0, 1.8, 2.5, yaw, CONCRETE, CONCRETE);
        }
    }
}

/// Stepped grandstands either side of the start line.
fn grandstands(b: &mut Builder, path: &road::RoadPath) {
    let p0 = path.points[0];
    let t = path.tangent(0);
    let side = vec2(-t.y, t.x);
    let yaw = (-t.x).atan2(-t.y);
    for s in [-1.0_f32, 1.0] {
        for along in [-55.0_f32, 55.0] {
            let centre = p0 + t * along + side * s * (ROAD_HALF_WIDTH + KERB_WIDTH + 24.0);
            let base_y = ground(centre, 20.0);
            // Rows step up away from the track
            for row in 0..4 {
                let offset = side * s * (row as f32 * 3.0);
                let c = centre + offset;
                let seat = if row % 2 == 0 { [0.25, 0.4, 0.75, 1.0] } else { [0.85, 0.85, 0.88, 1.0] };
                b.solid_block(vec3(c.x, base_y, c.y), 1.6, 2.0 + row as f32 * 2.0, 30.0, yaw, CONCRETE, seat);
            }
            // Roof on posts
            let back = centre + side * s * 10.0;
            b.solid_block(vec3(back.x, base_y, back.y), 0.5, 13.0, 30.0, yaw, DARK, DARK);
            let roof = centre + side * s * 4.5;
            b.block(vec3(roof.x, base_y + 13.0, roof.y), 6.5, 0.6, 31.0, yaw, RED, RED);
        }
    }
}

fn build_landmark(b: &mut Builder, l: &Landmark) {
    let p = l.pos;
    let seed = (l.seed % 1000) as f32;
    let yaw = hash(p.x, p.y) * std::f32::consts::TAU;
    match l.kind {
        Kind::Windmill => {
            let y = ground(p, 8.0);
            let base = vec3(p.x, y - 1.0, p.y);
            b.column(base, 7.0, 4.5, 27.0, 8, WHITE);
            b.solid_post(base, 7.0, 27.0);
            b.cone(base + Vec3::Y * 27.0, 5.5, 6.0, 8, DARK);
            // Four sails on a hub facing `yaw`
            let r = Quat::from_rotation_y(yaw);
            let hub = base + Vec3::Y * 24.0 + r * vec3(0.0, 0.0, -6.0);
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
                let dir = r * vec3(a.cos(), a.sin(), 0.0);
                let across = r * vec3(-a.sin(), a.cos(), 0.0) * 2.2;
                let tip = hub + dir * 20.0;
                let root = hub + dir * 3.0;
                b.quad(root, tip, tip + across, root + across, WOOD);
                b.quad(root + across, tip + across, tip, root, [0.85, 0.8, 0.7, 1.0]);
            }
            b.block(hub - Vec3::Y * 1.0, 1.0, 2.0, 1.0, yaw, DARK, DARK);
        }
        Kind::Lighthouse => {
            let y = ground(p, 7.0);
            let mut base = vec3(p.x, y - 1.0, p.y);
            b.block(base, 9.0, 3.0, 9.0, yaw, STONE, STONE);
            b.solid_post(base, 6.0, 40.0);
            base.y += 3.0;
            let bands = 6;
            for k in 0..bands {
                let r0 = 5.5 - k as f32 * 0.4;
                b.column(base, r0, r0 - 0.4, 6.0, 10, if k % 2 == 0 { WHITE } else { RED });
                base.y += 6.0;
            }
            b.block(base, 4.2, 0.8, 4.2, yaw, DARK, DARK);
            b.column(base + Vec3::Y * 0.8, 2.6, 2.6, 4.0, 8, [1.0, 0.92, 0.5, 1.0]);
            b.cone(base + Vec3::Y * 4.8, 3.6, 4.0, 8, RED);
        }
        Kind::RadioMast => {
            let y = ground(p, 6.0);
            let base = vec3(p.x, y - 0.5, p.y);
            b.solid_block(base + vec3(9.0, 0.0, 0.0), 4.0, 4.0, 3.0, yaw, CONCRETE, DARK);
            b.solid_post(base, 2.4, 72.0);
            let segments = 8;
            for k in 0..segments {
                let w0 = 2.4 - k as f32 * 0.25;
                b.column(base + Vec3::Y * (k as f32 * 9.0), w0, w0 - 0.25, 9.0, 4, if k % 2 == 0 { RED } else { WHITE });
            }
            b.cone(base + Vec3::Y * 72.0, 0.4, 8.0, 4, RED);
            // Guy wires as thin slabs
            for k in 0..3 {
                let a = yaw + k as f32 * std::f32::consts::TAU / 3.0;
                let anchor = base + vec3(a.cos(), 0.0, a.sin()) * 30.0;
                let top = base + Vec3::Y * 50.0;
                b.quad(anchor, anchor + Vec3::Y * 0.3, top + Vec3::Y * 0.3, top, DARK);
            }
        }
        Kind::StoneArch => {
            let y = ground(p, 14.0);
            let sand = [0.78, 0.5, 0.32, 1.0];
            let r = Quat::from_rotation_y(yaw);
            for s in [-1.0_f32, 1.0] {
                let foot = vec3(p.x, y - 2.0, p.y) + r * vec3(s * 11.0, 0.0, 0.0);
                b.solid_block(foot, 3.5, 22.0, 4.0, yaw, sand, sand);
            }
            b.block(vec3(p.x, y + 18.0, p.y), 15.0, 6.0, 4.5, yaw, [0.72, 0.45, 0.3, 1.0], sand);
            for k in 0..5 {
                let a = seed + k as f32 * 1.7;
                let q = vec3(p.x + a.cos() * 20.0, 0.0, p.y + a.sin() * 20.0);
                b.boulder(vec3(q.x, terrain_height(q.x, q.z) + 1.0, q.z), 2.0 + (k % 3) as f32, a, sand);
            }
        }
        Kind::Cabin => {
            let y = ground(p, 8.0);
            let base = vec3(p.x, y - 0.5, p.y);
            b.solid_block(base, 6.0, 6.0, 8.0, yaw, WOOD, WOOD);
            b.roof(base + Vec3::Y * 6.0, 6.0, 4.5, 8.0, yaw, [0.25, 0.3, 0.22, 1.0]);
            let chimney = base + Quat::from_rotation_y(yaw) * vec3(3.0, 0.0, 4.0);
            b.block(chimney, 0.9, 12.5, 0.9, yaw, STONE, DARK);
            let woodpile = base + Quat::from_rotation_y(yaw) * vec3(9.0, 0.0, 0.0);
            b.solid_block(woodpile, 1.5, 1.6, 3.0, yaw, [0.55, 0.38, 0.22, 1.0], [0.6, 0.45, 0.28, 1.0]);
        }
        Kind::Village => village(b, p, seed),
        Kind::Cove => {
            // A jetty out over the water from a boathouse, a boat tied up at the end
            let dir = (0..16)
                .map(|k| Vec2::from_angle(k as f32 * std::f32::consts::TAU / 16.0))
                .min_by(|a, b| {
                    let h = |d: Vec2| terrain::raw_height(p.x + d.x * 60.0, p.y + d.y * 60.0);
                    h(*a).total_cmp(&h(*b))
                })
                .unwrap();
            let face = dir.x.atan2(dir.y);
            let deck = terrain::WATER_LEVEL + 1.2;
            for k in 0..10 {
                let c = p + dir * (6.0 + k as f32 * 5.0);
                let bed = terrain::raw_height(c.x, c.y);
                b.block(vec3(c.x, deck - 0.4, c.y), 2.2, 0.4, 2.6, face, WOOD, [0.55, 0.4, 0.25, 1.0]);
                if k % 2 == 0 && bed < deck {
                    for s in [-1.0, 1.0] {
                        let post = c + dir.perp() * s * 2.0;
                        b.column(vec3(post.x, bed - 0.5, post.y), 0.3, 0.3, deck - bed + 1.2, 4, DARK);
                    }
                }
            }
            let end = p + dir * 54.0 + dir.perp() * 4.5;
            b.block(vec3(end.x, terrain::WATER_LEVEL - 0.2, end.y), 1.1, 0.9, 3.2, face, RED, WOOD);
            let house = p - dir * 6.0;
            let y = ground(house, 6.0);
            b.solid_block(vec3(house.x, y - 0.5, house.y), 5.0, 5.0, 4.0, face, [0.4, 0.5, 0.6, 1.0], WOOD);
            b.roof(vec3(house.x, y + 4.5, house.y), 5.0, 3.0, 4.0, face, [0.3, 0.25, 0.22, 1.0]);
        }
        Kind::Viewpoint => {
            // A bench facing the view, a coin telescope and a signpost
            let y = ground(p, 4.0);
            let r = Quat::from_rotation_y(yaw);
            let at = |x: f32, z: f32| vec3(p.x, y, p.y) + r * vec3(x, 0.0, z);
            b.block(at(0.0, 0.0) - Vec3::Y * 0.3, 3.0, 1.2, 0.8, yaw, WOOD, WOOD);
            b.block(at(0.0, 0.9) + Vec3::Y * 0.9, 3.0, 1.4, 0.2, yaw, WOOD, WOOD);
            let scope = at(5.0, -1.0);
            b.column(scope - Vec3::Y * 0.3, 0.25, 0.2, 3.4, 6, DARK);
            b.block(scope + Vec3::Y * 3.0, 0.4, 0.7, 1.2, yaw, [0.2, 0.45, 0.35, 1.0], DARK);
            let sign = at(-5.0, 0.5);
            b.column(sign - Vec3::Y * 0.3, 0.2, 0.2, 4.0, 4, WOOD);
            b.block(sign + Vec3::Y * 3.0, 1.6, 0.9, 0.15, yaw, [0.85, 0.8, 0.65, 1.0], WOOD);
        }
        Kind::Summit => {
            // A cairn of stacked stones, and a flag on a pole beside it
            let y = ground(p, 4.0);
            let mut top = y - 0.4;
            for k in 0..5 {
                let r = 2.6 - k as f32 * 0.45;
                let wobble = vec3(hash(seed, k as f32) - 0.5, 0.0, hash(k as f32, seed) - 0.5) * 0.6;
                b.boulder(vec3(p.x, top + r * 0.45, p.y) + wobble, r, seed + k as f32, if k % 2 == 0 { STONE } else { [0.47, 0.46, 0.44, 1.0] });
                top += r * 0.75;
            }
            let pole = vec3(p.x + 4.0, ground(p + vec2(4.0, 0.0), 0.5) - 0.3, p.y);
            b.column(pole, 0.25, 0.18, 10.0, 4, WOOD);
            let mast = pole + Vec3::Y * 9.8;
            let out = Quat::from_rotation_y(yaw) * vec3(4.0, 0.0, 0.0);
            let down = Vec3::Y * 2.4;
            b.quad(mast, mast + out, mast + out - down, mast - down, RED);
            b.quad(mast - down, mast + out - down, mast + out, mast, RED);
        }
        // Secrets are built by `secrets::build`
        _ => {}
    }
}

/// A cluster of houses round a church, kept off the road.
fn village(b: &mut Builder, centre: Vec2, seed: f32) {
    let clear = |q: Vec2| road::network().and_then(|n| n.nearest(q.x, q.y)).is_none_or(|s| s.dist > s.half_width() + 14.0);
    // Face buildings along the nearest road if there is one
    let face = road::network()
        .and_then(|n| n.paths.iter().flat_map(|path| path.points.iter().enumerate().map(move |(i, q)| (path, i, *q)))
            .min_by(|a, b| a.2.distance_squared(centre).total_cmp(&b.2.distance_squared(centre))))
        .map(|(path, i, _)| { let t = path.tangent(i); (-t.x).atan2(-t.y) })
        .unwrap_or(0.0);

    if clear(centre) {
        let y = ground(centre, 8.0);
        let base = vec3(centre.x, y - 0.5, centre.y);
        b.solid_block(base, 6.0, 9.0, 11.0, face, WHITE, WHITE);
        b.roof(base + Vec3::Y * 9.0, 6.0, 5.0, 11.0, face, ROOF_COLORS[2]);
        let tower = base + Quat::from_rotation_y(face) * vec3(0.0, 0.0, -13.0);
        b.solid_block(tower, 3.5, 22.0, 3.5, face, WHITE, WHITE);
        b.cone(tower + Vec3::Y * 22.0, 4.5, 9.0, 4, ROOF_COLORS[2]);
    }
    let houses = 7 + (seed as usize % 4);
    for k in 0..houses {
        let a = seed + k as f32 * 2.39996; // golden angle spiral
        let dist = 35.0 + (k as f32).sqrt() * 28.0;
        let q = centre + vec2(a.cos(), a.sin()) * dist;
        if !clear(q) {
            continue;
        }
        let h = hash(q.x, q.y);
        let (hx, hz) = (4.0 + h * 2.5, 5.5 + h * 3.0);
        let height = 4.5 + (k % 3) as f32 * 1.5;
        let yaw = face + if k % 2 == 0 { 0.0 } else { std::f32::consts::FRAC_PI_2 } + (h - 0.5) * 0.3;
        let y = ground(q, hz);
        let base = vec3(q.x, y - 0.5, q.y);
        b.solid_block(base, hx, height, hz, yaw, WALL_COLORS[k % 4], WALL_COLORS[k % 4]);
        b.roof(base + Vec3::Y * height, hx, 3.0 + h * 2.0, hz, yaw, ROOF_COLORS[(k + 1) % 4]);
    }
}
