# Image Editing Feature — Design Spec

> Date: 2026-03-30
> Status: Approved

## Overview

Add real-time image editing to the Detail tab via GPU shader-based adjustments. All color math runs in the WGSL fragment shader using uniform values, so slider drags are as fast as zoom/pan (uniform buffer update only, no texture re-upload). Lens profile corrections use Lensfun's open-source database (~900 lenses). Edits are non-destructive — the original image is never modified. A save action writes an edited copy with `_edited` suffix.

## Architecture

### Approach: Shader-based adjustments

All 12 color adjustments are passed as uniform floats to the WGSL fragment shader. The shader applies them per-pixel during rendering. This means:

- Slider drags update ~128 bytes of uniform data per frame — same cost as zoom/pan
- The original image texture is never re-uploaded or modified
- No CPU pixel processing during editing (only during save)

### Render Pipeline

```
Current:  Image Texture -> Single Pass (rect + bg_color) -> Screen

Proposed: Image Texture -> Pass 1: Gaussian blur at 1/4 res (for clarity/dehaze)
                        -> Pass 2: Main composite (all 12 adjustments + lens) -> Screen
```

The blur texture is rendered once per image load, not per slider drag. It provides the "blurred version" needed for clarity (local contrast) and dehaze (atmospheric light estimation).

### Three Categories of Work

1. **Per-pixel color math (shader uniforms):** Exposure, contrast, highlights, shadows, whites, blacks, temperature, tint, vibrance, saturation. Pure uniform-driven, near-zero cost per frame.

2. **Multi-pass effects (blur texture):** Clarity and dehaze need a blurred copy. Rendered at 1/4 resolution via two-pass separable Gaussian. Only re-rendered when image changes.

3. **Lens corrections (UV remapping + radial multiply):** Distortion remaps texture UVs via polynomial. Vignetting is a radial brightness correction. TCA samples R/G/B at different UV offsets. Coefficients from Lensfun passed as uniforms.

## Shader Color Math

### Processing Order

Matches the standard professional RAW processing pipeline:

```
sRGB texture sample
  -> linearize (sRGB -> linear RGB)
  -> exposure (in linear)
  -> temperature/tint (in linear, Bradford CAT)
  -> highlights/shadows/whites/blacks (luminance zone masks)
  -> contrast (S-curve on luminance)
  -> vibrance/saturation (in linear RGB)
  -> clarity/dehaze (local contrast, uses blur texture)
  -> lens vignetting correction (radial multiply)
  -> gamma encode (linear -> sRGB)
  -> output
```

### Adjustment Formulas

**Luminance:** `lum = dot(pixel, vec3(0.2126, 0.7152, 0.0722))` (Rec. 709)

**sRGB linearization:** `x <= 0.04045 ? x/12.92 : pow((x+0.055)/1.055, 2.4)`

All slider values in the -100..+100 range are normalized to -1.0..+1.0 before passing to the shader. Exposure uses its raw -5.0..+5.0 value directly.

| Adjustment | Formula (using normalized amount `a` in -1..+1 unless noted) | Color Space | Slider Range |
|---|---|---|---|
| Exposure | `pixel * 2^EV` (EV is raw slider value, -5 to +5) | Linear RGB | -5.0 to +5.0 (stops) |
| Temperature/Tint | `Bradford_3x3 * pixel` (matrix precomputed on CPU from Kelvin via CIE daylight chromaticity) | Linear RGB | -100 to +100 |
| Highlights | `pixel + a * smoothstep(0.5, 1.0, lum)` | Linear, luminance-masked | -100 to +100 |
| Shadows | `pixel + a * (1 - smoothstep(0.0, 0.5, lum))` | Linear, luminance-masked | -100 to +100 |
| Whites | `pixel + a * smoothstep(0.85, 1.0, lum)` | Linear, luminance-masked | -100 to +100 |
| Blacks | `pixel + a * (1 - smoothstep(0.0, 0.15, lum))` | Linear, luminance-masked | -100 to +100 |
| Contrast | Sigmoid S-curve: `k = 1 + a * 4` (range 0.6..5.0), `lum' = 1/(1 + exp(-k*(lum - 0.5)))`, RGB scaled by `lum'/lum` | On luminance | -100 to +100 |
| Vibrance | `weight = 1 + a * (1 - current_sat)`, scale saturation by weight. `current_sat = (max(R,G,B) - min(R,G,B)) / max(R,G,B)` | Linear RGB | -100 to +100 |
| Saturation | `mix(vec3(lum), pixel, 1 + a)` | Linear RGB | -100 to +100 |
| Clarity | `pixel + a * (pixel - blurred_pixel) * midtone_mask` where `midtone_mask = smoothstep(0,0.5,lum) * (1-smoothstep(0.5,1,lum))` | Linear RGB, blur texture | -100 to +100 |
| Dehaze | `J = (pixel - A) / max(t, 0.1) + A` where `t = 1 - a * min(R,G,B)/A`, A estimated from blur texture max channel | Linear RGB | -100 to +100 |

### Temperature/Tint Implementation Detail

- CPU computes a 3x3 Bradford chromatic adaptation matrix from the temperature/tint values
- Temperature maps to a CIE daylight chromaticity point using the standard two-range polynomial (4000-7000K and 7000-25000K)
- Tint shifts along the green-magenta axis perpendicular to the Planckian locus
- The combined `M_A_inv * D * M_A` matrix is passed as a uniform — per-pixel cost is one 3x3 matrix multiply

### Blur Pre-Pass for Clarity/Dehaze

- Render image at 1/4 resolution
- Two-pass separable Gaussian blur (horizontal then vertical)
- Output stored as a second texture bound to the main pass
- Only re-rendered when the image changes, not on slider drags
- Main pass samples both the sharp original and blur texture

## Lens Corrections (Lensfun)

### EXIF Reading

On image load, read EXIF tags for camera make/model and lens name using the `kamadak-exif` crate. Use these to look up the lens in the Lensfun database.

### Lensfun Database

Bundle Lensfun's XML database files. Parse with `quick-xml`. The database contains ~900 lens profiles with calibrated correction coefficients.

### Three Correction Types

**1. Distortion — UV coordinate remapping**

PTLens polynomial model (most common in Lensfun):
```
r_corrected = r * (a * r^3 + b * r^2 + c * r + 1 - a - b - c)
```
Coordinates normalized to image half-diagonal. The shader remaps texture UVs before sampling.

**2. Vignetting — radial brightness correction**
```
correction = 1 + k1 * r^2 + k2 * r^4 + k3 * r^6
pixel *= correction
```
Applied as a per-pixel multiply. r is distance from image center, normalized.

**3. Chromatic Aberration (TCA) — per-channel UV shift**
```
R = sample(uv_corrected_r)
G = sample(uv_corrected_g)   // reference channel
B = sample(uv_corrected_b)
```
Each channel gets slightly different UV correction using per-channel polynomial coefficients.

### Workflow

- Image load -> read EXIF -> look up lens in Lensfun DB -> extract coefficients
- Pass coefficients as uniforms (~12 floats)
- UI shows toggle: "Lens Correction: [On/Off]" with detected lens name
- If no lens found, toggle is grayed out: "No lens profile found"
- Toggling is undoable

## UI Design

### Edit Panel

Right-side panel (~280px wide), toggled via "Edit" button in the tab bar. Image viewer shrinks to accommodate. Panel has four collapsible sections:

```
+------------------------------------------+----------+
|                                          | Light    |
|                                          |  Exposure |
|            Image Viewer                  |  Contrast |
|          (zoom/pan still works)          |  Highlights|
|                                          |  Shadows  |
|                                          |  Whites   |
|                                          |  Blacks   |
|                                          |----------|
|                                          | Color    |
|                                          |  Temp    |
|                                          |  Tint    |
|                                          |  Vibrance|
|                                          |  Saturati|
|                                          |----------|
|                                          | Effects  |
|                                          |  Clarity |
|                                          |  Dehaze  |
|                                          |----------|
|                                          | Lens     |
|                                          |  [On/Off]|
|                                          |----------|
|                                          |[Reset All]|
+------------------------------------------+----------+
|  status bar                                         |
+----------------------------------------------------+
```

### Slider Interaction

Each slider row layout:
```
  Label               [value]
  ------[====|=========]------
```

- Slider track with center notch at zero
- Value displayed to the right of the label
- **Drag slider** -> image updates in real time
- **Double-click the knob** -> resets that slider to its default (0)
- **Click on the value text** -> spawns inline text input. Enter or click-outside confirms. ESC cancels.

### Buttons

- **Edit** (tab bar) — toggles the edit panel open/closed
- **Reset All** (bottom of edit panel) — sets all 12 sliders to 0, lens correction to off. Undoable.
- **Save** (tab bar, or Ctrl+S) — writes edited copy

## Undo/Redo

### Data Model

```rust
struct EditState {
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    temperature: f32,
    tint: f32,
    vibrance: f32,
    saturation: f32,
    clarity: f32,
    dehaze: f32,
    lens_correction: bool,
}

struct UndoHistory {
    states: Vec<EditState>,   // past states
    redo: Vec<EditState>,     // future states
    current: EditState,       // live state driving the shader
}
```

### Behavior

- **Slider drag ends** (mouse release): push `current` onto `states`, clear `redo`
- **Ctrl+Z**: push `current` onto `redo`, pop `states` into `current`
- **Ctrl+Shift+Z**: push `current` onto `states`, pop `redo` into `current`
- **Reset All**: normal edit (pushed to undo stack, so it's undoable)
- **Switch image**: `UndoHistory` stored per-image in `HashMap<PathBuf, UndoHistory>`. Switching back restores state.
- **Close app**: all history discarded, no persistence

## Save

1. User presses Ctrl+S or clicks Save button
2. CPU applies all adjustments to full-resolution pixels (same math as shader, implemented in Rust in `edit.rs`)
3. Lens corrections applied using Lensfun coefficients
4. Writes to `{original_stem}_edited.{ext}` (e.g., `photo_edited.jpg`)
5. If `_edited` file already exists, overwrites it silently
6. Status bar shows "Saved to photo_edited.jpg" briefly

CPU save is necessary because the shader renders at screen resolution, but the saved file needs full original resolution.

## Module Structure

### New Files

- `src/edit.rs` — `EditState`, `UndoHistory`, CPU-side adjustment math (Rust mirror of shader), save pipeline
- `src/lens.rs` — Lensfun XML parser, EXIF reader, lens profile lookup, correction coefficient extraction
- `assets/lensfun/` — Bundled Lensfun XML database files

### Modified Files

- `src/main.rs` — Edit panel UI (sliders, text inputs, undo/redo keybinds, save action, edit toggle in tab bar)
- `src/viewer.rs` — Extended `Uniforms` struct (12 floats + lens coefficients + blur texture binding), blur pre-pass
- `assets/shaders/image.wgsl` — Full adjustment pipeline in fragment shader
- `Cargo.toml` — New dependencies

### New Dependencies

| Crate | Purpose |
|---|---|
| `kamadak-exif` | Read camera/lens EXIF data (already transitive dep of image crate) |
| `quick-xml` | Parse Lensfun XML database |

### Boundary Rules

- Only `edit.rs` knows about adjustment math and undo history
- Only `lens.rs` reads EXIF and parses Lensfun XML
- Only `viewer.rs` touches wgpu — receives adjustment values and lens coefficients as plain data
- `main.rs` coordinates: UI sliders -> `EditState` -> viewer uniforms

## Testing Strategy

- **Unit tests for `edit.rs`:** Verify each adjustment formula produces expected output for known inputs. Test undo/redo stack behavior. Test save filename generation.
- **Unit tests for `lens.rs`:** Parse sample Lensfun XML, verify coefficient extraction. Test EXIF tag reading on sample images. Test lens lookup matching.
- **Integration tests:** Load image, apply adjustments, verify pixel values change. Verify reset returns to original values.
- **Manual testing:** Slider responsiveness, visual correctness against Lightroom reference.
