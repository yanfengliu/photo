//! Preview files on disk.
//!
//! Layout: `<previews>/<bucket>/<photo id>/<size>-<develop version>.jpg`,
//! where the bucket is the low byte of the id in hex, so no directory grows
//! past a few hundred entries. Each photo has a directory of its own, which
//! is all that writing a newer version has to look at to delete the older
//! ones.
//!
//! Each file is an ordinary baseline JPEG that also carries, in a comment
//! segment right after its header, what it was made from: the develop
//! version, where its pixels came from, the size and modification time of
//! the source file as they were when the pixels were read, and for a
//! standard preview its histogram. Files are written to a temporary name
//! and renamed into place, so a reader never sees half a file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageFormat};
use photo_core::colorimetry::RgbSpace;
use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoId;
use photo_imaging::Histogram;

use super::{PreviewOrigin, PreviewSize};
use crate::stamp::Stamp;

const MAGIC: &[u8; 8] = b"PHOTOPRV";
const FORMAT: u8 = 1;
const START_OF_IMAGE: [u8; 2] = [0xff, 0xd8];
const APP0: u8 = 0xe0;
const COMMENT: u8 = 0xfe;
const START_OF_SCAN: u8 = 0xda;

/// What a preview file holds besides its pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Header {
    pub(crate) version: u64,
    pub(crate) size: PreviewSize,
    pub(crate) origin: PreviewOrigin,
    pub(crate) source: Stamp,
    pub(crate) histogram: Option<Histogram>,
}

/// The file for one photo, size and develop version.
pub(crate) fn file_path(root: &Path, photo: PhotoId, size: PreviewSize, version: u64) -> PathBuf {
    photo_dir(root, photo).join(format!("{}-{version}.jpg", size.file_tag()))
}

fn photo_dir(root: &Path, photo: PhotoId) -> PathBuf {
    let bucket = format!("{:02x}", photo.0.rem_euclid(256));
    root.join(bucket).join(photo.0.to_string())
}

/// Read a preview file. `None` when there is no file, or it is not one this
/// code wrote (damaged, or from another format version): either way it is
/// rendered again and replaced.
pub(crate) fn read(path: &Path) -> Option<(Header, Rgba8Image)> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("cannot read preview {}: {error}", path.display());
            return None;
        }
    };
    let header = find_header(&bytes).and_then(decode_header)?;
    let decoded = match image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg) {
        Ok(decoded) => decoded.into_rgba8(),
        Err(error) => {
            log::warn!("damaged preview {}: {error}", path.display());
            return None;
        }
    };
    let image = Rgba8Image {
        width: decoded.width(),
        height: decoded.height(),
        pixels: decoded.into_raw(),
        space: RgbSpace::Srgb,
    };
    Some((header, image))
}

/// Write a preview file in one step, creating its directory. Pixels are
/// taken as sRGB; alpha is dropped (previews are opaque).
pub(crate) fn write(path: &Path, header: &Header, image: &Rgba8Image) -> io::Result<()> {
    let rgb: Vec<u8> = image
        .pixels
        .chunks_exact(Rgba8Image::CHANNELS)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, header.size.jpeg_quality())
        .encode(&rgb, image.width, image.height, ExtendedColorType::Rgb8)
        .map_err(io::Error::other)?;
    let bytes = with_header(&jpeg, &encode_header(header))?;
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("a preview path has no directory"))?;
    fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(".{}-{}.tmp", std::process::id(), next_temporary()));
    let written = fs::write(&temporary, &bytes).and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn next_temporary() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// The develop versions of this photo's files of `size`, with their paths.
/// Only the photo's own directory is listed.
fn versions(root: &Path, photo: PhotoId, size: PreviewSize) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = fs::read_dir(photo_dir(root, photo)) else {
        return Vec::new();
    };
    let prefix = format!("{}-", size.file_tag());
    entries
        .flatten()
        .filter_map(|entry| {
            let version = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix(&prefix))
                .and_then(|rest| rest.strip_suffix(".jpg"))
                .and_then(|number| number.parse::<u64>().ok())?;
            Some((version, entry.path()))
        })
        .collect()
}

/// Whether a file of `size` for a develop version newer than `version` is
/// already there, which makes one for `version` stale before it is written.
pub(crate) fn newer_exists(root: &Path, photo: PhotoId, size: PreviewSize, version: u64) -> bool {
    versions(root, photo, size)
        .iter()
        .any(|(found, _)| *found > version)
}

/// Delete this photo's files of `size` for develop versions older than
/// `version`. Only the photo's own directory is listed.
pub(crate) fn remove_older(root: &Path, photo: PhotoId, size: PreviewSize, version: u64) {
    for (found, path) in versions(root, photo, size) {
        if found < version
            && let Err(error) = fs::remove_file(&path)
        {
            log::warn!("cannot delete stale preview {}: {error}", path.display());
        }
    }
}

fn encode_header(header: &Header) -> Vec<u8> {
    let mut out = Vec::with_capacity(48 + 4 * 256 * 4);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT);
    out.push(header.size.code());
    out.push(header.origin.code());
    out.push(u8::from(header.histogram.is_some()));
    out.extend_from_slice(&header.version.to_le_bytes());
    out.extend_from_slice(&header.source.bytes.to_le_bytes());
    out.extend_from_slice(&header.source.modified_secs.to_le_bytes());
    out.extend_from_slice(&header.source.modified_nanos.to_le_bytes());
    if let Some(histogram) = &header.histogram {
        for bins in [
            &histogram.red,
            &histogram.green,
            &histogram.blue,
            &histogram.luma,
        ] {
            bins.iter()
                .for_each(|count| out.extend_from_slice(&count.to_le_bytes()));
        }
        out.extend_from_slice(&histogram.clipped_shadows.to_le_bytes());
        out.extend_from_slice(&histogram.clipped_highlights.to_le_bytes());
    }
    out
}

/// Reads fixed-size little-endian fields off the front of a byte slice.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    fn bins(&mut self) -> Option<[u32; 256]> {
        let mut bins = [0u32; 256];
        for bin in &mut bins {
            *bin = u32::from_le_bytes(self.take()?);
        }
        Some(bins)
    }
}

fn decode_header(payload: &[u8]) -> Option<Header> {
    let mut fields = Fields(payload);
    if fields.take::<8>()? != *MAGIC || fields.byte()? != FORMAT {
        return None;
    }
    let size = PreviewSize::from_code(fields.byte()?)?;
    let origin = PreviewOrigin::from_code(fields.byte()?)?;
    let has_histogram = fields.byte()? == 1;
    let version = u64::from_le_bytes(fields.take()?);
    let source = Stamp {
        bytes: u64::from_le_bytes(fields.take()?),
        modified_secs: i64::from_le_bytes(fields.take()?),
        modified_nanos: u32::from_le_bytes(fields.take()?),
    };
    let histogram = if has_histogram {
        Some(Histogram {
            red: fields.bins()?,
            green: fields.bins()?,
            blue: fields.bins()?,
            luma: fields.bins()?,
            clipped_shadows: f32::from_le_bytes(fields.take()?),
            clipped_highlights: f32::from_le_bytes(fields.take()?),
        })
    } else {
        None
    };
    Some(Header {
        version,
        size,
        origin,
        source,
        histogram,
    })
}

/// `jpeg` with a comment segment holding `payload` inserted after its
/// start-of-image marker and, if present, its JFIF header.
fn with_header(jpeg: &[u8], payload: &[u8]) -> io::Result<Vec<u8>> {
    if jpeg.get(..2) != Some(&START_OF_IMAGE[..]) {
        return Err(io::Error::other("the encoder did not produce a JPEG"));
    }
    let mut insert_at = 2;
    if jpeg.get(2..4) == Some(&[0xff, APP0][..])
        && let Some(length) = jpeg.get(4..6)
    {
        insert_at += 2 + usize::from(u16::from_be_bytes([length[0], length[1]]));
    }
    let (head, tail) = jpeg.split_at(insert_at.min(jpeg.len()));
    let length = u16::try_from(payload.len() + 2)
        .map_err(|_| io::Error::other("preview header too large for one segment"))?;
    let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
    out.extend_from_slice(head);
    out.extend_from_slice(&[0xff, COMMENT]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(tail);
    Ok(out)
}

/// The payload of the first comment segment before the image data that
/// starts with our magic.
fn find_header(jpeg: &[u8]) -> Option<&[u8]> {
    if jpeg.get(..2)? != START_OF_IMAGE {
        return None;
    }
    let mut at = 2;
    loop {
        let marker = jpeg.get(at..at + 2)?;
        if marker[0] != 0xff || marker[1] == START_OF_SCAN {
            return None;
        }
        let length = usize::from(u16::from_be_bytes(
            jpeg.get(at + 2..at + 4)?.try_into().ok()?,
        ));
        let payload = jpeg.get(at + 4..at + 2 + length)?;
        if marker[1] == COMMENT && payload.starts_with(MAGIC) {
            return Some(payload);
        }
        at += 2 + length;
    }
}

#[cfg(test)]
mod tests;
