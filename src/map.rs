//! The island as a picture, for the maps: the land's own colours, hill-shaded
//! so ridges and valleys read, and water by depth. Built once while the
//! world loads; the maps draw it as a grid of screen-space quads whose
//! corners sample it, so it clips cleanly, turns with a heading-up minimap
//! and zooms smoothly.

use glam::Vec2;

use crate::hud::{HudBuilder, MapView};
use crate::terrain::{self, TERRAIN_HALF_SIZE, WATER_LEVEL};

/// Samples along each side of the full-size picture: about 100 units apart,
/// close enough to show the land's smaller ponds and hills, not alias them.
const SIDE: usize = 768;
/// Halved this many times for the maps zoomed further out.
const LEVELS: usize = 5;
const SHALLOW: [f32; 3] = [0.24, 0.5, 0.62];
const DEEP: [f32; 3] = [0.08, 0.2, 0.38];
/// Beyond the picture: open sea.
const SEA: [f32; 4] = [0.08, 0.2, 0.38, 1.0];

/// One size of the picture: `side` samples a side.
struct Level {
    side: usize,
    colors: Vec<[f32; 4]>,
}

pub struct MapImage {
    /// Full size first, then each half the size of the one before (averaged).
    levels: Vec<Level>,
}

impl MapImage {
    pub fn build() -> Self {
        let step = TERRAIN_HALF_SIZE * 2.0 / (SIDE - 1) as f32;
        let at = |i: usize, j: usize| Vec2::new(i as f32, j as f32) * step - TERRAIN_HALF_SIZE;
        let heights: Vec<f32> = crate::layout::parallel_map((0..SIDE).collect(), |j| {
            (0..SIDE).map(|i| { let p = at(i, j); terrain::raw_height(p.x, p.y) }).collect::<Vec<_>>()
        }).into_iter().flatten().collect();
        let h = |i: usize, j: usize| heights[j.min(SIDE - 1) * SIDE + i.min(SIDE - 1)];
        let colors = crate::layout::parallel_map((0..SIDE).collect(), |j| {
            (0..SIDE).map(|i| {
                let p = at(i, j);
                let y = h(i, j);
                if y < WATER_LEVEL {
                    let deep = ((WATER_LEVEL - y) / 12.0).clamp(0.0, 1.0);
                    let c: [f32; 3] = std::array::from_fn(|k| SHALLOW[k] + (DEEP[k] - SHALLOW[k]) * deep);
                    return [c[0], c[1], c[2], 1.0];
                }
                // Lit from the north-west: slopes facing it brighter. Judged
                // over a few samples, so the land's fine ripples don't streak it
                let r = 2;
                let dx = h(i + r, j) - h(i.saturating_sub(r), j);
                let dz = h(i, j + r) - h(i, j.saturating_sub(r));
                let shade = (1.0 + (dx - dz) / (2.0 * r as f32 * step) * 1.8).clamp(0.6, 1.3);
                let c = terrain::terrain_color(p.x, p.y, y);
                [c[0] * shade, c[1] * shade, c[2] * shade, 1.0]
            }).collect::<Vec<_>>()
        }).into_iter().flatten().collect();
        let mut levels = vec![Level { side: SIDE, colors }];
        while levels.len() < LEVELS {
            let last = levels.last().unwrap();
            let side = last.side / 2;
            let colors = (0..side * side)
                .map(|k| {
                    let (i, j) = (k % side * 2, k / side * 2);
                    let at = |i: usize, j: usize| last.colors[j.min(last.side - 1) * last.side + i.min(last.side - 1)];
                    let four = [at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1)];
                    std::array::from_fn(|c| four.iter().map(|q| q[c]).sum::<f32>() / 4.0)
                })
                .collect();
            levels.push(Level { side, colors });
        }
        Self { levels }
    }

    /// The colour at world point `p` from picture `level`, blended between
    /// the nearest samples.
    fn sample(&self, p: Vec2, level: usize) -> [f32; 4] {
        let Level { side, colors } = &self.levels[level];
        let last = (*side - 1) as f32;
        let f = (p + TERRAIN_HALF_SIZE) / (TERRAIN_HALF_SIZE * 2.0) * last;
        if f.x < 0.0 || f.y < 0.0 || f.x > last || f.y > last {
            return SEA;
        }
        let (i, j) = (f.x.floor() as usize, f.y.floor() as usize);
        let (i1, j1) = ((i + 1).min(side - 1), (j + 1).min(side - 1));
        let (tx, tz) = (f.x - i as f32, f.y - j as f32);
        let c = |i: usize, j: usize| colors[j * side + i];
        std::array::from_fn(|k| {
            let top = c(i, j)[k] + (c(i1, j)[k] - c(i, j)[k]) * tx;
            let bottom = c(i, j1)[k] + (c(i1, j1)[k] - c(i, j1)[k]) * tx;
            top + (bottom - top) * tz
        })
    }

    /// Fill the screen rectangle `lo`..`hi` with the island as `view` sees it,
    /// in a grid of `cells` quads along the longer side, dimmed by `dim`.
    pub fn draw(&self, h: &mut HudBuilder, view: &MapView, lo: Vec2, hi: Vec2, cells: usize, dim: f32) {
        let size = hi - lo;
        let (nx, ny) = if size.x >= size.y {
            (cells, ((cells as f32 * size.y / size.x).ceil() as usize).max(1))
        } else {
            (((cells as f32 * size.x / size.y).ceil() as usize).max(1), cells)
        };
        // The picture as fine as the grid can show without aliasing: one
        // sample about every grid cell
        let cell = size.x / nx as f32 / view.scale;
        let spacing = TERRAIN_HALF_SIZE * 2.0 / SIDE as f32;
        let level = ((cell / spacing).log2().ceil().max(0.0) as usize).min(LEVELS - 1);
        let corners: Vec<(Vec2, [f32; 4])> = (0..=ny)
            .flat_map(|j| (0..=nx).map(move |i| lo + size * Vec2::new(i as f32 / nx as f32, j as f32 / ny as f32)))
            .map(|s| {
                let c = self.sample(view.unproject(s), level);
                (s, [c[0] * dim, c[1] * dim, c[2] * dim, 1.0])
            })
            .collect();
        let row = nx + 1;
        for j in 0..ny {
            for i in 0..nx {
                let k = j * row + i;
                let q = [corners[k], corners[k + 1], corners[k + row + 1], corners[k + row]];
                h.quad_colored(q.map(|c| c.0), q.map(|c| c.1));
            }
        }
    }
}
