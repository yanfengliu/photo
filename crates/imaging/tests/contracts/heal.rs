//! Healing and cloning spots, and the suggested source position.

use std::sync::Arc;

use photo_core::develop::{Spot, SpotMode};
use photo_core::geom::Point;
use photo_core::image::SourceImage;
use photo_imaging::{SourcePyramid, heal_image, heal_level, suggest_heal_source};

use crate::util::{Rng, mean, noise_source, source_from_fn, std_dev, unit};

fn spot(mode: SpotMode, target: (f32, f32), source: (f32, f32), radius: f32) -> Spot {
    Spot {
        mode,
        target: Point::new(target.0, target.1),
        source: Point::new(source.0, source.1),
        radius,
        feather: 0.0,
        opacity: 100.0,
    }
}

/// Distance from the centre of pixel `(x, y)` to `centre`, in pixels.
fn distance(x: u32, y: u32, centre: (f32, f32)) -> f32 {
    (x as f32 + 0.5 - centre.0).hypot(y as f32 + 0.5 - centre.1)
}

#[test]
fn a_hard_clone_copies_exactly_inside_and_leaves_the_outside_alone() {
    let image = noise_source(120, 100, 1);
    // Target (60, 50), radius 12 px, source 30 px right and 10 px up.
    let clone = spot(SpotMode::Clone, (0.5, 0.5), (0.75, 0.4), 0.1);
    let healed = heal_image(&image, &[clone]);
    assert_eq!(
        (healed.width, healed.height, healed.color),
        (120, 100, image.color)
    );
    let (mut inside, mut outside) = (0, 0);
    for y in 0..100u32 {
        for x in 0..120u32 {
            let d = distance(x, y, (60.0, 50.0));
            if d <= 11.5 {
                assert_eq!(
                    healed.pixel(x, y),
                    image.pixel(x + 30, y - 10),
                    "inside at ({x}, {y})"
                );
                inside += 1;
            } else if d >= 12.5 {
                assert_eq!(
                    healed.pixel(x, y),
                    image.pixel(x, y),
                    "outside at ({x}, {y})"
                );
                outside += 1;
            }
        }
    }
    assert!(inside > 380 && outside > 11_000);
    assert_eq!(
        image,
        noise_source(120, 100, 1),
        "the original is never written to"
    );
}

#[test]
fn no_spots_or_spots_with_no_effect_return_an_identical_copy() {
    let image = noise_source(60, 40, 2);
    assert_eq!(heal_image(&image, &[]), image);
    let mut invisible = spot(SpotMode::Heal, (0.5, 0.5), (0.2, 0.2), 0.1);
    invisible.opacity = 0.0;
    let mut pointless = spot(SpotMode::Clone, (0.5, 0.5), (0.2, 0.2), 0.0);
    pointless.feather = 50.0;
    let broken = spot(SpotMode::Clone, (f32::NAN, 0.5), (0.2, 0.2), 0.1);
    let nowhere = spot(SpotMode::Heal, (7.0, -3.0), (0.2, 0.2), 0.1);
    assert_eq!(
        heal_image(&image, &[invisible, pointless, broken, nowhere]),
        image
    );
}

#[test]
fn out_of_range_spot_settings_are_clamped_not_trusted() {
    let image = noise_source(60, 40, 9);
    let plain = spot(SpotMode::Heal, (0.5, 0.5), (0.2, 0.3), 0.1);
    let expected = heal_image(&image, &[plain]);
    // An unusable feather is no feather; opacity stops at 100.
    for (feather, opacity) in [
        (f32::NAN, 100.0),
        (-40.0, 100.0),
        (0.0, 900.0),
        (f32::NEG_INFINITY, f32::INFINITY),
    ] {
        let odd = Spot {
            feather,
            opacity,
            ..plain
        };
        let healed = heal_image(&image, &[odd]);
        assert!(healed == expected, "feather {feather}, opacity {opacity}");
    }
    let unseen = Spot {
        opacity: f32::NAN,
        ..plain
    };
    assert!(
        heal_image(&image, &[unseen]) == image,
        "a NaN opacity is no opacity"
    );
}

#[test]
fn a_source_absurdly_far_away_reads_the_nearest_edge() {
    // 1e20 widths to the right: far past where a pixel offset still fits
    // in an integer. Inside the 8 px spot every pixel is then a copy of the
    // image's right-hand column, as for any source beyond the edge.
    let image = noise_source(80, 60, 12);
    let far = spot(SpotMode::Clone, (0.5, 0.5), (1.0e20, 0.5), 0.1);
    let healed = heal_image(&image, &[far]);
    assert_eq!(healed.pixel(40, 30), image.pixel(79, 30));
    assert_eq!(healed.pixel(36, 27), image.pixel(79, 27));
    let below = spot(SpotMode::Heal, (0.5, 0.5), (0.5, -1.0e20), 0.1);
    let healed = heal_image(
        &image,
        &[
            below,
            Spot {
                mode: SpotMode::Heal,
                ..far
            },
        ],
    );
    assert_eq!(healed.pixels.len(), image.pixels.len());
}

#[test]
fn spots_larger_than_the_picture_are_held_at_its_size() {
    // The radius is held at the long edge, which also bounds the work a
    // spot can ask for.
    let image = noise_source(120, 80, 13);
    let huge = spot(SpotMode::Heal, (0.5, 0.5), (0.6, 0.5), 30.0);
    let largest = Spot {
        radius: 1.0,
        ..huge
    };
    assert!(heal_image(&image, &[huge]) == heal_image(&image, &[largest]));
    let endless = Spot {
        radius: f32::INFINITY,
        ..huge
    };
    assert!(heal_image(&image, &[endless]) == heal_image(&image, &[largest]));
}

#[test]
fn opacity_and_feather_blend_the_copy_in() {
    let image = source_from_fn(100, 100, |x, _| [if x < 50 { 0.2 } else { 0.8 }; 3]);
    let (dark, bright) = (image.pixel(0, 0)[0] as f32, image.pixel(99, 0)[0] as f32);
    // Target (25, 50) on the dark side, source (75, 50) on the bright side.
    let mut half = spot(SpotMode::Clone, (0.25, 0.5), (0.75, 0.5), 0.1);
    half.opacity = 50.0;
    let healed = heal_image(&image, &[half]);
    let centre = healed.pixel(25, 50)[0] as f32;
    assert!((centre - (dark + bright) / 2.0).abs() <= 1.0, "{centre}");

    let mut soft = spot(SpotMode::Clone, (0.25, 0.5), (0.75, 0.5), 0.1);
    soft.feather = 100.0;
    let healed = heal_image(&image, &[soft]);
    let fraction = |x: u32| (healed.pixel(x, 50)[0] as f32 - dark) / (bright - dark);
    // Soft from the centre: full in the middle, half at half the radius,
    // nothing at the rim, falling all the way.
    assert!(fraction(25) > 0.98, "{}", fraction(25));
    assert!((fraction(30) - 0.42).abs() < 0.05, "{}", fraction(30));
    assert!(fraction(34) < 0.03 && fraction(34) > 0.0);
    assert_eq!(healed.pixel(36, 50), image.pixel(36, 50));
    for x in 26..36 {
        assert!(fraction(x) <= fraction(x - 1));
    }
}

/// A smooth, slightly tinted gradient with a dark blob of radius 7 px in the
/// middle, and the same gradient without the blob.
fn blemished() -> (SourceImage, SourceImage) {
    let gradient = |x: u32, y: u32| {
        let level = 0.25 + 0.3 * x as f32 / 200.0 + 0.1 * y as f32 / 160.0;
        [level, level * 0.9, level * 0.75]
    };
    let clean = source_from_fn(200, 160, gradient);
    let marked = source_from_fn(200, 160, |x, y| {
        let d = distance(x, y, (100.0, 80.0));
        let darkening = 1.0 - 0.7 * (1.0 - ((d - 5.0) / 2.0).clamp(0.0, 1.0));
        gradient(x, y).map(|v| v * darkening)
    });
    (marked, clean)
}

/// Mean of each channel over the disc of `radius` px around `centre`.
fn disc_mean(image: &SourceImage, centre: (f32, f32), radius: f32) -> [f32; 3] {
    let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
    for y in 0..image.height {
        for x in 0..image.width {
            if distance(x, y, centre) <= radius {
                let p = image.pixel(x, y).map(unit);
                (0..3).for_each(|c| sum[c] += p[c]);
                count += 1.0;
            }
        }
    }
    sum.map(|s| s / count)
}

#[test]
fn healing_removes_a_dark_blob_from_a_smooth_gradient() {
    let (marked, clean) = blemished();
    let before = disc_mean(&marked, (100.0, 80.0), 9.0);
    let wanted = disc_mean(&clean, (100.0, 80.0), 9.0);
    assert!(
        wanted[1] - before[1] > 0.1,
        "the blob is clearly visible to begin with"
    );

    // Radius 14 px around the blob; texture from 50 px right and 8 px down,
    // where the gradient is a good deal brighter.
    let heal = Spot {
        feather: 30.0,
        ..spot(SpotMode::Heal, (0.5, 0.5), (0.75, 0.55), 0.07)
    };
    let healed = heal_image(&marked, &[heal]);
    let after = disc_mean(&healed, (100.0, 80.0), 9.0);
    for channel in 0..3 {
        assert!(
            (after[channel] - wanted[channel]).abs() < 0.006,
            "channel {channel}: {} vs {}",
            after[channel],
            wanted[channel]
        );
    }
    // Not just on average: no pixel of the repair strays far from the
    // gradient that should have been there, and there is no visible seam.
    for y in 60..100u32 {
        for x in 80..120u32 {
            let (got, want) = (unit(healed.pixel(x, y)[1]), unit(clean.pixel(x, y)[1]));
            assert!((got - want).abs() < 0.012, "({x}, {y}): {got} vs {want}");
        }
    }
    // A plain clone of the same area would have left a bright disc instead.
    let cloned = heal_image(
        &marked,
        &[Spot {
            mode: SpotMode::Clone,
            ..heal
        }],
    );
    assert!(disc_mean(&cloned, (100.0, 80.0), 9.0)[1] - wanted[1] > 0.05);
}

#[test]
fn healing_brings_the_texture_of_the_source_at_the_tone_of_the_target() {
    // Left half: flat and dark with a warm tint. Right half: bright, neutral,
    // with a strong one-pixel checker texture.
    let image = source_from_fn(200, 100, |x, y| {
        if x < 100 {
            [0.24, 0.2, 0.16]
        } else {
            [if (x + y) % 2 == 0 { 0.66 } else { 0.54 }; 3]
        }
    });
    let heal = spot(SpotMode::Heal, (0.25, 0.5), (0.75, 0.5), 0.06);
    let healed = heal_image(&image, &[heal]);
    let mut ratios = [Vec::new(), Vec::new(), Vec::new()];
    for y in 40..60u32 {
        for x in 40..60u32 {
            if distance(x, y, (50.0, 50.0)) <= 9.0 {
                let (got, flat) = (healed.pixel(x, y), image.pixel(x, y));
                (0..3).for_each(|c| ratios[c].push(got[c] as f32 / flat[c] as f32));
            }
        }
    }
    for (channel, ratio) in ratios.iter().enumerate() {
        // The tone and the tint are the target's own...
        assert!(
            (mean(ratio) - 1.0).abs() < 0.02,
            "channel {channel}: mean {}",
            mean(ratio)
        );
        // ...and the checker came along at its own relative contrast (+-10%).
        assert!(
            (std_dev(ratio) - 0.1).abs() < 0.02,
            "channel {channel}: texture {}",
            std_dev(ratio)
        );
    }
}

#[test]
fn spots_apply_in_order_each_seeing_the_one_before() {
    let image = noise_source(160, 60, 3);
    // First copy the patch at x = 120 to x = 80, then the patch at x = 80
    // (now holding the copy) to x = 40.
    let first = spot(SpotMode::Clone, (0.5, 0.5), (0.75, 0.5), 0.05);
    let second = spot(SpotMode::Clone, (0.25, 0.5), (0.5, 0.5), 0.05);
    let healed = heal_image(&image, &[first, second]);
    assert_eq!(healed.pixel(40, 30), image.pixel(120, 30));
    assert_eq!(healed.pixel(43, 27), image.pixel(123, 27));
    // In the other order the second spot copies what was there originally.
    let reversed = heal_image(&image, &[second, first]);
    assert_eq!(reversed.pixel(40, 30), image.pixel(80, 30));
}

#[test]
fn overlapping_source_and_target_read_the_pixels_from_before_the_spot() {
    let image = noise_source(100, 60, 4);
    // The source is only 6 px right of the target, with a 10 px radius.
    let near = spot(SpotMode::Clone, (0.5, 0.5), (0.56, 0.5), 0.1);
    let healed = heal_image(&image, &[near]);
    for (x, y) in [(50u32, 30u32), (44, 30), (55, 33), (50, 22)] {
        assert_eq!(healed.pixel(x, y), image.pixel(x + 6, y), "({x}, {y})");
    }
}

#[test]
fn spots_at_the_border_and_sources_outside_the_image_are_safe() {
    let image = noise_source(80, 60, 5);
    let corner = spot(SpotMode::Heal, (0.0, 0.0), (0.3, 0.3), 0.2);
    let far_source = spot(SpotMode::Clone, (0.9, 0.9), (4.0, -2.0), 0.15);
    let huge = spot(SpotMode::Heal, (0.5, 0.5), (0.6, 0.5), 3.0);
    let healed = heal_image(&image, &[corner, far_source, huge]);
    assert_eq!(healed.pixels.len(), image.pixels.len());
    // A source far outside reads the replicated edge: here the top-right pixel.
    let cloned = heal_image(&image, &[far_source]);
    assert_eq!(cloned.pixel(72, 54), image.pixel(79, 0));
}

#[test]
fn a_pyramid_level_heals_like_the_full_resolution_image() {
    let (marked, clean) = blemished();
    let pyramid = SourcePyramid::new(marked.clone());
    let heal = Spot {
        feather: 30.0,
        ..spot(SpotMode::Heal, (0.5, 0.5), (0.75, 0.55), 0.07)
    };
    let full = heal_level(&pyramid, 0, &[heal]);
    assert_eq!(full, heal_image(&marked, &[heal]));
    let half = heal_level(&pyramid, 1, &[heal]);
    assert_eq!((half.width, half.height), (100, 80));
    // The blob is gone at half size too, at the same place...
    let wanted = disc_mean(
        &SourcePyramid::new(clean).level(1).clone(),
        (50.0, 40.0),
        4.5,
    );
    let after = disc_mean(&half, (50.0, 40.0), 4.5);
    assert!(
        (after[1] - wanted[1]).abs() < 0.008,
        "{} vs {}",
        after[1],
        wanted[1]
    );
    // ...and the result is what halving the full-size repair would give.
    let reference = SourcePyramid::new(full);
    let reference = reference.level(1);
    let worst = half
        .pixels
        .iter()
        .zip(&reference.pixels)
        .map(|(&a, &b)| (unit(a) - unit(b)).abs())
        .fold(0.0, f32::max);
    assert!(worst < 0.03, "{worst}");
    // The healed level drops back into the pyramid for resampling.
    let swapped = pyramid.with_level(1, Arc::new(half)).unwrap();
    assert_eq!(swapped.level(1).width, 100);
}

fn half_textured(width: u32, height: u32) -> SourceImage {
    let mut rng = Rng::new(6);
    let noise: Vec<f32> = (0..width * height).map(|_| rng.unit()).collect();
    source_from_fn(width, height, |x, y| {
        if x < width / 2 {
            [0.4; 3]
        } else {
            [0.25 + 0.4 * noise[(y * width + x) as usize]; 3]
        }
    })
}

#[test]
fn the_suggested_source_is_nearby_inside_the_image_and_clear_of_the_target() {
    let pyramid = SourcePyramid::new(noise_source(400, 300, 7));
    let radius = 0.03;
    let radius_px = radius * 400.0;
    for target in [
        (0.5, 0.5),
        (0.04, 0.05),
        (0.97, 0.5),
        (0.5, 0.96),
        (0.98, 0.98),
    ] {
        let target = Point::new(target.0, target.1);
        let source = suggest_heal_source(&pyramid, target, radius);
        let (sx, sy) = (source.x * 400.0, source.y * 300.0);
        assert!(
            sx >= radius_px && sx <= 400.0 - radius_px,
            "{target:?} -> {source:?}"
        );
        assert!(
            sy >= radius_px && sy <= 300.0 - radius_px,
            "{target:?} -> {source:?}"
        );
        let apart = (sx - target.x * 400.0).hypot(sy - target.y * 300.0);
        assert!(
            apart >= 2.0 * radius_px && apart <= 5.0 * radius_px,
            "{target:?}: {apart}"
        );
        assert_eq!(
            source,
            suggest_heal_source(&pyramid, target, radius),
            "deterministic"
        );
    }
}

#[test]
fn the_suggested_source_has_surroundings_like_the_target() {
    let pyramid = SourcePyramid::new(half_textured(400, 300));
    let radius = 0.03;
    // Just inside the flat half, 8% of the width from the textured half.
    let flat = suggest_heal_source(&pyramid, Point::new(0.42, 0.5), radius);
    assert!(
        flat.x < 0.5 - radius,
        "a flat target wants a flat source, got {flat:?}"
    );
    // Just inside the textured half.
    let textured = suggest_heal_source(&pyramid, Point::new(0.58, 0.5), radius);
    assert!(
        textured.x > 0.5 + radius,
        "a textured target wants texture, got {textured:?}"
    );
}

#[test]
fn a_suggestion_is_always_usable_even_when_nothing_fits() {
    let pyramid = SourcePyramid::new(noise_source(40, 30, 8));
    // A spot bigger than the image: no candidate fits anywhere.
    let source = suggest_heal_source(&pyramid, Point::new(0.5, 0.5), 0.8);
    assert!((0.0..=1.0).contains(&source.x) && (0.0..=1.0).contains(&source.y));
    assert_ne!(source, Point::new(0.5, 0.5));
    let source = suggest_heal_source(&pyramid, Point::new(f32::NAN, 0.5), -1.0);
    assert!(source.x.is_finite() && source.y.is_finite());
    // Absurd but finite inputs still give a point inside the image.
    let in_image = |p: Point| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y);
    for (target, radius) in [
        (Point::new(0.5, 0.5), 1.0e36),
        (Point::new(0.5, 0.5), f32::INFINITY),
        (Point::new(3.0e38, -3.0e38), 0.05),
        (Point::new(f32::INFINITY, 0.2), 0.05),
    ] {
        let source = suggest_heal_source(&pyramid, target, radius);
        assert!(in_image(source), "{target:?}, {radius}: {source:?}");
    }
}
