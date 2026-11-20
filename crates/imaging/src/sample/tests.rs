use photo_core::colorimetry::RgbSpace;
use photo_core::image::SourceColor;

use super::*;

fn ramp() -> SourceImage {
    let pixels = (0..6u16 * 5)
        .flat_map(|i| [i * 1000, i * 500, 65535 - i * 100])
        .collect();
    SourceImage::new(6, 5, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap()
}

/// A scrambled image: every pixel unrelated to its neighbours.
fn scrambled() -> SourceImage {
    let pixels = (0..12u32 * 9 * 3)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u16)
        .collect();
    SourceImage::new(12, 9, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap()
}

/// The pure triangle of a given half-width.
fn triangle(radius: f64) -> Kernel {
    let kernel = Kernel::for_footprint(radius);
    assert_eq!(kernel.triangle, 1.0, "{radius} is still in the blend");
    kernel
}

/// `sample4` by its definition, reading every tap through a clamp.
fn reference(image: &SourceImage, x: f64, y: f64, kernel: Kernel) -> [f32; 3] {
    let (ix, tx) = split(x);
    let (iy, ty) = split(y);
    let (wx, wy) = (weights(tx, kernel), weights(ty, kernel));
    let tap = |i: i64, j: i64, channel: usize| {
        let column = clamp_index(ix - 1 + i, image.width as usize) as u32;
        let line = clamp_index(iy - 1 + j, image.height as usize) as u32;
        f32::from(image.pixel(column, line)[channel])
    };
    let range = |taps: &[f32]| {
        taps.iter()
            .fold((f32::INFINITY, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)))
    };
    std::array::from_fn(|channel| {
        let down = |i: i64| {
            wy[0] * tap(i, 0, channel)
                + wy[1] * tap(i, 1, channel)
                + wy[2] * tap(i, 2, channel)
                + wy[3] * tap(i, 3, channel)
        };
        let value = wx[0] * down(0) + wx[1] * down(1) + wx[2] * down(2) + wx[3] * down(3);
        if kernel.triangle >= 1.0 {
            return value / WHITE;
        }
        let near = [(1, 1), (2, 1), (1, 2), (2, 2)].map(|(i, j)| tap(i, j, channel));
        let all: Vec<f32> = (0..16).map(|k| tap(k % 4, k / 4, channel)).collect();
        let ((mut low, mut high), (lowest, highest)) = (range(&near), range(&all));
        if kernel.triangle > 0.0 {
            low += (lowest - low) * kernel.triangle;
            high += (highest - high) * kernel.triangle;
        }
        value.clamp(low, high) / WHITE
    })
}

#[test]
fn the_fast_paths_agree_with_the_definition_everywhere() {
    for image in [ramp(), scrambled()] {
        for kernel in [Kernel::CUBIC, Kernel::for_footprint(1.2), triangle(1.6)] {
            // A grid of positions from well outside the image, across
            // both borders and the interior: off the pixel grid, and
            // then exactly on pixel centres, which take a shortcut.
            for (pitch_x, pitch_y, offset) in [(0.23, 0.19, 0.0), (1.0, 1.0, 0.5), (1.0, 0.25, 0.5)]
            {
                for step_y in -12..60 {
                    for step_x in -12..70 {
                        let (x, y) = (
                            step_x as f64 * pitch_x + offset,
                            step_y as f64 * pitch_y + offset,
                        );
                        assert_eq!(
                            sample4(&image, x, y, kernel),
                            reference(&image, x, y, kernel),
                            "{kernel:?} at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn weights_sum_to_one_and_the_cubic_interpolates() {
    for footprint in [0.3, 1.0, 1.1, 1.3, 1.5, 2.0, 7.0] {
        let kernel = Kernel::for_footprint(footprint);
        for step in 0..=16 {
            let w = weights(step as f32 / 16.0, kernel);
            assert!(
                (w.iter().sum::<f32>() - 1.0).abs() < 1e-6,
                "{kernel:?} {step}"
            );
        }
    }
    assert_eq!(weights(0.0, Kernel::CUBIC), [0.0, 1.0, 0.0, 0.0]);
    assert_eq!(weights(1.0, Kernel::CUBIC), [0.0, 0.0, 1.0, 0.0]);
}

#[test]
fn the_kernel_follows_the_footprint_without_a_jump() {
    // Magnifying, 1:1 and an exact size that has been through f32 all
    // get the cubic alone.
    for footprint in [0.25, 1.0, 1.0005] {
        assert_eq!(
            Kernel::for_footprint(footprint),
            Kernel::CUBIC,
            "{footprint}"
        );
    }
    assert!(Kernel::CUBIC.interpolates());
    // From there the triangle fades in; it is alone from 1.5 and as wide
    // as the footprint, up to the two pixels four taps can hold.
    let share = |footprint: f64| Kernel::for_footprint(footprint).triangle;
    assert!(share(1.0025) > 0.0 && share(1.0025) < 1e-4);
    assert!(share(1.1) < share(1.25) && share(1.25) < share(1.4));
    assert_eq!((share(1.5), share(1.9), share(40.0)), (1.0, 1.0, 1.0));
    assert_eq!(Kernel::for_footprint(1.7).inverse_radius, 1.0 / 1.7f32);
    assert_eq!(Kernel::for_footprint(40.0).inverse_radius, 0.5);
    assert!(!Kernel::for_footprint(1.0025).interpolates());
    // Neighbouring footprints give neighbouring weights, all the way.
    let mut previous = weights(0.3, Kernel::for_footprint(0.9));
    for step in 0..=1200 {
        let current = weights(0.3, Kernel::for_footprint(0.9 + step as f64 * 0.001));
        for (a, b) in previous.iter().zip(current) {
            assert!(
                (a - b).abs() < 3e-3,
                "step {step}: {previous:?} -> {current:?}"
            );
        }
        previous = current;
    }
}

#[test]
fn a_triangle_never_has_negative_weights_and_is_symmetric() {
    for step in 0..=16 {
        let t = step as f32 / 16.0;
        let forward = weights(t, triangle(1.5));
        let mut mirrored = weights(1.0 - t, triangle(1.5));
        mirrored.reverse();
        for (a, b) in forward.iter().zip(mirrored) {
            assert!(*a >= 0.0 && (a - b).abs() < 1e-6);
        }
    }
}

#[test]
fn a_mix_never_leaves_the_range_of_its_sixteen_pixels() {
    let image = scrambled();
    for footprint in [1.0, 1.05, 1.2, 1.45, 1.8] {
        let kernel = Kernel::for_footprint(footprint);
        for step in 0..400 {
            let (x, y) = (2.0 + step as f64 * 0.019, 2.0 + step as f64 * 0.011);
            let got = sample4(&image, x, y, kernel);
            let (ix, iy) = (split(x).0, split(y).0);
            for channel in 0..3 {
                let taps = (0..16).map(|k| {
                    let pixel = image.pixel((ix - 1 + k % 4) as u32, (iy - 1 + k / 4) as u32);
                    f32::from(pixel[channel]) / WHITE
                });
                let (low, high) = taps.fold((1.0f32, 0.0f32), |(l, h), v| (l.min(v), h.max(v)));
                let value = got[channel];
                assert!(
                    value >= low - 1e-6 && value <= high + 1e-6,
                    "{footprint} at ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn both_samplers_return_the_pixel_at_its_centre() {
    let image = ramp();
    for (x, y) in [(0u32, 0u32), (3, 2), (5, 4)] {
        let expected = image.pixel(x, y).map(|v| v as f32 / WHITE);
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        assert_eq!(bilinear(&image, fx, fy), expected);
        assert_eq!(sample4(&image, fx, fy, Kernel::CUBIC), expected);
    }
}

#[test]
fn the_cubic_reproduces_a_linear_ramp_between_centres() {
    let image = ramp();
    // Red rises by 1000 per pixel along a row, away from the edges.
    let value = sample4(&image, 2.75, 2.5, Kernel::CUBIC)[0] * WHITE;
    let left = image.pixel(1, 2)[0] as f32;
    assert!((value - (left + 1250.0)).abs() < 0.01, "{value}");
}

#[test]
fn positions_outside_the_image_and_non_finite_ones_stay_finite() {
    let image = ramp();
    let corner = image.pixel(5, 4).map(|v| v as f32 / WHITE);
    for kernel in [Kernel::CUBIC, Kernel::for_footprint(1.2), triangle(1.7)] {
        let far = sample4(&image, 1.0e12, 1.0e12, kernel);
        for channel in 0..3 {
            assert!((far[channel] - corner[channel]).abs() < 1e-6);
        }
        assert!(
            sample4(&image, f64::NAN, f64::INFINITY, kernel)
                .iter()
                .all(|v| v.is_finite())
        );
    }
    assert_eq!(bilinear(&image, 1.0e12, 1.0e12), corner);
    assert!(
        bilinear(&image, f64::NAN, 0.0)
            .iter()
            .all(|v| v.is_finite())
    );
}
