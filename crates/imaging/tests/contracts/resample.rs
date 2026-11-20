//! The source pyramid and the resampler: exactness where the geometry is
//! pixel-preserving, and the R1 / R2 contracts of DESIGN §5.9.

use std::sync::Arc;

use photo_core::develop::{Crop, CropRect, LensCorrections, Orientation, Transform};
use photo_core::geom::{Point, Rect};
use photo_core::image::SourceImage;
use photo_imaging::{Geometry, RgbImage, SourcePyramid, View, resample, resample_level};

use crate::util::{
    flat, max_abs_diff, mean_abs_diff, noise_source, smooth_source, source_from_fn, std_dev, unit,
};

fn source_pixel(image: &SourceImage, x: u32, y: u32) -> [f32; 3] {
    image.pixel(x, y).map(unit)
}

fn orientation_only(width: u32, height: u32, orientation: Orientation) -> Geometry {
    Geometry::new(
        width,
        height,
        &LensCorrections::default(),
        orientation,
        &Transform::default(),
        &Crop::default(),
    )
}

#[test]
fn the_pyramid_halves_down_to_about_sixty_four_pixels_and_shares_level_zero() {
    let source = Arc::new(noise_source(301, 200, 1));
    let pyramid = SourcePyramid::new(source.clone());
    assert!(
        Arc::ptr_eq(pyramid.level(0), &source),
        "level 0 must not be copied"
    );
    let sizes: Vec<(u32, u32)> = (0..pyramid.level_count())
        .map(|level| (pyramid.level(level).width, pyramid.level(level).height))
        .collect();
    assert_eq!(sizes, [(301, 200), (151, 100), (76, 50), (38, 25)]);
    assert_eq!(pyramid.source_size(), (301, 200));
}

#[test]
fn each_level_is_a_two_by_two_box_average_in_linear_light() {
    let source = noise_source(128, 96, 2);
    let pyramid = SourcePyramid::new(source.clone());
    let level = pyramid.level(1);
    for (x, y) in [(0, 0), (5, 7), (63, 47)] {
        for channel in 0..3 {
            let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|(dx, dy)| source.pixel(2 * x + dx, 2 * y + dy)[channel] as u32)
                .sum();
            let got = level.pixel(x, y)[channel] as i64;
            assert!(
                (got - (sum as f64 / 4.0).round() as i64).abs() <= 1,
                "({x}, {y})"
            );
        }
    }
}

#[test]
fn an_odd_edge_keeps_its_last_column_instead_of_dropping_it() {
    // 131 columns: the last one has no partner, and must still be represented.
    let wide = source_from_fn(131, 2, |x, _| [if x == 130 { 1.0 } else { 0.0 }; 3]);
    let pyramid = SourcePyramid::new(wide);
    assert_eq!(pyramid.level(1).width, 66);
    assert_eq!(pyramid.level(1).pixel(65, 0), [65535; 3]);
    assert_eq!(pyramid.level(1).pixel(64, 0), [0; 3]);
    // An image already at the limit gets no further level.
    assert_eq!(
        SourcePyramid::new(source_from_fn(64, 5, |_, _| [0.5; 3])).level_count(),
        1
    );
}

#[test]
fn sampling_a_level_is_bilinear_and_clamps_to_the_edge() {
    let source = source_from_fn(4, 4, |x, y| [x as f32 / 4.0, y as f32 / 4.0, 0.5]);
    let pyramid = SourcePyramid::new(source.clone());
    // Pixel centres sit at +0.5 and return the pixel itself.
    assert_eq!(pyramid.sample(0, 2.5, 1.5), source_pixel(&source, 2, 1));
    // Halfway between two centres is their average.
    let between = pyramid.sample(0, 2.0, 1.5);
    let expected = (source_pixel(&source, 1, 1)[0] + source_pixel(&source, 2, 1)[0]) / 2.0;
    assert!((between[0] - expected).abs() < 1e-6);
    // Anything outside the image is the nearest edge pixel.
    assert_eq!(pyramid.sample(0, -50.0, 1.5), source_pixel(&source, 0, 1));
    assert_eq!(pyramid.sample(0, 3.5, 900.0), source_pixel(&source, 3, 3));
    let all = pyramid.sample(0, 1.7, 2.2);
    assert!(all.iter().all(|v| (0.0..=1.0).contains(v)));
}

#[test]
fn a_level_can_be_replaced_without_touching_the_others() {
    let pyramid = SourcePyramid::new(noise_source(200, 100, 3));
    let mut healed = (**pyramid.level(1)).clone();
    healed.pixels.fill(7);
    let swapped = pyramid.with_level(1, Arc::new(healed)).expect("same size");
    assert_eq!(swapped.level(1).pixel(3, 3), [7; 3]);
    assert!(Arc::ptr_eq(swapped.level(0), pyramid.level(0)));
    assert_ne!(
        pyramid.level(1).pixel(3, 3),
        [7; 3],
        "the original is untouched"
    );
    // A level of the wrong size is refused.
    assert!(
        pyramid
            .with_level(1, Arc::new(noise_source(9, 9, 4)))
            .is_none()
    );
    assert!(pyramid.with_level(99, pyramid.level(0).clone()).is_none());
}

#[test]
fn identity_geometry_at_full_scale_reproduces_the_source_exactly() {
    let source = noise_source(97, 61, 5);
    let pyramid = SourcePyramid::new(source.clone());
    let geometry = Geometry::identity(97, 61);
    let view = View::full(&geometry);
    assert_eq!((view.width(), view.height()), (97, 61));
    assert_eq!(view.scale(), 1.0);
    let image = resample(&pyramid, &geometry, &view, 0);
    assert_eq!((image.width(), image.height()), (97, 61));
    for y in 0..61 {
        for x in 0..97 {
            assert_eq!(
                image.pixel(x as usize, y as usize),
                source_pixel(&source, x, y)
            );
        }
    }
}

#[test]
fn every_orientation_is_a_pixel_exact_rearrangement() {
    let (w, h) = (23u32, 14u32);
    let source = noise_source(w, h, 6);
    let pyramid = SourcePyramid::new(source.clone());
    for turns in 0..4 {
        for flip in [false, true] {
            let orientation = Orientation::new(turns, flip);
            let geometry = orientation_only(w, h, orientation);
            let (ow, oh) = geometry.output_size();
            let image = resample(&pyramid, &geometry, &View::full(&geometry), 0);
            for y in 0..oh {
                for x in 0..ow {
                    let centre =
                        Point::new((x as f32 + 0.5) / ow as f32, (y as f32 + 0.5) / oh as f32);
                    let from = orientation.inverse(centre);
                    let (sx, sy) = (
                        (from.x * w as f32).floor() as u32,
                        (from.y * h as f32).floor() as u32,
                    );
                    assert_eq!(
                        image.pixel(x as usize, y as usize),
                        source_pixel(&source, sx, sy),
                        "{orientation:?} at ({x}, {y})"
                    );
                }
            }
        }
    }
}

#[test]
fn a_clockwise_quarter_turn_puts_the_top_left_pixel_at_the_top_right() {
    let source = noise_source(8, 5, 7);
    let pyramid = SourcePyramid::new(source.clone());
    let geometry = orientation_only(8, 5, Orientation::IDENTITY.rotated_cw());
    let image = resample(&pyramid, &geometry, &View::full(&geometry), 0);
    assert_eq!((image.width(), image.height()), (5, 8));
    assert_eq!(image.pixel(4, 0), source_pixel(&source, 0, 0));
    assert_eq!(image.pixel(4, 7), source_pixel(&source, 7, 0));
    assert_eq!(image.pixel(0, 0), source_pixel(&source, 0, 4));
}

#[test]
fn an_upright_crop_copies_pixels_without_interpolating() {
    let source = noise_source(100, 80, 8);
    let pyramid = SourcePyramid::new(source.clone());
    let crop = Crop {
        rect: Some(CropRect {
            cx: 0.5,
            cy: 0.5,
            width: 0.4,
            height: 0.5,
        }),
        ..Default::default()
    };
    let geometry = Geometry::new(
        100,
        80,
        &LensCorrections::default(),
        Orientation::IDENTITY,
        &Transform::default(),
        &crop,
    );
    assert_eq!(geometry.output_size(), (40, 40));
    let image = resample(&pyramid, &geometry, &View::full(&geometry), 0);
    for (x, y) in [(0, 0), (17, 3), (39, 39)] {
        assert_eq!(
            image.pixel(x, y),
            source_pixel(&source, x as u32 + 30, y as u32 + 20)
        );
    }
}

#[test]
fn padding_surrounds_the_view_and_clamps_outside_the_source() {
    let source = noise_source(40, 30, 9);
    let pyramid = SourcePyramid::new(source.clone());
    let geometry = Geometry::identity(40, 30);
    let view = View::full(&geometry);
    let plain = resample(&pyramid, &geometry, &view, 0);
    let padded = resample(&pyramid, &geometry, &view, 3);
    assert_eq!((padded.width(), padded.height()), (46, 36));
    assert_eq!(padded.crop(3, 3, 40, 30), plain);
    assert_eq!(padded.pixel(0, 0), source_pixel(&source, 0, 0));
    assert_eq!(padded.pixel(45, 35), source_pixel(&source, 39, 29));
    assert_eq!(padded.pixel(10, 1), source_pixel(&source, 7, 0));
}

#[test]
fn the_level_follows_the_view_scale() {
    let pyramid = SourcePyramid::new(noise_source(1024, 512, 10));
    let geometry = Geometry::identity(1024, 512);
    let roi = Rect::from_size(1024.0, 512.0);
    let level = |width: u32| resample_level(&pyramid, &geometry, &View::new(roi, width, width / 2));
    assert_eq!(
        level(2048),
        0,
        "magnifying samples the full-resolution level"
    );
    assert_eq!(level(1024), 0);
    assert_eq!(
        level(600),
        0,
        "the level's scale stays at or above the view's"
    );
    assert_eq!(level(512), 1);
    assert_eq!(level(300), 1);
    assert_eq!(level(256), 2);
    assert_eq!(level(4), pyramid.level_count() - 1);
}

fn varied_geometry(width: u32, height: u32) -> Vec<(&'static str, Geometry)> {
    let turned = Geometry::new(
        width,
        height,
        &LensCorrections::default(),
        Orientation::new(1, true),
        &Transform::default(),
        &Crop {
            angle: 9.0,
            ..Default::default()
        },
    );
    let warped = Geometry::new(
        width,
        height,
        &LensCorrections {
            distortion: 45.0,
            ..Default::default()
        },
        Orientation::IDENTITY,
        &Transform {
            vertical: -35.0,
            horizontal: 20.0,
            rotate: 3.0,
            scale: 115.0,
            ..Default::default()
        },
        &Crop::default(),
    );
    vec![
        ("identity", Geometry::identity(width, height)),
        ("turned and straightened", turned),
        ("warped", warped),
    ]
}

#[test]
fn r1_a_padded_tile_matches_the_same_region_of_the_whole_render() {
    let source = noise_source(240, 180, 11);
    let pyramid = SourcePyramid::new(source);
    for (name, geometry) in varied_geometry(240, 180) {
        let (ow, oh) = geometry.output_size();
        for scale in [1.0f32, 0.5, 0.25] {
            let (vw, vh) = ((ow as f32 * scale) as u32, (oh as f32 * scale) as u32);
            let roi = Rect::from_size(vw as f32 / scale, vh as f32 / scale);
            let view = View::new(roi, vw, vh);
            let whole = resample(&pyramid, &geometry, &view, 0);
            let tiles = [
                (0, 0, vw / 2, vh / 2),
                (vw / 3, vh / 4, vw / 2, vh / 3),
                (vw / 2, vh / 2, vw - vw / 2, vh - vh / 2),
            ];
            for (x, y, w, h) in tiles {
                let tile = view.tile(x, y, w, h);
                let padded = resample(&pyramid, &geometry, &tile, 5);
                let inner = padded.crop(5, 5, w as usize, h as usize);
                let expected = whole.crop(x as usize, y as usize, w as usize, h as usize);
                // Not merely close: a tile is the same bits.
                assert!(
                    bits(&inner) == bits(&expected),
                    "{name} at {scale}: tile ({x}, {y}) differs by {}",
                    max_abs_diff(flat(&inner), flat(&expected))
                );
            }
        }
    }
}

#[test]
fn r1_holds_at_a_scale_that_is_not_a_power_of_two() {
    let source = noise_source(400, 300, 12);
    let pyramid = SourcePyramid::new(source);
    for (name, geometry) in varied_geometry(400, 300) {
        for max_edge in [173, 287, 399] {
            let view = View::fit(&geometry, max_edge);
            let whole = resample(&pyramid, &geometry, &view, 0);
            let (x, y, w, h) = (31, 17, 60, 50);
            let padded = resample(&pyramid, &geometry, &view.tile(x, y, w, h), 4);
            let inner = padded.crop(4, 4, w as usize, h as usize);
            let expected = whole.crop(x as usize, y as usize, w as usize, h as usize);
            assert!(bits(&inner) == bits(&expected), "{name} at {max_edge}");
        }
    }
}

#[test]
fn r1_survives_a_scale_on_the_boundary_between_two_levels() {
    // A pitch within a few parts in ten million of the point where the
    // resampler moves from level 0 to level 1. A tile described only by a
    // rounded rectangle would compute a slightly different pitch and land
    // on the other side; a tile of the view's own grid cannot.
    let pyramid = SourcePyramid::new(noise_source(2000, 2000, 16));
    let geometry = Geometry::identity(2000, 2000);
    let view = View::new(Rect::new(0.0, 0.0, 1998.6143, 1998.6143), 1000, 1000);
    let whole = resample(&pyramid, &geometry, &view, 0);
    let level = resample_level(&pyramid, &geometry, &view);
    for (x, y, w, h) in [(333u32, 333u32, 100u32, 100u32), (901, 17, 64, 31), (0, 990, 7, 10)] {
        let tile = view.tile(x, y, w, h);
        assert_eq!(resample_level(&pyramid, &geometry, &tile), level);
        let inner = resample(&pyramid, &geometry, &tile, 3).crop(3, 3, w as usize, h as usize);
        let expected = whole.crop(x as usize, y as usize, w as usize, h as usize);
        assert!(bits(&inner) == bits(&expected), "tile at ({x}, {y})");
    }
}

/// Bit patterns of an image: equal only when not even a zero's sign differs.
fn bits(image: &RgbImage) -> Vec<u32> {
    image.as_slice().iter().map(|v| v.to_bits()).collect()
}

fn box_downsample(image: &RgbImage) -> RgbImage {
    RgbImage::from_fn(image.width() / 2, image.height() / 2, |x, y| {
        let mut sum = [0.0f32; 3];
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let p = image.pixel(2 * x + dx, 2 * y + dy);
            for c in 0..3 {
                sum[c] += p[c] / 4.0;
            }
        }
        sum
    })
}

#[test]
fn r2_half_scale_is_the_full_render_downsampled_by_two() {
    // Exactly, up to 16-bit rounding of the pyramid level, when the geometry
    // preserves the pixel grid.
    let source = noise_source(128, 96, 13);
    let pyramid = SourcePyramid::new(source);
    let geometry = Geometry::identity(128, 96);
    let roi = Rect::from_size(128.0, 96.0);
    let full = resample(&pyramid, &geometry, &View::new(roi, 128, 96), 0);
    let half = resample(&pyramid, &geometry, &View::new(roi, 64, 48), 0);
    let diff = max_abs_diff(flat(&half), flat(&box_downsample(&full)));
    assert!(diff <= 2e-5, "{diff}");
}

#[test]
fn r2_half_scale_approximates_the_downsampled_render_under_a_warp() {
    let source = smooth_source(400, 300, 14);
    let pyramid = SourcePyramid::new(source);
    for (name, geometry) in varied_geometry(400, 300) {
        let (ow, oh) = geometry.output_size();
        let (ow, oh) = (ow / 2 * 2, oh / 2 * 2);
        let roi = Rect::from_size(ow as f32, oh as f32);
        let full = resample(&pyramid, &geometry, &View::new(roi, ow, oh), 0);
        let half = resample(&pyramid, &geometry, &View::new(roi, ow / 2, oh / 2), 0);
        let reference = box_downsample(&full);
        let mean = mean_abs_diff(flat(&half), flat(&reference));
        let max = max_abs_diff(flat(&half), flat(&reference));
        assert!(mean < 1e-3 && max < 1e-2, "{name}: mean {mean}, max {max}");
    }
}

#[test]
fn interpolation_never_overshoots_its_neighbours() {
    // A hard edge resampled off-grid (magnified and rotated) must not ring:
    // linear-light halos next to a bright edge are far more visible than the
    // sharpness a negative-lobed kernel would buy.
    let source = source_from_fn(64, 64, |x, _| [if x < 32 { 0.02 } else { 0.9 }; 3]);
    let pyramid = SourcePyramid::new(source);
    let geometry = Geometry::new(
        64,
        64,
        &LensCorrections::default(),
        Orientation::IDENTITY,
        &Transform::default(),
        &Crop {
            angle: 17.0,
            ..Default::default()
        },
    );
    let (ow, oh) = geometry.output_size();
    let view = View::new(Rect::from_size(ow as f32, oh as f32), ow * 3, oh * 3);
    let image = resample(&pyramid, &geometry, &view, 0);
    let (lo, hi) = (
        unit((0.02f32 * 65535.0).round() as u16),
        unit((0.9f32 * 65535.0).round() as u16),
    );
    assert!(image.as_slice().iter().all(|&v| v >= lo && v <= hi));
    // And it really is an edge, not a blur across the whole picture.
    let values = image.as_slice();
    let extremes = values.iter().filter(|&&v| v == lo || v == hi).count();
    assert!(extremes as f32 > 0.8 * values.len() as f32);
}

#[test]
fn a_touch_of_distortion_does_not_change_the_sharpness_of_the_picture() {
    // At 1:1 a distortion of +1 covers 1.0025 source pixels per view pixel
    // at the centre and -1 covers 0.9975. Neither keeps the pixel grid, so
    // both interpolate everywhere, and they must do it with (nearly) the
    // same kernel: interpolating noise lowers its contrast, a soft kernel
    // by much more than a sharp one, so the two renders tell.
    let pyramid = SourcePyramid::new(noise_source(600, 400, 17));
    let contrast = |distortion: f32| {
        let lens = LensCorrections {
            distortion,
            ..Default::default()
        };
        let geometry = Geometry::new(
            600,
            400,
            &lens,
            Orientation::IDENTITY,
            &Transform::default(),
            &Crop::default(),
        );
        std_dev(flat(&resample(&pyramid, &geometry, &View::full(&geometry), 0)))
    };
    let (barrel, pincushion) = (contrast(1.0), contrast(-1.0));
    assert!((barrel / pincushion - 1.0).abs() < 0.02, "{barrel} vs {pincushion}");
    // And the kernel keeps softening gradually as the picture shrinks.
    let shrunk = |scale: f32| {
        let geometry = Geometry::new(
            600,
            400,
            &LensCorrections::default(),
            Orientation::IDENTITY,
            &Transform {
                scale,
                ..Default::default()
            },
            &Crop::default(),
        );
        std_dev(flat(&resample(&pyramid, &geometry, &View::full(&geometry), 0)))
    };
    let steps: Vec<f32> = [99.0, 95.0, 90.0, 85.0, 80.0, 75.0, 70.0].map(shrunk).to_vec();
    for pair in steps.windows(2) {
        // Never a cliff between two neighbouring scales...
        assert!(pair[1] < 1.01 * pair[0] && pair[1] > 0.9 * pair[0], "{steps:?}");
    }
    // ...though over the whole range the averaging clearly sets in.
    assert!(steps[6] < 0.85 * steps[0], "{steps:?}");
}

#[test]
fn shrinking_between_two_levels_prefilters_instead_of_point_sampling() {
    // One-pixel stripes at 70% scale: point sampling keeps them at full
    // contrast as moire; an area-aware kernel averages most of it away.
    let source = source_from_fn(200, 200, |x, _| [if x % 2 == 0 { 0.2 } else { 0.8 }; 3]);
    let pyramid = SourcePyramid::new(source);
    let geometry = Geometry::identity(200, 200);
    let view = View::new(Rect::from_size(200.0, 200.0), 140, 140);
    assert_eq!(resample_level(&pyramid, &geometry, &view), 0);
    let image = resample(&pyramid, &geometry, &view, 0);
    let row: Vec<f32> = (0..140).map(|x| image.pixel(x, 70)[0]).collect();
    let mean = row.iter().sum::<f32>() / row.len() as f32;
    assert!((mean - 0.5).abs() < 0.02, "{mean}");
    let swing = row.iter().fold(0.0f32, |m, v| m.max((v - mean).abs()));
    assert!(
        swing < 0.2,
        "stripes survive at amplitude {swing} (source 0.3)"
    );
}

#[test]
fn a_constant_source_stays_constant_through_any_view() {
    let source = source_from_fn(150, 90, |_, _| [0.25, 0.5, 0.75]);
    let expected = source.pixel(0, 0).map(unit);
    let pyramid = SourcePyramid::new(source);
    for (name, geometry) in varied_geometry(150, 90) {
        for max_edge in [400, 150, 101, 40, 9] {
            let (ow, oh) = geometry.output_size();
            let scale = max_edge as f32 / ow.max(oh) as f32;
            let view = View::new(
                Rect::from_size(ow as f32, oh as f32),
                (ow as f32 * scale).round().max(1.0) as u32,
                (oh as f32 * scale).round().max(1.0) as u32,
            );
            let image = resample(&pyramid, &geometry, &view, 2);
            for pixel in image.pixels() {
                for c in 0..3 {
                    assert!(
                        (pixel[c] - expected[c]).abs() < 2e-5,
                        "{name} at {max_edge}"
                    );
                }
            }
        }
    }
}

#[test]
fn an_empty_view_gives_an_empty_image() {
    let pyramid = SourcePyramid::new(noise_source(20, 20, 15));
    let geometry = Geometry::identity(20, 20);
    let image = resample(
        &pyramid,
        &geometry,
        &View::new(Rect::from_size(20.0, 20.0), 0, 0),
        0,
    );
    assert_eq!((image.width(), image.height()), (0, 0));
    let padded = resample(&pyramid, &geometry, &View::new(Rect::default(), 0, 0), 2);
    assert_eq!((padded.width(), padded.height()), (4, 4));
    assert!(padded.as_slice().iter().all(|v| v.is_finite()));
}
