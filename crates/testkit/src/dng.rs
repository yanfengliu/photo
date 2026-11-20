//! A synthetic camera RAW file: an uncompressed 16-bit Bayer (RGGB) DNG.
//!
//! The scene is photographed by an imaginary camera whose colour response is
//! fully known ([`dng_camera`]), so a test can undo it exactly:
//!
//! 1. The linear scene (sRGB primaries, D65) is converted to XYZ.
//! 2. `A'`, the camera's D65 colour matrix scaled so that D65 white lands on
//!    (1, 1, 1), takes XYZ to white-balanced camera RGB.
//! 3. Each channel is multiplied by the as-shot neutral, which is what makes
//!    the stored data *not* white balanced, exactly as a sensor delivers it.
//! 4. Values clip at 1.0 (sensor saturation), are scaled between the black
//!    and white levels, and one channel per pixel is kept in an RGGB mosaic.
//!
//! Multiplying a decoded pixel by the as-shot gains and applying the inverse
//! of `A'` therefore returns the scene's XYZ, wherever nothing clipped.

use std::path::{Path, PathBuf};

use image::RgbImage;
use image::imageops::thumbnail;
use photo_core::colorimetry::{Chromaticity, Mat3, RgbSpace, mat3_apply, mat3_mul};

use crate::exif::{ExifSpec, add_exif};
use crate::orient::{stored_size, to_stored};
use crate::scene::{radiance, scene};
use crate::tiff::{Ifd, TiffWriter, Value};
use crate::writers::{fixture_path, jpeg_bytes, stored_rgb, write_bytes};

const BLACK_LEVEL: u16 = 512;
const WHITE_LEVEL: u16 = 16383;
/// `BaselineExposure`, in hundredths of a stop.
const BASELINE_EXPOSURE_CENTISTOPS: i32 = 25;
/// Masked sensor columns and rows outside the active area: left, top, right,
/// bottom. Odd offsets on purpose, so the mosaic phase has to be tracked.
const MASKED_BORDER: [usize; 4] = [5, 3, 3, 1];
/// Active pixels the default crop trims from every side.
const CROP_MARGIN: usize = 4;
/// The colour matrix for D65, in ten-thousandths (the values Adobe publishes
/// for the Sony ILCE-7RM3).
const COLOR_MATRIX_D65: [i32; 9] = [6640, -1847, -503, -5238, 13010, 2474, -993, 1673, 6527];
/// A deliberately different matrix filed under Standard Illuminant A. A
/// reader that takes the first matrix instead of the D65 one gets visibly
/// wrong colours.
const COLOR_MATRIX_A: [i32; 9] = [9847, -3091, -928, -8485, 16345, 2225, -715, 595, 7103];
const MATRIX_DENOMINATOR: i32 = 10_000;
const NEUTRAL_DENOMINATOR: u32 = 1_000_000;

/// The colour response of the camera behind every DNG fixture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DngCamera {
    /// XYZ (D65) to camera-native RGB: the file's `ColorMatrix` for D65.
    pub xyz_to_camera: Mat3,
    /// The file's `AsShotNeutral`: camera-native RGB of a neutral object,
    /// green normalized to 1. It is this camera's response to D50, so the
    /// as-shot white balance is about 5000 K with next to no tint.
    pub as_shot_neutral: [f32; 3],
    /// Raw code value of black.
    pub black_level: u16,
    /// Raw code value at which the sensor saturates.
    pub white_level: u16,
    /// The file's `BaselineExposure`, in stops.
    pub baseline_exposure: f32,
}

impl DngCamera {
    /// White-balance multipliers that neutralize [`Self::as_shot_neutral`],
    /// green normalized to 1.
    pub fn as_shot_gains(&self) -> [f32; 3] {
        self.as_shot_neutral.map(|channel| 1.0 / channel)
    }

    /// `A'`: XYZ (D65) to white-balanced camera RGB, scaled so that D65 white
    /// maps to (1, 1, 1).
    pub fn xyz_to_balanced_camera(&self) -> Mat3 {
        let d65 = Chromaticity::D65.to_xyz().map(|v| v as f32);
        let white = mat3_apply(&self.xyz_to_camera, d65);
        let mut matrix = self.xyz_to_camera;
        for (row, response) in matrix.iter_mut().zip(white) {
            for value in row.iter_mut() {
                *value /= response;
            }
        }
        matrix
    }
}

/// The camera every DNG fixture is shot with.
pub fn dng_camera() -> DngCamera {
    let xyz_to_camera = matrix(&COLOR_MATRIX_D65);
    let neutral = neutral_fractions(&xyz_to_camera);
    DngCamera {
        xyz_to_camera,
        as_shot_neutral: neutral.map(|n| n as f32 / NEUTRAL_DENOMINATOR as f32),
        black_level: BLACK_LEVEL,
        white_level: WHITE_LEVEL,
        baseline_exposure: BASELINE_EXPOSURE_CENTISTOPS as f32 / 100.0,
    }
}

fn matrix(ten_thousandths: &[i32; 9]) -> Mat3 {
    let entry = |index: usize| ten_thousandths[index] as f32 / MATRIX_DENOMINATOR as f32;
    std::array::from_fn(|row| std::array::from_fn(|column| entry(row * 3 + column)))
}

/// Numerators of the as-shot neutral over [`NEUTRAL_DENOMINATOR`]: the
/// camera's response to D50 with green normalized to 1.
fn neutral_fractions(xyz_to_camera: &Mat3) -> [u32; 3] {
    let response = mat3_apply(xyz_to_camera, Chromaticity::D50.to_xyz().map(|v| v as f32));
    let scale = f64::from(NEUTRAL_DENOMINATOR);
    response.map(|channel| (f64::from(channel / response[1]) * scale).round() as u32)
}

/// What a DNG fixture should contain besides the mosaic.
#[derive(Clone, Debug, PartialEq)]
pub struct DngSpec {
    /// EXIF to write. Its orientation is the one the sensor data is stored
    /// in: as with the EXIF raster fixtures, the mosaic is turned so that a
    /// reader honouring the tag shows the scene upright.
    pub exif: ExifSpec,
    /// Lay the file out the way raw converters do, with a small uncompressed
    /// thumbnail in the first directory and the raw data and a half-size JPEG
    /// preview in sub-directories. Without it the raw data sits alone in the
    /// first directory and the file has no embedded picture at all.
    pub embed_previews: bool,
}

impl Default for DngSpec {
    /// Upright, with [`ExifSpec::sample`] metadata and embedded previews.
    fn default() -> Self {
        Self {
            exif: ExifSpec::sample(),
            embed_previews: true,
        }
    }
}

/// Size of the JPEG preview embedded in a `width` x `height` DNG fixture, as
/// seen upright: half the picture in each direction.
pub fn dng_preview_size(width: u32, height: u32) -> (u32, u32) {
    ((width / 2).max(1), (height / 2).max(1))
}

/// Writes the scene as a DNG that decodes to `width` x `height` pixels, with
/// the default [`DngSpec`].
///
/// The sensor frame is larger than the picture: a masked border surrounds the
/// active area, and the default crop trims a few more pixels from every side.
/// The file declares black and white levels, a decoy colour matrix for
/// illuminant A next to the real one for D65, an as-shot neutral, a baseline
/// exposure and basic EXIF. See [`dng_camera`] for the numbers.
pub fn write_dng(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    write_dng_with(dir, name, width, height, &DngSpec::default())
}

/// As [`write_dng`], with the metadata, orientation and layout of `spec`.
pub fn write_dng_with(dir: &Path, name: &str, width: u32, height: u32, spec: &DngSpec) -> PathBuf {
    assert!(width > 0 && height > 0, "a DNG fixture cannot be empty");
    let path = fixture_path(dir, name);
    let orientation = spec.exif.orientation;
    let sensor = Sensor::expose(width, height, orientation);

    let mut writer = TiffWriter::new();
    let samples = sensor.mosaic.iter();
    let strip: Vec<u8> = samples.flat_map(|sample| sample.to_le_bytes()).collect();
    let strip_offset = writer.data(&strip);
    let raw_ifd = sensor.raw_ifd(strip_offset, strip.len());

    let mut ifd0 = if spec.embed_previews {
        let upright = scene(width, height);
        let (preview_width, preview_height) = dng_preview_size(width, height);
        let preview = thumbnail(&upright, preview_width, preview_height);
        let preview = stored_rgb(&preview, orientation);
        let small = thumbnail(&upright, (width / 8).max(1), (height / 8).max(1));
        let small = stored_rgb(&small, orientation);

        let jpeg = jpeg_bytes(&preview);
        let jpeg_offset = writer.data(&jpeg);
        let mut preview_ifd = rgb_ifd(&preview, jpeg_offset, jpeg.len());
        // Compression 7 (JPEG) holding YCbCr.
        preview_ifd.short(0x0103, 7).short(0x0106, 6);

        let small_offset = writer.data(small.as_raw());
        let sub_ifds = vec![writer.ifd(&raw_ifd), writer.ifd(&preview_ifd)];
        let mut ifd0 = rgb_ifd(&small, small_offset, small.as_raw().len());
        ifd0.set(0x014a, Value::Long(sub_ifds));
        ifd0
    } else {
        raw_ifd
    };

    add_camera_tags(&mut ifd0, spec);
    add_exif(&mut writer, &mut ifd0, &spec.exif);
    let first = writer.ifd(&ifd0);
    write_bytes(&path, &writer.finish(first));
    path
}

/// The exposed sensor: the full frame of raw code values and where the
/// picture sits inside it.
struct Sensor {
    mosaic: Vec<u16>,
    frame_width: usize,
    frame_height: usize,
    /// Size of the default crop, in stored (sensor) orientation.
    crop_width: usize,
    crop_height: usize,
}

impl Sensor {
    fn expose(width: u32, height: u32, orientation: u8) -> Self {
        let camera = dng_camera();
        // Scene (linear sRGB) to raw camera RGB: XYZ, then A', then the
        // as-shot neutral.
        let to_xyz = RgbSpace::Srgb.to_xyz();
        let mut scene_to_raw = mat3_mul(&camera.xyz_to_balanced_camera(), &to_xyz);
        for (row, neutral) in scene_to_raw.iter_mut().zip(camera.as_shot_neutral) {
            for value in row.iter_mut() {
                *value *= neutral;
            }
        }

        // The upright scene with the crop margin around it, then turned into
        // sensor orientation. The margin is the same on every side, so it
        // stays a plain margin after the turn.
        let extended_width = width as usize + 2 * CROP_MARGIN;
        let extended_height = height as usize + 2 * CROP_MARGIN;
        let margin = CROP_MARGIN as i64;
        let mut upright = Vec::with_capacity(extended_width * extended_height);
        for y in 0..extended_height as i64 {
            for x in 0..extended_width as i64 {
                upright.push(radiance(x - margin, y - margin, width, height));
            }
        }
        let (active, active_width, active_height) =
            to_stored(&upright, extended_width, extended_height, orientation);

        let [left, top, right, bottom] = MASKED_BORDER;
        let frame_width = left + active_width + right;
        let frame_height = top + active_height + bottom;
        let mut mosaic = vec![BLACK_LEVEL; frame_width * frame_height];
        let range = f32::from(WHITE_LEVEL - BLACK_LEVEL);
        for (y, row) in active.chunks_exact(active_width).enumerate() {
            for (x, &linear) in row.iter().enumerate() {
                // RGGB, counted from the top-left of the active area.
                let channel = [0, 1, 1, 2][(y % 2) * 2 + x % 2];
                let signal = mat3_apply(&scene_to_raw, linear)[channel].clamp(0.0, 1.0);
                let code = BLACK_LEVEL + (signal * range).round() as u16;
                mosaic[(top + y) * frame_width + left + x] = code;
            }
        }

        let (crop_width, crop_height) = stored_size(orientation, width, height);
        Self {
            mosaic,
            frame_width,
            frame_height,
            crop_width: crop_width as usize,
            crop_height: crop_height as usize,
        }
    }

    fn raw_ifd(&self, strip_offset: u32, strip_len: usize) -> Ifd {
        let [left, top, right, bottom] = MASKED_BORDER.map(|v| v as u32);
        let (frame_width, frame_height) = (self.frame_width as u32, self.frame_height as u32);
        let crop_size = vec![self.crop_width as u32, self.crop_height as u32];
        // Active area: top, left, bottom, right.
        let active_area = vec![top, left, frame_height - bottom, frame_width - right];
        let mut ifd = Ifd::new();
        ifd.long(0x00fe, 0)
            .long(0x0100, frame_width)
            .long(0x0101, frame_height)
            .short(0x0102, 16)
            .short(0x0103, 1)
            // Photometric interpretation: colour filter array.
            .short(0x0106, 32803)
            .long(0x0111, strip_offset)
            .short(0x0115, 1)
            .long(0x0116, frame_height)
            .long(0x0117, strip_len as u32)
            .short(0x011c, 1)
            // CFA repeat pattern 2 x 2: red, green / green, blue.
            .set(0x828d, Value::Short(vec![2, 2]))
            .set(0x828e, Value::Byte(vec![0, 1, 1, 2]))
            .set(0xc616, Value::Byte(vec![0, 1, 2]))
            .short(0xc617, 1)
            .set(0xc619, Value::Short(vec![1, 1]))
            .set(0xc61a, Value::Short(vec![BLACK_LEVEL]))
            .set(0xc61d, Value::Short(vec![WHITE_LEVEL]))
            .set(0xc61e, Value::Rational(vec![(1, 1), (1, 1)]))
            .set(0xc61f, Value::Long(vec![CROP_MARGIN as u32; 2]))
            .set(0xc620, Value::Long(crop_size))
            .set(0xc68d, Value::Long(active_area));
        ifd
    }
}

/// Directory entries describing a single-strip 8-bit RGB image.
fn rgb_ifd(image: &RgbImage, strip_offset: u32, strip_len: usize) -> Ifd {
    let mut ifd = Ifd::new();
    ifd.long(0x00fe, 1)
        .long(0x0100, image.width())
        .long(0x0101, image.height())
        .set(0x0102, Value::Short(vec![8, 8, 8]))
        .short(0x0103, 1)
        .short(0x0106, 2)
        .long(0x0111, strip_offset)
        .short(0x0115, 3)
        .long(0x0116, image.height())
        .long(0x0117, strip_len as u32)
        .short(0x011c, 1);
    ifd
}

/// The DNG tags that describe the camera rather than one image.
fn add_camera_tags(ifd0: &mut Ifd, spec: &DngSpec) {
    let camera_matrix = |values: &[i32; 9]| {
        Value::SRational(values.iter().map(|&v| (v, MATRIX_DENOMINATOR)).collect())
    };
    let neutral = neutral_fractions(&matrix(&COLOR_MATRIX_D65));
    let neutral = Value::Rational(neutral.map(|n| (n, NEUTRAL_DENOMINATOR)).to_vec());
    let baseline = Value::SRational(vec![(BASELINE_EXPOSURE_CENTISTOPS, 100)]);
    let model = match (&spec.exif.camera_make, &spec.exif.camera_model) {
        (Some(make), Some(model)) => format!("{make} {model}"),
        (_, Some(name)) | (Some(name), None) => name.clone(),
        (None, None) => "Photo Testkit Synthetic".to_string(),
    };
    ifd0.ascii(0x0131, "photo-testkit")
        .set(0xc612, Value::Byte(vec![1, 4, 0, 0]))
        .set(0xc613, Value::Byte(vec![1, 1, 0, 0]))
        .ascii(0xc614, &model)
        .set(0xc621, camera_matrix(&COLOR_MATRIX_A))
        .set(0xc622, camera_matrix(&COLOR_MATRIX_D65))
        .set(0xc628, neutral)
        .set(0xc62a, baseline)
        // Calibration illuminants: 17 is Standard Light A, 21 is D65.
        .short(0xc65a, 17)
        .short(0xc65b, 21);
}

#[cfg(test)]
mod tests;
