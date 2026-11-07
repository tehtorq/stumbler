// The sun's shadow map, for the shaders that light the world.

@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

/// How much sun reaches `world_pos`: 0 in shadow, 1 in the open. Fades out
/// toward the edge of the map, so shadows never end in a line.
fn sun_reaches(world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    // Lifted off the surface, so a face doesn't shadow itself
    let p = globals.shadow_view_proj * vec4<f32>(world_pos + normal * 0.4, 1.0);
    let edge = max(abs(p.x), abs(p.y));
    if (edge >= 1.0 || p.z >= 1.0 || p.z <= 0.0) {
        return 1.0;
    }
    let uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    let texel = 1.0 / 2048.0;
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let angle = 0.785 + f32(i) * 1.5708;
        let offset = vec2<f32>(cos(angle), sin(angle)) * texel * 1.25;
        sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, p.z);
    }
    return mix(sum * 0.25, 1.0, smoothstep(0.82, 1.0, edge));
}
