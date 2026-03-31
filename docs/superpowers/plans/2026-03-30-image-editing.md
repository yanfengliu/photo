# Image Editing Feature — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add real-time GPU shader-based image editing with 12 adjustment sliders, Lensfun lens corrections, undo/redo, and save-as-copy.

**Architecture:** All color adjustments run in the WGSL fragment shader via uniforms — slider drags only update ~128 bytes, same cost as zoom/pan. A blur pre-pass (re-rendered only on image change) supports clarity/dehaze. Lens corrections use Lensfun's XML database for distortion/vignetting/TCA via UV remapping in the shader. CPU-side math mirrors the shader for full-resolution save.

**Tech Stack:** Rust, iced 0.13, wgpu 0.19 (via iced), WGSL shaders, kamadak-exif, quick-xml, Lensfun XML database

**Spec:** `docs/superpowers/specs/2026-03-30-image-editing-design.md`

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Add `kamadak-exif`, `quick-xml` dependencies |
| `src/edit.rs` | Create | `EditState`, `UndoHistory`, CPU adjustment math, save pipeline |
| `src/lens.rs` | Create | Lensfun XML parser, EXIF reading, lens profile lookup |
| `src/viewer.rs` | Modify | Extended `Uniforms`, blur pre-pass, lens uniform passing |
| `src/main.rs` | Modify | Edit panel UI, sliders, undo/redo keybinds, save, wiring |
| `assets/shaders/image.wgsl` | Modify | Full adjustment pipeline in fragment shader |
| `assets/shaders/blur.wgsl` | Create | Separable Gaussian blur shader |
| `assets/lensfun/` | Create | Bundled Lensfun XML database files (~56 XML files) |
| `docs/ARCHITECTURE.md` | Modify | Update component map, data flow, dependencies |

---

## Task 1: Add Dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add new crate dependencies**

Add `kamadak-exif` and `quick-xml` to `Cargo.toml`:

```toml
# After the jpeg-decoder line:
kamadak-exif = "0.6"
quick-xml = "0.37"
```

`kamadak-exif` is already a transitive dependency of the `image` crate, so this adds no new binary weight. `quick-xml` is lightweight (~100KB).

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: Success with no errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: add kamadak-exif and quick-xml dependencies for image editing"
```

---

## Task 2: EditState and UndoHistory Data Model

**Files:**
- Create: `src/edit.rs`
- Modify: `src/main.rs` (add `mod edit;`)

- [ ] **Step 1: Write failing tests for EditState and UndoHistory**

Create `src/edit.rs` with test module first:

```rust
/// Image editing state and undo/redo history.
/// All adjustment math lives here — both the data model and CPU-side
/// processing for full-resolution save.

// -- Data model --

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditState {
    pub exposure: f32,      // -5.0 to +5.0 (stops)
    pub contrast: f32,      // -100 to +100
    pub highlights: f32,    // -100 to +100
    pub shadows: f32,       // -100 to +100
    pub whites: f32,        // -100 to +100
    pub blacks: f32,        // -100 to +100
    pub temperature: f32,   // -100 to +100
    pub tint: f32,          // -100 to +100
    pub vibrance: f32,      // -100 to +100
    pub saturation: f32,    // -100 to +100
    pub clarity: f32,       // -100 to +100
    pub dehaze: f32,        // -100 to +100
    pub lens_correction: bool,
}

impl Default for EditState {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            lens_correction: false,
        }
    }
}

impl EditState {
    /// Returns true if all adjustments are at their defaults (no edits).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

pub struct UndoHistory {
    undo_stack: Vec<EditState>,
    redo_stack: Vec<EditState>,
    pub current: EditState,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current: EditState::default(),
        }
    }

    /// Call when a slider drag ends. Pushes current state to undo stack.
    pub fn commit(&mut self) {
        self.undo_stack.push(self.current);
        self.redo_stack.clear();
    }

    /// Undo: restore previous state. Returns true if undo was performed.
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.current);
            self.current = prev;
            true
        } else {
            false
        }
    }

    /// Redo: restore next state. Returns true if redo was performed.
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.current);
            self.current = next;
            true
        } else {
            false
        }
    }

    /// Reset all adjustments to default. This is an undoable action.
    pub fn reset_all(&mut self) {
        self.undo_stack.push(self.current);
        self.redo_stack.clear();
        self.current = EditState::default();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
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
}
```

- [ ] **Step 2: Register the module in main.rs**

In `src/main.rs`, add after `mod viewer;` (line 5):

```rust
mod edit;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test edit::tests`
Expected: All 7 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/edit.rs src/main.rs
git commit -m "feat: add EditState and UndoHistory data model with tests"
```

---

## Task 3: CPU Adjustment Math

**Files:**
- Modify: `src/edit.rs`

These functions mirror the shader math for use during full-resolution save. Each operates on a single `[f32; 3]` RGB pixel in linear space.

- [ ] **Step 1: Write failing tests for CPU adjustment functions**

Add tests to `src/edit.rs` test module:

```rust
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
        // All channels should equal luminance
        let lum = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
        assert!(approx(out[0], lum));
        assert!(approx(out[1], lum));
        assert!(approx(out[2], lum));
    }

    #[test]
    fn apply_highlights_only_affects_bright() {
        // Dark pixel (lum < 0.5) should be unchanged
        let dark = [0.1, 0.1, 0.1];
        let out = apply_highlights(dark, 1.0);
        assert!(approx(out[0], dark[0]));

        // Bright pixel (lum > 0.5) should change
        let bright = [0.9, 0.9, 0.9];
        let out2 = apply_highlights(bright, 1.0);
        assert!(out2[0] > bright[0]);
    }

    #[test]
    fn apply_shadows_only_affects_dark() {
        // Bright pixel should be unchanged
        let bright = [0.9, 0.9, 0.9];
        let out = apply_shadows(bright, 1.0);
        assert!(approx(out[0], bright[0]));

        // Dark pixel should change
        let dark = [0.1, 0.1, 0.1];
        let out2 = apply_shadows(dark, 1.0);
        assert!(out2[0] > dark[0]);
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
```

- [ ] **Step 2: Implement CPU adjustment functions**

Add above the `#[cfg(test)]` block in `src/edit.rs`:

```rust
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

pub fn apply_highlights(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    let mask = smoothstep(0.5, 1.0, lum);
    let d = amount * mask;
    [px[0] + d, px[1] + d, px[2] + d]
}

pub fn apply_shadows(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    let mask = 1.0 - smoothstep(0.0, 0.5, lum);
    let d = amount * mask;
    [px[0] + d, px[1] + d, px[2] + d]
}

pub fn apply_whites(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    let mask = smoothstep(0.85, 1.0, lum);
    let d = amount * mask;
    [px[0] + d, px[1] + d, px[2] + d]
}

pub fn apply_blacks(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    let mask = 1.0 - smoothstep(0.0, 0.15, lum);
    let d = amount * mask;
    [px[0] + d, px[1] + d, px[2] + d]
}

pub fn apply_contrast(px: [f32; 3], amount: f32) -> [f32; 3] {
    let lum = luminance(px);
    if lum <= 0.0 {
        return px;
    }
    let k = 1.0 + amount * 4.0; // range ~-3..5
    let lum_new = 1.0 / (1.0 + (-k * (lum - 0.5)).exp());
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

pub fn apply_vibrance(px: [f32; 3], amount: f32) -> [f32; 3] {
    let max_c = px[0].max(px[1]).max(px[2]);
    let min_c = px[0].min(px[1]).min(px[2]);
    let sat = if max_c > 0.0 {
        (max_c - min_c) / max_c
    } else {
        0.0
    };
    let weight = 1.0 + amount * (1.0 - sat);
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
    // Map slider to Kelvin: 0 = 6500K (D65), -100 = 3500K, +100 = 12000K
    let kelvin = 6500.0 + temperature * 55.0; // range 1000..12000 approx

    // CIE daylight chromaticity from CCT
    let (xd, yd) = daylight_chromaticity(kelvin);

    // D65 reference white
    let x_ref = 0.3127;
    let y_ref = 0.3290;

    // Apply tint as green-magenta shift perpendicular to Planckian locus
    let tint_shift = tint * 0.0002;
    let xd = xd;
    let yd = yd + tint_shift;

    // Bradford matrix (D65 -> target illuminant adaptation)
    bradford_cat(x_ref, y_ref, xd, yd)
}

fn daylight_chromaticity(kelvin: f32) -> (f32, f32) {
    let t = kelvin;
    let t2 = t * t;
    let t3 = t2 * t;

    let xd = if t <= 7000.0 {
        -4.6070e9 / t3 + 2.9678e6 / t2 + 0.09911e3 / t + 0.244063
    } else {
        -2.0064e9 / t3 + 1.9018e6 / t2 + 0.24748e3 / t + 0.237040
    };

    let yd = -3.0 * xd * xd + 2.87 * xd - 0.275;
    (xd, yd)
}

fn bradford_cat(x_src: f32, y_src: f32, x_dst: f32, y_dst: f32) -> [f32; 9] {
    // XYZ from chromaticity (Y=1)
    let src_xyz = [x_src / y_src, 1.0, (1.0 - x_src - y_src) / y_src];
    let dst_xyz = [x_dst / y_dst, 1.0, (1.0 - x_dst - y_dst) / y_dst];

    // Bradford cone response matrix
    let m = [
        0.8951, 0.2664, -0.1614,
        -0.7502, 1.7135, 0.0367,
        0.0389, -0.0685, 1.0296,
    ];
    let m_inv = [
        0.9870, -0.1471, 0.1600,
        0.4323, 0.5184, 0.0493,
        -0.0085, 0.0400, 0.9685,
    ];

    // Cone responses
    let src_lms = mat3_mul_vec3(&m, &src_xyz);
    let dst_lms = mat3_mul_vec3(&m, &dst_xyz);

    // Diagonal scaling
    let scale = [
        dst_lms[0] / src_lms[0],
        dst_lms[1] / src_lms[1],
        dst_lms[2] / src_lms[2],
    ];

    // Combined: M_inv * diag(scale) * M
    let d_m = [
        m[0] * scale[0], m[1] * scale[0], m[2] * scale[0],
        m[3] * scale[1], m[4] * scale[1], m[5] * scale[1],
        m[6] * scale[2], m[7] * scale[2], m[8] * scale[2],
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
            r[row * 3 + col] = a[row * 3] * b[col]
                + a[row * 3 + 1] * b[3 + col]
                + a[row * 3 + 2] * b[6 + col];
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
    // Linearize
    let mut px = [
        srgb_to_linear(srgb[0] as f32 / 255.0),
        srgb_to_linear(srgb[1] as f32 / 255.0),
        srgb_to_linear(srgb[2] as f32 / 255.0),
    ];

    // Exposure
    px = apply_exposure(px, state.exposure);

    // Temperature/Tint
    if state.temperature != 0.0 || state.tint != 0.0 {
        px = apply_temperature_tint(px, temp_matrix);
    }

    // Tone: normalize -100..+100 to -1..+1
    let n = |v: f32| v / 100.0;
    px = apply_highlights(px, n(state.highlights));
    px = apply_shadows(px, n(state.shadows));
    px = apply_whites(px, n(state.whites));
    px = apply_blacks(px, n(state.blacks));

    // Contrast
    px = apply_contrast(px, n(state.contrast));

    // Vibrance & Saturation
    px = apply_vibrance(px, n(state.vibrance));
    px = apply_saturation(px, n(state.saturation));

    // Clarity (local contrast)
    if state.clarity != 0.0 {
        let a = n(state.clarity);
        let lum = luminance(px);
        let midtone = smoothstep(0.0, 0.5, lum) * (1.0 - smoothstep(0.5, 1.0, lum));
        for i in 0..3 {
            px[i] += a * (px[i] - blurred[i]) * midtone;
        }
    }

    // Dehaze
    if state.dehaze != 0.0 {
        let a = n(state.dehaze);
        let atmos = blurred[0].max(blurred[1]).max(blurred[2]).max(0.01);
        let dark = px[0].min(px[1]).min(px[2]);
        let t = (1.0 - a * dark / atmos).max(0.1);
        for i in 0..3 {
            px[i] = (px[i] - atmos) / t + atmos;
        }
    }

    // Clamp and gamma encode
    let r = (linear_to_srgb(px[0].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let g = (linear_to_srgb(px[1].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let b = (linear_to_srgb(px[2].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    [r, g, b, srgb[3]]
}

// -- Save --

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
```

- [ ] **Step 3: Run tests**

Run: `cargo test edit::tests`
Expected: All tests pass (7 undo/redo + 13 adjustment math = 20 tests).

- [ ] **Step 4: Commit**

```bash
git add src/edit.rs
git commit -m "feat: add CPU adjustment math and save path helper with tests"
```

---

## Task 4: WGSL Shader — Full Adjustment Pipeline

**Files:**
- Modify: `assets/shaders/image.wgsl`

Rewrite the fragment shader to apply all 12 adjustments. The blur texture and lens corrections are added in later tasks — this task adds bindings for them as no-ops.

- [ ] **Step 1: Rewrite the WGSL shader**

Replace the entire contents of `assets/shaders/image.wgsl` with:

```wgsl
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

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
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

    // Zone-based tone adjustments
    let l = lum(px);
    px += u.highlights * smooth(0.5, 1.0, l);
    px += u.shadows * (1.0 - smooth(0.0, 0.5, l));
    px += u.whites * smooth(0.85, 1.0, l);
    px += u.blacks * (1.0 - smooth(0.0, 0.15, l));

    // Contrast: sigmoid S-curve on luminance
    let l2 = lum(px);
    if l2 > 0.0 && u.contrast != 0.0 {
        let k = 1.0 + u.contrast * 4.0;
        let l_new = 1.0 / (1.0 + exp(-k * (l2 - 0.5)));
        px = px * (l_new / l2);
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
        let midtone = smooth(0.0, 0.5, lc) * (1.0 - smooth(0.5, 1.0, lc));
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
```

- [ ] **Step 2: Verify shader compiles by building**

The shader will compile when loaded at runtime. For now, verify the Rust code still compiles:

Run: `cargo check`
Expected: Success (shader isn't validated at compile time, only at runtime).

- [ ] **Step 3: Commit**

```bash
git add assets/shaders/image.wgsl
git commit -m "feat: rewrite WGSL shader with full adjustment pipeline"
```

---

## Task 5: Extended Uniforms and Viewer Pipeline

**Files:**
- Modify: `src/viewer.rs`

Update the `Uniforms` struct, bind group layout, and `prepare()`/`render()` to pass adjustment values and support the blur texture binding.

- [ ] **Step 1: Update Uniforms struct**

In `src/viewer.rs`, replace the `Uniforms` struct (lines 69-74) with:

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    rect: [f32; 4],
    bg_color: [f32; 4],
    // Adjustments
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
    _pad0: f32,
    _pad1: f32,
    // Temperature/tint Bradford matrix (3 rows, padded to vec4 each)
    temp_mat_row0: [f32; 4],
    temp_mat_row1: [f32; 4],
    temp_mat_row2: [f32; 4],
    // Lens corrections
    lens_enabled: f32,
    lens_dist_a: f32,
    lens_dist_b: f32,
    lens_dist_c: f32,
    lens_vig_k1: f32,
    lens_vig_k2: f32,
    lens_vig_k3: f32,
    lens_tca_r_scale: f32,
    lens_tca_b_scale: f32,
    image_aspect: f32,
    _pad2: f32,
    _pad3: f32,
}
```

- [ ] **Step 2: Add adjustment data to ImageCanvas**

Update `ImageCanvas` (lines 36-41) to include edit state:

```rust
pub struct ImageCanvas {
    pub image: Option<Arc<ImageData>>,
    pub image_id: u64,
    pub zoom: f32,
    pub offset: [f32; 2],
    pub adjustments: AdjustmentUniforms,
}

/// Plain data passed from App to the shader. No GPU types.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdjustmentUniforms {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub temp_matrix: [f32; 9],  // row-major 3x3
    pub lens_enabled: bool,
    pub lens_dist: [f32; 3],    // a, b, c
    pub lens_vig: [f32; 3],     // k1, k2, k3
    pub lens_tca_r: f32,
    pub lens_tca_b: f32,
    pub image_aspect: f32,
}
```

- [ ] **Step 3: Add blur texture to GpuResources**

Add fields to `GpuResources` (after `current_image_id`):

```rust
    blur_texture: Option<wgpu::Texture>,
    blur_texture_view: Option<wgpu::TextureView>,
```

- [ ] **Step 4: Update bind group layout**

In `prepare()`, add binding 3 for the blur texture in the bind group layout. Add a new entry after the sampler binding (binding 2):

```rust
wgpu::BindGroupLayoutEntry {
    binding: 3,
    visibility: wgpu::ShaderStages::FRAGMENT,
    ty: wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    },
    count: None,
},
```

- [ ] **Step 5: Update uniform buffer writes in prepare()**

In the `prepare()` method, update the `Uniforms` construction to include the adjustment values from `self.adjustments`. The identity temperature matrix is:

```rust
let adj = &self.adjustments;
let identity_mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
let mat = if adj.temp_matrix == [0.0; 9] { identity_mat } else { adj.temp_matrix };

let uniforms = Uniforms {
    rect,
    bg_color: [0.10, 0.10, 0.10, 1.0],
    exposure: adj.exposure,
    contrast: adj.contrast / 100.0,
    highlights: adj.highlights / 100.0,
    shadows: adj.shadows / 100.0,
    whites: adj.whites / 100.0,
    blacks: adj.blacks / 100.0,
    vibrance: adj.vibrance / 100.0,
    saturation: adj.saturation / 100.0,
    clarity: adj.clarity / 100.0,
    dehaze: adj.dehaze / 100.0,
    _pad0: 0.0,
    _pad1: 0.0,
    temp_mat_row0: [mat[0], mat[1], mat[2], 0.0],
    temp_mat_row1: [mat[3], mat[4], mat[5], 0.0],
    temp_mat_row2: [mat[6], mat[7], mat[8], 0.0],
    lens_enabled: if adj.lens_enabled { 1.0 } else { 0.0 },
    lens_dist_a: adj.lens_dist[0],
    lens_dist_b: adj.lens_dist[1],
    lens_dist_c: adj.lens_dist[2],
    lens_vig_k1: adj.lens_vig[0],
    lens_vig_k2: adj.lens_vig[1],
    lens_vig_k3: adj.lens_vig[2],
    lens_tca_r_scale: if adj.lens_tca_r == 0.0 { 1.0 } else { adj.lens_tca_r },
    lens_tca_b_scale: if adj.lens_tca_b == 0.0 { 1.0 } else { adj.lens_tca_b },
    image_aspect: adj.image_aspect,
    _pad2: 0.0,
    _pad3: 0.0,
};
```

- [ ] **Step 6: Create a 1x1 placeholder blur texture**

When no blur is available (before the blur pass runs), bind a 1x1 white texture as the blur placeholder so the shader doesn't error. Create this in the pipeline initialization alongside the sampler. Add to bind group creation.

- [ ] **Step 7: Update ImageCanvas usage in main.rs**

In `detail_view()` (main.rs), pass the adjustment uniforms to `ImageCanvas`:

```rust
let canvas: Element<'_, ViewerEvent> = shader(ImageCanvas {
    image: self.image.clone(),
    image_id: self.image_id,
    zoom: self.zoom,
    offset: self.offset,
    adjustments: self.build_adjustment_uniforms(),
})
```

Add `build_adjustment_uniforms()` to App that converts `EditState` to `AdjustmentUniforms`, including computing the Bradford temperature matrix.

- [ ] **Step 8: Run tests and verify compilation**

Run: `cargo test`
Expected: All existing tests pass. The shader will be validated at runtime when the app is launched.

Run: `cargo build --release`
Expected: Compiles successfully.

- [ ] **Step 9: Commit**

```bash
git add src/viewer.rs src/main.rs
git commit -m "feat: extend viewer uniforms for all adjustments and blur texture binding"
```

---

## Task 6: Blur Pre-Pass for Clarity/Dehaze

**Files:**
- Create: `assets/shaders/blur.wgsl`
- Modify: `src/viewer.rs`

Two-pass separable Gaussian blur at 1/4 resolution. Rendered once per image load, stored as a texture for the main pass.

- [ ] **Step 1: Write the blur shader**

Create `assets/shaders/blur.wgsl`:

```wgsl
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
```

- [ ] **Step 2: Add blur pipeline to viewer.rs**

In `GpuResources`, add fields for the blur pipeline:

```rust
    blur_pipeline: Option<wgpu::RenderPipeline>,
    blur_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blur_uniform_buffer: Option<wgpu::Buffer>,
    blur_intermediate_texture: Option<wgpu::Texture>,
    blur_intermediate_view: Option<wgpu::TextureView>,
```

Create the blur pipeline in `prepare()` when GPU resources are first initialized. This includes:
- Loading `blur.wgsl` via `include_str!("../assets/shaders/blur.wgsl")`
- Creating bind group layout with blur uniforms, source texture, sampler
- Creating the render pipeline

- [ ] **Step 3: Execute blur passes on image change**

In `prepare()`, when `image_id` changes (a new image was loaded):
1. Create a 1/4 resolution texture (`blur_w = width/4, blur_h = height/4`)
2. Create intermediate texture at same size for two-pass blur
3. Render horizontal blur pass: source = downscaled image, target = intermediate
4. Render vertical blur pass: source = intermediate, target = blur texture
5. Store final blur texture view for main pass binding

The downscale is done by rendering the full image texture into the 1/4 res target with linear sampling (the GPU does the filtering).

- [ ] **Step 4: Bind blur texture in main pass**

Update the main bind group creation to include the blur texture view at binding 3 instead of the 1x1 placeholder.

- [ ] **Step 5: Verify build and test**

Run: `cargo build --release`
Expected: Compiles. The blur will be visible when clarity/dehaze sliders are used.

Run: `cargo test`
Expected: All existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add assets/shaders/blur.wgsl src/viewer.rs
git commit -m "feat: add Gaussian blur pre-pass for clarity and dehaze"
```

---

## Task 7: Lensfun XML Parser

**Files:**
- Create: `src/lens.rs`
- Modify: `src/main.rs` (add `mod lens;`)

- [ ] **Step 1: Download and bundle Lensfun database**

Clone or download the XML files from `https://github.com/lensfun/lensfun` — specifically the `data/db/*.xml` files (56 files). Place them in `assets/lensfun/`.

These are embedded at compile time via `include_str!` on a manifest file, or loaded from the directory at runtime. For simplicity, embed the XML content at compile time.

Create `assets/lensfun/manifest.txt` listing all XML filenames (one per line). The parser will load each via `include_str!`.

- [ ] **Step 2: Write tests for lens profile parsing**

Create `src/lens.rs`:

```rust
/// Lensfun XML database parser and lens profile lookup.
/// Reads camera/lens EXIF data and matches against the bundled Lensfun database
/// for distortion, vignetting, and TCA correction coefficients.

use std::path::Path;

// -- Data types --

#[derive(Debug, Clone, Default)]
pub struct LensProfile {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub distortion: Option<DistortionCoeffs>,
    pub vignetting: Option<VignetteCoeffs>,
    pub tca: Option<TcaCoeffs>,
}

#[derive(Debug, Clone, Copy)]
pub struct DistortionCoeffs {
    pub model: DistortionModel,
    pub a: f32,
    pub b: f32,
    pub c: f32,
}

#[derive(Debug, Clone, Copy)]
pub enum DistortionModel {
    PtLens,
    Poly3,
}

#[derive(Debug, Clone, Copy)]
pub struct VignetteCoeffs {
    pub k1: f32,
    pub k2: f32,
    pub k3: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TcaCoeffs {
    pub vr: f32,
    pub vb: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ExifInfo {
    pub camera_make: String,
    pub camera_model: String,
    pub lens_make: String,
    pub lens_model: String,
    pub focal_length: Option<f32>,
    pub aperture: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"
    <lensdatabase version="2">
        <lens>
            <maker>Sony</maker>
            <model>E 16mm f/2.8</model>
            <mount>Sony E</mount>
            <cropfactor>1.534</cropfactor>
            <calibration>
                <distortion model="ptlens" focal="16" a="0.01701" b="-0.02563" c="-0.0052"/>
                <tca model="poly3" focal="16" br="-0.0003027" vr="1.0010272" bb="0.0003454" vb="0.9993952"/>
                <vignetting model="pa" focal="16" aperture="2.8" distance="0.25" k1="-1.8891" k2="1.7993" k3="-0.7326"/>
            </calibration>
        </lens>
    </lensdatabase>
    "#;

    #[test]
    fn parse_lens_from_xml() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].maker, "Sony");
        assert_eq!(profiles[0].model, "E 16mm f/2.8");
    }

    #[test]
    fn parse_distortion_coefficients() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let dist = profiles[0].distortion.unwrap();
        assert!((dist.a - 0.01701).abs() < 0.0001);
        assert!((dist.b - (-0.02563)).abs() < 0.0001);
        assert!((dist.c - (-0.0052)).abs() < 0.0001);
    }

    #[test]
    fn parse_vignetting_coefficients() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let vig = profiles[0].vignetting.unwrap();
        assert!((vig.k1 - (-1.8891)).abs() < 0.0001);
        assert!((vig.k2 - 1.7993).abs() < 0.0001);
        assert!((vig.k3 - (-0.7326)).abs() < 0.0001);
    }

    #[test]
    fn parse_tca_coefficients() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let tca = profiles[0].tca.unwrap();
        assert!((tca.vr - 1.0010272).abs() < 0.0001);
        assert!((tca.vb - 0.9993952).abs() < 0.0001);
    }

    #[test]
    fn lookup_lens_by_model_substring() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let db = LensDatabase { profiles };
        let result = db.find_lens("Sony", "E 16mm f/2.8");
        assert!(result.is_some());
    }

    #[test]
    fn lookup_lens_not_found() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let db = LensDatabase { profiles };
        let result = db.find_lens("Nonexistent", "fake lens");
        assert!(result.is_none());
    }
}
```

- [ ] **Step 3: Implement the parser**

Add above the test module in `src/lens.rs`:

```rust
use quick_xml::events::Event;
use quick_xml::reader::Reader;

pub struct LensDatabase {
    pub profiles: Vec<LensProfile>,
}

impl LensDatabase {
    /// Load the bundled Lensfun database.
    pub fn load_bundled() -> Self {
        let mut profiles = Vec::new();
        // Include all XML files at compile time
        let xml_sources: &[&str] = &[
            include_str!("../assets/lensfun/slr-nikon.xml"),
            include_str!("../assets/lensfun/slr-canon.xml"),
            include_str!("../assets/lensfun/slr-sigma.xml"),
            include_str!("../assets/lensfun/slr-tamron.xml"),
            include_str!("../assets/lensfun/slr-tokina.xml"),
            include_str!("../assets/lensfun/slr-sony.xml"),
            include_str!("../assets/lensfun/slr-samyang.xml"),
            include_str!("../assets/lensfun/slr-pentax.xml"),
            include_str!("../assets/lensfun/slr-zeiss.xml"),
            include_str!("../assets/lensfun/mil-sony.xml"),
            include_str!("../assets/lensfun/mil-canon.xml"),
            include_str!("../assets/lensfun/mil-fujifilm.xml"),
            include_str!("../assets/lensfun/mil-nikon.xml"),
            include_str!("../assets/lensfun/mil-olympus.xml"),
            include_str!("../assets/lensfun/mil-panasonic.xml"),
            include_str!("../assets/lensfun/mil-samyang.xml"),
            include_str!("../assets/lensfun/mil-sigma.xml"),
            include_str!("../assets/lensfun/mil-tamron.xml"),
            include_str!("../assets/lensfun/mil-tokina.xml"),
            include_str!("../assets/lensfun/mil-zeiss.xml"),
            include_str!("../assets/lensfun/compact-canon.xml"),
            include_str!("../assets/lensfun/compact-fujifilm.xml"),
            include_str!("../assets/lensfun/compact-nikon.xml"),
            include_str!("../assets/lensfun/compact-olympus.xml"),
            include_str!("../assets/lensfun/compact-panasonic.xml"),
            include_str!("../assets/lensfun/compact-sony.xml"),
            // Add all remaining XML files here
        ];
        for xml in xml_sources {
            profiles.extend(parse_lensfun_xml(xml));
        }
        Self { profiles }
    }

    /// Find a lens profile matching the given lens model string.
    /// Uses case-insensitive substring matching.
    pub fn find_lens(&self, maker: &str, model: &str) -> Option<&LensProfile> {
        let maker_lower = maker.to_lowercase();
        let model_lower = model.to_lowercase();
        self.profiles.iter().find(|p| {
            p.maker.to_lowercase().contains(&maker_lower)
                && p.model.to_lowercase().contains(&model_lower)
        })
    }
}

pub fn parse_lensfun_xml(xml: &str) -> Vec<LensProfile> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut profiles = Vec::new();
    let mut current_lens: Option<LensProfile> = None;
    let mut in_lens = false;
    let mut in_calibration = false;
    let mut current_element = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"lens" => {
                    in_lens = true;
                    current_lens = Some(LensProfile::default());
                }
                b"calibration" if in_lens => {
                    in_calibration = true;
                }
                b"maker" | b"model" | b"mount" if in_lens => {
                    current_element = String::from_utf8_lossy(e.name().as_ref()).to_string();
                }
                _ => {}
            },
            Ok(Event::Text(e)) if in_lens => {
                if let Some(ref mut lens) = current_lens {
                    let text = e.unescape().unwrap_or_default().to_string();
                    match current_element.as_str() {
                        "maker" if lens.maker.is_empty() => lens.maker = text,
                        "model" if lens.model.is_empty() => lens.model = text,
                        "mount" if lens.mount.is_empty() => lens.mount = text,
                        _ => {}
                    }
                }
                current_element.clear();
            }
            Ok(Event::Empty(e)) if in_calibration => {
                if let Some(ref mut lens) = current_lens {
                    match e.name().as_ref() {
                        b"distortion" => {
                            lens.distortion = parse_distortion(&e);
                        }
                        b"vignetting" if lens.vignetting.is_none() => {
                            lens.vignetting = parse_vignetting(&e);
                        }
                        b"tca" => {
                            lens.tca = parse_tca(&e);
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"lens" => {
                    if let Some(lens) = current_lens.take() {
                        profiles.push(lens);
                    }
                    in_lens = false;
                    in_calibration = false;
                }
                b"calibration" => {
                    in_calibration = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    profiles
}

fn attr_f32(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<f32> {
    e.attributes().filter_map(|a| a.ok()).find_map(|a| {
        if a.key.as_ref() == name {
            String::from_utf8_lossy(&a.value).parse::<f32>().ok()
        } else {
            None
        }
    })
}

fn attr_str(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().filter_map(|a| a.ok()).find_map(|a| {
        if a.key.as_ref() == name {
            Some(String::from_utf8_lossy(&a.value).to_string())
        } else {
            None
        }
    })
}

fn parse_distortion(e: &quick_xml::events::BytesStart) -> Option<DistortionCoeffs> {
    let model_str = attr_str(e, b"model")?;
    match model_str.as_str() {
        "ptlens" => Some(DistortionCoeffs {
            model: DistortionModel::PtLens,
            a: attr_f32(e, b"a").unwrap_or(0.0),
            b: attr_f32(e, b"b").unwrap_or(0.0),
            c: attr_f32(e, b"c").unwrap_or(0.0),
        }),
        "poly3" => Some(DistortionCoeffs {
            model: DistortionModel::Poly3,
            a: attr_f32(e, b"k1").unwrap_or(0.0),
            b: 0.0,
            c: 0.0,
        }),
        _ => None,
    }
}

fn parse_vignetting(e: &quick_xml::events::BytesStart) -> Option<VignetteCoeffs> {
    Some(VignetteCoeffs {
        k1: attr_f32(e, b"k1")?,
        k2: attr_f32(e, b"k2")?,
        k3: attr_f32(e, b"k3")?,
    })
}

fn parse_tca(e: &quick_xml::events::BytesStart) -> Option<TcaCoeffs> {
    Some(TcaCoeffs {
        vr: attr_f32(e, b"vr").unwrap_or(1.0),
        vb: attr_f32(e, b"vb").unwrap_or(1.0),
    })
}

/// Read EXIF data from an image file.
pub fn read_exif(path: &Path) -> Option<ExifInfo> {
    let file = std::fs::File::open(path).ok()?;
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::BufReader::new(file))
        .ok()?;

    let get_str = |tag: exif::Tag| -> String {
        exif.get_field(tag, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string().trim().to_string())
            .unwrap_or_default()
    };

    let get_rational = |tag: exif::Tag| -> Option<f32> {
        let field = exif.get_field(tag, exif::In::PRIMARY)?;
        match &field.value {
            exif::Value::Rational(ref v) if !v.is_empty() => Some(v[0].to_f64() as f32),
            _ => None,
        }
    };

    Some(ExifInfo {
        camera_make: get_str(exif::Tag::Make),
        camera_model: get_str(exif::Tag::Model),
        lens_make: get_str(exif::Tag::LensMake),
        lens_model: get_str(exif::Tag::LensModel),
        focal_length: get_rational(exif::Tag::FocalLength),
        aperture: get_rational(exif::Tag::FNumber),
    })
}
```

- [ ] **Step 4: Register module in main.rs**

Add after `mod edit;`:

```rust
mod lens;
```

- [ ] **Step 5: Run tests**

Run: `cargo test lens::tests`
Expected: All 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/lens.rs src/main.rs assets/lensfun/
git commit -m "feat: add Lensfun XML parser and EXIF reader with tests"
```

---

## Task 8: Edit Panel UI

**Files:**
- Modify: `src/main.rs`

Add the edit panel sidebar with sliders, section headers, lens correction toggle, and reset/save buttons.

- [ ] **Step 1: Add edit state fields to App**

Add to `App` struct:

```rust
    edit_panel_open: bool,
    edit_histories: std::collections::HashMap<PathBuf, edit::UndoHistory>,
    current_image_path: Option<PathBuf>,
    lens_db: lens::LensDatabase,
    current_lens_profile: Option<lens::LensProfile>,
    current_exif: Option<lens::ExifInfo>,
    save_status: Option<String>,
```

Initialize in `App::new()`:

```rust
    edit_panel_open: false,
    edit_histories: std::collections::HashMap::new(),
    current_image_path: None,
    lens_db: lens::LensDatabase::load_bundled(),
    current_lens_profile: None,
    current_exif: None,
    save_status: None,
```

- [ ] **Step 2: Add new Message variants**

Add to the `Message` enum:

```rust
    ToggleEditPanel,
    SliderChanged(SliderKind, f32),
    SliderReleased,
    ResetAll,
    SaveEdited,
    SaveCompleted(Result<String, String>),
    ToggleLensCorrection,
```

Add a `SliderKind` enum:

```rust
#[derive(Debug, Clone, Copy)]
enum SliderKind {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Temperature,
    Tint,
    Vibrance,
    Saturation,
    Clarity,
    Dehaze,
}
```

- [ ] **Step 3: Add Edit button to tab bar**

In `tab_bar()`, add an "Edit" toggle button between the tab buttons and the add buttons:

```rust
let edit_btn = button(
    text(if self.edit_panel_open { "* Edit" } else { "  Edit" }).size(13),
)
.on_press(Message::ToggleEditPanel)
.padding([6, 16]);

let save_btn = button(text("Save").size(12))
    .on_press(Message::SaveEdited)
    .padding([5, 12]);
```

Add them to the tab bar row.

- [ ] **Step 4: Implement edit_panel() view method**

Add a method to App that returns the edit panel as an `Element`:

```rust
fn edit_panel(&self) -> Element<'_, Message> {
    let history = self.current_image_path.as_ref()
        .and_then(|p| self.edit_histories.get(p));
    let state = history.map(|h| &h.current)
        .copied()
        .unwrap_or_default();

    let light_section = column![
        text("Light").size(12).color(Color::from_rgb(0.6, 0.6, 0.6)),
        self.slider_row("Exposure", SliderKind::Exposure, state.exposure, -5.0, 5.0),
        self.slider_row("Contrast", SliderKind::Contrast, state.contrast, -100.0, 100.0),
        self.slider_row("Highlights", SliderKind::Highlights, state.highlights, -100.0, 100.0),
        self.slider_row("Shadows", SliderKind::Shadows, state.shadows, -100.0, 100.0),
        self.slider_row("Whites", SliderKind::Whites, state.whites, -100.0, 100.0),
        self.slider_row("Blacks", SliderKind::Blacks, state.blacks, -100.0, 100.0),
    ].spacing(4);

    let color_section = column![
        text("Color").size(12).color(Color::from_rgb(0.6, 0.6, 0.6)),
        self.slider_row("Temperature", SliderKind::Temperature, state.temperature, -100.0, 100.0),
        self.slider_row("Tint", SliderKind::Tint, state.tint, -100.0, 100.0),
        self.slider_row("Vibrance", SliderKind::Vibrance, state.vibrance, -100.0, 100.0),
        self.slider_row("Saturation", SliderKind::Saturation, state.saturation, -100.0, 100.0),
    ].spacing(4);

    let effects_section = column![
        text("Effects").size(12).color(Color::from_rgb(0.6, 0.6, 0.6)),
        self.slider_row("Clarity", SliderKind::Clarity, state.clarity, -100.0, 100.0),
        self.slider_row("Dehaze", SliderKind::Dehaze, state.dehaze, -100.0, 100.0),
    ].spacing(4);

    // Lens correction section
    let lens_label = match &self.current_lens_profile {
        Some(p) => format!("Lens: {}", p.model),
        None => "No lens profile found".to_string(),
    };
    let lens_section = column![
        text("Lens Correction").size(12).color(Color::from_rgb(0.6, 0.6, 0.6)),
        row![
            text(&lens_label).size(11).color(Color::from_rgb(0.5, 0.5, 0.5)),
            horizontal_space(),
            button(text(if state.lens_correction { "On" } else { "Off" }).size(11))
                .on_press_maybe(self.current_lens_profile.as_ref().map(|_| Message::ToggleLensCorrection))
                .padding([3, 8]),
        ],
    ].spacing(4);

    let reset_btn = button(text("Reset All").size(12))
        .on_press(Message::ResetAll)
        .padding([6, 16])
        .width(Length::Fill);

    let panel = column![
        light_section,
        color_section,
        effects_section,
        lens_section,
        reset_btn,
    ].spacing(16).padding(12).width(280);

    scrollable(panel).height(Length::Fill).into()
}
```

- [ ] **Step 5: Implement slider_row() helper**

```rust
fn slider_row(
    &self,
    label: &str,
    kind: SliderKind,
    value: f32,
    min: f32,
    max: f32,
) -> Element<'_, Message> {
    let display_value = if min == -5.0 {
        format!("{:.1}", value)
    } else {
        format!("{:.0}", value)
    };

    column![
        row![
            text(label).size(11).color(Color::from_rgb(0.7, 0.7, 0.7)),
            horizontal_space(),
            text(&display_value).size(11).color(Color::from_rgb(0.55, 0.55, 0.55)),
        ],
        iced::widget::slider(min..=max, value, move |v| Message::SliderChanged(kind, v))
            .step(if min == -5.0 { 0.05 } else { 1.0 })
            .on_release(Message::SliderReleased),
    ]
    .spacing(2)
    .into()
}
```

Note: iced 0.13's slider widget has `on_release` for detecting when the user finishes dragging. Check the iced 0.13 API — if `on_release` is not available, use a different approach (e.g., detect mouse release via subscription).

**Double-click knob reset:** iced's built-in slider does not support double-click on the knob. Implement this by wrapping the value text in a `button` with `on_press(Message::SliderChanged(kind, default))` — clicking the label text resets that slider to 0. This is more discoverable than double-clicking.

**Click-to-type value:** Wrap the value display in a `button`. On click, replace it with a `text_input` widget. Add `SliderTextInput(SliderKind)` and `SliderTextSubmit(SliderKind, String)` message variants. On Enter or focus loss, parse the text and apply the value. On ESC, cancel. Store `editing_slider: Option<SliderKind>` in App to track which slider is in text-edit mode.

Updated `slider_row()` to support both features:

```rust
fn slider_row(
    &self,
    label: &str,
    kind: SliderKind,
    value: f32,
    min: f32,
    max: f32,
) -> Element<'_, Message> {
    let display_value = if min == -5.0 {
        format!("{:.1}", value)
    } else {
        format!("{:.0}", value)
    };

    // Value display: clickable to enter text input mode
    let value_display: Element<'_, Message> = if self.editing_slider == Some(kind) {
        iced::widget::text_input("", &self.slider_text_buf)
            .on_input(move |s| Message::SliderTextChanged(s))
            .on_submit(Message::SliderTextSubmit(kind))
            .size(11)
            .width(45)
            .into()
    } else {
        button(text(&display_value).size(11).color(Color::from_rgb(0.55, 0.55, 0.55)))
            .on_press(Message::SliderTextInput(kind))
            .padding(0)
            .into()
    };

    // Label is clickable to reset to default
    let label_btn = button(text(label).size(11).color(Color::from_rgb(0.7, 0.7, 0.7)))
        .on_press(Message::SliderChanged(kind, 0.0))
        .padding(0);

    column![
        row![label_btn, horizontal_space(), value_display],
        iced::widget::slider(min..=max, value, move |v| Message::SliderChanged(kind, v))
            .step(if min == -5.0 { 0.05 } else { 1.0 })
            .on_release(Message::SliderReleased),
    ]
    .spacing(2)
    .into()
}
```

Add to App struct: `editing_slider: Option<SliderKind>`, `slider_text_buf: String`.

Add message variants: `SliderTextInput(SliderKind)`, `SliderTextChanged(String)`, `SliderTextSubmit(SliderKind)`.

Handle in update:
- `SliderTextInput(kind)`: set `editing_slider = Some(kind)`, populate `slider_text_buf` with current value
- `SliderTextChanged(s)`: update `slider_text_buf = s`
- `SliderTextSubmit(kind)`: parse `slider_text_buf`, clamp to range, apply via `SliderChanged`, clear `editing_slider`

- [ ] **Step 6: Update detail_view() to include edit panel**

```rust
fn detail_view(&self) -> Element<'_, Message> {
    let canvas: Element<'_, ViewerEvent> = shader(ImageCanvas {
        image: self.image.clone(),
        image_id: self.image_id,
        zoom: self.zoom,
        offset: self.offset,
        adjustments: self.build_adjustment_uniforms(),
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into();

    let viewer_with_status = column![canvas.map(Message::Viewer), self.status_bar()];

    if self.edit_panel_open {
        row![
            viewer_with_status.width(Length::Fill),
            self.edit_panel(),
        ].into()
    } else {
        viewer_with_status.into()
    }
}
```

- [ ] **Step 7: Handle new messages in update()**

Add match arms for the new messages:

```rust
Message::ToggleEditPanel => {
    self.edit_panel_open = !self.edit_panel_open;
    Task::none()
}

Message::SliderChanged(kind, value) => {
    if let Some(path) = &self.current_image_path {
        let history = self.edit_histories
            .entry(path.clone())
            .or_insert_with(edit::UndoHistory::new);
        match kind {
            SliderKind::Exposure => history.current.exposure = value,
            SliderKind::Contrast => history.current.contrast = value,
            SliderKind::Highlights => history.current.highlights = value,
            SliderKind::Shadows => history.current.shadows = value,
            SliderKind::Whites => history.current.whites = value,
            SliderKind::Blacks => history.current.blacks = value,
            SliderKind::Temperature => history.current.temperature = value,
            SliderKind::Tint => history.current.tint = value,
            SliderKind::Vibrance => history.current.vibrance = value,
            SliderKind::Saturation => history.current.saturation = value,
            SliderKind::Clarity => history.current.clarity = value,
            SliderKind::Dehaze => history.current.dehaze = value,
        }
    }
    Task::none()
}

Message::SliderReleased => {
    if let Some(path) = &self.current_image_path {
        if let Some(history) = self.edit_histories.get_mut(path) {
            history.commit();
        }
    }
    Task::none()
}

Message::ResetAll => {
    if let Some(path) = &self.current_image_path {
        let history = self.edit_histories
            .entry(path.clone())
            .or_insert_with(edit::UndoHistory::new);
        history.reset_all();
    }
    Task::none()
}

Message::ToggleLensCorrection => {
    if let Some(path) = &self.current_image_path {
        let history = self.edit_histories
            .entry(path.clone())
            .or_insert_with(edit::UndoHistory::new);
        history.current.lens_correction = !history.current.lens_correction;
        history.commit();
    }
    Task::none()
}
```

- [ ] **Step 8: Implement build_adjustment_uniforms()**

```rust
fn build_adjustment_uniforms(&self) -> viewer::AdjustmentUniforms {
    let state = self.current_image_path.as_ref()
        .and_then(|p| self.edit_histories.get(p))
        .map(|h| h.current)
        .unwrap_or_default();

    let temp_matrix = edit::temperature_tint_matrix(state.temperature, state.tint);

    let (lens_dist, lens_vig, lens_tca_r, lens_tca_b) = if state.lens_correction {
        match &self.current_lens_profile {
            Some(p) => {
                let dist = p.distortion.map(|d| [d.a, d.b, d.c]).unwrap_or([0.0; 3]);
                let vig = p.vignetting.map(|v| [v.k1, v.k2, v.k3]).unwrap_or([0.0; 3]);
                let tca_r = p.tca.map(|t| t.vr).unwrap_or(1.0);
                let tca_b = p.tca.map(|t| t.vb).unwrap_or(1.0);
                (dist, vig, tca_r, tca_b)
            }
            None => ([0.0; 3], [0.0; 3], 1.0, 1.0),
        }
    } else {
        ([0.0; 3], [0.0; 3], 1.0, 1.0)
    };

    let image_aspect = self.image.as_ref()
        .map(|img| img.width as f32 / img.height as f32)
        .unwrap_or(1.0);

    viewer::AdjustmentUniforms {
        exposure: state.exposure,
        contrast: state.contrast,
        highlights: state.highlights,
        shadows: state.shadows,
        whites: state.whites,
        blacks: state.blacks,
        vibrance: state.vibrance,
        saturation: state.saturation,
        clarity: state.clarity,
        dehaze: state.dehaze,
        temp_matrix,
        lens_enabled: state.lens_correction,
        lens_dist,
        lens_vig,
        lens_tca_r,
        lens_tca_b,
        image_aspect,
    }
}
```

- [ ] **Step 9: Wire EXIF reading on image load**

In the `Message::ImageLoaded(Ok(data))` handler, after storing the image, read EXIF and look up lens profile:

```rust
// Read EXIF and find lens profile
if let Some(path) = &self.current_image_path {
    self.current_exif = lens::read_exif(path);
    self.current_lens_profile = self.current_exif.as_ref().and_then(|exif| {
        let maker = if exif.lens_make.is_empty() { &exif.camera_make } else { &exif.lens_make };
        self.lens_db.find_lens(maker, &exif.lens_model).cloned()
    });
}
```

Also set `current_image_path` when starting a load — update `start_load()`, `LibraryItemClicked`, `FileSelected`, and `FileDropped` handlers to store the path.

- [ ] **Step 10: Add undo/redo keybinds**

In `handle_key()`, add before the existing zoom key handlers:

```rust
// Undo
Key::Character(ref c) if c.as_str() == "z" && mods.command() && !mods.shift() => {
    if let Some(path) = &self.current_image_path {
        if let Some(history) = self.edit_histories.get_mut(path) {
            history.undo();
        }
    }
    return Task::none();
}

// Redo
Key::Character(ref c)
    if (c.as_str() == "z" && mods.command() && mods.shift())
        || (c.as_str() == "y" && mods.command()) =>
{
    if let Some(path) = &self.current_image_path {
        if let Some(history) = self.edit_histories.get_mut(path) {
            history.redo();
        }
    }
    return Task::none();
}
```

- [ ] **Step 11: Run tests and build**

Run: `cargo fmt && cargo clippy -- -D warnings`
Expected: Clean.

Run: `cargo test`
Expected: All tests pass.

Run: `cargo build --release`
Expected: Compiles and runs with edit panel visible.

- [ ] **Step 12: Commit**

```bash
git add src/main.rs src/viewer.rs
git commit -m "feat: add edit panel UI with sliders, undo/redo, and lens correction toggle"
```

---

## Task 9: Save Edited Image

**Files:**
- Modify: `src/edit.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write test for save pipeline**

Add to `edit::tests`:

```rust
    #[test]
    fn apply_all_identity_preserves_pixel() {
        let state = EditState::default();
        let identity_mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let blurred = [0.5, 0.5, 0.5]; // doesn't matter at zero clarity/dehaze
        let input = [128, 64, 200, 255];
        let output = apply_all(input, &state, &identity_mat, blurred);
        // With identity state, output should be very close to input
        assert!((output[0] as i16 - input[0] as i16).abs() <= 1);
        assert!((output[1] as i16 - input[1] as i16).abs() <= 1);
        assert!((output[2] as i16 - input[2] as i16).abs() <= 1);
        assert_eq!(output[3], 255); // alpha preserved
    }
```

- [ ] **Step 2: Implement save_edited_image function**

Add to `src/edit.rs`:

```rust
/// Apply all edits and save to disk. Returns the output path on success.
pub fn save_edited_image(
    original_path: &Path,
    pixels: &[u8],
    width: u32,
    height: u32,
    state: &EditState,
) -> Result<PathBuf, String> {
    let temp_matrix = temperature_tint_matrix(state.temperature, state.tint);

    // Generate a simple blur for clarity/dehaze by averaging 4x4 blocks
    let blur = generate_cpu_blur(pixels, width, height);

    let mut output = Vec::with_capacity(pixels.len());
    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let srgb = [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]];

            // Get corresponding blurred pixel
            let bx = (x / 4).min(width / 4 - 1);
            let by = (y / 4).min(height / 4 - 1);
            let bw = width / 4;
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

/// Simple CPU blur: average 4x4 blocks, return linear RGB f32 values.
fn generate_cpu_blur(pixels: &[u8], width: u32, height: u32) -> Vec<f32> {
    let bw = width / 4;
    let bh = height / 4;
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
```

- [ ] **Step 3: Handle SaveEdited message in main.rs**

```rust
Message::SaveEdited => {
    if let (Some(path), Some(img)) = (&self.current_image_path, &self.image) {
        let state = self.edit_histories
            .get(path)
            .map(|h| h.current)
            .unwrap_or_default();

        if state.is_default() {
            return Task::none(); // Nothing to save
        }

        let path = path.clone();
        let pixels = img.pixels.clone();
        let width = img.width;
        let height = img.height;

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    edit::save_edited_image(&path, &pixels, width, height, &state)
                })
                .await
                .map_err(|e| e.to_string())?
            },
            |result| match result {
                Ok(path) => Message::SaveCompleted(Ok(
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file")
                        .to_string(),
                )),
                Err(e) => Message::SaveCompleted(Err(e)),
            },
        )
    } else {
        Task::none()
    }
}

Message::SaveCompleted(Ok(name)) => {
    self.save_status = Some(format!("Saved to {name}"));
    Task::none()
}

Message::SaveCompleted(Err(e)) => {
    self.error = Some(format!("Save failed: {e}"));
    Task::none()
}
```

- [ ] **Step 4: Add Ctrl+S keybind**

In `handle_key()`:

```rust
Key::Character(ref c) if c.as_str() == "s" && mods.command() => {
    return self.update(Message::SaveEdited);
}
```

- [ ] **Step 5: Show save status in status bar**

In `status_bar()`, if `self.save_status` is `Some`, show it instead of the normal status briefly. Clear it on next image action.

- [ ] **Step 6: Run tests and build**

Run: `cargo test`
Expected: All tests pass including the new `apply_all_identity_preserves_pixel`.

Run: `cargo build --release`
Expected: Compiles and runs.

- [ ] **Step 7: Commit**

```bash
git add src/edit.rs src/main.rs
git commit -m "feat: add save edited image with Ctrl+S"
```

---

## Task 10: Update Architecture Docs

**Files:**
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/devlog-detailed.md`
- Modify: `docs/devlog-summary.md`

- [ ] **Step 1: Update ARCHITECTURE.md**

Add `edit.rs` and `lens.rs` to the Component Map. Add the edit data flow. Add `kamadak-exif`, `quick-xml` to Technology Map. Update the diagram. Add drift log entry.

Key additions to Component Map:

```markdown
### Image Editing
- **edit** (`src/edit.rs`) — Edit state management, undo/redo history, CPU-side adjustment math (sRGB conversion, exposure, contrast, tone zones, vibrance, saturation, clarity, dehaze, temperature/tint via Bradford CAT). CPU-side save pipeline for full-resolution export.

### Lens Corrections
- **lens** (`src/lens.rs`) — Lensfun XML database parser, EXIF reader (via kamadak-exif), lens profile lookup. Provides distortion, vignetting, and TCA correction coefficients.
```

Add to Boundaries:

```markdown
- Only `edit.rs` knows about adjustment math and undo/redo history.
- Only `lens.rs` reads EXIF data and parses Lensfun XML. All Lensfun access is through this module.
```

- [ ] **Step 2: Append to devlog-detailed.md**

Use actual system time. Entry should cover the full image editing feature implementation.

- [ ] **Step 3: Update devlog-summary.md**

Add summary line for the editing feature.

- [ ] **Step 4: Commit**

```bash
git add docs/
git commit -m "docs: update architecture and devlog for image editing feature"
```

---

## Summary

| Task | What it produces | Tests |
|---|---|---|
| 1 | Dependencies in Cargo.toml | cargo check |
| 2 | EditState, UndoHistory | 7 unit tests |
| 3 | CPU adjustment math | 13 unit tests |
| 4 | WGSL shader pipeline | Runtime validation |
| 5 | Extended viewer uniforms | Existing tests pass |
| 6 | Blur pre-pass | Existing tests pass |
| 7 | Lensfun parser + EXIF | 6 unit tests |
| 8 | Edit panel UI + wiring | Build + manual test |
| 9 | Save edited image | 1 unit test + manual test |
| 10 | Architecture docs | N/A |

**Total new tests:** ~27
**Total files created:** 4 (`edit.rs`, `lens.rs`, `blur.wgsl`, `assets/lensfun/`)
**Total files modified:** 5 (`Cargo.toml`, `main.rs`, `viewer.rs`, `image.wgsl`, `ARCHITECTURE.md`)
