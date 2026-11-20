//! Pixels: the frame the user sees, statistics of any image on disk, the
//! difference between two images, and the develop pipeline's own render.

use std::path::Path;

use image::RgbaImage;
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

/// `dump_render {source, max_dim}`: the active photo through the develop
/// pipeline at full resolution.
///
/// Waiting on the engine: it has no full-resolution render the harness can
/// call. The app only lends the harness `&Engine` (`PhotoApp::engine`), so
/// the call has to take `&self`, or hand back work the harness can run or
/// wait for.
pub fn dump_render(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["source", "max_dim"])?;
    let source = params.opt_string("source")?.unwrap_or("current");
    if !matches!(source, "current" | "original") {
        return Err(CmdError::invalid(format!(
            "`source` is `current` (the photo's settings) or `original` (the defaults), not `{source}`"
        )));
    }
    if let Some(max_dim) = params.opt_u64("max_dim")?
        && !(16..=16_384).contains(&max_dim)
    {
        return Err(CmdError::invalid("`max_dim` is from 16 to 16384 pixels"));
    }
    if session.driver.engine().active_photo().is_none() {
        return Err(CmdError::unavailable("no photo is active"));
    }
    Err(CmdError::new(
        Code::Unsupported,
        "dump_render needs a full-resolution render from photo-engine that the harness can reach \
         through `&Engine` (for example `Engine::render_full(&self, photo, settings) -> \
         Result<image, EngineError>`); the engine has none yet",
    ))
}
