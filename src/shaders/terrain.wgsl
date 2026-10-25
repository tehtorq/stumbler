// Terrain: flat per-face colours, sun lighting, drifting cloud shadows, fog.

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) world_pos: vec3<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = globals.view_proj * vec4<f32>(in.pos, 1.0);
    out.color = in.color;
    out.world_pos = in.pos;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = in.color.rgb;
    let normal = flat_normal(in.world_pos);
    var lit = sun_lighting(albedo, normal);

    // Dynamic lights (headlights)
    lit += eval_lights(in.world_pos, normal, albedo);

    // Cloud shadows — drifting dark patches (daytime only)
    let xz = in.world_pos.xz;
    let cloud_u = xz.x * 0.0003 + globals.wind_time * 0.015;
    let cloud_v = xz.y * 0.0003 + globals.wind_time * 0.006;
    let cloud = sin(cloud_u * 6.28) * sin(cloud_v * 6.28)
              + sin(cloud_u * 12.56 + 1.0) * sin(cloud_v * 12.56 + 2.0) * 0.5;
    let cloud_shadow = smoothstep(0.1, 0.5, cloud) * 0.15 * day_factor();
    lit *= (1.0 - cloud_shadow);

    lit = apply_fog(lit, in.world_pos);
    return vec4<f32>(clamp(lit, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
