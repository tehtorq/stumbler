// Generic vertex-colored mesh: trees, cars, decor, particles, water.

struct Object {
    model: mat4x4<f32>,
    // x = kind: 0 plain, 1 foliage (vertex alpha is a sway weight),
    // 2 water (vertex alpha is depth, 0 at the shore)
    params: vec4<f32>,
}

@group(1) @binding(0) var<uniform> object: Object;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) @interpolate(flat) kind: f32,
}

const PLAIN: f32 = 0.0;
const FOLIAGE: f32 = 1.0;
const WATER: f32 = 2.0;
const COVER: f32 = 3.0;
const GLOW: f32 = 4.0;
const PARTICLE: f32 = 5.0;
const CROWD: f32 = 6.0;
const FLAG: f32 = 7.0;
const PAINT: f32 = 8.0;
/// Ground cover sinks away between these distances from the camera
/// (the chunk streamer loads it out to COVER_RANGE, a little beyond).
const COVER_FADE: vec2<f32> = vec2<f32>(560.0, 740.0);

fn is_kind(kind: f32, k: f32) -> bool {
    return abs(kind - k) < 0.5;
}

/// Whether a home's window with this `bedtime` (0..1, from early to bed
/// to night owl) is lit at `clock` (0..1 from midnight): nearly all in the
/// evening, going out one by one from ten till two, a few night owls
/// after that, early risers from five, and some by day.
fn home_light(bedtime: f32, clock: f32) -> bool {
    let h = clock * 24.0;
    if (h >= 16.5 && h < 22.0) { return bedtime > 0.1; }
    if (h >= 22.0 || h < 2.0) {
        let late = (h - 22.0 + 24.0) % 24.0 / 4.0;
        return bedtime > 0.1 + 0.86 * late;
    }
    if (h < 5.0) { return bedtime > 0.96; }
    if (h < 7.5) { return bedtime > 0.7; }
    return bedtime > 0.6;
}

/// Swell on open water: a few crossing waves, gentle and slow.
fn wave_height(p: vec2<f32>, t: f32) -> f32 {
    return sin(p.x * 0.13 + t * 1.1) * 0.5
         + sin(p.y * 0.17 - t * 0.9) * 0.35
         + sin((p.x - p.y) * 0.09 + t * 0.7) * 0.45;
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var world_pos = object.model * vec4<f32>(in.pos, 1.0);
    let kind = object.params.x;
    let cover = is_kind(kind, COVER);
    let foliage = is_kind(kind, FOLIAGE) || cover;

    // Plants bend more the further up they reach; nothing else moves
    let sway_weight = select(0.0, in.color.a * select(1.0, 0.5, cover), foliage);
    if (cover) {
        // Small plants sink into the ground near the edge of their range
        let fade = smoothstep(COVER_FADE.y, COVER_FADE.x, distance(world_pos.xyz, globals.cam_pos));
        world_pos.y -= in.color.a * 4.0 * (1.0 - fade);
    }

    // Struck trees whip back and forth, bending more the higher up the trunk
    for (var i = 0u; i < select(0u, globals.sway_count, is_kind(kind, FOLIAGE)); i++) {
        let s = globals.sway[i * 2u];
        let dir = globals.sway[i * 2u + 1u].xy;
        let dx = world_pos.x - s.x;
        let dz = world_pos.z - s.y; // s.y = world z
        let dist = sqrt(dx * dx + dz * dz);
        let above = world_pos.y - s.z;
        if (dist < 12.0 && above > 0.0) {
            let height_factor = clamp(above * 0.06, 0.0, 1.0);
            let bend = s.w * (1.0 - dist / 12.0) * height_factor * 4.0;
            world_pos.x += dir.x * bend;
            world_pos.z += dir.y * bend;
        }
    }

    // Ambient wind: gusts roll across the landscape as a slow wave
    if (sway_weight > 0.0) {
        let phase = world_pos.x * 0.03 + world_pos.z * 0.02;
        let t = globals.wind_time;
        let gust = 0.7 + 0.3 * sin(t * 0.4 + world_pos.x * 0.004);
        let wind = (sin(t * 1.5 + phase) * 0.35 + sin(t * 2.3 + phase * 1.7) * 0.15) * gust;
        world_pos.x += wind * sway_weight;
        world_pos.z += wind * 0.6 * sway_weight;
    }

    // A crowd on its feet: everyone bounces to their own rhythm, harder as
    // the race heats up and the nearer the car; arms go up on top of that,
    // and now and then a wave runs round the stand
    if (is_kind(kind, CROWD)) {
        let t = globals.wind_time;
        let arm = in.color.a >= 0.5;
        let phase = fract(in.color.a * 2.0);
        let near = 1.0 - smoothstep(30.0, 260.0, distance(world_pos.xz, globals.crowd.xy));
        let energy = clamp(globals.crowd.z + near * 0.8, 0.12, 1.0);
        let beat = sin(t * (5.5 + phase * 3.5) + phase * 40.0);
        var lift = max(beat, 0.0) * 0.55 * energy;
        let wave = sin((world_pos.x + world_pos.z) * 0.035 - t * 1.6);
        lift += pow(max(wave, 0.0), 12.0) * 0.9 * globals.crowd.z;
        world_pos.y += lift;
        if (arm) {
            world_pos.y += (0.2 + beat * 0.45) * energy + pow(max(wave, 0.0), 12.0) * 0.5 * globals.crowd.z;
        }
        let lean = sin(t * 1.3 + phase * 30.0) * 0.1 * energy;
        world_pos.x += lean;
    }

    // Cloth ripples along its length, out toward the free edge
    if (is_kind(kind, FLAG)) {
        let t = globals.wind_time;
        let along = world_pos.x * 0.7 + world_pos.z * 0.7;
        let flap = sin(t * 7.0 + along * 1.4) * 0.45 + sin(t * 11.0 + along * 2.3) * 0.2;
        world_pos.x += flap * in.color.a;
        world_pos.z -= flap * 0.7 * in.color.a;
        world_pos.y += sin(t * 5.0 + along) * 0.25 * in.color.a;
    }

    // Water swells out in the deep and lies still at the shore
    if (is_kind(kind, WATER)) {
        world_pos.y += wave_height(world_pos.xz, globals.wind_time) * 0.7 * in.color.a;
    }

    var out: VertexOut;
    out.clip = globals.view_proj * world_pos;
    let animated = is_kind(kind, CROWD) || is_kind(kind, FLAG);
    out.color = vec4<f32>(in.color.rgb, select(in.color.a, 1.0, foliage || animated));

    // Cameras held up in the stands: a flash here and there, a ripple of
    // them as the car goes by, more when the race is close
    if (is_kind(kind, CROWD) && in.color.a >= 0.5) {
        let near = 1.0 - smoothstep(20.0, 180.0, distance(world_pos.xz, globals.crowd.xy));
        let chance = 0.0008 + near * near * 0.05 * (0.4 + globals.crowd.z);
        let slot = floor(globals.wind_time * 12.0);
        let roll = fract(sin(fract(in.color.a * 2.0) * 9173.1 + slot * 12.9898) * 43758.547);
        if (roll < chance) {
            out.color = vec4<f32>(3.0, 3.0, 2.9, 1.0);
        }
    }
    out.world_pos = world_pos.xyz;
    out.kind = kind;
    return out;
}

/// Water surface: deep colour, foam at the shore, sky reflected at shallow
/// viewing angles, and the sun glinting off wave facets.
fn shade_water(in: VertexOut, normal: vec3<f32>) -> vec4<f32> {
    let depth = in.color.a;
    let day = day_factor();
    let albedo = lin(in.color.rgb);
    var lit = sun_lighting(albedo, normal, cloud_light(in.world_pos.xz));

    let to_eye = normalize(globals.cam_pos - in.world_pos);
    let facing = max(dot(normal, to_eye), 0.0);
    let fresnel = pow(1.0 - facing, 4.0);
    lit = mix(lit, haze_color(reflect(-to_eye, normal)), fresnel * 0.6);

    // Each wave facet catches the sun on its own, so the sun's path sparkles
    let glint = pow(max(dot(reflect(-globals.sun_dir, normal), to_eye), 0.0), 260.0);
    lit += sun_color() * glint * 6.0 * day * (1.0 - globals.screen.z);

    // Foam laps along the waterline, coming and going
    let lap = 0.55 + 0.45 * sin(globals.wind_time * 1.7 + in.world_pos.x * 0.21 + in.world_pos.z * 0.17);
    let foam = (1.0 - smoothstep(0.0, 0.02, depth)) * 0.45 * lap;
    lit = mix(lit, lin(vec3<f32>(0.85, 0.9, 0.92) * (0.55 + 0.45 * day)), foam);

    // Clear in the shallows, opaque out in the deep
    let alpha = mix(0.32, 0.86, smoothstep(0.02, 0.7, depth)) + foam * 0.4 + fresnel * 0.1;
    lit += eval_lights(in.world_pos, normal, albedo) + eval_street(in.world_pos, normal, albedo);
    lit = apply_fog(lit, in.world_pos);
    return vec4<f32>(max(lit, vec3<f32>(0.0)), clamp(alpha, 0.0, 1.0));
}

/// A glossy coat: the sky reflected, strongest at a glancing angle, and a
/// hot glint off any facet turned just right to the sun (on flat facets the
/// whole face flashes as the car turns).
fn gloss(world_pos: vec3<f32>, normal: vec3<f32>, sun: f32) -> vec3<f32> {
    let to_eye = normalize(globals.cam_pos - world_pos);
    let r = reflect(-to_eye, normal);
    let fresnel = 0.01 + 0.3 * pow(1.0 - max(dot(normal, to_eye), 0.0), 5.0);
    // Paler at the horizon, deeper blue overhead
    let sky = mix(haze_color(r), lin(globals.fog.rgb) * vec3<f32>(0.45, 0.6, 1.0), clamp(r.y, 0.0, 1.0));
    let half_dir = normalize(globals.sun_dir + to_eye);
    let glint = pow(max(dot(normal, half_dir), 0.0), 80.0) * 4.0 * day_factor() * (1.0 - globals.screen.z) * sun;
    return sky * fresnel + sun_color() * glint;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (is_kind(in.kind, PARTICLE)) {
        let peak = max(in.color.r, max(in.color.g, in.color.b));
        // A light (fireflies, floodlight pools) shines; the fainter it's
        // drawn, the less it adds, as blending in linear light brightens more
        let glow = lin(min(in.color.rgb, vec3<f32>(1.0))) * (0.22 + 3.5 * in.color.a * in.color.a);
        let tone = select(lin(in.color.rgb * (0.3 + 0.7 * day_factor())), glow, peak > 1.0);
        return vec4<f32>(apply_fog(tone, in.world_pos), in.color.a);
    }
    if (is_kind(in.kind, CROWD) && in.color.r > 2.0) {
        return vec4<f32>(apply_fog(vec3<f32>(9.0, 9.0, 8.5), in.world_pos), 1.0);
    }
    let normal = flat_normal(in.world_pos);
    if (is_kind(in.kind, WATER)) {
        return shade_water(in, normal);
    }
    let albedo = lin(in.color.rgb);
    let sun = sun_reaches(in.world_pos, normal) * cloud_light(in.world_pos.xz);
    var lit = sun_lighting(albedo, normal, sun);
    if (is_kind(in.kind, FOLIAGE) || is_kind(in.kind, COVER)) {
        // Leaves let some sunlight through, so backlit trees glow a little
        // instead of going black
        let through = max(dot(-normal, globals.sun_dir), 0.0);
        lit += albedo * vec3<f32>(0.7, 0.9, 0.3) * (0.08 + 0.4 * through) * day_factor() * (0.4 + 0.6 * sun);
    }
    if (is_kind(in.kind, GLOW)) {
        if (in.color.a < 0.999 && !home_light(in.color.a, globals.extra.y)) {
            // Lights out: just dark glass
            lit = sun_lighting(lin(vec3<f32>(0.2, 0.26, 0.3)), normal, sun);
        } else {
            // A faint inner light by day, shining out after dark, and slowly pulsing
            let night = 1.0 - day_factor();
            let pulse = 0.85 + 0.15 * sin(globals.wind_time * 1.3 + in.world_pos.x * 0.05 + in.world_pos.z * 0.07);
            lit = mix(lit, albedo * pulse * 2.5, 0.2 + 0.8 * night);
        }
    }
    if (is_kind(in.kind, PAINT)) {
        lit += gloss(in.world_pos, normal, sun);
    }
    // A wet road: dark, level surfaces take on a sheen of the grey sky at
    // a glancing angle (and headlights glare off them a little more)
    let wet = globals.screen.w;
    let c = in.color.rgb;
    let grey = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)) < 0.08;
    if (wet > 0.01 && is_kind(in.kind, PLAIN) && normal.y > 0.97 && grey && dot(c, vec3<f32>(0.333)) < 0.42) {
        let to_eye = normalize(globals.cam_pos - in.world_pos);
        let fresnel = pow(1.0 - max(dot(to_eye, normal), 0.0), 4.0);
        lit = mix(lit, haze_color(reflect(-to_eye, normal)) * 0.8, fresnel * 0.55 * wet);
        lit *= 1.0 - 0.15 * wet;
    }
    lit += (eval_lights(in.world_pos, normal, albedo) + eval_street(in.world_pos, normal, albedo)) * (1.0 + 0.5 * wet);
    lit = apply_fog(lit, in.world_pos);
    // A glowing window's alpha is its bedtime, not see-through
    return vec4<f32>(max(lit, vec3<f32>(0.0)), select(in.color.a, 1.0, is_kind(in.kind, GLOW)));
}
