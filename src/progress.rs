//! What the player has found and achieved, saved between sessions as a
//! small text file of `kind name [value]` lines.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Default)]
pub struct Progress {
    pub circuits: HashSet<String>,
    pub landmarks: HashSet<String>,
    /// Best race lap per circuit, seconds.
    pub best_laps: HashMap<String, f32>,
    /// Best finishing position per circuit.
    pub best_finish: HashMap<String, usize>,
    pub orbs: HashSet<usize>,
    /// Chosen paint job (index into `car::CAR_COLORS`).
    pub paint: usize,
    /// How quick the rivals are: 0 easy, 1 normal, 2 hard.
    pub difficulty: usize,
    /// The car's setup (index into `physics::SETUPS`).
    pub setup: usize,
    path: Option<PathBuf>,
}

impl Progress {
    /// Load from the save file (or start fresh). STUMBLER_SAVE overrides the path.
    pub fn load() -> Self {
        let path = std::env::var_os("STUMBLER_SAVE").map(PathBuf::from).or_else(default_path);
        let mut p = Self { path: path.clone(), difficulty: 1, ..Default::default() };
        let Some(text) = path.and_then(|f| std::fs::read_to_string(f).ok()) else { return p };
        for line in text.lines() {
            let mut parts = line.splitn(2, ' ');
            let (Some(kind), Some(rest)) = (parts.next(), parts.next()) else { continue };
            match kind {
                "circuit" => { p.circuits.insert(rest.to_string()); }
                "landmark" => { p.landmarks.insert(rest.to_string()); }
                "orb" => { if let Ok(i) = rest.parse() { p.orbs.insert(i); } }
                "paint" => { if let Ok(i) = rest.parse() { p.paint = i; } }
                "difficulty" => { if let Ok(i) = rest.parse::<usize>() { p.difficulty = i.min(2); } }
                "setup" => { if let Ok(i) = rest.parse::<usize>() { p.setup = i.min(crate::physics::SETUPS.len() - 1); } }
                "best" | "finish" => {
                    // Name may contain spaces; the value is the last word
                    let Some((name, value)) = rest.rsplit_once(' ') else { continue };
                    if kind == "best" {
                        if let Ok(v) = value.parse() { p.best_laps.insert(name.to_string(), v); }
                    } else if let Ok(v) = value.parse() {
                        p.best_finish.insert(name.to_string(), v);
                    }
                }
                _ => {}
            }
        }
        p
    }

    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        let mut out = String::new();
        let mut sorted = |kind: &str, items: Vec<String>| {
            let mut items = items;
            items.sort();
            for item in items {
                out.push_str(&format!("{kind} {item}\n"));
            }
        };
        sorted("circuit", self.circuits.iter().cloned().collect());
        sorted("landmark", self.landmarks.iter().cloned().collect());
        sorted("orb", self.orbs.iter().map(|i| i.to_string()).collect());
        sorted("best", self.best_laps.iter().map(|(n, v)| format!("{n} {v:.3}")).collect());
        sorted("finish", self.best_finish.iter().map(|(n, v)| format!("{n} {v}")).collect());
        out.push_str(&format!("paint {}\n", self.paint));
        out.push_str(&format!("difficulty {}\n", self.difficulty));
        out.push_str(&format!("setup {}\n", self.setup));
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, out) {
            eprintln!("could not save progress to {}: {e}", path.display());
        }
    }

    /// Where circuit `name`'s best-lap ghost is kept, beside the save file.
    fn ghost_path(&self, name: &str) -> Option<PathBuf> {
        let file = format!("{}.txt", name.replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
        Some(self.path.as_ref()?.with_file_name("ghosts").join(file))
    }

    /// Keep the ghost of a best lap on circuit `name`.
    pub fn save_ghost(&self, name: &str, time: f32, frames: &[crate::race::GhostFrame]) {
        let Some(path) = self.ghost_path(name) else { return };
        let mut out = format!("{time}\n");
        for f in frames {
            let (p, r) = (f.pos, f.rot);
            out.push_str(&format!("{} {} {} {} {} {} {} {}\n", f.t, p.x, p.y, p.z, r.x, r.y, r.z, r.w));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, out) {
            eprintln!("could not save ghost to {}: {e}", path.display());
        }
    }

    /// The saved ghost for circuit `name`: its lap time and frames.
    pub fn load_ghost(&self, name: &str) -> Option<(f32, Vec<crate::race::GhostFrame>)> {
        let text = std::fs::read_to_string(self.ghost_path(name)?).ok()?;
        parse_ghost(&text)
    }

    /// Record a lap; returns true if it's a new best for the circuit.
    pub fn record_lap(&mut self, circuit: &str, time: f32) -> bool {
        let best = self.best_laps.get(circuit).is_none_or(|&b| time < b);
        if best {
            self.best_laps.insert(circuit.to_string(), time);
        }
        best
    }

    pub fn record_finish(&mut self, circuit: &str, position: usize) {
        let entry = self.best_finish.entry(circuit.to_string()).or_insert(position);
        *entry = (*entry).min(position);
    }
}

fn parse_ghost(text: &str) -> Option<(f32, Vec<crate::race::GhostFrame>)> {
    let mut lines = text.lines();
    let time: f32 = lines.next()?.trim().parse().ok()?;
    let frames: Option<Vec<_>> = lines.map(|line| {
        let v: Vec<f32> = line.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        (v.len() == 8).then(|| crate::race::GhostFrame {
            t: v[0],
            pos: glam::Vec3::new(v[1], v[2], v[3]),
            rot: glam::Quat::from_xyzw(v[4], v[5], v[6], v[7]).normalize(),
        })
    }).collect();
    frames.filter(|f| f.len() > 1).map(|f| (time, f))
}

pub(crate) fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("stumbler").join("progress.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_save_file() {
        let file = std::env::temp_dir().join(format!("stumbler-progress-{}.txt", std::process::id()));
        let mut p = Progress { path: Some(file.clone()), ..Default::default() };
        p.circuits.insert("RED BULL RING".into());
        p.landmarks.insert("MILLER'S REST".into());
        p.orbs.insert(7);
        p.paint = 5;
        assert!(p.record_lap("RED BULL RING", 61.5));
        assert!(!p.record_lap("RED BULL RING", 70.0));
        p.record_finish("RED BULL RING", 2);
        p.save();

        std::env::set_var("STUMBLER_SAVE", &file);
        let q = Progress::load();
        std::env::remove_var("STUMBLER_SAVE");
        let _ = std::fs::remove_file(&file);
        assert!(q.circuits.contains("RED BULL RING"));
        assert!(q.landmarks.contains("MILLER'S REST"));
        assert!(q.orbs.contains(&7));
        assert_eq!(q.best_laps.get("RED BULL RING"), Some(&61.5));
        assert_eq!(q.best_finish.get("RED BULL RING"), Some(&2));
        assert_eq!(q.paint, 5);
    }
}

#[cfg(test)]
mod ghost_tests {
    use super::*;

    #[test]
    fn ghosts_come_back_as_they_were_saved() {
        let dir = std::env::temp_dir().join(format!("stumbler-ghost-test-{}", std::process::id()));
        let p = Progress { path: Some(dir.join("save.txt")), ..Default::default() };
        let frames: Vec<crate::race::GhostFrame> = (0..3).map(|k| crate::race::GhostFrame {
            t: k as f32 * 0.05,
            pos: glam::Vec3::new(k as f32, 1.0, -2.0),
            rot: glam::Quat::from_rotation_y(k as f32 * 0.3),
        }).collect();
        p.save_ghost("RED BULL RING", 61.25, &frames);
        let (time, back) = p.load_ghost("RED BULL RING").expect("saved ghost");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(time, 61.25);
        assert_eq!(back.len(), 3);
        assert!((back[2].pos - frames[2].pos).length() < 1e-4);
        assert!(back[2].rot.angle_between(frames[2].rot) < 1e-3);
    }
}
