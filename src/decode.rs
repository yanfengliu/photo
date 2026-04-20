use std::path::Path;
use std::sync::Arc;

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
    let decoder =
        rawler::get_decoder(&rawfile).map_err(|e| format!("Failed to initialize RAW decoder: {e}"))?;
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
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

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

    Ok(Arc::new(ImageData {
        pixels,
        width,
        height,
        file_size,
    }))
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

fn decode_svg(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read SVG: {e}"))?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| format!("Failed to parse SVG: {e}"))?;

    let size = tree.size();
    let mut w = size.width() as u32;
    let mut h = size.height() as u32;

    // Clamp to reasonable size
    if w == 0 || h == 0 {
        return Err("SVG has zero dimensions".to_string());
    }
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
    let data = std::fs::read(path).map_err(|e| format!("Failed to read SVG: {e}"))?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .map_err(|e| format!("Failed to parse SVG: {e}"))?;

    let size = tree.size();
    let orig_w = size.width() as u32;
    let orig_h = size.height() as u32;
    if orig_w == 0 || orig_h == 0 {
        return Err("SVG has zero dimensions".to_string());
    }

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

        let mut take_embedded_image = |kind| {
            match decode_embedded_image_kind(decoder, rawfile, params, max_dim, kind) {
                Ok(Some(image)) => Some(image),
                Ok(None) => None,
                Err(e) => {
                    last_optional_error = Some(e);
                    None
                }
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
        let path = dir.join(name);
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <rect width="200" height="100" fill="red"/>
        </svg>"#;
        std::fs::write(&path, svg).unwrap();
        path
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
}
