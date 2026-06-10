//! Baked local-edit persistence: repo-local cache files for committed edits.

use crate::decode::{self, ImageData};
use crate::edit;
use crate::loading::BaseImageSource;
use crate::repo::photo_repo_root;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const LOCAL_EDIT_CACHE_DIR_NAME: &str = "local-edits";
pub(crate) const LOCAL_EDIT_CACHE_MAGIC: &[u8; 8] = b"PHOEDITS";
pub(crate) const LOCAL_EDIT_CACHE_SCHEMA_VERSION: u32 = 3;
// Magic + schema + generation + source metadata + path/dimension metadata before the variable path/pixels.
pub(crate) const LOCAL_EDIT_CACHE_SCHEMA_V2_FIXED_HEADER_BYTES: u64 = LOCAL_EDIT_CACHE_MAGIC.len()
    as u64
    + (std::mem::size_of::<u64>() as u64 * 4)
    + (std::mem::size_of::<u32>() as u64 * 5);
pub(crate) const LOCAL_EDIT_CACHE_SCHEMA_V3_FIXED_HEADER_BYTES: u64 =
    LOCAL_EDIT_CACHE_SCHEMA_V2_FIXED_HEADER_BYTES + (std::mem::size_of::<u32>() as u64 * 2);
pub(crate) const LOCAL_EDIT_THUMBNAIL_MAX_DIM: u32 = 200;
pub(crate) static NEXT_LOCAL_EDIT_CACHE_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);
pub(crate) static NEXT_LOCAL_EDIT_CACHE_GENERATION_NONCE: AtomicU64 = AtomicU64::new(0);
// Serializes paired full/thumbnail cache mutations so readers never observe a mixed generation.
pub(crate) static LOCAL_EDIT_CACHE_IO_GUARD: std::sync::OnceLock<Mutex<()>> =
    std::sync::OnceLock::new();
#[cfg(test)]
pub(crate) type TestHookCell = std::sync::OnceLock<Mutex<Option<Box<dyn FnOnce() + Send>>>>;
#[cfg(test)]
pub(crate) static TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_HOOK: TestHookCell = TestHookCell::new();
#[cfg(test)]
pub(crate) static TEST_LOCAL_EDIT_THUMBNAIL_FAST_PATH_HOOK: TestHookCell = TestHookCell::new();
#[cfg(test)]
pub(crate) static TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_WRITE_ERROR: std::sync::OnceLock<
    Mutex<Option<String>>,
> = std::sync::OnceLock::new();
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalEditCacheVariant {
    Full,
    Thumbnail,
}

impl LocalEditCacheVariant {
    pub(crate) fn file_suffix(self) -> &'static str {
        match self {
            Self::Full => ".full.rgba",
            Self::Thumbnail => ".thumb.rgba",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LocalEditPersistRequest {
    pub(crate) request_id: u64,
    pub(crate) path: PathBuf,
    pub(crate) image: Arc<ImageData>,
    pub(crate) logical_dimensions: (u32, u32),
    pub(crate) state: edit::EditState,
    pub(crate) lens: edit::LensCorrection,
    pub(crate) base_source: BaseImageSource,
}

pub(crate) struct LoadedLocalEditCacheVariant {
    pub(crate) generation_id: u64,
    pub(crate) logical_dimensions: (u32, u32),
    pub(crate) image: Arc<ImageData>,
}

pub(crate) struct LoadedLocalEditCacheVariantHeader {
    pub(crate) generation_id: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) struct LoadedPersistedLocalEdit {
    pub(crate) image: Arc<ImageData>,
    pub(crate) logical_dimensions: (u32, u32),
}

pub(crate) struct ValidatedLocalEditCacheHeader {
    generation_id: u64,
    width: u32,
    height: u32,
    logical_dimensions: Option<(u32, u32)>,
    source_file_size: u64,
}

pub(crate) enum LocalEditThumbnailRepairDecision {
    Missing,
    Return(Arc<ImageData>),
    Derive { generation_id: u64 },
}

pub(crate) enum FinalizeLocalEditThumbnailRepair {
    Return(Arc<ImageData>),
    Retry,
}

pub(crate) fn local_edit_cache_dir_for_repo_root(repo_root: &Path) -> PathBuf {
    repo_root.join(LOCAL_EDIT_CACHE_DIR_NAME)
}

pub(crate) fn local_edit_cache_dir() -> Option<PathBuf> {
    photo_repo_root().map(|repo_root| local_edit_cache_dir_for_repo_root(&repo_root))
}

pub(crate) fn normalized_source_path_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn source_file_state(path: &Path) -> Option<(u64, u64, u32)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())?;
    Some((metadata.len(), modified.as_secs(), modified.subsec_nanos()))
}

pub(crate) fn local_edit_cache_file_path_for_path_key(
    cache_dir: &Path,
    path_key: &str,
    variant: LocalEditCacheVariant,
) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    path_key.hash(&mut hasher);
    cache_dir.join(format!("{:016x}{}", hasher.finish(), variant.file_suffix()))
}

pub(crate) fn local_edit_cache_file_path(
    cache_dir: &Path,
    path: &Path,
    variant: LocalEditCacheVariant,
) -> PathBuf {
    let path_key = normalized_source_path_key(path);
    local_edit_cache_file_path_for_path_key(cache_dir, &path_key, variant)
}

pub(crate) fn local_edit_cache_temp_file_path(final_path: &Path) -> PathBuf {
    let temp_id = NEXT_LOCAL_EDIT_CACHE_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    final_path.with_extension(format!("{}.tmp", temp_id))
}

pub(crate) fn next_local_edit_cache_generation_id() -> u64 {
    let time_part = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let nonce = u128::from(NEXT_LOCAL_EDIT_CACHE_GENERATION_NONCE.fetch_add(1, Ordering::Relaxed));
    let mixed = time_part ^ nonce;
    u64::try_from(mixed.min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
}

pub(crate) fn local_edit_cache_io_lock() -> &'static Mutex<()> {
    LOCAL_EDIT_CACHE_IO_GUARD.get_or_init(|| Mutex::new(()))
}

pub(crate) fn with_local_edit_cache_io_lock<T>(
    f: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guard = local_edit_cache_io_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f()
}

#[cfg(test)]
pub(crate) fn set_test_local_edit_thumbnail_repair_hook(hook: impl FnOnce() + Send + 'static) {
    *TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(hook));
}

#[cfg(test)]
pub(crate) fn set_test_local_edit_thumbnail_fast_path_hook(hook: impl FnOnce() + Send + 'static) {
    *TEST_LOCAL_EDIT_THUMBNAIL_FAST_PATH_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(hook));
}

#[cfg(test)]
pub(crate) fn run_test_local_edit_thumbnail_repair_hook() {
    if let Some(hook) = TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        hook();
    }
}

#[cfg(test)]
pub(crate) fn run_test_local_edit_thumbnail_fast_path_hook() {
    if let Some(hook) = TEST_LOCAL_EDIT_THUMBNAIL_FAST_PATH_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        hook();
    }
}

#[cfg(test)]
pub(crate) fn set_test_local_edit_thumbnail_repair_write_error(error: impl Into<String>) {
    *TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_WRITE_ERROR
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.into());
}

#[cfg(test)]
pub(crate) fn clear_test_local_edit_thumbnail_hooks() {
    *TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    *TEST_LOCAL_EDIT_THUMBNAIL_FAST_PATH_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    *TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_WRITE_ERROR
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

pub(crate) fn write_repaired_local_edit_thumbnail(
    cache_dir: &Path,
    path: &Path,
    generation_id: u64,
    image: &edit::RenderedImage,
) -> Result<(), String> {
    #[cfg(test)]
    if let Some(error) = TEST_LOCAL_EDIT_THUMBNAIL_REPAIR_WRITE_ERROR
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        return Err(error);
    }

    write_local_edit_cache_variant_with_generation_to(
        cache_dir,
        path,
        LocalEditCacheVariant::Thumbnail,
        generation_id,
        image,
    )
}

pub(crate) fn persisted_local_edit_exists(path: &Path, variant: LocalEditCacheVariant) -> bool {
    let Some(cache_dir) = local_edit_cache_dir() else {
        return false;
    };
    local_edit_cache_file_path(&cache_dir, path, variant).exists()
}

pub(crate) fn local_edit_cache_fixed_header_bytes(schema_version: u32) -> Result<u64, String> {
    match schema_version {
        2 => Ok(LOCAL_EDIT_CACHE_SCHEMA_V2_FIXED_HEADER_BYTES),
        3 => Ok(LOCAL_EDIT_CACHE_SCHEMA_V3_FIXED_HEADER_BYTES),
        _ => Err("Local edit cache schema mismatch".to_string()),
    }
}

pub(crate) fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), String> {
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|e| format!("Failed to write cache: {e}"))
}

pub(crate) fn write_u64(writer: &mut impl Write, value: u64) -> Result<(), String> {
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|e| format!("Failed to write cache: {e}"))
}

pub(crate) fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Failed to read cache: {e}"))?;
    Ok(u32::from_le_bytes(bytes))
}

pub(crate) fn read_u64(reader: &mut impl Read) -> Result<u64, String> {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Failed to read cache: {e}"))?;
    Ok(u64::from_le_bytes(bytes))
}

pub(crate) fn thumbnail_from_rendered_image(
    rendered: &edit::RenderedImage,
    max_dim: u32,
) -> Result<edit::RenderedImage, String> {
    if rendered.width <= max_dim && rendered.height <= max_dim {
        return Ok(rendered.clone());
    }

    let source =
        image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.pixels.clone())
            .ok_or_else(|| "Failed to build thumbnail source image".to_string())?;
    let (thumb_width, thumb_height) =
        thumbnail_dimensions_for_image(rendered.width, rendered.height, max_dim);
    let thumb = image::imageops::resize(
        &source,
        thumb_width,
        thumb_height,
        image::imageops::FilterType::Triangle,
    );
    let (width, height) = thumb.dimensions();
    Ok(edit::RenderedImage {
        pixels: thumb.into_raw(),
        width,
        height,
    })
}

pub(crate) fn thumbnail_dimensions_for_image(width: u32, height: u32, max_dim: u32) -> (u32, u32) {
    if width == 0 || height == 0 || max_dim == 0 {
        return (width.min(max_dim), height.min(max_dim));
    }

    if width <= max_dim && height <= max_dim {
        return (width, height);
    }

    let max_side = u64::from(width.max(height));
    let max_dim = u64::from(max_dim);
    (
        ((u64::from(width) * max_dim) / max_side)
            .try_into()
            .unwrap_or(u32::MAX)
            .max(1),
        ((u64::from(height) * max_dim) / max_side)
            .try_into()
            .unwrap_or(u32::MAX)
            .max(1),
    )
}

pub(crate) fn legacy_local_edit_logical_dimensions(
    path: &Path,
    variant: LocalEditCacheVariant,
    actual_dimensions: (u32, u32),
) -> (u32, u32) {
    if !matches!(variant, LocalEditCacheVariant::Full) {
        return actual_dimensions;
    }

    let Ok(source_dimensions) = decode::source_dimensions(path) else {
        return actual_dimensions;
    };

    if actual_dimensions == (source_dimensions.1, source_dimensions.0) {
        return actual_dimensions;
    }

    if actual_dimensions.0 > source_dimensions.0 || actual_dimensions.1 > source_dimensions.1 {
        source_dimensions
    } else {
        actual_dimensions
    }
}

#[cfg(test)]
pub(crate) fn write_local_edit_cache_variant_to(
    cache_dir: &Path,
    path: &Path,
    variant: LocalEditCacheVariant,
    image: &edit::RenderedImage,
) -> Result<(), String> {
    write_local_edit_cache_variant_with_generation_to(
        cache_dir,
        path,
        variant,
        next_local_edit_cache_generation_id(),
        image,
    )
}

pub(crate) fn write_local_edit_cache_variant_with_generation_to(
    cache_dir: &Path,
    path: &Path,
    variant: LocalEditCacheVariant,
    generation_id: u64,
    image: &edit::RenderedImage,
) -> Result<(), String> {
    write_local_edit_cache_variant_with_generation_and_logical_dimensions_to(
        cache_dir,
        path,
        variant,
        generation_id,
        image,
        (image.width, image.height),
    )
}

pub(crate) fn write_local_edit_cache_variant_with_generation_and_logical_dimensions_to(
    cache_dir: &Path,
    path: &Path,
    variant: LocalEditCacheVariant,
    generation_id: u64,
    image: &edit::RenderedImage,
    logical_dimensions: (u32, u32),
) -> Result<(), String> {
    let Some((file_size, modified_secs, modified_nanos)) = source_file_state(path) else {
        return Err("Failed to read source file metadata".to_string());
    };
    let path_key = normalized_source_path_key(path);
    let final_path = local_edit_cache_file_path_for_path_key(cache_dir, &path_key, variant);
    let temp_path = local_edit_cache_temp_file_path(&final_path);

    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create local edit dir: {e}"))?;

    let write_result: Result<(), String> = (|| {
        let file =
            File::create(&temp_path).map_err(|e| format!("Failed to create cache file: {e}"))?;
        let mut writer = BufWriter::new(file);
        let path_bytes = path_key.as_bytes();
        let path_len = u32::try_from(path_bytes.len())
            .map_err(|_| "Cache path key exceeded u32 length".to_string())?;
        let pixel_len = u64::try_from(image.pixels.len())
            .map_err(|_| "Cache pixel data exceeded u64 length".to_string())?;

        writer
            .write_all(LOCAL_EDIT_CACHE_MAGIC)
            .map_err(|e| format!("Failed to write cache: {e}"))?;
        write_u32(&mut writer, LOCAL_EDIT_CACHE_SCHEMA_VERSION)?;
        write_u64(&mut writer, generation_id)?;
        write_u64(&mut writer, file_size)?;
        write_u64(&mut writer, modified_secs)?;
        write_u32(&mut writer, modified_nanos)?;
        write_u32(&mut writer, path_len)?;
        write_u32(&mut writer, image.width)?;
        write_u32(&mut writer, image.height)?;
        write_u32(&mut writer, logical_dimensions.0)?;
        write_u32(&mut writer, logical_dimensions.1)?;
        write_u64(&mut writer, pixel_len)?;
        writer
            .write_all(path_bytes)
            .map_err(|e| format!("Failed to write cache: {e}"))?;
        writer
            .write_all(&image.pixels)
            .map_err(|e| format!("Failed to write cache: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("Failed to flush cache: {e}"))?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }

    std::fs::rename(&temp_path, &final_path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        format!("Failed to finalize cache file: {e}")
    })
}

pub(crate) fn remove_persisted_local_edit(path: &Path) -> Result<(), String> {
    let Some(cache_dir) = local_edit_cache_dir() else {
        return Ok(());
    };

    with_local_edit_cache_io_lock(|| {
        for variant in [
            LocalEditCacheVariant::Full,
            LocalEditCacheVariant::Thumbnail,
        ] {
            let cache_path = local_edit_cache_file_path(&cache_dir, path, variant);
            if let Err(error) = std::fs::remove_file(&cache_path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("Failed to remove local edit cache: {error}"));
                }
            }
        }

        Ok(())
    })
}

pub(crate) fn load_persisted_local_edit_variant_header(
    path: &Path,
    variant: LocalEditCacheVariant,
) -> Result<Option<LoadedLocalEditCacheVariantHeader>, String> {
    let Some(cache_dir) = local_edit_cache_dir() else {
        return Ok(None);
    };
    let Some((file_size, modified_secs, modified_nanos)) = source_file_state(path) else {
        return Ok(None);
    };
    let path_key = normalized_source_path_key(path);
    let cache_path = local_edit_cache_file_path_for_path_key(&cache_dir, &path_key, variant);
    if !cache_path.exists() {
        return Ok(None);
    }

    let read_result: Result<LoadedLocalEditCacheVariantHeader, String> = (|| {
        let file =
            File::open(&cache_path).map_err(|e| format!("Failed to open local edit cache: {e}"))?;
        let cache_file_len = file
            .metadata()
            .map_err(|e| format!("Failed to stat local edit cache: {e}"))?
            .len();
        let mut reader = BufReader::new(file);
        let header = read_validated_local_edit_cache_header(
            &mut reader,
            &path_key,
            file_size,
            modified_secs,
            modified_nanos,
            cache_file_len,
        )?;
        Ok(LoadedLocalEditCacheVariantHeader {
            generation_id: header.generation_id,
            width: header.width,
            height: header.height,
        })
    })();

    match read_result {
        Ok(header) => Ok(Some(header)),
        Err(error) => Err(error),
    }
}

pub(crate) fn read_validated_local_edit_cache_header(
    reader: &mut BufReader<File>,
    path_key: &str,
    file_size: u64,
    modified_secs: u64,
    modified_nanos: u32,
    cache_file_len: u64,
) -> Result<ValidatedLocalEditCacheHeader, String> {
    let mut magic = [0u8; LOCAL_EDIT_CACHE_MAGIC.len()];
    reader
        .read_exact(&mut magic)
        .map_err(|e| format!("Failed to read local edit cache: {e}"))?;
    if &magic != LOCAL_EDIT_CACHE_MAGIC {
        return Err("Local edit cache magic mismatch".to_string());
    }

    let schema_version = read_u32(reader)?;
    let fixed_header_bytes = local_edit_cache_fixed_header_bytes(schema_version)?;

    let generation_id = read_u64(reader)?;
    let cached_file_size = read_u64(reader)?;
    let cached_modified_secs = read_u64(reader)?;
    let cached_modified_nanos = read_u32(reader)?;
    let path_len = read_u32(reader)? as usize;
    let width = read_u32(reader)?;
    let height = read_u32(reader)?;
    let logical_dimensions = if schema_version >= 3 {
        let logical_width = read_u32(reader)?;
        let logical_height = read_u32(reader)?;
        if logical_width == 0 || logical_height == 0 {
            return Err("Local edit cache logical dimensions were invalid".to_string());
        }
        Some((logical_width, logical_height))
    } else {
        None
    };
    let pixel_len = read_u64(reader)?;

    if cached_file_size != file_size
        || cached_modified_secs != modified_secs
        || cached_modified_nanos != modified_nanos
    {
        return Err("Local edit cache source metadata mismatch".to_string());
    }

    let expected_pixel_len = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| "Local edit cache dimensions overflowed".to_string())?;
    if pixel_len != expected_pixel_len {
        return Err("Local edit cache pixel length mismatch".to_string());
    }

    let path_len =
        u64::try_from(path_len).map_err(|_| "Local edit cache path too long".to_string())?;
    let expected_file_len = fixed_header_bytes
        .checked_add(path_len)
        .and_then(|len| len.checked_add(pixel_len))
        .ok_or_else(|| "Local edit cache file length overflowed".to_string())?;
    if cache_file_len != expected_file_len {
        return Err("Local edit cache file length mismatch".to_string());
    }

    let mut cached_path = vec![
        0u8;
        usize::try_from(path_len)
            .map_err(|_| "Local edit cache path too long".to_string())?
    ];
    reader
        .read_exact(&mut cached_path)
        .map_err(|e| format!("Failed to read local edit cache path: {e}"))?;
    if cached_path != path_key.as_bytes() {
        return Err("Local edit cache path key mismatch".to_string());
    }

    Ok(ValidatedLocalEditCacheHeader {
        generation_id,
        width,
        height,
        logical_dimensions,
        source_file_size: file_size,
    })
}

pub(crate) fn load_persisted_local_edit_variant(
    path: &Path,
    variant: LocalEditCacheVariant,
) -> Result<Option<LoadedLocalEditCacheVariant>, String> {
    let Some(cache_dir) = local_edit_cache_dir() else {
        return Ok(None);
    };
    let Some((file_size, modified_secs, modified_nanos)) = source_file_state(path) else {
        return Ok(None);
    };
    let path_key = normalized_source_path_key(path);
    let cache_path = local_edit_cache_file_path_for_path_key(&cache_dir, &path_key, variant);
    if !cache_path.exists() {
        return Ok(None);
    }
    let read_result: Result<LoadedLocalEditCacheVariant, String> = (|| {
        let file =
            File::open(&cache_path).map_err(|e| format!("Failed to open local edit cache: {e}"))?;
        let cache_file_len = file
            .metadata()
            .map_err(|e| format!("Failed to stat local edit cache: {e}"))?
            .len();
        let mut reader = BufReader::new(file);
        let header = read_validated_local_edit_cache_header(
            &mut reader,
            &path_key,
            file_size,
            modified_secs,
            modified_nanos,
            cache_file_len,
        )?;

        let pixel_len = usize::try_from(
            u64::from(header.width)
                .checked_mul(u64::from(header.height))
                .and_then(|count| count.checked_mul(4))
                .ok_or_else(|| "Local edit cache dimensions overflowed".to_string())?,
        )
        .map_err(|_| "Local edit cache pixel length exceeded usize".to_string())?;
        let mut pixels = vec![0u8; pixel_len];
        reader
            .read_exact(&mut pixels)
            .map_err(|e| format!("Failed to read local edit cache pixels: {e}"))?;

        Ok(LoadedLocalEditCacheVariant {
            generation_id: header.generation_id,
            logical_dimensions: header.logical_dimensions.unwrap_or_else(|| {
                legacy_local_edit_logical_dimensions(path, variant, (header.width, header.height))
            }),
            image: Arc::new(ImageData {
                pixels,
                width: header.width,
                height: header.height,
                file_size: header.source_file_size,
            }),
        })
    })();

    match read_result {
        Ok(image) => Ok(Some(image)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub(crate) fn load_persisted_local_edit_image(
    path: &Path,
) -> Result<Option<Arc<ImageData>>, String> {
    Ok(load_persisted_local_edit(path)?.map(|entry| entry.image))
}

pub(crate) fn load_persisted_local_edit(
    path: &Path,
) -> Result<Option<LoadedPersistedLocalEdit>, String> {
    Ok(
        load_persisted_local_edit_variant(path, LocalEditCacheVariant::Full)?.map(|entry| {
            LoadedPersistedLocalEdit {
                image: entry.image,
                logical_dimensions: entry.logical_dimensions,
            }
        }),
    )
}

pub(crate) fn persisted_thumbnail_matches_generation_and_dimensions(
    thumbnail_entry: &LoadedLocalEditCacheVariant,
    generation_id: u64,
    expected_dimensions: (u32, u32),
) -> bool {
    thumbnail_entry.generation_id == generation_id
        && thumbnail_entry.image.width == expected_dimensions.0
        && thumbnail_entry.image.height == expected_dimensions.1
}

pub(crate) fn load_repaired_local_edit_thumbnail(
    path: &Path,
    max_dim: u32,
) -> Result<Option<Arc<ImageData>>, String> {
    for _attempt in 0..8 {
        let repair_decision = with_local_edit_cache_io_lock(|| {
            #[cfg(test)]
            run_test_local_edit_thumbnail_repair_hook();

            let full_header = match load_persisted_local_edit_variant_header(
                path,
                LocalEditCacheVariant::Full,
            ) {
                Ok(Some(header)) => Some(header),
                Ok(None) => None,
                Err(error) => {
                    log::debug!(
                            "Ignoring persisted local edit full cache while repairing thumbnail for {}: {}",
                            path.display(),
                            error
                        );
                    None
                }
            };
            let Some(full_header) = full_header else {
                return Ok(LocalEditThumbnailRepairDecision::Missing);
            };

            let expected_dimensions =
                thumbnail_dimensions_for_image(full_header.width, full_header.height, max_dim);
            let thumbnail_header = match load_persisted_local_edit_variant_header(
                path,
                LocalEditCacheVariant::Thumbnail,
            ) {
                Ok(Some(header)) => Some(header),
                Ok(None) => None,
                Err(error) => {
                    log::debug!(
                        "Ignoring persisted local edit thumbnail cache while repairing {}: {}",
                        path.display(),
                        error
                    );
                    None
                }
            };
            if let Some(thumbnail_header) = thumbnail_header {
                if thumbnail_header.generation_id == full_header.generation_id
                    && thumbnail_header.width == expected_dimensions.0
                    && thumbnail_header.height == expected_dimensions.1
                {
                    let thumbnail_entry = match load_persisted_local_edit_variant(
                        path,
                        LocalEditCacheVariant::Thumbnail,
                    ) {
                        Ok(Some(image)) => Some(image),
                        Ok(None) => None,
                        Err(error) => {
                            log::debug!(
                                    "Ignoring persisted local edit thumbnail cache while repairing {}: {}",
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
                            return Ok(LocalEditThumbnailRepairDecision::Return(
                                thumbnail_entry.image,
                            ));
                        }
                    }
                }
            }

            Ok(LocalEditThumbnailRepairDecision::Derive {
                generation_id: full_header.generation_id,
            })
        })?;

        let generation_id = match repair_decision {
            LocalEditThumbnailRepairDecision::Missing => return Ok(None),
            LocalEditThumbnailRepairDecision::Return(image) => return Ok(Some(image)),
            LocalEditThumbnailRepairDecision::Derive { generation_id } => generation_id,
        };

        let full_entry = match load_persisted_local_edit_variant(path, LocalEditCacheVariant::Full)
        {
            Ok(Some(image)) => Some(image),
            Ok(None) => None,
            Err(error) => {
                log::debug!(
                    "Ignoring persisted local edit full cache while deriving repair for {}: {}",
                    path.display(),
                    error
                );
                None
            }
        };
        let Some(full_entry) = full_entry else {
            continue;
        };
        if full_entry.generation_id != generation_id {
            continue;
        }

        let full_image = full_entry.image;

        let derived_thumb = thumbnail_from_rendered_image(
            &edit::RenderedImage {
                pixels: full_image.pixels.clone(),
                width: full_image.width,
                height: full_image.height,
            },
            max_dim,
        )?;
        let repaired_thumb = Arc::new(ImageData {
            pixels: derived_thumb.pixels.clone(),
            width: derived_thumb.width,
            height: derived_thumb.height,
            file_size: full_image.file_size,
        });

        let finalize = with_local_edit_cache_io_lock(|| {
            let full_header = match load_persisted_local_edit_variant_header(
                path,
                LocalEditCacheVariant::Full,
            ) {
                Ok(Some(header)) => Some(header),
                Ok(None) => None,
                Err(error) => {
                    log::debug!(
                        "Ignoring persisted local edit full cache header while finalizing repair for {}: {}",
                        path.display(),
                        error
                    );
                    None
                }
            };
            let Some(full_header) = full_header else {
                return Ok(FinalizeLocalEditThumbnailRepair::Retry);
            };

            let expected_dimensions =
                thumbnail_dimensions_for_image(full_header.width, full_header.height, max_dim);
            let thumbnail_entry = match load_persisted_local_edit_variant(
                path,
                LocalEditCacheVariant::Thumbnail,
            ) {
                Ok(Some(image)) => Some(image),
                Ok(None) => None,
                Err(error) => {
                    log::debug!(
                        "Ignoring persisted local edit thumbnail cache while finalizing repair for {}: {}",
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
                    return Ok(FinalizeLocalEditThumbnailRepair::Return(
                        thumbnail_entry.image,
                    ));
                }
            }

            if full_header.generation_id != generation_id {
                return Ok(FinalizeLocalEditThumbnailRepair::Retry);
            }

            if let Some(cache_dir) = local_edit_cache_dir() {
                if let Err(error) = write_repaired_local_edit_thumbnail(
                    &cache_dir,
                    path,
                    generation_id,
                    &derived_thumb,
                ) {
                    log::warn!(
                        "Failed to repair stale local edit thumbnail for {}: {}",
                        path.display(),
                        error
                    );
                }
            }

            Ok(FinalizeLocalEditThumbnailRepair::Return(
                repaired_thumb.clone(),
            ))
        })?;

        match finalize {
            FinalizeLocalEditThumbnailRepair::Return(image) => return Ok(Some(image)),
            FinalizeLocalEditThumbnailRepair::Retry => continue,
        };
    }

    let full_header =
        match load_persisted_local_edit_variant_header(path, LocalEditCacheVariant::Full) {
            Ok(Some(header)) => Some(header),
            Ok(None) => None,
            Err(error) => {
                log::debug!(
                "Ignoring persisted local edit full cache header after repair retries for {}: {}",
                path.display(),
                error
            );
                None
            }
        };
    if let Some(full_header) = full_header {
        let expected_dimensions =
            thumbnail_dimensions_for_image(full_header.width, full_header.height, max_dim);
        let thumbnail_entry =
            match load_persisted_local_edit_variant(path, LocalEditCacheVariant::Thumbnail) {
                Ok(Some(image)) => Some(image),
                Ok(None) => None,
                Err(error) => {
                    log::debug!(
                    "Ignoring persisted local edit thumbnail cache after repair retries for {}: {}",
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
                return Ok(Some(thumbnail_entry.image));
            }
        }

        let full_entry = match load_persisted_local_edit_variant(path, LocalEditCacheVariant::Full)
        {
            Ok(Some(image)) => Some(image),
            Ok(None) => None,
            Err(error) => {
                log::debug!(
                    "Ignoring persisted local edit full cache after repair retries for {}: {}",
                    path.display(),
                    error
                );
                None
            }
        };
        if let Some(full_entry) = full_entry {
            if full_entry.generation_id == full_header.generation_id {
                let derived_thumb = thumbnail_from_rendered_image(
                    &edit::RenderedImage {
                        pixels: full_entry.image.pixels.clone(),
                        width: full_entry.image.width,
                        height: full_entry.image.height,
                    },
                    max_dim,
                )?;
                return Ok(Some(Arc::new(ImageData {
                    pixels: derived_thumb.pixels,
                    width: derived_thumb.width,
                    height: derived_thumb.height,
                    file_size: full_entry.image.file_size,
                })));
            }
        }
    }

    Ok(None)
}

pub(crate) fn persist_local_edit(
    request: &LocalEditPersistRequest,
) -> Result<Option<Arc<ImageData>>, String> {
    if request.state.is_default() && matches!(request.base_source, BaseImageSource::Original) {
        remove_persisted_local_edit(&request.path)?;
        return Ok(None);
    }

    let full = edit::render_edited_image(
        &request.image.pixels,
        request.image.width,
        request.image.height,
        &request.state,
        request.lens,
    );
    let thumb = thumbnail_from_rendered_image(&full, LOCAL_EDIT_THUMBNAIL_MAX_DIM)?;

    if let Some(cache_dir) = local_edit_cache_dir() {
        with_local_edit_cache_io_lock(|| {
            let generation_id = next_local_edit_cache_generation_id();
            write_local_edit_cache_variant_with_generation_and_logical_dimensions_to(
                &cache_dir,
                &request.path,
                LocalEditCacheVariant::Full,
                generation_id,
                &full,
                request.logical_dimensions,
            )?;
            write_local_edit_cache_variant_with_generation_to(
                &cache_dir,
                &request.path,
                LocalEditCacheVariant::Thumbnail,
                generation_id,
                &thumb,
            )?;
            Ok(())
        })?;
    }

    Ok(Some(Arc::new(ImageData {
        pixels: thumb.pixels,
        width: thumb.width,
        height: thumb.height,
        file_size: request.image.file_size,
    })))
}
