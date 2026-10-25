//! Low-poly car meshes. Car-local space: -Z forward, +Y up, origin at the
//! centre of mass (roughly the belt line).

use crate::geometry::{Vertex, push_box, push_quad};

pub const WHEEL_RADIUS: f32 = 1.2;
const WHEEL_WIDTH: f32 = 1.4;
const WHEEL_SIDES: usize = 10;

/// 4 wheel mount positions in car-local space: FL, FR, RL, RR
pub const WHEEL_MOUNTS: [[f32; 3]; 4] = [
    [-2.1, -0.2, -2.6], // front-left
    [ 2.1, -0.2, -2.6], // front-right
    [-2.1, -0.2,  2.6], // rear-left
    [ 2.1, -0.2,  2.6], // rear-right
];

/// Paint jobs, in the order exploring unlocks them.
pub const CAR_COLORS: [[f32; 3]; 14] = [
    [0.9, 0.15, 0.1],   // red
    [0.1, 0.4, 0.9],    // blue
    [0.1, 0.8, 0.2],    // green
    [0.95, 0.7, 0.1],   // yellow
    [0.92, 0.92, 0.9],  // white
    [0.6, 0.1, 0.8],    // purple
    [0.1, 0.8, 0.8],    // cyan
    [0.95, 0.5, 0.1],   // orange
    [0.95, 0.4, 0.65],  // pink
    [0.05, 0.5, 0.45],  // teal
    [0.6, 0.9, 0.15],   // lime
    [0.65, 0.68, 0.72], // silver
    [0.2, 0.2, 0.2],    // dark grey
    [0.85, 0.65, 0.2],  // gold
];

const GLASS: [f32; 4] = [0.12, 0.16, 0.24, 1.0];
const TRIM: [f32; 4] = [0.12, 0.12, 0.13, 1.0];
const HEADLIGHT: [f32; 4] = [1.0, 0.95, 0.75, 1.0];
const TAILLIGHT: [f32; 4] = [0.9, 0.08, 0.08, 1.0];

fn rgba(c: [f32; 3], k: f32) -> [f32; 4] {
    [(c[0] * k).min(1.0), (c[1] * k).min(1.0), (c[2] * k).min(1.0), 1.0]
}

/// One cross-section of a loft: z position and a symmetric outline in (x, y),
/// listed for the right-hand half from bottom to top (mirrored automatically).
struct Section {
    z: f32,
    half: Vec<[f32; 2]>,
}

impl Section {
    /// Full closed outline: right half bottom→top, then left half top→bottom.
    fn outline(&self) -> Vec<[f32; 3]> {
        let right = self.half.iter().map(|&[x, y]| [x, y, self.z]);
        let left = self.half.iter().rev().map(|&[x, y]| [-x, y, self.z]);
        right.chain(left).collect()
    }
}

/// Connect consecutive sections with quads. `color(span, edge)` picks each face colour.
fn loft(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    sections: &[Section],
    cap_ends: bool,
    color: impl Fn(usize, usize, usize) -> [f32; 4],
) {
    let outlines: Vec<Vec<[f32; 3]>> = sections.iter().map(Section::outline).collect();
    let edges = outlines[0].len();
    for span in 0..outlines.len() - 1 {
        let (a, b) = (&outlines[span], &outlines[span + 1]);
        for e in 0..edges {
            let e2 = (e + 1) % edges;
            push_quad(vertices, indices, a[e], a[e2], b[e2], b[e], color(span, e, edges));
        }
    }
    if cap_ends {
        for (outline, c) in [(&outlines[0], color(usize::MAX, 0, edges)), (outlines.last().unwrap(), color(usize::MAX - 1, 0, edges))] {
            let base = vertices.len() as u16;
            vertices.extend(outline.iter().map(|&pos| Vertex { pos, color: c }));
            for k in 1..outline.len() as u16 - 1 {
                indices.extend_from_slice(&[base, base + k, base + k + 1]);
            }
        }
    }
}

/// Build the car body mesh with the given color index.
pub fn generate_body_mesh(color_idx: usize) -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let c = CAR_COLORS[color_idx % CAR_COLORS.len()];
    let body = rgba(c, 1.0);
    let body_light = rgba(c, 1.15);
    let body_dark = rgba(c, 0.7);

    // Lower body: a bevelled wedge, low nose rising to a squared-off tail.
    // Outline per section: [skirt, side, shoulder bevel, top edge]
    let lower = |z: f32, bottom: f32, top: f32, hw: f32| Section {
        z,
        half: vec![[hw - 0.25, bottom], [hw, bottom + 0.35], [hw, top - 0.35], [hw - 0.4, top]],
    };
    let lower_sections = [
        lower(-4.7, -1.15, -0.45, 1.8),
        lower(-3.9, -1.25, -0.05, 2.15),
        lower(-1.6, -1.3, 0.15, 2.25),
        lower(2.6, -1.3, 0.2, 2.25),
        lower(4.2, -1.2, 0.1, 2.1),
    ];
    // Edges (right half then mirrored): 0 bottom, 1 lower bevel, 2 side, 3 shoulder, 4 top, 5.. mirrored
    loft(&mut vertices, &mut indices, &lower_sections, true, |span, e, n| {
        if span >= usize::MAX - 1 {
            return if span == usize::MAX { body_dark } else { body_dark };
        }
        let mirrored = if e < n / 2 { e } else { n - 1 - e };
        match mirrored {
            0 if e == n - 1 => TRIM,          // underside
            0 | 1 => TRIM,                    // skirt
            2 => body,                        // side
            3 => body_light,                  // shoulder bevel
            _ => body_light,                  // hood / deck top
        }
    });

    // Cabin: glass all round with a body-coloured roof
    let cabin = |z: f32, top: f32, hw_bottom: f32, hw_top: f32| Section {
        z,
        half: vec![[hw_bottom, 0.1], [hw_top, top]],
    };
    let cabin_sections = [
        cabin(-1.7, 0.18, 1.95, 1.9),
        cabin(-0.3, 1.25, 1.95, 1.45),
        cabin(1.6, 1.25, 1.95, 1.45),
        cabin(2.9, 0.25, 1.95, 1.9),
    ];
    loft(&mut vertices, &mut indices, &cabin_sections, false, |span, e, n| {
        let top_edge = e == n / 2 - 1;
        if top_edge && span == 1 { body } else if top_edge { GLASS } else if e == n - 1 { body } else { GLASS }
    });

    // Rear wing on two struts
    for x in [-1.3_f32, 1.3] {
        push_box(&mut vertices, &mut indices, x, 0.55, 3.8, 0.12, 0.45, 0.2, [TRIM; 6]);
    }
    push_box(&mut vertices, &mut indices, 0.0, 1.05, 3.9, 2.2, 0.08, 0.5,
        [body_light, TRIM, body, body, body_dark, body_dark]);

    // Lights: flush panels on the nose and tail
    for x in [-1.35_f32, 1.35] {
        push_quad(&mut vertices, &mut indices,
            [x - 0.45, -0.85, -4.72], [x + 0.45, -0.85, -4.72], [x + 0.45, -0.6, -4.72], [x - 0.45, -0.6, -4.72], HEADLIGHT);
        push_quad(&mut vertices, &mut indices,
            [x + 0.5, -0.55, 4.22], [x - 0.5, -0.55, 4.22], [x - 0.5, -0.25, 4.22], [x + 0.5, -0.25, 4.22], TAILLIGHT);
    }
    // Front splitter
    push_box(&mut vertices, &mut indices, 0.0, -1.2, -4.75, 1.9, 0.06, 0.25, [TRIM; 6]);

    (vertices, indices)
}

/// The brake lights, just proud of the tail lights, drawn only while braking
/// (colours above 1 mark them as lights: shown at full strength, day or night).
pub fn generate_brake_light_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let glow = [2.4, 0.25, 0.15, 1.0];
    for x in [-1.35_f32, 1.35] {
        push_quad(&mut vertices, &mut indices,
            [x + 0.55, -0.58, 4.25], [x - 0.55, -0.58, 4.25], [x - 0.55, -0.22, 4.25], [x + 0.55, -0.22, 4.25], glow);
    }
    (vertices, indices)
}

/// Build a faceted wheel: tyre tread, sidewalls and a raised hub, axis along X.
pub fn generate_wheel_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    let tread: [f32; 4] = [0.07, 0.07, 0.08, 1.0];
    let sidewall: [f32; 4] = [0.11, 0.11, 0.12, 1.0];
    let rim: [f32; 4] = [0.75, 0.75, 0.78, 1.0];
    let hub: [f32; 4] = [0.45, 0.45, 0.48, 1.0];

    let hw = WHEEL_WIDTH * 0.5;
    let ring = |r: f32, i: usize| {
        let a = std::f32::consts::TAU * (i % WHEEL_SIDES) as f32 / WHEEL_SIDES as f32;
        (a.cos() * r, a.sin() * r)
    };

    for i in 0..WHEEL_SIDES {
        let (y0, z0) = ring(WHEEL_RADIUS, i);
        let (y1, z1) = ring(WHEEL_RADIUS, i + 1);
        push_quad(&mut vertices, &mut indices, [-hw, y0, z0], [hw, y0, z0], [hw, y1, z1], [-hw, y1, z1], tread);

        for side in [-1.0_f32, 1.0] {
            let x = side * hw;
            let xr = side * (hw + 0.08);
            let (ry0, rz0) = ring(WHEEL_RADIUS * 0.62, i);
            let (ry1, rz1) = ring(WHEEL_RADIUS * 0.62, i + 1);
            // Sidewall ring from tread edge to rim, then rim face
            push_quad(&mut vertices, &mut indices, [x, y0, z0], [x, y1, z1], [x, ry1, rz1], [x, ry0, rz0], sidewall);
            push_quad(&mut vertices, &mut indices, [x, ry0, rz0], [x, ry1, rz1], [xr, ry1 * 0.5, rz1 * 0.5], [xr, ry0 * 0.5, rz0 * 0.5],
                if i % 2 == 0 { rim } else { hub });
            let base = vertices.len() as u16;
            vertices.push(Vertex { pos: [xr, 0.0, 0.0], color: hub });
            vertices.push(Vertex { pos: [xr, ry0 * 0.5, rz0 * 0.5], color: hub });
            vertices.push(Vertex { pos: [xr, ry1 * 0.5, rz1 * 0.5], color: hub });
            indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    }

    (vertices, indices)
}
