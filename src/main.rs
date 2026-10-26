use std::sync::Arc;
use std::time::Instant;

use glam::{Vec2, Vec3, vec3};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

mod ai_cars;
mod audio;
mod birds;
mod boosts;
mod camera;
mod car;
mod chunks;
mod circuit_data;
mod collectibles;
mod collisions;
mod farms;
mod gamepad;
mod cover;
mod geometry;
mod gpu;
mod hud;
mod landmarks;
mod layout;
mod noise;
mod lights;
mod particles;
mod physics;
mod progress;
mod props;
mod race;
mod railway;
mod rescue;
mod road;
mod roadside;
mod screen;
mod secrets;
mod skids;
mod spectators;
mod stages;
mod speedlines;
mod terrain;
mod track;
mod tuning;
mod weather;
mod trackside;
mod trails;
mod tracks;
mod trees;
mod wildlife;
mod world;
#[cfg(test)]
mod sim_tests;

use ai_cars::{AiCar, AiDriver, AiTrack, Personality};
use camera::ChaseCamera;
use chunks::ChunkManager;
use gpu::{DynamicMesh, Mesh};
use particles::Particles;
use physics::{CarInput, CarPhysics, WorldGround};
use progress::Progress;
use race::{Phase, Race, RaceEvent};
use world::World;

const MAX_SWAY: usize = 8;
/// Seconds for a full day/night cycle.
const DAY_LENGTH: f32 = 360.0;
/// Top speed the AI plans for (units/s).
const AI_TOP_SPEED: f32 = 165.0;
/// A tour: this many races at neighbouring circuits, driven between, with
/// these points for 1st to 8th in each.
pub(crate) const TOUR_ROUNDS: usize = 5;
const TOUR_POINTS: [u32; RACERS] = [10, 8, 6, 5, 4, 3, 2, 1];

/// A tour under way.
pub(crate) struct Tour {
    /// The circuits in order (indices into `world.circuits`).
    pub circuits: Vec<usize>,
    /// The round to race next.
    pub round: usize,
    /// Points so far: the player's, then each rival's.
    pub points: [u32; RACERS],
}

/// Everyone's finishing places for a tour round: 0 is the player, who came
/// `position` (1-based); `rivals` are rival indices best placed first, and
/// rival k is `k + 1`.
fn finishing_order(position: usize, rivals: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = rivals.iter().map(|&k| k + 1).collect();
    order.insert((position - 1).min(order.len()), 0);
    order
}

impl Tour {
    /// The player's place overall (1-based), ties going the player's way.
    pub fn standing(&self) -> usize {
        1 + self.points[1..].iter().filter(|&&p| p > self.points[0]).count()
    }

    pub fn next_circuit(&self) -> Option<usize> {
        self.circuits.get(self.round).copied()
    }
}

/// Difficulty settings: their names and how they scale the rivals' pace.
pub(crate) const DIFFICULTIES: [(&str, f32); 3] = [("EASY", 0.88), ("NORMAL", 1.0), ("HARD", 1.06)];
/// The rivals, fastest first: pace, side of the line on the straights (in
/// 8-unit steps, see `AiDriver::line_offset`), aggression, composure, how
/// keen on drifting, and reaction off the line (see `Personality`).
pub(crate) const RIVALS: [Personality; 7] = [
    // The class of the field: quick, tidy, rarely rattled
    Personality { pace: 0.96, line: 0.5, aggression: 0.45, composure: 0.85, drift: 0.2, reaction: 0.22 },
    // The hard charger: dives for gaps, shuts the door, a bit ragged
    Personality { pace: 0.945, line: -0.65, aggression: 0.9, composure: 0.45, drift: 0.5, reaction: 0.18 },
    // The showman: drifts every slow corner it can
    Personality { pace: 0.93, line: 0.3, aggression: 0.5, composure: 0.4, drift: 0.9, reaction: 0.3 },
    // Smooth and steady: never drifts, gives room, hardly ever errs
    Personality { pace: 0.915, line: -0.35, aggression: 0.3, composure: 0.9, drift: 0.0, reaction: 0.3 },
    // The wild one: wide on the straights, elbows out, mistakes under pressure
    Personality { pace: 0.895, line: 0.7, aggression: 0.8, composure: 0.3, drift: 0.6, reaction: 0.2 },
    // The veteran: cautious, slow off the line, hard to rattle
    Personality { pace: 0.88, line: -0.5, aggression: 0.2, composure: 0.75, drift: 0.1, reaction: 0.38 },
    // The rookie: nervous with a car behind, and slow to react
    Personality { pace: 0.86, line: 0.15, aggression: 0.4, composure: 0.2, drift: 0.3, reaction: 0.4 },
];
/// Cars in a race: the player and the rivals.
pub(crate) const RACERS: usize = RIVALS.len() + 1;
/// Converts units/s to the km/h shown on the speedometer (1 unit = 0.5 m).
const KMH_PER_UNIT: f32 = 1.8;
/// Other cars' engines can be heard within this distance.
const PASS_BY_RANGE: f32 = 150.0;
/// Throttle down within this long before the start signal earns a boost
/// off the line.
const PERFECT_START_WINDOW: f32 = 0.5;
const PERFECT_START_BOOST: f32 = 1.0;
/// Pull up within this distance of a start line to be offered a race.
const RACE_PROMPT_DISTANCE: f32 = 80.0;
/// A race is abandoned after this long this far from the circuit.
const ABANDON_DISTANCE: f32 = 350.0;
const ABANDON_TIME: f32 = 4.0;
/// Paint jobs available before you've found any landmarks.
const STARTING_PAINTS: usize = 2;
/// Every this many landmarks found unlocks another paint job.
const LANDMARKS_PER_PAINT: usize = 3;

/// How many paint jobs `landmarks_found` landmarks have unlocked.
fn unlocked_paints(landmarks_found: usize) -> usize {
    (STARTING_PAINTS + landmarks_found / LANDMARKS_PER_PAINT).min(car::CAR_COLORS.len())
}

/// R puts you back on a road or trail within this distance, else upright where you are.
const RESET_REACH: f32 = 400.0;
/// You discover a circuit by driving this close to it.
const CIRCUIT_DISCOVER_DISTANCE: f32 = 250.0;
/// AI cars further than this from the player glide along without physics.
const AI_PHYSICS_RANGE: f32 = 1500.0;

#[derive(Clone, Copy)]
struct SwayEvent {
    x: f32,
    z: f32,
    /// Ground height at the trunk, so the tree bends from its base.
    base_y: f32,
    /// Horizontal direction the tree is knocked toward.
    dir: Vec2,
    strength: f32, // 1.0 = just hit, decays to 0
    time: f32,     // accumulated time for oscillation
}

#[derive(Default)]
struct Controls {
    accelerate: bool,
    brake: bool,
    left: bool,
    right: bool,
    handbrake: bool,
}

struct Stage {
    gpu: gpu::Gpu,
    start_time: Instant,
    last_time: f32,

    world: World,
    chunks: ChunkManager,
    /// Circuits first (same indices as `world.circuits`), then road loops.
    ai_tracks: Vec<AiTrack>,
    /// Ambient traffic first, then the current race's rivals.
    cars: Vec<AiCar>,
    traffic_count: usize,
    /// Ambient cars taken off the circuit being raced on, until the race ends.
    benched: Vec<AiCar>,
    race: Option<Race>,
    abandon_timer: f32,
    player: CarPhysics,
    /// Set by STUMBLER_DEMO: the AI drives the player's car around the home circuit.
    demo_driver: Option<AiDriver>,
    camera: ChaseCamera,
    particles: Particles,
    skids: skids::Skids,
    weather: weather::Weather,
    rain_mesh: DynamicMesh,
    speed_lines: speedlines::SpeedLines,
    collectibles: Vec<collectibles::Collectible>,
    flocks: Vec<birds::Flock>,
    wildlife: wildlife::Wildlife,
    /// Fishes the player out of lakes.
    rescue: rescue::WaterRescue,
    wildlife_mesh: DynamicMesh,
    spectators: Vec<spectators::Spectator>,
    sway_events: Vec<SwayEvent>,
    /// The quickest lap driven on each circuit this session (by circuit
    /// index): its time and the recording raced against as a ghost.
    ghosts: std::collections::HashMap<usize, (f32, Vec<race::GhostFrame>)>,
    /// The tour being raced, if any.
    tour: Option<Tour>,
    /// The steam train going round the railway.
    train: Option<railway::Train>,
    progress: Progress,
    discover_timer: f32,

    body_mesh: Mesh,
    ai_body_meshes: Vec<Mesh>,
    wheel_mesh: Mesh,
    shadow_mesh: Mesh,
    train_meshes: screen::TrainMeshes,
    ghost_mesh: Mesh,
    /// Start lights, lit: red during the countdown, green at the off.
    start_light_meshes: [Mesh; 2],
    brake_light_mesh: Mesh,
    /// Whether the player is on the brakes (for the brake lights).
    braking: bool,
    collectible_mesh: DynamicMesh,
    bird_mesh: DynamicMesh,
    spectator_mesh: DynamicMesh,
    particle_mesh: DynamicMesh,
    skid_mesh: DynamicMesh,
    speed_line_mesh: DynamicMesh,
    /// Pools of floodlight on the circuits after dark.
    light_pool_mesh: DynamicMesh,
    /// When the skid mark mesh was last rebuilt (it fades, so now and then
    /// even without new marks).
    skid_mesh_time: f32,
    hud_mesh: DynamicMesh,

    controls: Controls,
    /// Seconds the throttle has been held down.
    throttle_held: f32,
    /// Seconds of fireworks left over the finish line.
    fireworks: f32,
    fireworks_accum: f32,
    /// Steering as the keys have eased it in (-1 left to 1 right).
    steer: f32,
    throttle: f32,
    boost: boosts::Boost,
    lap_pulse: f32,
    show_map: bool,
    /// Brief HUD message: text, colour, seconds left.
    banner: Option<(String, [f32; 4], f32)>,
    fullscreen: bool,
    car_color: usize,
    game_audio: Option<audio::GameAudio>,
    gamepad: Option<gamepad::Gamepad>,
    day_time: f32, // 0..1 where 0=midnight, 0.25=sunrise, 0.5=noon, 0.75=sunset
    wind_time: f32,
    /// When the last frame started, and frames per second smoothed for display.
    last_frame: Option<Instant>,
    fps: f32,
    /// CPU time spent updating and drawing, summed (reported when profiling).
    cpu_update_ms: f64,
    cpu_draw_ms: f64,
    /// Frames drawn (reported by screenshot runs).
    frames: u32,
}

/// Dev aid: a spot near the end of the trail out to the named landmark,
/// facing along it, close enough to discover the landmark.
fn trail_start(landmark: &str) -> Option<(Vec2, f32)> {
    let l = landmarks::all().iter().find(|l| l.name.eq_ignore_ascii_case(landmark))?;
    let trail = road::network()?.paths.iter()
        .find(|p| p.kind == road::PathKind::Trail && p.points.last().is_some_and(|q| q.distance(l.pos) < 60.0))?;
    let i = trail.points.len().saturating_sub(10);
    let dir = trail.tangent(i);
    Some((trail.points[i], (-dir.x).atan2(-dir.y)))
}

impl Stage {
    fn new(gpu: gpu::Gpu) -> Stage {
        let t0 = Instant::now();
        let world = World::build(&gpu);

        let mut stage_roads = road::network().into_iter().flat_map(|n| n.paths.iter()).filter(|p| p.kind == road::PathKind::Gravel);
        let mut ai_tracks: Vec<AiTrack> = world.circuits.iter()
            .map(|c| match c.stage {
                true => ai_cars::stage_track(stage_roads.next().expect("a road for each stage")),
                false => AiTrack::racing(c.points.clone(), AI_TOP_SPEED, ai_cars::RACING_ROOM),
            })
            .chain(world.road_loops.iter().map(|points| AiTrack::new(points.clone(), AI_TOP_SPEED)))
            .collect();
        // Cruising traffic on connecting roads is slower
        for track in ai_tracks.iter_mut().skip(world.circuits.len()) {
            for v in &mut track.speed {
                *v = v.min(110.0);
            }
        }
        let cars = ai_cars::create_traffic(&ai_tracks, None);
        let traffic_count = cars.len();

        // Start at the home circuit's line (STUMBLER_START=<circuit name> picks another,
        // and STUMBLER_START=<landmark name> starts near the end of its trail)
        let start_name = std::env::var("STUMBLER_START").ok();
        let start_circuit = start_name.as_ref()
            .and_then(|name| world.circuits.iter().position(|c| c.name.eq_ignore_ascii_case(name)))
            .unwrap_or(0);
        let track = &ai_tracks[start_circuit];
        // Dev aid: STUMBLER_AT=x,z,yaw starts anywhere (yaw in degrees, 0 = facing -Z)
        let at = std::env::var("STUMBLER_AT").ok().and_then(|s| {
            let v: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            (v.len() >= 2).then(|| (Vec2::new(v[0], v[1]), v.get(2).copied().unwrap_or(0.0).to_radians()))
        });
        // Dev aid: STUMBLER_START=TRAIN starts by the line as the train comes,
        // STUMBLER_START=CROSSING at a level crossing just before it does
        let mut train = railway::get().map(railway::Train::new);
        let by_train = match (start_name.as_deref(), railway::get(), &mut train) {
            (Some("TRAIN" | "train"), Some(rail), Some(_)) => {
                let p = rail.sample(rail.start + 260.0);
                let ahead = rail.sample(rail.start + 270.0) - p;
                let side = Vec2::new(-ahead.z, ahead.x).normalize_or_zero() * 70.0;
                Some((Vec2::new(p.x, p.z) + side, (side.x).atan2(side.y)))
            }
            (Some("CROSSING" | "crossing"), Some(rail), Some(t)) => rail.crossings.first().map(|c| {
                t.distance = c.at - railway::TRAIN_SPEED * 9.0;
                let (pivot, _, face) = c.barriers()[0];
                let p = Vec2::new(pivot.x, pivot.z) + Vec2::new(face.x, face.z) * 45.0 - Vec2::new(-c.road_dir.y, c.road_dir.x) * 8.0;
                (p, (face.x).atan2(face.z))
            }),
            _ => None,
        };
        let (pos, yaw) = at
            .or(by_train)
            .or_else(|| start_name.as_deref().and_then(trail_start))
            .unwrap_or_else(|| match world.circuits[start_circuit].stage {
                true => track.pose(world.circuits[start_circuit].start_idx - 12, -6.0),
                false => track.pose(track.len() - 12, -10.0),
            });
        let mut player = CarPhysics::new(Vec3::new(pos.x, 0.0, pos.y), yaw);
        let camera = ChaseCamera::new(&player);
        let chunks = ChunkManager::new(&gpu, pos);

        let demo_driver = std::env::var_os("STUMBLER_DEMO").map(|_| {
            let mut d = AiDriver::new(0, Personality::plain(1.0, 0.0), 1.0);
            d.track_progress(&ai_tracks[start_circuit], pos);
            d
        });

        let centroids: Vec<(f32, f32)> = world.circuits.iter().filter(|c| !c.stage).map(|c| (c.centre.x, c.centre.y)).collect();
        let loops: Vec<Vec<Vec2>> = world.circuits.iter().filter(|c| !c.stage).map(|c| c.points.clone()).collect();
        let progress = Progress::load();
        player.setup = physics::SETUPS[progress.setup.min(physics::SETUPS.len() - 1)];
        let mut collectibles = collectibles::create_collectibles();
        for (i, item) in collectibles.iter_mut().enumerate() {
            item.collected = progress.orbs.contains(&i);
        }

        let car_color = progress.paint.min(unlocked_paints(progress.landmarks.len()) - 1);
        let (body_verts, body_idxs) = car::generate_body_mesh(car_color);
        let (wheel_verts, wheel_idxs) = car::generate_wheel_mesh();
        let (shadow_verts, shadow_idxs) = screen::shadow_mesh();
        eprintln!("world ready in {:.1}s: {} circuits, {} roads, {} landmarks, {} AI cars",
            t0.elapsed().as_secs_f32(), world.circuits.len(), world.road_loops.len(), landmarks::all().len(), cars.len());

        let mut stage = Stage {
            start_time: Instant::now(),
            last_time: 0.0,
            ai_body_meshes: (0..car::CAR_COLORS.len())
                .map(|i| {
                    let (v, idx) = car::generate_body_mesh(i);
                    Mesh::new(&gpu, &v, &idx)
                })
                .collect(),
            body_mesh: Mesh::new(&gpu, &body_verts, &body_idxs),
            wheel_mesh: Mesh::new(&gpu, &wheel_verts, &wheel_idxs),
            shadow_mesh: Mesh::new(&gpu, &shadow_verts, &shadow_idxs),
            train_meshes: screen::TrainMeshes::new(&gpu),
            ghost_mesh: {
                // The car, pale and see-through
                let (mut v, idx) = car::generate_body_mesh(0);
                for vert in &mut v {
                    let shade = (vert.color[0] + vert.color[1] + vert.color[2]) / 3.0;
                    vert.color = [0.6 + shade * 0.4, 0.8 + shade * 0.2, 1.0, 0.35];
                }
                Mesh::new(&gpu, &v, &idx)
            },
            ghosts: Default::default(),
            tour: None,
            start_light_meshes: [[2.6, 0.2, 0.12, 1.0], [0.3, 2.6, 0.4, 1.0]].map(|color| {
                let mut b = props::Builder::default();
                b.blob(Vec3::ZERO, 1.2, 1.0, color, 2.0);
                Mesh::new(&gpu, &b.verts, &b.idxs)
            }),
            brake_light_mesh: {
                let (v, idx) = car::generate_brake_light_mesh();
                Mesh::new(&gpu, &v, &idx)
            },
            braking: false,
            train,
            collectible_mesh: DynamicMesh::new(&gpu, "collectibles"),
            bird_mesh: DynamicMesh::new(&gpu, "birds"),
            spectator_mesh: DynamicMesh::new(&gpu, "spectators"),
            particle_mesh: DynamicMesh::new(&gpu, "particles"),
            skid_mesh: DynamicMesh::new(&gpu, "skid marks"),
            speed_line_mesh: DynamicMesh::new(&gpu, "speed lines"),
            light_pool_mesh: DynamicMesh::new(&gpu, "light pools"),
            skid_mesh_time: -1.0,
            hud_mesh: DynamicMesh::new(&gpu, "hud"),
            collectibles,
            flocks: birds::create_flocks(&centroids),
            wildlife: wildlife::Wildlife::new(0x5eed_1234),
            rescue: rescue::WaterRescue::default(),
            wildlife_mesh: DynamicMesh::new(&gpu, "wildlife"),
            spectators: spectators::create_spectators(&loops),
            world,
            chunks,
            ai_tracks,
            cars,
            traffic_count,
            benched: Vec::new(),
            race: None,
            abandon_timer: 0.0,
            player,
            demo_driver,
            camera,
            particles: Particles::new(),
            skids: skids::Skids::default(),
            weather: weather::Weather::new(),
            rain_mesh: DynamicMesh::new(&gpu, "rain"),
            speed_lines: speedlines::SpeedLines::new(),
            sway_events: Vec::new(),
            progress,
            discover_timer: 0.0,
            controls: Controls::default(),
            throttle_held: 0.0,
            fireworks: 0.0,
            fireworks_accum: 0.0,
            steer: 0.0,
            throttle: 0.0,
            boost: boosts::Boost::default(),
            lap_pulse: 0.0,
            show_map: false,
            banner: None,
            fullscreen: false,
            car_color,
            game_audio: audio::GameAudio::new(),
            gamepad: gamepad::Gamepad::new(),
            // Dev aid: STUMBLER_DAY_TIME=0.9 starts at night (0..1, 0.5 = noon)
            day_time: std::env::var("STUMBLER_DAY_TIME").ok().and_then(|s| s.parse().ok()).unwrap_or(0.35),
            wind_time: 0.0,
            frames: 0,
            last_frame: None,
            fps: 60.0,
            cpu_update_ms: 0.0,
            cpu_draw_ms: 0.0,
            gpu,
        };
        // Dev aid: STUMBLER_RACE=1 lines up for a race straight away
        // (STUMBLER_RACE=trial for a time trial, =tour for a tour)
        match std::env::var("STUMBLER_RACE").as_deref() {
            Ok("trial") => stage.start_time_trial(start_circuit),
            Ok("tour") => stage.start_tour(start_circuit),
            Ok(_) => stage.start_race(start_circuit),
            Err(_) => {}
        }
        stage
    }

    /// Seconds since the game started.
    fn time(&self) -> f32 {
        self.start_time.elapsed().as_secs_f32()
    }

    fn audio(&self) -> Option<&audio::AudioParams> {
        self.game_audio.as_ref().map(|a| &*a.params)
    }

    fn player_xz(&self) -> Vec2 {
        Vec2::new(self.player.pos.x, self.player.pos.z)
    }

    fn circuit_name(&self, i: usize) -> &'static str {
        self.world.circuits[i].name
    }

    /// Begin a tour at circuit `ci`, taking in its nearest neighbours one
    /// after another, and line up for the first race.
    fn start_tour(&mut self, ci: usize) {
        let mut circuits = vec![ci];
        let venues = self.world.circuits.iter().filter(|c| !c.stage).count();
        while circuits.len() < TOUR_ROUNDS.min(venues) {
            let here = self.world.circuits[*circuits.last().unwrap()].start();
            let next = (0..self.world.circuits.len())
                .filter(|&i| !circuits.contains(&i) && !self.world.circuits[i].stage)
                .min_by(|&a, &b| self.world.circuits[a].start().distance(here).total_cmp(&self.world.circuits[b].start().distance(here)))
                .unwrap();
            circuits.push(next);
        }
        self.tour = Some(Tour { circuits, round: 0, points: [0; RACERS] });
        self.start_race(ci);
    }

    /// Score a finished tour round: points by finishing order (rivals still
    /// running placed by how far round they are), then on to the next.
    fn score_tour_round(&mut self, ci: usize, position: usize) {
        let Some(tour) = &mut self.tour else { return };
        if tour.next_circuit() != Some(ci) {
            return;
        }
        let Some(race) = &self.race else { return };
        let n = self.ai_tracks[ci].len();
        let mut rivals: Vec<usize> = (0..race.rivals.len()).collect();
        rivals.sort_by(|&a, &b| self.cars[race.rivals[b]].driver.progress(n).total_cmp(&self.cars[race.rivals[a]].driver.progress(n)));
        for (place, &who) in finishing_order(position, &rivals).iter().enumerate() {
            tour.points[who] += TOUR_POINTS.get(place).copied().unwrap_or(0);
        }
        tour.round += 1;
        let gained = TOUR_POINTS.get(position - 1).copied().unwrap_or(0);
        let standing = tour.standing();
        let text = match tour.next_circuit() {
            Some(next) => format!("TOUR ROUND {}/{}  +{gained} POINTS\n{} OVERALL - NEXT: {}", tour.round, tour.circuits.len(), hud::ordinal(standing), self.world.circuits[next].name),
            None if standing == 1 => format!("TOUR WON!\n{} POINTS", tour.points[0]),
            None => format!("TOUR FINISHED {}\n{} POINTS", hud::ordinal(standing), tour.points[0]),
        };
        let color = if standing == 1 { hud::YELLOW } else { hud::WHITE };
        self.banner = Some((text, color, 6.0));
        if tour.next_circuit().is_none() {
            self.tour = None;
        }
    }

    /// Line up on circuit `ci`'s grid against a fresh field of rivals.
    fn start_race(&mut self, ci: usize) {
        self.end_race();
        self.clear_circuit(ci);
        let track = &self.ai_tracks[ci];
        let first = self.cars.len();
        let scale = DIFFICULTIES[self.progress.difficulty.min(DIFFICULTIES.len() - 1)].1 * tuning::get().rival_pace;
        for (k, style) in RIVALS.iter().enumerate() {
            self.cars.push(AiCar::new(track, ci, 0, 1 + k * 2, Personality { pace: style.pace * scale, ..*style }));
        }
        let mut race = Race::new(ci, (first..self.cars.len()).collect());
        race.stage = self.world.circuits[ci].stage;
        race.start(&mut self.player, &mut self.cars, track);
        self.race = Some(race);
        self.abandon_timer = 0.0;
        self.camera.snap(&self.player);
        self.skids.lift(0);
        self.boost = boosts::Boost::default();
        self.banner = None;
        if let Some(d) = &mut self.demo_driver {
            *d = AiDriver::new(0, Personality::plain(1.0, 0.0), 1.0);
            d.track_progress(track, Vec2::new(self.player.pos.x, self.player.pos.z));
        }
    }

    /// A time trial on circuit `ci`: alone on the grid, lap after lap,
    /// chasing the ghost of your best lap.
    fn start_time_trial(&mut self, ci: usize) {
        self.end_race();
        self.clear_circuit(ci);
        // Bring back the ghost of your best lap here from an earlier session
        if !self.ghosts.contains_key(&ci) {
            if let Some(ghost) = self.progress.load_ghost(self.world.circuits[ci].name) {
                self.ghosts.insert(ci, ghost);
            }
        }
        let track = &self.ai_tracks[ci];
        let mut race = Race::time_trial(ci);
        race.start(&mut self.player, &mut self.cars, track);
        self.race = Some(race);
        self.abandon_timer = 0.0;
        self.camera.snap(&self.player);
        self.skids.lift(0);
        self.boost = boosts::Boost::default();
        self.banner = None;
        if let Some(d) = &mut self.demo_driver {
            *d = AiDriver::new(0, Personality::plain(1.0, 0.0), 1.0);
            d.track_progress(track, Vec2::new(self.player.pos.x, self.player.pos.z));
        }
    }

    /// Walking out of a tour race ends the tour.
    fn abandon_tour(&mut self) {
        if self.tour.take().is_some() {
            self.banner = Some(("RACE ABANDONED\nTOUR OVER".into(), hud::RED, 3.0));
        }
    }

    /// Back to free roam: the rivals leave.
    fn end_race(&mut self) {
        self.race = None;
        self.cars.truncate(self.traffic_count);
        // The circuit's own traffic comes back out
        self.traffic_count += self.benched.len();
        self.cars.append(&mut self.benched);
    }

    /// Take the ambient traffic off circuit `ci` while it's raced on.
    fn clear_circuit(&mut self, ci: usize) {
        let (keep, bench): (Vec<AiCar>, Vec<AiCar>) = self.cars.drain(..).partition(|c| c.track_idx != ci);
        self.cars = keep;
        self.benched = bench;
        self.traffic_count = self.cars.len();
    }

    fn update(&mut self) {
        let now = self.time();
        let dt = (now - self.last_time).clamp(0.001, 0.05);
        self.last_time = now;
        self.day_time = (self.day_time + dt / DAY_LENGTH).fract();
        self.weather.update(now, dt);
        let grip = weather::grip();
        for car in &mut self.cars {
            car.driver.grip = grip;
        }
        if let Some(d) = &mut self.demo_driver {
            d.grip = grip;
        }
        self.wind_time += dt;

        // --- Player input ---
        let actions = self.gamepad.as_mut().map(|g| g.poll()).unwrap_or_default();
        for action in actions {
            // Buttons do what their keys do
            let key = match action {
                gamepad::PadAction::Race => KeyCode::Enter,
                gamepad::PadAction::TimeTrial => KeyCode::KeyT,
                gamepad::PadAction::Tour => KeyCode::KeyG,
                gamepad::PadAction::Setup => KeyCode::KeyV,
                gamepad::PadAction::Difficulty => KeyCode::Tab,
                gamepad::PadAction::Abandon => KeyCode::Backspace,
                gamepad::PadAction::Map => KeyCode::KeyM,
                gamepad::PadAction::Reset => KeyCode::KeyR,
                gamepad::PadAction::Paint => KeyCode::KeyC,
            };
            self.key_down(key);
        }
        let traffic = ai_cars::traffic(std::iter::once((&self.player, self.braking))
            .chain(self.cars.iter().map(|c| (&c.physics, c.last_input.brake > 0.1))));
        let demo_track = self.race.as_ref().map_or(0, |r| r.circuit);
        let mut input = match &mut self.demo_driver {
            Some(driver) => driver.input(&self.ai_tracks[demo_track], &self.player, &traffic, dt),
            None => {
                let c = &self.controls;
                let pad = self.gamepad.as_ref().map(|g| g.state).unwrap_or_default();
                if pad.steer != 0.0 {
                    // The stick steers directly
                    self.steer = pad.steer;
                } else {
                    // Keys ease the steering in, and let it come back faster,
                    // so a tap is a small correction and a hold is full lock
                    let target = (c.right as i32 - c.left as i32) as f32;
                    let tune = tuning::get();
                    let rate = if target == 0.0 || target * self.steer < 0.0 { tune.steer_return } else { tune.steer_in };
                    self.steer += (target - self.steer).clamp(-rate * dt, rate * dt);
                }
                CarInput {
                    throttle: if c.accelerate { 1.0 } else { pad.throttle },
                    brake: if c.brake { 1.0 } else { pad.brake },
                    steer: self.steer,
                    handbrake: c.handbrake || pad.handbrake,
                }
            }
        };
        // How long the throttle has been held, for judging the launch
        self.throttle_held = if input.throttle > 0.5 { self.throttle_held + dt } else { 0.0 };
        if self.race.as_ref().is_some_and(|r| r.holding()) {
            input = CarInput { handbrake: true, ..Default::default() };
        }
        self.throttle = input.throttle;
        self.braking = input.brake > 0.1 && self.player.speed > 1.0;

        // --- Physics ---
        let ground = WorldGround;
        self.slipstreams();
        self.player.update(&ground, dt, input);
        if let (Some(train), Some(rail)) = (&mut self.train, railway::get()) {
            train.update(rail, dt);
        }
        if self.boost.update(&mut self.player, dt) {
            if let Some(a) = self.audio() { a.trigger_pickup(); }
        }
        let player_pos = self.player.pos;
        for car in &mut self.cars {
            let track = &self.ai_tracks[car.track_idx];
            if car.physics.pos.distance(player_pos) < AI_PHYSICS_RANGE || car.held {
                car.update(track, &ground, &traffic, dt);
            } else {
                car.update_far(track, dt);
            }
        }

        self.collide(dt);
        self.water_rescue(dt);
        self.pickups();
        self.update_race(dt);
        self.discover(dt);

        if let Some((_, _, t)) = &mut self.banner {
            *t -= dt;
            if *t <= 0.0 {
                self.banner = None;
            }
        }

        // --- Ambient world ---
        for s in &mut self.sway_events {
            s.time += dt;
            s.strength *= 1.0 - 1.5 * dt; // decay over ~0.7s
        }
        self.sway_events.retain(|s| s.strength > 0.02);
        birds::update_flocks(&mut self.flocks, self.player.pos, dt, now);
        self.wildlife.update(self.player.pos, self.player.vel, dt);

        if let Some(a) = self.audio() {
            a.set_speed(self.player.speed);
            let slide = (0..4)
                .filter(|&i| self.player.wheel_sliding[i])
                .map(|i| self.player.wheel_slip[i])
                .fold(0.0, f32::max);
            a.set_sliding(slide);
            a.set_throttle(self.throttle);
            a.set_rain(self.weather.rain, self.weather.wet);
            a.set_rumble(self.camera.rumble);
            let rear = self.player.wheel_contact_pos[2];
            a.set_paved(road::paved(rear.x, rear.z));
            // The nearest car close by, louder the closer, its note rising
            // as it closes and falling as it goes (sound at ~340 units/s)
            let near = self.cars.iter()
                .map(|c| (c, c.physics.pos.distance(self.player.pos)))
                .filter(|&(_, d)| d < PASS_BY_RANGE)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            match near {
                Some((car, d)) => {
                    let to_me = (self.player.pos - car.physics.pos).normalize_or_zero();
                    let closing = (car.physics.vel - self.player.vel).dot(to_me);
                    let pitch = (1.0 + closing / 340.0).clamp(0.7, 1.4);
                    a.set_other(car.physics.speed, (1.0 - d / PASS_BY_RANGE).powi(2), pitch);
                }
                None => a.set_other(0.0, 0.0, 1.0),
            }
        }

        // --- Camera ---
        let airborne = 1.0 - self.player.grounded_wheels() as f32 / 4.0;
        self.lap_pulse = (self.lap_pulse - dt).max(0.0);
        let zoom = 1.0 + airborne * 0.3 + self.lap_pulse * 0.25;
        // Field of view kicks out under boost and in a tow; kerbs and rough
        // ground shake the view, more the faster you go
        let kick = if self.boost.timer > 0.0 { tuning::get().fov_boost } else { 0.0 } + self.player.draft * 4.0;
        self.camera.fov_kick += (kick - self.camera.fov_kick) * (1.0 - (-4.0 * dt).exp());
        let fast = (self.player.speed.abs() / 120.0).min(1.0);
        let rough = (0..4).filter(|&i| self.player.wheel_contact[i])
            .map(|i| {
                let p = self.player.wheel_contact_pos[i];
                match road::network().and_then(|n| n.nearest_ground(p.x, p.z)) {
                    // Kerbs judder; grass and dirt rumble
                    Some(s) if s.kind == road::PathKind::Circuit && s.dist > s.half_width() - 1.0 && s.dist < s.half_width() + road::KERB_WIDTH + 0.5 => 1.0,
                    Some(s) if s.kind >= road::PathKind::Road && s.dist < s.half_width() + road::KERB_WIDTH => 0.0,
                    _ => 0.5,
                }
            })
            .fold(0.0, f32::max);
        self.camera.rumble += (rough * fast - self.camera.rumble) * (1.0 - (-10.0 * dt).exp());
        self.camera.update(&self.player, zoom, dt);
        let look = (self.camera.target - self.camera.pos).normalize_or_zero();
        self.speed_lines.update(self.camera.pos, look);
        let (v, i) = self.weather.rain_mesh(self.camera.pos, self.player.vel, dt);
        self.rain_mesh.update(&self.gpu, &v, &i);
        self.chunks.update(&self.gpu, self.camera.pos);

        // --- Particles ---
        let on_road = road::on_road(self.player.pos.x, self.player.pos.z);
        self.particles.emit_from_car(&self.player, on_road, self.throttle, dt);
        let paved = |p: Vec3| if road::paved(p.x, p.z) { 1.0 } else { 0.0 };
        let ground_color = |p: Vec3| {
            let c = terrain::terrain_color(p.x, p.z, p.y);
            [c[0], c[1], c[2]]
        };
        self.particles.rubble(&self.player, paved, ground_color, dt);
        let wet = self.weather.wet;
        let rear = self.player.wheel_contact_pos[2];
        self.particles.spray(&self.player, wet, road::paved(rear.x, rear.z), dt);
        self.skids.update(0, &self.player, paved, now);
        for (k, car) in self.cars.iter().enumerate() {
            let near = car.physics.pos.distance_squared(self.camera.pos);
            if near < 250.0 * 250.0 {
                let on_road = road::on_road(car.physics.pos.x, car.physics.pos.z);
                self.particles.emit_from_car(&car.physics, on_road, 0.0, dt);
                self.particles.rubble(&car.physics, paved, ground_color, dt);
                let rear = car.physics.wheel_contact_pos[2];
                self.particles.spray(&car.physics, wet, road::paved(rear.x, rear.z), dt);
            }
            if near < 500.0 * 500.0 {
                self.skids.update(k + 1, &car.physics, paved, now);
            } else {
                self.skids.lift(k + 1);
            }
        }
        self.skids.expire(now);
        if let (Some(train), Some(rail)) = (&mut self.train, railway::get()) {
            let poses = train.poses(rail);
            if poses[0].centre.distance(self.camera.pos) < 1500.0 {
                train.smoke(rail, &poses, &mut self.particles, dt);
            }
        }
        self.celebrate(dt);
        self.particles.emit_ambient(self.camera.pos, self.day_factor(), dt);
        self.secret_atmosphere(dt);
        self.particles.update(dt);
    }

    fn update_race(&mut self, dt: f32) {
        let Some(race) = &mut self.race else { return };
        let track = &self.ai_tracks[race.circuit];
        let name = self.world.circuits[race.circuit].name;
        let race_circuit = race.circuit;
        let events = race.update(dt, Vec2::new(self.player.pos.x, self.player.pos.z), &mut self.cars, track);
        race.record(self.player.pos, self.player.rot);
        race.check_direction(Vec2::new(self.player.vel.x, self.player.vel.z), track, dt);
        let final_lap = !race.trial && race.laps_done + 1 == race::RACE_LAPS;
        // Keep the quickest lap this session on each circuit as its ghost
        if let Some((time, frames)) = race.completed_lap.take() {
            if self.ghosts.get(&race.circuit).is_none_or(|g| time < g.0) {
                self.progress.save_ghost(name, time, &frames);
                self.ghosts.insert(race.circuit, (time, frames));
            }
        }

        // Wandering off abandons the race
        if race.phase == Phase::Racing && race.off_track > ABANDON_DISTANCE * race.roam() {
            self.abandon_timer += dt;
        } else {
            self.abandon_timer = 0.0;
        }
        let abandon = self.abandon_timer > ABANDON_TIME;

        for event in events {
            match event {
                RaceEvent::Go => {
                    // On the power just as the lights go: a flying start.
                    // Floor it too early and you've simply bogged down
                    if self.throttle_held > 0.0 && self.throttle_held <= PERFECT_START_WINDOW {
                        self.boost.timer = PERFECT_START_BOOST;
                        self.banner = Some(("PERFECT START!".into(), hud::GREEN, 2.5));
                        if let Some(a) = self.audio() { a.trigger_pickup(); }
                    }
                }
                RaceEvent::Lap { time, .. } => {
                    self.lap_pulse = 1.0;
                    if let Some(a) = self.game_audio.as_ref() { a.params.trigger_chime(); }
                    let record = self.progress.record_lap(name, time);
                    let label = if record { "LAP RECORD" } else { "LAP" };
                    let color = if record { hud::GREEN } else { hud::WHITE };
                    // Into the last lap of a race: say so under the time
                    let extra = if final_lap { "\nFINAL LAP" } else { "" };
                    self.banner = Some((format!("{label} {}{extra}", hud::format_time(time)), color, 2.5));
                    self.progress.save();
                }
                RaceEvent::LapInvalid => self.banner = Some(("LAP NOT COUNTED - MISSED A SECTOR".into(), hud::RED, 2.5)),
                RaceEvent::Finished => {
                    if let Some(a) = self.game_audio.as_ref() { a.params.trigger_chime(); }
                    if let Some(Race { phase: Phase::Finished { position, total_time }, stage, .. }) = &self.race {
                        self.progress.record_finish(name, *position);
                        // A stage's time is its record to beat
                        if *stage && self.progress.record_lap(name, *total_time) {
                            self.banner = Some((format!("STAGE RECORD {}", hud::format_time(*total_time)), hud::GREEN, 4.0));
                        }
                        self.progress.save();
                        // Fireworks: a proper show for a win
                        self.fireworks = if *position == 1 { 4.0 } else { 1.5 };
                        let (ci, position) = (race_circuit, *position);
                        self.score_tour_round(ci, position);
                    }
                }
            }
        }
        if abandon {
            let trial = self.race.as_ref().is_some_and(|r| r.trial);
            self.end_race();
            self.banner = Some((if trial { "TIME TRIAL ENDED" } else { "RACE ABANDONED" }.into(), hud::RED, 3.0));
            self.abandon_tour();
        }
    }

    /// Mark circuits and landmarks as discovered when the player gets close.
    fn discover(&mut self, dt: f32) {
        self.discover_timer -= dt;
        if self.discover_timer > 0.0 {
            return;
        }
        self.discover_timer = 0.25;
        let p = self.player_xz();
        let paints_before = unlocked_paints(self.progress.landmarks.len());
        let mut found: Option<String> = None;
        for c in &self.world.circuits {
            if !self.progress.circuits.contains(c.name)
                && c.outline.iter().any(|q| q.distance_squared(p) < CIRCUIT_DISCOVER_DISTANCE * CIRCUIT_DISCOVER_DISTANCE)
            {
                self.progress.circuits.insert(c.name.to_string());
                found = Some(format!("DISCOVERED {}", c.name));
            }
        }
        let mut secret = false;
        for l in landmarks::all() {
            if !self.progress.landmarks.contains(&l.name) && l.pos.distance(p) < landmarks::DISCOVER_RADIUS {
                self.progress.landmarks.insert(l.name.clone());
                if l.kind.is_secret() {
                    found = Some(format!("{}\n{}", l.name, landmarks::lore(l.kind)));
                    secret = true;
                } else {
                    found = Some(format!("FOUND {}", l.name));
                }
            }
        }
        if let Some(mut text) = found {
            if unlocked_paints(self.progress.landmarks.len()) > paints_before {
                text.push_str("\nNEW PAINT UNLOCKED - PRESS C");
            }
            if let Some(a) = self.audio() { a.trigger_chime(); }
            // Secrets get longer to read their lines, in gold
            self.banner = Some(if secret { (text, hud::GOLD, 7.0) } else { (text, hud::YELLOW, 3.0) });
            self.progress.save();
        }
    }

    /// Who's tucked in behind whom, for the player and the cars around them.
    fn slipstreams(&mut self) {
        let near: Vec<usize> = (0..self.cars.len())
            .filter(|&i| self.cars[i].physics.pos.distance_squared(self.player.pos) < 400.0 * 400.0)
            .collect();
        let all: Vec<&CarPhysics> = std::iter::once(&self.player).chain(near.iter().map(|&i| &self.cars[i].physics)).collect();
        let drafts = ai_cars::drafts(&all);
        self.player.draft = drafts[0];
        for car in &mut self.cars {
            car.physics.draft = 0.0;
        }
        for (k, &i) in near.iter().enumerate() {
            self.cars[i].physics.draft = drafts[k + 1];
        }
    }

    /// Fireworks bursting in the sky ahead as you take the flag.
    fn celebrate(&mut self, dt: f32) {
        if self.fireworks <= 0.0 {
            return;
        }
        self.fireworks -= dt;
        // Up ahead, where you're looking as you cross the line
        let ahead = self.player.pos + self.player.heading() * 140.0;
        self.fireworks_accum += dt * 2.5;
        while self.fireworks_accum >= 1.0 {
            self.fireworks_accum -= 1.0;
            let k = (self.fireworks * 7.0) as usize;
            let colours = [[2.2, 0.5, 0.3], [0.4, 1.8, 2.2], [2.2, 2.0, 0.5], [1.8, 0.5, 2.2], [0.5, 2.2, 0.8]];
            let spread = Vec3::new(((k * 37) % 11) as f32 - 5.0, 0.0, ((k * 53) % 11) as f32 - 5.0) * 4.0;
            self.particles.firework(ahead + Vec3::Y * (22.0 + (k % 3) as f32 * 8.0) + spread, colours[k % colours.len()]);
        }
    }

    /// Trees, scenery and car-to-car contacts.
    fn collide(&mut self, dt: f32) {
        let (px, pz) = (self.player.pos.x, self.player.pos.z);
        let trees: Vec<[f32; 2]> = self.chunks.trees_near(px, pz).copied().collect();
        if let Some((ti, hit)) = collisions::car_vs_trees(&mut self.player, &trees, dt) {
            let [tx, tz] = trees[ti];
            // The first moment in a tree: it whips away from the car, shedding leaves
            let already = self.sway_events.iter().any(|s| (s.x - tx).abs() < 1.0 && (s.z - tz).abs() < 1.0);
            if !already {
                if hit.impact_speed > 5.0 {
                    if let Some(a) = self.audio() { a.trigger_impact(); }
                    self.particles.burst(hit.point + Vec3::Y * 6.0, [0.35, 0.55, 0.2], 14, 9.0);
                }
                if self.sway_events.len() >= MAX_SWAY {
                    self.sway_events.remove(0);
                }
                let push = Vec2::new(self.player.vel.x, self.player.vel.z).try_normalize()
                    .or_else(|| Vec2::new(tx - px, tz - pz).try_normalize())
                    .unwrap_or(Vec2::X);
                self.sway_events.push(SwayEvent {
                    x: tx,
                    z: tz,
                    base_y: terrain::terrain_height(tx, tz),
                    dir: push,
                    strength: (hit.impact_speed / 40.0).clamp(0.3, 1.0),
                    time: 0.0,
                });
            }
        }

        let solids: Vec<collisions::Solid> = self.chunks.solids_near(px, pz).copied().collect();
        if let Some(hit) = collisions::car_vs_solids(&mut self.player, &solids) {
            if hit.impact_speed > 5.0 {
                if let Some(a) = self.audio() { a.trigger_impact(); }
                self.particles.burst(hit.point, [0.6, 0.58, 0.55], 8, 6.0);
                if hit.impact_speed > 12.0 {
                    self.particles.sparks(hit.point + Vec3::Y * 0.5, (hit.impact_speed * 0.6) as usize, 14.0);
                }
            }
        }

        // The train shoves aside anything on the line
        if let (Some(train), Some(rail)) = (&self.train, railway::get()) {
            let poses = train.poses(rail);
            if poses[0].centre.distance(self.player.pos) < 300.0 {
                let solids = railway::Train::solids(&poses);
                if let Some(hit) = collisions::car_vs_moving(&mut self.player, &solids) {
                    if hit.impact_speed > 4.0 {
                        if let Some(a) = self.audio() { a.trigger_impact(); }
                        self.particles.burst(hit.point, [0.6, 0.58, 0.55], 10, 7.0);
                    }
                }
                for car in &mut self.cars {
                    collisions::car_vs_moving(&mut car.physics, &solids);
                }
            }
        }

        let player_pos = self.player.pos;
        let near = move |c: &CarPhysics| c.pos.distance_squared(player_pos) < AI_PHYSICS_RANGE * AI_PHYSICS_RANGE;
        // Cars nearby hit tyre walls, hoardings and buildings too
        for car in self.cars.iter_mut().filter(|c| near(&c.physics)) {
            let solids: Vec<collisions::Solid> = self.chunks.solids_near(car.physics.pos.x, car.physics.pos.z).copied().collect();
            if let Some(hit) = collisions::car_vs_solids(&mut car.physics, &solids) {
                if hit.impact_speed > 12.0 && hit.point.distance(self.camera.pos) < 400.0 {
                    self.particles.sparks(hit.point + Vec3::Y * 0.5, (hit.impact_speed * 0.5) as usize, 12.0);
                }
            }
        }
        let mut player_hit = false;
        let paint = |idx: usize| car::CAR_COLORS[idx % car::CAR_COLORS.len()];
        // Knocks between cars throw sparks, and bits of each car's paint
        let mut knocks: Vec<(Vec3, f32, [f32; 3], [f32; 3])> = Vec::new();
        for car in &mut self.cars {
            if let Some(hit) = collisions::car_vs_car(&mut self.player, &mut car.physics) {
                player_hit |= hit.impact_speed > 4.0;
                knocks.push((hit.point, hit.impact_speed, paint(self.car_color), paint(car.color_idx)));
            }
        }
        for i in 0..self.cars.len() {
            if !near(&self.cars[i].physics) {
                continue;
            }
            let (head, tail) = self.cars.split_at_mut(i + 1);
            for other in tail {
                if let Some(hit) = collisions::car_vs_car(&mut head[i].physics, &mut other.physics) {
                    knocks.push((hit.point, hit.impact_speed, paint(head[i].color_idx), paint(other.color_idx)));
                }
            }
        }
        for (at, speed, a, b) in knocks {
            if speed > 6.0 && at.distance(self.camera.pos) < 400.0 {
                let bits = (speed * 0.4).min(12.0) as usize;
                self.particles.sparks(at + Vec3::Y * 0.8, bits * 2, 12.0);
                self.particles.shards(at + Vec3::Y * 1.2, a, bits / 2 + 1, 8.0);
                self.particles.shards(at + Vec3::Y * 1.2, b, bits / 2 + 1, 8.0);
            }
        }
        if player_hit {
            if let Some(a) = self.audio() { a.trigger_impact(); }
        }
    }

    /// Collectible orbs and boost pads.
    fn pickups(&mut self) {
        let pos = self.player.pos;
        let mut picked = Vec::new();
        for (i, item) in self.collectibles.iter_mut().enumerate() {
            if !item.collected && Vec2::new(item.x - pos.x, item.z - pos.z).length_squared() < 8.0 * 8.0 {
                item.collected = true;
                picked.push((i, vec3(item.x, pos.y + 2.0, item.z)));
            }
        }
        for (i, at) in picked {
            // Each orb permanently adds a little engine power
            self.player.speed_bonus += 2.0;
            self.progress.orbs.insert(i);
            self.progress.save();
            if let Some(a) = self.audio() { a.trigger_pickup(); }
            self.particles.burst(at, [1.0, 0.85, 0.2], 16, 12.0);
        }

        // Rivals take the pads too
        for car in &mut self.cars {
            let p = car.physics.pos;
            if self.world.boost_pads.iter().any(|pad| pad.contains(p.x, p.z)) {
                car.boost.pad();
            }
        }
        let on_pad = self.world.boost_pads.iter().any(|pad| pad.contains(pos.x, pos.z));
        if on_pad && self.boost.pad() {
            if let Some(a) = self.audio() { a.trigger_pickup(); }
            self.particles.burst(pos, [0.2, 0.85, 1.0], 20, 10.0);
        }
    }

    fn sun_dir(&self) -> Vec3 {
        // Sun circles the sky: day_time 0=midnight, 0.25=sunrise, 0.5=noon, 0.75=sunset
        let a = (self.day_time - 0.25) * std::f32::consts::TAU;
        vec3(a.cos(), a.sin(), -0.3).normalize()
    }

    /// Fireflies at the magical places after dark; steam off the hot springs.
    fn secret_atmosphere(&mut self, dt: f32) {
        let night = 1.0 - self.day_factor();
        let me = self.player_xz();
        for l in landmarks::all().iter().filter(|l| l.kind.is_secret() && l.pos.distance(me) < 250.0) {
            let centre = Vec3::new(l.pos.x, terrain::terrain_height(l.pos.x, l.pos.y), l.pos.y);
            if l.kind == landmarks::Kind::HotSpring {
                self.particles.steam(centre, 9.0, 14.0, dt);
            } else if l.kind.magical() && night > 0.3 {
                self.particles.fireflies(centre, 22.0, 12.0 * night, dt);
            }
        }
    }

    /// Splashes and bubbles; a car that sinks is set back down on the shore.
    fn water_rescue(&mut self, dt: f32) {
        let step = self.rescue.update(&mut self.player, dt);
        let me = self.player_xz();
        if let Some(speed) = step.splash {
            self.particles.splash(Vec3::new(me.x, terrain::WATER_LEVEL + 0.3, me.y), speed);
            if let Some(a) = self.audio() { a.trigger_impact(); }
        }
        if step.sinking {
            self.particles.bubbles(self.player.pos, terrain::WATER_LEVEL, dt);
        }
        if step.rescued {
            self.camera.snap(&self.player);
        }

        // AI cars that end up in the water go back to their road
        for car in &mut self.cars {
            if car.physics.submerged > 0.9 {
                let track = &self.ai_tracks[car.track_idx];
                let (pos, yaw) = track.pose(car.driver.progress_idx, 0.0);
                car.physics.place(&WorldGround, pos.x, pos.y, yaw);
                car.physics.pos.y += rescue::SHORE_DROP;
            }
        }
    }

    fn day_factor(&self) -> f32 {
        (self.sun_dir().y * 3.0 + 0.3).clamp(0.0, 1.0)
    }

    /// Circuit whose start line the player is parked near, if no race is on.
    fn race_offer(&self) -> Option<usize> {
        if self.race.is_some() {
            return None;
        }
        self.world.nearest_start(self.player_xz()).filter(|&(_, d)| d < RACE_PROMPT_DISTANCE).map(|(i, _)| i)
    }

    /// Returns true if the game should quit.
    fn key_down(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::ArrowUp | KeyCode::KeyW => self.controls.accelerate = true,
            KeyCode::ArrowDown | KeyCode::KeyS => self.controls.brake = true,
            KeyCode::ArrowLeft | KeyCode::KeyA => self.controls.left = true,
            KeyCode::ArrowRight | KeyCode::KeyD => self.controls.right = true,
            KeyCode::Space => self.controls.handbrake = true,
            KeyCode::Enter => {
                if let Some(Race { phase: Phase::Finished { .. }, circuit, .. }) = self.race {
                    self.start_race(circuit);
                } else if let Some(ci) = self.race_offer() {
                    self.start_race(ci);
                }
            }
            KeyCode::KeyG => {
                // A tour from this start line: here, then its neighbours
                if let Some(ci) = self.race_offer().filter(|&ci| !self.world.circuits[ci].stage) {
                    self.start_tour(ci);
                }
            }
            KeyCode::KeyV => {
                // The car's setup, changed at a start line
                if self.race_offer().is_some() {
                    self.progress.setup = (self.progress.setup + 1) % physics::SETUPS.len();
                    self.player.setup = physics::SETUPS[self.progress.setup];
                    self.progress.save();
                }
            }
            KeyCode::Tab => {
                // Rivals' difficulty, chosen at a start line
                if self.race_offer().is_some() {
                    self.progress.difficulty = (self.progress.difficulty + 1) % DIFFICULTIES.len();
                    self.progress.save();
                }
            }
            KeyCode::KeyT => {
                if let Some(ci) = self.race_offer().filter(|&ci| !self.world.circuits[ci].stage) {
                    self.start_time_trial(ci);
                }
            }
            KeyCode::Backspace => {
                if let Some(race) = &self.race {
                    let finished = matches!(race.phase, Phase::Finished { .. });
                    let trial = race.trial;
                    self.end_race();
                    if trial {
                        self.banner = Some(("TIME TRIAL ENDED".into(), hud::WHITE, 2.0));
                    } else if !finished {
                        self.banner = Some(("RACE ABANDONED".into(), hud::RED, 3.0));
                        self.abandon_tour();
                    }
                }
            }
            KeyCode::KeyM => self.show_map = !self.show_map,
            KeyCode::KeyF => {
                self.fullscreen = !self.fullscreen;
                self.gpu.window.set_fullscreen(self.fullscreen.then_some(Fullscreen::Borderless(None)));
            }
            KeyCode::KeyR if self.race.as_ref().is_some_and(|r| r.phase == Phase::Racing) => {
                // Mid-race: back on the circuit where you'd got to, facing the
                // way the race goes, beside (not on top of) anyone there
                let race = self.race.as_ref().unwrap();
                let track = &self.ai_tracks[race.circuit];
                let (centre, _) = track.pose(race.progress_idx(), 0.0);
                let side = {
                    let n = track.len();
                    let t = (track.centre[(race.progress_idx() + 1) % n] - centre).normalize_or_zero();
                    Vec2::new(-t.y, t.x)
                };
                let crowd = self.cars.iter().map(|c| Vec2::new(c.physics.pos.x, c.physics.pos.z))
                    .filter(|p| p.distance(centre) < 14.0)
                    .map(|p| (p - centre).dot(side))
                    .next();
                let lateral = crowd.map_or(0.0, |s| if s > 0.0 { -10.0 } else { 10.0 });
                let (pos, yaw) = track.pose(race.progress_idx(), lateral);
                self.player.place(&WorldGround, pos.x, pos.y, yaw);
                self.camera.snap(&self.player);
                self.skids.lift(0);
            }
            KeyCode::KeyR => {
                // Back onto the nearest road or trail, facing along it the way
                // we were heading; out in the wilds, just upright where we are
                let here = self.player_xz();
                let heading = Vec2::new(self.player.heading().x, self.player.heading().z);
                let nearest = road::network().and_then(|n| {
                    n.paths.iter()
                        .flat_map(|p| (0..p.points.len()).map(move |i| (p, i)))
                        .min_by(|a, b| a.0.points[a.1].distance_squared(here).total_cmp(&b.0.points[b.1].distance_squared(here)))
                });
                let (pos, dir) = match nearest {
                    Some((path, i)) if path.points[i].distance(here) < RESET_REACH => {
                        let t = path.tangent(i);
                        (path.points[i], if t.dot(heading) < 0.0 { -t } else { t })
                    }
                    _ => (here, heading),
                };
                self.player.place(&WorldGround, pos.x, pos.y, (-dir.x).atan2(-dir.y));
                self.camera.snap(&self.player);
                self.skids.lift(0);
            }
            KeyCode::KeyC => {
                self.car_color = (self.car_color + 1) % unlocked_paints(self.progress.landmarks.len());
                self.progress.paint = self.car_color;
                self.progress.save();
                let (verts, _) = car::generate_body_mesh(self.car_color);
                self.body_mesh.write_vertices(&self.gpu, &verts);
            }
            KeyCode::Escape => return true,
            _ => {}
        }
        false
    }

    fn key_up(&mut self, key: KeyCode) {
        match key {
            KeyCode::ArrowUp | KeyCode::KeyW => self.controls.accelerate = false,
            KeyCode::ArrowDown | KeyCode::KeyS => self.controls.brake = false,
            KeyCode::ArrowLeft | KeyCode::KeyA => self.controls.left = false,
            KeyCode::ArrowRight | KeyCode::KeyD => self.controls.right = false,
            KeyCode::Space => self.controls.handbrake = false,
            _ => {}
        }
    }

    fn mouse_wheel(&mut self, y: f32) {
        self.camera.distance = (self.camera.distance - y * 4.0).clamp(14.0, 70.0);
    }
}

#[derive(Default)]
struct App {
    stage: Option<Stage>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.stage.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Stumbler")
            .with_inner_size(LogicalSize::new(1280, 720));
        let window = Arc::new(event_loop.create_window(attributes).expect("create window"));
        let gpu = pollster::block_on(gpu::Gpu::new(window, event_loop.owned_display_handle()));
        self.stage = Some(Stage::new(gpu));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(stage) = self.stage.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => stage.gpu.resize(size.width, size.height),
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key: PhysicalKey::Code(key), state, repeat, .. },
                ..
            } => {
                if !state.is_pressed() {
                    stage.key_up(key);
                } else if !repeat && stage.key_down(key) {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                stage.mouse_wheel(y);
            }
            WindowEvent::RedrawRequested => {
                let frame_start = Instant::now();
                if let Some(last) = stage.last_frame.replace(frame_start) {
                    // Smoothed over about half a second, for the on-screen counter
                    let dt = (frame_start - last).as_secs_f32().max(1e-4);
                    stage.fps += (1.0 / dt - stage.fps) * (dt * 2.0).min(1.0);
                }
                stage.update();
                stage.cpu_update_ms += frame_start.elapsed().as_secs_f64() * 1000.0;
                // Dev aid: STUMBLER_SCREENSHOT=path saves one frame and exits,
                // after STUMBLER_SCREENSHOT_AT seconds (default 2)
                if let Ok(path) = std::env::var("STUMBLER_SCREENSHOT") {
                    let at = std::env::var("STUMBLER_SCREENSHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0);
                    stage.frames += 1;
                    if stage.time() > at {
                        eprintln!("frames: {} in {:.1}s ({:.1} ms/frame), {}k triangles, {} draw calls", stage.frames, stage.time(), stage.time() * 1000.0 / stage.frames as f32, stage.gpu.last_triangles / 1000, stage.gpu.last_draw_calls);
                        if let Some(timer) = &stage.gpu.timer {
                            let n = stage.frames as f64;
                            eprintln!("profile: cpu update {:.2} ms, cpu draw {:.2} ms, gpu pass {:.2} ms (averages)", stage.cpu_update_ms / n, stage.cpu_draw_ms / n, timer.average_ms());
                        }
                        stage.show_map |= std::env::var_os("STUMBLER_SCREENSHOT_MAP").is_some();
                        stage.gpu.capture_path = Some(path.into());
                        stage.draw();
                        event_loop.exit();
                        return;
                    }
                }
                let draw_start = Instant::now();
                stage.draw();
                stage.cpu_draw_ms += draw_start.elapsed().as_secs_f64() * 1000.0;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(stage) = &self.stage {
            stage.gpu.window.request_redraw();
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.run_app(&mut App::default()).expect("run event loop");
}

#[cfg(test)]
mod tour_tests {
    use super::*;

    #[test]
    fn tour_points_go_by_finishing_order() {
        // Player 2nd; rival 2 won, then rival 0, then rival 1
        assert_eq!(finishing_order(2, &[2, 0, 1]), vec![3, 0, 1, 2]);
        assert_eq!(finishing_order(1, &[0, 1, 2]), vec![0, 1, 2, 3]);
        assert_eq!(finishing_order(4, &[1, 0, 2]), vec![2, 1, 3, 0]);
        assert_eq!(finishing_order(8, &[6, 5, 4, 3, 2, 1, 0]), vec![7, 6, 5, 4, 3, 2, 1, 0]);
        let mut tour = Tour { circuits: vec![0, 1, 2, 3, 4], round: 0, points: [16, 16, 10, 4, 3, 2, 1, 0] };
        assert_eq!(tour.standing(), 1, "ties go the player's way");
        tour.points[1] = 20;
        assert_eq!(tour.standing(), 2);
        tour.round = 5;
        assert_eq!(tour.next_circuit(), None);
    }
}
