//! Raster files (JPEG, PNG, TIFF, WebP, BMP, GIF) through the `image` crate.

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use image::codecs::tiff::TiffDecoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use photo_core::colorimetry::RgbSpace;
use photo_core::image::{Rgba8Image, SourceColor, SourceImage};
use photo_core::meta::PhotoMetadata;

use crate::error::DecodeError;
use crate::exif::{self, Container, ExifData};
use crate::linear::to_linear_rgb;
use crate::orient::Orientation;
use crate::preview::to_preview;
use crate::{Decoded, MAX_PIXELS, icc};

/// Most memory a codec may claim for one image: the largest picture we
/// accept at its widest pixel format, 16-bit RGBA.
const MAX_CODEC_BYTES: u64 = MAX_PIXELS * 8;

/// A raster file with its header parsed and nothing decoded yet.
struct Opened {
    decoder: Box<dyn ImageDecoder>,
    format: ImageFormat,
    /// The embedded ICC profile, when the file has one.
    profile: Option<Vec<u8>>,
}

impl Opened {
    fn open(path: &Path) -> Result<Self, DecodeError> {
        let mut reader = ImageReader::open(path)?
            .with_guessed_format()
            .map_err(DecodeError::from_read)?;
        // The content decides; the extension only breaks ties.
        let format = reader
            .format()
            .ok_or_else(|| DecodeError::Unsupported("unrecognised raster format".into()))?;
        let mut limits = Limits::no_limits();
        limits.max_alloc = Some(MAX_CODEC_BYTES);

        let (decoder, profile): (Box<dyn ImageDecoder>, _) = match format {
            ImageFormat::Tiff => {
                // Opened directly so the profile can be read before the
                // limits go on: with limits applied this decoder caps tag
                // values at the size of the pixel data, which for a small
                // image is less than its profile.
                let mut decoder = TiffDecoder::new(reader.into_inner())?;
                let profile = decoder.icc_profile().ok().flatten();
                decoder.set_limits(limits)?;
                (Box::new(decoder), profile)
            }
            ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Gif => {
                reader.limits(limits);
                let mut decoder = reader.into_decoder()?;
                let profile = decoder.icc_profile().ok().flatten();
                (Box::new(decoder), profile)
            }
            other => {
                let name = format!("{other:?}").to_ascii_uppercase();
                return Err(DecodeError::Unsupported(format!(
                    "{name} files are not read"
                )));
            }
        };
        Ok(Self {
            decoder,
            format,
            profile,
        })
    }

    /// The stored pixel grid size, refusing empty and oversized images.
    fn decodable_size(&self) -> Result<(u32, u32), DecodeError> {
        let (width, height) = self.decoder.dimensions();
        let pixels = u64::from(width) * u64::from(height);
        if pixels == 0 {
            return Err(DecodeError::Corrupt("the image has no pixels".into()));
        }
        if pixels > MAX_PIXELS {
            return Err(DecodeError::TooLarge(format!(
                "{width} x {height} pixels is over the {} megapixel limit",
                MAX_PIXELS / 1_000_000
            )));
        }
        Ok((width, height))
    }

    fn exif(&self, path: &Path) -> Option<ExifData> {
        let container = match self.format {
            ImageFormat::Tiff => Container::Tiff,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP => Container::Chunked,
            // The remaining formats cannot carry EXIF.
            _ => return None,
        };
        exif::read(path, container)
    }

    /// The colour space the stored values are encoded in: the one the
    /// embedded profile's primaries match, sRGB otherwise.
    fn space(&self) -> RgbSpace {
        self.profile
            .as_deref()
            .and_then(icc::detect_space)
            .unwrap_or(RgbSpace::Srgb)
    }

    fn into_pixels(self) -> Result<DynamicImage, DecodeError> {
        Ok(DynamicImage::from_decoder(self.decoder)?)
    }
}

fn metadata(width: u32, height: u32, exif: Option<&ExifData>) -> PhotoMetadata {
    let mut metadata = PhotoMetadata {
        width,
        height,
        ..PhotoMetadata::default()
    };
    if let Some(exif) = exif {
        exif.fill(&mut metadata);
    }
    metadata
}

fn orientation(exif: Option<&ExifData>) -> Orientation {
    exif.map_or(Orientation::UPRIGHT, ExifData::orientation)
}

pub(crate) fn read_metadata(path: &Path) -> Result<PhotoMetadata, DecodeError> {
    let opened = Opened::open(path)?;
    let exif = opened.exif(path);
    let (stored_width, stored_height) = opened.decoder.dimensions();
    let (width, height) =
        orientation(exif.as_ref()).upright_size(stored_width as usize, stored_height as usize);
    Ok(metadata(width as u32, height as u32, exif.as_ref()))
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, DecodeError> {
    let opened = Opened::open(path)?;
    let (stored_width, stored_height) = opened.decodable_size()?;
    if opened.format == ImageFormat::Jpeg && !jpeg_is_complete(path)? {
        // The JPEG codec is lenient and would hand back a picture that is
        // grey from the break onwards.
        return Err(DecodeError::Corrupt(
            "the JPEG data stops before its end-of-image marker".into(),
        ));
    }
    let exif = opened.exif(path);
    let orientation = orientation(exif.as_ref());
    let space = opened.space();

    let pixels = to_linear_rgb(&opened.into_pixels()?, orientation, space)?;
    let (width, height) = orientation.upright_size(stored_width as usize, stored_height as usize);
    let (width, height) = (width as u32, height as u32);
    let image = SourceImage::new(width, height, pixels, SourceColor::Rgb(space))
        .ok_or_else(|| DecodeError::Corrupt("the decoded pixels do not fill the image".into()))?;
    Ok(Decoded {
        image,
        metadata: metadata(width, height, exif.as_ref()),
    })
}

pub(crate) fn preview(path: &Path, max_edge: u32) -> Result<Rgba8Image, DecodeError> {
    let opened = Opened::open(path)?;
    opened.decodable_size()?;
    let orientation = orientation(opened.exif(path).as_ref());
    let space = opened.space();
    Ok(to_preview(
        opened.into_pixels()?,
        orientation,
        space,
        max_edge,
    ))
}

/// Whether the JPEG at `path` runs through to the end-of-image marker of
/// its main image. Walks the marker structure without decoding anything;
/// data after that marker (extra previews, vendor trailers) is ignored.
fn jpeg_is_complete(path: &Path) -> Result<bool, DecodeError> {
    let mut reader = BufReader::new(File::open(path)?);
    match walk_jpeg(&mut reader) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(DecodeError::Io(error)),
    }
}

/// Reads markers until end-of-image; running out of data first is the
/// `UnexpectedEof` error.
fn walk_jpeg(reader: &mut impl BufRead) -> io::Result<()> {
    const START_OF_IMAGE: u8 = 0xd8;
    const END_OF_IMAGE: u8 = 0xd9;
    const TEMPORARY: u8 = 0x01;

    loop {
        match next_marker(reader)? {
            END_OF_IMAGE => return Ok(()),
            // These stand alone, without a segment.
            START_OF_IMAGE | TEMPORARY => {}
            // Every other marker opens a segment that states its length.
            // After a scan header comes entropy-coded data, which the next
            // call to `next_marker` steps over.
            _ => {
                let mut length = [0u8; 2];
                reader.read_exact(&mut length)?;
                skip(
                    reader,
                    u64::from(u16::from_be_bytes(length)).saturating_sub(2),
                )?;
            }
        }
    }
}

/// Skips to the next marker and returns its code. Inside entropy-coded data
/// a 0xFF byte is always followed by 0x00 (a stuffed data byte) or a restart
/// marker, so those pairs are stepped over; anything else is a real marker.
fn next_marker(reader: &mut impl BufRead) -> io::Result<u8> {
    loop {
        skip_until_ff(reader)?;
        // Any number of 0xFF fill bytes may precede the code.
        let mut code = [0xffu8];
        while code[0] == 0xff {
            reader.read_exact(&mut code)?;
        }
        if code[0] != 0x00 && !(0xd0..=0xd7).contains(&code[0]) {
            return Ok(code[0]);
        }
    }
}

/// Consumes input up to and including the next 0xFF byte.
fn skip_until_ff(reader: &mut impl BufRead) -> io::Result<()> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        match buffer.iter().position(|&byte| byte == 0xff) {
            Some(index) => {
                reader.consume(index + 1);
                return Ok(());
            }
            None => {
                let length = buffer.len();
                reader.consume(length);
            }
        }
    }
}

fn skip(reader: &mut impl BufRead, mut count: u64) -> io::Result<()> {
    while count > 0 {
        let available = reader.fill_buf()?.len();
        if available == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let step = available.min(usize::try_from(count).unwrap_or(usize::MAX));
        reader.consume(step);
        count -= step as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(bytes: &[u8]) -> bool {
        match walk_jpeg(&mut &bytes[..]) {
            Ok(()) => true,
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
                false
            }
        }
    }

    /// SOI, one APP0 segment, a scan header, entropy data, EOI.
    fn tiny_jpeg(entropy: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0xff, 0xd9];
        bytes.extend_from_slice(&[0xff, 0xda, 0x00, 0x03, 0x00]);
        bytes.extend_from_slice(entropy);
        bytes.extend_from_slice(&[0xff, 0xd9]);
        bytes
    }

    #[test]
    fn a_whole_stream_is_complete_and_any_prefix_of_it_is_not() {
        let jpeg = tiny_jpeg(&[0x12, 0x34, 0xff, 0x00, 0x56, 0xff, 0xd3, 0x78]);
        assert!(complete(&jpeg));
        for length in 0..jpeg.len() {
            assert!(!complete(&jpeg[..length]), "prefix of {length} bytes");
        }
    }

    #[test]
    fn marker_bytes_inside_a_segment_do_not_count() {
        // The APP0 payload above is FF D9: it must be skipped, not taken as
        // the end of the image. Cutting the real end off shows it was.
        let jpeg = tiny_jpeg(&[0x01, 0x02]);
        assert!(complete(&jpeg));
        assert!(!complete(&jpeg[..jpeg.len() - 2]));
    }

    #[test]
    fn stuffed_bytes_restart_markers_and_fill_bytes_are_not_the_end() {
        assert!(!complete(
            &tiny_jpeg(&[0xff, 0x00, 0xff, 0xd0, 0xff, 0xd7])[..18]
        ));
        // Fill bytes before the final marker are legal.
        let mut padded = tiny_jpeg(&[0x42]);
        let end = padded.len() - 2;
        padded.splice(end..end, [0xff, 0xff, 0xff]);
        assert!(complete(&padded));
    }

    #[test]
    fn trailing_data_after_the_end_is_ignored() {
        let mut jpeg = tiny_jpeg(&[0x42]);
        jpeg.extend_from_slice(b"vendor trailer \xff\xd8 another image, cut short");
        assert!(complete(&jpeg));
    }

    #[test]
    fn several_scans_are_followed_to_the_last() {
        // Progressive files have more than one scan, with tables between.
        let mut jpeg = vec![0xff, 0xd8];
        for _ in 0..3 {
            jpeg.extend_from_slice(&[0xff, 0xc4, 0x00, 0x03, 0x00]);
            jpeg.extend_from_slice(&[0xff, 0xda, 0x00, 0x03, 0x00, 0x11, 0x22, 0xff, 0x00]);
        }
        assert!(!complete(&jpeg));
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        assert!(complete(&jpeg));
    }
}
