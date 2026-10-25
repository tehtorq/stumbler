use glam::{Vec2, vec2};
use crate::geometry::Vertex;
use crate::road::RoadNetwork;
use crate::terrain::terrain_height;

pub struct BoostPad {
    pub pos: Vec2,   // center XZ
    pub dir: Vec2,   // forward direction (normalized)
    pub width: f32,
    pub length: f32,
}

impl BoostPad {
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let p = vec2(x, z) - self.pos;
        let along = p.dot(self.dir);
        let across = p.dot(vec2(-self.dir.y, self.dir.x));
        along.abs() < self.length * 0.5 && across.abs() < self.width * 0.5
    }
}

/// Place boost pads along long circuit straights.
pub fn create_boost_pads(network: &RoadNetwork) -> Vec<BoostPad> {
    network
        .straights(1500.0)
        .into_iter()
        .map(|(pos, dir)| BoostPad { pos, dir, width: 16.0, length: 20.0 })
        .collect()
}

/// Build the visual mesh for all boost pads — glowing chevron strips.
pub fn build_boost_mesh(pads: &[BoostPad]) -> (Vec<Vertex>, Vec<u16>) {
    let mut verts = Vec::new();
    let mut idxs = Vec::new();

    let glow_color = [0.2, 0.9, 1.0, 0.9]; // cyan glow
    let chevron_color = [0.1, 0.6, 0.8, 0.9];

    for pad in pads {
        let right = vec2(-pad.dir.y, pad.dir.x);
        let hw = pad.width * 0.5;
        let hl = pad.length * 0.5;
        let y_offset = 0.3;

        // Main pad surface
        let corners = [
            pad.pos - pad.dir * hl - right * hw,
            pad.pos - pad.dir * hl + right * hw,
            pad.pos + pad.dir * hl + right * hw,
            pad.pos + pad.dir * hl - right * hw,
        ];

        let base = verts.len() as u16;
        for c in &corners {
            let y = terrain_height(c.x, c.y) + y_offset;
            verts.push(Vertex { pos: [c.x, y, c.y], color: glow_color });
        }
        // CW winding for LH projection (tl, tr, br / tl, br, bl)
        idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

        // Chevron arrows (3 arrows pointing in pad.dir)
        for i in 0..3 {
            let offset = pad.dir * (-hl + hl * 0.5 + i as f32 * hl * 0.6);
            let tip = pad.pos + offset + pad.dir * 3.0;
            let left = pad.pos + offset - right * hw * 0.4;
            let right_pt = pad.pos + offset + right * hw * 0.4;

            let base = verts.len() as u16;
            let ty = terrain_height(tip.x, tip.y) + y_offset + 0.1;
            let ly = terrain_height(left.x, left.y) + y_offset + 0.1;
            let ry = terrain_height(right_pt.x, right_pt.y) + y_offset + 0.1;
            verts.push(Vertex { pos: [tip.x, ty, tip.y], color: chevron_color });
            verts.push(Vertex { pos: [left.x, ly, left.y], color: chevron_color });
            verts.push(Vertex { pos: [right_pt.x, ry, right_pt.y], color: chevron_color });
            idxs.extend_from_slice(&[base, base + 1, base + 2]);
            idxs.extend_from_slice(&[base, base + 2, base + 1]); // both sides
        }
    }

    (verts, idxs)
}
