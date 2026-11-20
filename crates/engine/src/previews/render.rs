//! **The seam where previews get their pixels.**
//!
//! [`render`] is the one function that turns a photo and a size into
//! pixels. Everything around it (validating the cache against the source
//! file, writing the cache, the histogram, the memory cache, scheduling) is
//! independent of how the pixels are made.
//!
//! In this part of the engine the pixels come from the decoder's fast
//! preview: the camera's embedded JPEG for a RAW file, the image itself
//! scaled down for a raster file, turned to the user's orientation. The
//! develop pipeline replaces the body of [`render`] with a render of the
//! photo's [`DevelopSettings`] and reports [`PreviewOrigin::Rendered`];
//! nothing else has to change.

use std::io;
use std::path::Path;

use photo_core::develop::{DevelopSettings, Orientation};
use photo_core::image::Rgba8Image;
use photo_core::meta::FileKind;
use photo_decode::DecodeError;

use super::PreviewOrigin;

/// What a render needs to know about the photo.
pub(crate) struct RenderSource<'a> {
    /// The file (for a virtual copy, its master's file).
    pub(crate) path: &'a Path,
    /// Whether it is a raster or a RAW file.
    pub(crate) kind: FileKind,
    /// The photo's develop settings, as stored when the job was queued.
    pub(crate) settings: &'a DevelopSettings,
}

/// Pixels for one photo, with where they came from.
pub(crate) struct Rendered {
    /// Opaque sRGB pixels, upright, longer edge at most the size asked for.
    pub(crate) image: Rgba8Image,
    pub(crate) origin: PreviewOrigin,
}

/// Why no pixels could be made.
#[derive(Debug)]
pub(crate) enum RenderError {
    /// The source file is not there.
    Missing,
    /// Anything else, as a sentence for the user.
    Failed(String),
}

/// Pixels for `source` with the longer edge at most `long_edge`.
///
/// This is the seam the develop pipeline plugs into; see the module docs.
pub(crate) fn render(source: &RenderSource<'_>, long_edge: u32) -> Result<Rendered, RenderError> {
    let embedded =
        photo_decode::embedded_preview(source.path, long_edge).map_err(|error| match error {
            DecodeError::Io(io) if io.kind() == io::ErrorKind::NotFound => RenderError::Missing,
            other => RenderError::Failed(other.to_string()),
        })?;
    let image = embedded.ok_or_else(|| {
        let reason = match source.kind {
            FileKind::Raw => "the file has no embedded preview",
            FileKind::Raster => "the file gave no picture",
        };
        RenderError::Failed(reason.to_string())
    })?;
    Ok(Rendered {
        image: orient(image, source.settings.orientation),
        origin: PreviewOrigin::EmbeddedPreview,
    })
}

/// `image` mirrored and turned as `orientation` says: the mirror first, then
/// the clockwise quarter turns, as [`Orientation::forward`] maps points.
pub(crate) fn orient(image: Rgba8Image, orientation: Orientation) -> Rgba8Image {
    if orientation == Orientation::IDENTITY {
        return image;
    }
    let (width, height) = (image.width as usize, image.height as usize);
    let (out_width, out_height) = orientation.apply_to_size(width, height);
    let mut pixels = vec![0u8; image.pixels.len()];
    for (index, source) in image.pixels.chunks_exact(Rgba8Image::CHANNELS).enumerate() {
        let (mut x, mut y) = (index % width, index / width);
        if orientation.flip_horizontal {
            x = width - 1 - x;
        }
        let (mut w, mut h) = (width, height);
        for _ in 0..orientation.quarter_turns % 4 {
            (x, y) = (h - 1 - y, x);
            (w, h) = (h, w);
        }
        let at = (y * out_width + x) * Rgba8Image::CHANNELS;
        pixels[at..at + Rgba8Image::CHANNELS].copy_from_slice(source);
    }
    Rgba8Image {
        width: out_width as u32,
        height: out_height as u32,
        pixels,
        space: image.space,
    }
}

#[cfg(test)]
mod tests {
    use photo_core::colorimetry::RgbSpace;
    use photo_core::geom::Point;

    use super::*;

    /// A 3 x 2 image whose pixel at (x, y) has red x and green y.
    fn labelled() -> Rgba8Image {
        let pixels = (0..2u8)
            .flat_map(|y| (0..3u8).flat_map(move |x| [x, y, 0, 255]))
            .collect();
        Rgba8Image {
            width: 3,
            height: 2,
            pixels,
            space: RgbSpace::Srgb,
        }
    }

    #[test]
    fn every_orientation_moves_pixels_where_the_settings_map_points() {
        for turns in 0..4 {
            for flip in [false, true] {
                let orientation = Orientation::new(turns, flip);
                let turned = orient(labelled(), orientation);
                let (w, h) = orientation.apply_to_size(3u32, 2u32);
                assert_eq!((turned.width, turned.height), (w, h), "{orientation:?}");
                for y in 0..2u8 {
                    for x in 0..3u8 {
                        let centre =
                            Point::new((f32::from(x) + 0.5) / 3.0, (f32::from(y) + 0.5) / 2.0);
                        let moved = orientation.forward(centre);
                        let out_x = (moved.x * w as f32 - 0.5).round() as u32;
                        let out_y = (moved.y * h as f32 - 0.5).round() as u32;
                        assert_eq!(
                            turned.pixel(out_x, out_y),
                            [x, y, 0, 255],
                            "{orientation:?} ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}
