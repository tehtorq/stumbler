// Screen-space ambient occlusion at half resolution: creases, the feet of
// walls and trees, and the ground under cars darken a little. The world's
// depth is first copied down to half resolution (one sample of the
// multisampled buffer per texel), which is far less to read on integrated GPUs.

@group(0) @binding(4) var depth_tex: texture_depth_multisampled_2d;
@group(0) @binding(6) var half_depth: texture_2d<f32>;

const AO_SAMPLES: i32 = 10;
const AO_RADIUS: f32 = 2.5;
/// Distance written for the sky, so nothing blurs into it.
const SKY_DISTANCE: f32 = 60000.0;

@fragment
fn fs_half_depth(in: ScreenOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy) * 2;
    let size = vec2<i32>(textureDimensions(depth_tex));
    return vec4<f32>(textureLoad(depth_tex, min(px, size - 1), 0), 0.0, 0.0, 1.0);
}

fn depth_at(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(half_depth));
    return textureLoad(half_depth, clamp(px, vec2<i32>(0), size - 1), 0).r;
}

fn world_at(px: vec2<i32>, depth: f32) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(half_depth));
    let uv = (vec2<f32>(px) + 0.5) / size;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let p = globals.inv_view_proj * ndc;
    return p.xyz / p.w;
}

@fragment
fn fs_main(in: ScreenOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(half_depth));
    let px = vec2<i32>(in.clip.xy);
    let depth = depth_at(px);
    if (depth >= 1.0) {
        return vec4<f32>(1.0, SKY_DISTANCE, 0.0, 1.0);
    }
    let p = world_at(px, depth);
    // Face normal from the neighbouring pixels (the world is flat-shaded anyway)
    let dx = world_at(px + vec2<i32>(1, 0), depth_at(px + vec2<i32>(1, 0))) - p;
    let dy = world_at(px + vec2<i32>(0, 1), depth_at(px + vec2<i32>(0, 1))) - p;
    let dx2 = p - world_at(px - vec2<i32>(1, 0), depth_at(px - vec2<i32>(1, 0)));
    let dy2 = p - world_at(px - vec2<i32>(0, 1), depth_at(px - vec2<i32>(0, 1)));
    // Take the smaller step either side, so silhouettes don't smear the normal
    let ex = select(dx2, dx, dot(dx, dx) < dot(dx2, dx2));
    let ey = select(dy2, dy, dot(dy, dy) < dot(dy2, dy2));
    var n = normalize(cross(ey, ex));
    let to_cam = globals.cam_pos - p;
    if (dot(n, to_cam) < 0.0) { n = -n; }

    let dist = length(to_cam);
    // A little wider far off, so the distance still has some shape
    let radius = AO_RADIUS * (1.0 + dist * 0.004);
    let pixel_radius = min(radius * 1.5 * size.y * 0.5 / dist, 45.0);
    if (pixel_radius < 1.0) {
        return vec4<f32>(1.0, dist, 0.0, 1.0);
    }
    // Interleaved gradient noise turns the spiral per pixel; the blur in the
    // final pass smooths it out
    let noise = fract(52.9829189 * fract(dot(vec2<f32>(px), vec2<f32>(0.06711056, 0.00583715))));
    var occlusion = 0.0;
    for (var i = 0; i < AO_SAMPLES; i++) {
        let f = (f32(i) + 0.5) / f32(AO_SAMPLES);
        let angle = (f * 7.0 + noise) * 6.2832;
        let offset = vec2<f32>(cos(angle), sin(angle)) * pixel_radius * f;
        let q_px = px + vec2<i32>(offset);
        let q = world_at(q_px, depth_at(q_px));
        let v = q - p;
        let vv = dot(v, v);
        // Height above the surface less a few centimetres, so paint and
        // kerbs laid just proud of the road don't draw a halo
        let cos_term = max((dot(v, n) - 0.06) * inverseSqrt(max(vv, 1e-4)) - 0.1, 0.0);
        let falloff = max(1.0 - vv / (radius * radius), 0.0);
        occlusion += cos_term * falloff;
    }
    let ao = pow(clamp(1.0 - occlusion * 2.5 / f32(AO_SAMPLES), 0.0, 1.0), 1.5);
    return vec4<f32>(ao, dist, 0.0, 1.0);
}
