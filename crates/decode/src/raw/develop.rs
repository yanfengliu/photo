//! Sensor data to camera-native linear RGB: level scaling, demosaicing,
//! cropping, turning upright and quantizing.
//!
//! The steps are those of rawler's own development (`imgop::develop`),
//! driven one by one so the mosaic is never copied, nothing is white
//! balanced on the way out, and every size is checked before rawler's
//! unchecked inner loops see it.

use photo_core::image::{CameraColor, SourceColor, SourceImage};
use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};
use rawler::imgop::sensor::bayer::bilinear::Bilinear4Channel;
use rawler::imgop::sensor::bayer::ppg::PPGDemosaic;
use rawler::imgop::sensor::{Demosaic, SensorType};
use rawler::pixarray::PixF32;
use rawler::rawimage::{CFAConfig, RawPhotometricInterpretation};
use rawler::{RawImage, RawImageData};
use rayon::prelude::*;

use super::color::{
    as_shot_gains, camera_matrix, fold_four_colors, monochrome_matrix, nearest_d65_matrix,
};
use super::frame::{Area, Frame};
use super::levels::{SiteGains, normalized};
use super::xtrans;
use crate::error::DecodeError;
use crate::orient::Orientation;

/// Smallest active area, per side, that is handed to one of rawler's
/// demosaicers. Its Bayer interpolation indexes three pixels in from every
/// edge without checking, and loops forever below six columns.
const MIN_MOSAIC_SIDE: usize = 8;
/// Baseline exposures beyond this many stops are taken as garbage.
const MAX_BASELINE_EXPOSURE: f32 = 10.0;

/// What kind of sensor data a raw image holds.
enum Sensor<'a> {
    /// A 2 x 2 red/green/blue mosaic.
    Bayer(&'a CFAConfig),
    /// Fujifilm's 6 x 6 red/green/blue mosaic.
    XTrans(&'a CFAConfig),
    /// A 2 x 2 mosaic of four different filters (CYGM, RGBE).
    FourColor(&'a CFAConfig),
    /// Three samples per pixel: already demosaiced.
    Rgb,
    /// One sample per pixel and no colour filters.
    Monochrome,
}

impl<'a> Sensor<'a> {
    fn of(raw: &'a RawImage) -> Result<Self, DecodeError> {
        match (raw.cpp, &raw.photometric) {
            (1, RawPhotometricInterpretation::Cfa(config)) => {
                let bayer = config.sensor == SensorType::Bayer;
                if bayer && config.cfa.is_rgb() {
                    Ok(Self::Bayer(config))
                } else if config.sensor == SensorType::Xtrans && config.cfa.is_rgb() {
                    Ok(Self::XTrans(config))
                } else if bayer
                    && config.cfa.unique_colors() == 4
                    && config.colors.plane_count() == 4
                {
                    Ok(Self::FourColor(config))
                } else {
                    let pattern = &config.cfa.name;
                    Err(DecodeError::Unsupported(format!(
                        "sensors with a {pattern} colour filter pattern"
                    )))
                }
            }
            (1, _) => Ok(Self::Monochrome),
            (3, _) => Ok(Self::Rgb),
            (samples, _) => Err(DecodeError::Unsupported(format!(
                "raw data with {samples} samples per pixel"
            ))),
        }
    }
}

/// Develops a decoded raw image into a [`SourceImage`].
///
/// The result is camera-native linear RGB: black subtracted, the white
/// level at 1.0, clamped to 0..=1, cropped to the recommended area and
/// turned upright by `orientation`. It is not white balanced. (Mosaics are
/// balanced while they are interpolated, because interpolation assumes the
/// channels move together, and the gains are divided out again afterwards;
/// a measured sample comes back as exactly what the sensor recorded.)
///
/// The colour description carries the matrix calibrated nearest D65, the
/// as-shot gains with a daylight fallback, and `baseline_exposure`.
/// Four-colour sensors are folded into three channels and monochrome ones
/// are copied to all three; see [`fold_four_colors`] and
/// [`monochrome_matrix`].
pub(crate) fn develop(
    mut raw: RawImage,
    orientation: Orientation,
    baseline_exposure: f32,
) -> Result<SourceImage, DecodeError> {
    let frame = Frame::of(&raw)?;
    let data = std::mem::replace(&mut raw.data, RawImageData::Integer(Vec::new()));
    let baseline_exposure = if baseline_exposure.abs() <= MAX_BASELINE_EXPOSURE {
        baseline_exposure
    } else {
        0.0
    };
    let camera = |xyz_to_camera, as_shot_gains| CameraColor {
        xyz_to_camera,
        as_shot_gains,
        baseline_exposure,
    };
    let rgb_color = || {
        let matrix = camera_matrix(&raw.color_matrix);
        let [red, green, blue, _] = raw.wb_coeffs;
        camera(matrix, as_shot_gains([red, green, blue], &matrix))
    };

    let (pixels, (width, height), color) = match Sensor::of(&raw)? {
        Sensor::Bayer(config) | Sensor::XTrans(config) => {
            let color = rgb_color();
            let gains = color.as_shot_gains;
            let mosaic = normalized(&raw, data, &SiteGains::for_mosaic(&config.cfa, gains))?;
            let (rgb, window) = demosaic_rgb(&raw, config, &frame, mosaic)?;
            let unbalance = gains.map(|gain| 1.0 / gain);
            let fetch = |index: usize| {
                let pixel = rgb.pixels[index];
                std::array::from_fn(|channel| pixel[channel] * unbalance[channel])
            };
            (
                assemble(rgb.width, window, orientation, fetch),
                window.size(),
                color,
            )
        }
        Sensor::FourColor(config) => {
            let matrix = nearest_d65_matrix(&raw.color_matrix, 4)
                .and_then(|flat| <[[f32; 3]; 4]>::try_from(flat.as_chunks::<3>().0).ok());
            let fold = matrix
                .and_then(|matrix| fold_four_colors(&matrix, raw.wb_coeffs))
                .ok_or_else(|| {
                    DecodeError::Unsupported(
                        "a four-colour sensor without a usable colour matrix".into(),
                    )
                })?;
            let mosaic = normalized(&raw, data, &SiteGains::NONE)?;
            check_mosaic_size(frame.active)?;
            let mosaic = PixF32::new_with(mosaic, raw.width, raw.height);
            let area = frame.active.rect();
            let planes =
                Bilinear4Channel::new().demosaic(&mosaic, &config.cfa, &config.colors, area);
            let window = Window::inside(&frame, planes.width, planes.height)?;
            let fetch = |index: usize| {
                let sensor: [f32; 4] = planes.data[index];
                let dot = |row: [f32; 4]| {
                    (0..4)
                        .map(|channel| row[channel] * sensor[channel])
                        .sum::<f32>()
                };
                fold.fold.map(dot)
            };
            let color = camera(fold.xyz_to_camera, fold.as_shot_gains);
            (
                assemble(planes.width, window, orientation, fetch),
                window.size(),
                color,
            )
        }
        Sensor::Rgb => {
            let samples = normalized(&raw, data, &SiteGains::NONE)?;
            let window = Window::whole_frame(&frame);
            let (pixels, _) = samples.as_chunks::<3>();
            let fetch = |index: usize| pixels[index];
            (
                assemble(raw.width, window, orientation, fetch),
                window.size(),
                rgb_color(),
            )
        }
        Sensor::Monochrome => {
            let samples = normalized(&raw, data, &SiteGains::NONE)?;
            let window = Window::whole_frame(&frame);
            let fetch = |index: usize| [samples[index]; 3];
            let color = camera(monochrome_matrix(), [1.0; 3]);
            (
                assemble(raw.width, window, orientation, fetch),
                window.size(),
                color,
            )
        }
    };

    let (width, height) = orientation.upright_size(width, height);
    SourceImage::new(
        width as u32,
        height as u32,
        pixels,
        SourceColor::Camera(color),
    )
    .ok_or_else(|| DecodeError::Corrupt("the developed raw image is empty".into()))
}

/// Full-colour pixels for a rectangle of the sensor, in row order.
struct Interpolated {
    width: usize,
    pixels: Vec<[f32; 3]>,
}

/// The part of an interpolated buffer that becomes the picture.
#[derive(Clone, Copy)]
struct Window {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Window {
    fn size(self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The crop as a window into a buffer that covers the whole sensor.
    fn whole_frame(frame: &Frame) -> Self {
        Self {
            x: frame.crop.x,
            y: frame.crop.y,
            width: frame.crop.width,
            height: frame.crop.height,
        }
    }

    /// The crop as a window into a buffer that covers just the active area,
    /// after checking that the buffer really has that size.
    fn inside(
        frame: &Frame,
        buffer_width: usize,
        buffer_height: usize,
    ) -> Result<Self, DecodeError> {
        if (buffer_width, buffer_height) != (frame.active.width, frame.active.height) {
            return Err(DecodeError::Corrupt(
                "the demosaiced image has the wrong size".into(),
            ));
        }
        // `Frame` guarantees the crop lies inside the active area.
        Ok(Self {
            x: frame.crop.x - frame.active.x,
            y: frame.crop.y - frame.active.y,
            width: frame.crop.width,
            height: frame.crop.height,
        })
    }
}

fn check_mosaic_size(active: Area) -> Result<(), DecodeError> {
    if active.width < MIN_MOSAIC_SIDE || active.height < MIN_MOSAIC_SIDE {
        return Err(DecodeError::Unsupported(format!(
            "a {} x {} sensor area is too small to demosaic",
            active.width, active.height
        )));
    }
    Ok(())
}

/// Interpolates a balanced red/green/blue mosaic to full colour, returning
/// the pixels and the window of them to keep.
///
/// Bayer mosaics go through rawler's PPG interpolation; X-Trans mosaics
/// through [`xtrans::interpolate`]. Only the active area is interpolated, so
/// masked border pixels never bleed into the picture.
fn demosaic_rgb(
    raw: &RawImage,
    config: &CFAConfig,
    frame: &Frame,
    mosaic: Vec<f32>,
) -> Result<(Interpolated, Window), DecodeError> {
    if raw.fuji_rotation_width.is_some() {
        return demosaic_fuji_rotated(raw, frame, mosaic);
    }
    let (width, height, pixels) = match config.sensor {
        SensorType::Bayer => {
            check_mosaic_size(frame.active)?;
            let mosaic = PixF32::new_with(mosaic, raw.width, raw.height);
            let area = frame.active.rect();
            let rgb = PPGDemosaic::new().demosaic(&mosaic, &config.cfa, &config.colors, area);
            (rgb.width, rgb.height, rgb.data)
        }
        SensorType::Xtrans => {
            let pixels = xtrans::interpolate(&mosaic, raw.width, frame.active, &config.cfa);
            (frame.active.width, frame.active.height, pixels)
        }
    };
    let window = Window::inside(frame, width, height)?;
    Ok((Interpolated { width, pixels }, window))
}

/// Fuji Super CCD data is stored turned by 45 degrees. Turning it back is
/// part of rawler's development pipeline and not available on its own, so
/// this one case runs that pipeline (demosaic, rotate, crop) on a copy of
/// the image holding the already scaled mosaic.
fn demosaic_fuji_rotated(
    raw: &RawImage,
    frame: &Frame,
    mosaic: Vec<f32>,
) -> Result<(Interpolated, Window), DecodeError> {
    let stored = Area {
        x: 0,
        y: 0,
        width: raw.width,
        height: raw.height,
    };
    check_mosaic_size(stored)?;
    let mut scaled = raw.clone();
    scaled.data = RawImageData::Float(mosaic);
    // The checked crop, so the pipeline's own crop cannot leave the image.
    scaled.active_area = None;
    scaled.crop_area = Some(frame.crop.rect());
    let steps = [
        ProcessingStep::Demosaic,
        ProcessingStep::FujiRotate,
        ProcessingStep::CropDefault,
    ];
    let expected = (frame.crop.width, frame.crop.height);
    match RawDevelop::new_with(&steps).develop_intermediate(&scaled)? {
        Intermediate::ThreeColor(rgb) if (rgb.width, rgb.height) == expected => {
            let window = Window {
                x: 0,
                y: 0,
                width: rgb.width,
                height: rgb.height,
            };
            let rgb = Interpolated {
                width: rgb.width,
                pixels: rgb.data,
            };
            Ok((rgb, window))
        }
        _ => Err(DecodeError::Corrupt(
            "the rotated sensor data did not develop to the expected size".into(),
        )),
    }
}

/// Copies `window` out of a buffer `buffer_width` pixels wide, turned
/// upright, as interleaved 16-bit RGB. `fetch` supplies the linear colour of
/// the buffer pixel with a given index; values are clamped to 0..=1 (a NaN
/// becomes 0) and rounded to the nearest code.
fn assemble(
    buffer_width: usize,
    window: Window,
    orientation: Orientation,
    fetch: impl Fn(usize) -> [f32; 3] + Sync,
) -> Vec<u16> {
    let (out_width, out_height) = orientation.upright_size(window.width, window.height);
    let mut out = vec![0u16; out_width * out_height * 3];
    if out.is_empty() {
        return out;
    }
    out.par_chunks_mut(out_width * 3)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
                let (stored_x, stored_y) =
                    orientation.stored_position(x, y, window.width, window.height);
                let colour = fetch((window.y + stored_y) * buffer_width + window.x + stored_x);
                for (code, value) in pixel.iter_mut().zip(colour) {
                    // A NaN survives the clamp and the cast then makes it 0.
                    *code = (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
                }
            }
        });
    out
}

#[cfg(test)]
mod tests;
