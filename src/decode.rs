use std::path::Path;
use std::sync::Arc;

/// Decoded image in RGBA8 format ready for GPU upload.
#[derive(Debug)]
pub struct ImageData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
}

const MAX_TEXTURE_DIM: u32 = 16384;

pub fn decode_image(path: &Path) -> Result<Arc<ImageData>, String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let (pixels, width, height) = match ext.as_str() {
        "svg" | "svgz" => decode_svg(path)?,
        _ => decode_raster(path)?,
    };

    Ok(Arc::new(ImageData {
        pixels,
        width,
        height,
        file_size,
    }))
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
        let resized =
            image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Lanczos3);
        let (rw, rh) = resized.dimensions();
        return Ok((resized.into_raw(), rw, rh));
    }

    Ok((rgba.into_raw(), w, h))
}

/// Visible for testing.
pub fn decode_raster_raw(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    decode_raster(path)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let path = dir.join(name);
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        img.save(&path).unwrap();
        path
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
}
