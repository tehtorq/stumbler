//! What the window shows while the world is built on a loading thread.

use std::time::Instant;

use glam::Mat4;

use crate::gpu::{self, Draw, DynamicMesh, Gpu, PipelineKind};
use crate::hud::{self, Align, HudBuilder};

const BACKGROUND: [f32; 4] = [0.05, 0.06, 0.1, 1.0];
const BAR_TRACK: [f32; 4] = [1.0, 1.0, 1.0, 0.12];

pub struct LoadingScreen {
    mesh: DynamicMesh,
    started: Instant,
}

impl LoadingScreen {
    pub fn new(gpu: &Gpu) -> Self {
        Self { mesh: DynamicMesh::new(gpu, "loading"), started: Instant::now() }
    }

    /// The title, what's under way, and a bar sweeping to and fro.
    pub fn draw(&mut self, gpu: &mut Gpu, status: &str) {
        let (width, height) = gpu.size();
        let px = (height / 720.0 * 3.0).max(2.0);
        let t = self.started.elapsed().as_secs_f32();
        let (cx, cy) = (width * 0.5, height * 0.5);

        let mut h = HudBuilder::new();
        h.rect(0.0, 0.0, width, height, BACKGROUND);
        let title_px = px * 3.0;
        h.text(cx, cy - HudBuilder::text_height(title_px) - px * 8.0, title_px, Align::Center, "STUMBLER", hud::YELLOW);
        // Padded so the line doesn't shift as the dots come and go
        let dots = ".".repeat((t * 2.0) as usize % 4);
        h.text(cx, cy, px, Align::Center, &format!("{status}{dots:<3}"), hud::DIM);

        let bar_w = (width * 0.4).min(px * 160.0);
        let (x, y, bar_h) = (cx - bar_w * 0.5, cy + HudBuilder::text_height(px) + px * 6.0, px * 2.0);
        h.rect(x, y, bar_w, bar_h, BAR_TRACK);
        let block = bar_w * 0.25;
        let f = 1.0 - ((t * 0.8).fract() * 2.0 - 1.0).abs();
        let f = f * f * (3.0 - 2.0 * f);
        h.rect(x + (bar_w - block) * f, y, block, bar_h, hud::YELLOW);
        self.mesh.update(gpu, &h.verts, &h.idxs);

        // Only the HUD's screen size matters; the sky behind is covered over
        let globals = gpu::Globals {
            view_proj: Mat4::IDENTITY,
            inv_view_proj: Mat4::IDENTITY,
            screen: [width, height, 0.0, 0.0],
            ..bytemuck::Zeroable::zeroed()
        };
        gpu.render(&globals, &[Draw::new(PipelineKind::Hud, self.mesh.mesh(), Mat4::IDENTITY)]);
    }
}
