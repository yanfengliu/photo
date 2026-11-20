//! A baseline TIFF writer (TIFF 6.0, Part 1): uncompressed, chunky RGB at 8
//! or 16 bits, little-endian.
//!
//! Uncompressed strips have sizes known in advance, so the directories are
//! written first, followed by the pixel data in one run, streamed strip by
//! strip as it is rendered. The first directory carries the image tags, the
//! ICC profile (tag 34675), the XMP packet (tag 700) and the EXIF tags, with
//! the EXIF and GPS directories it points to written just before it.

use std::io::Write;

use super::{Encoding, strips, to_eight_bits};
use crate::ifd::{Ifd, TiffStream, Value};
use crate::item::ExportError;
use crate::metadata::Metadata;
use crate::settings::BitDepth;
use crate::strips::StripSource;

/// Bytes per TIFF strip, about: readers handle any size, and this keeps the
/// strip tables short without making a reader hold much at once.
const STRIP_BYTES: u64 = 256 * 1024;

mod tag {
    pub(super) const IMAGE_WIDTH: u16 = 256;
    pub(super) const IMAGE_LENGTH: u16 = 257;
    pub(super) const BITS_PER_SAMPLE: u16 = 258;
    pub(super) const COMPRESSION: u16 = 259;
    pub(super) const PHOTOMETRIC: u16 = 262;
    pub(super) const STRIP_OFFSETS: u16 = 273;
    pub(super) const ORIENTATION: u16 = 274;
    pub(super) const SAMPLES_PER_PIXEL: u16 = 277;
    pub(super) const ROWS_PER_STRIP: u16 = 278;
    pub(super) const STRIP_BYTE_COUNTS: u16 = 279;
    pub(super) const X_RESOLUTION: u16 = 282;
    pub(super) const Y_RESOLUTION: u16 = 283;
    pub(super) const PLANAR_CONFIGURATION: u16 = 284;
    pub(super) const RESOLUTION_UNIT: u16 = 296;
    pub(super) const XMP: u16 = 700;
    pub(super) const ICC_PROFILE: u16 = 34675;
}

pub(super) fn write(
    mut out: impl Write,
    pixels: &mut impl StripSource,
    depth: BitDepth,
    icc: &[u8],
    metadata: &Metadata,
) -> Result<(), Encoding> {
    let (width, height) = pixels.size();
    let bits = u16::from(depth.bits());
    let row_bytes = u64::from(width) * 3 * u64::from(bits / 8);
    let rows_per_strip = (STRIP_BYTES / row_bytes.max(1)).clamp(1, u64::from(height.max(1))) as u32;
    let strip_count = height.div_ceil(rows_per_strip);
    let strip_bytes = |index: u32| {
        let rows = rows_per_strip.min(height - index * rows_per_strip);
        u64::from(rows) * row_bytes
    };

    let mut primary = Ifd::new();
    let mut stream = TiffStream::new();
    metadata.add_to(&mut primary, &mut stream);
    primary.set(tag::IMAGE_WIDTH, Value::Long(vec![width]));
    primary.set(tag::IMAGE_LENGTH, Value::Long(vec![height]));
    primary.set(tag::BITS_PER_SAMPLE, Value::Short(vec![bits; 3]));
    primary.set(tag::COMPRESSION, Value::Short(vec![1]));
    primary.set(tag::PHOTOMETRIC, Value::Short(vec![2]));
    primary.set(tag::ORIENTATION, Value::Short(vec![1]));
    primary.set(tag::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
    primary.set(tag::ROWS_PER_STRIP, Value::Long(vec![rows_per_strip]));
    primary.set(tag::X_RESOLUTION, Value::Rational(vec![(72, 1)]));
    primary.set(tag::Y_RESOLUTION, Value::Rational(vec![(72, 1)]));
    primary.set(tag::PLANAR_CONFIGURATION, Value::Short(vec![1]));
    primary.set(tag::RESOLUTION_UNIT, Value::Short(vec![2]));
    primary.set(tag::ICC_PROFILE, Value::Undefined(icc.to_vec()));
    if let Some(xmp) = &metadata.xmp {
        primary.set(tag::XMP, Value::Byte(xmp.as_bytes().to_vec()));
    }
    let counts: Vec<u32> = (0..strip_count)
        .map(|index| strip_bytes(index) as u32)
        .collect();
    primary.set(tag::STRIP_BYTE_COUNTS, Value::Long(counts));

    // The directory's size does not depend on the offsets' values: lay it
    // out once with placeholders to learn where the pixels start.
    primary.set(
        tag::STRIP_OFFSETS,
        Value::Long(vec![0; strip_count as usize]),
    );
    let mut trial = stream.clone();
    trial.directory(&primary);
    let start = trial.len().next_multiple_of(2) as u64;
    let total = start + row_bytes * u64::from(height);
    if total > u64::from(u32::MAX) {
        return Err(Encoding::Export(ExportError::Unsupported(format!(
            "a {width} x {height} TIFF at {bits} bits would be larger than the 4 GiB a TIFF can address"
        ))));
    }
    let offsets: Vec<u32> = (0..strip_count)
        .map(|index| (start + u64::from(index * rows_per_strip) * row_bytes) as u32)
        .collect();
    primary.set(tag::STRIP_OFFSETS, Value::Long(offsets));
    let first = stream.directory(&primary);
    let mut head = stream.finish(first);
    head.resize(start as usize, 0);
    out.write_all(&head)?;

    let mut bytes = Vec::new();
    for (first, count) in strips(pixels) {
        let rows = pixels.rows(first, count)?;
        bytes.clear();
        match depth {
            BitDepth::Eight => bytes.extend(rows.into_iter().map(to_eight_bits)),
            BitDepth::Sixteen => bytes.extend(rows.into_iter().flat_map(u16::to_le_bytes)),
        }
        out.write_all(&bytes)?;
    }
    out.flush()?;
    Ok(())
}
