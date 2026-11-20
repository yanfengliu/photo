//! Textures of what the Develop canvas and the Navigator draw: the open
//! photo's newest frame, its before frame, the newest preset preview, and
//! an underlay of the whole photo.
//!
//! A frame is uploaded once, when the engine has a new one; until then the
//! one before stays on screen, so a slider never blanks the canvas. The
//! underlay is the last frame that showed the whole photo, or before the
//! first render the camera's embedded preview: drawn under a zoomed frame,
//! it fills what the frame does not cover while a pan waits for the next
//! render. Clipping overlays are made from a frame's own pixels, only while
//! they are switched on.

use std::sync::Arc;

use egui::{Color32, ColorImage, TextureHandle, TextureId, TextureOptions};
use photo_core::geom;
use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoId;
use photo_engine::{DevelopFrame, DevelopSession, PresetKey};

use crate::theme::palette;
use crate::widgets::ClippingToggles;

/// What the canvas shows of the photo, as last drawn. For tests and the
/// automation harness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CanvasImage {
    /// Nothing yet: an empty well.
    #[default]
    Nothing,
    /// The Library's preview of the photo, while the session starts.
    LibraryPreview,
    /// The camera's embedded preview, while a RAW file is decoded.
    EmbeddedPreview,
    /// A frame the develop pipeline rendered.
    Rendered,
}

/// What the Develop canvas showed in the last frame drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasShows {
    /// The photo shown.
    pub photo: Option<PhotoId>,
    /// Where its pixels came from.
    pub image: CanvasImage,
    /// The settings version of the rendered frame on top, if any.
    pub version: Option<u64>,
    /// Whether that frame is the newest version of the settings.
    pub current: bool,
}

/// One uploaded image.
#[derive(Clone)]
pub(crate) struct Uploaded {
    image: Arc<Rgba8Image>,
    pub(crate) handle: TextureHandle,
    /// The output rectangle the pixels cover; `None` for an image of the
    /// whole photo whose size is its own (the embedded preview).
    pub(crate) roi: Option<geom::Rect>,
    /// The settings version, for a rendered frame.
    pub(crate) version: Option<u64>,
    overlay: Option<(ClippingToggles, TextureHandle)>,
}

impl Uploaded {
    fn of_frame(ctx: &egui::Context, name: &str, frame: &DevelopFrame) -> Option<Self> {
        let mut uploaded = Self::of_image(ctx, name, frame.image())?;
        uploaded.roi = Some(frame.view().roi());
        uploaded.version = Some(frame.version());
        Some(uploaded)
    }

    fn of_image(ctx: &egui::Context, name: &str, image: &Arc<Rgba8Image>) -> Option<Self> {
        let color = color_image(image)?;
        Some(Self {
            image: Arc::clone(image),
            handle: ctx.load_texture(name, color, TextureOptions::LINEAR),
            roi: None,
            version: None,
            overlay: None,
        })
    }

    fn is(&self, image: &Arc<Rgba8Image>) -> bool {
        Arc::ptr_eq(&self.image, image)
    }

    /// The texture to draw.
    pub(crate) fn id(&self) -> TextureId {
        self.handle.id()
    }

    /// The pixels' size.
    pub(crate) fn size(&self) -> egui::Vec2 {
        egui::vec2(self.image.width as f32, self.image.height as f32)
    }

    /// The clipping overlay for `toggles`, made on first use; `None` when
    /// both are off.
    pub(crate) fn overlay(
        &mut self,
        ctx: &egui::Context,
        toggles: ClippingToggles,
    ) -> Option<TextureId> {
        if !toggles.shadows && !toggles.highlights {
            return None;
        }
        if let Some((made_for, handle)) = &self.overlay
            && *made_for == toggles
        {
            return Some(handle.id());
        }
        let image = clipping_overlay(&self.image, toggles)?;
        let handle = ctx.load_texture("develop-clipping", image, TextureOptions::NEAREST);
        let id = handle.id();
        self.overlay = Some((toggles, handle));
        Some(id)
    }
}

/// The Develop textures. See the [module documentation](self).
#[derive(Default)]
pub(crate) struct Frames {
    /// The photo, and the session's load state count, the textures are for.
    photo: Option<PhotoId>,
    pub(crate) main: Option<Uploaded>,
    pub(crate) whole: Option<Uploaded>,
    pub(crate) before: Option<Uploaded>,
    pub(crate) look: Option<(PresetKey, Uploaded)>,
    /// What the canvas drew last.
    pub(crate) shows: CanvasShows,
}

impl Frames {
    /// Bring the textures in line with the open session: upload what is new,
    /// forget everything when the photo changed or the session closed.
    pub(crate) fn sync(&mut self, ctx: &egui::Context, session: Option<&DevelopSession>) {
        let Some(session) = session else {
            self.forget();
            return;
        };
        if self.photo != Some(session.photo()) {
            self.forget();
            self.photo = Some(session.photo());
        }
        if let Some(frame) = session.frame()
            && !self
                .main
                .as_ref()
                .is_some_and(|main| main.is(frame.image()))
        {
            self.main = Uploaded::of_frame(ctx, "develop-frame", frame);
            if covers_whole(frame, session.output_size()) {
                self.whole.clone_from(&self.main);
            }
        }
        if self.whole.is_none()
            && let Some(embedded) = session.embedded_preview()
        {
            self.whole = Uploaded::of_image(ctx, "develop-embedded", embedded);
        }
        match session.before_frame() {
            Some(frame)
                if !self
                    .before
                    .as_ref()
                    .is_some_and(|before| before.is(frame.image())) =>
            {
                self.before = Uploaded::of_frame(ctx, "develop-before", frame);
            }
            Some(_) => {}
            None => self.before = None,
        }
        match session.preset_preview() {
            Some((key, frame))
                if !self
                    .look
                    .as_ref()
                    .is_some_and(|(shown, look)| shown == key && look.is(frame.image())) =>
            {
                self.look = Uploaded::of_frame(ctx, "develop-look", frame).map(|look| (*key, look));
            }
            Some(_) => {}
            None => self.look = None,
        }
    }

    /// Let every texture go (the module is left), keeping only the record
    /// of what was shown.
    pub(crate) fn forget(&mut self) {
        let shows = self.shows;
        *self = Self::default();
        self.shows = shows;
    }
}

/// Whether `frame` shows the whole photo of `output` pixels.
fn covers_whole(frame: &DevelopFrame, output: Option<(u32, u32)>) -> bool {
    let Some((width, height)) = output else {
        return false;
    };
    let roi = frame.view().roi();
    roi.x0 <= 0.5 && roi.y0 <= 0.5 && roi.x1 >= width as f32 - 0.5 && roi.y1 >= height as f32 - 0.5
}

/// The pixels as egui takes them; `None` when they do not match their size.
fn color_image(image: &Rgba8Image) -> Option<ColorImage> {
    let (width, height) = (image.width as usize, image.height as usize);
    let expected = width.checked_mul(height)?.checked_mul(4)?;
    if width == 0 || height == 0 || image.pixels.len() != expected {
        log::warn!("a develop frame of {width} x {height} does not match its buffer");
        return None;
    }
    // Frames are opaque, so their bytes are already premultiplied.
    Some(ColorImage::from_rgba_premultiplied(
        [width, height],
        &image.pixels,
    ))
}

/// The colour a pixel takes in the clipping overlay: highlight clipping
/// where any channel is at full scale, shadow clipping where any is at
/// zero, and see-through elsewhere. These are the histogram's own tests
/// (`clipped_highlights`, `clipped_shadows`).
pub(crate) fn clipping_color(pixel: [u8; 3], toggles: ClippingToggles) -> Color32 {
    if toggles.highlights && pixel.contains(&255) {
        palette::CLIP_HIGHLIGHTS
    } else if toggles.shadows && pixel.contains(&0) {
        palette::CLIP_SHADOWS
    } else {
        Color32::TRANSPARENT
    }
}

fn clipping_overlay(image: &Rgba8Image, toggles: ClippingToggles) -> Option<ColorImage> {
    let (width, height) = (image.width as usize, image.height as usize);
    if width == 0 || height == 0 || image.pixels.len() != width * height * 4 {
        return None;
    }
    let pixels = image
        .pixels
        .chunks_exact(4)
        .map(|pixel| clipping_color([pixel[0], pixel[1], pixel[2]], toggles))
        .collect();
    Some(ColorImage::new([width, height], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: ClippingToggles = ClippingToggles {
        shadows: true,
        highlights: true,
    };

    #[test]
    fn a_channel_at_either_end_is_marked_and_the_rest_shows_through() {
        assert_eq!(
            clipping_color([255, 40, 30], BOTH),
            palette::CLIP_HIGHLIGHTS
        );
        assert_eq!(clipping_color([0, 0, 0], BOTH), palette::CLIP_SHADOWS);
        assert_eq!(clipping_color([12, 0, 200], BOTH), palette::CLIP_SHADOWS);
        assert_eq!(clipping_color([128, 128, 128], BOTH), Color32::TRANSPARENT);
        let only_shadows = ClippingToggles {
            shadows: true,
            highlights: false,
        };
        assert_eq!(
            clipping_color([255, 255, 255], only_shadows),
            Color32::TRANSPARENT
        );
    }

    #[test]
    fn the_overlay_has_the_frames_size_and_pixels_that_do_not_match_are_refused() {
        let image = Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![255, 255, 255, 255, 10, 20, 30, 255],
            space: photo_core::colorimetry::RgbSpace::Srgb,
        };
        let overlay = clipping_overlay(&image, BOTH).unwrap();
        assert_eq!(overlay.size, [2, 1]);
        assert_eq!(overlay.pixels[0], palette::CLIP_HIGHLIGHTS);
        assert_eq!(overlay.pixels[1], Color32::TRANSPARENT);
        let broken = Rgba8Image {
            pixels: vec![0; 3],
            ..image
        };
        assert!(color_image(&broken).is_none());
        assert!(clipping_overlay(&broken, BOTH).is_none());
    }
}
