// -- Uniforms --

struct Uniforms {
    rect: vec4<f32>,
    bg_color: vec4<f32>,
    // Adjustments (normalized: -1..+1 except exposure which is -5..+5)
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    vibrance: f32,
    saturation: f32,
    clarity: f32,
    dehaze: f32,
    // Padding to align to 16 bytes
    _pad0: f32,
    _pad1: f32,
    // Temperature/tint: 3x3 Bradford CAT matrix (row-major, 3 vec4 padded)
    temp_mat_row0: vec4<f32>,
    temp_mat_row1: vec4<f32>,
    temp_mat_row2: vec4<f32>,
    // Lens correction coefficients
    lens_enabled: f32,         // 0.0 or 1.0
    lens_dist_a: f32,
    lens_dist_b: f32,
    lens_dist_c: f32,
    lens_vig_k1: f32,
    lens_vig_k2: f32,
    lens_vig_k3: f32,
    lens_tca_r_scale: f32,
    lens_tca_b_scale: f32,
    image_aspect: f32,         // width / height for lens correction coords
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var img_tex: texture_2d<f32>;
@group(0) @binding(2) var img_sampler: sampler;
@group(0) @binding(3) var blur_tex: texture_2d<f32>;

// -- Vertex shader --

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    // Full-screen quad from 6 vertices
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

// -- Color math helpers --

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { return c / 12.92; }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { return c * 12.92; }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn lum(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3(0.2126, 0.7152, 0.0722));
}

fn smooth_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// -- Lens correction helpers --

fn apply_distortion(uv: vec2<f32>, center: vec2<f32>) -> vec2<f32> {
    if u.lens_enabled < 0.5 { return uv; }
    let d = uv - center;
    let r = length(d);
    let r2 = r * r;
    let r3 = r2 * r;
    let a = u.lens_dist_a;
    let b = u.lens_dist_b;
    let c = u.lens_dist_c;
    let scale = a * r3 + b * r2 + c * r + 1.0 - a - b - c;
    return center + d * scale;
}

fn apply_tca(uv: vec2<f32>, center: vec2<f32>) -> vec3<f32> {
    if u.lens_enabled < 0.5 {
        let col = textureSample(img_tex, img_sampler, uv);
        return col.rgb;
    }
    let d = uv - center;
    let uv_r = center + d * u.lens_tca_r_scale;
    let uv_b = center + d * u.lens_tca_b_scale;
    let r = textureSample(img_tex, img_sampler, uv_r).r;
    let g = textureSample(img_tex, img_sampler, uv).g;
    let b = textureSample(img_tex, img_sampler, uv_b).b;
    return vec3(r, g, b);
}

fn apply_vignette(px: vec3<f32>, uv: vec2<f32>, center: vec2<f32>) -> vec3<f32> {
    if u.lens_enabled < 0.5 { return px; }
    let d = uv - center;
    let r2 = dot(d, d);
    let r4 = r2 * r2;
    let r6 = r4 * r2;
    let correction = 1.0 + u.lens_vig_k1 * r2 + u.lens_vig_k2 * r4 + u.lens_vig_k3 * r6;
    return px * correction;
}

// -- Fragment shader --

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let rect = u.rect;

    // Outside image rect: background
    if uv.x < rect.x || uv.x > rect.z || uv.y < rect.y || uv.y > rect.w {
        return u.bg_color;
    }

    // Map viewport UV to texture UV
    var tex_uv = (uv - rect.xy) / (rect.zw - rect.xy);
    let center = vec2(0.5, 0.5);

    // Lens distortion (UV remapping)
    tex_uv = apply_distortion(tex_uv, center);

    // Sample with TCA correction (per-channel UV)
    var rgb = apply_tca(tex_uv, center);
    let alpha = textureSample(img_tex, img_sampler, tex_uv).a;

    // Linearize
    var px = vec3(srgb_to_linear(rgb.r), srgb_to_linear(rgb.g), srgb_to_linear(rgb.b));

    // Exposure: pixel * 2^EV
    px = px * pow(2.0, u.exposure);

    // Temperature/Tint: Bradford CAT matrix multiply
    let temp_mat = mat3x3<f32>(
        u.temp_mat_row0.xyz,
        u.temp_mat_row1.xyz,
        u.temp_mat_row2.xyz,
    );
    px = temp_mat * px;

    // Zone-based tone adjustments (stop-based).
    // Zone weights target perceptual (gamma 2.2) luminance ranges.
    // Delta is in photographic stops: px *= 2^stops.
    // This bounds the effect uniformly — 1.5 stops max per slider regardless
    // of pixel brightness, preventing overblown darks.
    if u.highlights != 0.0 || u.shadows != 0.0 || u.whites != 0.0 || u.blacks != 0.0 {
        let L_lin = lum(px);
        if L_lin > 0.0001 {
            let L_p = pow(L_lin, 1.0 / 2.2);

            // Shadows: peaks ~0.20-0.25, fades by ~0.65
            let sh_rise = smooth_step(0.0, 0.20, L_p);
            let sh_fall = 1.0 - smooth_step(0.25, 0.65, L_p);
            let w_sh = sh_rise * sh_fall;

            // Highlights: rises from ~0.35, full above ~0.75
            let w_hi = smooth_step(0.35, 0.75, L_p);

            // Blacks: concentrated endpoint control, fades by ~0.15
            let t_bk = 1.0 - smooth_step(0.0, 0.15, L_p);
            let w_bk = t_bk * t_bk;

            // Whites: endpoint control, quadratic rise from ~0.75
            let t_wh = smooth_step(0.75, 1.0, L_p);
            let w_wh = t_wh * t_wh;

            // Stop-change: max ±1.5 stops per slider at full value
            let stops = u.shadows * w_sh * 1.5
                      + u.highlights * w_hi * 1.5
                      + u.blacks * w_bk * 1.5
                      + u.whites * w_wh * 1.5;

            px = px * pow(2.0, stops);
        }
    }

    // Contrast: sigmoid S-curve blend (k > 4 ensures proper contrast boost)
    let l2 = lum(px);
    if l2 > 0.0 && u.contrast != 0.0 {
        let k = 4.0 + abs(u.contrast) * 8.0;
        let sig = 1.0 / (1.0 + exp(-k * (l2 - 0.5)));
        let l_adj = l2 + u.contrast * (sig - l2);
        px = px * (l_adj / l2);
    }

    // Vibrance
    if u.vibrance != 0.0 {
        let mx = max(px.r, max(px.g, px.b));
        let mn = min(px.r, min(px.g, px.b));
        let sat = select(0.0, (mx - mn) / mx, mx > 0.0);
        let weight = 1.0 + u.vibrance * (1.0 - sat);
        let lv = lum(px);
        px = vec3(lv) + (px - vec3(lv)) * weight;
    }

    // Saturation
    if u.saturation != 0.0 {
        let ls = lum(px);
        px = mix(vec3(ls), px, 1.0 + u.saturation);
    }

    // Clarity (local contrast from blur texture)
    if u.clarity != 0.0 {
        let blur_uv = (uv - rect.xy) / (rect.zw - rect.xy);
        let blur_sample = textureSample(blur_tex, img_sampler, blur_uv).rgb;
        let blur_lin = vec3(srgb_to_linear(blur_sample.r), srgb_to_linear(blur_sample.g), srgb_to_linear(blur_sample.b));
        let lc = lum(px);
        let midtone = smooth_step(0.0, 0.5, lc) * (1.0 - smooth_step(0.5, 1.0, lc));
        px += u.clarity * (px - blur_lin) * midtone;
    }

    // Dehaze
    if u.dehaze != 0.0 {
        let blur_uv2 = (uv - rect.xy) / (rect.zw - rect.xy);
        let blur_s = textureSample(blur_tex, img_sampler, blur_uv2).rgb;
        let blur_l = vec3(srgb_to_linear(blur_s.r), srgb_to_linear(blur_s.g), srgb_to_linear(blur_s.b));
        let atmos = max(max(blur_l.r, blur_l.g), max(blur_l.b, 0.01));
        let dark = min(px.r, min(px.g, px.b));
        let t = max(1.0 - u.dehaze * dark / atmos, 0.1);
        px = (px - vec3(atmos)) / t + vec3(atmos);
    }

    // Lens vignetting correction
    px = apply_vignette(px, tex_uv, center);

    // Clamp and gamma encode
    px = clamp(px, vec3(0.0), vec3(1.0));
    let srgb = vec3(linear_to_srgb(px.r), linear_to_srgb(px.g), linear_to_srgb(px.b));

    // Alpha compositing (checkerboard for transparency)
    if alpha < 1.0 {
        let checker_size = 10.0;
        let pos = in.pos.xy;
        let checker = select(0.18, 0.25,
            (floor(pos.x / checker_size) + floor(pos.y / checker_size)) % 2.0 < 1.0);
        let bg = vec3(checker);
        return vec4(mix(bg, srgb, alpha), 1.0);
    }

    return vec4(srgb, 1.0);
}
