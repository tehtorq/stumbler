//! Screen-space HUD: blocky 5x7 pixel font and flat panels.

use glam::Vec2;

use crate::geometry::Vertex;

pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
pub const YELLOW: [f32; 4] = [1.0, 0.85, 0.2, 1.0];
/// Secrets: found places and their banners.
pub const GOLD: [f32; 4] = [1.0, 0.72, 0.3, 1.0];
pub const RED: [f32; 4] = [1.0, 0.3, 0.25, 1.0];
pub const GREEN: [f32; 4] = [0.4, 1.0, 0.45, 1.0];
pub const DIM: [f32; 4] = [0.8, 0.8, 0.85, 1.0];
pub const PANEL: [f32; 4] = [0.05, 0.06, 0.1, 0.55];
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.6];

pub const GLYPH_W: usize = 5;
pub const GLYPH_H: usize = 7;

/// 5x7 glyphs, one row per entry, high bit = leftmost pixel.
pub fn glyph(c: char) -> [u8; GLYPH_H] {
    match c.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        ':' => [0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        '/' => [0b00000, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b00000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        '\'' => [0b00100, 0b00100, 0b01000, 0b00000, 0b00000, 0b00000, 0b00000],
        '<' => [0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010],
        '>' => [0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000],
        _ => [0; GLYPH_H],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Collects HUD geometry for one frame (positions in pixels, origin top-left).
pub struct HudBuilder {
    pub verts: Vec<Vertex>,
    pub idxs: Vec<u16>,
}

impl HudBuilder {
    pub fn new() -> Self {
        Self { verts: Vec::new(), idxs: Vec::new() }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let base = self.verts.len() as u16;
        for (px, py) in [(x, y), (x + w, y), (x + w, y + h), (x, y + h)] {
            self.verts.push(Vertex { pos: [px, py, 0.0], color });
        }
        self.idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A straight line `width` pixels thick.
    pub fn line(&mut self, a: Vec2, b: Vec2, width: f32, color: [f32; 4]) {
        let d = (b - a).normalize_or_zero();
        let n = Vec2::new(-d.y, d.x) * width * 0.5;
        let base = self.verts.len() as u16;
        for p in [a - n, b - n, b + n, a + n] {
            self.verts.push(Vertex { pos: [p.x, p.y, 0.0], color });
        }
        self.idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A filled triangle pointing along `dir` (screen space), for map markers.
    pub fn arrow(&mut self, at: Vec2, dir: Vec2, size: f32, color: [f32; 4]) {
        let d = dir.normalize_or_zero();
        let n = Vec2::new(-d.y, d.x);
        let base = self.verts.len() as u16;
        for p in [at + d * size, at - d * size * 0.6 + n * size * 0.7, at - d * size * 0.6 - n * size * 0.7] {
            self.verts.push(Vertex { pos: [p.x, p.y, 0.0], color });
        }
        self.idxs.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Width in pixels of `text` at pixel size `px`.
    pub fn text_width(text: &str, px: f32) -> f32 {
        let n = text.chars().count() as f32;
        (n * (GLYPH_W as f32 + 1.0) - 1.0).max(0.0) * px
    }

    pub fn text_height(px: f32) -> f32 {
        GLYPH_H as f32 * px
    }

    /// Draw text with a drop shadow. `px` is the size of one font pixel.
    pub fn text(&mut self, x: f32, y: f32, px: f32, align: Align, text: &str, color: [f32; 4]) {
        let x = match align {
            Align::Left => x,
            Align::Center => x - Self::text_width(text, px) * 0.5,
            Align::Right => x - Self::text_width(text, px),
        };
        let shadow = (px * 0.5).max(1.0);
        self.glyphs(x + shadow, y + shadow, px, text, SHADOW);
        self.glyphs(x, y, px, text, color);
    }

    fn glyphs(&mut self, x: f32, y: f32, px: f32, text: &str, color: [f32; 4]) {
        for (i, c) in text.chars().enumerate() {
            let gx = x + i as f32 * (GLYPH_W as f32 + 1.0) * px;
            for (row, bits) in glyph(c).iter().enumerate() {
                // Merge horizontal runs into one quad
                let mut col = 0;
                while col < GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                        col += 1;
                        continue;
                    }
                    let start = col;
                    while col < GLYPH_W && bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                        col += 1;
                    }
                    self.rect(gx + start as f32 * px, y + row as f32 * px, (col - start) as f32 * px, px, color);
                }
            }
        }
    }
}

/// Maps world XZ positions onto a screen-space map.
pub struct MapView {
    /// Screen position of `focus`.
    pub centre: Vec2,
    /// World position at the centre of the map.
    pub focus: Vec2,
    /// Pixels per world unit.
    pub scale: f32,
    /// Heading-up minimaps rotate by the player's yaw; `None` is north-up.
    pub yaw: Option<f32>,
}

impl MapView {
    pub fn project(&self, p: Vec2) -> Vec2 {
        let d = p - self.focus;
        match self.yaw {
            Some(yaw) => {
                // Forward is up the screen; the car's right-hand side is screen right
                let fwd = Vec2::new(-yaw.sin(), -yaw.cos());
                let right = Vec2::new(-yaw.cos(), yaw.sin());
                self.centre + Vec2::new(d.dot(right), -d.dot(fwd)) * self.scale
            }
            // North (+Z) up, east (+X) to the right
            None => self.centre + Vec2::new(d.x, -d.y) * self.scale,
        }
    }

    /// Screen direction of a world heading (for arrows).
    pub fn direction(&self, yaw: f32) -> Vec2 {
        let ahead = Vec2::new(-yaw.sin(), -yaw.cos());
        self.project(self.focus + ahead) - self.centre
    }
}

/// Draw a polyline through a map, keeping segments inside `clip` (a screen rect).
pub fn map_polyline(h: &mut HudBuilder, view: &MapView, points: &[Vec2], closed: bool, clip: (Vec2, Vec2), width: f32, color: [f32; 4]) {
    let inside = |p: Vec2| p.x >= clip.0.x && p.y >= clip.0.y && p.x <= clip.1.x && p.y <= clip.1.y;
    let n = points.len();
    let count = if closed { n } else { n.saturating_sub(1) };
    for i in 0..count {
        let a = view.project(points[i]);
        let b = view.project(points[(i + 1) % n]);
        if inside(a) && inside(b) {
            h.line(a, b, width, color);
        }
    }
}

/// Format seconds as M:SS.cc
pub fn format_time(seconds: f32) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0) as u32;
    let rest = seconds - minutes as f32 * 60.0;
    format!("{minutes}:{rest:05.2}")
}

pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "TH",
        (1, _) => "ST",
        (2, _) => "ND",
        (3, _) => "RD",
        _ => "TH",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0.0), "0:00.00");
        assert_eq!(format_time(65.5), "1:05.50");
        assert_eq!(format_time(125.257), "2:05.26");
    }

    #[test]
    fn ordinals() {
        assert_eq!(ordinal(1), "1ST");
        assert_eq!(ordinal(2), "2ND");
        assert_eq!(ordinal(3), "3RD");
        assert_eq!(ordinal(4), "4TH");
        assert_eq!(ordinal(11), "11TH");
    }
}
