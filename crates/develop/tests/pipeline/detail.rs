//! Direction tests for presence, detail, lens corrections, transform and
//! effects.

use photo_core::develop::DevelopSettings;
use photo_core::image::Rgba8Image;
use photo_develop::Source;

use crate::fixtures::{chroma, edit, full, luma, mean, raster, raster_linear, render, rgb, spread};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

/// A patch of the dark, finely textured ground.
const GROUND: (u32, u32, u32, u32) = (300, 190, 120, 12);
/// The hazy sky just above the horizon.
const HAZE: (u32, u32, u32, u32) = (40, 150, 160, 20);

/// Spread of the difference between each pixel and the mean of its eight
/// neighbours: the finest detail's strength.
fn fine_detail(image: &Rgba8Image, (x0, y0, w, h): (u32, u32, u32, u32)) -> f32 {
    let value = |x: u32, y: u32| luma(image, x, y, 1, 1);
    let mut values = Vec::new();
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let around = (luma(image, x - 1, y - 1, 3, 3) * 9.0 - value(x, y)) / 8.0;
            values.push(value(x, y) - around);
        }
    }
    let average = values.iter().sum::<f32>() / values.len() as f32;
    (values.iter().map(|v| (v - average).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
}

fn rendered(source: &Source, settings: &DevelopSettings) -> Rgba8Image {
    render(source, settings, full(source, settings))
}

fn spread_of(image: &Rgba8Image, (x, y, w, h): (u32, u32, u32, u32)) -> f32 {
    spread(image, x, y, w, h)
}

#[test]
fn texture_clarity_and_sharpening_raise_and_lower_local_contrast() {
    let (_, source) = raster(WIDTH, HEIGHT);
    let base = rendered(&source, &DevelopSettings::default());
    let (fine, broad) = (fine_detail(&base, GROUND), spread_of(&base, GROUND));
    let check =
        |name: &str, more: DevelopSettings, less: Option<DevelopSettings>, fine_scale: bool| {
            let measure = |image: &Rgba8Image| {
                if fine_scale {
                    fine_detail(image, GROUND)
                } else {
                    spread_of(image, GROUND)
                }
            };
            let reference = if fine_scale { fine } else { broad };
            let up = measure(&rendered(&source, &more));
            assert!(up > reference * 1.05, "{name} +: {reference} -> {up}");
            if let Some(less) = less {
                let down = measure(&rendered(&source, &less));
                assert!(down < reference * 0.95, "{name} -: {reference} -> {down}");
            }
        };
    check(
        "texture",
        edit(|s| s.presence.texture = 100.0),
        Some(edit(|s| s.presence.texture = -100.0)),
        true,
    );
    check(
        "clarity",
        edit(|s| s.presence.clarity = 100.0),
        Some(edit(|s| s.presence.clarity = -100.0)),
        false,
    );
    check(
        "sharpening",
        edit(|s| s.detail.sharpening.amount = 120.0),
        None,
        true,
    );
}

#[test]
fn dehaze_clears_the_haze_and_negative_dehaze_adds_it() {
    let (_, source) = raster(WIDTH, HEIGHT);
    let haze = |settings: &DevelopSettings| {
        let image = rendered(&source, settings);
        let (x, y, w, h) = HAZE;
        (luma(&image, x, y, w, h), chroma(mean(&image, x, y, w, h)))
    };
    let (light, colour) = haze(&DevelopSettings::default());
    let (clear_light, clear_colour) = haze(&edit(|s| s.presence.dehaze = 80.0));
    let (hazy_light, hazy_colour) = haze(&edit(|s| s.presence.dehaze = -80.0));
    assert!(
        clear_light < light - 3.0 && clear_colour > colour + 3.0,
        "{light} {colour} -> {clear_light} {clear_colour}"
    );
    assert!(
        hazy_light > light + 3.0 && hazy_colour < colour - 3.0,
        "{light} {colour} -> {hazy_light} {hazy_colour}"
    );
}

#[test]
fn clarity_and_dehaze_keep_a_flat_patch_flat() {
    // Flat patches, a skin tone and a magenta, brighter than the textured
    // ground under a hazy sky: neither a halo along their rim nor a blotch
    // in their middle may appear.
    let (width, height) = (1200, 800);
    let patches = [
        (400u32, 500u32, [0.539f32, 0.305, 0.223]),
        (700, 500, [0.5, 0.05, 0.4]),
    ];
    let source = raster_linear(width, height, |x, y| {
        if y < 300 {
            return [0.55, 0.68, 0.85];
        }
        for (px, py, colour) in patches {
            if (px..px + 60).contains(&x) && (py..py + 60).contains(&y) {
                return colour;
            }
        }
        let weave = 1.0 + 0.3 * (x as f32 * 2.1 + y as f32 * 0.9).sin();
        [0.034 * weave, 0.038 * weave, 0.024 * weave]
    });
    // Before the fixes the rim stood 10 codes above the middle with clarity
    // and the middle 7 below the rim with dehaze. What is left is largest
    // in a corner, where the edge-aware means are interpolated across two
    // edges from the reduced grid: 3.5 codes over 3 pixels, invisible.
    for settings in [
        edit(|s| s.presence.clarity = 100.0),
        edit(|s| s.presence.dehaze = 100.0),
    ] {
        let image = rendered(&source, &settings);
        for (px, py, _) in patches {
            let block = |x: u32, y: u32| luma(&image, x, y, 3, 3);
            let centre = block(px + 29, py + 29);
            for (x, y) in [
                (px + 2, py + 29),
                (px + 55, py + 29),
                (px + 29, py + 2),
                (px + 3, py + 3),
            ] {
                let rim = block(x, y);
                assert!(
                    (rim - centre).abs() < 4.0,
                    "patch at {px} {:?}: rim ({x}, {y}) {rim} vs centre {centre}",
                    settings.presence
                );
            }
        }
    }
}

/// A flat grey frame with independent per-channel noise.
fn noisy_grey() -> Source {
    raster_linear(WIDTH, HEIGHT, |x, y| {
        let hash = |seed: u32| {
            let mut h = x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77) ^ seed;
            h ^= h >> 15;
            h = h.wrapping_mul(0x2c1b_3c6d);
            h ^= h >> 12;
            (h % 1000) as f32 / 1000.0 - 0.5
        };
        [
            0.18 * (1.0 + 0.3 * hash(1)),
            0.18 * (1.0 + 0.3 * hash(2)),
            0.18 * (1.0 + 0.3 * hash(3)),
        ]
    })
}

#[test]
fn noise_reduction_smooths_luminance_and_colour_noise() {
    let source = noisy_grey();
    let region = (100, 100, 200, 100);
    let colour_noise = |image: &Rgba8Image| {
        let (x0, y0, w, h) = region;
        let mut sum = 0.0;
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                sum += chroma(rgb(image, x, y).map(f32::from));
            }
        }
        sum / (w * h) as f32
    };
    let base = rendered(&source, &DevelopSettings::default());
    let luminance = rendered(
        &source,
        &edit(|s| s.detail.noise_reduction.luminance = 80.0),
    );
    assert!(spread_of(&luminance, region) < spread_of(&base, region) * 0.8);
    let colour = rendered(&source, &edit(|s| s.detail.noise_reduction.color = 80.0));
    assert!(colour_noise(&colour) < colour_noise(&base) * 0.8);
}

/// A flat grey frame with a dark vertical line at a tenth of its width.
fn grey_with_line() -> Source {
    raster_linear(WIDTH, HEIGHT, |x, _| {
        if x == WIDTH / 10 { [0.01; 3] } else { [0.2; 3] }
    })
}

/// The column of the darkest pixel along the middle row.
fn darkest_column(image: &Rgba8Image) -> u32 {
    (0..image.width)
        .min_by_key(|&x| rgb(image, x, image.height / 2)[1])
        .unwrap()
}

#[test]
fn lens_and_transform_move_and_brighten_what_they_say() {
    let source = grey_with_line();
    let base = rendered(&source, &DevelopSettings::default());
    let corner_vs_centre = |image: &Rgba8Image| {
        luma(image, 2, 2, 20, 20) - luma(image, WIDTH / 2 - 10, HEIGHT / 2 - 10, 20, 20)
    };
    let brighter = rendered(&source, &edit(|s| s.lens.vignetting = 100.0));
    let darker = rendered(&source, &edit(|s| s.lens.vignetting = -100.0));
    let flat = corner_vs_centre(&base);
    assert!(corner_vs_centre(&brighter) > flat + 10.0 && corner_vs_centre(&darker) < flat - 10.0);
    // Correcting barrel distortion draws the edges' content inward.
    let line = darkest_column(&base);
    let corrected = rendered(&source, &edit(|s| s.lens.distortion = 100.0));
    assert!(
        darkest_column(&corrected) > line + 2,
        "{line} -> {}",
        darkest_column(&corrected)
    );
    // A positive horizontal offset moves the picture right.
    let moved = rendered(&source, &edit(|s| s.transform.x_offset = 40.0));
    assert!(
        darkest_column(&moved) > line + 20,
        "{line} -> {}",
        darkest_column(&moved)
    );
}

#[test]
fn the_post_crop_vignette_and_grain_do_what_they_say() {
    let source = raster_linear(WIDTH, HEIGHT, |_, _| [0.2; 3]);
    let corner = |image: &Rgba8Image| luma(image, 2, 2, 20, 20);
    let centre = |image: &Rgba8Image| luma(image, WIDTH / 2 - 10, HEIGHT / 2 - 10, 20, 20);
    let base = rendered(&source, &DevelopSettings::default());
    let dark = rendered(&source, &edit(|s| s.effects.vignette.amount = -80.0));
    let light = rendered(&source, &edit(|s| s.effects.vignette.amount = 80.0));
    assert!(corner(&dark) < corner(&base) - 20.0 && corner(&light) > corner(&base) + 20.0);
    assert!(
        (centre(&dark) - centre(&base)).abs() < 1.0,
        "the centre moved"
    );
    let grainy = rendered(&source, &edit(|s| s.effects.grain.amount = 60.0));
    let region = (100, 100, 200, 100);
    assert!(spread_of(&grainy, region) > spread_of(&base, region) + 1.0);
    assert!((luma(&grainy, 100, 100, 200, 100) - luma(&base, 100, 100, 200, 100)).abs() < 1.0);
}
