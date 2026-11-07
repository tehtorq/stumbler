// Sky gradient with sun disc, sunset tint and stars, in linear HDR. Fullscreen triangle, no vertex buffer.

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) screen: vec2<f32>,
    @location(1) ray_dir: vec3<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOut {
    // Oversized triangle covering the screen
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    var out: VertexOut;
    // On the far plane: drawn after the world, it only fills what's left uncovered
    out.clip = vec4<f32>(xy, 1.0, 1.0);
    out.screen = xy;

    // Unproject two points at different depths to get a world-space ray direction
    let near = globals.inv_view_proj * vec4<f32>(xy, 0.0, 1.0);
    let far = globals.inv_view_proj * vec4<f32>(xy, 1.0, 1.0);
    out.ray_dir = far.xyz / far.w - near.xyz / near.w;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let dir = normalize(in.ray_dir);
    // t: 0 at horizon, 1 at zenith (based on ray Y direction)
    let t = clamp(dir.y * 2.0, 0.0, 1.0);

    let sun_scr = globals.sun_screen.xy;
    let sun_elev = globals.sun_screen.z;

    // Day/night
    let day = smoothstep(-0.3, 0.3, sun_elev);

    // Sky colors. The horizon is the fog colour (computed on the CPU) so
    // distant terrain melts into the sky.
    let day_zenith = vec3<f32>(0.25, 0.45, 0.85);
    let sunset_factor = smoothstep(0.5, 0.0, abs(sun_elev)) * step(0.0, sun_elev + 0.2);
    let sunset_zenith = vec3<f32>(0.4, 0.3, 0.6);
    let night_zenith = vec3<f32>(0.02, 0.02, 0.08);

    var zenith = mix(night_zenith, day_zenith, day);
    zenith = mix(zenith, sunset_zenith, sunset_factor);
    // Rain cloud: a flat grey sky
    let rain = globals.screen.z;
    zenith = mix(zenith, globals.fog.rgb * 0.9, rain);
    // Graded by eye as before, then into linear light, with the haze's warm
    // glow toward the sun added low down
    var sky = lin(mix(globals.fog.rgb, zenith, pow(t, 0.6))) + (haze_color(dir) - lin(globals.fog.rgb)) * (1.0 - t);

    // Sun disc at projected screen position: far brighter than anything
    // lit, so it blooms
    let sun_dist = length(in.screen - sun_scr);

    if (sun_scr.y > -1.5 && rain < 0.6) {
        let sun_disc = smoothstep(0.1, 0.04, sun_dist);
        let disc_color = mix(vec3<f32>(1.0, 0.5, 0.15), vec3<f32>(1.0, 0.93, 0.8), day);
        sky = mix(sky, disc_color * 14.0, sun_disc * (1.0 - rain / 0.6));

        let glow = exp(-sun_dist * sun_dist * 8.0) * 0.5 * day;
        let warm_glow = exp(-sun_dist * sun_dist * 3.0) * 0.35 * sunset_factor;
        sky += vec3<f32>(0.85, 0.7, 0.4) * glow;
        sky += vec3<f32>(1.0, 0.25, 0.05) * warm_glow;
    }

    // Stars at night — spherical mapping on world ray direction
    if (day < 0.3 && rain < 0.5) {
        let star_intensity = 1.0 - day / 0.3;
        let azimuth = atan2(dir.x, dir.z);
        let elev = asin(clamp(dir.y, -1.0, 1.0));
        // Sparse grid cells, each holding at most one small square star
        let cell = vec2<f32>(azimuth * 120.0, elev * 240.0);
        let sp = floor(cell);
        let star = fract(sin(dot(sp, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        let in_cell = abs(fract(cell) - 0.5);
        if (star > 0.97 && max(in_cell.x, in_cell.y) < 0.18) {
            let brightness = fract(sin(dot(sp, vec2<f32>(93.97, 41.23))) * 31415.9);
            sky += vec3<f32>(star_intensity * (0.2 + 1.3 * brightness * brightness));
        }
    }

    return vec4<f32>(max(sky, vec3<f32>(0.0)), 1.0);
}
