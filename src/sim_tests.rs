//! Headless end-to-end checks: real terrain, roads, physics, AI and race
//! logic, no GPU. Run with `cargo test --release -- --nocapture` for stats.

use glam::{Vec2, Vec3};

use crate::ai_cars::{AiCar, AiDriver, AiTrack, Personality};
use crate::physics::{CarInput, CarPhysics, WorldGround};
use crate::race::{Phase, Race, RACE_LAPS};
use crate::road::{self, RoadNetwork};
use crate::{collisions, terrain};

pub(crate) fn network() -> &'static RoadNetwork {
    road::install(|| crate::world::build_network(crate::layout::build(terrain::TERRAIN_HALF_SIZE)))
}

fn race_track() -> AiTrack {
    let points = network().paths.iter().find(|p| p.circuit()).expect("a circuit on the map").points.clone();
    AiTrack::racing(points, crate::AI_TOP_SPEED, crate::ai_cars::RACING_ROOM)
}

#[test]
fn roads_are_flat_across_and_match_terrain() {
    let net = network();
    assert!(net.paths.iter().any(|p| p.circuit()), "no circuit inside the terrain bounds");
    for path in &net.paths {
        let hw = path.kind.half_width();
        for (i, p) in path.points.iter().enumerate().step_by(25) {
            let t = path.tangent(i);
            let side = Vec2::new(-t.y, t.x);
            let centre = terrain::terrain_height(p.x, p.y);
            // Running onto a wider path (or crossing another trail, or itself
            // at a crossover like Suzuka's): that path owns this spot. So does
            // the one a trail leaves, under its first few units (on a steep
            // trail, its own slope runs across the join)
            let crosses_itself = path.points.iter().enumerate()
                .any(|(j, q)| i.abs_diff(j) > 40 && q.distance(*p) < hw * 2.0 + 10.0);
            let joining = path.kind == road::PathKind::Trail && i as f32 * road::SAMPLE_SPACING < hw * 3.0;
            if net.nearest_matching(p.x, p.y, |k| k > path.kind).is_some_and(|s| s.dist < s.half_width() + hw + s.shoulder())
                || crosses_itself
                || joining
                || (centre - path.heights[i]).abs() > 0.01
            {
                continue;
            }
            for off in [-hw, hw] {
                let q = *p + side * off;
                let edge = terrain::terrain_height(q.x, q.y);
                // Overlapping roads can disagree slightly at junctions
                assert!((edge - centre).abs() < 1.5, "road not flat at {p}: centre {centre} edge {edge}");
            }
        }
    }
}

#[test]
fn every_landmark_has_a_trail() {
    let net = network();
    for l in crate::landmarks::all().iter().filter(|l| !l.kind.is_secret()) {
        let reached = net.paths.iter()
            .filter(|p| p.kind == road::PathKind::Trail)
            .any(|t| t.points.last().unwrap().distance(l.pos) < 60.0);
        assert!(reached, "no trail out to {}", l.name);
    }
}

#[test]
fn secrets_are_hidden_away_with_cairns_leading_to_them() {
    let net = network();
    let secrets: Vec<_> = crate::landmarks::all().iter().filter(|l| l.kind.is_secret()).collect();
    for s in &secrets {
        println!("{:?} {} at {}", s.kind, s.name, s.pos);
    }
    assert!(secrets.len() >= 7, "only {} secrets found sites", secrets.len());
    for s in &secrets {
        let trail = net.paths.iter().any(|p| p.points.last().is_some_and(|q| q.distance(s.pos) < 60.0));
        assert!(!trail, "{} has a trail to it", s.name);
        let cairn = crate::landmarks::cairns().iter().any(|c| c.distance(s.pos) < 200.0);
        let by_road = net.nearest(s.pos.x, s.pos.y).is_some();
        assert!(cairn || by_road, "nothing leads to {}", s.name);
    }
}

#[test]
fn sunk_cars_come_back_on_dry_ground_near_the_water() {
    let _ = network();
    // Find some open water, and dry land a little way back from it
    let mut checked = 0;
    let mut z = -8000.0;
    while z < 8000.0 && checked < 20 {
        let mut x = -8000.0;
        while x < 8000.0 && checked < 20 {
            let sunk = Vec2::new(x, z);
            if terrain::raw_height(x, z) < terrain::WATER_LEVEL - 4.0 {
                let (spot, facing) = crate::rescue::shore_spot(None, sunk);
                assert!(terrain::raw_height(spot.x, spot.y) > terrain::WATER_LEVEL + 2.5, "set down in the water at {spot}");
                assert!(spot.distance(sunk) < 800.0, "carried far from {sunk} to {spot}");
                let ahead = spot + facing * 10.0;
                assert!(terrain::raw_height(ahead.x, ahead.y) > terrain::WATER_LEVEL, "facing back into the water at {spot}");
                checked += 1;
            }
            x += 731.0;
        }
        z += 613.0;
    }
    assert!(checked > 5, "hardly any water found to test with");
}

#[test]
fn circuits_have_no_impossibly_tight_corners() {
    let layout = crate::layout::build(terrain::TERRAIN_HALF_SIZE);
    for c in &layout.circuits {
        let pts = road::resample(&c.track.points_2d, true, 4.0);
        let (i, r) = (0..pts.len())
            .map(|i| (i, road::corner_radius(&pts, i, 3)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        // Resampling can tighten an eased corner very slightly
        assert!(r > crate::layout::MIN_CORNER_RADIUS * 0.9, "{}: corner radius {r:.1} at {}", c.name, pts[i]);
    }
}

#[test]
fn ai_driven_player_finishes_the_race() {
    // Rivals' reactions and form vary race to race; check a few
    for seed in [1, 2, 3] {
        race_with_seed(seed);
    }
}

fn race_with_seed(seed: u32) {
    let track = race_track();
    let n = track.len();
    let mut cars: Vec<AiCar> = crate::RIVALS.iter().enumerate()
        .map(|(k, &style)| AiCar::new(&track, 0, 0, 1 + k, style))
        .collect();
    let mut player = CarPhysics::new(Vec3::ZERO, 0.0);
    let mut race = Race::new(0, (0..cars.len()).collect());
    race.seed = Some(seed);
    race.start(&mut player, &mut cars, &track);
    assert_eq!(race.position, 1, "position before first update");

    let mut driver = AiDriver::new(0, Personality::plain(1.0, 0.0), 1.0);
    let mut rescue = crate::rescue::WaterRescue::default();
    let dt = 1.0 / 60.0;
    let mut t = 0.0;
    let mut worst_up = 1.0_f32;
    let mut worst_off_line = 0.0_f32;
    // Contact can knock a car off the road; what matters is getting back on
    let mut off_since = [None::<f32>; crate::RACERS];
    let mut longest_off = 0.0_f32;
    let mut lap_times = Vec::new();
    let mut started_last = false;

    while t < 900.0 && !matches!(race.phase, Phase::Finished { .. }) {
        let traffic = crate::ai_cars::traffic(std::iter::once((&player, false)).chain(cars.iter().map(|c| (&c.physics, c.last_input.brake > 0.1))));
        let mut input = driver.input(&track, &player, &traffic, dt);
        if race.holding() {
            input = CarInput { handbrake: true, ..Default::default() };
        }
        player.update(&WorldGround, dt, input);
        // Knocked into a lake, the car is fished out just as in the game
        rescue.update(&mut player, dt);
        for car in &mut cars {
            car.update(&track, &WorldGround, &traffic, dt);
        }
        for car in &mut cars {
            collisions::car_vs_car(&mut player, &mut car.physics);
        }
        for i in 0..cars.len() {
            let (head, tail) = cars.split_at_mut(i + 1);
            for other in tail {
                collisions::car_vs_car(&mut head[i].physics, &mut other.physics);
            }
        }
        let events = race.update(dt, Vec2::new(player.pos.x, player.pos.z), &mut cars, &track);
        for e in events {
            if let crate::race::RaceEvent::Lap { time, .. } = e {
                lap_times.push(time);
            }
        }
        if t < 1.0 {
            started_last |= race.position == race.racer_count();
        }
        if !race.holding() {
            for (ci, c) in std::iter::once(&player).chain(cars.iter().map(|c| &c.physics)).enumerate() {
                worst_up = worst_up.min(c.up().y);
                let p = Vec2::new(c.pos.x, c.pos.z);
                let off = crate::ai_cars::nearest_in_window(&track.points, p, 0, 0, n).unwrap().1.sqrt();
                worst_off_line = worst_off_line.max(off);
                if off > 40.0 {
                    let since = *off_since[ci].get_or_insert(t);
                    longest_off = longest_off.max(t - since);
                } else {
                    off_since[ci] = None;
                }
            }
        }
        t += dt;
    }

    println!("track length {:.0} units, laps {:?}, position {}, worst up.y {:.2}, worst off-line {:.1}, longest off {:.1}s",
        n as f32 * 4.0, lap_times, race.position, worst_up, worst_off_line, longest_off);
    assert!(started_last, "player should start at the back of the grid");
    let Phase::Finished { total_time, .. } = race.phase else { panic!("race did not finish within 900s") };
    assert_eq!(race.laps_done, RACE_LAPS);
    assert_eq!(lap_times.len(), RACE_LAPS as usize);
    assert!(lap_times.iter().all(|&l| l > 15.0), "implausibly short lap: {lap_times:?}");
    assert!(total_time > lap_times.iter().sum::<f32>(), "total includes the run to the line");
    // Running wide off an embankment can rock a car well over; on its roof
    // is a roll
    assert!(worst_up > 0.0, "a car rolled over (up.y = {worst_up})");
    // A hard hit can throw a car ~100 units; it should be back racing within seconds
    assert!(longest_off < 8.0, "a car spent {longest_off:.1}s off the circuit");
}

#[test]
#[ignore]
fn render_layout_map() {
    let half = 9500.0;
    let t0 = std::time::Instant::now();
    let layout = crate::layout::build(half);
    println!("layout built in {:.2}s: {} circuits, {} roads", t0.elapsed().as_secs_f32(), layout.circuits.len(), layout.roads.len());
    let size = 900usize;
    let mut img = vec![30u8; size * size * 3];
    let mut plot = |p: Vec2, c: [u8; 3]| {
        let x = ((p.x + half) / (2.0 * half) * size as f32) as i32;
        let y = ((p.y + half) / (2.0 * half) * size as f32) as i32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (x + dx, y + dy);
                if px >= 0 && py >= 0 && (px as usize) < size && (py as usize) < size {
                    let i = (py as usize * size + px as usize) * 3;
                    img[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    };
    for r in &layout.roads {
        for p in &r.points_2d { plot(*p, [230, 200, 60]); }
    }
    for c in &layout.circuits {
        let len: f32 = c.track.points_2d.windows(2).map(|w| w[0].distance(w[1])).sum();
        let gap = crate::layout::self_gap(&c.track.points_2d);
        let centre = c.track.points_2d.iter().copied().sum::<Vec2>() / c.track.points_2d.len() as f32;
        println!("{:18} centre {:6.0},{:6.0} length {:6.0} self-gap {:5.0}", c.name, centre.x, centre.y, len, gap);
        for p in &c.track.points_2d { plot(*p, [230, 60, 60]); }
        plot(c.track.points_2d[0], [255, 255, 255]);
    }
    let mut ppm = format!("P6\n{size} {size}\n255\n").into_bytes();
    ppm.extend_from_slice(&img);
    std::fs::write(std::env::var("LAYOUT_PPM").unwrap_or("layout.ppm".into()), ppm).unwrap();
}

#[test]
#[ignore]
fn render_circuit_sketches() {
    // The circuits as built (placed, scaled, corners eased), north up,
    // start marked in green, the first stretch in red and the real outline
    // (fitted to the same box) in blue underneath
    let layout = crate::layout::build(terrain::TERRAIN_HALF_SIZE);
    let cell = 300usize;
    let cols = 6;
    let rows = layout.circuits.len().div_ceil(cols);
    let (w, h) = (cell * cols, cell * rows);
    let mut img = vec![30u8; w * h * 3];
    for (k, circuit) in layout.circuits.iter().enumerate() {
        let pts = &circuit.track.points_2d;
        let (lo, hi) = pts.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let scale = (cell as f32 - 30.0) / (hi - lo).max_element();
        let length: f32 = (0..pts.len()).map(|i| pts[i].distance(pts[(i + 1) % pts.len()])).sum();
        let real = crate::circuit_data::CIRCUITS.iter().find(|c| c.0 == circuit.name).map_or(0.0, |c| c.1);
        println!("{k:2} {:18} length {length:5.0} (real {real:5.0} m) gap {:4.0} at {}", circuit.name, crate::layout::self_gap(pts), (lo + hi) * 0.5);
        let (ox, oy) = ((k % cols) * cell + 15, (k / cols) * cell + 15);
        let mut plot = |p: Vec2, col: [u8; 3], r: i32| {
            let x = ((p.x - lo.x) * scale) as i32 + ox as i32;
            let y = ((hi.y - p.y) * scale) as i32 + oy as i32;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (x, y) = (x + dx, y + dy);
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        let i = (y as usize * w + x as usize) * 3;
                        img[i..i + 3].copy_from_slice(&col);
                    }
                }
            }
        };
        // The real outline underneath, fitted to the same bounding box, and
        // how far the built line strays from it (as a share of its size)
        if let Some(&(_, _, outline)) = crate::circuit_data::CIRCUITS.iter().find(|c| c.0 == circuit.name) {
            let (rlo, rhi) = outline.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), &[x, z]| (lo.min(Vec2::new(x, z)), hi.max(Vec2::new(x, z))));
            let k = (hi - lo).max_element() / (rhi - rlo).max_element();
            let real: Vec<Vec2> = outline.iter().map(|&[x, z]| (Vec2::new(x, z) - (rlo + rhi) * 0.5) * k + (lo + hi) * 0.5).collect();
            let dense: Vec<Vec2> = real.windows(2).flat_map(|w| (0..20).map(move |s| w[0].lerp(w[1], s as f32 / 20.0))).collect();
            let stray = pts.iter().map(|p| dense.iter().map(|q| q.distance(*p)).fold(f32::MAX, f32::min)).sum::<f32>() / pts.len() as f32;
            println!("   shape error {:.1}% of size", stray / (hi - lo).max_element() * 100.0);
            for q in &dense {
                plot(*q, [60, 110, 200], 1);
            }
        }
        for p in pts {
            plot(*p, [230, 230, 230], 0);
        }
        for p in pts.iter().take(pts.len() / 30) {
            plot(*p, [240, 70, 60], 1);
        }
        plot(pts[0], [80, 240, 80], 3);
    }
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    ppm.extend_from_slice(&img);
    std::fs::write(std::env::var("SKETCH_PPM").unwrap_or("sketches.ppm".into()), ppm).unwrap();
}

/// Hard off-road driving from many spots on the real map: weaving at full
/// throttle with the odd handbrake, and a side swipe every few seconds. Returns (rollovers, cars that ended stuck
/// on their side or roof, seconds spent tipped past 60 degrees).
fn offroad_torture(seconds: f32) -> (usize, usize, f32) {
    let _ = network();
    let dt = 1.0 / 60.0;
    let (mut rollovers, mut stuck, mut tipped) = (0, 0, 0.0);
    for k in 0..40 {
        let a = k as f32 * 2.39996;
        let r = 1500.0 + (k as f32 * 977.0) % 6500.0;
        let (x, z) = (a.cos() * r, a.sin() * r);
        if terrain::raw_height(x, z) < terrain::WATER_LEVEL + 2.0 {
            continue;
        }
        let mut car = CarPhysics::new(Vec3::new(x, 0.0, z), a);
        // Arrive flat out, as if from a long straight
        car.vel = car.forward() * 170.0;
        let mut t = 0.0;
        let mut was_upside_down = false;
        while t < seconds {
            let steer = (t * (0.7 + 0.05 * k as f32)).sin() * 1.2;
            let input = CarInput {
                throttle: 1.0,
                steer: steer.clamp(-1.0, 1.0),
                handbrake: (t * 0.37 + k as f32).sin() > 0.93,
                ..Default::default()
            };
            car.update(&WorldGround, dt, input);
            // Every few seconds, a side swipe like a car or tree strike
            if (t % 4.0) < dt {
                let side = car.rot * Vec3::X * if k % 2 == 0 { 1.0 } else { -1.0 };
                car.apply_impulse(side * 40.0 * 40.0, car.pos + car.forward() * 3.0);
            }
            let up = car.up().y;
            if up < 0.0 && !was_upside_down {
                rollovers += 1;
            }
            was_upside_down = up < 0.0;
            if up < 0.5 {
                tipped += dt;
            }
            t += dt;
        }
        // Stuck means it can't get back up: give it a moment hands-off,
        // in case the time ran out mid-tumble
        for _ in 0..(3.0 / dt) as usize {
            car.update(&WorldGround, dt, CarInput::default());
        }
        if car.up().y < 0.5 {
            stuck += 1;
        }
    }
    (rollovers, stuck, tipped)
}

#[test]
fn hard_offroad_driving_keeps_cars_on_their_wheels() {
    let (rollovers, stuck, tipped) = offroad_torture(30.0);
    println!("rollovers {rollovers}, ended stuck {stuck}, tipped for {tipped:.1}s");
    assert!(rollovers <= 2, "{rollovers} rollovers");
    assert_eq!(stuck, 0, "cars left on their side or roof");
    assert!(tipped < 10.0, "tipped over for {tipped:.1}s in total");
}

/// Height of the terrain mesh at (x, z) for a chunk grid of cell `step`,
/// interpolated over the same triangles the mesh uses.
fn terrain_mesh_at(x: f32, z: f32, step: f32) -> f32 {
    let o = -terrain::TERRAIN_HALF_SIZE;
    let (u, v) = ((x - o) / step, (z - o) / step);
    let (iu, iv) = (u.floor(), v.floor());
    let (fu, fv) = (u - iu, v - iv);
    let at = |du: f32, dv: f32| terrain::terrain_mesh_height(o + (iu + du) * step, o + (iv + dv) * step, step);
    let (tl, tr, bl, br) = (at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0));
    if terrain::cell_flipped(o + iu * step, o + iv * step, step) {
        // Split top-right to bottom-left
        if fu + fv <= 1.0 { tl + fu * (tr - tl) + fv * (bl - tl) } else { br + (1.0 - fu) * (bl - br) + (1.0 - fv) * (tr - br) }
    } else if fu >= fv {
        tl + fu * (tr - tl) + fv * (br - tr)
    } else {
        tl + fv * (bl - tl) + fu * (br - bl)
    }
}

/// How well the terrain mesh at cell size `step` fits around the roads:
/// (% of road surface points with grass more than 0.25 above them,
///  % of road edge points standing more than 1.5 above the grass).
fn road_fit(step: f32) -> (f32, f32) {
    let net = network();
    let (mut points, mut covered, mut edges, mut floating) = (0, 0, 0, 0);
    for path in &net.paths {
        let hw = path.kind.half_width();
        for i in (0..path.points.len()).step_by(6) {
            // Tunnels and viaducts leave the ground alone
            if path.structure[i] != road::Structure::Ground {
                continue;
            }
            let t = path.tangent(i);
            let side = Vec2::new(-t.y, t.x);
            for off in [-hw, -hw * 0.5, 0.0, hw * 0.5, hw] {
                let q = path.points[i] + side * off;
                // Only where this path owns the surface (not under a junction)
                if net.nearest(q.x, q.y).is_none_or(|s| s.kind != path.kind) {
                    continue;
                }
                let surface = terrain::terrain_height(q.x, q.y);
                let mesh = terrain_mesh_at(q.x, q.y, step);
                points += 1;
                covered += (mesh > surface + 0.25) as usize;
                if off.abs() == hw {
                    edges += 1;
                    floating += (mesh < surface - 1.5) as usize;
                }
            }
        }
    }
    (covered as f32 * 100.0 / points as f32, floating as f32 * 100.0 / edges as f32)
}

#[test]
fn terrain_fits_snugly_around_roads() {
    // Chunks are 500 across at 40/20/10 cells: near, middle and far detail.
    // Roads aren't drawn beyond the far level.
    // The far level (1800+ units away) can't follow every dip in the road;
    // a few percent there is about a pixel
    for (step, max_covered, max_floating) in [(12.5, 0.1, 0.1), (25.0, 0.4, 0.6), (50.0, 3.5, 12.0)] {
        let (covered, floating) = road_fit(step);
        println!("cell {step}: grass over road {covered:.2}%, road edge floating {floating:.2}%");
        assert!(covered < max_covered, "cell {step}: grass over the road at {covered:.2}% of points");
        assert!(floating < max_floating, "cell {step}: road edge floating at {floating:.2}% of points");
    }
}

/// Laps of circuit `name` (from its start line), driven by the AI's choices
/// either as they are or rounded to what a keyboard can do (full or nothing
/// on each key, the steering eased in as the game does). Returns (seconds
/// per lap, worst distance off the line, seconds spent off the road,
/// seconds with all four wheels in the air).
/// The real map in the wet: everything has rain-soaked grip.
struct WetGround;
impl crate::physics::Ground for WetGround {
    fn height(&self, x: f32, z: f32) -> f32 { WorldGround.height(x, z) }
    fn grip(&self, x: f32, z: f32) -> f32 { WorldGround.grip(x, z) * crate::weather::WET_GRIP }
    fn on_road(&self, x: f32, z: f32) -> f32 { WorldGround.on_road(x, z) }
    fn water(&self, x: f32, z: f32) -> Option<f32> { WorldGround.water(x, z) }
}

fn drive_laps(name: &str, laps: u32, keyboard: bool) -> (f32, f32, f32, f32) {
    drive_laps_on(name, laps, keyboard, false)
}

fn drive_laps_on(name: &str, laps: u32, keyboard: bool, wet: bool) -> (f32, f32, f32, f32) {
    drive_laps_with(name, laps, keyboard, wet, crate::physics::SETUPS[0])
}

fn drive_laps_with(name: &str, laps: u32, keyboard: bool, wet: bool, setup: crate::physics::Setup) -> (f32, f32, f32, f32) {
    let ground: &dyn crate::physics::Ground = if wet { &WetGround } else { &WorldGround };
    let net = network();
    let layout_names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
    let k = layout_names.iter().position(|&n| n == name).expect("circuit");
    let points = net.paths.iter().filter(|p| p.circuit()).nth(k).unwrap().points.clone();
    // The centreline, with room either side: this is about handling, not lines
    let track = AiTrack::new(points, crate::AI_TOP_SPEED);
    let n = track.len();
    let (pos, yaw) = track.pose(0, 0.0);
    let mut car = CarPhysics::new_on(ground, Vec3::new(pos.x, 0.0, pos.y), yaw);
    let mut driver = AiDriver::new(0, Personality::plain(1.0, 0.0), 1.0);
    car.setup = setup;
    driver.grip = if wet { crate::weather::WET_GRIP } else { 1.0 };
    let dt = 1.0 / 60.0;
    let (mut t, mut steer, mut worst, mut off, mut air) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
    let mut laps_done = 0;
    let mut last = 0;
    while laps_done < laps && t < 600.0 {
        let mut input = driver.input(&track, &car, &[], dt);
        if keyboard {
            let target = if input.steer > 0.25 { 1.0 } else if input.steer < -0.25 { -1.0 } else { 0.0 };
            let tune = crate::tuning::get();
            let rate = if target == 0.0 || target * steer < 0.0 { tune.steer_return } else { tune.steer_in };
            steer += (target - steer).clamp(-rate * dt, rate * dt);
            input = CarInput { steer, throttle: if input.throttle > 0.5 { 1.0 } else { 0.0 }, brake: if input.brake > 0.2 { 1.0 } else { 0.0 }, handbrake: false };
        }
        car.update(ground, dt, input);
        t += dt;
        let p = Vec2::new(car.pos.x, car.pos.z);
        worst = worst.max(track.points[driver.progress_idx].distance(p));
        if road::on_road(p.x, p.y) < 0.5 {
            off += dt;
        }
        if car.grounded_wheels() == 0 {
            air += dt;
        }
        if driver.progress_idx < last && last > n * 3 / 4 {
            laps_done += 1;
        }
        last = driver.progress_idx;
    }
    (t / laps as f32, worst, off, air)
}

#[test]
#[ignore]
fn keyboard_driving_report() {
    for name in ["STUMBLER RING", "MONACO", "SUZUKA", "MONZA"] {
        let (a_lap, a_worst, a_off, a_air) = drive_laps(name, 2, false);
        let (k_lap, k_worst, k_off, k_air) = drive_laps(name, 2, true);
        println!("{name:14} analogue {a_lap:6.1}s/lap off-line {a_worst:4.0} off-road {a_off:4.1}s air {a_air:3.1}s | keyboard {k_lap:6.1}s/lap off-line {k_worst:4.0} off-road {k_off:4.1}s air {k_air:3.1}s");
        let (w_lap, w_worst, w_off, _) = drive_laps_on(name, 2, true, true);
        println!("{:14} wet keyboard {w_lap:6.1}s/lap off-line {w_worst:4.0} off-road {w_off:4.1}s", "");
    }
}

#[test]
fn keyboard_driving_stays_on_the_road_without_taking_off() {
    // On/off keys (with the game's steering ease) should lap about as well
    // as smooth analogue inputs, and nothing on a circuit should launch a car
    for name in ["STUMBLER RING", "MONACO"] {
        let (a_lap, _, _, _) = drive_laps(name, 1, false);
        let (k_lap, worst, off, air) = drive_laps(name, 1, true);
        println!("{name}: keyboard {k_lap:.1}s vs analogue {a_lap:.1}s, {worst:.0} off the line, {off:.1}s off the road, {air:.1}s in the air");
        assert!(k_lap < a_lap * 1.05, "{name}: keyboard lap {k_lap:.1}s vs {a_lap:.1}s");
        assert!(off < 0.5 && worst < 30.0, "{name}: ran off the road ({worst:.0} off the line, {off:.1}s)");
        assert!(air < 0.5, "{name}: airborne for {air:.1}s");
    }
}

#[test]
fn slipstream_lets_a_following_car_close_in() {
    struct Flat;
    impl crate::physics::Ground for Flat {
        fn height(&self, _x: f32, _z: f32) -> f32 { 0.0 }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn normal(&self, _x: f32, _z: f32) -> Vec3 { Vec3::Y }
    }
    let gap_after = |draft: bool| {
        let mut lead = CarPhysics::new_on(&Flat, Vec3::new(0.0, 0.0, -30.0), 0.0);
        let mut chase = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        for car in [&mut lead, &mut chase] {
            car.vel = car.forward() * 150.0;
        }
        let input = CarInput { throttle: 1.0, ..Default::default() };
        for _ in 0..(6.0 * 60.0) as usize {
            let drafts = crate::ai_cars::drafts(&[&lead, &chase]);
            chase.draft = if draft { drafts[1] } else { 0.0 };
            lead.update(&Flat, 1.0 / 60.0, input);
            chase.update(&Flat, 1.0 / 60.0, input);
        }
        // How far the leader is ahead (negative once it's been passed)
        (lead.pos - chase.pos).dot(lead.forward())
    };
    let (with, without) = (gap_after(true), gap_after(false));
    println!("gap after 6s: {with:.1} in the slipstream, {without:.1} out of it");
    assert!(without > 28.0, "cars should hold station without it: {without:.1}");
    assert!(with < without - 15.0, "slipstream gained too little: {with:.1} vs {without:.1}");
}

/// Two cars side by side at speed; the left one steers into the right one
/// for a moment. Returns (the victim's worst heading change in degrees,
/// its speed lost, the attacker's worst heading change).
fn side_swipe(speed: f32, steer_time: f32) -> (f32, f32, f32) {
    struct Flat;
    impl crate::physics::Ground for Flat {
        fn height(&self, _x: f32, _z: f32) -> f32 { 0.0 }
        fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn on_road(&self, _x: f32, _z: f32) -> f32 { 1.0 }
        fn normal(&self, _x: f32, _z: f32) -> Vec3 { Vec3::Y }
    }
    let mut a = CarPhysics::new_on(&Flat, Vec3::new(-6.0, 0.0, 0.0), 0.0);
    let mut b = CarPhysics::new_on(&Flat, Vec3::new(6.0, 0.0, 0.0), 0.0);
    for c in [&mut a, &mut b] {
        c.vel = c.forward() * speed;
    }
    // Which way is "toward b" for a's steering
    let toward = { let mut probe = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0); probe.vel = probe.forward() * 40.0;
        for _ in 0..20 { probe.update(&Flat, 1.0 / 60.0, CarInput { steer: 1.0, throttle: 0.5, ..Default::default() }); } if probe.pos.x > 0.0 { 1.0 } else { -1.0 } };
    let (yaw_a, yaw_b) = (a.yaw(), b.yaw());
    let (mut worst_a, mut worst_b) = (0.0_f32, 0.0_f32);
    let dt = 1.0 / 60.0;
    for k in 0..(3.0 / dt) as usize {
        let t = k as f32 * dt;
        let steer = if t < steer_time { toward } else { 0.0 };
        a.update(&Flat, dt, CarInput { steer, throttle: 0.6, ..Default::default() });
        b.update(&Flat, dt, CarInput { throttle: 0.6, ..Default::default() });
        collisions::car_vs_car(&mut a, &mut b);
        let wrap = |d: f32| (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        worst_a = worst_a.max(wrap(a.yaw() - yaw_a).abs());
        worst_b = worst_b.max(wrap(b.yaw() - yaw_b).abs());
    }
    (worst_b.to_degrees(), speed - b.vel.length(), worst_a.to_degrees())
}

#[test]
#[ignore]
fn contact_report() {
    for speed in [60.0, 100.0, 140.0] {
        for steer_time in [0.25, 0.6] {
            let (victim, lost, attacker) = side_swipe(speed, steer_time);
            println!("{:3.0} km/h, steering in for {steer_time}s: victim turned {victim:5.1} deg, lost {lost:5.1} u/s; attacker turned {attacker:5.1} deg", speed * 1.8);
        }
    }
}

/// How a race went, from `race_report`.
#[derive(Default)]
struct RaceStats {
    /// Places changing hands (and staying changed for a second).
    passes: usize,
    /// Hard knocks (one per pair per second), with the player's car or
    /// between rivals.
    player_contacts: usize,
    rival_contacts: usize,
    /// Car-seconds off the road, and spins.
    off_road: f32,
    spins: usize,
    /// Rivals' average distance from the racing line (units).
    off_line: f32,
    /// Player's finishing place, and seconds from first to last at the end.
    position: usize,
    spread: f32,
    /// Per rival (in `RIVALS` order): drifts started, hard knocks it was
    /// in, and places gained from the grid.
    drifts: [usize; crate::RIVALS.len()],
    knocks: [usize; crate::RIVALS.len()],
    gained: [i32; crate::RIVALS.len()],
}

/// A full race on circuit `name` with the AI driving the player's car too
/// (at `player_pace`), plus slipstreams as the game has them.
fn race_report(name: &str, player_pace: f32, seed: u32) -> RaceStats {
    let net = network();
    let layout_names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
    let k = layout_names.iter().position(|&n| n == name).expect("circuit");
    let points = net.paths.iter().filter(|p| p.circuit()).nth(k).unwrap().points.clone();
    let track = AiTrack::racing(points, crate::AI_TOP_SPEED, crate::ai_cars::RACING_ROOM);
    let n = track.len();
    let mut cars: Vec<AiCar> = crate::RIVALS.iter().enumerate()
        .map(|(k, &style)| AiCar::new(&track, 0, 0, 1 + k, style)).collect();
    let mut player = CarPhysics::new(Vec3::ZERO, 0.0);
    let mut race = Race::new(0, (0..cars.len()).collect());
    race.seed = Some(seed);
    race.start(&mut player, &mut cars, &track);
    let mut driver = AiDriver::new(0, Personality::plain(player_pace, 0.0), 1.0);
    let dt = 1.0 / 60.0;
    let mut t = 0.0;
    let mut stats = RaceStats::default();
    let (mut order, mut pending): (Vec<usize>, Option<(Vec<usize>, f32)>) = (Vec::new(), None);
    let mut spinning = [false; crate::RACERS];
    let mut was_drifting = [false; crate::RIVALS.len()];
    let place = |cars: &[AiCar], driver: &AiDriver, k: usize| {
        let mine = cars[k].driver.progress(n);
        cars.iter().filter(|c| c.driver.progress(n) > mine).count() + usize::from(driver.progress(n) > mine)
    };
    let grid: Vec<usize> = (0..cars.len()).map(|k| place(&cars, &driver, k)).collect();
    // When each pair (by car index, player 0) last had a hard knock
    let mut last_knock = [[-10.0_f32; crate::RACERS]; crate::RACERS];
    while t < 600.0 && !matches!(race.phase, Phase::Finished { .. }) {
        let all: Vec<&CarPhysics> = std::iter::once(&player).chain(cars.iter().map(|c| &c.physics)).collect();
        let drafts = crate::ai_cars::drafts(&all);
        player.draft = drafts[0];
        for (c, d) in cars.iter_mut().zip(&drafts[1..]) {
            c.physics.draft = *d;
        }
        let traffic = crate::ai_cars::traffic(std::iter::once((&player, false)).chain(cars.iter().map(|c| (&c.physics, c.last_input.brake > 0.1))));
        let mut input = driver.input(&track, &player, &traffic, dt);
        if race.holding() {
            input = CarInput { handbrake: true, ..Default::default() };
        }
        player.update(&WorldGround, dt, input);
        for car in &mut cars {
            car.update(&track, &WorldGround, &traffic, dt);
        }
        let mut knock = |a: usize, b: usize, hit: bool, stats: &mut RaceStats| {
            if hit && t - last_knock[a][b] > 1.0 {
                if a == 0 { stats.player_contacts += 1 } else { stats.rival_contacts += 1 }
                for c in [a, b].into_iter().filter(|&c| c > 0) {
                    stats.knocks[c - 1] += 1;
                }
            }
            if hit {
                last_knock[a][b] = t;
            }
        };
        for (ci, car) in cars.iter_mut().enumerate() {
            let hit = collisions::car_vs_car(&mut player, &mut car.physics).is_some_and(|h| h.impact_speed > 6.0);
            knock(0, ci + 1, hit, &mut stats);
        }
        for i in 0..cars.len() {
            let (head, tail) = cars.split_at_mut(i + 1);
            for (j, other) in tail.iter_mut().enumerate() {
                let hit = collisions::car_vs_car(&mut head[i].physics, &mut other.physics).is_some_and(|h| h.impact_speed > 6.0);
                knock(i + 1, i + j + 2, hit, &mut stats);
            }
        }
        race.update(dt, Vec2::new(player.pos.x, player.pos.z), &mut cars, &track);
        for (k, car) in cars.iter().enumerate() {
            if car.driver.drifting() && !was_drifting[k] {
                stats.drifts[k] += 1;
            }
            was_drifting[k] = car.driver.drifting();
        }
        if !race.holding() {
            // Order by progress round the track; a change counts once it has
            // held for a second, so cars side by side don't count over and over
            let mut progress: Vec<(usize, f32)> = std::iter::once(driver.progress(n))
                .chain(cars.iter().map(|c| c.driver.progress(n))).enumerate().collect();
            progress.sort_by(|a, b| b.1.total_cmp(&a.1));
            let now: Vec<usize> = progress.iter().map(|p| p.0).collect();
            if order.is_empty() {
                order = now.clone();
            }
            if now == order {
                pending = None;
            } else if pending.as_ref().is_none_or(|(p, _)| *p != now) {
                pending = Some((now, t));
            } else if let Some((p, since)) = &pending {
                if t - since > 1.0 {
                    stats.passes += 1;
                    order = p.clone();
                    pending = None;
                }
            }
            for car in &cars {
                let p = Vec2::new(car.physics.pos.x, car.physics.pos.z);
                stats.off_line += track.points[car.driver.progress_idx].distance(p) * dt / cars.len() as f32;
            }
            for (ci, c) in std::iter::once(&player).chain(cars.iter().map(|c| &c.physics)).enumerate() {
                if road::on_road(c.pos.x, c.pos.z) < 0.5 {
                    stats.off_road += dt;
                }
                let side = c.vel.dot(c.rot * Vec3::X);
                let slip = side.atan2(c.speed.abs().max(1.0)).abs();
                let now_spinning = slip > 1.2 && c.vel.length() > 10.0;
                if now_spinning && !spinning[ci] {
                    stats.spins += 1;
                }
                spinning[ci] = now_spinning;
            }
        }
        t += dt;
    }
    let Phase::Finished { position, total_time } = race.phase else {
        panic!("{name} seed {seed}: race didn't finish; player at {:?} on lap {} point {}, {:.1} from the line", player.pos, driver.laps, driver.progress_idx,
            track.points[driver.progress_idx].distance(Vec2::new(player.pos.x, player.pos.z)))
    };
    // Spread: how far round the last car was when the player finished, in seconds
    let last = cars.iter().map(|c| c.driver.progress(n)).fold(f32::MAX, f32::min);
    let first = cars.iter().map(|c| c.driver.progress(n)).fold(driver.progress(n), f32::max);
    stats.spread = (first - last) * 4.0 / (n as f32 * 4.0 / (total_time / 3.0));
    stats.position = position;
    for k in 0..cars.len() {
        stats.gained[k] = grid[k] as i32 - place(&cars, &driver, k) as i32;
    }
    stats.off_line /= t;
    stats
}

#[test]
#[ignore]
fn race_quality_report() {
    let only = std::env::var("RQ_ONLY").ok();
    for name in ["STUMBLER RING", "SUZUKA", "MONACO"].into_iter().filter(|n| only.as_deref().is_none_or(|o| o == *n)) {
        for pace in [1.0, 0.92] {
            let mut sum = RaceStats::default();
            let count: u32 = std::env::var("RQ_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
            let seeds: Vec<u32> = (0..count).map(|k| k * 2 + 1).collect();
            for &seed in &seeds {
                let s = race_report(name, pace, seed);
                sum.passes += s.passes;
                sum.player_contacts += s.player_contacts;
                sum.rival_contacts += s.rival_contacts;
                sum.off_road += s.off_road;
                sum.spins += s.spins;
                sum.position += s.position;
                sum.spread += s.spread;
                sum.off_line += s.off_line;
            }
            let k = seeds.len() as f32;
            println!("{name:14} pace {pace:4}: per race {:4.1} passes, {:3.1} knocks with player, {:3.1} between rivals, {:4.1}s off road, {:3.1} spins; places {:3.1}, spread {:4.1}s, {:3.1} off the line",
                sum.passes as f32 / k, sum.player_contacts as f32 / k, sum.rival_contacts as f32 / k, sum.off_road / k, sum.spins as f32 / k, sum.position as f32 / k, sum.spread / k, sum.off_line / k);
        }
    }
}

#[test]
#[ignore]
fn race_seed_sweep() {
    // The seed is made odd, so step through odd ones
    for seed in (1..=121).step_by(2) {
        let result = std::panic::catch_unwind(|| race_with_seed(seed));
        if result.is_err() {
            println!("seed {seed} failed");
        }
    }
}

#[test]
fn keyboard_driving_in_the_wet_stays_on_the_road() {
    // Slower in the rain, but still drivable on the keys
    let (dry, _, _, _) = drive_laps("MONACO", 1, true);
    let (wet, worst, off, _) = drive_laps_on("MONACO", 1, true, true);
    println!("Monaco on the keys: dry {dry:.1}s, wet {wet:.1}s, {worst:.0} off the line, {off:.1}s off the road");
    assert!(wet > dry, "the wet should cost time");
    assert!(off < 0.5 && worst < 30.0, "ran off the road in the wet ({worst:.0} off the line, {off:.1}s)");
}

#[test]
#[ignore]
fn setup_report() {
    for setup in crate::physics::SETUPS {
        // As a player would, find the quickest pace each setup can be
        // driven at without leaving the road
        let best = |name: &str| {
            [0.92, 0.96, 1.0, 1.04, 1.08, 1.12].iter()
                .map(|&g| drive_laps_paced(name, setup, g))
                .filter(|&(_, off)| off < 0.5)
                .map(|(lap, _)| lap)
                .fold(f32::MAX, f32::min)
        };
        let (monza, monaco) = (best("MONZA"), best("MONACO"));
        struct Flat;
        impl crate::physics::Ground for Flat {
            fn height(&self, _x: f32, _z: f32) -> f32 { 0.0 }
            fn grip(&self, _x: f32, _z: f32) -> f32 { 1.0 }
            fn on_road(&self, _x: f32, _z: f32) -> f32 { 1.0 }
            fn normal(&self, _x: f32, _z: f32) -> Vec3 { Vec3::Y }
        }
        let mut car = CarPhysics::new_on(&Flat, Vec3::ZERO, 0.0);
        car.setup = setup;
        for _ in 0..(30.0 * 60.0) as usize {
            car.update(&Flat, 1.0 / 60.0, CarInput { throttle: 1.0, ..Default::default() });
        }
        println!("{:9} top {:3.0} km/h  Monza {monza:5.1}s  Monaco {monaco:5.1}s", setup.name, car.speed * crate::KMH_PER_UNIT);
    }
}

/// One lap of `name` on the racing line with `setup`, planning corners for
/// `grip` times the usual grip. Returns (lap time, seconds off the road).
fn drive_laps_paced(name: &str, setup: crate::physics::Setup, grip: f32) -> (f32, f32) {
    let net = network();
    let layout_names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
    let k = layout_names.iter().position(|&n| n == name).expect("circuit");
    let points = net.paths.iter().filter(|p| p.circuit()).nth(k).unwrap().points.clone();
    let track = AiTrack::racing(points, crate::AI_TOP_SPEED * 1.2, crate::ai_cars::RACING_ROOM);
    let n = track.len();
    let (pos, yaw) = track.pose(0, 0.0);
    let mut car = CarPhysics::new(Vec3::new(pos.x, 0.0, pos.y), yaw);
    car.setup = setup;
    let mut driver = AiDriver::new(0, Personality::plain(1.0, 0.0), 1.0);
    driver.grip = grip;
    let dt = 1.0 / 60.0;
    let (mut t, mut off, mut last, mut laps) = (0.0_f32, 0.0_f32, 0, 0);
    while laps < 1 && t < 300.0 {
        let input = driver.input(&track, &car, &[], dt);
        car.update(&WorldGround, dt, input);
        t += dt;
        if road::on_road(car.pos.x, car.pos.z) < 0.5 {
            off += dt;
        }
        if driver.progress_idx < last && last > n * 3 / 4 {
            laps += 1;
        }
        last = driver.progress_idx;
    }
    (t, off)
}

#[test]
fn a_full_grid_reaches_no_further_into_the_last_corner() {
    // Lining up the whole field behind the line bends no more than the
    // old four-car grid did (3 + 3 * 7 points back): the back of the grid
    // doesn't wrap round into the last corner
    let net = network();
    let names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
    for (path, name) in net.paths.iter().filter(|p| p.circuit()).zip(names) {
        let pts = &path.points;
        let n = pts.len();
        let dir = |i: usize| (pts[(i + 1) % n] - pts[i]).normalize();
        let line = dir(n - 1);
        let bend = |back: usize| (1..=back).map(|k| dir(n - k).angle_between(line).abs()).fold(0.0_f32, f32::max);
        let (front_row, field) = (bend(26), bend(3 + (crate::RACERS - 1) * 5 + 2));
        assert!(field - front_row < 10f32.to_radians(), "{name}: the back of the grid turns {:.0} deg more than a four-car grid's", (field - front_row).to_degrees());
    }
}

/// Two solo laps of `name` on the racing line by a driver who drifts every
/// slow corner it can (`drift` 1) or none (0). Returns (seconds per lap,
/// drifts, drifts paid out, seconds off the road, spins).
fn drift_laps(name: &str, drift: f32) -> (f32, usize, usize, f32, usize) {
    let net = network();
    let layout_names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
    let k = layout_names.iter().position(|&n| n == name).expect("circuit");
    let points = net.paths.iter().filter(|p| p.circuit()).nth(k).unwrap().points.clone();
    let track = AiTrack::racing(points, crate::AI_TOP_SPEED, crate::ai_cars::RACING_ROOM);
    let n = track.len();
    let mut car = AiCar::new(&track, 0, 0, 1, Personality { drift, ..Personality::plain(0.95, 0.0) });
    let dt = 1.0 / 60.0;
    let (mut t, mut off, mut last, mut laps) = (0.0_f32, 0.0_f32, 0, 0);
    let (mut drifts, mut paid, mut spins, mut was_drifting, mut was_spinning) = (0, 0, 0, false, false);
    // A flying start: time from the first crossing of the line
    car.physics.vel = car.physics.heading() * 40.0;
    let mut t0 = None;
    while laps < 3 && t < 400.0 {
        let before = car.boost.timer;
        car.update(&track, &WorldGround, &[], dt);
        t += dt;
        if t0.is_some() {
            if car.physics.drifting && !was_drifting { drifts += 1; }
            if car.boost.timer > before + 0.1 { paid += 1; }
            if road::on_road(car.physics.pos.x, car.physics.pos.z) < 0.5 { off += dt; }
            let c = &car.physics;
            let slip = c.vel.dot(c.rot * Vec3::X).atan2(c.speed.abs().max(1.0)).abs();
            let spinning = slip > 1.2 && c.vel.length() > 10.0;
            if spinning && !was_spinning { spins += 1; }
            was_spinning = spinning;
        }
        was_drifting = car.physics.drifting;
        if car.driver.progress_idx < last && last > n * 3 / 4 {
            laps += 1;
            t0.get_or_insert(t);
        }
        last = car.driver.progress_idx;
    }
    ((t - t0.unwrap_or(0.0)) / 2.0, drifts, paid, off, spins)
}

#[test]
#[ignore]
fn drift_report() {
    let names: Vec<&str> = crate::layout::build(terrain::TERRAIN_HALF_SIZE).circuits.iter().map(|c| c.name).collect();
    let only = std::env::var("DRIFT_ONLY").ok();
    for name in names.into_iter().filter(|n| only.as_deref().is_none_or(|o| o == *n)) {
        let (grip, ..) = drift_laps(name, 0.0);
        let (lap, drifts, paid, off, spins) = drift_laps(name, 1.0);
        println!("{name:14} grip {grip:6.2}s/lap, drifting {lap:6.2}s/lap ({:+.2}): {drifts} drifts, {paid} paid, {off:4.1}s off road, {spins} spins", lap - grip);
    }
}

#[test]
#[ignore]
fn personality_report() {
    // Each rival's drifts, knocks and places gained per race, over a few
    // circuits and seeds
    let names = ["STUMBLER RING", "SUZUKA", "MONACO", "HUNGARORING"];
    let seeds = [1, 3, 5];
    let rivals = crate::RIVALS.len();
    let (mut drifts, mut knocks, mut gained) = (vec![0usize; rivals], vec![0usize; rivals], vec![0i32; rivals]);
    for name in names {
        for seed in seeds {
            let s = race_report(name, 0.96, seed);
            for k in 0..rivals {
                drifts[k] += s.drifts[k];
                knocks[k] += s.knocks[k];
                gained[k] += s.gained[k];
            }
        }
    }
    let races = (names.len() * seeds.len()) as f32;
    for (k, p) in crate::RIVALS.iter().enumerate() {
        println!("rival {k} pace {:.3} aggr {:.2} comp {:.2} drift {:.1}: {:4.1} drifts, {:3.1} knocks, {:+4.1} places per race",
            p.pace, p.aggression, p.composure, p.drift, drifts[k] as f32 / races, knocks[k] as f32 / races, gained[k] as f32 / races);
    }
}

/// Dev aid: the whole map as `world.ppm` (or `$WORLD_PPM`): water, woods
/// and height, every road, trail and circuit, and the gravel stages.
#[test]
#[ignore]
fn render_world_map() {
    let net = network();
    let half = terrain::TERRAIN_HALF_SIZE;
    let size = 950usize;
    let mut img = vec![0u8; size * size * 3];
    let to_world = |px: usize, py: usize| Vec2::new((px as f32 + 0.5) / size as f32 * 2.0 * half - half, (py as f32 + 0.5) / size as f32 * 2.0 * half - half);
    for py in 0..size {
        for px in 0..size {
            let p = to_world(px, py);
            let h = terrain::raw_height(p.x, p.y);
            let c = if h < terrain::WATER_LEVEL {
                [40, 70, 140]
            } else {
                let wood = (crate::trees::density(p.x, p.y, h) * 1.5).min(1.0);
                let shade = (0.55 + (h / 400.0).clamp(0.0, 0.45)) * 255.0;
                [(shade * (0.75 - 0.45 * wood)) as u8, (shade * (0.8 - 0.2 * wood)) as u8, (shade * (0.55 - 0.35 * wood)) as u8]
            };
            // Where a stage can't go (near a circuit): tinted
            let c = if h >= terrain::WATER_LEVEL && !crate::stages::open(net, p) { [c[0] / 2 + 100, c[1] / 2, c[2] / 2] } else { c };
            let i = (py * size + px) * 3;
            img[i..i + 3].copy_from_slice(&c);
        }
    }
    let mut plot = |p: Vec2, c: [u8; 3], r: i32| {
        let x = ((p.x + half) / (2.0 * half) * size as f32) as i32;
        let y = ((p.y + half) / (2.0 * half) * size as f32) as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                let (px, py) = (x + dx, y + dy);
                if px >= 0 && py >= 0 && (px as usize) < size && (py as usize) < size {
                    let i = (py as usize * size + px as usize) * 3;
                    img[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    };
    for path in &net.paths {
        let (c, r) = match path.kind {
            road::PathKind::Circuit => ([230, 50, 50], 1),
            road::PathKind::Road => ([235, 205, 70], 0),
            road::PathKind::Trail => ([150, 110, 70], 0),
            road::PathKind::Rail => ([20, 20, 20], 0),
            road::PathKind::Gravel => ([255, 140, 0], 1),
        };
        for p in path.points.iter().step_by(2) {
            plot(*p, c, r);
        }
        for &k in &path.ramps {
            plot(path.points[k], [255, 255, 255], 1);
        }
    }
    // Grid every 2000 units for reading positions off
    for k in -4..=4 {
        for t in (-95..=95).map(|t| t as f32 * 100.0) {
            plot(Vec2::new(k as f32 * 2000.0, t), [90, 90, 90], 0);
            plot(Vec2::new(t, k as f32 * 2000.0), [90, 90, 90], 0);
        }
    }
    for (path, name) in net.paths.iter().filter(|p| p.kind == road::PathKind::Gravel).zip(crate::stages::names()) {
        let len = path.points.len() as f32 * road::SAMPLE_SPACING;
        let span = path.points[0].distance(*path.points.last().unwrap());
        println!("{name:14} {len:6.0} units ({span:5.0} start to finish), {} jumps", path.ramps.len());
    }
    let mut ppm = format!("P6\n{size} {size}\n255\n").into_bytes();
    ppm.extend_from_slice(&img);
    std::fs::write(std::env::var("WORLD_PPM").unwrap_or("world.ppm".into()), ppm).unwrap();
}

/// The AI's line along gravel stage `name`.
fn stage_track(name: &str) -> AiTrack {
    let net = network();
    let k = crate::stages::names().iter().position(|&n| n == name).expect("stage");
    crate::ai_cars::stage_track(net.paths.iter().filter(|p| p.kind == road::PathKind::Gravel).nth(k).unwrap())
}

/// One AI car alone down stage `name` at `pace`: (seconds to the finish,
/// seconds off the road, seconds in the air, lowest up.y, finished).
/// With `keyboard`, through on/off keys only (with the game's steering
/// ease), the way a player on a keyboard drives.
fn drive_stage_with(name: &str, pace: f32, keyboard: bool) -> (f32, f32, f32, f32, bool) {
    let track = stage_track(name);
    let n = track.len();
    let mut car = AiCar::new(&track, 0, 2, 1, Personality::plain(pace, 0.0));
    let finish = n - 1 - crate::race::STAGE_RUNOFF;
    let dt = 1.0 / 60.0;
    let (mut t, mut off, mut air, mut worst) = (0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32);
    let mut steer = 0.0_f32;
    while t < 600.0 && car.driver.progress_idx < finish {
        if keyboard {
            // The AI's wishes, through on/off keys
            let wish = car.driver.input(&track, &car.physics, &[], dt);
            let target = if wish.steer > 0.25 { 1.0 } else if wish.steer < -0.25 { -1.0 } else { 0.0 };
            let tune = crate::tuning::get();
            let rate = if target == 0.0 || target * steer < 0.0 { tune.steer_return } else { tune.steer_in };
            steer += (target - steer).clamp(-rate * dt, rate * dt);
            let input = CarInput { steer, throttle: if wish.throttle > 0.5 { 1.0 } else { 0.0 }, brake: if wish.brake > 0.2 { 1.0 } else { 0.0 }, handbrake: false };
            car.physics.update(&WorldGround, dt, input);
            car.boost.update(&mut car.physics, dt);
        } else {
            car.update(&track, &WorldGround, &[], dt);
        }
        t += dt;
        let p = car.physics.pos;
        if road::on_road(p.x, p.z) < 0.5 { off += dt; }
        if car.physics.grounded_wheels() == 0 { air += dt; }
        if car.physics.up().y < 0.3 && worst >= 0.3 {
            let near_ramp = road::network().unwrap().paths.iter().filter(|p| p.kind == road::PathKind::Gravel)
                .flat_map(|p| p.ramps.iter().map(|&r| p.points[r]))
                .map(|q| q.distance(Vec2::new(p.x, p.z))).fold(f32::MAX, f32::min);
            println!("  rolled at point {} ({}), {:.0} from a jump, speed {:.0}", car.driver.progress_idx, p, near_ramp, car.physics.speed);
        }
        worst = worst.min(car.physics.up().y);
    }
    if car.driver.progress_idx < finish {
        println!("  stuck at point {} of {}, at {} ({:.0} from the line), speed {:.1}", car.driver.progress_idx, n, car.physics.pos,
            track.points[car.driver.progress_idx].distance(Vec2::new(car.physics.pos.x, car.physics.pos.z)), car.physics.speed);
    }
    (t, off, air, worst, car.driver.progress_idx >= finish)
}

#[test]
#[ignore]
fn stage_report() {
    let _ = network();
    for &name in crate::stages::names() {
        for (pace, keyboard) in [(1.0, false), (0.9, false), (1.0, true)] {
            let (t, off, air, worst, done) = drive_stage_with(name, pace, keyboard);
            let pace = if keyboard { "keys".to_string() } else { pace.to_string() };
            println!("{name:14} pace {pace:3}: {:4.0}s ({}:{:02}) {}, {off:4.1}s off road, {air:4.1}s in the air, worst up {worst:.2}",
                t, (t / 60.0) as u32, (t % 60.0) as u32, if done { "finished" } else { "DID NOT FINISH" });
        }
    }
}

#[test]
fn gravel_stages_have_no_steps_but_their_jumps() {
    let net = network();
    let stages: Vec<_> = net.paths.iter().filter(|p| p.kind == road::PathKind::Gravel).collect();
    assert!(stages.len() >= 2, "only {} gravel stages", stages.len());
    for path in stages {
        assert!(!path.ramps.is_empty(), "a stage without jumps");
        // Never alongside itself: the surfaces and shoulders of two
        // stretches would meet at different heights
        let clear = 2.0 * (road::GRAVEL_HALF_WIDTH + road::SHOULDER);
        for (i, p) in path.points.iter().enumerate().step_by(3) {
            for (j, q) in path.points.iter().enumerate().skip(i + 60).step_by(3) {
                assert!(p.distance(*q) > clear, "stage runs back past itself at {p} (points {i} and {j})");
            }
        }
        for i in 1..path.points.len() {
            let lip = path.ramps.iter().any(|&r| (r + road::RAMP_SAMPLES..r + road::RAMP_SAMPLES + 2).contains(&i));
            let p = path.points[i];
            let (a, b) = (terrain::solid_height(path.points[i - 1].x, path.points[i - 1].y), terrain::solid_height(p.x, p.y));
            assert!(lip || (a - b).abs() < 1.5, "step of {:.1} at {p} (point {i})", b - a);
        }
    }
}

/// A full stage race on `name` with the AI driving the player's car too:
/// (finishing time, place, rivals who reached the finish, longest any car
/// was off the road in one go).
fn stage_race(name: &str, seed: u32) -> (f32, usize, usize, f32) {
    let _ = network();
    let track = stage_track(name);
    let n = track.len();
    let mut cars: Vec<AiCar> = crate::RIVALS.iter().enumerate().map(|(k, &style)| AiCar::new(&track, 0, 0, 1 + k, style)).collect();
    let mut player = CarPhysics::new(Vec3::ZERO, 0.0);
    let mut race = Race::new(0, (0..cars.len()).collect());
    race.stage = true;
    race.seed = Some(seed);
    race.start(&mut player, &mut cars, &track);
    let mut driver = AiDriver::new(0, Personality::plain(0.93, 0.0), 1.0);
    let dt = 1.0 / 60.0;
    let mut t = 0.0;
    let mut off_since = [None::<f32>; crate::RACERS];
    let mut longest_off = 0.0_f32;
    while t < 600.0 && !matches!(race.phase, Phase::Finished { .. }) {
        let traffic = crate::ai_cars::traffic(std::iter::once((&player, false)).chain(cars.iter().map(|c| (&c.physics, c.last_input.brake > 0.1))));
        let mut input = driver.input(&track, &player, &traffic, dt);
        if race.holding() {
            input = CarInput { handbrake: true, ..Default::default() };
        }
        player.update(&WorldGround, dt, input);
        for car in &mut cars {
            car.update(&track, &WorldGround, &traffic, dt);
        }
        for car in &mut cars {
            collisions::car_vs_car(&mut player, &mut car.physics);
        }
        for i in 0..cars.len() {
            let (head, tail) = cars.split_at_mut(i + 1);
            for other in tail {
                collisions::car_vs_car(&mut head[i].physics, &mut other.physics);
            }
        }
        race.update(dt, Vec2::new(player.pos.x, player.pos.z), &mut cars, &track);
        for (ci, c) in std::iter::once(&player).chain(cars.iter().map(|c| &c.physics)).enumerate() {
            if road::on_road(c.pos.x, c.pos.z) < 0.3 {
                let since = *off_since[ci].get_or_insert(t);
                longest_off = longest_off.max(t - since);
            } else {
                off_since[ci] = None;
            }
        }
        t += dt;
    }
    let Phase::Finished { position, total_time } = race.phase else {
        panic!("{name}: stage race didn't finish; player at point {} of {}", driver.progress_idx, n)
    };
    let finished = cars.iter().filter(|c| c.driver.progress_idx >= n - 1 - crate::race::STAGE_RUNOFF).count();
    (total_time, position, finished, longest_off)
}

#[test]
fn stage_races_finish_in_three_to_five_minutes() {
    let _ = network();
    for &name in crate::stages::names() {
        let (time, position, finished, longest_off) = stage_race(name, 1);
        println!("{name:14} finished {} in {}:{:05.2}, {finished} rivals home, longest off the road {longest_off:.1}s",
            crate::hud::ordinal(position), (time / 60.0) as u32, time % 60.0);
        assert!((180.0..300.0).contains(&time), "{name}: {time:.0}s");
        assert!(longest_off < 30.0, "{name}: a car spent {longest_off:.0}s off the road in one go");
    }
}

/// Drive flat out in a straight line for `seconds` from `start` heading
/// `dir`, through whatever trees stand there (as the game does for the
/// player). Returns (distance covered, trees brushed).
fn plough(start: Vec2, dir: Vec2, seconds: f32, trees: &[[f32; 2]]) -> (f32, usize) {
    let yaw = (-dir.x).atan2(-dir.y);
    let mut car = CarPhysics::new_on(&WorldGround, Vec3::new(start.x, 0.0, start.y), yaw);
    car.vel = car.heading() * 50.0;
    let dt = 1.0 / 60.0;
    let mut hit = std::collections::HashSet::new();
    let mut t = 0.0;
    while t < seconds {
        // Hold the line: steer back toward the heading we set off on
        let fwd = Vec2::new(car.heading().x, car.heading().z);
        let steer = (-fwd.perp_dot(dir) * 3.0).clamp(-1.0, 1.0);
        car.update(&WorldGround, dt, CarInput { throttle: 1.0, steer, ..Default::default() });
        if let Some((i, _)) = collisions::car_vs_trees(&mut car, trees, dt) {
            hit.insert(i);
        }
        t += dt;
    }
    (Vec2::new(car.pos.x, car.pos.z).distance(start), hit.len())
}

#[test]
fn driving_through_the_deep_forest_slows_you_but_doesnt_stop_you() {
    let _ = network();
    // Real trees, in the thick of the north-western forest
    let (lo, hi) = (Vec2::new(-5600.0, 4700.0), Vec2::new(-4600.0, 5100.0));
    let trees = crate::trees::generate_trees(lo.x - 200.0, lo.y - 200.0, hi.x + 200.0, hi.y + 200.0).trunks;
    let mut worst = f32::MAX;
    let mut total_hits = 0;
    for k in 0..5 {
        let start = Vec2::new(lo.x, lo.y + 60.0 + k as f32 * 70.0);
        let (wooded, hits) = plough(start, Vec2::X, 8.0, &trees);
        let (open, _) = plough(start, Vec2::X, 8.0, &[]);
        println!("line {k}: {hits} trees brushed, {wooded:.0} units against {open:.0} with no trees ({:.0}%)", wooded / open * 100.0);
        worst = worst.min(wooded / open);
        total_hits += hits;
    }
    assert!(total_hits >= 5, "the test lines missed the forest ({total_hits} trees)");
    assert!(worst > 0.6, "the trees held a car to {:.0}% of the distance it covers on open ground", worst * 100.0);
}

#[test]
fn fords_can_be_driven_across() {
    let _ = network();
    let fords = crate::waterways::fords();
    assert!(fords.len() > 40, "only {} fords", fords.len());
    // A spread of them, up and down the rivers
    for &(ford, across) in fords.iter().step_by(fords.len() / 8) {
        let from = ford - across * 90.0;
        let mut car = CarPhysics::new(Vec3::ZERO, 0.0);
        car.place(&WorldGround, from.x, from.y, (-across.x).atan2(-across.y));
        let mut worst_wet: f32 = 0.0;
        let mut t = 0.0;
        let dt = 1.0 / 120.0;
        while t < 20.0 && (Vec2::new(car.pos.x, car.pos.z) - ford).dot(across) < 90.0 {
            // Steer back onto the line straight across
            let off = (Vec2::new(car.pos.x, car.pos.z) - ford).dot(across.perp());
            let input = CarInput { throttle: 0.6, steer: (off * 0.05).clamp(-0.5, 0.5), ..Default::default() };
            car.update(&WorldGround, dt, input);
            worst_wet = worst_wet.max(car.submerged);
            t += dt;
        }
        let reached = (Vec2::new(car.pos.x, car.pos.z) - ford).dot(across);
        println!("ford at {ford}: across in {t:.1}s, wettest {worst_wet:.2}");
        assert!(reached >= 90.0, "stuck in the ford at {ford} ({reached:.0} across after {t:.0}s)");
        assert!(worst_wet < 0.6, "nearly sank crossing the ford at {ford} ({worst_wet:.2})");
    }
}

/// What can be driven: on a grid over the island, the land cells a car can
/// get to from the home circuit and back again.
struct Reach {
    step: f32,
    side: usize,
    land: Vec<bool>,
    /// There and back.
    drivable: Vec<bool>,
}

/// `wet`: as after rain, when every surface grips less.
fn reach(step: f32, wet: bool) -> Reach {
    use std::collections::VecDeque;
    let net = network();
    let half = terrain::TERRAIN_HALF_SIZE;
    let side = (half * 2.0 / step) as usize;
    let pos = |i: usize| Vec2::new((i % side) as f32 + 0.5, (i / side) as f32 + 0.5) * step - half;
    // Height, grip and how much road, cell by cell (a row at a time in parallel)
    let rows = crate::layout::parallel_map((0..side).collect(), |row| {
        (0..side).map(|col| {
            let p = pos(row * side + col);
            let h = terrain::solid_height(p.x, p.y);
            let rain = if wet { crate::weather::WET_GRIP } else { 1.0 };
            (h, terrain::wet_grip(p.x, p.y, rain), road::on_road(p.x, p.y))
        }).collect::<Vec<_>>()
    });
    let cells: Vec<(f32, f32, f32)> = rows.into_iter().flatten().collect();
    let land: Vec<bool> = cells.iter().map(|c| c.0 > terrain::WATER_LEVEL - 2.0).collect();
    // Can a car go from a to b (neighbours)? Up only as steep as it
    // climbs; down no steeper than 1 in 1
    let can = |a: usize, b: usize| {
        let ((ha, ga, ra), (hb, gb, rb)) = (cells[a], cells[b]);
        let grade = (hb - ha) / step;
        land[a] && land[b] && grade >= -1.0 && grade <= crate::physics::steepest_climb(ga.min(gb), ra.max(rb)) * 0.95
    };
    // Paths are narrower than a cell, so the grid can miss them: each also
    // links the cells it runs through (they're graded to be driven, both ways)
    let cell_of = |p: Vec2| {
        let (x, z) = (((p.x + half) / step) as usize, ((p.y + half) / step) as usize);
        (x < side && z < side).then(|| z * side + x)
    };
    let mut along_paths: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for path in net.paths.iter().filter(|p| p.kind != road::PathKind::Rail) {
        let cells: Vec<usize> = path.points.iter().filter_map(|&p| cell_of(p)).collect();
        for w in cells.windows(2).filter(|w| w[0] != w[1]) {
            along_paths.entry(w[0]).or_default().push(w[1]);
            along_paths.entry(w[1]).or_default().push(w[0]);
        }
    }
    let neighbours = |i: usize| {
        let (x, z) = (i % side, i / side);
        let grid = [(x > 0).then(|| i - 1), (x + 1 < side).then(|| i + 1), (z > 0).then(|| i - side), (z + 1 < side).then(|| i + side)];
        grid.into_iter().flatten().map(move |j| (j, false))
            .chain(along_paths.get(&i).into_iter().flatten().map(|&j| (j, true)))
    };
    let start = net.paths.iter().find(|p| p.circuit()).unwrap().points[0];
    let s = ((start.y + half) / step) as usize * side + ((start.x + half) / step) as usize;
    let flood = |forward: bool| {
        let mut seen = vec![false; side * side];
        let mut queue = VecDeque::from([s]);
        seen[s] = true;
        while let Some(i) = queue.pop_front() {
            for (j, path) in neighbours(i) {
                let ok = path || if forward { can(i, j) } else { can(j, i) };
                if !seen[j] && ok {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
        seen
    };
    let (there, back) = (flood(true), flood(false));
    let drivable = there.iter().zip(&back).map(|(a, b)| *a && *b).collect();
    Reach { step, side, land, drivable }
}

/// Dev aid: how much of the island can be driven, and the biggest places
/// that can't, to look at with STUMBLER_AT.
#[test]
#[ignore]
fn reach_report() {
    let wet = std::env::var_os("STUMBLER_WET").is_some();
    let r = reach(60.0, wet);
    let land = r.land.iter().filter(|&&l| l).count();
    let drivable = r.drivable.iter().filter(|&&d| d).count();
    println!("drivable: {:.1}% of {:.0} km² of land", drivable as f32 / land as f32 * 100.0, land as f32 * (r.step * 0.5 / 1000.0).powi(2));
    // Connected patches of land that can't be driven, biggest first
    let half = terrain::TERRAIN_HALF_SIZE;
    let mut patch = vec![usize::MAX; r.side * r.side];
    let mut patches: Vec<(usize, Vec2)> = Vec::new();
    for i in 0..r.side * r.side {
        if !r.land[i] || r.drivable[i] || patch[i] != usize::MAX {
            continue;
        }
        let id = patches.len();
        let (mut count, mut sum) = (0, Vec2::ZERO);
        let mut stack = vec![i];
        patch[i] = id;
        while let Some(k) = stack.pop() {
            count += 1;
            sum += Vec2::new((k % r.side) as f32, (k / r.side) as f32) * r.step - half;
            let (x, z) = (k % r.side, k / r.side);
            for n in [(x > 0).then(|| k - 1), (x + 1 < r.side).then(|| k + 1), (z > 0).then(|| k - r.side), (z + 1 < r.side).then(|| k + r.side)].into_iter().flatten() {
                if r.land[n] && !r.drivable[n] && patch[n] == usize::MAX {
                    patch[n] = id;
                    stack.push(n);
                }
            }
        }
        patches.push((count, sum / count as f32));
    }
    patches.sort_by(|a, b| b.0.cmp(&a.0));
    for (count, centre) in patches.iter().take(15) {
        let p = *centre;
        println!("  {:6.2} km² round {:.0},{:.0}: height {:.0}, mountain {:.1}, alpine {:.1}, desert {:.1}, tundra {:.1}",
            *count as f32 * (r.step * 0.5 / 1000.0).powi(2), p.x, p.y, terrain::raw_height(p.x, p.y),
            terrain::biome_mountain(p.x, p.y), terrain::alpine(p.x, p.y), terrain::biome_desert(p.x, p.y), terrain::biome_tundra(p.x, p.y));
    }
}

#[test]
fn nearly_all_the_island_can_be_driven() {
    // All but the sheerest alpine crags, there and back from the home
    // circuit, after rain as much as in the dry
    for (wet, least) in [(false, 0.97), (true, 0.97)] {
        let r = reach(60.0, wet);
        let land = r.land.iter().filter(|&&l| l).count();
        let drivable = r.drivable.iter().filter(|&&d| d).count() as f32 / land as f32;
        assert!(drivable > least, "only {:.1}% of the land can be driven (wet: {wet})", drivable * 100.0);
    }
}

#[test]
fn trails_can_be_climbed_even_in_the_wet() {
    let net = network();
    let mut steepest: f32 = 0.0;
    for trail in net.paths.iter().filter(|p| p.kind == road::PathKind::Trail) {
        for i in 1..trail.points.len() {
            let p = trail.points[i];
            let grade = (trail.heights[i] - trail.heights[i - 1]).abs() / road::SAMPLE_SPACING;
            let grip = terrain::wet_grip(p.x, p.y, crate::weather::WET_GRIP);
            let limit = crate::physics::steepest_climb(grip, road::on_road(p.x, p.y));
            steepest = steepest.max(grade / limit);
            assert!(grade < limit, "trail at {p} is {:.0}% steep, more than the {:.0}% a car climbs there in the wet)", grade * 100.0, limit * 100.0);
        }
    }
    println!("steepest trail stretch: {:.0}% of what the car can climb there in the wet", steepest * 100.0);
}

/// Places worth driving to: landmarks and secrets, farms, the circuits, and
/// the rivers and canyons.
fn points_of_interest() -> Vec<Vec2> {
    let net = network();
    let mut points: Vec<Vec2> = crate::landmarks::all().iter().map(|l| l.pos).collect();
    points.extend(crate::farms::all().iter().map(|f| f.yard));
    for c in net.paths.iter().filter(|p| p.circuit()) {
        points.extend(c.points.iter().step_by(50));
    }
    for course in crate::waterways::all() {
        points.extend(course.on_land().iter().step_by(10));
    }
    points
}

/// How far from the nearest point of interest each spot of open land on a
/// grid is: (spot, distance), skipping the sea.
fn interest_gaps(step: f32) -> Vec<(Vec2, f32)> {
    let points = points_of_interest();
    let half = terrain::TERRAIN_HALF_SIZE;
    let n = (half * 2.0 / step) as usize;
    let spots: Vec<Vec2> = (0..n * n)
        .map(|i| Vec2::new((i % n) as f32 + 0.5, (i / n) as f32 + 0.5) * step - half)
        .filter(|p| terrain::raw_height(p.x, p.y) > terrain::WATER_LEVEL)
        .collect();
    crate::layout::parallel_map(spots.chunks(256).map(|c| c.to_vec()).collect(), |chunk| {
        chunk.into_iter().map(|p| (p, points.iter().map(|q| q.distance(p)).fold(f32::MAX, f32::min))).collect::<Vec<_>>()
    }).into_iter().flatten().collect()
}

/// Dev aid: how far the land is from anything to find, and the emptiest spots.
#[test]
#[ignore]
fn interest_report() {
    let mut gaps = interest_gaps(400.0);
    gaps.sort_by(|a, b| b.1.total_cmp(&a.1));
    let at = |share: f32| gaps[((gaps.len() as f32 * share) as usize).min(gaps.len() - 1)].1;
    println!("from the nearest place of interest: worst {:.0}, 1% {:.0}, 10% {:.0}, median {:.0} (units; 2 per metre)",
        gaps[0].1, at(0.01), at(0.1), at(0.5));
    let mut shown: Vec<Vec2> = Vec::new();
    for &(p, d) in &gaps {
        if shown.len() == 12 || d < 2500.0 {
            break;
        }
        if shown.iter().all(|s| s.distance(p) > 4000.0) {
            println!("  {:.0},{:.0}: {d:.0} away; height {:.0}, mountain {:.1}, alpine {:.1}, desert {:.1}, forest {:.1}, tundra {:.1}", p.x, p.y,
                terrain::raw_height(p.x, p.y), terrain::biome_mountain(p.x, p.y), terrain::alpine(p.x, p.y), terrain::biome_desert(p.x, p.y), terrain::biome_forest(p.x, p.y), terrain::biome_tundra(p.x, p.y));
            shown.push(p);
        }
    }
}

#[test]
fn there_is_always_something_to_find_nearby() {
    let mut gaps: Vec<f32> = interest_gaps(400.0).into_iter().map(|g| g.1).collect();
    gaps.sort_by(|a, b| b.total_cmp(a));
    // Never much over a couple of kilometres (two units a metre) from a
    // landmark, a circuit, a river or a canyon; most land much nearer
    let tenth = gaps[gaps.len() / 10];
    assert!(gaps[0] < 5500.0, "land {:.0} from anything to find", gaps[0]);
    assert!(tenth < 3000.0, "a tenth of the land is over {tenth:.0} from anything to find");
}
