//! Image editing state and undo/redo history.
//! All adjustment math lives here — both the data model and CPU-side
//! processing for full-resolution save.

// -- Data model --

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EditState {
    pub exposure: f32,    // -3.0 to +3.0 (stops)
    pub contrast: f32,    // -50 to +50
    pub highlights: f32,  // -100 to +100
    pub shadows: f32,     // -100 to +100
    pub whites: f32,      // -100 to +100
    pub blacks: f32,      // -100 to +100
    pub temperature: f32, // -30 to +30
    pub tint: f32,        // -30 to +30
    pub vibrance: f32,    // -50 to +50
    pub saturation: f32,  // -50 to +50
    pub clarity: f32,     // -50 to +50
    pub dehaze: f32,      // -50 to +50
    pub lens_correction: bool,
}

impl EditState {
    /// Returns true if all adjustments are at their defaults (no edits).
    #[cfg(test)]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug)]
pub struct UndoHistory {
    undo_stack: Vec<EditState>,
    redo_stack: Vec<EditState>,
    /// The last committed (stable) state. On commit(), this is pushed to the
    /// undo stack and then updated to `current`.
    committed: EditState,
    pub current: EditState,
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            committed: EditState::default(),
            current: EditState::default(),
        }
    }

    /// Call when a slider drag ends. Pushes the pre-edit (committed) state to
    /// the undo stack and marks the current state as the new committed baseline.
    pub fn commit(&mut self) {
        self.undo_stack.push(self.committed);
        self.committed = self.current;
        self.redo_stack.clear();
    }

    /// Undo: restore previous state. Returns true if undo was performed.
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.committed);
            self.committed = prev;
            self.current = prev;
            true
        } else {
            false
        }
    }

    /// Redo: restore next state. Returns true if redo was performed.
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.committed);
            self.committed = next;
            self.current = next;
            true
        } else {
            false
        }
    }

    /// Reset all adjustments to default. This is an undoable action.
    pub fn reset_all(&mut self) {
        self.undo_stack.push(self.committed);
        self.redo_stack.clear();
        self.committed = EditState::default();
        self.current = EditState::default();
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

use std::path::{Path, PathBuf};

// -- sRGB <-> linear conversion --

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

// -- Per-pixel adjustments (linear RGB) --

pub fn apply_exposure(px: [f32; 3], ev: f32) -> [f32; 3] {
    let m = 2.0_f32.powf(ev);
    [px[0] * m, px[1] * m, px[2] * m]
}

/// Zone-based tone adjustments (stop-based, ±2 stops max per slider).
/// Matches darktable tone equalizer's ±2 stop clamp (correction 0.25x-4.0x).
/// Zone weights in perceptual (gamma 2.2) luminance space with overlapping
/// smoothstep transitions (analogous to darktable's Gaussian-windowed bands).
/// Whites/blacks are endpoint controls with wider zones than highlights/shadows.
pub fn apply_tone_zones(
    px: [f32; 3],
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
) -> [f32; 3] {
    let l_lin = luminance(px);
    if l_lin <= 0.0001 {
        return px;
    }
    let l_p = l_lin.powf(1.0 / 2.2);

    // Shadows: peaks ~0.20-0.25, fades by ~0.65
    let sh_rise = smoothstep(0.0, 0.20, l_p);
    let sh_fall = 1.0 - smoothstep(0.25, 0.65, l_p);
    let w_sh = sh_rise * sh_fall;

    // Highlights: rises from ~0.35, full above ~0.75
    let w_hi = smoothstep(0.35, 0.75, l_p);

    // Blacks: endpoint control, affects bottom ~30% of perceptual range
    let w_bk = 1.0 - smoothstep(0.0, 0.30, l_p);

    // Whites: endpoint control, affects top ~40% of perceptual range
    let w_wh = smoothstep(0.60, 1.0, l_p);

    let stops = shadows * w_sh * 2.0
        + highlights * w_hi * 2.0
        + blacks * w_bk * 2.0
        + whites * w_wh * 2.0;

    let ratio = 2.0_f32.powf(stops);
    [px[0] * ratio, px[1] * ratio, px[2] * ratio]
}

pub fn apply_contrast(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    if lum <= 0.0 {
        return px;
    }
    // Sigmoid contrast: blend between original lum and a steep S-curve.
    // k must be > 4 (the identity slope at midpoint) to actually boost contrast.
    // At amount=0, blend factor is 0 so the result is identity.
    let k = 4.0 + amount.abs() * 8.0;
    let sig = 1.0 / (1.0 + (-k * (lum - 0.5)).exp());
    let lum_new = lum + amount * (sig - lum);
    let ratio = lum_new / lum;
    [px[0] * ratio, px[1] * ratio, px[2] * ratio]
}

pub fn apply_saturation(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    let t = 1.0 + amount;
    [
        lum + (px[0] - lum) * t,
        lum + (px[1] - lum) * t,
        lum + (px[2] - lum) * t,
    ]
}

/// Vibrance: selective saturation boost that protects already-saturated colors.
/// Uses darktable's power-law approach from colorbalancergb.c:
///   attenuation = pow(chroma, |amount|)
/// This gives a smoother rolloff than linear (1-sat) weighting — already-vivid
/// colors are barely affected while muted colors get the full boost.
pub fn apply_vibrance(px: [f32; 3], amount: f32) -> [f32; 3] {
    let max_c = px[0].max(px[1]).max(px[2]);
    let min_c = px[0].min(px[1]).min(px[2]);
    let sat = if max_c > 0.0 {
        (max_c - min_c) / max_c
    } else {
        0.0
    };
    // Power-law attenuation: pow(sat, |amount|) approaches 1 for high sat,
    // meaning already-saturated pixels get almost no additional boost.
    let attenuation = 1.0 - sat.powf(amount.abs().max(0.001));
    let weight = 1.0 + amount * attenuation;
    let lum = luminance(px);
    [
        lum + (px[0] - lum) * weight,
        lum + (px[1] - lum) * weight,
        lum + (px[2] - lum) * weight,
    ]
}

/// Bradford chromatic adaptation matrix for temperature/tint.
/// Temperature: -100..+100 maps to ~3500K..~12000K shift from D65 (6500K).
/// Tint: -100..+100 shifts green/magenta.
/// Returns a 3x3 row-major matrix for linear RGB transform.
pub fn temperature_tint_matrix(temperature: f32, tint: f32) -> [f32; 9] {
    let kelvin = 6500.0 + temperature * 55.0;

    let (xd, yd) = daylight_chromaticity(kelvin);

    let x_ref = 0.3127;
    let y_ref = 0.3290;

    let tint_shift = tint * 0.0002;
    let yd = yd + tint_shift;

    bradford_cat(x_ref, y_ref, xd, yd)
}

fn daylight_chromaticity(kelvin: f32) -> (f32, f32) {
    let t = kelvin;
    let t2 = t * t;
    let t3 = t2 * t;

    let xd = if t <= 7000.0 {
        -4.607_0e9 / t3 + 2.967_8e6 / t2 + 0.099_11e3 / t + 0.244_063
    } else {
        -2.006_4e9 / t3 + 1.901_8e6 / t2 + 0.247_48e3 / t + 0.237_040
    };

    let yd = -3.0 * xd * xd + 2.87 * xd - 0.275;
    (xd, yd)
}

fn bradford_cat(x_src: f32, y_src: f32, x_dst: f32, y_dst: f32) -> [f32; 9] {
    let src_xyz = [x_src / y_src, 1.0, (1.0 - x_src - y_src) / y_src];
    let dst_xyz = [x_dst / y_dst, 1.0, (1.0 - x_dst - y_dst) / y_dst];

    let m = [
        0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296,
    ];
    let m_inv = [
        0.9870, -0.1471, 0.1600, 0.4323, 0.5184, 0.0493, -0.0085, 0.0400, 0.9685,
    ];

    let src_lms = mat3_mul_vec3(&m, &src_xyz);
    let dst_lms = mat3_mul_vec3(&m, &dst_xyz);

    let scale = [
        dst_lms[0] / src_lms[0],
        dst_lms[1] / src_lms[1],
        dst_lms[2] / src_lms[2],
    ];

    let d_m = [
        m[0] * scale[0],
        m[1] * scale[0],
        m[2] * scale[0],
        m[3] * scale[1],
        m[4] * scale[1],
        m[5] * scale[1],
        m[6] * scale[2],
        m[7] * scale[2],
        m[8] * scale[2],
    ];

    mat3_mul_mat3(&m_inv, &d_m)
}

fn mat3_mul_vec3(m: &[f32; 9], v: &[f32; 3]) -> [f32; 3] {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
        m[6] * v[0] + m[7] * v[1] + m[8] * v[2],
    ]
}

fn mat3_mul_mat3(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    let mut r = [0.0f32; 9];
    for row in 0..3 {
        for col in 0..3 {
            r[row * 3 + col] =
                a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
        }
    }
    r
}

pub fn apply_temperature_tint(px: [f32; 3], matrix: &[f32; 9]) -> [f32; 3] {
    mat3_mul_vec3(matrix, &px)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Apply all adjustments to a single pixel (sRGB u8 input -> sRGB u8 output).
/// `blurred` is the corresponding blurred pixel for clarity/dehaze (linear RGB).
/// `temp_matrix` is the precomputed Bradford CAT matrix.
pub fn apply_all(
    srgb: [u8; 4],
    state: &EditState,
    temp_matrix: &[f32; 9],
    blurred: [f32; 3],
) -> [u8; 4] {
    let mut px = [
        srgb_to_linear(srgb[0] as f32 / 255.0),
        srgb_to_linear(srgb[1] as f32 / 255.0),
        srgb_to_linear(srgb[2] as f32 / 255.0),
    ];

    px = apply_exposure(px, state.exposure);

    if state.temperature != 0.0 || state.tint != 0.0 {
        px = apply_temperature_tint(px, temp_matrix);
    }

    let n = |v: f32| v / 100.0;
    px = apply_tone_zones(
        px,
        n(state.highlights),
        n(state.shadows),
        n(state.whites),
        n(state.blacks),
    );

    px = apply_contrast(px, n(state.contrast));

    px = apply_vibrance(px, n(state.vibrance));
    px = apply_saturation(px, n(state.saturation));

    if state.clarity != 0.0 {
        let a = n(state.clarity);
        let lum = luminance(px);
        let midtone = smoothstep(0.0, 0.5, lum) * (1.0 - smoothstep(0.5, 1.0, lum));
        for i in 0..3 {
            px[i] += a * (px[i] - blurred[i]) * midtone;
        }
    }

    if state.dehaze != 0.0 {
        let a = n(state.dehaze);
        let atmos = blurred[0].max(blurred[1]).max(blurred[2]).max(0.01);
        let dark = px[0].min(px[1]).min(px[2]);
        let t = (1.0 - a * dark / atmos).max(0.1);
        for px_c in &mut px {
            *px_c = (*px_c - atmos) / t + atmos;
        }
    }

    let r = (linear_to_srgb(px[0].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let g = (linear_to_srgb(px[1].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let b = (linear_to_srgb(px[2].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    [r, g, b, srgb[3]]
}

// -- Save --

/// Apply all edits and save to disk. Returns the output path on success.
pub fn save_edited_image(
    original_path: &Path,
    pixels: &[u8],
    width: u32,
    height: u32,
    state: &EditState,
) -> Result<PathBuf, String> {
    let temp_matrix = temperature_tint_matrix(state.temperature, state.tint);
    let blur = generate_cpu_blur(pixels, width, height);

    let mut output = Vec::with_capacity(pixels.len());
    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let srgb = [
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            ];
            let bx = (x / 4).min((width / 4).saturating_sub(1));
            let by = (y / 4).min((height / 4).saturating_sub(1));
            let bw = (width / 4).max(1);
            let bidx = ((by * bw + bx) * 3) as usize;
            let blurred = [blur[bidx], blur[bidx + 1], blur[bidx + 2]];
            let result = apply_all(srgb, state, &temp_matrix, blurred);
            output.extend_from_slice(&result);
        }
    }

    let save_path = edited_save_path(original_path);
    let img = image::RgbaImage::from_raw(width, height, output)
        .ok_or_else(|| "Failed to create output image".to_string())?;
    img.save(&save_path)
        .map_err(|e| format!("Failed to save: {e}"))?;
    Ok(save_path)
}

fn generate_cpu_blur(pixels: &[u8], width: u32, height: u32) -> Vec<f32> {
    let bw = (width / 4).max(1);
    let bh = (height / 4).max(1);
    let mut blur = vec![0.0f32; (bw * bh * 3) as usize];
    for by in 0..bh {
        for bx in 0..bw {
            let mut r = 0.0f32;
            let mut g = 0.0f32;
            let mut b = 0.0f32;
            let mut count = 0.0;
            for dy in 0..4 {
                for dx in 0..4 {
                    let x = bx * 4 + dx;
                    let y = by * 4 + dy;
                    if x < width && y < height {
                        let idx = ((y * width + x) * 4) as usize;
                        r += srgb_to_linear(pixels[idx] as f32 / 255.0);
                        g += srgb_to_linear(pixels[idx + 1] as f32 / 255.0);
                        b += srgb_to_linear(pixels[idx + 2] as f32 / 255.0);
                        count += 1.0;
                    }
                }
            }
            let bidx = ((by * bw + bx) * 3) as usize;
            blur[bidx] = r / count;
            blur[bidx + 1] = g / count;
            blur[bidx + 2] = b / count;
        }
    }
    blur
}

pub fn edited_save_path(original: &Path) -> PathBuf {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = original.extension().and_then(|e| e.to_str());
    let new_name = match ext {
        Some(e) => format!("{stem}_edited.{e}"),
        None => format!("{stem}_edited"),
    };
    original.with_file_name(new_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_edit_state_is_zeroed() {
        let s = EditState::default();
        assert_eq!(s.exposure, 0.0);
        assert_eq!(s.contrast, 0.0);
        assert_eq!(s.highlights, 0.0);
        assert_eq!(s.shadows, 0.0);
        assert_eq!(s.whites, 0.0);
        assert_eq!(s.blacks, 0.0);
        assert_eq!(s.temperature, 0.0);
        assert_eq!(s.tint, 0.0);
        assert_eq!(s.vibrance, 0.0);
        assert_eq!(s.saturation, 0.0);
        assert_eq!(s.clarity, 0.0);
        assert_eq!(s.dehaze, 0.0);
        assert!(!s.lens_correction);
        assert!(s.is_default());
    }

    #[test]
    fn is_default_false_when_modified() {
        let mut s = EditState::default();
        s.exposure = 1.0;
        assert!(!s.is_default());
    }

    #[test]
    fn undo_redo_basic_flow() {
        let mut h = UndoHistory::new();
        assert!(!h.can_undo());
        assert!(!h.can_redo());

        // Make an edit
        h.current.exposure = 1.5;
        h.commit();
        assert!(h.can_undo());
        assert!(!h.can_redo());

        // Make another edit
        h.current.contrast = 50.0;
        h.commit();

        // Undo once — should restore exposure=1.5, contrast=0
        assert!(h.undo());
        assert_eq!(h.current.exposure, 1.5);
        assert_eq!(h.current.contrast, 0.0);
        assert!(h.can_redo());

        // Redo — should restore contrast=50
        assert!(h.redo());
        assert_eq!(h.current.contrast, 50.0);
    }

    #[test]
    fn undo_on_empty_returns_false() {
        let mut h = UndoHistory::new();
        assert!(!h.undo());
    }

    #[test]
    fn redo_on_empty_returns_false() {
        let mut h = UndoHistory::new();
        assert!(!h.redo());
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut h = UndoHistory::new();
        h.current.exposure = 1.0;
        h.commit();
        h.current.exposure = 2.0;
        h.commit();

        // Undo
        h.undo();
        assert!(h.can_redo());

        // New edit should clear redo
        h.current.exposure = 3.0;
        h.commit();
        assert!(!h.can_redo());
    }

    #[test]
    fn reset_all_is_undoable() {
        let mut h = UndoHistory::new();
        h.current.exposure = 2.5;
        h.current.contrast = -30.0;
        h.commit();

        h.reset_all();
        assert!(h.current.is_default());
        assert!(h.can_undo());

        // Undo the reset
        h.undo();
        assert_eq!(h.current.exposure, 2.5);
        assert_eq!(h.current.contrast, -30.0);
    }

    // -- CPU adjustment math tests --

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn apply_exposure_zero_is_identity() {
        let px = [0.5, 0.3, 0.1];
        let out = apply_exposure(px, 0.0);
        assert!(approx(out[0], 0.5));
        assert!(approx(out[1], 0.3));
        assert!(approx(out[2], 0.1));
    }

    #[test]
    fn apply_exposure_plus_one_doubles() {
        let px = [0.25, 0.25, 0.25];
        let out = apply_exposure(px, 1.0);
        assert!(approx(out[0], 0.5));
    }

    #[test]
    fn apply_exposure_minus_one_halves() {
        let px = [0.5, 0.5, 0.5];
        let out = apply_exposure(px, -1.0);
        assert!(approx(out[0], 0.25));
    }

    #[test]
    fn srgb_to_linear_zero() {
        assert!(approx(srgb_to_linear(0.0), 0.0));
    }

    #[test]
    fn srgb_to_linear_one() {
        assert!(approx(srgb_to_linear(1.0), 1.0));
    }

    #[test]
    fn linear_to_srgb_roundtrip() {
        for i in 0..=10 {
            let v = i as f32 / 10.0;
            let rt = linear_to_srgb(srgb_to_linear(v));
            assert!(approx(rt, v));
        }
    }

    #[test]
    fn apply_saturation_zero_is_identity() {
        let px = [0.8, 0.2, 0.4];
        let out = apply_saturation(px, 0.0);
        assert!(approx(out[0], px[0]));
        assert!(approx(out[1], px[1]));
        assert!(approx(out[2], px[2]));
    }

    #[test]
    fn apply_saturation_minus_one_is_grayscale() {
        let px = [0.8, 0.2, 0.4];
        let out = apply_saturation(px, -1.0);
        let lum = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
        assert!(approx(out[0], lum));
        assert!(approx(out[1], lum));
        assert!(approx(out[2], lum));
    }

    #[test]
    fn tone_zones_highlights_affects_bright_not_dark() {
        // Highlights should brighten bright pixels
        let bright = [0.9, 0.9, 0.9];
        let out = apply_tone_zones(bright, 1.0, 0.0, 0.0, 0.0);
        assert!(out[0] > bright[0], "highlights should brighten bright pixels");

        // Highlights should minimally affect dark pixels
        let dark = [0.02, 0.02, 0.02];
        let out2 = apply_tone_zones(dark, 1.0, 0.0, 0.0, 0.0);
        assert!(
            (out2[0] - dark[0]).abs() < 0.01,
            "highlights should minimally affect dark pixels"
        );
    }

    #[test]
    fn tone_zones_shadows_affects_dark_not_bright() {
        // Shadows should brighten dark pixels
        let dark = [0.02, 0.02, 0.02];
        let out = apply_tone_zones(dark, 0.0, 1.0, 0.0, 0.0);
        assert!(out[0] > dark[0], "shadows should brighten dark pixels");

        // Shadows should minimally affect bright pixels
        let bright = [0.9, 0.9, 0.9];
        let out2 = apply_tone_zones(bright, 0.0, 1.0, 0.0, 0.0);
        // Bright pixels are above the shadow zone, so minimal effect
        assert!(
            (out2[0] - bright[0]).abs() / bright[0] < 0.05,
            "shadows should minimally affect bright pixels"
        );
    }

    #[test]
    fn tone_zones_whites_brightens_bright_pixels() {
        // Whites at +1 should noticeably brighten near-white pixels
        let bright = [0.8, 0.8, 0.8];
        let out = apply_tone_zones(bright, 0.0, 0.0, 1.0, 0.0);
        let pct_change = (out[0] - bright[0]) / bright[0];
        assert!(
            pct_change > 0.10,
            "whites should brighten bright pixels by >10%, got {:.1}%",
            pct_change * 100.0
        );

        // Whites should minimally affect dark pixels
        let dark = [0.02, 0.02, 0.02];
        let out2 = apply_tone_zones(dark, 0.0, 0.0, 1.0, 0.0);
        assert!(
            (out2[0] - dark[0]).abs() / dark[0] < 0.05,
            "whites should minimally affect dark pixels"
        );
    }

    #[test]
    fn tone_zones_blacks_darkens_dark_pixels() {
        // Blacks at -1 should noticeably darken near-black pixels
        let dark = [0.02, 0.02, 0.02];
        let out = apply_tone_zones(dark, 0.0, 0.0, 0.0, -1.0);
        let pct_change = (dark[0] - out[0]) / dark[0];
        assert!(
            pct_change > 0.10,
            "blacks should darken dark pixels by >10%, got {:.1}%",
            pct_change * 100.0
        );

        // Blacks should minimally affect bright pixels
        let bright = [0.8, 0.8, 0.8];
        let out2 = apply_tone_zones(bright, 0.0, 0.0, 0.0, -1.0);
        assert!(
            (out2[0] - bright[0]).abs() / bright[0] < 0.05,
            "blacks should minimally affect bright pixels"
        );
    }

    #[test]
    fn apply_contrast_zero_is_identity() {
        let px = [0.5, 0.3, 0.7];
        let out = apply_contrast(px, 0.0);
        assert!(approx(out[0], px[0]));
        assert!(approx(out[1], px[1]));
        assert!(approx(out[2], px[2]));
    }

    #[test]
    fn contrast_positive_increases_contrast() {
        // Positive contrast should darken shadows and brighten highlights
        let shadow = [0.2, 0.2, 0.2];
        let highlight = [0.8, 0.8, 0.8];
        let out_s = apply_contrast(shadow, 0.5);
        let out_h = apply_contrast(highlight, 0.5);
        assert!(out_s[0] < shadow[0], "positive contrast should darken shadows");
        assert!(out_h[0] > highlight[0], "positive contrast should brighten highlights");
    }

    #[test]
    fn contrast_negative_reduces_contrast() {
        let shadow = [0.2, 0.2, 0.2];
        let highlight = [0.8, 0.8, 0.8];
        let out_s = apply_contrast(shadow, -0.5);
        let out_h = apply_contrast(highlight, -0.5);
        assert!(out_s[0] > shadow[0], "negative contrast should brighten shadows");
        assert!(out_h[0] < highlight[0], "negative contrast should darken highlights");
    }

    #[test]
    fn tone_zones_preserve_color_ratios() {
        // A colored pixel should maintain its R:G:B ratios after zone adjustment
        let colored = [0.6, 0.3, 0.1];
        let out = apply_tone_zones(colored, -0.5, 0.0, 0.0, 0.0);

        // Ratios should be preserved (R/G and G/B)
        let orig_rg = colored[0] / colored[1];
        let orig_gb = colored[1] / colored[2];
        let out_rg = out[0] / out[1];
        let out_gb = out[1] / out[2];
        assert!(approx(orig_rg, out_rg), "R:G ratio shifted");
        assert!(approx(orig_gb, out_gb), "G:B ratio shifted");
    }

    #[test]
    fn tone_zones_zero_is_identity() {
        let px = [0.5, 0.3, 0.1];
        let out = apply_tone_zones(px, 0.0, 0.0, 0.0, 0.0);
        assert!(approx(out[0], px[0]));
        assert!(approx(out[1], px[1]));
        assert!(approx(out[2], px[2]));
    }

    #[test]
    fn tone_zones_positive_brightens_negative_darkens() {
        let mid = [0.2, 0.2, 0.2]; // dark-ish pixel for shadows
        let bright = [0.8, 0.8, 0.8]; // bright pixel for highlights

        // Positive shadows should brighten
        let out_s = apply_tone_zones(mid, 0.0, 0.5, 0.0, 0.0);
        assert!(out_s[0] > mid[0], "positive shadows should brighten");

        // Negative shadows should darken
        let out_sn = apply_tone_zones(mid, 0.0, -0.5, 0.0, 0.0);
        assert!(out_sn[0] < mid[0], "negative shadows should darken");

        // Negative highlights should darken bright pixels
        let out_hn = apply_tone_zones(bright, -0.5, 0.0, 0.0, 0.0);
        assert!(out_hn[0] < bright[0], "negative highlights should darken");
    }

    #[test]
    fn save_path_appends_edited() {
        use std::path::PathBuf;
        let p = PathBuf::from("/photos/sunset.jpg");
        let out = edited_save_path(&p);
        assert_eq!(out, PathBuf::from("/photos/sunset_edited.jpg"));
    }

    #[test]
    fn save_path_handles_no_extension() {
        use std::path::PathBuf;
        let p = PathBuf::from("/photos/image");
        let out = edited_save_path(&p);
        assert_eq!(out, PathBuf::from("/photos/image_edited"));
    }

    #[test]
    fn apply_all_identity_preserves_pixel() {
        let state = EditState::default();
        let identity_mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let blurred = [0.5, 0.5, 0.5];
        let input = [128, 64, 200, 255];
        let output = apply_all(input, &state, &identity_mat, blurred);
        assert!((output[0] as i16 - input[0] as i16).abs() <= 1);
        assert!((output[1] as i16 - input[1] as i16).abs() <= 1);
        assert!((output[2] as i16 - input[2] as i16).abs() <= 1);
        assert_eq!(output[3], 255);
    }
}
