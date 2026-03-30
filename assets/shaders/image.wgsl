// GPU image rendering shader for Photo viewer
// Renders a textured quad with zoom/pan transform and transparency checkerboard

struct Uniforms {
    // Image rectangle in viewport-normalized UV [0,1]: (left, top, right, bottom)
    rect: vec4<f32>,
    // Background color
    bg_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var img_tex: texture_2d<f32>;
@group(0) @binding(2) var img_sampler: sampler;

struct VsOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    // Two-triangle fullscreen quad
    var positions = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0),  vec2(1.0, -1.0), vec2(1.0, 1.0),
    );
    var out: VsOutput;
    let p = positions[idx];
    out.pos = vec4(p, 0.0, 1.0);
    // Map clip-space to UV: [-1,1] -> [0,1], flip Y for image coords
    out.uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

fn checkerboard(pixel_pos: vec2<f32>) -> vec3<f32> {
    let grid = floor(pixel_pos / 10.0);
    let c = ((grid.x + grid.y) % 2.0 + 2.0) % 2.0;
    return mix(vec3(0.18), vec3(0.25), vec3(c));
}

@fragment
fn fs_main(input: VsOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    let r = uniforms.rect;

    // Outside image rectangle: draw background
    if uv.x < r.x || uv.x > r.z || uv.y < r.y || uv.y > r.w {
        return uniforms.bg_color;
    }

    // Map viewport UV -> image texture UV [0,1]
    let img_uv = (uv - r.xy) / (r.zw - r.xy);
    let color = textureSample(img_tex, img_sampler, img_uv);

    // Composite transparent pixels over checkerboard
    if color.a < 1.0 {
        let checker = checkerboard(input.pos.xy);
        return vec4(mix(checker, color.rgb, color.a), 1.0);
    }

    return color;
}
