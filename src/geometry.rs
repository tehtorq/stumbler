#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

pub fn push_quad(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    color: [f32; 4],
) {
    let base = vertices.len() as u16;
    vertices.push(Vertex { pos: p0, color });
    vertices.push(Vertex { pos: p1, color });
    vertices.push(Vertex { pos: p2, color });
    vertices.push(Vertex { pos: p3, color });
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Axis-aligned box centered at (cx,cy,cz) with half-extents (hx,hy,hz).
/// Each face gets its own color from `colors` [top, bottom, front, back, left, right].
pub fn push_box(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    cx: f32, cy: f32, cz: f32,
    hx: f32, hy: f32, hz: f32,
    colors: [[f32; 4]; 6],
) {
    let (x0, x1) = (cx - hx, cx + hx);
    let (y0, y1) = (cy - hy, cy + hy);
    let (z0, z1) = (cz - hz, cz + hz);

    // Top (+Y)
    push_quad(vertices, indices,
        [x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1], colors[0]);
    // Bottom (-Y)
    push_quad(vertices, indices,
        [x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0], colors[1]);
    // Front (-Z)
    push_quad(vertices, indices,
        [x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], colors[2]);
    // Back (+Z)
    push_quad(vertices, indices,
        [x1, y0, z1], [x0, y0, z1], [x0, y1, z1], [x1, y1, z1], colors[3]);
    // Left (-X)
    push_quad(vertices, indices,
        [x0, y0, z1], [x0, y0, z0], [x0, y1, z0], [x0, y1, z1], colors[4]);
    // Right (+X)
    push_quad(vertices, indices,
        [x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0], colors[5]);
}
