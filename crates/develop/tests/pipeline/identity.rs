//! T1 and the two output formats.

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::DevelopSettings;
use photo_core::meta::FileKind;
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer};
use photo_testkit::grey_ramp;

use crate::fixtures::{camera_scene, camera_source, edit, full, raster, render, render16, rgb};

#[test]
fn t1_default_settings_reproduce_a_raster_file_code_for_code() {
    let (file, source) = raster(480, 320);
    for settings in [
        DevelopSettings::default(),
        DevelopSettings::default_for(FileKind::Raster),
    ] {
        let image = render(&source, &settings, full(&source, &settings));
        assert_eq!((image.width, image.height), file.dimensions());
        let mut off_by_one = 0;
        for (x, y, pixel) in file.enumerate_pixels() {
            let got = rgb(&image, x, y);
            for (a, b) in got.iter().zip(pixel.0) {
                let difference = a.abs_diff(b);
                assert!(difference <= 1, "({x}, {y}): {got:?} vs {:?}", pixel.0);
                off_by_one += usize::from(difference == 1);
            }
            assert_eq!(image.pixel(x, y)[3], 255, "opaque");
        }
        // Rounding at a code boundary may move a value by one, rarely.
        assert!(off_by_one * 1000 < file.as_raw().len(), "{off_by_one}");
    }
}

#[test]
fn t1_a_camera_source_renders_finite_with_neutral_greys_neutral() {
    let (width, height) = (480, 320);
    let source = camera_scene(width, height);
    let (x0, y0, ramp_width, ramp_height) = grey_ramp(width, height);
    // The raw defaults reduce colour noise, which (as wavelet colour noise
    // reduction does) lets the yellowish ground around the 25-pixel ramp
    // strip bleed into it by a few percent of its colour, under one 8-bit
    // code; the 16-bit check of neutrality is made on a neutral frame below.
    for (settings, margin, strict) in [
        (DevelopSettings::default(), 2, true),
        (DevelopSettings::default_for(FileKind::Raw), 8, false),
    ] {
        let view = full(&source, &settings);
        let image = render(&source, &settings, view);
        let words = render16(&source, &settings, view);
        let row = y0 + ramp_height / 2;
        let mut previous = 0u8;
        for x in x0 + margin..x0 + ramp_width - margin {
            let [r, g, b] = rgb(&image, x, row);
            let (high, low) = (r.max(g).max(b), r.min(g).min(b));
            assert!(high - low <= 1, "grey ramp at {x}: {:?}", [r, g, b]);
            if strict {
                assert_neutral(words.pixel(x, row), x);
            }
            assert!(g + 1 >= previous, "the ramp falls at {x}");
            previous = g;
        }
        // A NaN anywhere would have come out black; the sky never is.
        let [r, g, b] = rgb(&image, width / 4, height / 5);
        assert!(r > 20 && g > 20 && b > 20, "{:?}", [r, g, b]);
    }

    // A frame that is neutral throughout stays neutral with every raw
    // default on.
    let card = camera_source(width, height, 0.25, |x, _| {
        let level = (x as f32 + 0.5) / width as f32;
        [level * level; 3]
    });
    let settings = DevelopSettings::default_for(FileKind::Raw);
    let words = render16(&card, &settings, full(&card, &settings));
    for x in 0..width {
        assert_neutral(words.pixel(x, height / 2), x);
    }
}

/// In the deepest shadows the raw values are a few dozen codes, and their
/// quantization, amplified by the camera matrix, is more than any 16-bit
/// tolerance; from 3% of white up a grey must stay within 1%.
fn assert_neutral([r, g, b]: [u16; 3], x: u32) {
    if g >= 2000 {
        let spread = r.max(g).max(b) - r.min(g).min(b);
        assert!(
            spread <= 64 + g / 100,
            "16-bit grey at {x}: {:?}",
            [r, g, b]
        );
    }
}

#[test]
fn the_16_bit_and_8_bit_renders_agree_to_8_bit_precision() {
    let (_, source) = raster(360, 240);
    let settings = edit(|s| {
        s.tone.exposure = 0.4;
        s.tone.contrast = 30.0;
        s.tone.highlights = -40.0;
        s.tone.shadows = 30.0;
        s.presence.vibrance = 25.0;
        s.presence.clarity = 30.0;
        s.detail.sharpening.amount = 50.0;
        s.effects.vignette.amount = -30.0;
    });
    for space in [RgbSpace::Srgb, RgbSpace::DisplayP3, RgbSpace::AdobeRgb] {
        let request = RenderRequest {
            source: &source,
            settings: &settings,
            view: full(&source, &settings),
            quality: Quality::Full,
            space,
        };
        let cancel = CancelToken::new();
        let bytes = Renderer::new().render_rgba8(&request, &cancel).unwrap();
        let words = Renderer::new().render_rgb16(&request, &cancel).unwrap();
        assert_eq!((words.width, words.height), (bytes.width, bytes.height));
        assert_eq!((words.space, bytes.space), (space, space));
        for y in 0..bytes.height {
            for x in 0..bytes.width {
                let eight = rgb(&bytes, x, y);
                let sixteen = words.pixel(x, y);
                for (a, b) in eight.iter().zip(sixteen) {
                    let widened = (f32::from(b) / 257.0).round() as u8;
                    assert!(a.abs_diff(widened) <= 1, "{space:?} at ({x}, {y})");
                }
            }
        }
    }
}
