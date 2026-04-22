use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use image25::DynamicImage as RawDynamicImage;
use rawler::decoders::{Decoder, RawDecodeParams};
use rawler::imgop::develop::RawDevelop;
use rawler::rawsource::RawSource;

use crate::nav;

/// Decoded image in RGBA8 format ready for GPU upload.
#[derive(Debug)]
pub struct ImageData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
}

const MAX_TEXTURE_DIM: u32 = 16384;
const DECODE_CACHE_MAGIC: &[u8; 8] = b"PHOCACHE";
const DECODE_CACHE_SCHEMA_VERSION: u32 = 3;
// Older persisted RAW cache entries were observed at bogus oversized dimensions, so force a
// one-time rebuild rather than trusting pre-fix cache content across sessions.
const DECODE_CACHE_CONTRACT_VERSION: u64 = 4;
const DECODE_CACHE_DIR_NAME: &str = "decoded-cache";
const DECODE_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DECODE_CACHE_TRIM_TARGET_BYTES: u64 = 1_536 * 1024 * 1024;
const DECODE_CACHE_PRUNE_WRITE_INTERVAL: u64 = 8;
const SOURCE_FINGERPRINT_BUFFER_BYTES: usize = 64 * 1024;
static NEXT_CACHE_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);
static CACHE_WRITES_SINCE_PRUNE: AtomicU64 = AtomicU64::new(0);
static PHOTO_REPO_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
#[cfg(test)]
static TEST_DECODE_CACHE_DIR_OVERRIDE: OnceLock<std::sync::Mutex<Option<Option<PathBuf>>>> =
    OnceLock::new();
#[cfg(test)]
static TEST_DECODE_CACHE_DIR_GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
#[cfg(test)]
static TEST_PHOTO_REPO_ROOT_OVERRIDE: OnceLock<std::sync::Mutex<Option<Option<PathBuf>>>> =
    OnceLock::new();
#[cfg(test)]
static TEST_PHOTO_REPO_ROOT_GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFingerprint {
    path_key: String,
    file_size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedImage {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl SourceFingerprint {
    fn from_path(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| (duration.as_secs(), duration.subsec_nanos()))
            .unwrap_or((0, 0));

        Some(Self {
            path_key: normalized_source_path_key(path),
            file_size: metadata.len(),
            modified_secs: modified.0,
            modified_nanos: modified.1,
        })
    }
}

fn normalized_source_path_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn source_content_hash(path: &Path, file_size: u64) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut hasher = DefaultHasher::new();
    file_size.hash(&mut hasher);
    let mut buffer = vec![0; SOURCE_FINGERPRINT_BUFFER_BYTES];

    loop {
        let read = reader.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        buffer[..read].hash(&mut hasher);
    }

    Some(hasher.finish())
}

#[derive(Clone, Copy)]
enum EmbeddedImageKind {
    Thumbnail,
    Preview,
    FullImage,
}

impl EmbeddedImageKind {
    fn label(self) -> &'static str {
        match self {
            EmbeddedImageKind::Thumbnail => "thumbnail",
            EmbeddedImageKind::Preview => "preview",
            EmbeddedImageKind::FullImage => "full image",
        }
    }
}

fn raw_dynamic_image_to_rgba(image: RawDynamicImage, max_dim: u32) -> (Vec<u8>, u32, u32) {
    let image = image.thumbnail(max_dim, max_dim);
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    (rgba.into_raw(), w, h)
}

fn with_raw_decoder<T>(
    path: &Path,
    f: impl FnOnce(&RawSource, &dyn Decoder, &RawDecodeParams) -> Result<T, String>,
) -> Result<T, String> {
    let rawfile = RawSource::new(path).map_err(|e| format!("Failed to open RAW container: {e}"))?;
    let decoder = rawler::get_decoder(&rawfile)
        .map_err(|e| format!("Failed to initialize RAW decoder: {e}"))?;
    let params = RawDecodeParams::default();
    f(&rawfile, decoder.as_ref(), &params)
}

fn decode_embedded_image_kind(
    decoder: &dyn Decoder,
    rawfile: &RawSource,
    params: &RawDecodeParams,
    max_dim: u32,
    kind: EmbeddedImageKind,
) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
    let result = match kind {
        EmbeddedImageKind::Thumbnail => decoder.thumbnail_image(rawfile, params),
        EmbeddedImageKind::Preview => decoder.preview_image(rawfile, params),
        EmbeddedImageKind::FullImage => decoder.full_image(rawfile, params),
    };

    match result {
        Ok(Some(image)) => Ok(Some(raw_dynamic_image_to_rgba(image, max_dim))),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("Failed to extract RAW {}: {e}", kind.label())),
    }
}

pub fn decode_image(path: &Path) -> Result<Arc<ImageData>, String> {
    let cache_dir = decode_image_cache_dir(path);
    decode_image_with_cache_dir(path, cache_dir.as_deref())
}

pub fn source_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "svg" | "svgz" => svg_source_dimensions(path),
        _ if nav::is_raw_file(path) => raw_source_dimensions(path),
        _ => image::image_dimensions(path)
            .map_err(|e| format!("Failed to inspect image dimensions: {e}")),
    }
}

pub fn warm_persisted_decoded_cache(path: &Path) -> Result<bool, String> {
    if !supports_persisted_decoded_cache(path) {
        return Ok(false);
    }

    decode_image(path).map(|_| true)
}

fn decode_image_cache_dir(path: &Path) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let override_dir = TEST_DECODE_CACHE_DIR_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(Some(None)))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        return match override_dir {
            Some(cache_dir) => cache_dir,
            None => decoded_cache_dir_for(path),
        };
    }

    #[cfg(not(test))]
    {
        decoded_cache_dir_for(path)
    }
}

fn decode_image_with_cache_dir(
    path: &Path,
    cache_dir: Option<&Path>,
) -> Result<Arc<ImageData>, String> {
    load_or_decode_cached_full_image(path, cache_dir, decode_image_uncached)
}

fn decode_image_uncached(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let (pixels, width, height) = match ext.as_str() {
        "svg" | "svgz" => decode_svg(path)?,
        _ if nav::is_raw_file(path) => decode_raw(path, MAX_TEXTURE_DIM, false)?,
        _ => decode_raster(path)?,
    };

    Ok((pixels, width, height))
}

pub fn decode_embedded_preview(path: &Path) -> Result<Option<Arc<ImageData>>, String> {
    if !nav::is_raw_file(path) {
        return Ok(None);
    }

    let Some((pixels, width, height)) = decode_raw_embedded_preview(path, MAX_TEXTURE_DIM)? else {
        return Ok(None);
    };

    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(Some(Arc::new(ImageData {
        pixels,
        width,
        height,
        file_size,
    })))
}

fn decode_raster(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path).map_err(|e| format!("Failed to decode: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();

    // Downscale if exceeding GPU texture limits
    if w > MAX_TEXTURE_DIM || h > MAX_TEXTURE_DIM {
        let scale = MAX_TEXTURE_DIM as f32 / w.max(h) as f32;
        let nw = (w as f32 * scale) as u32;
        let nh = (h as f32 * scale) as u32;
        let resized = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Lanczos3);
        let (rw, rh) = resized.dimensions();
        return Ok((resized.into_raw(), rw, rh));
    }

    Ok((rgba.into_raw(), w, h))
}

fn load_or_decode_cached_full_image(
    path: &Path,
    cache_dir: Option<&Path>,
    decode_source: impl FnOnce(&Path) -> Result<(Vec<u8>, u32, u32), String>,
) -> Result<Arc<ImageData>, String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let fingerprint = cache_dir.and_then(|_| SourceFingerprint::from_path(path));

    if let (Some(cache_dir), Some(fingerprint)) = (cache_dir, fingerprint.as_ref()) {
        match read_decoded_cache(cache_dir, path, fingerprint) {
            Ok(Some(cached)) => {
                return Ok(Arc::new(ImageData {
                    pixels: cached.pixels,
                    width: cached.width,
                    height: cached.height,
                    file_size,
                }));
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!(
                    "Decoded cache read failed for {}: {}",
                    path.display(),
                    error
                );
            }
        }
    }

    let (pixels, width, height) = decode_source(path)?;

    if let (Some(cache_dir), Some(fingerprint)) = (cache_dir, fingerprint.as_ref()) {
        match source_content_hash(path, fingerprint.file_size) {
            Some(content_hash) => {
                if let Err(error) = write_decoded_cache(
                    cache_dir,
                    fingerprint,
                    content_hash,
                    width,
                    height,
                    &pixels,
                ) {
                    log::warn!(
                        "Decoded cache write failed for {}: {}",
                        path.display(),
                        error
                    );
                }
            }
            None => {
                log::warn!(
                    "Decoded cache write skipped for {}: failed to hash source file",
                    path.display()
                );
            }
        }
    }

    Ok(Arc::new(ImageData {
        pixels,
        width,
        height,
        file_size,
    }))
}

fn decoded_cache_dir_for(path: &Path) -> Option<PathBuf> {
    if !supports_persisted_decoded_cache(path) {
        return None;
    }

    let repo_root = photo_repo_root()?;
    decoded_cache_dir_for_repo_root(path, &repo_root)
}

fn decoded_cache_dir_for_repo_root(path: &Path, repo_root: &Path) -> Option<PathBuf> {
    if !supports_persisted_decoded_cache(path) {
        return None;
    }

    Some(decoded_cache_root(repo_root))
}

fn supports_persisted_decoded_cache(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    matches!(ext.as_str(), "svg" | "svgz") || nav::is_raw_file(path)
}

pub fn path_uses_persisted_decoded_cache(path: &Path) -> bool {
    supports_persisted_decoded_cache(path)
}

fn decoded_cache_file_path(cache_dir: &Path, path_key: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    path_key.hash(&mut hasher);
    cache_dir.join(format!("{:016x}.rgba", hasher.finish()))
}

fn decoded_cache_root(repo_root: &Path) -> PathBuf {
    repo_root.join(DECODE_CACHE_DIR_NAME)
}

fn photo_repo_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        let override_root = TEST_PHOTO_REPO_ROOT_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(repo_root) = override_root {
            return repo_root;
        }
    }

    PHOTO_REPO_ROOT
        .get_or_init(discover_photo_repo_root)
        .clone()
}

fn discover_photo_repo_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| find_photo_repo_root(path.parent()?))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|dir| find_photo_repo_root(&dir))
        })
}

fn find_photo_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_photo_repo_root(candidate))
        .map(Path::to_path_buf)
}

fn is_photo_repo_root(candidate: &Path) -> bool {
    candidate.join(".git").exists()
        && candidate.join("AGENTS.md").is_file()
        && candidate.join("Cargo.toml").is_file()
        && candidate.join("src").join("decode.rs").is_file()
}

fn decoded_cache_contract_hash() -> u64 {
    DECODE_CACHE_CONTRACT_VERSION
}

fn decoded_cache_temp_file_path(cache_path: &Path) -> PathBuf {
    let temp_id = NEXT_CACHE_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    cache_path.with_extension(format!("tmp-{}-{temp_id}", std::process::id()))
}

fn maybe_prune_decoded_cache_after_write(
    cache_dir: &Path,
    writes_since_prune: u64,
) -> Result<(), String> {
    if writes_since_prune.is_multiple_of(DECODE_CACHE_PRUNE_WRITE_INTERVAL) {
        prune_decoded_cache(
            cache_dir,
            DECODE_CACHE_MAX_BYTES,
            DECODE_CACHE_TRIM_TARGET_BYTES,
        )?;
    }
    Ok(())
}

fn write_decoded_cache(
    cache_dir: &Path,
    fingerprint: &SourceFingerprint,
    content_hash: u64,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), String> {
    let expected_len = pixel_len(width, height)?;
    if pixels.len() != expected_len {
        return Err(format!(
            "Cache pixel length mismatch: expected {expected_len}, got {}",
            pixels.len()
        ));
    }

    std::fs::create_dir_all(cache_dir).map_err(|e| format!("Failed to create cache dir: {e}"))?;
    let cache_path = decoded_cache_file_path(cache_dir, &fingerprint.path_key);
    let temp_path = decoded_cache_temp_file_path(&cache_path);
    let file = File::create(&temp_path).map_err(|e| format!("Failed to create cache: {e}"))?;
    let write_result: Result<(), String> = (|| {
        let mut writer = BufWriter::new(file);
        let path_bytes = fingerprint.path_key.as_bytes();
        let path_len = u32::try_from(path_bytes.len())
            .map_err(|_| "Cache path key exceeded u32 length".to_string())?;
        let pixel_len = u64::try_from(pixels.len())
            .map_err(|_| "Cache pixel buffer exceeded u64 length".to_string())?;

        writer
            .write_all(DECODE_CACHE_MAGIC)
            .map_err(|e| format!("Failed to write cache header: {e}"))?;
        writer
            .write_all(&DECODE_CACHE_SCHEMA_VERSION.to_le_bytes())
            .map_err(|e| format!("Failed to write cache schema version: {e}"))?;
        writer
            .write_all(&decoded_cache_contract_hash().to_le_bytes())
            .map_err(|e| format!("Failed to write cache contract hash: {e}"))?;
        writer
            .write_all(&path_len.to_le_bytes())
            .map_err(|e| format!("Failed to write cache path length: {e}"))?;
        writer
            .write_all(path_bytes)
            .map_err(|e| format!("Failed to write cache path key: {e}"))?;
        writer
            .write_all(&fingerprint.file_size.to_le_bytes())
            .map_err(|e| format!("Failed to write cache file size: {e}"))?;
        writer
            .write_all(&fingerprint.modified_secs.to_le_bytes())
            .map_err(|e| format!("Failed to write cache modified secs: {e}"))?;
        writer
            .write_all(&fingerprint.modified_nanos.to_le_bytes())
            .map_err(|e| format!("Failed to write cache modified nanos: {e}"))?;
        writer
            .write_all(&content_hash.to_le_bytes())
            .map_err(|e| format!("Failed to write cache content hash: {e}"))?;
        writer
            .write_all(&width.to_le_bytes())
            .map_err(|e| format!("Failed to write cache width: {e}"))?;
        writer
            .write_all(&height.to_le_bytes())
            .map_err(|e| format!("Failed to write cache height: {e}"))?;
        writer
            .write_all(&pixel_len.to_le_bytes())
            .map_err(|e| format!("Failed to write cache pixel length: {e}"))?;
        writer
            .write_all(pixels)
            .map_err(|e| format!("Failed to write cache pixels: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("Failed to flush cache file: {e}"))?;
        drop(writer);

        let _ = std::fs::remove_file(&cache_path);
        std::fs::rename(&temp_path, &cache_path)
            .map_err(|e| format!("Failed to finalize cache file: {e}"))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    write_result?;
    let writes_since_prune = CACHE_WRITES_SINCE_PRUNE.fetch_add(1, Ordering::Relaxed) + 1;
    maybe_prune_decoded_cache_after_write(cache_dir, writes_since_prune)?;
    Ok(())
}

fn read_decoded_cache(
    cache_dir: &Path,
    path: &Path,
    fingerprint: &SourceFingerprint,
) -> Result<Option<CachedImage>, String> {
    let cache_path = decoded_cache_file_path(cache_dir, &fingerprint.path_key);
    if !cache_path.exists() {
        return Ok(None);
    }

    let file = File::open(&cache_path).map_err(|e| format!("Failed to open cache: {e}"))?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 8];
    reader
        .read_exact(&mut magic)
        .map_err(|e| format!("Failed to read cache header: {e}"))?;
    if &magic != DECODE_CACHE_MAGIC {
        return Ok(None);
    }
    if read_u32(&mut reader)? != DECODE_CACHE_SCHEMA_VERSION {
        return Ok(None);
    }
    if read_u64(&mut reader)? != decoded_cache_contract_hash() {
        return Ok(None);
    }

    let path_len = usize::try_from(read_u32(&mut reader)?)
        .map_err(|_| "Cache path length exceeded usize".to_string())?;
    if path_len > 65_536 {
        return Err(format!(
            "Cache path length {path_len} exceeded safety limit"
        ));
    }
    let mut path_bytes = vec![0u8; path_len];
    reader
        .read_exact(&mut path_bytes)
        .map_err(|e| format!("Failed to read cache path key: {e}"))?;
    let cached_path = String::from_utf8(path_bytes)
        .map_err(|e| format!("Cache path key was not valid UTF-8: {e}"))?;
    let cached_file_size = read_u64(&mut reader)?;
    let cached_modified_secs = read_u64(&mut reader)?;
    let cached_modified_nanos = read_u32(&mut reader)?;
    let cached_content_hash = read_u64(&mut reader)?;

    if cached_path != fingerprint.path_key || cached_file_size != fingerprint.file_size {
        return Ok(None);
    }
    if cached_modified_secs != fingerprint.modified_secs
        || cached_modified_nanos != fingerprint.modified_nanos
    {
        let actual_content_hash = source_content_hash(path, fingerprint.file_size)
            .ok_or_else(|| format!("Failed to hash source file for {}", path.display()))?;
        if cached_content_hash != actual_content_hash {
            return Ok(None);
        }
    }

    let width = read_u32(&mut reader)?;
    let height = read_u32(&mut reader)?;
    let cached_pixel_len = usize::try_from(read_u64(&mut reader)?)
        .map_err(|_| "Cache pixel length exceeded usize".to_string())?;
    let expected_len = pixel_len(width, height)?;
    if cached_pixel_len != expected_len {
        return Err(format!(
            "Cache pixel length mismatch: expected {expected_len}, got {cached_pixel_len}"
        ));
    }

    let mut pixels = vec![0u8; cached_pixel_len];
    reader
        .read_exact(&mut pixels)
        .map_err(|e| format!("Failed to read cache pixels: {e}"))?;

    Ok(Some(CachedImage {
        pixels,
        width,
        height,
    }))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Failed to read u32: {e}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, String> {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Failed to read u64: {e}"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn pixel_len(width: u32, height: u32) -> Result<usize, String> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Decoded image dimensions overflowed pixel buffer size".to_string())?;
    usize::try_from(pixels).map_err(|_| "Decoded image buffer exceeded usize".to_string())
}

fn prune_decoded_cache(
    cache_dir: &Path,
    max_bytes: u64,
    trim_target_bytes: u64,
) -> Result<(), String> {
    if trim_target_bytes > max_bytes {
        return Err(format!(
            "Cache trim target {trim_target_bytes} exceeded max budget {max_bytes}"
        ));
    }

    let mut total_bytes = 0u64;
    let mut entries = vec![];

    for entry in
        std::fs::read_dir(cache_dir).map_err(|e| format!("Failed to read cache dir: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Failed to inspect cache entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rgba") {
            continue;
        }

        let metadata = entry
            .metadata()
            .map_err(|e| format!("Failed to read cache entry metadata: {e}"))?;
        total_bytes = total_bytes.saturating_add(metadata.len());
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        entries.push((path, metadata.len(), modified));
    }

    if total_bytes <= max_bytes {
        return Ok(());
    }

    entries.sort_by_key(|(_, _, modified)| *modified);
    for (path, len, _) in entries {
        if total_bytes <= trim_target_bytes {
            break;
        }

        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove cache entry {}: {e}", path.display()))?;
        total_bytes = total_bytes.saturating_sub(len);
    }

    Ok(())
}

fn load_svg_tree(path: &Path) -> Result<resvg::usvg::Tree, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read SVG: {e}"))?;
    resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| format!("Failed to parse SVG: {e}"))
}

fn svg_tree_dimensions(tree: &resvg::usvg::Tree) -> Result<(u32, u32), String> {
    let size = tree.size();
    let width = size.width() as u32;
    let height = size.height() as u32;
    if width == 0 || height == 0 {
        return Err("SVG has zero dimensions".to_string());
    }
    Ok((width, height))
}

fn svg_source_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let tree = load_svg_tree(path)?;
    svg_tree_dimensions(&tree)
}

fn raw_source_dimensions(path: &Path) -> Result<(u32, u32), String> {
    with_raw_decoder(path, |rawfile, decoder, params| {
        let rawimage = decoder
            .raw_image(rawfile, params, true)
            .map_err(|e| format!("Failed to read RAW dimensions: {e}"))?;
        let width = u32::try_from(rawimage.width)
            .map_err(|_| format!("RAW width {} exceeded u32", rawimage.width))?;
        let height = u32::try_from(rawimage.height)
            .map_err(|_| format!("RAW height {} exceeded u32", rawimage.height))?;
        Ok((width, height))
    })
}

fn decode_svg(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let tree = load_svg_tree(path)?;
    let size = tree.size();
    let (mut w, mut h) = svg_tree_dimensions(&tree)?;

    // Clamp to reasonable size
    if w > MAX_TEXTURE_DIM || h > MAX_TEXTURE_DIM {
        let scale = MAX_TEXTURE_DIM as f32 / w.max(h) as f32;
        w = (w as f32 * scale) as u32;
        h = (h as f32 * scale) as u32;
    }

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| "Failed to create pixmap".to_string())?;

    let sx = w as f32 / size.width();
    let sy = h as f32 / size.height();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );

    Ok((pixmap.take(), w, h))
}

pub fn decode_thumbnail(path: &Path, max_dim: u32) -> Result<Arc<ImageData>, String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let (pixels, w, h) = match ext.as_str() {
        "jpg" | "jpeg" => decode_jpeg_thumbnail(path, max_dim)?,
        "svg" | "svgz" => decode_svg_thumbnail(path, max_dim)?,
        _ if nav::is_raw_file(path) => decode_raw(path, max_dim, true)?,
        _ => decode_raster_thumbnail(path, max_dim)?,
    };

    Ok(Arc::new(ImageData {
        pixels,
        width: w,
        height: h,
        file_size,
    }))
}

fn decode_jpeg_thumbnail(path: &Path, max_dim: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open: {e}"))?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));

    // Use DCT-level downscaling (1/8, 1/4, 1/2) for fast decode
    let (scaled_w, scaled_h) = decoder
        .scale(max_dim as u16, max_dim as u16)
        .map_err(|e| format!("JPEG scale error: {e}"))?;

    let raw = decoder
        .decode()
        .map_err(|e| format!("JPEG decode error: {e}"))?;
    let info = decoder.info().ok_or_else(|| "No JPEG info".to_string())?;

    // Convert to RGBA
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let mut out = Vec::with_capacity(raw.len() / 3 * 4);
            for chunk in raw.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        jpeg_decoder::PixelFormat::L8 => {
            let mut out = Vec::with_capacity(raw.len() * 4);
            for &lum in &raw {
                out.extend_from_slice(&[lum, lum, lum, 255]);
            }
            out
        }
        _ => return decode_raster_thumbnail(path, max_dim),
    };

    let w = scaled_w as u32;
    let h = scaled_h as u32;

    // If DCT scaling already got us small enough, return directly
    if w <= max_dim && h <= max_dim {
        return Ok((rgba, w, h));
    }

    // Otherwise do a final resize from the reduced image
    let src = image::RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| "Failed to create image buffer".to_string())?;
    let scale = max_dim as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale) as u32).max(1);
    let nh = ((h as f32 * scale) as u32).max(1);
    let resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Nearest);
    let (rw, rh) = resized.dimensions();
    Ok((resized.into_raw(), rw, rh))
}

fn decode_svg_thumbnail(path: &Path, max_dim: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let tree = load_svg_tree(path)?;
    let size = tree.size();
    let (orig_w, orig_h) = svg_tree_dimensions(&tree)?;

    // Render directly at thumbnail size
    let scale = max_dim as f32 / orig_w.max(orig_h) as f32;
    let scale = scale.min(1.0); // Don't upscale
    let w = ((orig_w as f32 * scale) as u32).max(1);
    let h = ((orig_h as f32 * scale) as u32).max(1);

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| "Failed to create pixmap".to_string())?;

    let sx = w as f32 / size.width();
    let sy = h as f32 / size.height();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );

    Ok((pixmap.take(), w, h))
}

fn decode_raster_thumbnail(path: &Path, max_dim: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path).map_err(|e| format!("Failed to decode: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();

    if w <= max_dim && h <= max_dim {
        return Ok((rgba.into_raw(), w, h));
    }

    let scale = max_dim as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale) as u32).max(1);
    let nh = ((h as f32 * scale) as u32).max(1);
    let resized = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Nearest);
    let (rw, rh) = resized.dimensions();
    Ok((resized.into_raw(), rw, rh))
}

fn decode_raw(
    path: &Path,
    max_dim: u32,
    prefer_thumbnail: bool,
) -> Result<(Vec<u8>, u32, u32), String> {
    with_raw_decoder(path, |rawfile, decoder, params| {
        let mut last_optional_error: Option<String> = None;

        let mut take_embedded_image =
            |kind| match decode_embedded_image_kind(decoder, rawfile, params, max_dim, kind) {
                Ok(Some(image)) => Some(image),
                Ok(None) => None,
                Err(e) => {
                    last_optional_error = Some(e);
                    None
                }
            };

        let format_error = |primary: String, optional: Option<String>| match optional {
            Some(optional) => format!("{primary} (after {optional})"),
            None => primary,
        };

        let decode_raw_pixels = || -> Result<(Vec<u8>, u32, u32), String> {
            let rawimage = decoder
                .raw_image(rawfile, params, false)
                .map_err(|e| format!("Failed to read RAW pixel data: {e}"))?;
            let image = RawDevelop::default()
                .develop_intermediate(&rawimage)
                .map_err(|e| format!("Failed to develop RAW pixel data: {e}"))?
                .to_dynamic_image()
                .ok_or_else(|| "Failed to convert RAW output to an image".to_string())?;

            Ok(raw_dynamic_image_to_rgba(image, max_dim))
        };

        let embedded_kinds = if prefer_thumbnail {
            [
                EmbeddedImageKind::Thumbnail,
                EmbeddedImageKind::Preview,
                EmbeddedImageKind::FullImage,
            ]
        } else {
            [
                EmbeddedImageKind::FullImage,
                EmbeddedImageKind::Preview,
                EmbeddedImageKind::Thumbnail,
            ]
        };

        if prefer_thumbnail {
            for kind in embedded_kinds {
                if let Some(image) = take_embedded_image(kind) {
                    return Ok(image);
                }
            }

            return decode_raw_pixels().map_err(|error| format_error(error, last_optional_error));
        }

        let raw_error = match decode_raw_pixels() {
            Ok(image) => return Ok(image),
            Err(error) => error,
        };

        for kind in embedded_kinds {
            if let Some(image) = take_embedded_image(kind) {
                return Ok(image);
            }
        }

        Err(format_error(raw_error, last_optional_error))
    })
}

fn decode_raw_embedded_preview(
    path: &Path,
    max_dim: u32,
) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
    with_raw_decoder(path, |rawfile, decoder, params| {
        let mut last_error: Option<String> = None;

        for kind in [
            EmbeddedImageKind::FullImage,
            EmbeddedImageKind::Preview,
            EmbeddedImageKind::Thumbnail,
        ] {
            match decode_embedded_image_kind(decoder, rawfile, params, max_dim, kind) {
                Ok(Some(image)) => return Ok(Some(image)),
                Ok(None) => {}
                Err(e) => last_error = Some(e),
            }
        }

        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use rawler::dng::writer::DngWriter;
    use rawler::dng::{DngCompression, DNG_VERSION_V1_4};

    fn create_test_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let path = dir.join(name);
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        img.save(&path).unwrap();
        path
    }

    fn create_test_raw_dng(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let path = dir.join(name);
        let pixels: Vec<u8> = (0..(w * h))
            .flat_map(|index| {
                let value = (index % 255) as u8;
                [value, value.saturating_add(16), value.saturating_add(32)]
            })
            .collect();

        let file = File::create(&path).unwrap();
        let mut dng = DngWriter::new(file, DNG_VERSION_V1_4).unwrap();
        {
            let mut raw = dng.subframe_on_root(0);
            raw.rgb_image_u8(
                &pixels,
                w as usize,
                h as usize,
                DngCompression::Uncompressed,
                1,
            )
            .unwrap();
            raw.finalize().unwrap();
        }
        dng.close().unwrap();

        path
    }

    fn create_test_raw_dng_with_full_image(
        dir: &Path,
        name: &str,
        w: u32,
        h: u32,
        full_rgb: [u8; 3],
    ) -> PathBuf {
        let path = dir.join(name);
        let pixels: Vec<u8> = std::iter::repeat(full_rgb)
            .take((w * h) as usize)
            .flat_map(|rgb| rgb)
            .collect();

        let file = File::create(&path).unwrap();
        let mut dng = DngWriter::new(file, DNG_VERSION_V1_4).unwrap();
        {
            let mut raw = dng.subframe_on_root(0);
            raw.rgb_image_u8(
                &pixels,
                w as usize,
                h as usize,
                DngCompression::Uncompressed,
                1,
            )
            .unwrap();
            raw.finalize().unwrap();
        }
        dng.close().unwrap();

        path
    }

    fn create_test_raw_dng_with_preview(
        dir: &Path,
        name: &str,
        w: u32,
        h: u32,
        raw_rgb: [u8; 3],
        thumbnail_rgb: [u8; 3],
        preview_rgb: [u8; 3],
    ) -> PathBuf {
        let path = dir.join(name);
        let pixels: Vec<u8> = std::iter::repeat(raw_rgb)
            .take((w * h) as usize)
            .flat_map(|rgb| rgb)
            .collect();
        let thumbnail = RawDynamicImage::ImageRgb8(image25::RgbImage::from_pixel(
            w,
            h,
            image25::Rgb(thumbnail_rgb),
        ));
        let preview = RawDynamicImage::ImageRgb8(image25::RgbImage::from_pixel(
            w,
            h,
            image25::Rgb(preview_rgb),
        ));

        let file = File::create(&path).unwrap();
        let mut dng = DngWriter::new(file, DNG_VERSION_V1_4).unwrap();
        {
            let mut raw = dng.subframe_on_root(0);
            raw.rgb_image_u8(
                &pixels,
                w as usize,
                h as usize,
                DngCompression::Uncompressed,
                1,
            )
            .unwrap();
            raw.finalize().unwrap();
        }
        dng.thumbnail(&thumbnail).unwrap();
        {
            let mut preview_frame = dng.subframe(1);
            preview_frame.preview(&preview, 1.0).unwrap();
            preview_frame.finalize().unwrap();
        }
        dng.close().unwrap();

        path
    }

    fn create_test_raw_dng_with_thumbnail(
        dir: &Path,
        name: &str,
        w: u32,
        h: u32,
        raw_rgb: [u8; 3],
        thumbnail_rgb: [u8; 3],
    ) -> PathBuf {
        let path = dir.join(name);
        let pixels: Vec<u8> = std::iter::repeat(raw_rgb)
            .take((w * h) as usize)
            .flat_map(|rgb| rgb)
            .collect();
        let thumbnail = RawDynamicImage::ImageRgb8(image25::RgbImage::from_pixel(
            w,
            h,
            image25::Rgb(thumbnail_rgb),
        ));

        let file = File::create(&path).unwrap();
        let mut dng = DngWriter::new(file, DNG_VERSION_V1_4).unwrap();
        {
            let mut raw = dng.subframe_on_root(0);
            raw.rgb_image_u8(
                &pixels,
                w as usize,
                h as usize,
                DngCompression::Uncompressed,
                1,
            )
            .unwrap();
            raw.finalize().unwrap();
        }
        dng.thumbnail(&thumbnail).unwrap();
        dng.close().unwrap();

        path
    }

    fn assert_rgb_close(actual: &[u8], expected: [u8; 3], tolerance: u8) {
        for (channel, expected) in actual.iter().take(3).zip(expected) {
            assert!(
                (*channel as i16 - expected as i16).abs() <= tolerance as i16,
                "expected {expected} +/- {tolerance}, got {channel}"
            );
        }
    }

    fn create_test_svg(dir: &Path, name: &str) -> PathBuf {
        create_test_svg_with_fill(dir, name, "red")
    }

    fn create_test_svg_with_fill(dir: &Path, name: &str, fill: &str) -> PathBuf {
        let path = dir.join(name);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <rect width="200" height="100" fill="{fill}"/>
        </svg>"#
        );
        std::fs::write(&path, svg).unwrap();
        path
    }

    fn with_default_test_decode_cache_dir<T>(f: impl FnOnce() -> T) -> T {
        with_test_decode_cache_dir_override(None, f)
    }

    fn with_test_decode_cache_dir_override<T>(
        cache_dir: Option<Option<&Path>>,
        f: impl FnOnce() -> T,
    ) -> T {
        let _guard = TEST_DECODE_CACHE_DIR_GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let override_lock =
            TEST_DECODE_CACHE_DIR_OVERRIDE.get_or_init(|| std::sync::Mutex::new(Some(None)));
        let mut guard = override_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = guard.clone();
        *guard = cache_dir.map(|path| path.map(Path::to_path_buf));
        drop(guard);

        let result = f();

        *override_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = previous;
        result
    }

    fn with_test_photo_repo_root<T>(repo_root: Option<Option<&Path>>, f: impl FnOnce() -> T) -> T {
        let _guard = TEST_PHOTO_REPO_ROOT_GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let override_lock =
            TEST_PHOTO_REPO_ROOT_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
        let mut guard = override_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = guard.clone();
        *guard = repo_root.map(|path| path.map(Path::to_path_buf));
        drop(guard);

        let result = f();

        *override_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = previous;
        result
    }

    #[test]
    fn decode_png_returns_correct_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "test.png", 64, 48);

        let result = decode_image(&path).unwrap();
        assert_eq!(result.width, 64);
        assert_eq!(result.height, 48);
    }

    #[test]
    fn source_dimensions_report_raw_container_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng(dir.path(), "source.dng", 24, 12);

        let dimensions = source_dimensions(&path).unwrap();

        assert_eq!(dimensions, (24, 12));
    }

    #[test]
    fn source_dimensions_report_png_file_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "source.png", 64, 48);

        let dimensions = source_dimensions(&path).unwrap();

        assert_eq!(dimensions, (64, 48));
    }

    #[test]
    fn source_dimensions_report_svg_intrinsic_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_svg(dir.path(), "source.svg");

        let dimensions = source_dimensions(&path).unwrap();

        assert_eq!(dimensions, (200, 100));
    }

    #[test]
    fn source_dimensions_return_an_error_for_missing_files() {
        let result = source_dimensions(Path::new("/no/such/file.png"));

        assert!(result.is_err());
    }

    #[test]
    fn decode_png_returns_rgba_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "test.png", 4, 4);

        let result = decode_image(&path).unwrap();
        // 4 * 4 pixels * 4 channels (RGBA) = 64 bytes
        assert_eq!(result.pixels.len(), 4 * 4 * 4);
        // Every pixel has alpha = 255
        for chunk in result.pixels.chunks(4) {
            assert_eq!(chunk[3], 255);
        }
    }

    #[test]
    fn decode_captures_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "test.png", 8, 8);

        let result = decode_image(&path).unwrap();
        let actual_size = std::fs::metadata(&path).unwrap().len();
        assert_eq!(result.file_size, actual_size);
        assert!(result.file_size > 0);
    }

    #[test]
    fn decode_svg_renders_to_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_svg(dir.path(), "test.svg");

        let result = decode_image(&path).unwrap();
        assert_eq!(result.width, 200);
        assert_eq!(result.height, 100);
        assert_eq!(result.pixels.len(), (200 * 100 * 4) as usize);
    }

    #[test]
    fn decode_invalid_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbage.png");
        std::fs::write(&path, b"not an image").unwrap();

        let result = decode_image(&path);
        assert!(result.is_err());
    }

    #[test]
    fn decode_nonexistent_file_returns_error() {
        let result = decode_image(Path::new("/no/such/file.png"));
        assert!(result.is_err());
    }

    #[test]
    fn decode_bmp_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bmp");
        let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 255, 255]));
        img.save(&path).unwrap();

        let result = decode_image(&path).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 10);
    }

    #[test]
    fn thumbnail_jpeg_uses_fast_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let img = image::RgbaImage::from_fn(800, 600, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        img.save(&path).unwrap();

        let result = decode_thumbnail(&path, 200).unwrap();
        assert!(result.width <= 200);
        assert!(result.height <= 200);
        assert!(result.width > 0 && result.height > 0);
    }

    #[test]
    fn thumbnail_respects_max_dim() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "big.png", 800, 600);

        let result = decode_thumbnail(&path, 200).unwrap();
        assert!(result.width <= 200);
        assert!(result.height <= 200);
        // Aspect ratio preserved: 800x600 -> 200x150
        assert_eq!(result.width, 200);
        assert_eq!(result.height, 150);
    }

    #[test]
    fn thumbnail_preserves_small_images() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "small.png", 50, 30);

        let result = decode_thumbnail(&path, 200).unwrap();
        assert_eq!(result.width, 50);
        assert_eq!(result.height, 30);
    }

    #[test]
    fn thumbnail_preserves_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_png(dir.path(), "test.png", 400, 300);

        let result = decode_thumbnail(&path, 100).unwrap();
        let actual_size = std::fs::metadata(&path).unwrap().len();
        assert_eq!(result.file_size, actual_size);
    }

    #[test]
    fn decode_raw_dng_returns_rgba_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng(dir.path(), "test.dng", 24, 12);

        let result = decode_image(&path).unwrap();
        assert!(result.width > 0);
        assert!(result.height > 0);
        assert!(result.width <= MAX_TEXTURE_DIM);
        assert!(result.height <= MAX_TEXTURE_DIM);
        assert_eq!(
            result.pixels.len(),
            (result.width * result.height * 4) as usize
        );
    }

    #[test]
    fn decode_raw_dng_thumbnail_respects_max_dim() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng(dir.path(), "thumb.dng", 24, 12);

        let result = decode_thumbnail(&path, 10).unwrap();
        assert!(result.width <= 10);
        assert!(result.height <= 10);
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
    }

    #[test]
    fn decode_raw_thumbnail_prefers_embedded_preview_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng_with_preview(
            dir.path(),
            "preview-thumb.dng",
            24,
            12,
            [16, 96, 32],
            [30, 180, 220],
            [220, 40, 60],
        );

        let result = decode_thumbnail(&path, 10).unwrap();
        assert_eq!(result.width, 10);
        assert_eq!(result.height, 5);
        assert_rgb_close(&result.pixels, [30, 180, 220], 12);
    }

    #[test]
    fn decode_raw_image_falls_back_to_embedded_image_when_raw_pixels_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng_with_preview(
            dir.path(),
            "preview-full.dng",
            24,
            12,
            [16, 96, 32],
            [30, 180, 220],
            [220, 40, 60],
        );

        let result = decode_image(&path).unwrap();
        assert!(result.width > 0);
        assert!(result.height > 0);
        assert!(result.width <= MAX_TEXTURE_DIM);
        assert!(result.height <= MAX_TEXTURE_DIM);
        assert_rgb_close(&result.pixels, [220, 40, 60], 12);
    }

    #[test]
    fn decode_raw_image_falls_back_to_full_image_when_raw_pixels_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng_with_full_image(
            dir.path(),
            "full-only.dng",
            24,
            12,
            [90, 140, 210],
        );

        let result = decode_image(&path).unwrap();
        assert!(result.width > 0);
        assert!(result.height > 0);
        assert!(result.width <= MAX_TEXTURE_DIM);
        assert!(result.height <= MAX_TEXTURE_DIM);
        assert_eq!(
            result.pixels.len(),
            (result.width * result.height * 4) as usize
        );
    }

    #[test]
    fn decode_raw_image_falls_back_to_embedded_thumbnail_when_needed() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng_with_thumbnail(
            dir.path(),
            "thumb-only.dng",
            24,
            12,
            [16, 96, 32],
            [30, 180, 220],
        );

        let result = decode_image(&path).unwrap();
        assert!(result.width > 0);
        assert!(result.height > 0);
        assert!(result.width <= MAX_TEXTURE_DIM);
        assert!(result.height <= MAX_TEXTURE_DIM);
        assert_rgb_close(&result.pixels, [30, 180, 220], 12);
    }

    #[test]
    fn decode_embedded_preview_returns_none_when_raw_has_no_embedded_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_test_raw_dng(dir.path(), "raw-only.dng", 6, 4);

        let result = decode_embedded_preview(&path).unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn decode_embedded_preview_falls_back_to_preview_then_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let preview_path = create_test_raw_dng_with_preview(
            dir.path(),
            "preview-only.dng",
            8,
            4,
            [1, 2, 3],
            [10, 20, 30],
            [200, 150, 100],
        );
        let thumbnail_path = create_test_raw_dng_with_thumbnail(
            dir.path(),
            "thumbnail-only.dng",
            5,
            5,
            [1, 2, 3],
            [90, 45, 180],
        );

        let preview = decode_embedded_preview(&preview_path).unwrap().unwrap();
        let thumbnail = decode_embedded_preview(&thumbnail_path).unwrap().unwrap();

        assert_rgb_close(&preview.pixels, [200, 150, 100], 2);
        assert_rgb_close(&thumbnail.pixels, [90, 45, 180], 2);
    }

    #[test]
    fn decode_malformed_raw_returns_contextual_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.dng");
        std::fs::write(&path, b"not a real raw file").unwrap();

        let error = decode_image(&path).unwrap_err();
        assert!(error.contains("RAW"), "unexpected error: {error}");
    }

    #[test]
    fn persisted_decode_cache_targets_svg_and_raw_detail_loads() {
        let dir = tempfile::tempdir().unwrap();
        let png = create_test_png(dir.path(), "frame.png", 8, 4);
        let svg = create_test_svg(dir.path(), "frame.svg");
        let svgz = dir.path().join("frame.svgz");
        std::fs::write(&svgz, b"svgz").unwrap();
        let raw = create_test_raw_dng(dir.path(), "frame.dng", 8, 4);

        assert!(!supports_persisted_decoded_cache(&png));
        assert!(supports_persisted_decoded_cache(&svg));
        assert!(supports_persisted_decoded_cache(&svgz));
        assert!(supports_persisted_decoded_cache(&raw));
    }

    #[test]
    fn decoded_cache_dir_targets_a_visible_repo_local_directory_when_repo_root_is_known() {
        let repo_root = tempfile::tempdir().unwrap();
        let svg = create_test_svg(repo_root.path(), "frame.svg");

        assert_eq!(
            decoded_cache_dir_for_repo_root(&svg, repo_root.path()),
            Some(repo_root.path().join(DECODE_CACHE_DIR_NAME))
        );
    }

    #[test]
    fn decoded_cache_dir_resolves_under_this_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        let svg = create_test_svg(dir.path(), "frame.svg");

        with_test_photo_repo_root(Some(Some(Path::new(env!("CARGO_MANIFEST_DIR")))), || {
            assert_eq!(
                decoded_cache_dir_for(&svg),
                Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DECODE_CACHE_DIR_NAME))
            );
        });
    }

    #[test]
    fn decode_image_uses_the_public_repo_local_cache_entrypoint() {
        let repo_root = tempfile::tempdir().unwrap();
        std::fs::write(repo_root.path().join("AGENTS.md"), "test repo").unwrap();
        std::fs::write(
            repo_root.path().join("Cargo.toml"),
            "[package]\nname = \"photo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(repo_root.path().join(".git")).unwrap();
        std::fs::create_dir(repo_root.path().join("src")).unwrap();
        std::fs::write(repo_root.path().join("src").join("decode.rs"), "// marker").unwrap();
        let svg = create_test_svg(repo_root.path(), "frame.svg");
        let fingerprint = SourceFingerprint::from_path(&svg).unwrap();
        let cache_dir = repo_root.path().join(DECODE_CACHE_DIR_NAME);
        let cache_file = decoded_cache_file_path(&cache_dir, &fingerprint.path_key);
        let _ = std::fs::remove_file(&cache_file);

        with_default_test_decode_cache_dir(|| {
            with_test_photo_repo_root(Some(Some(repo_root.path())), || {
                let first = decode_image(&svg).unwrap();
                assert!(cache_file.exists());
                let first_cache_modified =
                    std::fs::metadata(&cache_file).unwrap().modified().unwrap();

                let second = decode_image(&svg).unwrap();
                let second_cache_modified =
                    std::fs::metadata(&cache_file).unwrap().modified().unwrap();

                assert_eq!(first.width, second.width);
                assert_eq!(first.height, second.height);
                assert_eq!(first.pixels, second.pixels);
                assert_eq!(first_cache_modified, second_cache_modified);
            });
        });
    }

    #[test]
    fn decode_image_reuses_the_public_repo_local_raw_cache_entrypoint() {
        let repo_root = tempfile::tempdir().unwrap();
        std::fs::write(repo_root.path().join("AGENTS.md"), "test repo").unwrap();
        std::fs::write(
            repo_root.path().join("Cargo.toml"),
            "[package]\nname = \"photo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(repo_root.path().join(".git")).unwrap();
        std::fs::create_dir(repo_root.path().join("src")).unwrap();
        std::fs::write(repo_root.path().join("src").join("decode.rs"), "// marker").unwrap();
        let raw = create_test_raw_dng(repo_root.path(), "frame.dng", 8, 4);
        let fingerprint = SourceFingerprint::from_path(&raw).unwrap();
        let cache_dir = repo_root.path().join(DECODE_CACHE_DIR_NAME);
        let cache_file = decoded_cache_file_path(&cache_dir, &fingerprint.path_key);
        let _ = std::fs::remove_file(&cache_file);

        with_default_test_decode_cache_dir(|| {
            with_test_photo_repo_root(Some(Some(repo_root.path())), || {
                let first = decode_image(&raw).unwrap();
                assert!(cache_file.exists());
                let first_cache_modified =
                    std::fs::metadata(&cache_file).unwrap().modified().unwrap();

                let second = decode_image(&raw).unwrap();
                let second_cache_modified =
                    std::fs::metadata(&cache_file).unwrap().modified().unwrap();

                assert_eq!(first.width, second.width);
                assert_eq!(first.height, second.height);
                assert_eq!(first.pixels, second.pixels);
                assert_eq!(first_cache_modified, second_cache_modified);
            });
        });
    }

    #[test]
    fn decode_image_skips_persisted_cache_when_repo_root_discovery_is_forced_off() {
        let dir = tempfile::tempdir().unwrap();
        let svg = create_test_svg(dir.path(), "frame.svg");
        let repo_local_cache = dir.path().join(DECODE_CACHE_DIR_NAME);
        let _ = std::fs::remove_dir_all(&repo_local_cache);

        with_default_test_decode_cache_dir(|| {
            with_test_photo_repo_root(Some(None), || {
                let result = decode_image(&svg).unwrap();

                assert_eq!(result.width, 200);
                assert_eq!(result.height, 100);
                assert!(!repo_local_cache.exists());
            });
        });
    }

    #[test]
    fn warm_persisted_decoded_cache_populates_the_public_repo_local_cache() {
        let repo_root = tempfile::tempdir().unwrap();
        std::fs::write(repo_root.path().join("AGENTS.md"), "test repo").unwrap();
        std::fs::write(
            repo_root.path().join("Cargo.toml"),
            "[package]\nname = \"photo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(repo_root.path().join(".git")).unwrap();
        std::fs::create_dir(repo_root.path().join("src")).unwrap();
        std::fs::write(repo_root.path().join("src").join("decode.rs"), "// marker").unwrap();
        let svg = create_test_svg(repo_root.path(), "frame.svg");
        let fingerprint = SourceFingerprint::from_path(&svg).unwrap();
        let cache_dir = repo_root.path().join(DECODE_CACHE_DIR_NAME);
        let cache_file = decoded_cache_file_path(&cache_dir, &fingerprint.path_key);
        let _ = std::fs::remove_file(&cache_file);

        with_default_test_decode_cache_dir(|| {
            with_test_photo_repo_root(Some(Some(repo_root.path())), || {
                assert_eq!(warm_persisted_decoded_cache(&svg).unwrap(), true);
                assert!(cache_file.exists());
            });
        });
    }

    #[test]
    fn warm_persisted_decoded_cache_skips_unsupported_formats() {
        let dir = tempfile::tempdir().unwrap();
        let png = create_test_png(dir.path(), "frame.png", 8, 4);

        assert_eq!(warm_persisted_decoded_cache(&png).unwrap(), false);
    }

    #[test]
    fn find_photo_repo_root_ignores_other_rust_repositories() {
        let repo_root = tempfile::tempdir().unwrap();
        let nested = repo_root.path().join("target").join("debug");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            repo_root.path().join("Cargo.toml"),
            "[package]\nname = \"not-photo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(repo_root.path().join(".git")).unwrap();

        assert_eq!(find_photo_repo_root(&nested), None);
    }

    #[test]
    fn decode_image_with_cache_dir_creates_and_reuses_the_persisted_svg_cache() {
        let dir = tempfile::tempdir().unwrap();
        let svg = create_test_svg(dir.path(), "frame.svg");
        let cache_dir = tempfile::tempdir().unwrap();
        let fingerprint = SourceFingerprint::from_path(&svg).unwrap();
        let cache_file = decoded_cache_file_path(cache_dir.path(), &fingerprint.path_key);
        let _ = std::fs::remove_file(&cache_file);

        let first = decode_image_with_cache_dir(&svg, Some(cache_dir.path())).unwrap();
        assert!(cache_file.exists());
        let first_cache_modified = std::fs::metadata(&cache_file).unwrap().modified().unwrap();

        let second = decode_image_with_cache_dir(&svg, Some(cache_dir.path())).unwrap();
        let second_cache_modified = std::fs::metadata(&cache_file).unwrap().modified().unwrap();

        assert_eq!(first.width, second.width);
        assert_eq!(first.height, second.height);
        assert_eq!(first.pixels, second.pixels);
        assert_eq!(first_cache_modified, second_cache_modified);

        let _ = std::fs::remove_file(&cache_file);
    }

    #[test]
    fn cached_full_image_reuses_decoded_pixels_when_source_is_unchanged() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("frame.svg");
        std::fs::write(&source, b"<svg />").unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let decode_calls = AtomicUsize::new(0);

        let first = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![1, 2, 3, 255], 1, 1))
        })
        .unwrap();

        let fingerprint = SourceFingerprint::from_path(&source).unwrap();
        let cache_file = decoded_cache_file_path(cache_dir.path(), &fingerprint.path_key);
        assert!(cache_file.exists());

        let second = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![9, 8, 7, 255], 1, 1))
        })
        .unwrap();

        assert_eq!(decode_calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.pixels, vec![1, 2, 3, 255]);
        assert_eq!(second.pixels, first.pixels);
        assert_eq!(second.file_size, std::fs::metadata(&source).unwrap().len());
    }

    #[test]
    fn cached_full_image_redocodes_when_source_file_changes() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("frame.svg");
        std::fs::write(&source, b"<svg>A</svg>").unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let decode_calls = AtomicUsize::new(0);

        let first = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![1, 2, 3, 255], 1, 1))
        })
        .unwrap();

        std::fs::write(&source, b"<svg>B</svg>").unwrap();

        let second = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![9, 8, 7, 255], 1, 1))
        })
        .unwrap();

        let third = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![5, 5, 5, 255], 1, 1))
        })
        .unwrap();

        assert_eq!(decode_calls.load(Ordering::SeqCst), 2);
        assert_eq!(first.pixels, vec![1, 2, 3, 255]);
        assert_eq!(second.pixels, vec![9, 8, 7, 255]);
        assert_eq!(third.pixels, second.pixels);
    }

    #[test]
    fn cached_full_image_redocodes_when_cache_contract_changes() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("frame.svg");
        std::fs::write(&source, b"<svg />").unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let decode_calls = AtomicUsize::new(0);

        let first = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![1, 2, 3, 255], 1, 1))
        })
        .unwrap();

        let fingerprint = SourceFingerprint::from_path(&source).unwrap();
        let cache_file = decoded_cache_file_path(cache_dir.path(), &fingerprint.path_key);
        let mut bytes = std::fs::read(&cache_file).unwrap();
        let contract_offset = DECODE_CACHE_MAGIC.len() + std::mem::size_of::<u32>();
        bytes[contract_offset..contract_offset + 8].copy_from_slice(&0u64.to_le_bytes());
        std::fs::write(&cache_file, bytes).unwrap();

        let second = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![9, 8, 7, 255], 1, 1))
        })
        .unwrap();

        assert_eq!(decode_calls.load(Ordering::SeqCst), 2);
        assert_eq!(first.pixels, vec![1, 2, 3, 255]);
        assert_eq!(second.pixels, vec![9, 8, 7, 255]);
    }

    #[test]
    fn decoded_cache_temp_paths_are_unique_per_write_attempt() {
        let cache_path = Path::new("cache-file.rgba");
        let first = decoded_cache_temp_file_path(cache_path);
        let second = decoded_cache_temp_file_path(cache_path);

        assert_ne!(first, second);
        assert!(first.to_string_lossy().contains(".tmp-"));
        assert!(second.to_string_lossy().contains(".tmp-"));
    }

    #[test]
    fn source_fingerprint_normalizes_equivalent_paths() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let source = create_test_svg(&nested, "frame.svg");
        let alternate = nested.join("..").join("nested").join("frame.svg");

        let direct = SourceFingerprint::from_path(&source).unwrap();
        let alternate = SourceFingerprint::from_path(&alternate).unwrap();

        assert_eq!(direct.path_key, alternate.path_key);
        assert_eq!(direct.file_size, alternate.file_size);
        assert_eq!(direct.modified_secs, alternate.modified_secs);
        assert_eq!(direct.modified_nanos, alternate.modified_nanos);
    }

    #[test]
    fn source_fingerprint_changes_when_same_size_contents_change() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("frame.svg");
        std::fs::write(&source, b"<svg>A</svg>").unwrap();
        let first = SourceFingerprint::from_path(&source).unwrap();
        let first_hash = source_content_hash(&source, first.file_size).unwrap();

        std::fs::write(&source, b"<svg>B</svg>").unwrap();
        let second = SourceFingerprint::from_path(&source).unwrap();
        let second_hash = source_content_hash(&source, second.file_size).unwrap();

        assert_eq!(first.path_key, second.path_key);
        assert_eq!(first.file_size, second.file_size);
        assert_ne!(first_hash, second_hash);
    }

    #[test]
    fn write_decoded_cache_removes_temp_file_when_finalize_fails() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = create_test_svg(source_dir.path(), "frame.svg");
        let cache_dir = tempfile::tempdir().unwrap();
        let fingerprint = SourceFingerprint::from_path(&source).unwrap();
        let cache_path = decoded_cache_file_path(cache_dir.path(), &fingerprint.path_key);
        std::fs::create_dir(&cache_path).unwrap();

        let content_hash = source_content_hash(&source, fingerprint.file_size).unwrap();
        let error = write_decoded_cache(
            cache_dir.path(),
            &fingerprint,
            content_hash,
            1,
            1,
            &[1, 2, 3, 4],
        )
        .expect_err("finalize failure should surface");

        assert!(error.contains("Failed to finalize cache file"));

        let leftover_temp_entries = std::fs::read_dir(cache_dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.starts_with("tmp-"))
            })
            .collect::<Vec<_>>();
        assert!(leftover_temp_entries.is_empty());
    }

    #[test]
    fn corrupt_cached_full_image_falls_back_to_a_fresh_decode() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = create_test_svg_with_fill(source_dir.path(), "frame.svg", "red");
        let cache_dir = tempfile::tempdir().unwrap();

        let first = decode_image_with_cache_dir(&source, Some(cache_dir.path())).unwrap();
        let fingerprint = SourceFingerprint::from_path(&source).unwrap();
        let cache_file = decoded_cache_file_path(cache_dir.path(), &fingerprint.path_key);
        std::fs::write(&cache_file, DECODE_CACHE_MAGIC).unwrap();
        create_test_svg_with_fill(source_dir.path(), "frame.svg", "blue");

        let second = decode_image_with_cache_dir(&source, Some(cache_dir.path())).unwrap();

        assert_eq!(first.width, second.width);
        assert_eq!(first.height, second.height);
        assert_ne!(first.pixels, second.pixels);
    }

    #[test]
    fn invalid_cache_schema_falls_back_to_a_fresh_decode() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("frame.svg");
        std::fs::write(&source, b"<svg />").unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let decode_calls = AtomicUsize::new(0);

        let first = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![1, 2, 3, 255], 1, 1))
        })
        .unwrap();

        let fingerprint = SourceFingerprint::from_path(&source).unwrap();
        let cache_file = decoded_cache_file_path(cache_dir.path(), &fingerprint.path_key);
        let mut bytes = std::fs::read(&cache_file).unwrap();
        let schema_offset = DECODE_CACHE_MAGIC.len();
        bytes[schema_offset..schema_offset + std::mem::size_of::<u32>()]
            .copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(&cache_file, bytes).unwrap();

        let second = load_or_decode_cached_full_image(&source, Some(cache_dir.path()), |_| {
            decode_calls.fetch_add(1, Ordering::SeqCst);
            Ok((vec![9, 8, 7, 255], 1, 1))
        })
        .unwrap();

        assert_eq!(decode_calls.load(Ordering::SeqCst), 2);
        assert_eq!(first.pixels, vec![1, 2, 3, 255]);
        assert_eq!(second.pixels, vec![9, 8, 7, 255]);
    }

    #[test]
    fn decode_cache_prunes_oldest_entries_when_budget_is_exceeded() {
        let source_dir = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let source_a = source_dir.path().join("a.svg");
        let source_b = source_dir.path().join("b.svg");
        let source_c = source_dir.path().join("c.svg");
        std::fs::write(&source_a, b"<svg>a</svg>").unwrap();
        std::fs::write(&source_b, b"<svg>bb</svg>").unwrap();
        std::fs::write(&source_c, b"<svg>ccc</svg>").unwrap();

        let fingerprint_a = SourceFingerprint::from_path(&source_a).unwrap();
        let fingerprint_b = SourceFingerprint::from_path(&source_b).unwrap();
        let fingerprint_c = SourceFingerprint::from_path(&source_c).unwrap();

        write_decoded_cache(
            cache_dir.path(),
            &fingerprint_a,
            source_content_hash(&source_a, fingerprint_a.file_size).unwrap(),
            2,
            2,
            &[1; 16],
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        write_decoded_cache(
            cache_dir.path(),
            &fingerprint_b,
            source_content_hash(&source_b, fingerprint_b.file_size).unwrap(),
            2,
            2,
            &[2; 16],
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        write_decoded_cache(
            cache_dir.path(),
            &fingerprint_c,
            source_content_hash(&source_c, fingerprint_c.file_size).unwrap(),
            2,
            2,
            &[3; 16],
        )
        .unwrap();

        let cache_a = decoded_cache_file_path(cache_dir.path(), &fingerprint_a.path_key);
        let cache_b = decoded_cache_file_path(cache_dir.path(), &fingerprint_b.path_key);
        let cache_c = decoded_cache_file_path(cache_dir.path(), &fingerprint_c.path_key);
        let size_a = std::fs::metadata(&cache_a).unwrap().len();
        let size_b = std::fs::metadata(&cache_b).unwrap().len();
        let size_c = std::fs::metadata(&cache_c).unwrap().len();
        let total = size_a + size_b + size_c;

        prune_decoded_cache(cache_dir.path(), total - 1, size_b + size_c).unwrap();

        assert!(!cache_a.exists());
        assert!(cache_b.exists());
        assert!(cache_c.exists());
    }

    #[test]
    fn write_decoded_cache_triggers_periodic_pruning() {
        let cache_dir = tempfile::tempdir().unwrap();
        let source = create_test_svg(cache_dir.path(), "frame.svg");
        let fingerprint = SourceFingerprint::from_path(&source).unwrap();
        let stale_path = cache_dir.path().join("stale.rgba");
        File::create(&stale_path)
            .unwrap()
            .set_len(DECODE_CACHE_MAX_BYTES + 1)
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        for _ in 0..DECODE_CACHE_PRUNE_WRITE_INTERVAL {
            write_decoded_cache(
                cache_dir.path(),
                &fingerprint,
                source_content_hash(&source, fingerprint.file_size).unwrap(),
                2,
                2,
                &[3; 16],
            )
            .unwrap();
            if !stale_path.exists() {
                break;
            }
        }
        assert!(!stale_path.exists());
    }
}
