# API reference

The public surface of each crate. A crate appears here when it has one; crates still listed as "In progress" or "Planned" in `docs/architecture/ARCHITECTURE.md` are added as they land.

## photo-core

Shared data types. No I/O, no pixel loops.

### `photo_core::develop`

#### `DevelopSettings`

Everything that determines how a photo is rendered. Serialized as JSON; every field has a default and unknown fields are ignored. Fields: `process_version: u32`, `treatment: Treatment`, `profile: Profile`, `white_balance: WhiteBalance`, `tone: Tone`, `presence: Presence`, `tone_curve: ToneCurve`, `color_mixer: ColorMixer`, `color_grading: ColorGrading`, `detail: Detail`, `lens: LensCorrections`, `transform: Transform`, `effects: Effects`, `calibration: Calibration`, `orientation: Orientation`, `crop: Crop`, `spots: Vec<Spot>`, `masks: Vec<Mask>`.

- `DevelopSettings::default()` — the neutral state; rendering a raster file with it reproduces the file.
- `DevelopSettings::default_for(kind: FileKind) -> Self` — the starting settings for a newly imported photo. RAW starts with sharpening amount `RAW_SHARPEN_AMOUNT` (40) and colour noise reduction `RAW_COLOR_NOISE_REDUCTION` (25).
- `is_default_for(&self, kind) -> bool`.
- `sanitize_for(&mut self, kind)` — clamps every scalar adjustment into its range for `kind`. A custom white balance whose temperature is outside the range for `kind` (kelvin on a raster file or the reverse) falls back to as-shot.
- `copy_groups_from(&mut self, source: &Self, groups: GroupSet)` — overwrite the chosen groups, leave the rest.
- `differing_groups(&self, other: &Self) -> GroupSet`.

`PROCESS_VERSION: u32` is the version of the rendering math stored with every photo.

#### `Treatment`, `Profile`

`Treatment::{Color, BlackAndWhite}`. `Profile::{Standard, Neutral, Vivid, Linear}` is the base rendering applied to camera RAW data before any slider; it does not affect raster files. `Profile::ALL`, `Profile::label()`.

#### `WhiteBalance`, `WbPreset`

`WhiteBalance { preset, temp, tint }`. For camera RAW, `temp` is kelvin (2000–50000) and `tint` is on the DNG scale (−150 to 150, positive is magenta). For raster files both are relative offsets (−100 to 100; positive `temp` is warmer). With `WbPreset::AsShot` the numbers are ignored.

`WbPreset::{AsShot, Auto, Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash, Custom}`; `WbPreset::ALL`, `label()`, and `illuminant() -> Option<(f32, f32)>` giving the fixed (kelvin, tint) of the illuminant presets.

#### `Tone`, `Presence`

`Tone { exposure, contrast, highlights, shadows, whites, blacks }` — exposure in stops (−5 to 5), the rest −100 to 100. `Presence { texture, clarity, dehaze, vibrance, saturation }` — each −100 to 100.

#### `ToneCurve`, `ParametricCurve`, `PointCurve`, `CurvePoint`

`ToneCurve { parametric, rgb, red, green, blue }` with `is_identity()`. `ParametricCurve { highlights, lights, darks, shadows, split_shadows, split_midtones, split_highlights }` with `is_identity()`; the splits default to 25, 50 and 75.

`PointCurve` holds control points on a 0–1 square and keeps this invariant: at least two points, all in range, strictly increasing `x`. `PointCurve::new(points)` drops non-finite points, clamps, sorts and thins the rest so neighbours are at least `PointCurve::MIN_GAP` apart, and falls back to the identity when fewer than two remain. `points()`, `is_identity()`. Deserialization goes through `new`, so a stored curve is always valid.

#### `ColorBand`, `ColorMixer`, `ColorGrading`, `GradeWheel`, `Calibration`

`ColorBand::{Red, Orange, Yellow, Green, Aqua, Blue, Purple, Magenta}` with `COUNT`, `ALL`, `index()`, `label()`. `ColorMixer { hue, saturation, luminance, bw_mix }`, each `[f32; 8]` indexed by band; `hsl_is_identity()`. `ColorGrading { shadows, midtones, highlights, global: GradeWheel, blending, balance }` with `is_identity()`; `GradeWheel { hue (0–360), saturation (0–100), luminance (−100 to 100) }`. `Calibration { shadows_tint, red_hue, red_saturation, green_hue, green_saturation, blue_hue, blue_saturation }`.

#### `Detail`, `Sharpening`, `NoiseReduction`, `Effects`, `Vignette`, `Grain`

`Detail { sharpening, noise_reduction }`. `Sharpening { amount (0–150), radius (0.5–3.0), detail, masking }`. `NoiseReduction { luminance, luminance_detail, luminance_contrast, color, color_detail, color_smoothness }`. `Effects { vignette, grain }`. `Vignette { amount, midpoint, roundness, feather, highlights }`. `Grain { amount, size, roughness }`.

#### Geometry: `Orientation`, `Crop`, `CropRect`, `CropConstraint`, `Transform`, `LensCorrections`

Frames of reference, in pipeline order: source → corrected → oriented → transformed → output. Normalized coordinates run 0 to 1 across a frame.

`Orientation { quarter_turns, flip_horizontal }` — mirror first, then clockwise quarter turns. `Orientation::IDENTITY`, `new(turns, flip)` (turns taken modulo 4), `rotated_cw()`, `rotated_ccw()`, `flipped_horizontal()`, `flipped_vertical()` (each composes in the already-oriented view), `swaps_axes()`, `apply_to_size(w, h)`, `forward(p)` and `inverse(p)` on normalized points.

`CropRect { cx, cy, width, height }` in normalized transformed-frame coordinates, sides measured along the rectangle's own axes. `CropRect::FULL`, `MIN_SIDE`, `corners(angle_degrees, frame_aspect) -> [Point; 4]`, `fits(angle, frame_aspect)`, `inscribed(angle, frame_aspect, crop_aspect)` (the largest centred rectangle of that aspect that fits when rotated), `aspect(frame_aspect)`.

`Crop { rect: Option<CropRect>, angle, constraint }` with `resolved(frame_aspect) -> CropRect` (an unset rectangle means the full frame, or the largest inscribed one when `angle` is non-zero) and `is_identity()`. `CropConstraint::{Original, Free, Ratio { width, height }}`.

`Transform { vertical, horizontal, rotate, aspect, scale, x_offset, y_offset }` with `is_identity()`. `LensCorrections { distortion, vignetting, vignetting_midpoint }`.

#### Local edits: `Spot`, `SpotMode`, `Mask`, `MaskComponent`, `MaskShape`, `BrushStroke`, `LocalAdjustments`

Positions are normalized source-frame coordinates; lengths are fractions of the source's long edge.

`Spot { mode: SpotMode::{Heal, Clone}, target, source, radius, feather, opacity }`. `Mask { name, enabled, invert, amount, components, adjust }` with `Mask::from_shape(shape, number)` and `is_inert()`. `MaskComponent { shape, subtract }`. `MaskShape::{Linear { start, end }, Radial { center, radius_x, radius_y, angle, feather, invert }, Brush { strokes }}` with `label()`. `BrushStroke { points, radius, feather, flow, erase }`. `LocalAdjustments { exposure, contrast, highlights, shadows, whites, blacks, temp, tint, saturation, texture, clarity, dehaze, sharpness, noise }` with `is_identity()`.

#### The adjustment table: `Adj`, `AdjSpec`

`Adj` enumerates every scalar adjustment: 70 named variants plus `MixHue(ColorBand)`, `MixSaturation(ColorBand)`, `MixLuminance(ColorBand)` and `MixBw(ColorBand)`. It is the single source of each slider's key, label, range and step.

- `Adj::SCALARS`, `Adj::all()`.
- `spec() -> AdjSpec` and `spec_for(kind) -> AdjSpec` (camera RAW temperature is in kelvin, `KELVIN_MIN`..`KELVIN_MAX`; RAW tint is ±`RAW_TINT_LIMIT`).
- `get(&DevelopSettings) -> f32`, `set(&mut DevelopSettings, f32)` (writes as given; setting temperature or tint does not change the preset).
- `default_for(kind) -> f32`, `from_key(&str) -> Option<Adj>`.

`AdjSpec { key, label, min, max, step, decimals, group }` with `clamp(value)` (non-finite input becomes the neutral value) and `format(value)` (signed when the range is signed).

#### Groups: `SettingsGroup`, `GroupSet`

`SettingsGroup::{Treatment, WhiteBalance, Tone, Presence, ToneCurve, ColorMixer, ColorGrading, Sharpening, NoiseReduction, LensCorrections, Transform, Vignette, Grain, Calibration, Crop, Healing, Masking}` with `ALL` and `label()`. `GroupSet`: `NONE`, `all()`, `default_copy()` (everything except crop, healing and masking), `contains`, `with`, `without`, `set`, `is_empty`, `iter`; serializes as a list of group names.

#### `describe_change(before, after, kind) -> String`

A history-step label for the difference between two states: a single slider with its value (`Exposure +0.35`), a structural change by name (`Convert to Black & White`, `Crop Rectangle`, `Add Mask`), a group name when several sliders in one group changed, otherwise `Multiple Settings`.

### `photo_core::meta`

Id newtypes over `i64`: `PhotoId`, `FolderId`, `CollectionId`, `KeywordId`, `ImportId`, `PresetId`, `SnapshotId`, `HistoryId`.

`Rating` (0–5): `new` (clamps), `UNRATED`, `stars()`, `increased()`, `decreased()`. `Flag::{Rejected, Unflagged, Picked}` with `to_i8()` / `from_i8()` (−1, 0, 1). `ColorLabel::{None, Red, Yellow, Green, Blue, Purple}` with `ALL`, `name()` and case-insensitive `from_name()`. `FileKind::{Raster, Raw}`.

`PhotoMetadata { width, height, capture_time, camera_make, camera_model, lens, focal_length_mm, focal_length_35mm, aperture, shutter_seconds, iso, exposure_bias_ev, flash_fired, gps, artist, copyright }` with `camera_name()`. `GpsPosition { latitude, longitude, altitude_m }`. `format_shutter(seconds) -> String` (`1/250`, `0.4"`, `30"`).

### `photo_core::time`

`DateTime { year, month, day, hour, minute, second }`: a civil date-time that orders chronologically and whose ISO 8601 text sorts the same way. `new` (rejects impossible values), `from_unix_seconds`, `to_unix_seconds`, `parse_exif` (`YYYY:MM:DD HH:MM:SS`, tolerant of `-` and `T`), `date_string()`, `Display` and `FromStr` in ISO form, serde as ISO text. `ParseDateTimeError`.

### `photo_core::image`

`SourceImage { width, height, pixels: Vec<u16>, color }`: interleaved linear RGB where `SourceImage::WHITE` (65535) is 1.0, upright per EXIF. `new` validates the buffer length; `pixel(x, y)`, `long_edge()`.

`SourceColor::{Rgb(RgbSpace), Camera(CameraColor)}` with `is_camera()`. `CameraColor { xyz_to_camera, as_shot_gains, baseline_exposure }` describes demosaiced camera-native data that is not white balanced.

`Rgba8Image { width, height, pixels, space }` (display) and `Rgb16Image { width, height, pixels, space }` (export), each with `pixel(x, y)`.

### `photo_core::colorimetry`

`RgbSpace::{Srgb, DisplayP3, AdobeRgb, ProPhoto, Rec2020}`: `ALL`, `name()`, `primaries()`, `to_xyz()`, `from_xyz()`, `decode(encoded) -> linear`, `encode(linear) -> encoded` (both odd-symmetric and monotonic).

`WORKING_SPACE` (Rec. 2020), `WORKING_LUMA`, `luminance(rgb)`. `Mat3`, `IDENTITY`, `mat3_apply`, `mat3_mul`, `mat3_invert`. `rgb_to_rgb(from, to)` (includes Bradford adaptation; white maps to white), `xyz_d65_to_working()`, `bradford(from_white, to_white)`. `Chromaticity { x, y }` with `D65`, `D50`, `to_xyz()`; `Primaries { red, green, blue, white }`.

### `photo_core::geom`

`Point { x, y }`: `ZERO`, `new`, `distance`, `lerp`. `Rect { x0, y0, x1, y1 }`: `new`, `from_size`, `width`, `height`, `center`, `is_empty`, `contains`, `intersect`, `expand`, `translate`.
