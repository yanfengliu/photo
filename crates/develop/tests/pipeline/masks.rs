//! Masks as per-pixel effective parameters.

use photo_core::develop::{Crop, CropRect, DevelopSettings, Mask, MaskShape, Orientation};
use photo_core::geom::Point;
use photo_develop::{geometry, mask_alpha};

use crate::fixtures::{edit, full, luma, raster, render, rgb};

/// A hard-edged ellipse in normalized source coordinates.
fn disc(cx: f32, cy: f32, radius: f32) -> MaskShape {
    MaskShape::Radial {
        center: Point::new(cx, cy),
        radius_x: radius,
        radius_y: radius,
        angle: 0.0,
        feather: 0.0,
        invert: false,
    }
}

fn exposure_mask(shape: MaskShape, stops: f32) -> Mask {
    let mut mask = Mask::from_shape(shape, 1);
    mask.adjust.exposure = stops;
    mask
}

#[test]
fn two_overlapping_one_stop_masks_give_two_stops_in_the_overlap() {
    let (_, source) = raster(400, 300);
    let masked = edit(|s| {
        s.masks = vec![
            exposure_mask(disc(0.45, 0.5, 0.2), 1.0),
            exposure_mask(disc(0.55, 0.5, 0.2), 1.0),
        ];
    });
    let global = edit(|s| s.tone.exposure = 2.0);
    let one = edit(|s| s.tone.exposure = 1.0);
    let view = full(&source, &masked);
    let (two_masks, two_stops, one_stop) = (
        render(&source, &masked, view),
        render(&source, &global, view),
        render(&source, &one, view),
    );
    // The overlap around the centre, and the left disc alone.
    for (x, y) in [(200, 150), (190, 140), (210, 165)] {
        let (a, b) = (rgb(&two_masks, x, y), rgb(&two_stops, x, y));
        for c in 0..3 {
            assert!(a[c].abs_diff(b[c]) <= 1, "({x}, {y}): {a:?} vs {b:?}");
        }
    }
    let (a, b) = (rgb(&two_masks, 120, 150), rgb(&one_stop, 120, 150));
    for c in 0..3 {
        assert!(a[c].abs_diff(b[c]) <= 1, "{a:?} vs {b:?}");
    }
}

#[test]
fn a_mask_that_adjusts_nothing_leaves_the_render_bit_identical() {
    let (_, source) = raster(320, 220);
    let base = edit(|s| {
        s.tone.highlights = -40.0;
        s.presence.clarity = 30.0;
        s.detail.sharpening.amount = 40.0;
    });
    let reference = render(&source, &base, full(&source, &base));
    let neutral = Mask::from_shape(disc(0.5, 0.5, 0.3), 1);
    let mut disabled = exposure_mask(disc(0.5, 0.5, 0.3), 1.0);
    disabled.enabled = false;
    let mut weightless = exposure_mask(disc(0.5, 0.5, 0.3), 1.0);
    weightless.amount = 0.0;
    for mask in [neutral, disabled, weightless] {
        let settings = DevelopSettings {
            masks: vec![mask.clone()],
            ..base.clone()
        };
        assert_eq!(
            render(&source, &settings, full(&source, &settings)),
            reference,
            "{mask:?}"
        );
    }
}

#[test]
fn a_mask_stays_on_its_content_through_orientation_and_crop() {
    let (_, source) = raster(480, 320);
    // The sun of the testkit scene, darkened by three stops.
    let sun = Point::new(0.76, 0.2);
    let shape = disc(sun.x, sun.y, 0.06);
    let geometries = [
        (Orientation::IDENTITY, Crop::default()),
        (Orientation::new(1, false), Crop::default()),
        (Orientation::new(2, true), Crop::default()),
        (
            Orientation::IDENTITY,
            Crop {
                rect: Some(CropRect {
                    cx: 0.62,
                    cy: 0.4,
                    width: 0.6,
                    height: 0.7,
                }),
                ..Default::default()
            },
        ),
        (
            Orientation::new(3, false),
            Crop {
                angle: 6.0,
                ..Default::default()
            },
        ),
    ];
    for (orientation, crop) in geometries {
        let plain = edit(|s| {
            s.orientation = orientation;
            s.crop = crop;
        });
        let masked = DevelopSettings {
            masks: vec![exposure_mask(shape.clone(), -3.0)],
            ..plain.clone()
        };
        let view = full(&source, &plain);
        let (before, after) = (
            render(&source, &plain, view),
            render(&source, &masked, view),
        );
        // Where the mask darkened the picture, weighted by how much.
        let (mut sum, mut total) = ((0.0f64, 0.0f64), 0.0f64);
        for y in 0..before.height {
            for x in 0..before.width {
                let drop = f64::from(luma(&before, x, y, 1, 1) - luma(&after, x, y, 1, 1));
                if drop > 0.0 {
                    sum.0 += drop * (f64::from(x) + 0.5);
                    sum.1 += drop * (f64::from(y) + 0.5);
                    total += drop;
                }
            }
        }
        assert!(total > 0.0, "{orientation:?} {crop:?}: nothing darkened");
        let centroid = Point::new((sum.0 / total) as f32, (sum.1 / total) as f32);
        let (width, height) = source.size();
        let expected = geometry(&source, &plain)
            .src_to_out(Point::new(sun.x * width as f32, sun.y * height as f32))
            .unwrap();
        assert!(
            centroid.distance(expected) < 3.0,
            "{orientation:?} {crop:?}: darkened around {centroid:?}, the sun is at {expected:?}"
        );
        // The overlay shows the same place.
        let alpha = mask_alpha(&source, &masked, 0, view).unwrap();
        let at = |p: Point| alpha.get(p.x as usize, p.y as usize);
        assert_eq!(at(expected), 1.0);
    }
}
