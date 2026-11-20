//! PNG through `image`'s encoder, which takes the whole picture at once:
//! the strips are assembled at the output depth first. The encoder writes
//! the ICC profile (`iCCP`) and EXIF (`eXIf`); XMP goes in an `iTXt` chunk
//! with the keyword `XML:com.adobe.xmp` (XMP Part 3, §1.1.5), inserted right
//! after the header chunk.

use std::io::{self, Write};

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};

use super::{Encoding, strips, to_eight_bits};
use crate::metadata::Metadata;
use crate::settings::BitDepth;
use crate::strips::StripSource;

/// The PNG signature and the `IHDR` chunk, which always come first.
const HEADER_LEN: usize = 8 + 4 + 4 + 13 + 4;

pub(super) fn write(
    out: impl Write,
    pixels: &mut impl StripSource,
    depth: BitDepth,
    icc: &[u8],
    metadata: &Metadata,
) -> Result<(), Encoding> {
    let (width, height) = pixels.size();
    let bytes_per_sample = usize::from(depth.bits() / 8);
    let mut buffer = Vec::with_capacity(width as usize * height as usize * 3 * bytes_per_sample);
    for (first, count) in strips(pixels) {
        let rows = pixels.rows(first, count)?;
        match depth {
            BitDepth::Eight => buffer.extend(rows.into_iter().map(to_eight_bits)),
            // The encoder takes 16-bit samples in native byte order.
            BitDepth::Sixteen => buffer.extend(rows.into_iter().flat_map(u16::to_ne_bytes)),
        }
    }
    let chunk = metadata.xmp.as_ref().map(|xmp| {
        chunk(
            b"iTXt",
            &[b"XML:com.adobe.xmp\0\0\0\0\0".as_slice(), xmp.as_bytes()].concat(),
        )
    });
    let mut writer = io::BufWriter::new(Inserter::new(out, chunk.unwrap_or_default()));
    let mut encoder =
        PngEncoder::new_with_quality(&mut writer, CompressionType::Default, FilterType::Adaptive);
    encoder
        .set_icc_profile(icc.to_vec())
        .map_err(image::ImageError::Unsupported)?;
    if let Some(exif) = metadata.exif_block() {
        encoder
            .set_exif_metadata(exif)
            .map_err(image::ImageError::Unsupported)?;
    }
    let color = match depth {
        BitDepth::Eight => ExtendedColorType::Rgb8,
        BitDepth::Sixteen => ExtendedColorType::Rgb16,
    };
    encoder.write_image(&buffer, width, height, color)?;
    writer.flush()?;
    let inserter = writer.into_inner().map_err(|error| error.into_error())?;
    inserter.finish()?;
    Ok(())
}

/// A PNG chunk: length, type, data and CRC.
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
    chunk
}

/// The CRC-32 of ISO 3309 that PNG chunks carry (PNG §5.5).
fn crc32(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 == 1 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };
    !bytes.iter().fold(u32::MAX, |crc, &byte| {
        TABLE[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
    })
}

/// Passes a PNG stream through, putting `chunk` right after `IHDR`.
struct Inserter<W> {
    inner: W,
    head: Vec<u8>,
    chunk: Option<Vec<u8>>,
}

impl<W: Write> Inserter<W> {
    fn new(inner: W, chunk: Vec<u8>) -> Self {
        Self {
            inner,
            head: Vec::new(),
            chunk: (!chunk.is_empty()).then_some(chunk),
        }
    }

    /// Writes the chunk when the stream ended before it found its place
    /// (it never does for a real PNG).
    fn finish(mut self) -> io::Result<()> {
        if let Some(chunk) = self.chunk.take() {
            self.inner.write_all(&self.head)?;
            self.inner.write_all(&chunk)?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Write for Inserter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(chunk) = &self.chunk else {
            return self.inner.write(buf);
        };
        self.head.extend_from_slice(buf);
        if self.head.len() >= HEADER_LEN {
            if &self.head[12..16] != b"IHDR" {
                return Err(io::Error::other("the encoder did not start with IHDR"));
            }
            self.inner.write_all(&self.head[..HEADER_LEN])?;
            self.inner.write_all(chunk)?;
            self.inner.write_all(&self.head[HEADER_LEN..])?;
            self.chunk = None;
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

    #[test]
    fn the_crc_is_the_one_png_uses() {
        // The IEND chunk's CRC, as found at the end of every PNG file.
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn the_chunk_goes_right_after_the_header_chunk() {
        let mut stream = b"\x89PNG\r\n\x1a\n".to_vec();
        stream.extend(chunk(b"IHDR", &[0; 13]));
        stream.extend(chunk(b"IDAT", b"data"));
        let extra = chunk(b"iTXt", b"text");
        let mut out = Vec::new();
        let mut inserter = Inserter::new(&mut out, extra.clone());
        for byte in &stream {
            inserter.write_all(std::slice::from_ref(byte)).unwrap();
        }
        inserter.finish().unwrap();
        let expected = [&stream[..HEADER_LEN], &extra[..], &stream[HEADER_LEN..]].concat();
        assert_eq!(out, expected);
    }
}
