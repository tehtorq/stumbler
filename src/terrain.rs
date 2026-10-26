use crate::geometry::Vertex;
use crate::road;

/// How far the world's layout is spread out: the circuit grid, the biomes and
/// the big landforms sit this many times further from the centre than they
/// were first drawn (the coordinates below are in that original layout and
/// scaled by `at`). Circuits, roads and the features themselves keep their size.
pub const WORLD_SCALE: f32 = 4.0;
pub const TERRAIN_HALF_SIZE: f32 = 9500.0 * WORLD_SCALE;
/// The volcano's peak, in the far east.
pub const VOLCANO: (f32, f32) = (6500.0 * WORLD_SCALE, 0.0);

/// A position in the original layout, spread out to the world's scale.
const fn at(v: f32) -> f32 {
    v * WORLD_SCALE
}
pub const WATER_LEVEL: f32 = -20.0;

// ── Helpers ──

fn bump(x: f32, z: f32, cx: f32, cz: f32, r: f32, h: f32) -> f32 {
    let d2 = ((x - cx) * (x - cx) + (z - cz) * (z - cz)) / (r * r);
    h * (-d2).exp()
}

/// Smooth ridge along a line from (x0,z0) to (x1,z1), width `w`, height `h`.
fn ridge(x: f32, z: f32, x0: f32, z0: f32, x1: f32, z1: f32, w: f32, h: f32) -> f32 {
    let dx = x1 - x0;
    let dz = z1 - z0;
    let len_sq = dx * dx + dz * dz;
    if len_sq < 1.0 { return 0.0; }
    let t = ((x - x0) * dx + (z - z0) * dz) / len_sq;
    let t = t.clamp(0.0, 1.0);
    let px = x0 + dx * t;
    let pz = z0 + dz * t;
    let dist = ((x - px) * (x - px) + (z - pz) * (z - pz)).sqrt();
    let falloff = 1.0 - (dist / w).min(1.0);
    h * falloff * falloff
}

/// Crater: ring of raised terrain around a depression.
fn crater(x: f32, z: f32, cx: f32, cz: f32, r: f32, depth: f32) -> f32 {
    let dist = ((x - cx) * (x - cx) + (z - cz) * (z - cz)).sqrt();
    let t = dist / r;
    if t > 2.0 { return 0.0; }
    if t < 0.8 {
        // Floor of crater
        -depth * 0.6
    } else if t < 1.0 {
        // Rim wall going up
        let s = (t - 0.8) / 0.2;
        -depth * 0.6 + (depth * 0.6 + depth * 0.4) * s
    } else if t < 1.3 {
        // Outer rim
        let s = (t - 1.0) / 0.3;
        depth * 0.4 * (1.0 - s * s)
    } else {
        0.0
    }
}

/// Plateau: flat top above a certain height.
fn plateau(x: f32, z: f32, cx: f32, cz: f32, r: f32, h: f32) -> f32 {
    let dist = ((x - cx) * (x - cx) + (z - cz) * (z - cz)).sqrt();
    if dist > r * 1.5 { return 0.0; }
    let edge = ((1.0 - dist / (r * 1.5)).max(0.0) * 3.0).min(1.0);
    h * edge * edge
}

/// A plateau with one long, gentle slope up to its top on the side facing
/// (dx, dz), so it can be driven up.
fn mesa(x: f32, z: f32, cx: f32, cz: f32, dx: f32, dz: f32, r: f32, h: f32) -> f32 {
    let (ox, oz) = (x - cx, z - cz);
    let along = ox * dx + oz * dz;
    let across = ox * dz - oz * dx;
    // Distances out on the ramp side count for a fifth, stretching that slope
    let along = if along > 0.0 { along / 5.0 } else { along };
    plateau(along * dx + across * dz + cx, along * dz - across * dx + cz, cx, cz, r, h)
}

/// Directional ramp: steep on one side, gentle on other.
/// Center (cx,cz), direction (dx,dz) is the launch direction, width `w`, height `h`.
fn ramp(x: f32, z: f32, cx: f32, cz: f32, dx: f32, dz: f32, w: f32, h: f32) -> f32 {
    let len = (dx * dx + dz * dz).sqrt();
    if len < 0.01 { return 0.0; }
    let ndx = dx / len;
    let ndz = dz / len;
    // Distance along ramp direction and perpendicular
    let along = (x - cx) * ndx + (z - cz) * ndz;
    let across = ((x - cx) * ndz - (z - cz) * ndx).abs();
    if across > w * 0.5 { return 0.0; }
    // Ramp profile: rises steeply over length, flat top
    let ramp_len = w;
    let t = (along / ramp_len + 0.5).clamp(0.0, 1.0);
    let profile = if t < 0.8 { t / 0.8 } else { 1.0 - (t - 0.8) / 0.2 };
    let edge = 1.0 - (across / (w * 0.5)).powi(2);
    h * profile * edge
}

// ── Biome weights: each returns 0..1 indicating how much of that biome applies ──

pub fn biome_desert(x: f32, z: f32) -> f32 {
    // Southwest region — large arid zone
    let cx = at(-5500.0);
    let cz = at(-5500.0);
    let dist = ((x - cx) * (x - cx) + (z - cz) * (z - cz)).sqrt();
    (1.0 - dist / at(4500.0)).max(0.0).min(1.0)
}

pub fn biome_mountain(x: f32, z: f32) -> f32 {
    // Northern mountain range — wide band
    let strength = ((z - at(3500.0)) / at(3000.0)).max(0.0).min(1.0);
    let east_bias = ((x + at(1000.0)) / at(8000.0)).max(0.0).min(1.0);
    strength * (0.4 + 0.6 * east_bias)
}

pub fn biome_forest(x: f32, z: f32) -> f32 {
    // Northwest deep forest
    let cx = at(-5000.0);
    let cz = at(4500.0);
    let d1 = ((x - cx) * (x - cx) + (z - cz) * (z - cz)).sqrt();
    let f1 = (1.0 - d1 / at(4000.0)).max(0.0).min(1.0);
    // Secondary forest patch (west)
    let d2 = ((x + at(7000.0)) * (x + at(7000.0)) + z * z).sqrt();
    let f2 = (1.0 - d2 / at(2500.0)).max(0.0).min(1.0);
    (f1 + f2).min(1.0)
}

pub fn biome_tundra(_x: f32, z: f32) -> f32 {
    // Far north — frozen plains
    let strength = ((z - at(6500.0)) / at(2000.0)).max(0.0).min(1.0);
    strength
}

// ── Height function ──

/// Natural terrain height clamped above the water surface, ignoring roads.
pub fn natural_height(x: f32, z: f32) -> f32 {
    terrain_height_raw(x, z).max(WATER_LEVEL + 1.0)
}

/// Public terrain height: natural ground, flattened under and around roads.
/// This is the surface physics drives on and the road mesh sits on.
pub fn terrain_height(x: f32, z: f32) -> f32 {
    let natural = natural_height(x, z);
    match road::network().and_then(|n| n.nearest_ground(x, z)) {
        Some(s) => road_blend(s, natural),
        None => natural,
    }
}

/// What a car rests on: like `terrain_height`, but down on the lake bed under
/// open water (roads still cross water on their embankments).
pub fn solid_height(x: f32, z: f32) -> f32 {
    let raw = terrain_height_raw(x, z);
    match road::network().and_then(|n| n.nearest_ground(x, z)) {
        Some(s) => road_blend(s, raw.max(WATER_LEVEL + 1.0)),
        None => raw,
    }
}

/// Flat across the asphalt and shoulder, then a smooth blend to natural ground.
fn road_blend(s: road::RoadSample, natural: f32) -> f32 {
    let flat = s.half_width() + s.shoulder();
    if s.dist <= flat {
        return s.height;
    }
    let t = ((s.dist - flat) / road::BLEND).min(1.0);
    let t = t * t * (3.0 - 2.0 * t);
    s.height + (natural - s.height) * t
}

/// Height used for terrain mesh vertices on a grid of cell size `step`: like
/// `terrain_height`, but can dip below water away from roads, and sits just
/// under the nearest road's surface (at the same cross-section) within a
/// cell's diagonal of it, so triangles reaching onto the road stay beneath it.
pub(crate) fn terrain_mesh_height(x: f32, z: f32, step: f32) -> f32 {
    let raw = terrain_height_raw(x, z);
    match road::network().and_then(|n| n.nearest_ground(x, z)) {
        Some(s) => {
            let h = road_blend(s, raw.max(WATER_LEVEL + 1.0));
            if s.dist < s.half_width() + road::KERB_WIDTH + step * std::f32::consts::SQRT_2 {
                // Coarser cells bridge dips in the road more, so sit a little lower
                h.min(s.height - 0.3 - step * 0.012)
            } else {
                h
            }
        }
        None => raw,
    }
}

/// Raw terrain height (can go below water), ignoring roads.
pub fn raw_height(x: f32, z: f32) -> f32 {
    terrain_height_raw(x, z)
}

fn terrain_height_raw(x: f32, z: f32) -> f32 {
    crate::waterways::carve(x, z, uncarved_height(x, z))
}

/// The land before rivers and canyons are cut into it (they're laid out
/// over this).
pub fn uncarved_height(x: f32, z: f32) -> f32 {
    let desert = biome_desert(x, z);
    let tundra = biome_tundra(x, z);
    let base_amp = 1.0 - 0.6 * desert - 0.3 * tundra;

    // ── Macro landscape: huge valleys and ridges (wavelength 3000-6000 units) ──
    // These define the big journey — cresting a major ridge, descending into a valley
    let macro1 = 60.0 * (x * 0.0011 + 0.7).sin() * (z * 0.0014 + 1.3).sin();
    let macro2 = 40.0 * (x * 0.0019 + 2.1).sin() * (z * 0.0023 + 0.5).sin();
    let macro3 = 25.0 * (x * 0.0007 + 0.2).sin() * (z * 0.0009 + 1.8).sin();
    let macro_terrain = (macro1 + macro2 + macro3) * base_amp;

    // ── Medium hills: the rolling terrain within each valley (wavelength 400-900 units) ──
    let h1 = 15.0 * base_amp * (x * 0.0073 + 0.3).sin() * (z * 0.0097 + 0.7).sin();
    let h2 = 9.0 * base_amp * (x * 0.0131 + 1.1).sin() * (z * 0.0171 + 2.3).sin();
    let h3 = 5.0 * base_amp * (x * 0.0237 + 0.5).sin() * (z * 0.0293 + 1.9).sin();
    let base = macro_terrain + h1 + h2 + h3;

    // ── High-frequency terrain roughness (drives per-wheel suspension) ──
    let mtn = biome_mountain(x, z);
    let forest = biome_forest(x, z);
    let rough1 = 1.5 * (x * 0.05 + 0.8).sin() * (z * 0.06 + 1.2).sin();
    let rough2 = 0.8 * (x * 0.09 + 2.1).sin() * (z * 0.11 + 0.4).sin();
    let rough_scale = 0.4 + 1.2 * mtn + 0.6 * forest;
    let roughness = (rough1 + rough2) * rough_scale;

    // ── Mountain range (north) — massive peaks with rocky ridges ──
    let mountain_height = mtn * 90.0 * (1.0 + 0.5 * (x * 0.003).sin() * (z * 0.004 + 1.0).sin());
    let peaks = mtn * 35.0 * ((x * 0.008).sin() * (z * 0.011).sin()).max(0.0);
    let mtn_ridges = mtn * 15.0 * ((x * 0.02 + z * 0.01).sin()).abs();
    let cliff_n = mtn * ridge(x, z, at(-3000.0), at(5500.0), at(5000.0), at(5000.0), 150.0, 45.0);
    let cliff_n2 = mtn * ridge(x, z, at(1000.0), at(6500.0), at(6000.0), at(6000.0), 120.0, 35.0);

    // ── Desert features (southwest) ──
    let desert_floor = -20.0 * desert;
    let mesa1 = plateau(x, z, at(-6000.0), at(-6000.0), 500.0, 45.0) * desert;
    let mesa2 = plateau(x, z, at(-4500.0), at(-7000.0), 350.0, 35.0) * desert;
    let mesa3 = plateau(x, z, at(-7000.0), at(-4500.0), 400.0, 40.0) * desert;
    let canyon = ridge(x, z, at(-7500.0), at(-4000.0), at(-4000.0), at(-7500.0), 80.0, -30.0) * desert;
    let dunes = desert * 8.0 * (x * 0.015 + z * 0.005).sin() * (x * 0.003 - z * 0.012).sin();
    let washboard = desert * 3.0 * (x * 0.05 + z * 0.03).sin();
    let desert_ridge1 = desert * ridge(x, z, at(-7000.0), at(-6000.0), at(-5000.0), at(-4000.0), 100.0, 25.0);
    let desert_ridge2 = desert * ridge(x, z, at(-6500.0), at(-3500.0), at(-4500.0), at(-5500.0), 80.0, 20.0);

    // ── Forest (northwest) — dense rolling hills with gullies ──
    let forest_hills = forest * 20.0 * (x * 0.006 + 0.5).sin() * (z * 0.008 + 0.3).sin();
    let gully1 = forest * ridge(x, z, at(-6000.0), at(3000.0), at(-4000.0), at(5000.0), 50.0, -12.0);
    let gully2 = forest * ridge(x, z, at(-5500.0), at(4500.0), at(-3500.0), at(3500.0), 40.0, -10.0);
    let forest_bumps = forest * 3.0 * ((x * 0.04).sin() * (z * 0.06).sin()).max(0.0) * 4.0;

    // ── Tundra — gentle undulation with frozen ridges ──
    let tundra_roll = tundra * 5.0 * (x * 0.004).sin() * (z * 0.005).sin();
    let tundra_ridge = tundra * ridge(x, z, at(-2000.0), at(7500.0), at(4000.0), at(8000.0), 200.0, 20.0);

    // ── Grassland ridgelines (center, visible from tracks) ──
    let grass_mask = (1.0 - mtn) * (1.0 - desert);
    let grass_ridge1 = ridge(x, z, at(-2000.0), at(-1000.0), at(2000.0), at(500.0), 180.0, 25.0) * grass_mask;
    let grass_ridge2 = ridge(x, z, at(500.0), at(-3000.0), at(3000.0), at(-1500.0), 150.0, 20.0) * grass_mask;
    let escarpment = ridge(x, z, at(-1000.0), at(1500.0), at(4000.0), at(2000.0), 100.0, 35.0) * (1.0 - mtn);

    // ── Landmarks ──
    // Volcano (far east)
    let volcano = bump(x, z, VOLCANO.0, VOLCANO.1, 600.0, 120.0)
                + crater(x, z, VOLCANO.0, VOLCANO.1, 220.0, 40.0);
    // Crater lake (between GP and figure-8)
    let crater_h = crater(x, z, at(3500.0), at(-3000.0), 350.0, 40.0);
    // Twin peaks (north center)
    let twin1 = bump(x, z, at(-1000.0), at(6000.0), 400.0, 100.0);
    let twin2 = bump(x, z, at(-1000.0) + 500.0, at(6000.0) + 300.0, 350.0, 80.0);
    let valley = ridge(x, z, at(-1000.0), at(6000.0), at(-1000.0) + 500.0, at(6000.0) + 300.0, 100.0, -35.0);
    // Coastal cliffs (south edge)
    let cliff_south = ridge(x, z, at(-7000.0), at(-8500.0), at(7000.0), at(-8500.0), 300.0, 50.0);
    // Lone mountain (far west)
    let lone_mtn = bump(x, z, at(-7500.0), at(0.0), 500.0, 70.0);
    // Island hill (southeast)
    let island = bump(x, z, at(5000.0), at(-6000.0), 400.0, 55.0);
    // Great ridge (runs east-west through center-south)
    let great_ridge = ridge(x, z, at(-4000.0), at(-3000.0), at(4000.0), at(-2500.0), 250.0, 35.0);
    // Frozen lake depression (far north)
    let frozen_lake = bump(x, z, at(2000.0), at(7500.0), 500.0, -20.0);
    // Badlands spires (deep desert)
    let spire1 = bump(x, z, at(-5800.0), at(-5200.0), 80.0, 50.0);
    let spire2 = bump(x, z, at(-5800.0) + 300.0, at(-5200.0) - 400.0, 60.0, 40.0);
    let spire3 = bump(x, z, at(-5800.0) - 400.0, at(-5200.0) - 600.0, 70.0, 45.0);

    // ── Track ramps (gentle hills on circuits) ──
    let ramps = 0.0
        + bump(x, z, 1000.0, -400.0, 80.0, 14.0)
        + bump(x, z, -175.0, -750.0, 100.0, 16.0)
        + bump(x, z, -600.0, -450.0, 120.0, 12.0)
        + bump(x, z, -700.0, 0.0, 90.0, 15.0)
        + bump(x, z, 900.0, 300.0, 100.0, 13.0)
        + bump(x, z, 300.0, -650.0, 70.0, 10.0)
        + bump(x, z, 2100.0, 1400.0, 120.0, 18.0)
        + bump(x, z, 1700.0, 1100.0, 80.0, 12.0);

    // ── Dedicated launch ramps (steep, directional, BIG air) ──
    let launch_ramps = 0.0
        // Silverstone exit — launches toward Monza road
        + ramp(x, z, 1600.0, -100.0, 1.0, 0.5, 40.0, 20.0)
        // Connecting road south — mid-journey jump
        + ramp(x, z, 0.0, -1800.0, 0.0, -1.0, 35.0, 18.0)
        // Near Spa approach
        + ramp(x, z, 1800.0, 2200.0, 0.5, 1.0, 35.0, 16.0)
        // Desert road jump
        + ramp(x, z, -1200.0, -800.0, -1.0, -0.5, 40.0, 22.0)
        // Forest shortcut jump
        + ramp(x, z, -1500.0, 700.0, -0.5, 1.0, 30.0, 15.0)
        // Open field — just for fun
        + ramp(x, z, 500.0, 800.0, 0.0, 1.0, 50.0, 25.0);

    // ── The Alps (far north-east): jagged ridges with snowy peaks on the skyline ──
    let alps = alps_height(x, z);

    let land = base + roughness + alps
        + mountain_height + peaks + mtn_ridges + cliff_n + cliff_n2
        + desert_floor + mesa1 + mesa2 + mesa3 + canyon + dunes + washboard
        + desert_ridge1 + desert_ridge2
        + forest_hills + forest_bumps + gully1 + gully2
        + tundra_roll + tundra_ridge
        + grass_ridge1 + grass_ridge2 + escarpment
        + crater_h + volcano + twin1 + twin2 + valley
        + cliff_south + lone_mtn + island + great_ridge
        + frozen_lake + spire1 + spire2 + spire3
        + ramps + launch_ramps
        + scattered(x, z, desert, forest, mtn, tundra);
    coast(x, z, land)
}

/// The sea floor out past the coast.
const SEA_FLOOR: f32 = WATER_LEVEL - 25.0;
/// The coast runs about this far in from the map's edge, wandering either way.
pub const COAST: f32 = 2600.0;
pub const COAST_WANDER: f32 = 1100.0;

/// The world is an island: the land runs down into the sea before the
/// map's edge, so there's always a shore to drive to and nowhere to fall off.
fn coast(x: f32, z: f32, land: f32) -> f32 {
    let out = x.abs().max(z.abs()) + (crate::noise::noise(x, z, 3200.0) - 0.5) * 2.0 * COAST_WANDER;
    let shore = TERRAIN_HALF_SIZE - COAST;
    let t = smooth(shore - 900.0, shore + 700.0, out);
    land + (SEA_FLOOR - land) * t
}

/// Side of the cells that scattered landforms are dealt out on.
const SCATTER_CELL: f32 = 1600.0;
/// Circuit slots: the layout's grid spacing, in the original layout.
const SLOT_SPACING: f32 = at(3500.0);

/// Landforms dealt out over the whole map, one or none per cell, to suit
/// the biome: mesas and spires in the desert, hills in the woods and
/// grassland with the odd ridge or crater lake, hollows in the tundra.
/// They fill the country between circuits and keep clear of the circuit
/// slots themselves.
fn scattered(x: f32, z: f32, desert: f32, forest: f32, mtn: f32, tundra: f32) -> f32 {
    use crate::noise::hash;
    // Clear of the circuits: nothing within reach of a grid slot
    let slot = |v: f32| (v - (v / SLOT_SPACING).round() * SLOT_SPACING).abs();
    let clear = smooth(2400.0, 3400.0, slot(x).hypot(slot(z)));
    if clear <= 0.0 || mtn > 0.6 {
        return 0.0;
    }
    let (ci, cj) = ((x / SCATTER_CELL).floor(), (z / SCATTER_CELL).floor());
    let mut h = 0.0;
    for dj in -1..=1 {
        for di in -1..=1 {
            let (i, j) = (ci + di as f32, cj + dj as f32);
            let roll = hash(i * 1.7 + 11.0, j * 2.3 - 7.0);
            if roll > 0.55 {
                continue;
            }
            // Kept well inside the cell, so only its neighbours can reach x, z
            let cx = (i + 0.25 + 0.5 * hash(i, j + 31.0)) * SCATTER_CELL;
            let cz = (j + 0.25 + 0.5 * hash(i + 17.0, j)) * SCATTER_CELL;
            let size = hash(i - 5.0, j + 9.0);
            let pick = roll / 0.55;
            h += if desert > 0.3 {
                if pick < 0.6 {
                    let a = hash(i + 3.0, j - 11.0) * std::f32::consts::TAU;
                    mesa(x, z, cx, cz, a.cos(), a.sin(), 250.0 + 250.0 * size, 25.0 + 20.0 * size) * desert
                } else {
                    (bump(x, z, cx, cz, 70.0, 45.0) + bump(x, z, cx + 260.0, cz - 340.0, 60.0, 35.0)) * desert
                }
            } else if tundra > 0.5 {
                bump(x, z, cx, cz, 350.0 + 250.0 * size, -18.0) * tundra
            } else if pick < 0.12 && forest < 0.3 {
                crater(x, z, cx, cz, 200.0 + 150.0 * size, 30.0 + 10.0 * size)
            } else if pick < 0.3 {
                let a = size * std::f32::consts::TAU;
                let (dx, dz) = (a.cos() * 700.0, a.sin() * 700.0);
                ridge(x, z, cx - dx, cz - dz, cx + dx, cz + dz, 120.0 + 60.0 * size, 18.0 + 14.0 * size)
            } else {
                bump(x, z, cx, cz, 300.0 + 400.0 * size, 12.0 + 25.0 * size * (1.0 - 0.5 * forest))
            };
        }
    }
    h * clear * (1.0 - mtn)
}

/// How far into the north-eastern alps (x, z) is, 0..1.
pub fn alpine(x: f32, z: f32) -> f32 {
    smooth(at(2800.0), at(5200.0), x) * smooth(at(5000.0), at(7000.0), z)
}

/// Extra height of the north-eastern range: crests along the ridgelines of
/// two scales of noise, rising out of the foothills.
fn alps_height(x: f32, z: f32) -> f32 {
    use crate::noise::noise;
    let mask = alpine(x, z);
    if mask <= 0.0 {
        return 0.0;
    }
    let ridged = |scale: f32, dx: f32| {
        let n = noise(x + dx, z - dx * 0.5, scale);
        let r = 1.0 - (2.0 * n - 1.0).abs();
        r * r
    };
    mask * (240.0 * ridged(1100.0, 0.0) + 90.0 * ridged(380.0, 4000.0) + 25.0 * ridged(120.0, 9000.0))
}

// ── Biome-based vertex coloring ──

type Rgb = [f32; 3];

fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smooth(edge0: f32, edge1: f32, v: f32) -> f32 {
    let t = ((v - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

const MEADOW: Rgb = [0.3, 0.54, 0.17];
const MEADOW_LUSH: Rgb = [0.2, 0.45, 0.14];
const MEADOW_DRY: Rgb = [0.55, 0.58, 0.25];
const FOREST_FLOOR: Rgb = [0.17, 0.32, 0.12];
const NEEDLES: Rgb = [0.33, 0.27, 0.16];
const SAND: Rgb = [0.84, 0.72, 0.47];
const SAND_DARK: Rgb = [0.72, 0.56, 0.34];
const RED_ROCK: Rgb = [0.64, 0.37, 0.24];
const ROCK: Rgb = [0.46, 0.44, 0.41];
const SNOW: Rgb = [0.9, 0.92, 0.97];
const FROZEN_GRASS: Rgb = [0.55, 0.57, 0.46];
const BEACH: Rgb = [0.8, 0.74, 0.55];
const SHALLOWS: Rgb = [0.55, 0.6, 0.5];

/// Ground colour at a terrain vertex `y` high.
pub fn terrain_color(x: f32, z: f32, y: f32) -> [f32; 4] {
    use crate::noise::patchy;

    // Slope over a wide span, so small bumps don't read as rock
    let e = 9.0;
    let dhdx = (terrain_height_raw(x + e, z) - terrain_height_raw(x - e, z)) / (2.0 * e);
    let dhdz = (terrain_height_raw(x, z + e) - terrain_height_raw(x, z - e)) / (2.0 * e);
    let slope = (dhdx * dhdx + dhdz * dhdz).sqrt();
    let raw = terrain_height_raw(x, z);

    let desert = biome_desert(x, z);
    let mountain = biome_mountain(x, z);
    let forest = biome_forest(x, z);
    let tundra = biome_tundra(x, z);
    let total = desert + mountain + forest + tundra;
    let (desert, mountain, forest, tundra, grass) = if total > 1.0 {
        (desert / total, mountain / total, forest / total, tundra / total, 0.0)
    } else {
        (desert, mountain, forest, tundra, 1.0 - total)
    };

    // Grassland: meadows lush in hollows, drier in sunny patches and up high
    let moist = patchy(x, z, 700.0);
    let mut meadow = mix3(MEADOW_DRY, MEADOW, smooth(0.3, 0.6, moist));
    meadow = mix3(meadow, MEADOW_LUSH, smooth(0.65, 0.85, moist) - smooth(20.0, 60.0, y) * 0.5);
    meadow = mix3(meadow, MEADOW_DRY, smooth(40.0, 110.0, y) * 0.5);

    // Forest floor: dark moss with drifts of needles
    let floor = mix3(FOREST_FLOOR, NEEDLES, smooth(0.55, 0.75, patchy(x - 300.0, z, 160.0)) * 0.7);

    // Desert: rippled sand, darker in troughs
    let ripple = patchy(x * 1.0 + z * 0.4, z, 120.0);
    let sand = mix3(SAND_DARK, SAND, smooth(0.3, 0.7, ripple));

    // Mountains: grass giving way to rock, then snow above a ragged snowline
    let snowline = snowline(x, z);
    let mut peak = mix3(mix3(MEADOW_DRY, FOREST_FLOOR, 0.4), ROCK, smooth(70.0, 130.0, y));
    peak = mix3(peak, SNOW, smooth(snowline - 10.0, snowline + 15.0, y));

    // Tundra: snow, with frozen grass where the wind has scoured it
    let tundra_c = mix3(SNOW, FROZEN_GRASS, smooth(0.66, 0.86, patchy(x + 900.0, z, 300.0)) * 0.7);

    let mut c = [0.0; 3];
    for (w, col) in [(grass, meadow), (forest, floor), (desert, sand), (mountain, peak), (tundra, tundra_c)] {
        for k in 0..3 {
            c[k] += col[k] * w;
        }
    }

    // The volcano: dark basalt and ash, scorched red-brown round the crater
    let to_cone = ((x - VOLCANO.0).powi(2) + (z - VOLCANO.1).powi(2)).sqrt();
    let volcanic = smooth(650.0, 350.0, to_cone);
    if volcanic > 0.0 {
        let ash = mix3([0.24, 0.22, 0.22], [0.36, 0.33, 0.31], smooth(0.4, 0.7, patchy(x, z, 90.0)));
        let scorched = mix3(ash, [0.45, 0.22, 0.14], smooth(300.0, 220.0, to_cone) * 0.8);
        c = mix3(c, scorched, volcanic);
    }

    // Farmland: each field its own shade of its crop, the yard packed earth
    match crate::farms::at(glam::Vec2::new(x, z)) {
        Some(crate::farms::Spot::Field(field)) => {
            let shade = 0.92 + 0.12 * crate::props::hash(field.seed, 3.0) + 0.06 * (patchy(x, z, 40.0) - 0.5);
            c = field.crop.ground().map(|v| v * shade);
        }
        Some(crate::farms::Spot::Yard) => c = mix3([0.5, 0.43, 0.32], [0.45, 0.4, 0.3], patchy(x, z, 25.0)),
        None => {}
    }

    // Steep ground shows bare rock: red in the desert, grey elsewhere,
    // except where snow clings on
    // Strata: bands of lighter and darker, warmer and cooler stone
    let strata = patchy(x * 0.3, y * 6.0 + z * 0.3, 40.0);
    let stone = mix3(mix3(ROCK, [0.36, 0.34, 0.32], smooth(0.35, 0.65, strata)), [0.55, 0.5, 0.43], smooth(0.6, 0.9, patchy(x, z, 230.0)) * 0.6);
    let rock = mix3(stone, RED_ROCK, desert * 1.5);
    let cliff = smooth(0.45, 0.85, slope) * 0.85;
    let snowy = smooth(snowline, snowline + 25.0, y) * mountain + tundra * 0.6;
    c = mix3(c, rock, cliff * (1.0 - snowy * 0.6));

    // Beaches where land meets water, and pale sand in the shallows
    let shore = smooth(WATER_LEVEL + 3.5, WATER_LEVEL + 0.5, raw) * (1.0 - tundra);
    c = mix3(c, BEACH, shore * (1.0 - cliff));
    if raw < WATER_LEVEL {
        let depth = WATER_LEVEL - raw;
        c = mix3(SHALLOWS, [0.2, 0.3, 0.32], smooth(0.0, 10.0, depth));
    }

    // Per-vertex brightness jitter: with flat interpolation each face takes its
    // first vertex's colour, giving the faceted low-poly patchwork.
    let jitter = 0.95 + 0.1 * vertex_hash(x, z);
    [c[0] * jitter, c[1] * jitter, c[2] * jitter, 1.0]
}

fn vertex_hash(x: f32, z: f32) -> f32 {
    let h = (x * 12.9898 + z * 78.233).sin() * 43758.547;
    h - h.floor()
}

/// Surface grip multiplier at world position. 1.0 = normal grass, <1 = slippery/sandy.
/// Where the mountains' ragged snowline runs (a height).
fn snowline(x: f32, z: f32) -> f32 {
    150.0 + 40.0 * crate::noise::patchy(x, z, 900.0)
}

/// How snowy the ground at (x, z) is, 0..1: high mountains above the
/// snowline, and the tundra. (Judged on the land before rivers are cut in,
/// which laying them out needs to know.)
pub fn snow(x: f32, z: f32) -> f32 {
    let y = uncarved_height(x, z);
    let mountain = biome_mountain(x, z).max(alpine(x, z));
    (smooth(snowline(x, z), snowline(x, z) + 25.0, y) * mountain + biome_tundra(x, z) * 0.6).min(1.0)
}

/// `surface_grip` when the weather leaves grip at `wet` (1 dry, less after
/// rain). Where it's snowy it falls as snow, and the ground is no worse.
pub fn wet_grip(x: f32, z: f32, wet: f32) -> f32 {
    surface_grip(x, z) * (1.0 - (1.0 - wet) * (1.0 - snow(x, z)))
}

pub fn surface_grip(x: f32, z: f32) -> f32 {
    let desert = biome_desert(x, z);
    let tundra = biome_tundra(x, z);
    // Desert = sandy/loose (0.6 grip), tundra = icy (0.5 grip), grassland =
    // normal (1.0). Up among the alpine crags it's bare rock more than ice
    let ice = tundra * (1.0 - 0.6 * alpine(x, z));
    1.0 - 0.4 * desert - 0.5 * ice
}

pub struct TerrainChunk {
    pub verts: Vec<Vertex>,
    pub idxs: Vec<u32>,
    pub min_y: f32,
    pub max_y: f32,
}

/// Whether the terrain cell with corner (x, z) on a grid of `step` splits
/// along its other diagonal (top-right to bottom-left).
pub fn cell_flipped(x: f32, z: f32, step: f32) -> bool {
    let i = ((x + TERRAIN_HALF_SIZE) / step).round() as i64;
    let j = ((z + TERRAIN_HALF_SIZE) / step).round() as i64;
    (i + j) % 2 != 0
}

/// Terrain mesh for the square at (x0, z0) of side `size`, with `cells` cells
/// per side. Grid lines sit on a global lattice so neighbouring chunks at the
/// same detail share edges; a skirt hangs off each edge to hide cracks
/// between chunks at different detail levels.
pub fn generate_terrain_chunk(x0: f32, z0: f32, size: f32, cells: usize) -> TerrainChunk {
    let step = size / cells as f32;
    let row = cells + 1;
    let mut verts = Vec::with_capacity(row * row + cells * 8);
    let mut idxs = Vec::with_capacity(cells * cells * 6 + cells * 24);
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for iz in 0..row {
        for ix in 0..row {
            let x = x0 + ix as f32 * step;
            let z = z0 + iz as f32 * step;
            let y = terrain_mesh_height(x, z, step);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            verts.push(Vertex { pos: [x, y, z], color: terrain_color(x, z, y) });
        }
    }
    for iz in 0..cells {
        for ix in 0..cells {
            let tl = (iz * row + ix) as u32;
            let tr = tl + 1;
            let bl = tl + row as u32;
            let br = bl + 1;
            // Alternate the diagonal cell by cell (on the global lattice, so
            // neighbouring chunks agree) to break up the regular grid look
            if cell_flipped(x0 + ix as f32 * step, z0 + iz as f32 * step, step) {
                idxs.extend_from_slice(&[tl, tr, bl, tr, br, bl]);
            } else {
                idxs.extend_from_slice(&[tl, tr, br, tl, br, bl]);
            }
        }
    }

    // Skirts: duplicate each edge vertex lower down and join with a strip
    let drop = step * 0.8 + 2.0;
    let edges: [Vec<usize>; 4] = [
        (0..row).map(|i| i).collect(),                         // z = z0
        (0..row).map(|i| i * row + cells).collect(),           // x = x1
        (0..row).rev().map(|i| cells * row + i).collect(),     // z = z1
        (0..row).rev().map(|i| i * row).collect(),             // x = x0
    ];
    for edge in edges {
        let base = verts.len() as u32;
        for &k in &edge {
            let mut v = verts[k];
            v.pos[1] -= drop;
            verts.push(v);
        }
        for e in 0..edge.len() - 1 {
            let (a, b) = (edge[e] as u32, edge[e + 1] as u32);
            let (c, d) = (base + e as u32 + 1, base + e as u32);
            idxs.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }

    TerrainChunk { verts, idxs, min_y: min_y - drop, max_y }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesas_have_a_drivable_ramp_to_the_top() {
        // The biggest and tallest the scattered mesas come, any way round
        let (r, top) = (500.0, 45.0);
        for k in 0..8 {
            let a = k as f32 * 0.8;
            let dir = glam::Vec2::new(a.cos(), a.sin());
            let h = |d: f32| { let p = dir * d; mesa(p.x, p.y, 0.0, 0.0, dir.x, dir.y, r, top) };
            assert!(h(r * 0.5) > top - 0.5, "the top isn't flat");
            let steepest = (0..400).map(|s| s as f32 * 25.0).map(|d| (h(d) - h(d + 10.0)) / 10.0).fold(0.0, f32::max);
            assert!(steepest < 0.15, "ramp {steepest:.2} steep");
            // The other sides stay steep
            let back = |d: f32| { let p = -dir * d; mesa(p.x, p.y, 0.0, 0.0, dir.x, dir.y, r, top) };
            let wall = (0..50).map(|s| r + s as f32 * 5.0).map(|d| (back(d) - back(d + 5.0)) / 5.0).fold(0.0, f32::max);
            assert!(wall > 0.3, "no steep side behind ({wall:.2})");
        }
    }
}
