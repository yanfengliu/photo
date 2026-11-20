//! The encoders driven by a synthetic picture of many small strips: each
//! strip is asked for once, in order, and failures and cancellation stop
//! the encoder with the right error.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageReader};
use photo_core::colorimetry::RgbSpace;
use photo_develop::CancelToken;

use super::{to_eight_bits, write};
use crate::icc;
use crate::item::ExportError;
use crate::metadata::Metadata;
use crate::settings::{BitDepth, Format};
use crate::strips::StripSource;

/// A smooth colour field, served in strips, remembering what was asked.
struct Synthetic {
    size: (u32, u32),
    strip_rows: u32,
    asked: Vec<(u32, u32)>,
    fail_at: Option<u32>,
    cancel_at: Option<(u32, CancelToken)>,
}

impl Synthetic {
    fn new(width: u32, height: u32) -> Self {
        Self {
            size: (width, height),
            strip_rows: 8,
            asked: Vec::new(),
            fail_at: None,
            cancel_at: None,
        }
    }

    fn pixel(&self, x: u32, y: u32) -> [u16; 3] {
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let (u, v) = (x as f32 / w, y as f32 / h);
        [u, v, 0.5 + 0.4 * (6.0 * u).sin() * (5.0 * v).cos()].map(|c| (c * 65535.0) as u16)
    }

    fn picture(&self) -> Vec<u16> {
        (0..self.size.1)
            .flat_map(|y| (0..self.size.0).flat_map(move |x| self.pixel(x, y)))
            .collect()
    }
}

impl StripSource for Synthetic {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn strip_rows(&self) -> u32 {
        self.strip_rows
    }

    fn rows(&mut self, first: u32, count: u32) -> Result<Vec<u16>, ExportError> {
        self.asked.push((first, count));
        if self.fail_at == Some(first) {
            return Err(ExportError::Unsupported("synthetic failure".into()));
        }
        if let Some((at, cancel)) = &self.cancel_at
            && *at == first
        {
            cancel.cancel();
            return Err(ExportError::Cancelled);
        }
        let this = &*self;
        Ok((first..first + count)
            .flat_map(|y| (0..this.size.0).flat_map(move |x| this.pixel(x, y)))
            .collect())
    }
}

const FORMATS: [Format; 5] = [
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
];

fn encode(
    format: Format,
    pixels: &mut Synthetic,
    cancel: &CancelToken,
) -> Result<Vec<u8>, ExportError> {
    let mut out = Vec::new();
    let profile = icc::profile(RgbSpace::Srgb);
    write(
        format,
        &mut out,
        Path::new("synthetic"),
        pixels,
        &profile,
        &Metadata::default(),
        cancel,
    )?;
    Ok(out)
}

#[test]
fn every_strip_is_asked_for_once_in_order_and_lands_in_place() {
    for format in FORMATS {
        let mut pixels = Synthetic::new(37, 61);
        let bytes = encode(format, &mut pixels, &CancelToken::new()).unwrap();
        let expected: Vec<(u32, u32)> = (0..61).step_by(8).map(|y| (y, 8.min(61 - y))).collect();
        assert_eq!(pixels.asked, expected, "{format:?}");

        let decoded = ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!((decoded.width(), decoded.height()), (37, 61), "{format:?}");
        let picture = pixels.picture();
        match decoded {
            DynamicImage::ImageRgb16(image) => assert_eq!(image.as_raw(), &picture, "{format:?}"),
            DynamicImage::ImageRgb8(image) => {
                let want: Vec<u8> = picture.iter().map(|&v| to_eight_bits(v)).collect();
                if matches!(format, Format::Jpeg { .. }) {
                    let worst = image
                        .as_raw()
                        .iter()
                        .zip(&want)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max();
                    assert!(worst.unwrap() <= 12, "JPEG worst error {worst:?}");
                } else {
                    assert_eq!(image.as_raw(), &want, "{format:?}");
                }
            }
            other => panic!("{format:?} decoded as {:?}", other.color()),
        }
    }
}

#[test]
fn a_strip_that_fails_stops_the_encoder_with_its_error() {
    for format in FORMATS {
        let mut pixels = Synthetic::new(40, 64);
        pixels.fail_at = Some(24);
        let result = encode(format, &mut pixels, &CancelToken::new());
        assert!(
            matches!(&result, Err(ExportError::Unsupported(message)) if message == "synthetic failure"),
            "{format:?}: {result:?}"
        );
        assert_eq!(
            pixels.asked.last(),
            Some(&(24, 8)),
            "{format:?}: nothing asked after the failure"
        );
    }
}

#[test]
fn cancelling_between_strips_stops_the_encoder() {
    for format in FORMATS {
        let cancel = CancelToken::new();
        let mut pixels = Synthetic::new(40, 64);
        pixels.cancel_at = Some((16, cancel.clone()));
        let result = encode(format, &mut pixels, &cancel);
        assert!(
            matches!(result, Err(ExportError::Cancelled)),
            "{format:?}: {result:?}"
        );
        assert_eq!(pixels.asked.last(), Some(&(16, 8)), "{format:?}");
    }
}
