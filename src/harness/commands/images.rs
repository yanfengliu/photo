//! Pixels: the frame the user sees, statistics of any image on disk, the
//! difference between two images, and the develop pipeline's own render.

use std::borrow::Cow;
use std::path::Path;
use std::time::Instant;

use image::RgbaImage;
use photo_engine::{EngineError, StillSettings};
use photo_ui::harness::RenderError;
use serde_json::{Value, json};

use super::{Session, performed};
use crate::harness::protocol::{CmdError, Code, Params};
use crate::harness::stats;

/// `screenshot`: render the frame the user would see, at the harness's
/// pixel density, and save it as a PNG artifact.
pub fn screenshot(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&[])?;
    // Bring what is shown up to date first, as the window would have.
    let warnings = session.driver.settle();
    let frame = session.driver.harness().render().map_err(render_error)?;
    let (absolute, relative) = session.run.next_artifact("screenshot");
    frame
        .save_with_format(&absolute, image::ImageFormat::Png)
        .map_err(|error| CmdError::io(format!("cannot write {}: {error}", absolute.display())))?;
    let look = session.driver.look();
    session.run.add_artifact(json!({
        "path": relative,
        "kind": "screenshot",
        "request_id": session.request_id,
        "width": frame.width(),
        "height": frame.height(),
    }));
    Ok(performed(
        json!({
            "path": relative,
            "absolute_path": absolute,
            "width": frame.width(),
            "height": frame.height(),
            "pixels_per_point": look.pixels_per_point,
            "logical_size": look.size,
        }),
        warnings,
    ))
}

fn render_error(error: RenderError) -> CmdError {
    match error {
        RenderError::NoAdapter(_) => CmdError::unavailable(error.to_string()),
        RenderError::Failed(_) => CmdError::internal(error.to_string()),
        RenderError::Save(_) => CmdError::io(error.to_string()),
    }
}

/// `image_stats {path}`: statistics of a PNG or JPEG on disk; a relative
/// path is taken from the run directory.
pub fn image_stats(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["path"])?;
    let path = session.run.resolve(params.string("path")?);
    let image = load(&path)?;
    let mut data = stats::image_stats(&image)
        .map_err(CmdError::invalid)?
        .to_json();
    data["path"] = json!(path);
    Ok(data)
}

/// `compare_images {path_a, path_b}`: how two images of one size differ.
pub fn compare_images(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["path_a", "path_b"])?;
    let path_a = session.run.resolve(params.string("path_a")?);
    let path_b = session.run.resolve(params.string("path_b")?);
    let (a, b) = (load(&path_a)?, load(&path_b)?);
    let mut data = stats::compare(&a, &b).map_err(CmdError::invalid)?.to_json();
    data["path_a"] = json!(path_a);
    data["path_b"] = json!(path_b);
    Ok(data)
}

/// An image file, decoded to 8-bit RGBA.
fn load(path: &Path) -> Result<RgbaImage, CmdError> {
    if !path.is_file() {
        return Err(CmdError::not_found(format!(
            "there is no file at {}",
            path.display()
        )));
    }
    let decoded = image::open(path).map_err(|error| match error {
        image::ImageError::IoError(error) => {
            CmdError::io(format!("cannot read {}: {error}", path.display()))
        }
        image::ImageError::Unsupported(error) => CmdError::new(
            Code::Unsupported,
            format!(
                "{} is not an image this harness reads: {error}",
                path.display()
            ),
        ),
        error => CmdError::invalid(format!("{} cannot be decoded: {error}", path.display())),
    })?;
    Ok(decoded.to_rgba8())
}

/// The longest side of a render's PNG unless `max_dim` says otherwise: a
/// standard preview's.
const DEFAULT_MAX_DIM: u64 = 2048;

/// `dump_render {source, max_dim}`: the active photo through the develop
/// pipeline at full resolution, with the photo's settings (`current`) or
/// the defaults for its kind (`original`). The statistics are of the full
/// render, in the 8-bit sRGB encoding the user sees, as `image_stats`
/// measures any image; the PNG artifact is scaled down to `max_dim`.
///
/// The render runs on the engine's caller, so the session answers nothing
/// else until it is done.
pub fn dump_render(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["source", "max_dim"])?;
    let source = params.opt_string("source")?.unwrap_or("current");
    let settings = match source {
        "current" => StillSettings::Current,
        "original" => StillSettings::Default,
        other => {
            return Err(CmdError::invalid(format!(
                "`source` is `current` (the photo's settings) or `original` (the defaults), not `{other}`"
            )));
        }
    };
    let max_dim = params.opt_u64("max_dim")?.unwrap_or(DEFAULT_MAX_DIM);
    if !(16..=16_384).contains(&max_dim) {
        return Err(CmdError::invalid("`max_dim` is from 16 to 16384 pixels"));
    }
    let engine = session.driver.engine();
    let Some(photo) = engine.active_photo() else {
        return Err(CmdError::unavailable("no photo is active"));
    };
    let file_name = engine
        .summary(photo)
        .map(|summary| summary.file_name.clone());
    let started = Instant::now();
    let rendered =
        engine
            .render_photo_rgba8(photo, settings, None)
            .map_err(|error| match error {
                EngineError::Render(reason) => CmdError::io(format!("the render failed: {reason}")),
                other => CmdError::internal(format!("the render failed: {other}")),
            })?;
    let render_ms = started.elapsed().as_millis() as u64;
    let (width, height) = (rendered.width, rendered.height);
    let full = RgbaImage::from_raw(width, height, rendered.pixels)
        .ok_or_else(|| CmdError::internal("the render's pixels do not fill its size"))?;
    let measured = stats::image_stats(&full).map_err(CmdError::internal)?;
    let shown = scaled_to(&full, max_dim as u32);
    let (absolute, relative) = session.run.next_artifact(&format!("render-{source}"));
    shown
        .save_with_format(&absolute, image::ImageFormat::Png)
        .map_err(|error| CmdError::io(format!("cannot write {}: {error}", absolute.display())))?;
    session.run.add_artifact(json!({
        "path": relative,
        "kind": "render",
        "source": source,
        "request_id": session.request_id,
        "width": shown.width(),
        "height": shown.height(),
        "render_size": [width, height],
    }));
    Ok(json!({
        "source": source,
        "photo": { "id": photo.0, "file_name": file_name },
        "render": { "width": width, "height": height, "render_ms": render_ms },
        "path": relative,
        "absolute_path": absolute,
        "artifact": { "width": shown.width(), "height": shown.height(), "max_dim": max_dim },
        "stats": measured.to_json(),
    }))
}

/// `image` with its longer side at most `max_dim` pixels: itself when it
/// fits, never enlarged.
fn scaled_to(image: &RgbaImage, max_dim: u32) -> Cow<'_, RgbaImage> {
    let longer = image.width().max(image.height());
    if longer <= max_dim {
        return Cow::Borrowed(image);
    }
    let scale = f64::from(max_dim) / f64::from(longer);
    let side = |length: u32| ((f64::from(length) * scale).round() as u32).clamp(1, max_dim);
    Cow::Owned(image::imageops::resize(
        image,
        side(image.width()),
        side(image.height()),
        image::imageops::FilterType::Triangle,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_is_scaled_down_to_fit_and_never_enlarged() {
        let wide = RgbaImage::new(400, 100);
        let scaled = scaled_to(&wide, 64);
        assert_eq!(scaled.dimensions(), (64, 16));
        let small = RgbaImage::new(30, 20);
        assert!(matches!(scaled_to(&small, 64), Cow::Borrowed(_)));
        let tall = RgbaImage::new(10, 1000);
        assert_eq!(scaled_to(&tall, 100).dimensions(), (1, 100));
    }
}
