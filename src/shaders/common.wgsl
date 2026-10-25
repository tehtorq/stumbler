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
        result += albedo * colint.xyz * colint.w * atten * ndotl;
    }
    return result;
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

fn day_factor() -> f32 {
    return clamp(globals.sun_dir.y * 3.0 + 0.3, 0.0, 1.0);
}

// Flat-shaded look: strong key light so facets read, cool sky fill from above.
fn sun_lighting(albedo: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let ndotl = max(dot(normal, globals.sun_dir), 0.0);
    let day = day_factor();
    let sun_color = mix(vec3<f32>(1.0, 0.75, 0.55), vec3<f32>(1.0, 0.97, 0.9), clamp(globals.sun_dir.y * 2.0, 0.0, 1.0));
    let sky_fill = mix(vec3<f32>(0.25, 0.28, 0.45), vec3<f32>(0.42, 0.46, 0.55), day);
    // Under rain cloud the sun goes in and the light flattens out
    let rain = globals.screen.z;
    let fill = sky_fill * (0.75 + 0.25 * normal.y) * (1.0 + 0.15 * rain);
    return albedo * (fill + sun_color * ndotl * 0.75 * day * (1.0 - 0.7 * rain));
}

// Exponential-squared distance fog toward the horizon colour.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let d = length(world_pos - globals.cam_pos) * globals.fog.a;
    let f = 1.0 - exp(-d * d);
    return mix(color, globals.fog.rgb, clamp(f, 0.0, 1.0));
}
