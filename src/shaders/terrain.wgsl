// Terrain: flat per-face colours, sun lighting, drifting cloud shadows, fog. Outputs linear HDR.

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
    let albedo = lin(in.color.rgb);
    let normal = flat_normal(in.world_pos);
    let sun = sun_reaches(in.world_pos, normal) * cloud_light(in.world_pos.xz);
    var lit = sun_lighting(albedo, normal, sun);

    // Dynamic lights (headlights)
    lit += eval_lights(in.world_pos, normal, albedo);

    lit = apply_fog(lit, in.world_pos);
    return vec4<f32>(max(lit, vec3<f32>(0.0)), 1.0);
}
