//! The debug menu (]): jump the car to any circuit, landmark or town (or
//! under the biplane), drop it anywhere by clicking the world map, set the
//! time and weather, and change the graphics settings live.
//!
//! Up and down pick a row, left and right change it, Enter acts on it, and
//! ] or Escape closes the menu.

use glam::Vec2;
use winit::keyboard::KeyCode;

use crate::hud::{self, Align, HudBuilder};
use crate::{landmarks, towns, Stage};

/// Graphics settings the menu changes, read by the renderer every frame.
pub struct Graphics {
    pub ao: bool,
    /// Index into `BLOOM_LEVELS`.
    pub bloom: usize,
    pub shadows: bool,
    pub exposure: f32,
    pub vignette: bool,
    /// Streetlight brightness, as a share of normal.
    pub streetlights: f32,
    /// Haze density, as a share of normal.
    pub fog: f32,
    pub hud: bool,
    /// The frame rate in the corner.
    pub fps: bool,
    pub vsync: bool,
    /// Index into `RESOLUTIONS`.
    pub resolution: usize,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            ao: true,
            bloom: 2,
            shadows: true,
            exposure: 1.0,
            vignette: true,
            streetlights: 1.0,
            fog: 1.0,
            // Dev aid: STUMBLER_NO_HUD=1 for clean screenshots
            hud: std::env::var_os("STUMBLER_NO_HUD").is_none(),
            fps: true,
            // STUMBLER_VSYNC=1 starts with it on (see `Gpu::new`)
            vsync: std::env::var_os("STUMBLER_VSYNC").is_some(),
            resolution: RESOLUTIONS.len() - 1,
        }
    }
}

const BLOOM_LEVELS: [(&str, f32); 4] = [("OFF", 0.0), ("LOW", 0.06), ("NORMAL", 0.12), ("HIGH", 0.25)];
/// The world drawn at these shares of the window's size, then scaled up to it.
const RESOLUTIONS: [f32; 6] = [0.5, 0.6, 0.67, 0.75, 0.85, 1.0];
/// How strongly AO darkens creases when it's on.
const AO_STRENGTH: f32 = 0.85;

impl Graphics {
    /// For the post pass: AO strength, bloom strength, exposure, vignette.
    pub fn post(&self) -> [f32; 4] {
        [if self.ao { AO_STRENGTH } else { 0.0 }, BLOOM_LEVELS[self.bloom].1, self.exposure, if self.vignette { 1.0 } else { 0.0 }]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Circuit,
    Landmark,
    Town,
    Biplane,
    MapDrop,
    Time,
    Freeze,
    Weather,
    Ao,
    Bloom,
    Shadows,
    Exposure,
    Vignette,
    Streetlights,
    Fog,
    Resolution,
    Vsync,
    Fps,
    Hud,
}

const ROWS: [Row; 19] = [
    Row::Circuit, Row::Landmark, Row::Town, Row::Biplane, Row::MapDrop,
    Row::Time, Row::Freeze, Row::Weather,
    Row::Ao, Row::Bloom, Row::Shadows, Row::Exposure, Row::Vignette, Row::Streetlights, Row::Fog,
    Row::Resolution, Row::Vsync, Row::Fps, Row::Hud,
];

/// Headings drawn above these rows.
fn heading(row: Row) -> Option<&'static str> {
    match row {
        Row::Circuit => Some("TELEPORT"),
        Row::Time => Some("WORLD"),
        Row::Ao => Some("GRAPHICS"),
        Row::Resolution => Some("DISPLAY"),
        _ => None,
    }
}

#[derive(Default)]
pub struct DebugMenu {
    pub open: bool,
    row: usize,
    circuit: usize,
    landmark: usize,
    town: usize,
    /// While set, clicking the world map (or Enter on it) drops the car there.
    pub map_drop: bool,
    pub freeze_time: bool,
    pub graphics: Graphics,
}

/// One step of the time of day: a quarter of an hour.
const TIME_STEP: f32 = 1.0 / 96.0;

fn cycle(i: usize, by: i32, len: usize) -> usize {
    if len == 0 { 0 } else { (i as i32 + by).rem_euclid(len as i32) as usize }
}

impl Stage {
    /// A key pressed with the debug menu open. Returns true if it used it.
    pub(crate) fn debug_key(&mut self, key: KeyCode) -> bool {
        let row = ROWS[self.debug.row];
        let step = match key {
            KeyCode::ArrowLeft | KeyCode::KeyA => -1,
            KeyCode::ArrowRight | KeyCode::KeyD => 1,
            _ => 0,
        };
        match key {
            KeyCode::BracketRight | KeyCode::Escape => self.debug.open = false,
            KeyCode::ArrowUp | KeyCode::KeyW => self.debug.row = cycle(self.debug.row, -1, ROWS.len()),
            KeyCode::ArrowDown | KeyCode::KeyS => self.debug.row = cycle(self.debug.row, 1, ROWS.len()),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.debug_act(row, 1),
            _ if step != 0 => self.debug_adjust(row, step),
            _ => return false,
        }
        true
    }

    /// Enter on a row: jump there, or flip it.
    fn debug_act(&mut self, row: Row, step: i32) {
        match row {
            Row::Circuit => {
                let i = self.debug.circuit.min(self.world.circuits.len() - 1);
                let c = &self.world.circuits[i];
                let track = &self.ai_tracks[i];
                let (pos, yaw) = if c.stage { track.pose(c.start_idx.saturating_sub(12), -6.0) } else { track.pose(track.len() - 12, -10.0) };
                self.teleport(pos, yaw, c.name.to_string());
            }
            Row::Landmark => {
                if let Some(l) = landmarks::all().get(self.debug.landmark) {
                    let (pos, yaw) = crate::trail_start(&l.name).unwrap_or((l.pos + Vec2::new(0.0, 60.0), 0.0));
                    self.teleport(pos, yaw, l.name.clone());
                }
            }
            Row::Town => {
                if let Some(t) = towns::all().get(self.debug.town) {
                    let (pos, yaw) = towns::street_start(&t.name).unwrap_or((t.centre, 0.0));
                    self.teleport(pos, yaw, t.name.clone());
                }
            }
            Row::Biplane => {
                let yaw = self.player.yaw();
                self.teleport(self.biplane.xz(), yaw, "UNDER THE BIPLANE".into());
            }
            _ => self.debug_adjust(row, step),
        }
    }

    /// Left or right on a row.
    fn debug_adjust(&mut self, row: Row, step: i32) {
        let d = &mut self.debug;
        let g = &mut d.graphics;
        match row {
            Row::Circuit => d.circuit = cycle(d.circuit, step, self.world.circuits.len()),
            Row::Landmark => d.landmark = cycle(d.landmark, step, landmarks::all().len()),
            Row::Town => d.town = cycle(d.town, step, towns::all().len()),
            Row::Biplane => {}
            Row::MapDrop => d.map_drop = !d.map_drop,
            Row::Time => self.day_time = (self.day_time + step as f32 * TIME_STEP).rem_euclid(1.0),
            Row::Freeze => d.freeze_time = !d.freeze_time,
            Row::Weather => {
                let next = match self.weather.forced() {
                    None => Some(0.0),
                    Some(r) if r < 0.5 => Some(1.0),
                    Some(_) => None,
                };
                // (Left goes round the other way)
                let next = if step < 0 { match self.weather.forced() { None => Some(1.0), Some(r) if r > 0.5 => Some(0.0), _ => None } } else { next };
                self.weather.force(next);
            }
            Row::Ao => g.ao = !g.ao,
            Row::Bloom => g.bloom = cycle(g.bloom, step, BLOOM_LEVELS.len()),
            Row::Shadows => g.shadows = !g.shadows,
            Row::Exposure => g.exposure = (g.exposure + step as f32 * 0.1).clamp(0.3, 2.5),
            Row::Vignette => g.vignette = !g.vignette,
            Row::Streetlights => g.streetlights = (g.streetlights + step as f32 * 0.25).clamp(0.0, 3.0),
            Row::Fog => g.fog = (g.fog + step as f32 * 0.25).clamp(0.0, 3.0),
            Row::Resolution => {
                g.resolution = (g.resolution as i32 + step).clamp(0, RESOLUTIONS.len() as i32 - 1) as usize;
                self.gpu.set_render_scale(RESOLUTIONS[g.resolution]);
            }
            Row::Vsync => {
                g.vsync = !g.vsync;
                self.gpu.set_vsync(g.vsync);
            }
            Row::Fps => g.fps = !g.fps,
            Row::Hud => g.hud = !g.hud,
        }
    }

    /// Put the car down at `pos` facing `yaw`, leaving any race.
    pub(crate) fn teleport(&mut self, pos: Vec2, yaw: f32, label: String) {
        if self.race.is_some() {
            self.end_race();
        }
        self.player.place(&crate::WorldGround, pos.x, pos.y, yaw);
        self.camera.snap(&self.player);
        self.skids.lift(0);
        self.banner = Some((label, hud::WHITE, 2.0));
    }

    fn debug_value(&self, row: Row) -> String {
        let d = &self.debug;
        let g = &d.graphics;
        let on = |b: bool| if b { "ON" } else { "OFF" }.to_string();
        let of = |i: usize, n: usize| format!("{}/{}", i + 1, n);
        match row {
            Row::Circuit => {
                let i = d.circuit.min(self.world.circuits.len() - 1);
                format!("{}  {}", self.world.circuits[i].name, of(i, self.world.circuits.len()))
            }
            Row::Landmark => landmarks::all().get(d.landmark).map_or("-".into(), |l| format!("{}  {}", l.name, of(d.landmark, landmarks::all().len()))),
            Row::Town => towns::all().get(d.town).map_or("-".into(), |t| format!("{}  {}", t.name, of(d.town, towns::all().len()))),
            Row::Biplane => "ENTER".into(),
            Row::MapDrop => on(d.map_drop),
            Row::Time => {
                let minutes = (self.day_time * 24.0 * 60.0).round() as i32 % (24 * 60);
                format!("{:02}:{:02}", minutes / 60, minutes % 60)
            }
            Row::Freeze => on(d.freeze_time),
            Row::Weather => match self.weather.forced() {
                None => "AUTO".into(),
                Some(r) if r < 0.5 => "DRY".into(),
                Some(_) => "RAIN".into(),
            },
            Row::Ao => on(g.ao),
            Row::Bloom => BLOOM_LEVELS[g.bloom].0.into(),
            Row::Shadows => on(g.shadows),
            Row::Exposure => format!("{:.1}", g.exposure),
            Row::Vignette => on(g.vignette),
            Row::Streetlights => format!("X{:.2}", g.streetlights),
            Row::Fog => format!("X{:.2}", g.fog),
            Row::Resolution => {
                let (w, h) = self.gpu.render_size();
                format!("{w}X{h}")
            }
            Row::Vsync => on(self.gpu.vsync()),
            Row::Fps => on(g.fps),
            Row::Hud => on(g.hud),
        }
    }

    /// The menu panel, down the left of the screen.
    pub(crate) fn debug_menu(&self, back: &mut HudBuilder, h: &mut HudBuilder, height: f32, px: f32) {
        let size = px * 0.8;
        let line = HudBuilder::text_height(size) + px * 2.2;
        let headings = ROWS.iter().filter(|&&r| heading(r).is_some()).count();
        let (x, w) = (px * 6.0, px * 150.0);
        let panel_h = line * (ROWS.len() + headings + 2) as f32 + px * 4.0;
        let mut y = ((height - panel_h) * 0.5).max(px * 4.0);
        back.rect(x - px * 3.0, y - px * 3.0, w + px * 6.0, panel_h, [0.02, 0.03, 0.06, 0.88]);
        h.text(x, y, size, Align::Left, "DEBUG", hud::YELLOW);
        h.text(x + w, y, size * 0.8, Align::Right, "UP/DOWN  LEFT/RIGHT  ENTER  ESC", hud::DIM);
        y += line;
        let label_w = px * 42.0;
        for (i, &row) in ROWS.iter().enumerate() {
            if let Some(title) = heading(row) {
                y += line * 0.3;
                h.text(x, y, size * 0.85, Align::Left, title, [0.55, 0.75, 1.0, 1.0]);
                y += line * 0.7;
            }
            let selected = i == self.debug.row;
            if selected {
                back.rect(x - px * 1.5, y - px * 1.0, w + px * 3.0, line, [0.25, 0.3, 0.45, 0.8]);
            }
            let color = if selected { hud::WHITE } else { [0.75, 0.75, 0.8, 1.0] };
            let name = match row {
                Row::Circuit => "CIRCUIT",
                Row::Landmark => "LANDMARK",
                Row::Town => "TOWN",
                Row::Biplane => "BIPLANE",
                Row::MapDrop => "MAP CLICK DROPS CAR",
                Row::Time => "TIME OF DAY",
                Row::Freeze => "FREEZE TIME",
                Row::Weather => "WEATHER",
                Row::Ao => "AMBIENT OCCLUSION",
                Row::Bloom => "BLOOM",
                Row::Shadows => "SHADOWS",
                Row::Exposure => "EXPOSURE",
                Row::Vignette => "VIGNETTE",
                Row::Streetlights => "STREETLIGHTS",
                Row::Fog => "HAZE",
                Row::Resolution => "RESOLUTION",
                Row::Vsync => "V-SYNC",
                Row::Fps => "FPS COUNTER",
                Row::Hud => "HUD",
            };
            h.text(x, y, size, Align::Left, name, color);
            let mut value = self.debug_value(row);
            // Long names are cut to fit the panel
            while HudBuilder::text_width(&value, size) > w - label_w - px * 6.0 && value.len() > 4 {
                value.pop();
            }
            h.text(x + w, y, size, Align::Right, &value, if selected { hud::YELLOW } else { color });
            y += line;
        }
    }
}
