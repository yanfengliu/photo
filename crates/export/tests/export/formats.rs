//! Each format round-trips: the decoded file has the requested size and
//! depth, and the pixels of a direct render within the encoder's tolerance.

use image::DynamicImage;
use photo_core::colorimetry::RgbSpace;
use photo_develop::CancelToken;
use photo_export::{BitDepth, Format, export};

use crate::common::{Fixture, linear_source, open, written};

/// `value` rounded from 16 bits to 8, the way an export quantizes.
fn eight(value: u16) -> u8 {
    ((u32::from(value) * 255 + 32_767) / 65_535) as u8
}

#[test]
fn every_format_round_trips_with_its_size_depth_and_pixels() {
    let fixture = Fixture::new(150, 100);
    let cancel = CancelToken::new();
    for space in [RgbSpace::Srgb, RgbSpace::DisplayP3] {
        let expected = fixture.render((150, 100), space);
        for format in [
            Format::Jpeg { quality: 95 },
            Format::Png {
                depth: BitDepth::Eight,
            },
            Format::Png {
                depth: BitDepth::Sixteen,
            },
            Format::Tiff {
                depth: BitDepth::Eight,
            },
            Format::Tiff {
                depth: BitDepth::Sixteen,
            },
        ] {
            let mut settings = fixture.settings_out();
            settings.format = format;
            settings.color_space = space;
            let outcome = export(&fixture.item(), &settings, &cancel).unwrap();
            let (path, width, height) = written(&outcome);
            assert_eq!((width, height), (150, 100), "{format:?}");
            assert_eq!(path.extension().unwrap(), format.extension(), "{format:?}");
            let decoded = open(&path);
            assert_eq!(
                (decoded.width(), decoded.height()),
                (150, 100),
                "{format:?}"
            );
            match (format.bits(), decoded) {
                (16, DynamicImage::ImageRgb16(image)) => {
                    assert_eq!(image.as_raw(), &expected.pixels, "{format:?} {space:?}");
                }
                (8, DynamicImage::ImageRgb8(image)) => {
                    let want: Vec<u8> = expected.pixels.iter().map(|&v| eight(v)).collect();
                    if matches!(format, Format::Jpeg { .. }) {
                        let errors: Vec<u32> = image
                            .as_raw()
                            .iter()
                            .zip(&want)
                            .map(|(a, b)| u32::from(a.abs_diff(*b)))
                            .collect();
                        let mean = errors.iter().sum::<u32>() as f64 / errors.len() as f64;
                        let worst = errors.iter().max().copied().unwrap_or(0);
                        assert!(mean < 1.5, "{space:?}: JPEG mean error {mean}");
                        assert!(worst < 40, "{space:?}: JPEG worst error {worst}");
                    } else {
                        assert_eq!(image.as_raw(), &want, "{format:?} {space:?}");
                    }
                }
                (bits, other) => panic!(
                    "{format:?}: {bits}-bit export decoded as {:?}",
                    other.color()
                ),
            }
            std::fs::remove_file(&path).unwrap();
        }
    }
}

#[test]
fn jpeg_quality_trades_size_for_fidelity() {
    let fixture = Fixture::new(160, 120);
    let cancel = CancelToken::new();
    let expected = fixture.render((160, 120), RgbSpace::Srgb);
    let mut previous: Option<(u64, f64)> = None;
    for quality in [10u8, 60, 100] {
        let mut settings = fixture.settings_out();
        settings.format = Format::Jpeg { quality };
        settings.naming.template = format!("q{quality}");
        let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
        let size = std::fs::metadata(&path).unwrap().len();
        let decoded = open(&path).into_rgb8();
        let error = decoded
            .as_raw()
            .iter()
            .zip(&expected.pixels)
            .map(|(&a, &b)| f64::from(a.abs_diff(eight(b))))
            .sum::<f64>()
            / decoded.as_raw().len() as f64;
        if let Some((smaller, worse)) = previous {
            assert!(size > smaller, "quality {quality}: {size} bytes");
            assert!(error < worse, "quality {quality}: error {error}");
        }
        previous = Some((size, error));
    }
}

#[test]
fn an_item_without_a_source_decodes_the_original_itself() {
    let fixture = Fixture::new(90, 60);
    let mut item = fixture.item();
    item.source = None;
    let mut settings = fixture.settings_out();
    settings.format = Format::Png {
        depth: BitDepth::Sixteen,
    };
    let (path, _, _) = written(&export(&item, &settings, &CancelToken::new()).unwrap());
    let expected = fixture.render((90, 60), RgbSpace::Srgb);
    assert_eq!(open(&path).into_rgb16().as_raw(), &expected.pixels);
}

/// A picture tall enough to be rendered in several strips is, once written,
/// exactly the single render of the whole view (contract R1 as the export
/// uses it, with spatial stages that need padding across strip edges).
#[test]
fn a_picture_written_in_several_strips_is_the_single_render() {
    let (width, height) = (1024u32, 4400u32);
    let mut fixture = Fixture::new(8, 8);
    fixture.source = linear_source(width, height, |x, y| {
        let ripple = ((x as f32 * 0.37).sin() * (y as f32 * 0.21).cos() + 1.0) / 2.0;
        [
            ripple * 0.8,
            (y % 251) as f32 / 251.0,
            ((x / 13 + y / 17) % 2) as f32 * 0.6,
        ]
    });
    fixture.settings.presence.texture = 40.0;
    fixture.settings.detail.noise_reduction.luminance = 30.0;
    let size = (1024, 4400);
    let expected = fixture.render(size, RgbSpace::AdobeRgb);
    for format in [
        Format::Tiff {
            depth: BitDepth::Sixteen,
        },
        Format::Jpeg { quality: 98 },
    ] {
        let mut settings = fixture.settings_out();
        settings.format = format;
        settings.color_space = RgbSpace::AdobeRgb;
        let (path, _, _) =
            written(&export(&fixture.item(), &settings, &CancelToken::new()).unwrap());
        match open(&path) {
            DynamicImage::ImageRgb16(image) => assert_eq!(image.as_raw(), &expected.pixels),
            DynamicImage::ImageRgb8(image) => {
                let want: Vec<u8> = expected.pixels.iter().map(|&v| eight(v)).collect();
                let errors: Vec<u32> = image
                    .as_raw()
                    .iter()
                    .zip(&want)
                    .map(|(&a, &b)| u32::from(a.abs_diff(b)))
                    .collect();
                // Block-based coding error only: a misplaced strip would be
                // off by far more across the strip's whole width.
                let worst = errors.iter().max().unwrap();
                assert!(*worst < 48, "worst JPEG error {worst}");
                let row_mean = |y: u32| {
                    let row = (y * width * 3) as usize..((y + 1) * width * 3) as usize;
                    errors[row].iter().sum::<u32>() / width
                };
                // Rows either side of where a strip ends are as close as any.
                for y in [2047u32, 2048, 4095, 4096] {
                    assert!(row_mean(y) < 4, "row {y}: mean error {}", row_mean(y));
                }
            }
            other => panic!("{format:?} decoded as {:?}", other.color()),
        }
    }
}
