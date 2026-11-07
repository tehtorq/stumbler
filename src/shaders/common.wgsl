// Shared by every shader: per-frame globals (group 0) and dynamic lights.
// Must match `gpu::Globals` on the Rust side.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec3<f32>,
    wind_time: f32,
    cam_pos: vec3<f32>,
    light_count: u32,
    sun_screen: vec3<f32>, // xy = sun NDC position, z = sun elevation
    sway_count: u32,
    sway: array<vec4<f32>, 16>,  // per tree hit: (x, z, base y, bend), (dir x, dir z, -, -)
    lights: array<vec4<f32>, 24>, // 3 vec4s per light: pos+radius, dir+cos, color+intensity
    fog: vec4<f32>,    // rgb = horizon colour, a = density
    screen: vec4<f32>, // xy = framebuffer size in pixels, z = rain, w = road wetness
    shadow_view_proj: mat4x4<f32>, // world to the sun's shadow map
    crowd: vec4<f32>, // xy = the car they're cheering, z = excitement 0..1, w = spare
    street: array<vec4<f32>, 32>, // nearest streetlights: lamp xyz, w = how far on
    street_count: u32,
    post: vec4<f32>,  // AO strength, bloom strength, exposure, vignette
    extra: vec4<f32>, // x = sun shadows on
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
}

fn eval_lights(frag_pos: vec3<f32>, normal: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    for (var i = 0u; i < globals.light_count; i++) {
        let posrad = globals.lights[i * 3u];
        let dircos = globals.lights[i * 3u + 1u];
        let colint = globals.lights[i * 3u + 2u];
        let to_light = posrad.xyz - frag_pos;
        let dist = length(to_light);
        if (dist > posrad.w) { continue; }
        let l = to_light / max(dist, 0.1);
        var atten = 1.0 - dist / posrad.w;
        atten = atten * atten;
        if (dircos.w > 0.01) {
            let spot_cos = dot(-l, dircos.xyz);
            if (spot_cos < dircos.w) { continue; }
            atten *= clamp((spot_cos - dircos.w) / (1.0 - dircos.w) * 2.0, 0.0, 1.0);
        }
        // Use half-lambert for softer, more visible ground lighting
        let ndotl = dot(normal, l) * 0.5 + 0.5;
        result += albedo * lin(colint.xyz) * colint.w * 1.6 * atten * ndotl;
    }
    return result;
}

/// Light from the streetlights: warm, falling off with distance like a real
/// lamp (inverse square, eased to nothing at its reach), and thrown mostly
/// downward by the lamp's reflector, so the ground beneath is brightest.
fn eval_street(frag_pos: vec3<f32>, normal: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let reach = 55.0;
    let warm = vec3<f32>(1.0, 0.78, 0.52);
    var sum = 0.0;
    for (var i = 0u; i < globals.street_count; i++) {
        let lamp = globals.street[i];
        let to_lamp = lamp.xyz - frag_pos;
        let d2 = dot(to_lamp, to_lamp);
        if (d2 > reach * reach) { continue; }
        let d = sqrt(d2);
        let l = to_lamp / max(d, 0.01);
        let window = 1.0 - d2 / (reach * reach);
        let falloff = window * window / (1.0 + d2 / 64.0);
        // l.y is how far below the lamp this is, as a share of the distance
        let downward = smoothstep(-0.05, 0.55, l.y);
        sum += max(dot(normal, l), 0.0) * falloff * downward * lamp.w;
    }
    return albedo * warm * sum * 28.0;
}

// Flat face normal from screen-space derivatives, flipped to face the camera.
// (Flipping on n.y flickered on vertical faces where n.y is ~0.)
fn flat_normal(world_pos: vec3<f32>) -> vec3<f32> {
    let raw_n = cross(dpdy(world_pos), dpdx(world_pos));
    let nlen = length(raw_n);
    var n = select(vec3<f32>(0.0, 1.0, 0.0), raw_n / nlen, nlen > 0.001);
    if (dot(n, globals.cam_pos - world_pos) < 0.0) { n = -n; }
    return n;
}

/// 1 by day, 0 at night, easing through a long dusk. Must match
/// `Stage::day_factor` on the CPU.
fn day_factor() -> f32 {
    return smoothstep(-0.3, 0.5, globals.sun_dir.y);
}

/// Colours are authored by eye (sRGB); lighting happens in linear light.
fn lin(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

/// The sun's colour, warmer as it gets low.
fn sun_color() -> vec3<f32> {
    return mix(vec3<f32>(1.0, 0.52, 0.28), vec3<f32>(1.0, 0.95, 0.87), clamp(globals.sun_dir.y * 2.5, 0.0, 1.0));
}

/// Drifting cloud shadow over the land: 1 in the open, less under cloud.
fn cloud_light(xz: vec2<f32>) -> f32 {
    let cloud_u = xz.x * 0.0003 + globals.wind_time * 0.015;
    let cloud_v = xz.y * 0.0003 + globals.wind_time * 0.006;
    let cloud = sin(cloud_u * 6.28) * sin(cloud_v * 6.28)
              + sin(cloud_u * 12.56 + 1.0) * sin(cloud_v * 12.56 + 2.0) * 0.5;
    return 1.0 - smoothstep(0.1, 0.5, cloud) * 0.45 * (1.0 - globals.screen.z);
}

// Flat-shaded look in linear light: a strong warm key so facets read, and a
// hemisphere fill (blue sky above, warm bounce off the ground below).
// `sun` is how much of it gets here (see `sun_reaches`). Albedo is linear.
fn sun_lighting(albedo: vec3<f32>, normal: vec3<f32>, sun: f32) -> vec3<f32> {
    let ndotl = max(dot(normal, globals.sun_dir), 0.0);
    let day = day_factor();
    let rain = globals.screen.z;
    let sky = mix(vec3<f32>(0.035, 0.045, 0.12), vec3<f32>(0.085, 0.115, 0.21), day);
    let ground = mix(vec3<f32>(0.015, 0.015, 0.03), vec3<f32>(0.06, 0.055, 0.045), day);
    let up = normal.y * 0.5 + 0.5;
    // Under rain cloud the sun goes in and the light flattens out
    let fill = mix(ground, sky, up) * (1.0 + 1.2 * rain);
    let direct = sun_color() * ndotl * 1.85 * day * (1.0 - 0.75 * rain) * sun;
    return albedo * (fill + direct);
}

/// Colour of the haze looking along `dir`: the horizon, glowing warm toward the sun.
fn haze_color(dir: vec3<f32>) -> vec3<f32> {
    let toward_sun = max(dot(dir, globals.sun_dir), 0.0);
    let glow = pow(toward_sun, 6.0) * 0.35 * day_factor() * (1.0 - globals.screen.z);
    return lin(globals.fog.rgb) + sun_color() * glow;
}

// Exponential-squared distance fog toward the horizon colour.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let d = length(world_pos - globals.cam_pos) * globals.fog.a;
    // Squared again: mixing in linear light hazes more than it looks
    let f = 1.0 - exp(-d * d);
    let haze = f * f;
    return mix(color, haze_color(normalize(world_pos - globals.cam_pos)), clamp(haze, 0.0, 1.0));
}
