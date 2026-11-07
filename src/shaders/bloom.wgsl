// Bloom: the bright parts of the frame (the sun, glints, lamps, flashes)
// downsampled through a mip chain, then blurred back up and added together.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;

fn tap(uv: vec2<f32>, dx: f32, dy: f32, texel: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(source, bilinear, uv + vec2<f32>(dx, dy) * texel, 0.0).rgb;
}

// 13 taps in overlapping boxes, which keeps small bright things from flickering
fn downsample(uv: vec2<f32>) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    let a = tap(uv, -2.0, -2.0, texel); let b = tap(uv, 0.0, -2.0, texel); let c = tap(uv, 2.0, -2.0, texel);
    let d = tap(uv, -2.0, 0.0, texel);  let e = tap(uv, 0.0, 0.0, texel);  let f = tap(uv, 2.0, 0.0, texel);
    let g = tap(uv, -2.0, 2.0, texel);  let h = tap(uv, 0.0, 2.0, texel);  let i = tap(uv, 2.0, 2.0, texel);
    let j = tap(uv, -1.0, -1.0, texel); let k = tap(uv, 1.0, -1.0, texel);
    let l = tap(uv, -1.0, 1.0, texel);  let m = tap(uv, 1.0, 1.0, texel);
    return e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
}

/// First step down from the frame: only what's brighter than anything lit
/// (a soft knee, so nothing switches on with a pop)
@fragment
fn fs_prefilter(in: ScreenOut) -> @location(0) vec4<f32> {
    let c = min(downsample(in.uv), vec3<f32>(60.0));
    let peak = max(c.r, max(c.g, c.b));
    // Sunlit paint reaches about 2, so only true lights clear this
    let threshold = 2.2;
    let knee = 0.8;
    let soft = clamp(peak - threshold + knee, 0.0, 2.0 * knee);
    let weight = max(soft * soft / (4.0 * knee + 1e-4), peak - threshold) / max(peak, 1e-4);
    return vec4<f32>(c * weight, 1.0);
}

@fragment
fn fs_down(in: ScreenOut) -> @location(0) vec4<f32> {
    return vec4<f32>(downsample(in.uv), 1.0);
}

/// 3x3 tent, added into the next mip up
@fragment
fn fs_up(in: ScreenOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    var c = tap(in.uv, 0.0, 0.0, texel) * 4.0;
    c += (tap(in.uv, -1.0, 0.0, texel) + tap(in.uv, 1.0, 0.0, texel) + tap(in.uv, 0.0, -1.0, texel) + tap(in.uv, 0.0, 1.0, texel)) * 2.0;
    c += tap(in.uv, -1.0, -1.0, texel) + tap(in.uv, 1.0, -1.0, texel) + tap(in.uv, -1.0, 1.0, texel) + tap(in.uv, 1.0, 1.0, texel);
    return vec4<f32>(c / 16.0, 1.0);
}
