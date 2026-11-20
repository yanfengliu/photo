//! Sizing: every mode gives the right dimensions on the rotated, cropped
//! output; impossible sizes are refused; a reduction is clean.

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{CropRect, Orientation};
use photo_develop::{CancelToken, output_size};
use photo_export::{BitDepth, ExportError, Format, Resize, Sizing, export};

use crate::common::{Fixture, linear_source, names, open, written};

fn size_of(fixture: &Fixture, resize: Resize, dont_enlarge: bool) -> (u32, u32) {
    let mut settings = fixture.settings_out();
    settings.format = Format::Png {
        depth: BitDepth::Eight,
    };
    settings.sizing = Sizing {
        resize,
        dont_enlarge,
    };
    let (path, width, height) =
        written(&export(&fixture.item(), &settings, &CancelToken::new()).unwrap());
    let decoded = open(&path);
    assert_eq!(
        (decoded.width(), decoded.height()),
        (width, height),
        "{resize:?}"
    );
    std::fs::remove_file(path).unwrap();
    (width, height)
}

#[test]
fn every_mode_sizes_the_upright_output() {
    let fixture = Fixture::new(300, 200);
    let cases = [
        (Resize::Original, (300, 200)),
        (Resize::LongEdge { pixels: 150 }, (150, 100)),
        (Resize::ShortEdge { pixels: 50 }, (75, 50)),
        (
            Resize::Fit {
                width: 100,
                height: 100,
            },
            (100, 67),
        ),
        (
            Resize::Fit {
                width: 600,
                height: 40,
            },
            (60, 40),
        ),
        (Resize::Megapixels { megapixels: 0.015 }, (150, 100)),
        (Resize::Percent { percent: 25.0 }, (75, 50)),
        (Resize::Percent { percent: 33.3 }, (100, 67)),
    ];
    for (resize, expected) in cases {
        assert_eq!(size_of(&fixture, resize, true), expected, "{resize:?}");
    }
}

#[test]
fn dont_enlarge_caps_at_full_resolution_and_otherwise_enlarges() {
    let fixture = Fixture::new(120, 80);
    for resize in [
        Resize::LongEdge { pixels: 480 },
        Resize::ShortEdge { pixels: 160 },
        Resize::Fit {
            width: 1000,
            height: 1000,
        },
        Resize::Megapixels { megapixels: 1.0 },
        Resize::Percent { percent: 400.0 },
    ] {
        assert_eq!(size_of(&fixture, resize, true), (120, 80), "{resize:?}");
    }
    assert_eq!(
        size_of(&fixture, Resize::LongEdge { pixels: 480 }, false),
        (480, 320)
    );
    assert_eq!(
        size_of(&fixture, Resize::Percent { percent: 150.0 }, false),
        (180, 120)
    );
}

#[test]
fn quarter_turns_and_crops_change_what_the_long_edge_is() {
    let mut fixture = Fixture::new(300, 200);
    fixture.settings.orientation = Orientation::new(1, false);
    assert_eq!(output_size(&fixture.source, &fixture.settings), (200, 300));
    assert_eq!(
        size_of(&fixture, Resize::LongEdge { pixels: 150 }, true),
        (100, 150)
    );
    assert_eq!(size_of(&fixture, Resize::Original, true), (200, 300));

    // A crop that is taller than wide out of a landscape frame.
    fixture.settings.orientation = Orientation::IDENTITY;
    fixture.settings.crop.rect = Some(CropRect {
        cx: 0.5,
        cy: 0.5,
        width: 0.5,
        height: 0.8,
    });
    assert_eq!(output_size(&fixture.source, &fixture.settings), (150, 160));
    assert_eq!(
        size_of(&fixture, Resize::LongEdge { pixels: 80 }, true),
        (75, 80)
    );
    assert_eq!(
        size_of(
            &fixture,
            Resize::Fit {
                width: 30,
                height: 100
            },
            true
        ),
        (30, 32)
    );

    // The crop is normalized to the turned frame: 0.5 x 200 by 0.8 x 300.
    fixture.settings.orientation = Orientation::new(3, true);
    assert_eq!(output_size(&fixture.source, &fixture.settings), (100, 240));
    assert_eq!(
        size_of(&fixture, Resize::ShortEdge { pixels: 50 }, true),
        (50, 120)
    );

    // A straightened crop: the size is the pipeline's, never recomputed.
    fixture.settings.crop.angle = 7.5;
    fixture.settings.crop.rect = Some(CropRect::inscribed(7.5, 200.0 / 300.0, 0.7));
    let output = output_size(&fixture.source, &fixture.settings);
    assert_eq!(size_of(&fixture, Resize::Original, true), output);
}

#[test]
fn sizes_that_cannot_be_written_are_refused_before_anything_is_written() {
    let fixture = Fixture::new(70, 2);
    for (resize, format) in [
        (Resize::LongEdge { pixels: 0 }, Format::default()),
        (
            Resize::Fit {
                width: 10,
                height: 0,
            },
            Format::default(),
        ),
        (Resize::Percent { percent: f32::NAN }, Format::default()),
        (Resize::Megapixels { megapixels: -1.0 }, Format::default()),
        // 70 000 pixels wide is more than a JPEG can hold.
        (
            Resize::Percent { percent: 100_000.0 },
            Format::Jpeg { quality: 90 },
        ),
        // Seven hundred megapixels is more than any export may have.
        (
            Resize::Percent {
                percent: 1_000_000.0,
            },
            Format::Png {
                depth: BitDepth::Eight,
            },
        ),
    ] {
        let mut settings = fixture.settings_out();
        settings.format = format;
        settings.sizing = Sizing {
            resize,
            dont_enlarge: false,
        };
        let result = export(&fixture.item(), &settings, &CancelToken::new());
        assert!(
            matches!(result, Err(ExportError::Unsupported(_))),
            "{resize:?}: {result:?}"
        );
    }
    assert!(!fixture.out_dir().exists() || names(&fixture.out_dir()).is_empty());
}

/// Rendering straight at the target size is the antialiasing reduction:
/// on a zone plate, rings finer than the export can show come out as flat
/// grey, while the coarser rings keep their contrast.
#[test]
fn a_reduced_export_is_antialiased_and_keeps_its_detail() {
    let (width, height) = (900u32, 600u32);
    let zone = |x: u32, y: u32| {
        let (dx, dy) = (x as f32 + 0.5 - 450.0, y as f32 + 0.5 - 300.0);
        let phase = (dx * dx + dy * dy) * std::f32::consts::PI / (2.0 * width as f32);
        [0.4 + 0.4 * phase.cos(); 3]
    };
    let mut fixture = Fixture::new(8, 8);
    fixture.source = linear_source(width, height, zone);
    fixture.settings = Default::default();
    let mut settings = fixture.settings_out();
    settings.format = Format::Png {
        depth: BitDepth::Sixteen,
    };
    settings.sizing.resize = Resize::Percent { percent: 30.0 };
    let (path, w, h) = written(&export(&fixture.item(), &settings, &CancelToken::new()).unwrap());
    let image = open(&path).into_rgb16();
    let (mut pass, mut stop) = (Vec::new(), Vec::new());
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (
                x as f32 + 0.5 - w as f32 / 2.0,
                y as f32 + 0.5 - h as f32 / 2.0,
            );
            let radius = (dx * dx + dy * dy).sqrt() / 0.3;
            // Local period of the rings in exported pixels.
            let period = 0.3 * 2.0 * width as f32 / radius.max(1e-3);
            let value = RgbSpace::Srgb.decode(f32::from(image.get_pixel(x, y)[1]) / 65535.0);
            if period >= 4.0 && radius > 20.0 {
                pass.push(value);
            } else if period <= 1.5 {
                stop.push(value);
            }
        }
    }
    let spread = |values: &[f32]| {
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
    };
    // A sinusoid of amplitude 0.4 has a spread of 0.28.
    assert!(spread(&pass) > 0.25, "passband spread {}", spread(&pass));
    assert!(
        spread(&stop) < 0.06,
        "aliasing: stopband spread {}",
        spread(&stop)
    );
}
