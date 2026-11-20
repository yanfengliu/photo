//! Histogram, grain noise and the thumbnail downscale.

use photo_core::colorimetry::RgbSpace;
use photo_core::geom::Rect;
use photo_core::image::Rgba8Image;
use photo_imaging::{Histogram, View, downscale_rgba8, grain_noise, grain_plane, histogram};

use crate::util::{Rng, mean, std_dev};

fn rgba(width: u32, height: u32, space: RgbSpace, f: impl Fn(u32, u32) -> [u8; 4]) -> Rgba8Image {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&f(x, y));
        }
    }
    Rgba8Image {
        width,
        height,
        pixels,
        space,
    }
}

fn noise_rgba(width: u32, height: u32, seed: u64) -> Rgba8Image {
    let mut rng = Rng::new(seed);
    let bytes: Vec<u8> = (0..width * height * 4)
        .map(|_| (rng.next_u64() >> 56) as u8)
        .collect();
    rgba(width, height, RgbSpace::Srgb, |x, y| {
        let i = ((y * width + x) * 4) as usize;
        [bytes[i], bytes[i + 1], bytes[i + 2], 255]
    })
}

fn total(bins: &[u32; 256]) -> u64 {
    bins.iter().map(|&count| u64::from(count)).sum()
}

#[test]
fn every_histogram_sums_to_the_pixel_count() {
    let image = noise_rgba(123, 77, 1);
    let histogram: Histogram = histogram(&image);
    let pixels = 123 * 77;
    assert_eq!(total(&histogram.red), pixels);
    assert_eq!(total(&histogram.green), pixels);
    assert_eq!(total(&histogram.blue), pixels);
    assert_eq!(total(&histogram.luma), pixels);
    assert_eq!(histogram.pixel_count(), pixels);
    // Noise fills every bin about evenly.
    assert!(histogram.red.iter().all(|&count| count > 5 && count < 90));
}

#[test]
fn channels_and_luminance_land_in_their_bins() {
    let image = rgba(3, 1, RgbSpace::Srgb, |x, _| match x {
        0 => [10, 200, 30, 255],
        1 => [10, 200, 30, 0],
        _ => [77, 77, 77, 255],
    });
    let histogram = histogram(&image);
    assert_eq!(
        (histogram.red[10], histogram.green[200], histogram.blue[30]),
        (2, 2, 2)
    );
    assert_eq!(
        (histogram.red[77], histogram.green[77], histogram.blue[77]),
        (1, 1, 1)
    );
    // 0.2126 * 10 + 0.7152 * 200 + 0.0722 * 30 = 147.3; alpha plays no part.
    assert_eq!(histogram.luma[147], 2);
    assert_eq!(histogram.luma[77], 1);
}

#[test]
fn a_grey_pixel_has_its_own_value_as_luminance_in_every_space() {
    for space in RgbSpace::ALL {
        let ramp = rgba(256, 1, space, |x, _| [x as u8, x as u8, x as u8, 255]);
        let histogram = histogram(&ramp);
        assert!(histogram.luma.iter().all(|&count| count == 1), "{space:?}");
    }
}

#[test]
fn luminance_uses_the_weights_of_the_image_space() {
    let green = |space| histogram(&rgba(1, 1, space, |_, _| [0, 255, 0, 255]));
    assert_eq!(green(RgbSpace::Srgb).luma[182], 1);
    assert_eq!(green(RgbSpace::Rec2020).luma[173], 1);
}

#[test]
fn clipping_is_the_fraction_of_pixels_with_any_channel_at_the_limit() {
    let image = rgba(4, 2, RgbSpace::Srgb, |x, y| match (x, y) {
        (0, _) => [255, 255, 255, 255],
        (1, 0) => [255, 40, 40, 255],
        (1, 1) => [40, 40, 0, 255],
        (2, 0) => [0, 0, 0, 255],
        _ => [128, 128, 128, 255],
    });
    let histogram = histogram(&image);
    assert_eq!(histogram.clipped_highlights, 3.0 / 8.0);
    assert_eq!(histogram.clipped_shadows, 2.0 / 8.0);
    // A pixel can be both: saturated red has blue and green at zero.
    let red = histogram_of_one([255, 0, 0, 255]);
    assert_eq!((red.clipped_shadows, red.clipped_highlights), (1.0, 1.0));
    let fine = histogram_of_one([1, 254, 128, 0]);
    assert_eq!((fine.clipped_shadows, fine.clipped_highlights), (0.0, 0.0));
}

fn histogram_of_one(pixel: [u8; 4]) -> Histogram {
    histogram(&rgba(1, 1, RgbSpace::Srgb, |_, _| pixel))
}

#[test]
fn an_empty_image_has_an_empty_histogram() {
    let histogram = histogram(&rgba(0, 0, RgbSpace::Srgb, |_, _| [0; 4]));
    assert_eq!(histogram.pixel_count(), 0);
    assert_eq!(
        (histogram.clipped_shadows, histogram.clipped_highlights),
        (0.0, 0.0)
    );
}

#[test]
fn grain_is_the_same_at_the_same_position_at_any_view_scale() {
    let size = 2.5;
    let full = View::new(Rect::from_size(256.0, 192.0), 256, 192);
    // Half scale, shifted by half an output pixel so that its pixel centres
    // (x.5 after the shift) coincide with every second centre of the full view.
    let half = View::new(Rect::new(-0.5, -0.5, 255.5, 191.5), 128, 96);
    let quarter = View::new(Rect::new(-1.5, -1.5, 254.5, 190.5), 64, 48);
    let (a, b, c) = (
        grain_plane(&full, size, 0),
        grain_plane(&half, size, 0),
        grain_plane(&quarter, size, 0),
    );
    for y in 0..48 {
        for x in 0..64 {
            assert_eq!(a.get(4 * x, 4 * y), c.get(x, y), "({x}, {y})");
            assert_eq!(a.get(4 * x, 4 * y), b.get(2 * x, 2 * y), "({x}, {y})");
        }
    }
    // And it is the documented point function of the output position.
    assert_eq!(a.get(10, 20), grain_noise(10.5, 20.5, size));
    assert_eq!(c.get(3, 7), grain_noise(12.5, 28.5, size));
}

#[test]
fn r1_a_tile_of_grain_matches_the_whole() {
    let view = View::new(Rect::from_size(300.0, 200.0), 150, 100);
    let whole = grain_plane(&view, 3.0, 0);
    let tile = grain_plane(&view.tile(40, 30, 50, 40), 3.0, 4);
    assert_eq!((tile.width(), tile.height()), (58, 48));
    assert_eq!(tile.crop(4, 4, 50, 40), whole.crop(40, 30, 50, 40));
    assert_eq!(
        tile.get(0, 0),
        whole.get(36, 26),
        "padding is real grain too"
    );
}

#[test]
fn grain_has_zero_mean_and_unit_variance_at_every_size() {
    for size in [1.0f32, 1.7, 3.0, 6.0] {
        let extent = 400.0 * size;
        let plane = grain_plane(
            &View::new(Rect::from_size(extent, extent), 800, 800),
            size,
            0,
        );
        let (m, s) = (mean(plane.as_slice()), std_dev(plane.as_slice()));
        assert!(m.abs() < 0.02, "size {size}: mean {m}");
        assert!((s - 1.0).abs() < 0.05, "size {size}: deviation {s}");
        assert!(
            plane.as_slice().iter().all(|v| v.abs() < 4.7),
            "size {size}"
        );
    }
}

fn correlation(size: f32, distance: usize) -> f32 {
    let plane = grain_plane(&View::new(Rect::from_size(600.0, 600.0), 600, 600), size, 0);
    let mut sum = 0.0;
    for y in 0..600 {
        for x in 0..600 - distance {
            sum += plane.get(x, y) * plane.get(x + distance, y);
        }
    }
    sum / (600 * (600 - distance)) as f32
}

#[test]
fn grain_is_band_limited_by_its_size() {
    // Smooth below the grain size...
    assert!(correlation(8.0, 1) > 0.9, "{}", correlation(8.0, 1));
    assert!(correlation(4.0, 1) > 0.65, "{}", correlation(4.0, 1));
    // ...unrelated well beyond it...
    assert!(
        correlation(8.0, 40).abs() < 0.05,
        "{}",
        correlation(8.0, 40)
    );
    assert!(
        correlation(2.0, 10).abs() < 0.05,
        "{}",
        correlation(2.0, 10)
    );
    // ...and with no energy at scales much larger than a grain: averaging
    // over a 10-grain block leaves almost nothing.
    let plane = grain_plane(&View::new(Rect::from_size(600.0, 600.0), 600, 600), 3.0, 0);
    let mut blocks = Vec::new();
    for by in 0..20 {
        for bx in 0..20 {
            let block = plane.crop(bx * 30, by * 30, 30, 30);
            blocks.push(mean(block.as_slice()));
        }
    }
    assert!(std_dev(&blocks) < 0.1, "{}", std_dev(&blocks));
}

#[test]
fn grain_depends_only_on_position_over_size() {
    assert_eq!(grain_noise(8.5, 4.25, 2.0), grain_noise(17.0, 8.5, 4.0));
    assert_ne!(grain_noise(8.5, 4.25, 2.0), grain_noise(8.5, 4.25, 4.0));
    assert_ne!(grain_noise(8.5, 4.25, 2.0), grain_noise(8.5, 5.25, 2.0));
    // Sizes below a pixel would alias; they are treated as one pixel.
    assert_eq!(grain_noise(8.5, 4.5, 0.1), grain_noise(8.5, 4.5, 1.0));
    assert_eq!(grain_noise(8.5, 4.5, f32::NAN), grain_noise(8.5, 4.5, 1.0));
    for (x, y) in [
        (-1.0e6f32, 3.0f32),
        (1.0e9, -1.0e9),
        (f32::NAN, 0.0),
        (f32::INFINITY, 1.0),
    ] {
        assert!(grain_noise(x, y, 2.0).is_finite(), "({x}, {y})");
    }
}

#[test]
fn downscaling_keeps_flat_colour_alpha_and_the_colour_space() {
    for space in RgbSpace::ALL {
        let image = rgba(37, 23, space, |_, _| [200, 100, 7, 255]);
        let small = downscale_rgba8(&image, 10, 6);
        assert_eq!((small.width, small.height, small.space), (10, 6, space));
        assert_eq!(small.pixels.len(), 10 * 6 * 4);
        assert!(
            small
                .pixels
                .chunks_exact(4)
                .all(|p| p == [200, 100, 7, 255]),
            "{space:?}"
        );
    }
}

#[test]
fn downscaling_to_the_same_size_is_an_exact_copy() {
    let image = noise_rgba(31, 17, 2);
    assert_eq!(downscale_rgba8(&image, 31, 17), image);
}

#[test]
fn downscaling_averages_in_linear_light() {
    // A one-pixel black and white checker averages to half the light, which
    // is code 188 in sRGB, not the 128 that averaging code values would give.
    let checker = rgba(64, 64, RgbSpace::Srgb, |x, y| {
        let v = if (x + y) % 2 == 0 { 255 } else { 0 };
        [v, v, v, 255]
    });
    let small = downscale_rgba8(&checker, 32, 32);
    assert!(
        small
            .pixels
            .chunks_exact(4)
            .all(|p| p == [188, 188, 188, 255])
    );
    // Whole blocks come through untouched.
    let blocks = rgba(8, 8, RgbSpace::Srgb, |x, y| {
        [(x / 2 * 50) as u8, (y / 2 * 60) as u8, 9, 255]
    });
    let small = downscale_rgba8(&blocks, 4, 4);
    assert_eq!(small.pixel(3, 2), [150, 120, 9, 255]);
    assert_eq!(small.pixel(0, 0), [0, 0, 9, 255]);
}

#[test]
fn downscaling_by_a_fractional_factor_weights_pixels_by_the_area_covered() {
    // Five columns into two: each output pixel covers 2.5 input columns.
    let lin = |code: u8| RgbSpace::Srgb.decode(code as f32 / 255.0);
    let row = [10u8, 250, 60, 130, 200];
    let image = rgba(5, 1, RgbSpace::Srgb, |x, _| {
        [row[x as usize], row[x as usize], row[x as usize], 255]
    });
    let small = downscale_rgba8(&image, 2, 1);
    let expected = [
        (lin(10) + lin(250) + 0.5 * lin(60)) / 2.5,
        (0.5 * lin(60) + lin(130) + lin(200)) / 2.5,
    ];
    for (x, want) in expected.iter().enumerate() {
        let got = small.pixel(x as u32, 0)[0];
        let code = (RgbSpace::Srgb.encode(*want) * 255.0).round() as u8;
        assert_eq!(got, code, "column {x}");
    }
}

#[test]
fn downscaling_a_photo_sized_image_conserves_its_light() {
    let image = noise_rgba(640, 427, 3);
    let small = downscale_rgba8(&image, 96, 64);
    let light = |image: &Rgba8Image| {
        let sum: f64 = image
            .pixels
            .chunks_exact(4)
            .map(|p| f64::from(RgbSpace::Srgb.decode(f32::from(p[1]) / 255.0)))
            .sum();
        sum / f64::from(image.width * image.height)
    };
    assert!(
        (light(&small) - light(&image)).abs() < 2e-3,
        "{} vs {}",
        light(&small),
        light(&image)
    );
    // Averaging 44 noise pixels per output pixel leaves little contrast.
    let greens: Vec<f32> = small
        .pixels
        .chunks_exact(4)
        .map(|p| f32::from(p[1]))
        .collect();
    assert!(std_dev(&greens) < 20.0, "{}", std_dev(&greens));
}

#[test]
fn degenerate_downscale_requests_do_not_panic() {
    let image = noise_rgba(9, 7, 4);
    let empty = downscale_rgba8(&image, 0, 5);
    assert_eq!((empty.width, empty.height, empty.pixels.len()), (0, 5, 0));
    let one = downscale_rgba8(&image, 1, 1);
    assert_eq!(one.pixels.len(), 4);
    // Asking for more pixels than there are repeats them; it never fails.
    let bigger = downscale_rgba8(&image, 18, 14);
    assert_eq!(bigger.pixel(0, 0), image.pixel(0, 0));
    assert_eq!(bigger.pixel(17, 13), image.pixel(8, 6));
    assert_eq!(bigger.pixel(5, 9), image.pixel(2, 4));
    let nothing = rgba(0, 0, RgbSpace::Srgb, |_, _| [0; 4]);
    assert_eq!(downscale_rgba8(&nothing, 4, 4).pixels.len(), 0);
}
