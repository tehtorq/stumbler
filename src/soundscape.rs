//! The sounds of the world going about its day, placed where they come
//! from: fishing boats and tractors chugging, the chatter of a busy street
//! and the pub, cows and sheep in the fields, dogs barking, halyards
//! tinkling against masts at the moorings, the town-hall clock striking the
//! hour, and a balloon's burner roaring overhead. (The steam train's
//! chuff and whistle are `train_sounds`.)

use glam::{Vec2, Vec3, vec3};

use crate::audio::{self, Sound, DIESELS};
use crate::Stage;

/// Sounds are looked for this near (each also fades with distance).
const HERD_REACH: f32 = 350.0;
const DOG_REACH: f32 = 250.0;
const MOORING_REACH: f32 = 180.0;
const BELL_REACH: f32 = 2500.0;
const CHATTER_REACH: f32 = 70.0;

/// When each of the occasional sounds is next due, and the hour last struck.
#[derive(Default)]
pub struct Soundscape {
    rng: u32,
    next_call: f32,
    next_tink: f32,
    next_bark: f32,
    next_crackle: f32,
    struck: Option<u32>,
}

impl Soundscape {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223) | 1;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

impl Stage {
    pub(crate) fn world_sounds(&mut self, dt: f32) {
        let cam = self.camera.pos;
        let cam2 = Vec2::new(cam.x, cam.z);
        let hour = self.day_time * 24.0;
        let daytime = (6.0..21.0).contains(&hour);
        let flares = self.aloft.take_flares();

        // ── Diesels: the two loudest of the fishing boats and farm machines ──
        let mut engines: Vec<(Vec3, f32)> = self.boats.engines().collect();
        engines.extend(crate::machines::engines(self.player.pos, self.day_time, self.wind_time));
        let mut heard: Vec<(audio::Place, f32, f32)> = engines.iter().map(|&(at, rate)| {
            let (place, carry, _) = self.hearing(at);
            (place, carry, rate)
        }).collect();
        heard.sort_by(|a, b| b.1.total_cmp(&a.1));

        // ── Chatter: the townsfolk round about, or the campers round their fire ──
        let fires = self.camps.fires_near(cam, self.day_time, 120.0);
        let chatter = self.townsfolk.crowd_near(cam, CHATTER_REACH).map(|(n, at)| {
            let (place, carry, _) = self.hearing(at);
            (place, (n / 14.0).min(1.0) * (carry * 3.0).min(1.0))
        }).or_else(|| fires.first().map(|&at| {
            let (place, carry, _) = self.hearing(at);
            (place, 0.3 * (carry * 3.0).min(1.0))
        }));

        // ── Occasional calls ──
        // What, how loud from close by, where, and a moment's wait before it
        let mut plays: Vec<(Sound, f32, Vec3, f32)> = Vec::new();
        let s = &mut self.soundscape;
        s.next_call -= dt;
        if s.next_call <= 0.0 {
            s.next_call = 2.5 + 6.0 * s.rand();
            if daytime {
                let herds: Vec<(Vec2, bool)> = crate::farms::herds().into_iter().filter(|(p, _)| p.distance(cam2) < HERD_REACH).collect();
                if !herds.is_empty() {
                    let (p, sheep) = herds[(s.rand() * herds.len() as f32) as usize % herds.len()];
                    let jitter = Vec2::new(s.rand() - 0.5, s.rand() - 0.5) * 30.0;
                    let q = p + jitter;
                    plays.push((if sheep { Sound::Baa } else { Sound::Moo }, 1.0, vec3(q.x, crate::terrain::terrain_height(q.x, q.y) + 2.0, q.y), 0.0));
                }
            }
        }
        s.next_bark -= dt;
        if s.next_bark <= 0.0 {
            s.next_bark = 4.0 + 9.0 * s.rand();
            let mut dogs: Vec<Vec2> = self.travellers.dogs();
            dogs.extend(self.picnics.parties());
            dogs.retain(|d| d.distance(cam2) < DOG_REACH);
            if !dogs.is_empty() {
                let d = dogs[(s.rand() * dogs.len() as f32) as usize % dogs.len()];
                // A bark or two together
                let n = 1 + (s.rand() * 2.5) as usize;
                for k in 0..n {
                    plays.push((Sound::Bark, 0.9, vec3(d.x, crate::terrain::terrain_height(d.x, d.y) + 1.0, d.y), k as f32 * 0.35));
                }
            }
        }
        s.next_crackle -= dt;
        if s.next_crackle <= 0.0 {
            s.next_crackle = 0.08 + 0.4 * s.rand();
            if let Some(&f) = fires.first() {
                plays.push((Sound::Crackle, 0.4 + 0.6 * s.rand(), f + Vec3::Y * 0.5, 0.0));
            }
        }
        s.next_tink -= dt;
        if s.next_tink <= 0.0 {
            s.next_tink = 0.4 + 1.8 * s.rand();
            let moorings: Vec<Vec2> = crate::harbour::get().moorings.iter().map(|m| m.pos).filter(|p| p.distance(cam2) < MOORING_REACH).collect();
            if !moorings.is_empty() {
                let m = moorings[(s.rand() * moorings.len() as f32) as usize % moorings.len()];
                plays.push((Sound::Tink, 0.6 + 0.4 * s.rand(), vec3(m.x, crate::terrain::WATER_LEVEL + 6.0, m.y), 0.0));
            }
        }
        // The town-hall clocks strike the hour through the day
        let now = hour.floor() as u32;
        let mut tolls: Vec<(Vec3, u32)> = Vec::new();
        if s.struck != Some(now) {
            let first = s.struck.is_none();
            s.struck = Some(now);
            if !first && (7..=22).contains(&now) {
                let strokes = if now % 12 == 0 { 12 } else { now % 12 };
                for tower in crate::towns::clock_towers().into_iter().filter(|t| Vec2::new(t.x, t.z).distance(cam2) < BELL_REACH) {
                    tolls.push((tower, strokes));
                }
            }
        }

        let Some(a) = self.audio() else { return };
        for k in 0..DIESELS {
            match heard.get(k) {
                Some(&(place, carry, rate)) => a.set_diesel(k, carry * 5.0, rate, place),
                None => a.set_diesel(k, 0.0, 0.0, audio::Place::CENTRE),
            }
        }
        match chatter {
            Some((place, level)) => a.set_chatter(level, place),
            None => a.set_chatter(0.0, audio::Place::CENTRE),
        }
        for (sound, gain, at, wait) in plays {
            let (place, carry, delay) = self.hearing(at);
            a.play(sound, (gain * carry * 6.0).min(1.0), place, delay + wait);
        }
        for at in flares {
            let (place, carry, delay) = self.hearing(at);
            a.play(Sound::Burner, (carry * 8.0).min(1.0), place, delay);
        }
        for (tower, strokes) in tolls {
            let (place, carry, delay) = self.hearing(tower);
            for k in 0..strokes {
                a.play(Sound::Toll, (carry * 40.0).min(1.0), place, delay + k as f32 * 2.2);
            }
        }
    }
}
