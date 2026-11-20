//! The region contracts R1 to R4 (DESIGN §5.9).

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{DevelopSettings, Mask, MaskShape, Spot, SpotMode};
use photo_core::geom::{Point, Rect};
use photo_core::image::Rgba8Image;
use photo_develop::{Source, auto_tone, auto_white_balance};
use photo_imaging::View;

use crate::fixtures::{
    camera_scene, crop, edit, full, max_difference, pools, raster, render, render16, rgb,
};

/// Every spatial stage active, with two masks and a healing spot.
pub fn everything() -> DevelopSettings {
    edit(|s| {
        s.tone.exposure = 0.3;
        s.tone.contrast = 15.0;
        s.tone.highlights = -60.0;
        s.tone.shadows = 50.0;
        s.presence.dehaze = 30.0;
        s.presence.clarity = 40.0;
        s.presence.texture = 50.0;
        s.presence.vibrance = 20.0;
        s.detail.sharpening.amount = 60.0;
        s.detail.sharpening.masking = 30.0;
        s.detail.noise_reduction.luminance = 40.0;
        s.detail.noise_reduction.color = 40.0;
        s.lens.vignetting = 30.0;
        s.effects.vignette.amount = -30.0;
        s.effects.grain.amount = 30.0;
        s.spots.push(Spot {
            mode: SpotMode::Heal,
            target: Point::new(0.3, 0.75),
            source: Point::new(0.45, 0.72),
            radius: 0.04,
            feather: 50.0,
            opacity: 100.0,
        });
        let mut radial = Mask::from_shape(
            MaskShape::Radial {
                center: Point::new(0.6, 0.45),
                radius_x: 0.25,
                radius_y: 0.15,
                angle: 20.0,
                feather: 60.0,
                invert: false,
            },
            1,
        );
        radial.adjust.exposure = 0.7;
        radial.adjust.clarity = 30.0;
        radial.adjust.saturation = -40.0;
        radial.adjust.noise = 30.0;
        radial.adjust.temp = 20.0;
        radial.adjust.sharpness = 40.0;
        radial.adjust.texture = -30.0;
        let mut linear = Mask::from_shape(
            MaskShape::Linear {
                start: Point::new(0.5, 0.0),
                end: Point::new(0.5, 0.5),
            },
            2,
        );
        linear.adjust.highlights = -40.0;
        linear.adjust.dehaze = 30.0;
        s.masks = vec![radial, linear];
    })
}

/// The worst difference between tiles of `view` and the same regions of
/// the whole render.
fn worst_tile_difference(source: &Source, settings: &DevelopSettings, view: View) -> u8 {
    let whole = render(source, settings, view);
    let (width, height) = (view.width(), view.height());
    let tiles = [
        (width / 4 + 7, height / 4 + 3, width / 3, height / 3),
        (0, height - height / 3, width / 3, height / 3),
        (width - width / 4, 0, width / 4, height / 4),
        (width / 2, height / 2, width / 2, height / 2),
    ];
    tiles
        .iter()
        .map(|&(x, y, w, h)| {
            let tile = render(source, settings, view.tile(x, y, w, h));
            max_difference(&tile, &crop(&whole, x, y, w, h))
        })
        .max()
        .unwrap()
}

#[test]
fn r1_tiles_equal_the_same_region_of_the_whole_render() {
    let settings = everything();
    let (_, raster) = raster(600, 400);
    let camera = camera_scene(600, 400);
    for source in [&raster, &camera] {
        let view = full(source, &settings);
        assert!(worst_tile_difference(source, &settings, view) <= 1);
        let reduced = View::new(view.roi(), 300, 200);
        assert!(worst_tile_difference(source, &settings, reduced) <= 1);
    }
}

/// Average 2 x 2 blocks in linear light, as downsampling a picture does.
fn halve(image: &Rgba8Image) -> Vec<[f32; 3]> {
    let decode = |code: u8| RgbSpace::Srgb.decode(f32::from(code) / 255.0);
    let (width, height) = (image.width / 2, image.height / 2);
    let mut out = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                for (total, code) in sum.iter_mut().zip(rgb(image, 2 * x + dx, 2 * y + dy)) {
                    *total += decode(code) / 4.0;
                }
            }
            out.push(sum.map(|v| RgbSpace::Srgb.encode(v) * 255.0));
        }
    }
    out
}

#[test]
fn r2_a_half_scale_render_approximates_the_full_render_halved() {
    let settings = edit(|s| {
        s.tone.exposure = 0.3;
        s.tone.highlights = -60.0;
        s.tone.shadows = 50.0;
        s.presence.clarity = 40.0;
        s.presence.dehaze = 25.0;
        s.presence.texture = 40.0;
        s.detail.sharpening.amount = 50.0;
        s.detail.noise_reduction.luminance = 30.0;
        s.detail.noise_reduction.color = 30.0;
        s.effects.vignette.amount = -40.0;
    });
    let (_, source) = raster(640, 428);
    let view = full(&source, &settings);
    let whole = render(&source, &settings, view);
    let half = render(&source, &settings, View::new(view.roi(), 320, 214));
    let expected = halve(&whole);
    let mut errors: Vec<f32> = expected
        .iter()
        .enumerate()
        .flat_map(|(index, pixel)| {
            let got = rgb(&half, index as u32 % 320, index as u32 / 320);
            (0..3).map(move |c| (pixel[c] - f32::from(got[c])).abs())
        })
        .collect();
    errors.sort_by(f32::total_cmp);
    let mean = errors.iter().sum::<f32>() / errors.len() as f32;
    let p99 = errors[errors.len() * 99 / 100];
    println!("R2: mean {mean:.2} codes, 99th percentile {p99:.2}");
    assert!(mean < 1.5, "mean error {mean} codes");
    // The largest errors sit on edges, where any nonlinear stage maps the
    // average of two colours differently from the average of their two
    // mappings, and where colour noise reduction bleeds over a distance
    // that does not halve exactly with the scale. Measured per stage alone
    // the 99th percentile stays under 2 codes, except noise reduction
    // (about 5.5) on the edges of the colour chart.
    assert!(p99 < 12.0, "99th percentile {p99} codes");
}

#[test]
fn r3_a_tile_without_haze_dehazes_with_the_whole_images_light() {
    // The foreground alone is dark: a tile that estimated its own
    // atmospheric light would dehaze it as if it were all haze.
    let settings = edit(|s| s.presence.dehaze = 70.0);
    let (_, source) = raster(600, 400);
    let view = full(&source, &settings);
    let whole = render(&source, &settings, view);
    let tile = render(&source, &settings, view.tile(40, 300, 200, 90));
    assert!(max_difference(&tile, &crop(&whole, 40, 300, 200, 90)) <= 1);
}

#[test]
fn r3_auto_values_come_from_the_source_not_its_size_or_a_view() {
    let (_, small) = raster(640, 427);
    let (_, large) = raster(1600, 1067);
    let settings = DevelopSettings::default();
    let (a, b) = (auto_tone(&small, &settings), auto_tone(&large, &settings));
    assert!((a.exposure - b.exposure).abs() <= 0.05, "{a:?} vs {b:?}");
    for (x, y) in [
        (a.highlights, b.highlights),
        (a.shadows, b.shadows),
        (a.whites, b.whites),
        (a.blacks, b.blacks),
    ] {
        assert!((x - y).abs() <= 3.0, "{a:?} vs {b:?}");
    }
    let (a, b) = (auto_white_balance(&small), auto_white_balance(&large));
    assert!((a.temp - b.temp).abs() <= 1.0 && (a.tint - b.tint).abs() <= 1.0);
    // Rendering tiles changes nothing a later estimate sees.
    render(
        &small,
        &everything(),
        full(&small, &settings).tile(0, 0, 64, 64),
    );
    assert_eq!(auto_tone(&small, &settings), auto_tone(&small, &settings));
}

#[test]
fn r4_the_same_bytes_on_one_thread_and_on_many() {
    let settings = everything();
    let (_, source) = raster(500, 340);
    let camera = camera_scene(500, 340);
    let (single, many) = pools();
    for source in [&source, &camera] {
        let view = full(source, &settings);
        let tile = view.tile(100, 80, 230, 170);
        for view in [
            view,
            tile,
            View::new(Rect::new(10.0, 20.0, 400.0, 300.0), 200, 145),
        ] {
            let one = single.install(|| render(source, &settings, view));
            let seven = many.install(|| render(source, &settings, view));
            assert_eq!(one, seven);
            let one = single.install(|| render16(source, &settings, view));
            let seven = many.install(|| render16(source, &settings, view));
            assert_eq!(one, seven);
        }
    }
}
