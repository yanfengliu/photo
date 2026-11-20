//! The source pyramid: its levels, how they are built, sampled and swapped.

use std::sync::Arc;

use photo_core::image::SourceImage;
use photo_imaging::SourcePyramid;

use crate::util::{noise_source, source_from_fn, unit};

fn source_pixel(image: &SourceImage, x: u32, y: u32) -> [f32; 3] {
    image.pixel(x, y).map(unit)
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
