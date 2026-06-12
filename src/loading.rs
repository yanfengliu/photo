//! Async-load result types plus full-image and library-thumbnail base loading.

use crate::decode::{self, ImageData};
use crate::edit;
#[cfg(test)]
use crate::local_edits::run_test_local_edit_thumbnail_fast_path_hook;
use crate::local_edits::{
    load_persisted_local_edit, load_persisted_local_edit_variant,
    load_persisted_local_edit_variant_header, load_repaired_local_edit_thumbnail,
    persisted_thumbnail_matches_generation_and_dimensions, thumbnail_dimensions_for_image,
    LocalEditCacheVariant,
};
use crate::session_cache::{open_cache_validation_handle, SourceFileFingerprint};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseImageSource {
    Original,
    PersistedLocalEdit,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedFullImage {
    pub(crate) image: Arc<ImageData>,
    pub(crate) fingerprint: Option<SourceFileFingerprint>,
    pub(crate) base_source: BaseImageSource,
    pub(crate) logical_dimensions: (u32, u32),
}

/// A library-thumbnail base plus where it came from. Baked bases already contain
/// their committed edits, so handle construction must not re-apply session state
/// on top of them.
#[derive(Debug, Clone)]
pub(crate) struct LoadedThumbnailBase {
    pub(crate) image: Arc<ImageData>,
    pub(crate) base_source: BaseImageSource,
}

pub(crate) fn loaded_image_logical_dimensions(
    path: &Path,
    base_source: BaseImageSource,
    image: &ImageData,
) -> (u32, u32) {
    match base_source {
        BaseImageSource::Original => match decode::source_dimensions(path) {
            Ok(dimensions) => dimensions,
            Err(error) => {
                log::warn!(
                    "Failed to read source dimensions for {}: {}",
                    path.display(),
                    error
                );
                (image.width, image.height)
            }
        },
        BaseImageSource::PersistedLocalEdit => (image.width, image.height),
    }
}

pub(crate) fn display_dimensions_for_edit_state(
    base_dimensions: (u32, u32),
    rotation: edit::QuarterTurns,
    crop: Option<edit::CropRect>,
) -> (u32, u32) {
    let (display_w, display_h) =
        edit::rotated_dimensions(base_dimensions.0, base_dimensions.1, rotation);
    edit::cropped_dimensions(display_w, display_h, crop)
}

pub(crate) fn load_library_thumbnail_base_image(
    path: &Path,
    max_dim: u32,
) -> Result<LoadedThumbnailBase, String> {
    let thumbnail_header =
        match load_persisted_local_edit_variant_header(path, LocalEditCacheVariant::Thumbnail) {
            Ok(Some(header)) => Some(header),
            Ok(None) => None,
            Err(error) => {
                log::debug!(
                    "Ignoring persisted local edit thumbnail cache header for {}: {}",
                    path.display(),
                    error
                );
                None
            }
        };
    let full_header =
        match load_persisted_local_edit_variant_header(path, LocalEditCacheVariant::Full) {
            Ok(Some(header)) => Some(header),
            Ok(None) => None,
            Err(error) => {
                log::debug!(
                    "Ignoring persisted local edit full cache header for {}: {}",
                    path.display(),
                    error
                );
                None
            }
        };

    if let (Some(thumbnail_header), Some(full_header)) = (&thumbnail_header, &full_header) {
        let expected_dimensions =
            thumbnail_dimensions_for_image(full_header.width, full_header.height, max_dim);
        if thumbnail_header.generation_id == full_header.generation_id
            && thumbnail_header.width == expected_dimensions.0
            && thumbnail_header.height == expected_dimensions.1
        {
            #[cfg(test)]
            run_test_local_edit_thumbnail_fast_path_hook();

            let thumbnail_entry =
                match load_persisted_local_edit_variant(path, LocalEditCacheVariant::Thumbnail) {
                    Ok(Some(image)) => Some(image),
                    Ok(None) => None,
                    Err(error) => {
                        log::debug!(
                            "Ignoring persisted local edit thumbnail cache for {}: {}",
                            path.display(),
                            error
                        );
                        None
                    }
                };
            if let Some(thumbnail_entry) = thumbnail_entry {
                if persisted_thumbnail_matches_generation_and_dimensions(
                    &thumbnail_entry,
                    full_header.generation_id,
                    expected_dimensions,
                ) {
                    return Ok(LoadedThumbnailBase {
                        image: thumbnail_entry.image,
                        base_source: BaseImageSource::PersistedLocalEdit,
                    });
                }
            }
        }
    }

    if full_header.is_some() {
        if let Some(repaired_thumb) = load_repaired_local_edit_thumbnail(path, max_dim)? {
            return Ok(LoadedThumbnailBase {
                image: repaired_thumb,
                base_source: BaseImageSource::PersistedLocalEdit,
            });
        }
    }

    decode::decode_thumbnail(path, max_dim).map(|image| LoadedThumbnailBase {
        image,
        base_source: BaseImageSource::Original,
    })
}

pub(crate) fn load_full_image(
    path: &Path,
    preferred_source: BaseImageSource,
) -> Result<LoadedFullImage, String> {
    let mut guard = open_cache_validation_handle(path);
    let fingerprint = guard.as_mut().and_then(SourceFileFingerprint::from_file);
    let (image, base_source, logical_dimensions) = match preferred_source {
        BaseImageSource::PersistedLocalEdit => match load_persisted_local_edit(path) {
            Ok(Some(loaded)) => (
                loaded.image,
                BaseImageSource::PersistedLocalEdit,
                loaded.logical_dimensions,
            ),
            Ok(None) => {
                let image = decode::decode_image(path)?;
                let logical_dimensions =
                    loaded_image_logical_dimensions(path, BaseImageSource::Original, &image);
                (image, BaseImageSource::Original, logical_dimensions)
            }
            Err(error) => {
                log::debug!(
                    "Falling back to the original source for {} after persisted local edit load failed: {}",
                    path.display(),
                    error
                );
                let image = decode::decode_image(path)?;
                let logical_dimensions =
                    loaded_image_logical_dimensions(path, BaseImageSource::Original, &image);
                (image, BaseImageSource::Original, logical_dimensions)
            }
        },
        BaseImageSource::Original => {
            let image = decode::decode_image(path)?;
            let logical_dimensions =
                loaded_image_logical_dimensions(path, BaseImageSource::Original, &image);
            (image, BaseImageSource::Original, logical_dimensions)
        }
    };
    drop(guard);
    Ok(LoadedFullImage {
        image,
        fingerprint,
        base_source,
        logical_dimensions,
    })
}
