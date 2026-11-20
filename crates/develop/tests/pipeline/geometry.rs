//! Geometry is decided once: every consumer reads the same rotated,
//! pixel-snapped output rectangle.

use photo_core::develop::{
    Crop, CropRect, DevelopSettings, Mask, MaskShape, Orientation, Transform,
};
use photo_core::geom::{Point, Rect};
use photo_core::image::Rgba8Image;
use photo_develop::{actual_size_view, fit_view, output_size, scaled_view};

use crate::fixtures::{edit, full, max_difference, raster, render, render16};

fn orientations() -> impl Iterator<Item = Orientation> {
    (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
}

fn crops() -> [Crop; 3] {
    [
        Crop::default(),
        Crop {
            rect: Some(CropRect {
                cx: 0.4,
                cy: 0.55,
                width: 0.5,
                height: 0.6,
            }),
            ..Default::default()
        },
        Crop {
            angle: 8.5,
            ..Default::default()
        },
    ]
}

#[test]
fn every_consumer_agrees_with_output_size_for_every_orientation_and_crop() {
    let (_, source) = raster(300, 200);
    for orientation in orientations() {
        for crop in crops() {
            let settings = edit(|s| {
                s.orientation = orientation;
                s.crop = crop;
            });
            let size = output_size(&source, &settings);
            let (frame_w, frame_h) = orientation.apply_to_size(300, 200);
            match (crop.rect, crop.angle) {
                (None, 0.0) => assert_eq!(size, (frame_w, frame_h)),
                (Some(rect), 0.0) => assert_eq!(
                    size,
                    (
                        (rect.width * frame_w as f32).round() as u32,
                        (rect.height * frame_h as f32).round() as u32
                    )
                ),
                _ => assert!(size.0 < frame_w && size.1 < frame_h, "{size:?}"),
            }
            let output = Rect::from_size(size.0 as f32, size.1 as f32);
            let fit = fit_view(&source, &settings, 120, 120);
            assert_eq!(fit.roi(), output, "{orientation:?} {crop:?}");
            assert!(fit.width().max(fit.height()) == 120, "{orientation:?}");
            let one_to_one = actual_size_view(output);
            assert_eq!((one_to_one.width(), one_to_one.height()), size);
            assert_eq!(one_to_one.scale(), 1.0);
            let words = render16(&source, &settings, full(&source, &settings));
            assert_eq!((words.width, words.height), size);
            let bytes = render(&source, &settings, fit);
            assert_eq!((bytes.width, bytes.height), (fit.width(), fit.height()));
        }
    }
}

#[test]
fn a_transform_keeps_the_frame_size() {
    let (_, source) = raster(300, 200);
    let settings = edit(|s| {
        s.transform = Transform {
            vertical: -30.0,
            horizontal: 10.0,
            rotate: 2.0,
            scale: 110.0,
            ..Default::default()
        };
    });
    assert_eq!(output_size(&source, &settings), (300, 200));
    let image = render(&source, &settings, full(&source, &settings));
    assert_eq!((image.width, image.height), (300, 200));
    assert_ne!(
        image,
        render(
            &source,
            &DevelopSettings::default(),
            full(&source, &settings)
        )
    );
}

/// `image` turned a quarter clockwise.
fn turn(image: &Rgba8Image) -> Rgba8Image {
    let (width, height) = (image.height, image.width);
    let mut pixels = Vec::with_capacity(image.pixels.len());
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&image.pixel(y, image.height - 1 - x));
        }
    }
    Rgba8Image {
        width,
        height,
        pixels,
        space: image.space,
    }
}

#[test]
fn a_quarter_turned_render_is_the_upright_render_turned() {
    let (_, source) = raster(360, 240);
    let mut mask = Mask::from_shape(
        MaskShape::Radial {
            center: Point::new(0.3, 0.4),
            radius_x: 0.2,
            radius_y: 0.12,
            angle: 15.0,
            feather: 50.0,
            invert: false,
        },
        1,
    );
    mask.adjust.exposure = -0.8;
    let upright = edit(|s| {
        s.tone.highlights = -50.0;
        s.tone.shadows = 40.0;
        s.tone.contrast = 20.0;
        s.presence.clarity = 35.0;
        s.presence.dehaze = 20.0;
        s.detail.sharpening.amount = 40.0;
        s.detail.noise_reduction.luminance = 20.0;
        s.effects.vignette.amount = -35.0;
        s.masks.push(mask);
    });
    let turned = DevelopSettings {
        orientation: Orientation::new(1, false),
        ..upright.clone()
    };
    let expected = turn(&render(&source, &upright, full(&source, &upright)));
    let got = render(&source, &turned, full(&source, &turned));
    assert!(max_difference(&got, &expected) <= 1);
}

#[test]
fn views_at_a_scale_cover_their_rectangle() {
    let view = scaled_view(Rect::new(10.0, 20.0, 110.0, 70.0), 2.0);
    assert_eq!((view.width(), view.height()), (200, 100));
    assert_eq!(view.scale(), 2.0);
    let snapped = actual_size_view(Rect::new(10.4, 19.6, 60.2, 39.9));
    assert_eq!(snapped.roi(), Rect::new(10.0, 20.0, 60.0, 40.0));
}
