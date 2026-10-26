//! Rendering: the 3D frame, the HUD (race or free roam), minimap and world map.

use glam::{Mat4, Quat, Vec2, Vec3, vec3};

use crate::geometry::Vertex;
use crate::gpu::{self, Draw, PipelineKind};
use crate::hud::{self, Align, HudBuilder, MapView};
use crate::physics::{CarPhysics, Ground, WorldGround};
use crate::race::{self, Phase};
use crate::terrain::{solid_height, TERRAIN_HALF_SIZE};
use crate::{birds, collectibles, landmarks, lights, railway, spectators, Stage, KMH_PER_UNIT};

/// Cars beyond this aren't drawn.
const CAR_DRAW_DISTANCE: f32 = 900.0;
/// Dirt trails on the minimap and world map.
const TRAIL_COLOR: [f32; 4] = [0.62, 0.46, 0.3, 0.85];
/// The train is drawn this far off (it's big, and a fine sight across a valley).
const TRAIN_DRAW_DISTANCE: f32 = 3000.0;
/// Level crossing barriers are drawn this far off.
const CROSSING_DRAW_DISTANCE: f32 = 1500.0;
/// The railway on the maps.
const RAIL_COLOR: [f32; 4] = [0.6, 0.56, 0.66, 0.9];
/// The train on the maps.
const TRAIN_COLOR: [f32; 4] = [0.95, 0.75, 0.55, 1.0];
/// Minimap side length, in HUD pixel units.
const MINIMAP_SIZE: f32 = 70.0;
/// World units from the centre to the edge of the minimap.
const MINIMAP_RANGE: f32 = 1400.0;

/// The train's cars and the crossing barriers, built once.
pub(crate) struct TrainMeshes {
    loco: gpu::Mesh,
    loco_glow: gpu::Mesh,
    tender: gpu::Mesh,
    coach: gpu::Mesh,
    coach_glow: gpu::Mesh,
    barrier: gpu::Mesh,
    lamp: gpu::Mesh,
}

impl TrainMeshes {
    pub(crate) fn new(g: &gpu::Gpu) -> Self {
        let m = railway::build_meshes();
        let mesh = |(v, i): &(Vec<Vertex>, Vec<u32>)| gpu::Mesh::new(g, v, i);
        Self {
            loco: mesh(&m.loco),
            loco_glow: mesh(&m.loco_glow),
            tender: mesh(&m.tender),
            coach: mesh(&m.coach),
            coach_glow: mesh(&m.coach_glow),
            barrier: mesh(&m.barrier),
            lamp: mesh(&m.lamp),
        }
    }
}

impl Stage {
    pub(crate) fn draw(&mut self) {
        let time = self.time();
        let view_proj = self.camera.view_proj(self.gpu.aspect(), self.player.speed, time);
        let sun_dir = self.sun_dir();
        let night = 1.0 - self.day_factor();

        // Two slots per struck tree: where it stands and how far it's bent, then which way
        let mut sway = [[0.0_f32; 4]; 16];
        for (i, s) in self.sway_events.iter().take(8).enumerate() {
            let bend = s.strength * (s.time * 12.0).sin();
            sway[i * 2] = [s.x, s.z, s.base_y, bend];
            sway[i * 2 + 1] = [s.dir.x, s.dir.y, 0.0, 0.0];
        }

        // Headlights at night: the player's two, plus the nearest AI cars
        let mut frame_lights: Vec<lights::Light> = Vec::new();
        if night > 0.1 {
            let side = self.player.rot * Vec3::X;
            for offset in [-1.4, 1.4] {
                frame_lights.push(headlight(&self.player, side * offset, 160.0, night * 2.0));
            }
            let mut nearby: Vec<&CarPhysics> = self.cars.iter()
                .map(|c| &c.physics)
                .filter(|c| c.pos.distance(self.player.pos) < 300.0)
                .collect();
            nearby.sort_by(|a, b| a.pos.distance_squared(self.player.pos).total_cmp(&b.pos.distance_squared(self.player.pos)));
            for car in nearby.into_iter().take(lights::MAX_LIGHTS - 2) {
                frame_lights.push(headlight(car, Vec3::ZERO, 110.0, night * 1.5));
            }
        }
        // The locomotive's lamp lights the line ahead after dark
        let train_poses = match (&self.train, railway::get()) {
            (Some(train), Some(rail)) => train.poses(rail),
            _ => Vec::new(),
        };
        if let Some(loco) = train_poses.first() {
            if night > 0.1 && loco.centre.distance(self.camera.pos) < 800.0 && frame_lights.len() < lights::MAX_LIGHTS {
                let lamp = loco.model.transform_point3(vec3(0.0, 8.8, -11.6));
                let dir = (loco.forward - Vec3::Y * 0.15).normalize();
                frame_lights.push(lights::Light::spot(lamp, dir, 220.0, 22.0, vec3(1.0, 0.9, 0.7), night * 2.0));
            }
        }
        let light_uniforms = lights::LightUniforms::from_lights(&frame_lights);

        // Project the sun into screen space for the sky shader
        let sun_clip = view_proj * (self.camera.pos + sun_dir * 5000.0).extend(1.0);
        let sun_screen = if sun_clip.w > 0.0 {
            vec3(sun_clip.x / sun_clip.w, sun_clip.y / sun_clip.w, sun_dir.y)
        } else {
            vec3(0.0, -2.0, sun_dir.y) // behind camera
        };

        let (width, height) = self.gpu.size();
        // Rain greys the horizon over and closes the distance in
        let rain = self.weather.rain;
        let clear = horizon_color(sun_dir.y);
        let overcast = clear.dot(Vec3::splat(1.0 / 3.0)) * vec3(0.92, 0.95, 1.0) * 0.85;
        let fog_color = clear.lerp(overcast, rain);
        let fog_density = (1.0 + 1.2 * rain) / 1500.0;
        let globals = gpu::Globals {
            view_proj,
            inv_view_proj: view_proj.inverse(),
            sun_dir: sun_dir.into(),
            wind_time: self.wind_time,
            cam_pos: self.camera.pos.into(),
            light_count: light_uniforms.count,
            sun_screen: sun_screen.into(),
            sway_count: self.sway_events.len().min(8) as u32,
            sway,
            lights: light_uniforms.data,
            fog: [fog_color.x, fog_color.y, fog_color.z, fog_density],
            screen: [width, height, rain, self.weather.wet],
        };

        // --- Per-frame meshes ---
        let (v, i) = collectibles::build_collectible_mesh(&self.collectibles, self.camera.pos, time);
        self.collectible_mesh.update(&self.gpu, &v, &i);
        let (v, i) = spectators::build_spectator_mesh(&self.spectators, self.camera.pos, CAR_DRAW_DISTANCE, time);
        self.spectator_mesh.update(&self.gpu, &v, &i);
        let (v, i) = birds::build_bird_mesh(&self.flocks, time);
        self.bird_mesh.update(&self.gpu, &v, &i);
        let (v, i) = self.wildlife.build_mesh(self.camera.pos);
        self.wildlife_mesh.update(&self.gpu, &v, &i);
        // Skid marks fade, so rebuild them now and then even with no new ones
        if self.skids.changed || time - self.skid_mesh_time > 0.5 {
            let (v, i) = self.skids.mesh(self.camera.pos, 900.0, time);
            self.skid_mesh.update(&self.gpu, &v, &i);
            self.skids.changed = false;
            self.skid_mesh_time = time;
        }
        let (v, i) = self.speed_lines.mesh(self.camera.pos, self.player.vel);
        self.speed_line_mesh.update(&self.gpu, &v, &i);
        let (v, i) = light_pools(self.camera.pos, night);
        self.light_pool_mesh.update(&self.gpu, &v, &i);
        let (right, up) = self.camera.billboard_axes();
        let (v, i) = self.particles.build_mesh(right, up);
        self.particle_mesh.update(&self.gpu, &v, &i);
        // Dev aid: STUMBLER_NO_HUD=1 for clean screenshots
        let hud = if std::env::var_os("STUMBLER_NO_HUD").is_some() { HudBuilder::new() } else { self.build_hud(width, height) };
        self.hud_mesh.update(&self.gpu, &hud.verts, &hud.idxs);

        // --- Draw list (order matters for the blended pipelines) ---
        let identity = Mat4::IDENTITY;
        let chunk_draws = self.chunks.draws(self.camera.pos, view_proj);
        let mut draws: Vec<Draw> = chunk_draws.terrain.iter()
            .map(|&mesh| Draw::new(PipelineKind::Terrain, mesh, identity))
            .collect();
        draws.extend(chunk_draws.opaque.iter().map(|&mesh| Draw::mesh(mesh, identity)));
        draws.extend(chunk_draws.foliage.iter().map(|&mesh| Draw::foliage(mesh)));
        draws.extend(chunk_draws.cover.iter().map(|&mesh| Draw::cover(mesh)));
        draws.extend(chunk_draws.glow.iter().map(|&mesh| Draw::glow(mesh)));
        draws.extend([
            Draw::mesh(&self.world.gate_mesh, identity),
            Draw::mesh(&self.world.boost_mesh, identity),
            Draw::mesh(self.collectible_mesh.mesh(), identity),
            Draw::mesh(self.spectator_mesh.mesh(), identity),
            Draw::mesh(self.bird_mesh.mesh(), identity),
            Draw::mesh(self.wildlife_mesh.mesh(), identity),
        ]);

        let all_cars = std::iter::once((&self.player, &self.body_mesh, self.braking))
            .chain(self.cars.iter().map(|c| (&c.physics, &self.ai_body_meshes[c.color_idx % self.ai_body_meshes.len()], c.last_input.brake > 0.1 && c.physics.speed > 1.0)));
        for (car, body, braking) in all_cars {
            if car.pos.distance_squared(self.camera.pos) > CAR_DRAW_DISTANCE * CAR_DRAW_DISTANCE {
                continue;
            }
            draws.push(Draw::mesh(&self.shadow_mesh, shadow_transform(car)));
            draws.push(Draw::mesh(body, car.body_transform()));
            if braking {
                draws.push(Draw { kind: gpu::MeshKind::Particle, ..Draw::mesh(&self.brake_light_mesh, car.body_transform()) });
            }
            for wheel in 0..4 {
                draws.push(Draw::mesh(&self.wheel_mesh, car.wheel_transform(wheel)));
            }
        }

        // The train, lit windows after dark, and the crossing barriers
        if train_poses.first().is_some_and(|p| p.centre.distance(self.camera.pos) < TRAIN_DRAW_DISTANCE) {
            let m = &self.train_meshes;
            for pose in &train_poses {
                let (body, glow) = match pose.kind {
                    railway::CarKind::Locomotive => (&m.loco, Some(&m.loco_glow)),
                    railway::CarKind::Tender => (&m.tender, None),
                    railway::CarKind::Coach => (&m.coach, (night > 0.25).then_some(&m.coach_glow)),
                };
                draws.push(Draw::mesh(body, pose.model));
                if let Some(glow) = glow {
                    draws.push(Draw { model: pose.model, ..Draw::glow(glow) });
                }
            }
        }
        if let (Some(train), Some(rail)) = (&self.train, railway::get()) {
            let m = &self.train_meshes;
            let flash = (time * 1.6) as usize % 2;
            for (c, crossing) in rail.crossings.iter().enumerate() {
                if crossing.pos.distance(self.camera.pos) > CROSSING_DRAW_DISTANCE {
                    continue;
                }
                let warning = train.warning(rail, c);
                for (pivot, reach, face) in crossing.barriers() {
                    draws.push(Draw::mesh(&m.barrier, railway::barrier_transform(pivot, reach, train.barriers[c])));
                    if warning {
                        let lamp = railway::lamp_positions(pivot, reach, face)[flash];
                        draws.push(Draw { model: Mat4::from_translation(lamp), ..Draw::glow(&m.lamp) });
                    }
                }
            }
        }

        // Floodlit pools on the circuits after dark
        draws.push(Draw { kind: gpu::MeshKind::Particle, ..Draw::new(PipelineKind::Particles, self.light_pool_mesh.mesh(), identity) });

        // Skid marks lie see-through on the road and ground
        draws.push(Draw::new(PipelineKind::Particles, self.skid_mesh.mesh(), identity));

        // Water is translucent: after the opaque world so things show through it
        draws.extend(chunk_draws.water.iter().map(|&mesh| Draw::water(mesh)));
        // The start lights: on one by one through the countdown, then all
        // green as the race gets away
        if let Some(race) = &self.race {
            let lit = match race.phase {
                Phase::Countdown { remaining } => Some((((race::COUNTDOWN - remaining) / race::COUNTDOWN * 5.0).ceil().clamp(1.0, 5.0) as usize, 0)),
                Phase::Racing if race.clock < 1.5 => Some((5, 1)),
                _ => None,
            };
            if let Some((count, colour)) = lit {
                let venue = &self.world.circuits[race.circuit];
                let toward_grid = -venue.start_dir();
                // Filling left to right as the grid sees them
                for lamp in crate::world::start_lamps(venue).iter().rev().take(count) {
                    let at = *lamp + vec3(toward_grid.x, 0.0, toward_grid.y) * 1.3;
                    draws.push(Draw { kind: gpu::MeshKind::Particle, ..Draw::mesh(&self.start_light_meshes[colour], Mat4::from_translation(at)) });
                }
            }
        }

        // In a time trial, your best lap here as a ghost to chase
        if let Some(race) = &self.race {
            let ghost = self.ghosts.get(&race.circuit).filter(|_| race.trial && race.phase == Phase::Racing);
            if let Some((pos, rot)) = ghost.and_then(|(_, frames)| race::ghost_pose(frames, race.lap_time())) {
                draws.push(Draw::new(PipelineKind::Particles, &self.ghost_mesh, Mat4::from_rotation_translation(rot, pos)));
            }
        }
        draws.push(Draw { kind: gpu::MeshKind::Particle, ..Draw::new(PipelineKind::Particles, self.particle_mesh.mesh(), identity) });
        draws.push(Draw { kind: gpu::MeshKind::Particle, ..Draw::new(PipelineKind::Particles, self.speed_line_mesh.mesh(), identity) });
        draws.push(Draw { kind: gpu::MeshKind::Particle, ..Draw::new(PipelineKind::Particles, self.rain_mesh.mesh(), identity) });
        draws.push(Draw::new(PipelineKind::Hud, self.hud_mesh.mesh(), identity));

        self.gpu.render(&globals, &draws);
    }

    fn build_hud(&self, width: f32, height: f32) -> HudBuilder {
        let mut h = HudBuilder::new();
        let px = (height / 720.0 * 3.0).max(2.0);
        let margin = px * 6.0;
        let line = HudBuilder::text_height(px) + px * 3.0;

        if self.show_map {
            self.world_map(&mut h, width, height, px);
            return h;
        }

        match &self.race {
            Some(race) => self.race_hud(&mut h, race, width, height, px),
            None => self.roam_hud(&mut h, width, height, px),
        }

        // Top right corner: frame rate
        h.text(width - margin * 0.5, margin * 0.4, px * 0.8, Align::Right, &format!("{:.0} FPS", self.fps), hud::DIM);

        // Bottom right: speedometer
        let kmh = (self.player.speed.abs() * KMH_PER_UNIT).round() as i32;
        let big = HudBuilder::text_height(px * 4.0);
        let speed_y = height - margin - big - line;
        h.text(width - margin, speed_y, px * 4.0, Align::Right, &kmh.to_string(), hud::WHITE);
        h.text(width - margin, speed_y + big + px * 3.0, px, Align::Right, "KM/H", hud::DIM);
        if self.boost.timer > 0.0 {
            h.text(width - margin, speed_y - line * 1.5, px * 1.5, Align::Right, "BOOST!", [0.3, 0.9, 1.0, 1.0]);
        } else if self.player.draft > 0.3 {
            h.text(width - margin, speed_y - line * 1.2, px, Align::Right, "SLIPSTREAM", [0.75, 0.9, 1.0, 1.0]);
        }
        // Drift meter: fills while the slide is held, gold once it'll pay out
        if self.boost.drift_charge > 0.0 {
            let w = px * 60.0;
            let x = width * 0.5 - w * 0.5;
            let y = height - margin - line * 2.5;
            let full = self.boost.drift_charge / crate::boosts::DRIFT_CHARGE_MAX;
            let ready = self.boost.drift_charge >= crate::boosts::DRIFT_MIN;
            let color = if ready { hud::GOLD } else { [0.9, 0.55, 0.3, 1.0] };
            h.text(width * 0.5, y - line * 1.4, px * 1.2, Align::Center, "DRIFT", color);
            h.rect(x, y, w, px * 2.5, hud::PANEL);
            h.rect(x, y, w * full, px * 2.5, color);
        }

        if let Some((text, color, _)) = &self.banner {
            if self.race.as_ref().and_then(|r| r.countdown_text()).is_none() {
                // First line large, any further lines smaller and dimmer
                // Above the results panel once a race is over
                let finished = self.race.as_ref().is_some_and(|r| matches!(r.phase, Phase::Finished { .. }));
                let mut y = height * if finished { 0.1 } else { 0.22 };
                for (k, part) in text.lines().enumerate() {
                    let (size, c) = if k == 0 { (px * 1.5, *color) } else { (px, hud::WHITE) };
                    h.text(width * 0.5, y, size, Align::Center, part, c);
                    y += HudBuilder::text_height(size) + px * 4.0;
                }
            }
        }

        self.minimap(&mut h, height, px);
        h
    }

    fn race_hud(&self, h: &mut HudBuilder, race: &race::Race, width: f32, height: f32, px: f32) {
        let margin = px * 6.0;
        let line = HudBuilder::text_height(px) + px * 3.0;
        let name = self.circuit_name(race.circuit);

        // Top left: lap and times
        let title = match self.tour.as_ref().filter(|t| t.next_circuit() == Some(race.circuit)) {
            Some(t) => format!("TOUR {}/{} - {name}", t.round + 1, t.circuits.len()),
            None => name.to_string(),
        };
        let panel_w = (px * 78.0).max(HudBuilder::text_width(&title, px) + margin);
        h.rect(margin * 0.5, margin * 0.5, panel_w, line * 5.5 + px * 4.0, hud::PANEL);
        let x = margin;
        let mut y = margin;
        h.text(x, y, px, Align::Left, &title, hud::DIM);
        y += line;
        let record = self.progress.best_laps.get(name).copied();
        let best = race.best_lap.or(record).map_or("-:--.--".into(), hud::format_time);
        if race.stage {
            // A stage: how far there is to go, and the time against the record
            let to_go = race.to_go(&self.ai_tracks[race.circuit]);
            // (a unit is half a metre)
            h.text(x, y, px * 1.5, Align::Left, &format!("{:.1} KM TO GO", to_go * 0.5 / 1000.0), hud::WHITE);
            y += line * 1.5;
            h.text(x, y, px, Align::Left, &format!("TIME {}", hud::format_time(race.clock)), hud::WHITE);
            y += line * 2.0;
            h.text(x, y, px, Align::Left, &format!("RECORD {best}"), hud::YELLOW);
        } else {
            let lap = if race.trial { format!("LAP {}", race.display_lap()) } else { format!("LAP {}/{}", race.display_lap(), race::RACE_LAPS) };
            h.text(x, y, px * 1.5, Align::Left, &lap, hud::WHITE);
            y += line * 1.5;
            h.text(x, y, px, Align::Left, &format!("TIME {}", hud::format_time(race.lap_time())), hud::WHITE);
            y += line;
            let last = race.last_lap.map_or("-:--.--".into(), hud::format_time);
            h.text(x, y, px, Align::Left, &format!("LAST {last}"), hud::DIM);
            y += line;
            h.text(x, y, px, Align::Left, &format!("BEST {best}"), hud::YELLOW);
        }

        // Top right, under the frame rate: position in a race; in a time
        // trial, how each sector compares with the best lap
        if race.trial {
            h.text(width - margin, margin * 0.5 + line, px * 1.2, Align::Right, "TIME TRIAL", hud::DIM);
            if let Some(delta) = race.recent_split() {
                let (text, color) = if delta <= 0.0 { (format!("-{:.2}", -delta), hud::GREEN) } else { (format!("+{:.2}", delta), hud::RED) };
                h.text(width * 0.5, height * 0.14, px * 1.8, Align::Center, &text, color);
            }
        } else {
            let pos_text = format!("{}/{}", race.position, race.racer_count());
            let pos_w = HudBuilder::text_width(&pos_text, px * 2.5);
            let top = margin * 0.5 + line;
            h.rect(width - margin * 1.5 - pos_w, top, pos_w + margin, line * 3.4, hud::PANEL);
            h.text(width - margin, top + margin * 0.5, px, Align::Right, "POS", hud::DIM);
            h.text(width - margin, top + margin * 0.5 + line, px * 2.5, Align::Right, &pos_text, hud::WHITE);
        }

        let cx = width * 0.5;
        if let Some(text) = race.countdown_text() {
            let color = if text == "GO!" { hud::GREEN } else { hud::YELLOW };
            h.text(cx, height * 0.3, px * 6.0, Align::Center, text, color);
            if race.holding() {
                h.text(cx, height * 0.3 + HudBuilder::text_height(px * 6.0) + px * 4.0, px, Align::Center, "HIT THE THROTTLE AS IT GOES GREEN FOR A FLYING START", hud::DIM);
            }
        }
        if race.is_wrong_way() {
            h.text(cx, height * 0.4, px * 2.5, Align::Center, "WRONG WAY", hud::RED);
        }
        if race.is_off_track() {
            let left = (crate::ABANDON_TIME - self.abandon_timer).max(0.0).ceil();
            let what = if race.stage { "STAGE" } else { "CIRCUIT" };
            h.text(cx, height * 0.65, px * 1.5, Align::Center, &format!("RETURN TO THE {what} {left}"), hud::RED);
        }
        if let Phase::Finished { position, total_time } = race.phase {
            let longest = self.tour.as_ref().and_then(|t| t.next_circuit())
                .map_or(0.0, |next| HudBuilder::text_width(&format!("BACKSPACE: ON TO {}", self.world.circuits[next].name), px));
            let pw = (px * 130.0).max(longest + px * 12.0);
            let ph = line * 7.5;
            let top = height * 0.3;
            h.rect(cx - pw * 0.5, top, pw, ph, hud::PANEL);
            let color = if position == 1 { hud::YELLOW } else { hud::WHITE };
            h.text(cx, top + line * 0.6, px * 2.5, Align::Center, &format!("FINISHED {}", hud::ordinal(position)), color);
            h.text(cx, top + line * 2.7, px, Align::Center, &format!("TOTAL {}", hud::format_time(total_time)), hud::WHITE);
            let best_label = if race.stage { "STAGE RECORD" } else { "BEST LAP" };
            h.text(cx, top + line * 3.7, px, Align::Center, &format!("{best_label} {best}"), hud::YELLOW);
            // On a tour, the way on is the road to the next circuit
            match self.tour.as_ref().and_then(|t| t.next_circuit()) {
                Some(next) => {
                    let on = format!("BACKSPACE: ON TO {}", self.world.circuits[next].name);
                    h.text(cx, top + line * 5.2, px, Align::Center, &on, hud::WHITE);
                    h.text(cx, top + line * 6.2, px, Align::Center, "ENTER RACE HERE AGAIN", hud::DIM);
                }
                None => {
                    h.text(cx, top + line * 5.2, px, Align::Center, "ENTER RACE AGAIN", hud::DIM);
                    h.text(cx, top + line * 6.2, px, Align::Center, "BACKSPACE FREE ROAM", hud::DIM);
                }
            }
        } else if race.holding() {
            let leave = if race.trial { "BACKSPACE END TIME TRIAL" } else { "BACKSPACE ABANDON RACE" };
            h.text(margin, height - margin - line * 1.0 - px * 80.0, px, Align::Left, leave, hud::DIM);
        }
    }

    fn roam_hud(&self, h: &mut HudBuilder, width: f32, height: f32, px: f32) {
        let margin = px * 6.0;
        let line = HudBuilder::text_height(px) + px * 3.0;
        let p = self.player_xz();
        let total_orbs = self.collectibles.len();
        let orbs = self.collectibles.iter().filter(|c| c.collected).count();

        // Top left: exploration progress
        let found = self.progress.landmarks.len();
        let (secrets, places): (Vec<_>, Vec<_>) = landmarks::all().iter().partition(|l| l.kind.is_secret());
        let seen = |list: &[&landmarks::Landmark]| list.iter().filter(|l| self.progress.landmarks.contains(&l.name)).count();
        let venues = |stage: bool| self.world.circuits.iter().filter(|c| c.stage == stage).count();
        let found_venues = |stage: bool| self.world.circuits.iter().filter(|c| c.stage == stage && self.progress.circuits.contains(c.name)).count();
        let rows = [
            (format!("CIRCUITS {}/{}", found_venues(false), venues(false)), hud::WHITE),
            (format!("STAGES {}/{}", found_venues(true), venues(true)), hud::WHITE),
            (format!("LANDMARKS {}/{}", seen(&places), places.len()), hud::DIM),
            (format!("SECRETS {}/{}", seen(&secrets), secrets.len()), hud::GOLD),
            (format!("PAINTS {}/{}", crate::unlocked_paints(found), crate::car::CAR_COLORS.len()), hud::DIM),
            (format!("ORBS {orbs}/{total_orbs}"), hud::YELLOW),
        ];
        let panel_w = rows.iter().map(|(t, _)| HudBuilder::text_width(t, px)).fold(HudBuilder::text_width("FREE ROAM", px * 1.5), f32::max);
        h.rect(margin * 0.5, margin * 0.5, panel_w + margin, line * (rows.len() as f32 + 1.5) + px * 4.0, hud::PANEL);
        let x = margin;
        let mut y = margin;
        h.text(x, y, px * 1.5, Align::Left, "FREE ROAM", hud::WHITE);
        y += line * 1.5;
        for (text, color) in &rows {
            h.text(x, y, px, Align::Left, text, *color);
            y += line;
        }

        // Top centre: guide to the next round of a tour, or else to the
        // nearest circuit you haven't raced yet
        let tour_next = self.tour.as_ref().and_then(|t| t.next_circuit()).map(|ci| &self.world.circuits[ci]);
        let target = tour_next.map(|c| (c, c.start().distance(p))).filter(|(_, d)| *d > crate::RACE_PROMPT_DISTANCE).or_else(|| {
            if tour_next.is_some() {
                return None;
            }
            self.world.circuits.iter()
                .filter(|c| !self.progress.best_finish.contains_key(c.name))
                .chain(self.world.circuits.iter())
                .map(|c| (c, c.start().distance(p)))
                .filter(|(_, d)| *d > crate::RACE_PROMPT_DISTANCE)
                .min_by(|a, b| a.1.total_cmp(&b.1))
        });
        if let Some((c, dist)) = target {
            let to = (c.start() - p).normalize_or_zero();
            let heading = self.player.heading();
            let fwd = Vec2::new(heading.x, heading.z);
            // Screen arrow: up = straight ahead, right = car's right-hand side
            let right = Vec2::new(-fwd.y, fwd.x) * -1.0;
            let dir = Vec2::new(to.dot(right), -to.dot(fwd));
            let unknown = if c.stage { "UNKNOWN STAGE" } else { "UNKNOWN CIRCUIT" };
            let name = if self.progress.circuits.contains(c.name) || tour_next.is_some() { c.name } else { unknown };
            let label = match &self.tour {
                Some(t) => format!("TOUR {}/{}: {name} {:.1} KM", t.round + 1, t.circuits.len(), dist * 0.5 / 1000.0),
                None => format!("{name} {:.1} KM", dist * 0.5 / 1000.0),
            };
            let cx = width * 0.5;
            let w = HudBuilder::text_width(&label, px) + px * 16.0;
            h.rect(cx - w * 0.5, margin * 0.5, w, line * 1.6, hud::PANEL);
            h.arrow(Vec2::new(cx - w * 0.5 + px * 6.0, margin * 0.5 + line * 0.8), dir, px * 3.0, hud::YELLOW);
            h.text(cx + px * 4.0, margin * 0.5 + line * 0.35, px, Align::Center, &label, hud::WHITE);
        }

        // Centre: race offer at a start line
        if let Some(ci) = self.race_offer() {
            let name = self.circuit_name(ci);
            let cx = width * 0.5;
            let top = height * 0.3;
            let best = self.progress.best_laps.get(name).map_or("-:--.--".into(), |&t| hud::format_time(t));
            let finish = self.progress.best_finish.get(name).map_or("-".into(), |&p| hud::ordinal(p));
            let stage = self.world.circuits[ci].stage;
            let record = if stage { format!("STAGE RECORD {best}   BEST FINISH {finish}") } else { format!("BEST LAP {best}   BEST FINISH {finish}") };
            let difficulty = crate::DIFFICULTIES[self.progress.difficulty.min(crate::DIFFICULTIES.len() - 1)].0;
            let rivals = format!("RIVALS: {difficulty} - TAB    SETUP: {} - V", self.player.setup.name);
            let pw = HudBuilder::text_width(name, px * 2.0).max(HudBuilder::text_width(&record, px)).max(HudBuilder::text_width(&rivals, px)) + px * 16.0;
            h.rect(cx - pw * 0.5, top, pw, line * 6.2, hud::PANEL);
            h.text(cx, top + line * 0.5, px * 2.0, Align::Center, name, hud::YELLOW);
            h.text(cx, top + line * 2.0, px, Align::Center, &record, hud::DIM);
            let tour_here = self.tour.as_ref().filter(|t| t.next_circuit() == Some(ci));
            let enter = match tour_here {
                Some(t) => format!("PRESS ENTER FOR TOUR ROUND {}/{}", t.round + 1, t.circuits.len()),
                None => "PRESS ENTER TO RACE".to_string(),
            };
            h.text(cx, top + line * 3.1, px, Align::Center, &enter, hud::WHITE);
            let others = if stage {
                "GRAVEL STAGE TO THE FINISH - ANY WAY YOU CAN"
            } else if self.tour.is_some() {
                "T FOR A TIME TRIAL"
            } else {
                "T TIME TRIAL - G FIVE-RACE TOUR"
            };
            h.text(cx, top + line * 4.1, px, Align::Center, others, hud::DIM);
            h.text(cx, top + line * 5.1, px, Align::Center, &rivals, hud::DIM);
        }

        // First few seconds: controls, fitted between the minimap and the speedo
        if self.time() < 15.0 {
            let hints = ["ARROWS/WASD DRIVE  SPACE HANDBRAKE  M MAP  R RESET", "DRIVE TO ANY START LINE AND PRESS ENTER TO RACE"];
            let left = margin * 2.0 + MINIMAP_SIZE * px;
            let right = width - margin * 2.0 - HudBuilder::text_width("888", px * 4.0);
            let widest = hints.iter().map(|t| HudBuilder::text_width(t, px)).fold(0.0, f32::max);
            let tpx = px * ((right - left) / widest).min(1.0);
            let tline = HudBuilder::text_height(tpx) + tpx * 3.0;
            let y = height - margin - tline * 2.0;
            for (k, text) in hints.iter().enumerate() {
                h.text((left + right) * 0.5, y + tline * k as f32, tpx, Align::Center, text, hud::DIM);
            }
        }
    }

    /// Heading-up minimap in the bottom-left corner.
    fn minimap(&self, h: &mut HudBuilder, height: f32, px: f32) {
        let size = px * MINIMAP_SIZE;
        let margin = px * 6.0;
        let lo = Vec2::new(margin, height - margin - size);
        let hi = lo + Vec2::splat(size);
        h.rect(lo.x, lo.y, size, size, hud::PANEL);
        let view = MapView {
            centre: lo + Vec2::splat(size * 0.5),
            focus: self.player_xz(),
            scale: size * 0.5 / MINIMAP_RANGE,
            yaw: Some(self.player.yaw()),
        };
        let clip = (lo, hi);
        for trail in &self.world.trail_outlines {
            hud::map_polyline(h, &view, trail, false, clip, px * 0.5, TRAIL_COLOR);
        }
        for road in &self.world.road_outlines {
            hud::map_polyline(h, &view, road, false, clip, px * 0.8, [0.75, 0.7, 0.55, 0.9]);
        }
        if let Some(rail) = railway::get() {
            for line in &rail.outlines {
                hud::map_polyline(h, &view, line, false, clip, px * 0.6, RAIL_COLOR);
            }
        }
        let racing = self.race.as_ref().map(|r| r.circuit);
        for (i, c) in self.world.circuits.iter().enumerate() {
            let color = if Some(i) == racing { hud::YELLOW } else if self.progress.circuits.contains(c.name) { hud::WHITE } else { [0.6, 0.6, 0.65, 0.8] };
            hud::map_polyline(h, &view, &c.outline, !c.stage, clip, px * 1.0, if c.stage { stage_color(color) } else { color });
        }
        let dot = |h: &mut HudBuilder, p: Vec2, s: f32, color: [f32; 4]| {
            let q = view.project(p);
            if q.x > lo.x + s && q.y > lo.y + s && q.x < hi.x - s && q.y < hi.y - s {
                h.rect(q.x - s, q.y - s, s * 2.0, s * 2.0, color);
            }
        };
        for l in landmarks::all().iter().filter(|l| self.progress.landmarks.contains(&l.name)) {
            dot(h, l.pos, px * 1.2, if l.kind.is_secret() { hud::GOLD } else { [0.4, 0.9, 1.0, 1.0] });
        }
        for item in self.collectibles.iter().filter(|c| !c.collected) {
            if Vec2::new(item.x, item.z).distance(view.focus) < 700.0 {
                dot(h, Vec2::new(item.x, item.z), px * 0.8, hud::YELLOW);
            }
        }
        if let (Some(train), Some(rail)) = (&self.train, railway::get()) {
            for pose in train.poses(rail) {
                dot(h, Vec2::new(pose.centre.x, pose.centre.z), px * 0.9, TRAIN_COLOR);
            }
        }
        if let Some(race) = &self.race {
            for &ci in &race.rivals {
                let c = &self.cars[ci].physics;
                dot(h, Vec2::new(c.pos.x, c.pos.z), px * 1.0, hud::RED);
            }
        }
        h.arrow(view.centre, Vec2::new(0.0, -1.0), px * 2.5, hud::RED);
    }

    /// Full-screen north-up map of the whole world (M).
    fn world_map(&self, h: &mut HudBuilder, width: f32, height: f32, px: f32) {
        h.rect(0.0, 0.0, width, height, [0.04, 0.06, 0.1, 0.88]);
        let size = height.min(width) * 0.86;
        let view = MapView {
            centre: Vec2::new(width * 0.5, height * 0.52),
            focus: Vec2::ZERO,
            scale: size / (TERRAIN_HALF_SIZE * 2.0),
            yaw: None,
        };
        let clip = (Vec2::ZERO, Vec2::new(width, height));
        for trail in &self.world.trail_outlines {
            hud::map_polyline(h, &view, trail, false, clip, px * 0.4, TRAIL_COLOR);
        }
        for road in &self.world.road_outlines {
            hud::map_polyline(h, &view, road, false, clip, px * 0.7, [0.75, 0.7, 0.55, 0.8]);
        }
        if let Some(rail) = railway::get() {
            for line in &rail.outlines {
                hud::map_polyline(h, &view, line, false, clip, px * 0.6, RAIL_COLOR);
            }
            if let Some(train) = &self.train {
                let poses = train.poses(rail);
                let q = view.project(Vec2::new(poses[0].centre.x, poses[0].centre.z));
                h.rect(q.x - px * 1.2, q.y - px * 1.2, px * 2.4, px * 2.4, TRAIN_COLOR);
                h.text(q.x + px * 2.5, q.y - px * 2.0, px * 0.6, Align::Left, "TRAIN", TRAIN_COLOR);
            }
        }
        for c in &self.world.circuits {
            let known = self.progress.circuits.contains(c.name);
            let color = if known { hud::WHITE } else { [0.45, 0.45, 0.5, 0.9] };
            hud::map_polyline(h, &view, &c.outline, !c.stage, clip, px * 0.8, if c.stage { stage_color(color) } else { color });
            let label = view.project(if c.stage { c.start() } else { c.centre });
            if known {
                let raced = self.progress.best_finish.get(c.name);
                let text_color = if raced == Some(&1) { hud::YELLOW } else { hud::WHITE };
                h.text(label.x, label.y - px * 3.5, px * 0.8, Align::Center, c.name, text_color);
            } else {
                h.text(label.x, label.y - px * 3.5, px * 0.8, Align::Center, "?", [0.6, 0.6, 0.65, 1.0]);
            }
        }
        for l in landmarks::all().iter().filter(|l| self.progress.landmarks.contains(&l.name)) {
            let q = view.project(l.pos);
            let (mark, label) = if l.kind.is_secret() { (hud::GOLD, hud::GOLD) } else { ([0.4, 0.9, 1.0, 1.0], [0.6, 0.9, 1.0, 1.0]) };
            h.rect(q.x - px, q.y - px, px * 2.0, px * 2.0, mark);
            h.text(q.x + px * 2.0, q.y - px * 2.0, px * 0.6, Align::Left, &l.name, label);
        }
        let me = view.project(self.player_xz());
        h.arrow(me, view.direction(self.player.yaw()), px * 3.0, hud::RED);
        h.text(width * 0.5, px * 4.0, px * 1.5, Align::Center, "WORLD MAP", hud::WHITE);
        h.text(width * 0.5, height - px * 10.0, px, Align::Center, "M CLOSE  YELLOW: WINS  ORANGE: GRAVEL STAGES  BLUE: LANDMARKS  GOLD: SECRETS  GREY: RAILWAY", hud::DIM);
    }
}

/// Gravel stages show on the maps in a sandy orange, dimmer until found.
fn stage_color(circuit: [f32; 4]) -> [f32; 4] {
    if circuit == hud::YELLOW { circuit } else { [1.0, 0.6 * circuit[1] + 0.1, 0.2, circuit[3]] }
}

/// Soft discs of light on the ground under the circuit floodlights near
/// the camera, brightest in the middle, showing as night falls.
fn light_pools(camera: Vec3, night: f32) -> (Vec<Vertex>, Vec<u16>) {
    use crate::trackside::{POOL_RINGS, POOL_SIDES};
    const RANGE: f32 = 900.0;
    const ALPHAS: [f32; 3] = [0.17, 0.11, 0.05];
    let (mut verts, mut idxs) = (Vec::new(), Vec::new());
    let strength = ((night - 0.15) / 0.5).clamp(0.0, 1.0);
    let Some(network) = crate::road::network() else { return (verts, idxs) };
    if strength <= 0.0 {
        return (verts, idxs);
    }
    for light in crate::trackside::floodlights(network) {
        if light.pool.distance(Vec2::new(camera.x, camera.z)) > RANGE || verts.len() + POOL_SIDES * 12 > u16::MAX as usize {
            continue;
        }
        // Faces take one colour each, so the light falls off in steps:
        // rings, brightest in the middle (colours above 1 mark it as a
        // light: full strength, unshaded)
        let at = |r: usize, k: usize| light.rings[r * POOL_SIDES + k % POOL_SIDES];
        for r in 0..POOL_RINGS.len() - 1 {
            let color = [1.6, 1.5, 1.3, ALPHAS[r] * strength];
            for k in 0..POOL_SIDES {
                let base = verts.len() as u16;
                for p in [at(r, k), at(r + 1, k), at(r + 1, k + 1), at(r, k + 1)] {
                    verts.push(Vertex { pos: p.into(), color });
                }
                idxs.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    }
    (verts, idxs)
}

fn headlight(car: &CarPhysics, offset: Vec3, range: f32, intensity: f32) -> lights::Light {
    let fwd = car.forward();
    let dir = (fwd - Vec3::Y * 0.2).normalize();
    lights::Light::spot(car.pos + offset + fwd * 4.5, dir, range, 25.0, vec3(1.0, 0.95, 0.8), intensity)
}

/// Blob shadow under a car, lying on the ground; shrinks as the car leaves it.
fn shadow_transform(car: &CarPhysics) -> Mat4 {
    // On the real ground (the lake bed under water), fading out as the car goes under
    let ground = solid_height(car.pos.x, car.pos.z);
    let n = WorldGround.normal(car.pos.x, car.pos.z);
    let height = (car.pos.y - ground - 1.5).max(0.0);
    let scale = (1.0 - height * 0.03).clamp(0.4, 1.0) * (1.0 - car.submerged).max(0.0);
    let tilt = Quat::from_rotation_arc(Vec3::Y, n);
    Mat4::from_scale_rotation_translation(
        Vec3::splat(scale),
        tilt * Quat::from_rotation_y(car.yaw()),
        vec3(car.pos.x, ground + 0.12, car.pos.z),
    )
}

pub(crate) fn shadow_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let color = [0.0, 0.0, 0.0, 0.35];
    let sides = 12;
    let mut verts = vec![Vertex { pos: [0.0, 0.0, 0.0], color }];
    for i in 0..sides {
        let a = std::f32::consts::TAU * i as f32 / sides as f32;
        verts.push(Vertex { pos: [a.cos() * 3.0, 0.0, a.sin() * 5.2], color });
    }
    let idxs = (0..sides as u16).flat_map(|i| [0, 1 + i, 1 + (i + 1) % sides as u16]).collect();
    (verts, idxs)
}

/// Horizon/fog colour for a sun elevation; the sky shader blends up from it.
fn horizon_color(sun_elev: f32) -> Vec3 {
    let smooth = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let day = smooth(-0.3, 0.3, sun_elev);
    let sunset = smooth(0.5, 0.0, sun_elev.abs()) * if sun_elev + 0.2 >= 0.0 { 1.0 } else { 0.0 };
    let night = vec3(0.05, 0.05, 0.1);
    let noon = vec3(0.72, 0.82, 0.92);
    let dusk = vec3(0.95, 0.55, 0.3);
    night.lerp(noon, day).lerp(dusk, sunset)
}
