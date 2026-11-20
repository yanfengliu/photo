//! The export size: the full-resolution output scaled by the sizing
//! settings, keeping its aspect ratio.

use crate::item::ExportError;
use crate::settings::{Format, Resize, Sizing};

/// The largest picture this crate writes, in pixels: the largest the
/// decoder reads back.
pub(crate) const MAX_PIXELS: u64 = photo_decode::MAX_PIXELS;

/// The export size for a full-resolution output of `output` pixels, or why
/// the settings cannot be honoured.
///
/// The scale is one number for both axes, so the aspect ratio is kept up to
/// rounding; sides are rounded to whole pixels and at least one. A side
/// that the mode fixes (the long edge, the short edge, a box that binds)
/// comes out exactly as asked.
pub(crate) fn target_size(output: (u32, u32), sizing: &Sizing) -> Result<(u32, u32), ExportError> {
    let (width, height) = (f64::from(output.0.max(1)), f64::from(output.1.max(1)));
    let positive = |value: f64, what: &str| {
        if value.is_finite() && value > 0.0 {
            Ok(value)
        } else {
            Err(ExportError::Unsupported(format!(
                "{what} must be a positive number"
            )))
        }
    };
    let scale = match sizing.resize {
        Resize::Original => 1.0,
        Resize::LongEdge { pixels } => {
            positive(f64::from(pixels), "the long edge")? / width.max(height)
        }
        Resize::ShortEdge { pixels } => {
            positive(f64::from(pixels), "the short edge")? / width.min(height)
        }
        Resize::Fit {
            width: box_width,
            height: box_height,
        } => {
            let across = positive(f64::from(box_width), "the width to fit")? / width;
            let down = positive(f64::from(box_height), "the height to fit")? / height;
            across.min(down)
        }
        Resize::Megapixels { megapixels } => {
            let pixels = positive(f64::from(megapixels), "the megapixel count")? * 1e6;
            (pixels / (width * height)).sqrt()
        }
        Resize::Percent { percent } => positive(f64::from(percent), "the percentage")? / 100.0,
    };
    let scale = if sizing.dont_enlarge {
        scale.min(1.0)
    } else {
        scale
    };
    let side = |extent: f64| (extent * scale).round().clamp(1.0, f64::from(u32::MAX)) as u32;
    let size = (side(width), side(height));
    if u64::from(size.0) * u64::from(size.1) > MAX_PIXELS {
        return Err(ExportError::Unsupported(format!(
            "{} x {} pixels is more than the {} megapixels an export may have",
            size.0,
            size.1,
            MAX_PIXELS / 1_000_000
        )));
    }
    Ok(size)
}

/// Refuses sizes `format` cannot store.
pub(crate) fn check_format(format: Format, (width, height): (u32, u32)) -> Result<(), ExportError> {
    let limit = match format {
        Format::Jpeg { .. } => u32::from(u16::MAX),
        // PNG allows 2^31 - 1 per side; the pixel limit binds first.
        Format::Png { .. } => i32::MAX as u32,
        // A classic TIFF addresses at most 4 GiB; the pixel limit at 16 bits
        // stays below that.
        Format::Tiff { .. } => u32::MAX,
    };
    if width > limit || height > limit {
        return Err(ExportError::Unsupported(format!(
            "a {} file cannot be {width} x {height} pixels; the limit is {limit} per side",
            format.extension().to_ascii_uppercase()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(output: (u32, u32), resize: Resize, dont_enlarge: bool) -> (u32, u32) {
        target_size(
            output,
            &Sizing {
                resize,
                dont_enlarge,
            },
        )
        .unwrap()
    }

    #[test]
    fn the_pixel_limit_is_below_what_a_sixteen_bit_tiff_can_address() {
        assert!(MAX_PIXELS * 6 < u64::from(u32::MAX));
    }

    #[test]
    fn rounding_never_leaves_the_box_it_fits() {
        for (width, height) in [(1001, 667), (667, 1001), (4000, 3), (3, 4000), (999, 999)] {
            for (box_width, box_height) in [(100, 100), (333, 77), (1, 1), (640, 480)] {
                let (w, h) = size(
                    (width, height),
                    Resize::Fit {
                        width: box_width,
                        height: box_height,
                    },
                    false,
                );
                assert!(
                    w <= box_width && h <= box_height,
                    "{width}x{height} in {box_width}x{box_height}: {w}x{h}"
                );
                assert!(w == box_width || h == box_height || w == 1 || h == 1);
            }
        }
    }
}
