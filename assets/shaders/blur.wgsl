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
    // 9-tap Gaussian kernel (unrolled — wgpu 0.19 naga forbids dynamic array indexing)
    let w0 = 0.2492;
    let w1 = 0.1836;
    let w2 = 0.1216;
    let w3 = 0.0540;
    let w4 = 0.0162;

    var color = textureSample(src_tex, src_sampler, in.uv) * w0;

    let off1 = bu.direction * 1.0;
    color += textureSample(src_tex, src_sampler, in.uv + off1) * w1;
    color += textureSample(src_tex, src_sampler, in.uv - off1) * w1;

    let off2 = bu.direction * 2.0;
    color += textureSample(src_tex, src_sampler, in.uv + off2) * w2;
    color += textureSample(src_tex, src_sampler, in.uv - off2) * w2;

    let off3 = bu.direction * 3.0;
    color += textureSample(src_tex, src_sampler, in.uv + off3) * w3;
    color += textureSample(src_tex, src_sampler, in.uv - off3) * w3;

    let off4 = bu.direction * 4.0;
    color += textureSample(src_tex, src_sampler, in.uv + off4) * w4;
    color += textureSample(src_tex, src_sampler, in.uv - off4) * w4;

    return color;
}
