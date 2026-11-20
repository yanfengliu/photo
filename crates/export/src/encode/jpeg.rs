//! JPEG through `image`'s baseline encoder, fed strip by strip.
//!
//! The encoder reads pixels through [`GenericImageView`], visiting 8-row
//! block rows top to bottom and each pixel once (its documented behaviour
//! for `encode_image`). [`LazyStrips`] answers from the one strip it holds
//! and renders the next when the encoder moves past it. EXIF and XMP go in
//! `APP1` segments right after the encoder's `JFIF` header, where readers
//! look for them; the encoder writes the ICC profile itself.

use std::cell::RefCell;
use std::io::{self, Write};

use image::codecs::jpeg::JpegEncoder;
use image::{GenericImageView, ImageEncoder, Rgb};

use super::{Encoding, Failure, to_eight_bits};
use crate::item::ExportError;
use crate::metadata::Metadata;
use crate::strips::StripSource;

/// Most bytes one JPEG marker segment can carry after its length field.
const SEGMENT_PAYLOAD: usize = 65_533;
const EXIF_SIGNATURE: &[u8] = b"Exif\0\0";
const XMP_SIGNATURE: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

pub(super) fn write(
    out: impl Write,
    pixels: &mut impl StripSource,
    quality: u8,
    icc: &[u8],
    metadata: &Metadata,
    failure: &Failure,
) -> Result<(), Encoding> {
    let (width, height) = pixels.size();
    let mut segments = Vec::new();
    if let Some(exif) = metadata.exif_block() {
        segments.extend(app1(EXIF_SIGNATURE, &exif, "EXIF")?);
    }
    if let Some(xmp) = &metadata.xmp {
        segments.extend(app1(XMP_SIGNATURE, xmp.as_bytes(), "XMP")?);
    }
    let mut writer = io::BufWriter::new(Inserter::new(out, segments));
    let mut encoder = JpegEncoder::new_with_quality(&mut writer, quality.clamp(1, 100));
    encoder
        .set_icc_profile(icc.to_vec())
        .map_err(image::ImageError::Unsupported)?;
    let strip_rows = pixels.strip_rows();
    let view = LazyStrips {
        size: (width, height),
        strip_rows,
        strip: RefCell::new(Strip::default()),
        pixels: RefCell::new(pixels),
        failure,
    };
    encoder.encode_image(&view)?;
    drop(encoder);
    writer.flush()?;
    let inserter = writer.into_inner().map_err(|error| error.into_error())?;
    inserter.finish()?;
    Ok(())
}

/// One `APP1` segment, marker and length included.
fn app1(signature: &[u8], payload: &[u8], what: &str) -> Result<Vec<u8>, Encoding> {
    let length = signature.len() + payload.len();
    if length > SEGMENT_PAYLOAD {
        return Err(Encoding::Export(ExportError::Unsupported(format!(
            "the {what} metadata is {length} bytes, more than one JPEG segment holds ({SEGMENT_PAYLOAD})"
        ))));
    }
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&((length + 2) as u16).to_be_bytes());
    segment.extend_from_slice(signature);
    segment.extend_from_slice(payload);
    Ok(segment)
}

/// The strip being encoded, as 8-bit RGB.
#[derive(Default)]
struct Strip {
    first: u32,
    rows: u32,
    pixels: Vec<u8>,
}

/// The picture as the encoder sees it, rendered one strip at a time.
struct LazyStrips<'a, S> {
    size: (u32, u32),
    strip_rows: u32,
    strip: RefCell<Strip>,
    pixels: RefCell<&'a mut S>,
    failure: &'a Failure,
}

impl<S: StripSource> LazyStrips<'_, S> {
    /// Makes the strip holding row `y` the current one. After a failure
    /// the strip is black: the encoder is stopped at its next write.
    fn load(&self, strip: &mut Strip, y: u32) {
        let first = y / self.strip_rows * self.strip_rows;
        let rows = self.strip_rows.min(self.size.1 - first);
        let samples = (self.size.0 * rows * 3) as usize;
        let rendered = if self.failure.is_set() {
            None
        } else {
            match self.pixels.borrow_mut().rows(first, rows) {
                Ok(pixels) => Some(pixels),
                Err(error) => {
                    self.failure.set(error);
                    None
                }
            }
        };
        strip.pixels = match rendered {
            Some(pixels) => pixels.into_iter().map(to_eight_bits).collect(),
            None => vec![0; samples],
        };
        strip.first = first;
        strip.rows = rows;
    }
}

impl<S: StripSource> GenericImageView for LazyStrips<'_, S> {
    type Pixel = Rgb<u8>;

    fn dimensions(&self) -> (u32, u32) {
        self.size
    }

    fn get_pixel(&self, x: u32, y: u32) -> Rgb<u8> {
        let mut strip = self.strip.borrow_mut();
        if y < strip.first || y >= strip.first + strip.rows {
            self.load(&mut strip, y);
        }
        let at = (((y - strip.first) * self.size.0 + x) * 3) as usize;
        Rgb([strip.pixels[at], strip.pixels[at + 1], strip.pixels[at + 2]])
    }
}

/// Passes a JPEG stream through, putting `segments` right after the start
/// of image and the `JFIF` header that follows it.
struct Inserter<W> {
    inner: W,
    head: Vec<u8>,
    segments: Option<Vec<u8>>,
}

impl<W: Write> Inserter<W> {
    fn new(inner: W, segments: Vec<u8>) -> Self {
        Self {
            inner,
            head: Vec::new(),
            segments: (!segments.is_empty()).then_some(segments),
        }
    }

    /// Where the segments go once `head` shows it: after `SOI` and a
    /// following `APP0`, or right after `SOI` when there is no `APP0`.
    fn insertion_point(&self) -> Option<usize> {
        let head = &self.head;
        if head.len() < 6 {
            return None;
        }
        if head[2..4] != [0xff, 0xe0] {
            return Some(2);
        }
        let end = 4 + usize::from(u16::from_be_bytes([head[4], head[5]]));
        (head.len() >= end).then_some(end)
    }

    /// Writes the segments when the stream ended before they found their
    /// place (it never does for a real JPEG).
    fn finish(mut self) -> io::Result<()> {
        if let Some(segments) = self.segments.take() {
            self.inner.write_all(&self.head)?;
            self.inner.write_all(&segments)?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Write for Inserter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(segments) = &self.segments else {
            return self.inner.write(buf);
        };
        self.head.extend_from_slice(buf);
        if self.head.len() >= 2 && self.head[..2] != [0xff, 0xd8] {
            return Err(io::Error::other("the encoder did not start a JPEG stream"));
        }
        if let Some(at) = self.insertion_point() {
            self.inner.write_all(&self.head[..at])?;
            self.inner.write_all(segments)?;
            self.inner.write_all(&self.head[at..])?;
            self.segments = None;
            self.head = Vec::new();
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn through(chunks: &[&[u8]], segments: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut inserter = Inserter::new(&mut out, segments.to_vec());
        for chunk in chunks {
            inserter.write_all(chunk).unwrap();
        }
        inserter.finish().unwrap();
        out
    }

    #[test]
    fn segments_go_after_the_jfif_header_however_the_stream_is_cut() {
        let stream: &[u8] = &[
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0xaa, 0xbb, 0xff, 0xdb, 0x01,
        ];
        let expected: Vec<u8> = [&stream[..8], b"SEG", &stream[8..]].concat();
        assert_eq!(through(&[stream], b"SEG"), expected);
        let bytes: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(through(&bytes, b"SEG"), expected);
        assert_eq!(through(&[stream], b""), stream);
    }

    #[test]
    fn without_jfif_the_segments_follow_the_start_of_image() {
        let stream: &[u8] = &[0xff, 0xd8, 0xff, 0xdb, 0x00, 0x43, 0x01];
        let expected: Vec<u8> = [&stream[..2], b"SEG", &stream[2..]].concat();
        assert_eq!(through(&[stream], b"SEG"), expected);
    }

    #[test]
    fn oversized_metadata_is_refused_not_cut() {
        assert!(app1(XMP_SIGNATURE, &vec![b'x'; 70_000], "XMP").is_err());
        let segment = app1(EXIF_SIGNATURE, b"abc", "EXIF").ok().unwrap();
        assert_eq!(segment[..4], [0xff, 0xe1, 0x00, 11]);
    }
}
