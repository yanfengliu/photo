//! The one error type of this crate, and how the codecs' own errors map
//! onto it.

use std::any::Any;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::ImageError;
use rawler::RawlerError;

/// Why a file could not be read.
///
/// Every failure of this crate is one of these; malformed input never
/// panics.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The file is not of a type this crate reads, or it uses a feature
    /// (a sensor layout, a camera model, a sample format) that cannot be
    /// decoded. The file itself may be perfectly fine.
    #[error("unsupported image: {0}")]
    Unsupported(String),
    /// The file could not be opened or read from disk.
    #[error("cannot read image file: {0}")]
    Io(#[from] io::Error),
    /// The file was read, but its contents are damaged, truncated or not
    /// what its extension claims.
    #[error("corrupt image: {0}")]
    Corrupt(String),
    /// The image is larger than this crate is willing to hold in memory.
    #[error("image too large: {0}")]
    TooLarge(String),
}

impl DecodeError {
    /// An I/O failure met while decoding. Running out of file is damage to
    /// the image, not a problem with the disk, so it is reported as such.
    pub(crate) fn from_read(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Corrupt("the file ends before the image data does".into())
        } else {
            Self::Io(error)
        }
    }
}

impl From<ImageError> for DecodeError {
    fn from(error: ImageError) -> Self {
        match error {
            ImageError::IoError(error) => Self::from_read(error),
            ImageError::Unsupported(error) => Self::Unsupported(error.to_string()),
            ImageError::Limits(error) => Self::TooLarge(error.to_string()),
            other => Self::Corrupt(other.to_string()),
        }
    }
}

impl From<RawlerError> for DecodeError {
    fn from(error: RawlerError) -> Self {
        match error {
            // No decoder recognised the container at all: with a raw
            // extension on the file, that is damage rather than a camera we
            // have not heard of.
            RawlerError::Unsupported { make, model, .. } if make.is_empty() && model.is_empty() => {
                Self::Corrupt("not a recognisable camera raw file".into())
            }
            unsupported @ RawlerError::Unsupported { .. } => {
                Self::Unsupported(unsupported.to_string())
            }
            RawlerError::DecoderFailed(message) => Self::Corrupt(message),
        }
    }
}

/// Runs `work`, turning a panic inside it into [`DecodeError::Corrupt`].
///
/// The codecs this crate drives are third-party code fed with untrusted
/// files, and some of them panic on input they did not expect. A damaged
/// photo must never take the application down, so every public entry point
/// goes through here. (This relies on unwinding; it cannot help in a build
/// with `panic = "abort"`, and it cannot catch memory exhaustion.)
pub(crate) fn guard<T>(work: impl FnOnce() -> Result<T, DecodeError>) -> Result<T, DecodeError> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(result) => result,
        Err(payload) => {
            let message = panic_message(payload.as_ref());
            log::error!("image decoder panicked: {message}");
            Err(DecodeError::Corrupt(format!(
                "the decoder gave up on this file: {message}"
            )))
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "no further detail".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_a_corrupt_error_carrying_the_message() {
        let result: Result<(), _> = guard(|| panic!("index {} out of range", 7));
        match result {
            Err(DecodeError::Corrupt(message)) => assert!(message.contains("index 7 out of range")),
            other => panic!("unexpected {other:?}"),
        }
        let result: Result<(), _> = guard(|| std::panic::panic_any(42u8));
        assert!(matches!(result, Err(DecodeError::Corrupt(_))));
    }

    #[test]
    fn results_pass_through_the_guard_untouched() {
        assert_eq!(guard(|| Ok(5)).unwrap(), 5);
        let error = guard(|| Err::<(), _>(DecodeError::TooLarge("x".into()))).unwrap_err();
        assert!(matches!(error, DecodeError::TooLarge(_)));
    }

    #[test]
    fn running_out_of_file_is_corruption_but_other_io_errors_stay_io() {
        let eof = io::Error::new(io::ErrorKind::UnexpectedEof, "short");
        assert!(matches!(
            DecodeError::from_read(eof),
            DecodeError::Corrupt(_)
        ));
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "no");
        assert!(matches!(DecodeError::from_read(denied), DecodeError::Io(_)));
    }

    #[test]
    fn rawler_errors_keep_their_meaning() {
        let unknown_container = RawlerError::Unsupported {
            what: "No decoder found".into(),
            make: String::new(),
            model: String::new(),
            mode: String::new(),
        };
        assert!(matches!(
            DecodeError::from(unknown_container),
            DecodeError::Corrupt(_)
        ));
        let unknown_camera = RawlerError::Unsupported {
            what: "Unknown camera".into(),
            make: "ACME".into(),
            model: "X1".into(),
            mode: String::new(),
        };
        match DecodeError::from(unknown_camera) {
            DecodeError::Unsupported(message) => assert!(message.contains("ACME")),
            other => panic!("unexpected {other:?}"),
        }
        let failed = RawlerError::DecoderFailed("strip past the end".into());
        assert!(matches!(DecodeError::from(failed), DecodeError::Corrupt(_)));
    }
}
