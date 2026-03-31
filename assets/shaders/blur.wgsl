struct BlurUniforms {
    direction: vec2<f32>,  // (1/width, 0) for horizontal, (0, 1/height) for vertical
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> bu: BlurUniforms;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0,  1.0), vec2(1.0, -1.0), vec2(1.0,  1.0),
    );
    let p = positions[vi];
    var out: VertexOutput;
    out.pos = vec4(p, 0.0, 1.0);
    out.uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

// 9-tap Gaussian kernel (sigma ~2.5, radius 4)
// Weights: 0.0162, 0.0540, 0.1216, 0.1836, 0.2492, 0.1836, 0.1216, 0.0540, 0.0162
// At 1/4 res, this gives an effective radius of ~16px on the original.

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let weights = array<f32, 5>(0.2492, 0.1836, 0.1216, 0.0540, 0.0162);
    var color = textureSample(src_tex, src_sampler, in.uv) * weights[0];
    for (var i = 1u; i < 5u; i++) {
        let offset = bu.direction * f32(i);
        color += textureSample(src_tex, src_sampler, in.uv + offset) * weights[i];
        color += textureSample(src_tex, src_sampler, in.uv - offset) * weights[i];
    }
    return color;
}
