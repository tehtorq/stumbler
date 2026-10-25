// Screen-space overlay. Vertex positions are in pixels from the top-left.

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let ndc = in.pos.xy / globals.screen.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
