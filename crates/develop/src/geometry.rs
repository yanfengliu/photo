//! The output frame a source and its settings produce, and the views onto
//! it. Every consumer (preview fit, actual-size zoom, status text, export)
//! reads the size from [`output_size`], so they cannot disagree.

use photo_core::develop::DevelopSettings;
use photo_core::geom::Rect;
use photo_imaging::{Geometry, View};

use crate::source::Source;

/// The mapping between the source and the output for the lens,
/// orientation, transform and crop settings: what tool overlays use to
/// place handles and what the renderer samples through.
pub fn geometry(source: &Source, settings: &DevelopSettings) -> Geometry {
    let (width, height) = source.size();
    Geometry::from_settings(width, height, settings)
}

/// The full-resolution size of the rendered photo in pixels: the crop
/// rectangle (rotated, pixel-snapped) of the oriented and transformed
/// frame. This is the single source of truth for the output's dimensions;
/// a full render, [`fit_view`] and [`actual_size_view`] of the whole
/// output all agree with it.
pub fn output_size(source: &Source, settings: &DevelopSettings) -> (u32, u32) {
    geometry(source, settings).output_size()
}

/// A view of the whole output fitted inside a `box_width` x `box_height`
/// box, keeping the aspect ratio. Never enlarges (a photo smaller than the
/// box is shown 1:1); both sides are at least one pixel.
pub fn fit_view(
    source: &Source,
    settings: &DevelopSettings,
    box_width: u32,
    box_height: u32,
) -> View {
    let (width, height) = output_size(source, settings);
    let ratio = (f64::from(box_width.max(1)) / f64::from(width))
        .min(f64::from(box_height.max(1)) / f64::from(height))
        .min(1.0);
    let side = |pixels: u32| ((f64::from(pixels) * ratio).round() as u32).max(1);
    View::new(
        Rect::from_size(width as f32, height as f32),
        side(width),
        side(height),
    )
}

/// A 1:1 view of an output-space rectangle: one view pixel per output
/// pixel. The rectangle is snapped to whole output pixels (its corner
/// rounded, its sides rounded and at least one pixel), so each view pixel
/// sits exactly on an output pixel.
pub fn actual_size_view(rect: Rect) -> View {
    let snap = |value: f32| {
        if value.is_finite() {
            value.round()
        } else {
            0.0
        }
    };
    let (x0, y0) = (snap(rect.x0), snap(rect.y0));
    let width = snap(rect.width()).max(1.0);
    let height = snap(rect.height()).max(1.0);
    View::new(
        Rect::new(x0, y0, x0 + width, y0 + height),
        width as u32,
        height as u32,
    )
}

/// A view of the output-space rectangle `roi` at `scale` view pixels per
/// output pixel (2 is a 200% zoom, 0.25 a quarter-size preview). Sides are
/// rounded and at least one pixel; a scale that is not a positive number
/// means 1.
pub fn scaled_view(roi: Rect, scale: f32) -> View {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let side = |extent: f32| {
        let pixels = f64::from(extent) * f64::from(scale);
        if pixels.is_finite() {
            pixels.round().clamp(1.0, f64::from(u32::MAX)) as u32
        } else {
            1
        }
    };
    View::new(roi, side(roi.width()), side(roi.height()))
}
