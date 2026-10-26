//! A marker set on the world map, and the way to it: chevrons on the ground
//! ahead of the car pointing the way, a beam of light over the spot, and its
//! distance at the top of the screen.
//!
//! On the world map (M) a cursor picks the spot: the mouse, or the arrow keys
//! (or stick). Click or Enter sets the marker there, snapping to a known
//! place under the cursor; N marks the nearest place not yet found.

use glam::{Vec2, Vec3};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use crate::geometry::Vertex;
use crate::hud::{self, MapView};
use crate::terrain::{self, TERRAIN_HALF_SIZE};
use crate::{landmarks, waterways, Stage};

pub struct Marker {
    pub pos: Vec2,
    pub name: String,
}

/// Where the world map is looking, and its cursor.
pub struct MapControl {
    pub zoom: f32,
    /// World point at the middle of the map.
    pub focus: Vec2,
    pub cursor: Vec2,
    /// Mouse position on screen, once it has moved.
    pub mouse: Option<Vec2>,
}

impl Default for MapControl {
    fn default() -> Self {
        Self { zoom: 1.0, focus: Vec2::ZERO, cursor: Vec2::ZERO, mouse: None }
    }
}

const MAX_ZOOM: f32 = 12.0;
/// The marker is reached (and cleared) this close.
const ARRIVED: f32 = 70.0;
/// A click this many pixels from a known place marks the place itself.
const SNAP_PIXELS: f32 = 18.0;
/// The arrow on the ground this far ahead of the car, turning to point the way.
const ARROW_AHEAD: f32 = 26.0;
/// Then a trail of chevrons from the car toward the marker.
const CHEVRON_START: f32 = 24.0;
const CHEVRON_SPACING: f32 = 12.0;
const CHEVRON_COUNT: usize = 4;
/// The marker's pink (over 1 glows after dark).
const GUIDE_COLOR: [f32; 3] = [1.6, 0.6, 1.3];
/// The beam over the marker.
const BEAM_HEIGHT: f32 = 320.0;
const BEAM_WIDTH: f32 = 14.0;

impl Stage {
    /// The world map's view for a screen this size, and the rectangle it fills.
    pub(crate) fn world_map_view(&self, width: f32, height: f32) -> (MapView, Vec2, Vec2) {
        let (lo, hi) = (Vec2::new(width * 0.03, height * 0.09), Vec2::new(width * 0.97, height * 0.84));
        let fit = (hi - lo).min_element() / (TERRAIN_HALF_SIZE * 2.0);
        let view = MapView { centre: (lo + hi) * 0.5, focus: self.map.focus, scale: fit * self.map.zoom, yaw: None };
        (view, lo, hi)
    }

    /// Open or close the world map. It opens on the marker, else on the car.
    pub(crate) fn toggle_map(&mut self) {
        self.show_map = !self.show_map;
        if self.show_map {
            self.map.cursor = self.marker.as_ref().map_or(self.player_xz(), |m| m.pos);
            self.map.focus = if self.map.zoom > 1.0 { self.map.cursor } else { Vec2::ZERO };
            self.map.mouse = None;
        }
    }

    /// A key pressed with the map open: `Some(quit)` if the map used it.
    pub(crate) fn map_key(&mut self, key: KeyCode) -> Option<bool> {
        match key {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.set_marker(self.map.cursor),
            KeyCode::Backspace | KeyCode::Delete => self.clear_marker(),
            KeyCode::Equal | KeyCode::NumpadAdd | KeyCode::KeyE | KeyCode::PageUp => self.zoom_map(2.0_f32.sqrt(), self.map.cursor),
            KeyCode::Minus | KeyCode::NumpadSubtract | KeyCode::KeyQ | KeyCode::PageDown => self.zoom_map(0.5_f32.sqrt(), self.map.cursor),
            KeyCode::KeyN => self.mark_nearest_unfound(),
            // (The pad's D-pad up and down) zoom in a step, round to all of it
            KeyCode::Tab => {
                let by = if self.map.zoom * 2.0 > MAX_ZOOM { 1.0 / self.map.zoom } else { 2.0 };
                self.zoom_map(by, self.map.cursor);
            }
            KeyCode::KeyH | KeyCode::Home => {
                self.map.cursor = self.player_xz();
                self.map.focus = if self.map.zoom > 1.0 { self.map.cursor } else { Vec2::ZERO };
            }
            KeyCode::Escape => self.show_map = false,
            _ => return None,
        }
        Some(false)
    }

    /// With the map open: the held arrow keys (or the stick) move the
    /// cursor, and the view follows it.
    pub(crate) fn map_update(&mut self, dt: f32, stick: Vec2) {
        let c = &self.controls;
        let keys = Vec2::new((c.right as i32 - c.left as i32) as f32, (c.accelerate as i32 - c.brake as i32) as f32);
        let push = if keys != Vec2::ZERO { keys } else { stick };
        if push.length() > 0.1 {
            // Half the view's width a second
            let span = TERRAIN_HALF_SIZE * 2.0 / self.map.zoom;
            self.map.cursor += push.clamp_length_max(1.0) * span * 0.5 * dt;
            self.map.mouse = None;
        }
        self.map.cursor = self.map.cursor.clamp(Vec2::splat(-TERRAIN_HALF_SIZE), Vec2::splat(TERRAIN_HALF_SIZE));
        self.keep_cursor_in_view();
    }

    /// The mouse moved to `pos` on screen.
    pub(crate) fn map_mouse(&mut self, pos: Vec2) {
        self.map.mouse = Some(pos);
        if self.show_map {
            let (w, h) = self.gpu.size();
            let (view, lo, hi) = self.world_map_view(w, h);
            if pos.cmpge(lo).all() && pos.cmple(hi).all() {
                self.map.cursor = view.unproject(pos);
            }
        }
    }

    /// A click on the map: left sets the marker, right clears it.
    pub(crate) fn map_click(&mut self, button: MouseButton) {
        if !self.show_map || self.map.mouse.is_none() {
            return;
        }
        match button {
            MouseButton::Left => self.set_marker(self.map.cursor),
            MouseButton::Right => self.clear_marker(),
            _ => {}
        }
    }

    /// The scroll wheel over the map zooms about the cursor.
    pub(crate) fn map_scroll(&mut self, y: f32) {
        self.zoom_map(1.25_f32.powf(y), self.map.cursor);
    }

    fn zoom_map(&mut self, by: f32, about: Vec2) {
        let zoom = (self.map.zoom * by).clamp(1.0, MAX_ZOOM);
        // Keep `about` where it is on screen
        let k = self.map.zoom / zoom;
        self.map.focus = about + (self.map.focus - about) * k;
        self.map.zoom = zoom;
        if zoom <= 1.0 {
            self.map.focus = Vec2::ZERO;
        }
        self.keep_cursor_in_view();
    }

    /// Pan so the cursor stays well inside the map.
    fn keep_cursor_in_view(&mut self) {
        let reach = TERRAIN_HALF_SIZE / self.map.zoom * 0.8;
        let off = self.map.cursor - self.map.focus;
        self.map.focus += off - off.clamp(Vec2::splat(-reach), Vec2::splat(reach));
        let room = TERRAIN_HALF_SIZE - TERRAIN_HALF_SIZE / self.map.zoom;
        self.map.focus = self.map.focus.clamp(Vec2::splat(-room), Vec2::splat(room));
    }

    /// Known places, for snapping the marker to and naming it.
    fn known_places(&self) -> Vec<(Vec2, String)> {
        let mut places: Vec<(Vec2, String)> = self.world.circuits.iter()
            .filter(|c| self.progress.circuits.contains(c.name))
            .map(|c| (c.start(), c.name.to_string()))
            .collect();
        places.extend(landmarks::all().iter().filter(|l| self.progress.landmarks.contains(&l.name)).map(|l| (l.pos, l.name.clone())));
        for course in waterways::all().iter().filter(|c| self.progress.places.contains(c.name)) {
            places.extend(course.on_land().iter().step_by(25).map(|&p| (p, course.name.to_string())));
        }
        places
    }

    /// Set the marker at `p`, or on a known place right by it.
    fn set_marker(&mut self, p: Vec2) {
        let (w, h) = self.gpu.size();
        let snap = SNAP_PIXELS / self.world_map_view(w, h).0.scale;
        let place = self.known_places().into_iter()
            .map(|(q, name)| (q.distance(p), q, name))
            .filter(|(d, _, _)| *d < snap)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        self.marker = Some(match place {
            Some((_, q, name)) => Marker { pos: q, name },
            None => Marker { pos: p, name: "MARKER".into() },
        });
        self.map.cursor = self.marker.as_ref().unwrap().pos;
        if let Some(a) = self.audio() { a.trigger_chime(); }
    }

    fn clear_marker(&mut self) {
        self.marker = None;
    }

    /// Mark the nearest circuit, landmark (not a secret), river or canyon
    /// not yet found.
    fn mark_nearest_unfound(&mut self) {
        let me = self.player_xz();
        let mut unfound: Vec<(Vec2, &str)> = self.world.circuits.iter()
            .filter(|c| !self.progress.circuits.contains(c.name))
            .map(|c| (c.start(), if c.stage { "UNKNOWN STAGE" } else { "UNKNOWN CIRCUIT" }))
            .collect();
        unfound.extend(landmarks::all().iter()
            .filter(|l| !l.kind.is_secret() && !self.progress.landmarks.contains(&l.name))
            .map(|l| (l.pos, "UNKNOWN LANDMARK")));
        for course in waterways::all().iter().filter(|c| !self.progress.places.contains(c.name)) {
            let nearest = course.on_land().iter().copied().min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
            unfound.extend(nearest.map(|p| (p, "UNKNOWN WILD PLACE")));
        }
        if let Some((p, name)) = unfound.into_iter().min_by(|a, b| a.0.distance(me).total_cmp(&b.0.distance(me))) {
            self.marker = Some(Marker { pos: p, name: name.into() });
            self.map.cursor = p;
            self.keep_cursor_in_view();
            if let Some(a) = self.audio() { a.trigger_chime(); }
        }
    }

    /// Reaching the marker clears it.
    pub(crate) fn check_marker(&mut self) {
        let Some(m) = &self.marker else { return };
        if m.pos.distance(self.player_xz()) < ARRIVED {
            let text = if m.name == "MARKER" { "MARKER REACHED".to_string() } else { format!("REACHED {}", m.name) };
            self.banner = Some((text, hud::GREEN, 3.0));
            self.marker = None;
            if let Some(a) = self.audio() { a.trigger_chime(); }
        }
    }

    /// The chevrons on the ground ahead of the car pointing to the marker,
    /// and the beam of light over it. Drawn as see-through particles.
    pub(crate) fn guide_geometry(&self, time: f32) -> (Vec<Vertex>, Vec<u16>) {
        let mut verts: Vec<Vertex> = Vec::new();
        let mut idxs: Vec<u16> = Vec::new();
        let Some(m) = &self.marker else { return (verts, idxs) };
        let mut quad = |corners: [Vec3; 4], alpha: [f32; 4]| {
            let base = verts.len() as u16;
            for (p, a) in corners.into_iter().zip(alpha) {
                verts.push(Vertex { pos: p.into(), color: [GUIDE_COLOR[0], GUIDE_COLOR[1], GUIDE_COLOR[2], a] });
            }
            // Both faces, so it shows from either side
            idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3, base, base + 2, base + 1, base, base + 3, base + 2]);
        };
        let me = self.player_xz();
        let to = m.pos - me;
        let dist = to.length();
        let dir = to / dist.max(1e-3);
        let side = dir.perp();
        let ground = |p: Vec2| Vec3::new(p.x, terrain::solid_height(p.x, p.y) + 0.6, p.y);
        if dist > ARRIVED {
            // A big flat arrow just in front of the car, pointing the way
            let heading = self.player.heading();
            let at = me + Vec2::new(heading.x, heading.z).normalize_or_zero() * ARROW_AHEAD;
            let alpha = 0.75 + 0.15 * (time * 3.0).sin();
            let (tail, neck, tip) = (at - dir * 6.0, at + dir * 1.5, at + dir * 7.0);
            quad([ground(tail - side * 1.6), ground(neck - side * 1.6), ground(neck + side * 1.6), ground(tail + side * 1.6)], [alpha; 4]);
            quad([ground(neck - side * 4.8), ground(tip), ground(tip), ground(neck + side * 4.8)], [alpha; 4]);
            // Chevrons running off toward the marker, drifting forward,
            // fading in at the back and out at the front
            let drift = (time * 1.2).fract();
            for k in 0..=CHEVRON_COUNT {
                let f = (k as f32 + drift) / CHEVRON_COUNT as f32;
                let d = CHEVRON_START + CHEVRON_SPACING * (k as f32 + drift);
                if d > dist - ARRIVED * 0.5 {
                    break;
                }
                let at = me + dir * d;
                let alpha = (f * 3.0).min(1.0) * ((1.0 - f) * 3.0).min(1.0) * 0.8;
                let tip = at + dir * 3.5;
                for s in [-1.0, 1.0] {
                    // An arm from the tip back to one side, 2 wide
                    let back = at - dir * 1.5 + side * s * 4.5;
                    let width = (back - tip).normalize_or_zero().perp() * 1.0 * s;
                    quad([ground(tip - width), ground(back - width), ground(back + width), ground(tip + width)], [alpha; 4]);
                }
            }
        }
        // The beam: two crossed panels, bright at the foot, fading upward
        let foot = terrain::solid_height(m.pos.x, m.pos.y) - 1.0;
        let pulse = 0.55 + 0.15 * (time * 2.0).sin();
        for axis in [Vec2::X, Vec2::Y] {
            let (a, b) = (m.pos - axis * BEAM_WIDTH, m.pos + axis * BEAM_WIDTH);
            quad(
                [Vec3::new(a.x, foot, a.y), Vec3::new(b.x, foot, b.y), Vec3::new(b.x, foot + BEAM_HEIGHT, b.y), Vec3::new(a.x, foot + BEAM_HEIGHT, a.y)],
                [pulse, pulse, 0.0, 0.0],
            );
        }
        (verts, idxs)
    }
}
