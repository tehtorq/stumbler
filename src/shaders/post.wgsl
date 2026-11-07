// Final pass: the linear HDR frame, darkened in its creases (AO), with bloom
// added, tonemapped, lightly graded and encoded for the display.

@group(0) @binding(1) var hdr_tex: texture_2d<f32>;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var ao_tex: texture_2d<f32>;
@group(0) @binding(5) var bilinear: sampler;

const BLOOM: f32 = 0.12;
const AO_STRENGTH: f32 = 0.85;

/// The half-res AO, blurred over its neighbours at a similar distance
/// (the AO target holds the distance alongside)
fn ambient_occlusion(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(ao_tex));
    let at = px / 2;
    let centre = textureLoad(ao_tex, min(at, size - 1), 0).g;
    var sum = 0.0;
    var weight = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let s = textureLoad(ao_tex, clamp(at + vec2<i32>(x, y), vec2<i32>(0), size - 1), 0).rg;
            let w = 1.0 / (1.0 + abs(s.y - centre) / (centre * 0.02 + 0.05) * 4.0);
            sum += s.x * w;
            weight += w;
        }
    }
    let ao = sum / weight;
    // Fades out into the distance, where the fog takes over
    return mix(ao, 1.0, smoothstep(250.0, 600.0, centre));
}

/// The shoulder of Khronos PBR Neutral (without its toe, which crushes dark
/// channels and oversaturates): colours stay as authored up to near white,
/// then highlights roll off gently instead of clipping.
fn neutral_tonemap(color: vec3<f32>) -> vec3<f32> {
    let start = 0.8;
    let desaturation = 0.08;
    var c = color;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c *= new_peak / peak;
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

@fragment
fn fs_main(in: ScreenOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    var color = textureLoad(hdr_tex, px, 0).rgb;
    let ao = ambient_occlusion(px);
    color *= mix(1.0, ao, AO_STRENGTH);
    color += textureSampleLevel(bloom_tex, bilinear, in.uv, 0.0).rgb * BLOOM;
    // Eye adjusting to the high sun, as the midday light would otherwise
    // wash everything out
    let exposure = mix(1.0, 0.7, smoothstep(0.3, 0.95, globals.sun_dir.y));
    color = neutral_tonemap(color * exposure);

    // Gentle vignette pulls the eye to the middle
    let centred = in.uv - 0.5;
    color *= 1.0 - dot(centred, centred) * 0.35;

    var display = pow(color, vec3<f32>(1.0 / 2.2));
    // Dither away banding in the sky's long gradients
    let noise = fract(52.9829189 * fract(dot(in.clip.xy, vec2<f32>(0.06711056, 0.00583715))));
    display += (noise - 0.5) / 255.0;
    return vec4<f32>(display, 1.0);
}
