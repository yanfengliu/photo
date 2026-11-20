//! Reading and writing one sidecar, as a worker does it, and how what went
//! wrong is told to the user.

use std::path::Path;

use photo_core::meta::PhotoId;
use photo_xmp::{ImageInfo, XmpData, XmpError};

use crate::import::FileIssue;

/// What a worker did with one sidecar.
pub(super) struct Outcome {
    pub(super) found: Option<(PhotoId, XmpData)>,
    /// Something to tell the user about this photo, whether or not the
    /// rest of it worked.
    pub(super) issue: Option<FileIssue>,
    pub(super) failed: bool,
}

/// An issue about the sidecar at `path`.
pub(super) fn issue(path: &Path, message: impl Into<String>) -> Option<FileIssue> {
    Some(FileIssue {
        path: path.to_path_buf(),
        message: message.into(),
    })
}

/// Write one sidecar. Develop settings that would replace settings this
/// build cannot read are left out, and the rest is written.
pub(super) fn write_one(
    path: &Path,
    data: &XmpData,
    info: ImageInfo,
    note: Option<FileIssue>,
) -> Outcome {
    let failed = |error: XmpError| Outcome {
        found: None,
        issue: issue(path, describe(&error)),
        failed: true,
    };
    match photo_xmp::write_sidecar(path, data, info) {
        Ok(()) => Outcome {
            found: None,
            issue: note,
            failed: false,
        },
        Err(XmpError::UnreadableSettings { .. }) => {
            let without = XmpData {
                develop: None,
                ..data.clone()
            };
            match photo_xmp::write_sidecar(path, &without, info) {
                Ok(()) => Outcome {
                    found: None,
                    issue: issue(
                        path,
                        "the develop settings were not written: the sidecar holds settings this version cannot read",
                    ),
                    failed: false,
                },
                Err(error) => failed(error),
            }
        }
        Err(error) => failed(error),
    }
}

/// Read one sidecar.
pub(super) fn read_one(path: &Path, photo: PhotoId, info: ImageInfo) -> Outcome {
    match photo_xmp::read_sidecar(path, info) {
        Ok(Some(data)) => {
            let note = data.unreadable_settings_format.and_then(|format| {
                issue(
                    path,
                    format!("the develop settings (format {format}) cannot be read by this version and were not applied"),
                )
            });
            Outcome {
                found: Some((photo, data)),
                issue: note,
                failed: false,
            }
        }
        Ok(None) => Outcome {
            found: None,
            issue: None,
            failed: false,
        },
        Err(error) => Outcome {
            found: None,
            issue: issue(path, describe(&error)),
            failed: true,
        },
    }
}

fn describe(error: &XmpError) -> String {
    match error {
        XmpError::Conflict { .. } => {
            "the sidecar kept changing while it was being updated; it was left as the other application saved it".to_string()
        }
        other => other.to_string(),
    }
}
