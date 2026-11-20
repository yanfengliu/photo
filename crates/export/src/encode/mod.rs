//! Encoding the rendered strips into a file of the chosen format, with the
//! ICC profile and the metadata.
//!
//! | Format | Encoder | Strips | ICC | EXIF | XMP |
//! | --- | --- | --- | --- | --- | --- |
//! | JPEG | `image`'s baseline encoder, fed block row by block row | streamed | `APP2` (encoder) | `APP1` (inserted) | `APP1` (inserted) |
//! | PNG | `image`'s encoder, given the whole picture | assembled | `iCCP` (encoder) | `eXIf` (encoder) | `iTXt` (inserted) |
//! | TIFF | this crate's baseline writer | streamed | tag 34675 | IFD0 and sub-directories | tag 700 |
//!
//! `image`'s JPEG encoder reads its input through a pixel accessor one
//! 8-row block row at a time, so the strips are rendered as it reaches them
//! and only one strip is ever held. Its PNG encoder takes one buffer, so the
//! picture is assembled at the output depth before encoding. Its TIFF
//! encoder also takes one buffer and can write the ICC profile but neither
//! EXIF nor XMP, so TIFF is written here: uncompressed RGB strips after the
//! directories, streamed as they are rendered.

mod jpeg;
mod png;
mod tiff;

use std::cell::RefCell;
use std::io::{self, Write};
use std::path::Path;
use std::rc::Rc;

use photo_develop::CancelToken;

use crate::item::ExportError;
use crate::metadata::Metadata;
use crate::settings::Format;
use crate::strips::StripSource;

/// Writes the picture `pixels` produces to `out` as `format`. `path` names
/// the file in error messages.
pub(crate) fn write(
    format: Format,
    out: impl Write,
    path: &Path,
    pixels: &mut impl StripSource,
    icc: &[u8],
    metadata: &Metadata,
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    let failure = Failure::default();
    let guarded = Guarded {
        inner: out,
        cancel,
        failure: failure.clone(),
    };
    let result = match format {
        Format::Jpeg { quality } => jpeg::write(guarded, pixels, quality, icc, metadata, &failure),
        Format::Png { depth } => png::write(guarded, pixels, depth, icc, metadata),
        Format::Tiff { depth } => tiff::write(guarded, pixels, depth, icc, metadata),
    };
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    if let Some(error) = failure.take() {
        return Err(error);
    }
    result.map_err(|error| match error {
        Encoding::Io(error) => ExportError::io(path, error),
        Encoding::Export(error) => error,
        Encoding::Image(image::ImageError::IoError(error)) => ExportError::io(path, error),
        Encoding::Image(error) => ExportError::Unsupported(error.to_string()),
    })
}

/// Why an encoder stopped.
pub(crate) enum Encoding {
    /// Writing failed.
    Io(io::Error),
    /// Producing the pixels failed.
    Export(ExportError),
    /// The `image` encoder refused.
    Image(image::ImageError),
}

impl From<io::Error> for Encoding {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ExportError> for Encoding {
    fn from(error: ExportError) -> Self {
        Self::Export(error)
    }
}

impl From<image::ImageError> for Encoding {
    fn from(error: image::ImageError) -> Self {
        Self::Image(error)
    }
}

/// The first failure met while producing pixels inside an encoder that
/// cannot report it: kept here and returned once the encoder has stopped.
#[derive(Clone, Default)]
pub(crate) struct Failure(Rc<RefCell<Option<ExportError>>>);

impl Failure {
    /// Keeps `error` unless an earlier one is already kept.
    pub(crate) fn set(&self, error: ExportError) {
        self.0.borrow_mut().get_or_insert(error);
    }

    /// True once a failure has been kept.
    pub(crate) fn is_set(&self) -> bool {
        self.0.borrow().is_some()
    }

    fn take(&self) -> Option<ExportError> {
        self.0.borrow_mut().take()
    }
}

/// A writer that refuses to go on once the export is cancelled or a
/// failure has been kept, which stops an encoder at its next write.
struct Guarded<'a, W> {
    inner: W,
    cancel: &'a CancelToken,
    failure: Failure,
}

impl<W: Write> Write for Guarded<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() || self.failure.is_set() {
            return Err(io::Error::other("the export was stopped"));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// `value` rounded from 16 bits to 8.
#[inline]
pub(crate) fn to_eight_bits(value: u16) -> u8 {
    ((u32::from(value) * 255 + 32_767) / 65_535) as u8
}

/// The first row and row count of each strip of `pixels`, top to bottom.
pub(crate) fn strips(pixels: &impl StripSource) -> Vec<(u32, u32)> {
    let (_, height) = pixels.size();
    let step = pixels.strip_rows().max(1);
    (0..height)
        .step_by(step as usize)
        .map(|first| (first, step.min(height - first)))
        .collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rounding {
    use super::*;

    #[test]
    fn eight_bit_values_are_the_nearest() {
        for code in 0..=255u8 {
            assert_eq!(to_eight_bits(u16::from(code) * 257), code);
        }
        assert_eq!(to_eight_bits(128), 0);
        assert_eq!(to_eight_bits(129), 1);
        assert_eq!(to_eight_bits(65_535), 255);
    }
}
