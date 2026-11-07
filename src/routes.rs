//! The slow part of generating the world, kept between runs. Laying out the
//! circuits and finding every railway, stage and trail takes seconds, but it
//! comes out the same each time for the same program on the same machine, so
//! the routes are saved once and read back after that. Only the routes are
//! kept: everything built from them is put together afresh each run.

use std::path::PathBuf;

use glam::Vec2;

use crate::landmarks::{Kind, Landmark};
use crate::layout::{Circuit, Layout};
use crate::track::Track;

/// What's found once the layout is known (see `world::build_network`).
#[derive(Default)]
pub struct Routes {
    pub rail: Vec<Track>,
    pub stages: Vec<Track>,
    pub stage_names: Vec<&'static str>,
    pub sites: Vec<Landmark>,
    pub trails: Vec<Track>,
}

/// Saved routes, if there are any for this build of the game.
pub fn load() -> Option<(Layout, Routes)> {
    let bytes = std::fs::read(path()?).ok()?;
    parse(&bytes, key()?)
}

pub fn save(names: &[&str], circuits: &[Track], layout_roads: &[Track], routes: &Routes) {
    let (Some(path), Some(key)) = (path(), key()) else { return };
    let bytes = encode(names, circuits, layout_roads, routes, key);
    // Into place in one step, so a run cut short can't leave half a file
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&temp, bytes))
        .and_then(|_| std::fs::rename(&temp, &path));
    if let Err(e) = written {
        eprintln!("could not save routes to {}: {e}", path.display());
        let _ = std::fs::remove_file(&temp);
    }
}

/// STUMBLER_CACHE overrides the path (empty to keep nothing). Tests keep
/// nothing unless told to.
fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("STUMBLER_CACHE") {
        return (!path.is_empty()).then(|| path.into());
    }
    if cfg!(test) {
        return None;
    }
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("stumbler").join("routes.bin"))
}

/// Tells this build on this machine from any other: a rebuild may route
/// differently, and so may another number of cores (see `trails::route_all`).
fn key() -> Option<u64> {
    let exe = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
    let built = exe.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as u64;
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get()) as u64;
    Some(checksum(&[exe.len(), built, cores].map(u64::to_le_bytes).concat()))
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn encode(names: &[&str], circuits: &[Track], layout_roads: &[Track], routes: &Routes, key: u64) -> Vec<u8> {
    let mut w = Writer(key.to_le_bytes().to_vec());
    w.count(names.len());
    names.iter().for_each(|n| w.text(n));
    w.tracks(circuits);
    w.tracks(layout_roads);
    w.tracks(&routes.rail);
    w.tracks(&routes.stages);
    w.count(routes.stage_names.len());
    routes.stage_names.iter().for_each(|n| w.text(n));
    w.count(routes.sites.len());
    for site in &routes.sites {
        w.text(&site.name);
        w.0.push(site.kind as u8);
        w.points(&[site.pos]);
        w.count(site.seed as usize);
    }
    w.tracks(&routes.trails);
    let sum = checksum(&w.0);
    w.0.extend(sum.to_le_bytes());
    w.0
}

fn parse(bytes: &[u8], key: u64) -> Option<(Layout, Routes)> {
    let (body, sum) = bytes.split_last_chunk::<8>()?;
    if checksum(body) != u64::from_le_bytes(*sum) {
        return None;
    }
    let mut r = Reader(body);
    if r.take(8)? != key.to_le_bytes() {
        return None;
    }
    let names = r.many(Reader::text)?;
    let circuits = r.tracks()?;
    if names.len() != circuits.len() {
        return None;
    }
    let circuits = names.into_iter().zip(circuits)
        .map(|(name, track)| Circuit { name: name.leak(), track })
        .collect();
    let roads = r.tracks()?;
    let rail = r.tracks()?;
    let stages = r.tracks()?;
    let stage_names = r.many(Reader::text)?.into_iter().map(|n| &*n.leak()).collect();
    let sites = r.many(|r| {
        let name = r.text()?;
        let kind = *Kind::ALL.get(r.take(1)?[0] as usize)?;
        let pos = *r.points()?.first()?;
        Some(Landmark { kind, pos, name, seed: r.count()? as u32 })
    })?;
    let trails = r.tracks()?;
    r.0.is_empty().then_some((Layout { circuits, roads }, Routes { rail, stages, stage_names, sites, trails }))
}

struct Writer(Vec<u8>);

impl Writer {
    fn count(&mut self, n: usize) {
        self.0.extend((n as u32).to_le_bytes());
    }

    fn text(&mut self, text: &str) {
        self.count(text.len());
        self.0.extend(text.as_bytes());
    }

    fn points(&mut self, points: &[Vec2]) {
        self.count(points.len());
        self.0.extend_from_slice(bytemuck::cast_slice(points));
    }

    fn tracks(&mut self, tracks: &[Track]) {
        self.count(tracks.len());
        for track in tracks {
            self.0.push(track.closed as u8);
            self.points(&track.points_2d);
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn count(&mut self) -> Option<usize> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize)
    }

    fn many<T>(&mut self, mut one: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
        (0..self.count()?).map(|_| one(self)).collect()
    }

    fn text(&mut self) -> Option<String> {
        let n = self.count()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn points(&mut self) -> Option<Vec<Vec2>> {
        let n = self.count()?;
        Some(bytemuck::pod_collect_to_vec(self.take(n.checked_mul(size_of::<Vec2>())?)?))
    }

    fn tracks(&mut self) -> Option<Vec<Track>> {
        self.many(|r| {
            let closed = r.take(1)?[0] != 0;
            Some(Track { points_2d: r.points()?, closed })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(closed: bool, points: &[[f32; 2]]) -> Track {
        Track { points_2d: points.iter().map(|&p| p.into()).collect(), closed }
    }

    fn encoded() -> Vec<u8> {
        let routes = Routes {
            rail: vec![track(true, &[[1.5, -2.5], [f32::MIN_POSITIVE, 4.0]])],
            stages: vec![track(false, &[[0.1, 0.2]])],
            stage_names: vec!["PINE RIDGE"],
            sites: Kind::ALL.iter().enumerate()
                .map(|(i, &kind)| Landmark { kind, pos: Vec2::new(i as f32, -0.3), name: format!("SITE {i}"), seed: u32::MAX - i as u32 })
                .collect(),
            trails: vec![track(false, &[[9.0, 8.0], [7.0, 6.0]]), track(false, &[])],
        };
        encode(&["SPA"], &[track(true, &[[3.0, 4.0]])], &[track(false, &[[5.0, 6.0]])], &routes, 42)
    }

    #[test]
    fn routes_read_back_exactly() {
        let (layout, routes) = parse(&encoded(), 42).expect("reads back");
        assert_eq!(layout.circuits[0].name, "SPA");
        assert!(layout.circuits[0].track.closed);
        assert_eq!(layout.roads[0].points_2d, [Vec2::new(5.0, 6.0)]);
        assert_eq!(routes.rail[0].points_2d[1].x.to_bits(), f32::MIN_POSITIVE.to_bits());
        assert_eq!(routes.stage_names, ["PINE RIDGE"]);
        assert_eq!(routes.trails.len(), 2);
        for (i, site) in routes.sites.iter().enumerate() {
            assert_eq!(site.kind as usize, i, "Kind::ALL out of step with the enum");
            assert_eq!(site.kind, Kind::ALL[i]);
            assert_eq!(site.name, format!("SITE {i}"));
            assert_eq!(site.seed, u32::MAX - i as u32);
        }
    }

    #[test]
    fn another_build_or_a_damaged_file_is_ignored() {
        let bytes = encoded();
        assert!(parse(&bytes, 43).is_none(), "from another build");
        assert!(parse(&bytes[..bytes.len() / 2], 42).is_none(), "cut short");
        assert!(parse(&[], 42).is_none(), "empty");
        for at in [0, 9, bytes.len() / 2, bytes.len() - 1] {
            let mut bad = bytes.clone();
            bad[at] ^= 0x10;
            assert!(parse(&bad, 42).is_none(), "byte {at} flipped");
        }
    }
}
