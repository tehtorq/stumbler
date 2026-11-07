// Depth only, from the sun: what casts the shadows.

struct Object {
    model: mat4x4<f32>,
    params: vec4<f32>,
}

@group(1) @binding(0) var<uniform> object: Object;

@vertex
fn vs_main(in: VertexIn) -> @builtin(position) vec4<f32> {
    return globals.shadow_view_proj * object.model * vec4<f32>(in.pos, 1.0);
}
