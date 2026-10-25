//! Feel settings that can be adjusted without rebuilding: an optional text
//! file of `name value` lines (`#` starts a comment), read once at start.
//! It lives beside the save file as `tuning.txt`, or wherever
//! STUMBLER_TUNING points. Anything not in the file keeps its default.

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Tuning {
    /// How fast held steering keys wind the steering on, and how fast it
    /// comes back (full lock per second).
    pub steer_in: f32,
    pub steer_return: f32,
    /// Camera distance behind the car, and how much lower and closer it
    /// gets at top speed (0 none, 0.1 = 10%).
    pub camera_distance: f32,
    pub camera_drop: f32,
    /// Extra field of view (degrees) at top speed, and under boost.
    pub fov_at_speed: f32,
    pub fov_boost: f32,
    /// Wind streak brightness (0 turns them off).
    pub speed_lines: f32,
    /// Multiplies every rival's pace, on top of the difficulty setting.
    pub rival_pace: f32,
    /// Drift angle (radians) held with the wheel turned fully into it, and
    /// the rear tyres' sideways grip while drifting short of it.
    pub drift_angle: f32,
    pub drift_grip: f32,
    /// Share of the aero drag a car right on another's tail is spared.
    pub slipstream: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            steer_in: 4.5,
            steer_return: 8.0,
            camera_distance: 32.0,
            camera_drop: 0.1,
            fov_at_speed: 20.0,
            fov_boost: 9.0,
            speed_lines: 1.0,
            rival_pace: 1.0,
            drift_angle: 0.55,
            drift_grip: 0.55,
            slipstream: 0.4,
        }
    }
}

impl Tuning {
    fn parse(text: &str) -> Self {
        let mut t = Self::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(name), Some(value)) = (parts.next(), parts.next().and_then(|v| v.parse::<f32>().ok())) else {
                eprintln!("tuning: line {}: expected `name value`", n + 1);
                continue;
            };
            let slot = match name {
                "steer_in" => &mut t.steer_in,
                "steer_return" => &mut t.steer_return,
                "camera_distance" => &mut t.camera_distance,
                "camera_drop" => &mut t.camera_drop,
                "fov_at_speed" => &mut t.fov_at_speed,
                "fov_boost" => &mut t.fov_boost,
                "speed_lines" => &mut t.speed_lines,
                "rival_pace" => &mut t.rival_pace,
                "drift_angle" => &mut t.drift_angle,
                "drift_grip" => &mut t.drift_grip,
                "slipstream" => &mut t.slipstream,
                _ => {
                    eprintln!("tuning: line {}: unknown setting `{name}`", n + 1);
                    continue;
                }
            };
            *slot = value;
        }
        t
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("STUMBLER_TUNING").map(PathBuf::from).or_else(|| {
        let save = std::env::var_os("STUMBLER_SAVE").map(PathBuf::from).or_else(crate::progress::default_path)?;
        Some(save.with_file_name("tuning.txt"))
    })
}

static TUNING: OnceLock<Tuning> = OnceLock::new();

/// The settings in use. Tests always get the defaults.
pub fn get() -> &'static Tuning {
    TUNING.get_or_init(|| {
        if cfg!(test) {
            return Tuning::default();
        }
        match path().and_then(|p| std::fs::read_to_string(&p).ok().map(|text| (p, text))) {
            Some((p, text)) => {
                let t = Tuning::parse(&text);
                eprintln!("tuning: read {}", p.display());
                t
            }
            None => Tuning::default(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::Tuning;

    #[test]
    fn reads_settings_and_keeps_defaults_for_the_rest() {
        let t = Tuning::parse("# feel\nsteer_in 3.0  # slower\nrival_pace 0.9\nnonsense 1\nfov_boost\n");
        assert_eq!(t.steer_in, 3.0);
        assert_eq!(t.rival_pace, 0.9);
        assert_eq!(t.steer_return, Tuning::default().steer_return);
    }
}
