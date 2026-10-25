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
/// Ground cover sinks away between these distances from the camera
/// (the chunk streamer loads it out to COVER_RANGE, a little beyond).
const COVER_FADE: vec2<f32> = vec2<f32>(560.0, 740.0);

fn is_kind(kind: f32, k: f32) -> bool {
    return abs(kind - k) < 0.5;
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

    // Water swells out in the deep and lies still at the shore
    if (is_kind(kind, WATER)) {
        world_pos.y += wave_height(world_pos.xz, globals.wind_time) * 0.7 * in.color.a;
    }

    var out: VertexOut;
    out.clip = globals.view_proj * world_pos;
    out.color = vec4<f32>(in.color.rgb, select(in.color.a, 1.0, foliage));
    out.world_pos = world_pos.xyz;
    out.kind = kind;
    return out;
}

/// Water surface: deep colour, foam at the shore, sky reflected at shallow
/// viewing angles, and the sun glinting off wave facets.
fn shade_water(in: VertexOut, normal: vec3<f32>) -> vec4<f32> {
    let depth = in.color.a;
    let day = day_factor();
    var lit = sun_lighting(in.color.rgb, normal);

    let to_eye = normalize(globals.cam_pos - in.world_pos);
    let facing = max(dot(normal, to_eye), 0.0);
    let fresnel = pow(1.0 - facing, 4.0);
    lit = mix(lit, globals.fog.rgb, fresnel * 0.6);

    // Each wave facet catches the sun on its own, so the sun's path sparkles
    let glint = pow(max(dot(reflect(-globals.sun_dir, normal), to_eye), 0.0), 260.0);
    lit += vec3<f32>(1.0, 0.95, 0.85) * glint * 1.4 * day;

    // Foam laps along the waterline, coming and going
    let lap = 0.55 + 0.45 * sin(globals.wind_time * 1.7 + in.world_pos.x * 0.21 + in.world_pos.z * 0.17);
    let foam = (1.0 - smoothstep(0.0, 0.02, depth)) * 0.45 * lap;
    lit = mix(lit, vec3<f32>(0.85, 0.9, 0.92) * (0.55 + 0.45 * day), foam);

    // Clear in the shallows, opaque out in the deep
    let alpha = mix(0.32, 0.86, smoothstep(0.02, 0.7, depth)) + foam * 0.4 + fresnel * 0.1;
    lit += eval_lights(in.world_pos, normal, in.color.rgb);
    lit = apply_fog(lit, in.world_pos);
    return vec4<f32>(clamp(lit, vec3<f32>(0.0), vec3<f32>(1.0)), clamp(alpha, 0.0, 1.0));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (is_kind(in.kind, PARTICLE)) {
        let light = max(in.color.r, max(in.color.g, in.color.b)) > 1.0;
        let tone = select(in.color.rgb * (0.3 + 0.7 * day_factor()), min(in.color.rgb, vec3<f32>(1.0)), light);
        return vec4<f32>(apply_fog(tone, in.world_pos), in.color.a);
    }
    let normal = flat_normal(in.world_pos);
    if (is_kind(in.kind, WATER)) {
        return shade_water(in, normal);
    }
    var lit = sun_lighting(in.color.rgb, normal);
    if (is_kind(in.kind, FOLIAGE) || is_kind(in.kind, COVER)) {
        // Leaves let some sunlight through, so backlit trees glow a little
        // instead of going black
        let through = max(dot(-normal, globals.sun_dir), 0.0);
        lit += in.color.rgb * vec3<f32>(0.85, 0.95, 0.55) * (0.15 + 0.35 * through) * day_factor();
    }
    if (is_kind(in.kind, GLOW)) {
        // A faint inner light by day, shining out after dark, and slowly pulsing
        let night = 1.0 - day_factor();
        let pulse = 0.85 + 0.15 * sin(globals.wind_time * 1.3 + in.world_pos.x * 0.05 + in.world_pos.z * 0.07);
        lit = mix(lit, in.color.rgb * pulse * 1.15, 0.2 + 0.8 * night);
    }
    // A wet road: dark, level surfaces take on a sheen of the grey sky at
    // a glancing angle (and headlights glare off them a little more)
    let wet = globals.screen.w;
    let c = in.color.rgb;
    let grey = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)) < 0.08;
    if (wet > 0.01 && is_kind(in.kind, PLAIN) && normal.y > 0.97 && grey && dot(c, vec3<f32>(0.333)) < 0.42) {
        let to_eye = normalize(globals.cam_pos - in.world_pos);
        let fresnel = pow(1.0 - max(dot(to_eye, normal), 0.0), 4.0);
        lit = mix(lit, globals.fog.rgb * 0.9, fresnel * 0.55 * wet);
        lit *= 1.0 - 0.15 * wet;
    }
    lit += eval_lights(in.world_pos, normal, in.color.rgb) * (1.0 + 0.5 * wet);
    lit = apply_fog(lit, in.world_pos);
    return vec4<f32>(clamp(lit, vec3<f32>(0.0), vec3<f32>(1.0)), in.color.a);
}
