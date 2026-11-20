//! Suggesting where a healing spot should take its patch from.

use photo_core::geom::Point;
use photo_core::image::SourceImage;
use photo_imaging::{SourcePyramid, suggest_heal_source};

use crate::util::{Rng, noise_source, source_from_fn};

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
