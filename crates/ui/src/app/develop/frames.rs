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
//!
//! A frame shows the output as the settings of its version laid it out. The
//! layout of each recent version is kept ([`Layouts`]), so that a frame
//! rendered before a crop or a turn is still drawn where its content is.
//! The red overlay of a mask is a texture too, made from the coverage the
//! engine computed.

use std::collections::VecDeque;
use std::sync::Arc;

use egui::{Color32, ColorImage, TextureHandle, TextureId, TextureOptions};
use photo_core::geom;
use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoId;
use photo_engine::{DevelopFrame, DevelopSession, Geometry, MaskOverlay, Plane, PresetKey};

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
    /// The red overlay of the mask the engine was asked to show.
    pub(crate) mask: Option<MaskTexture>,
    /// How each recent version of the settings laid the output out.
    pub(crate) layouts: Layouts,
    /// What the canvas drew last.
    pub(crate) shows: CanvasShows,
}

/// How many versions' layouts are kept.
const LAYOUTS_KEPT: usize = 96;

/// The geometry of recent versions of the open photo's settings.
#[derive(Default)]
pub(crate) struct Layouts {
    versions: VecDeque<(u64, Geometry)>,
}

impl Layouts {
    /// Keep the layout of the session's current version, if it is new.
    pub(crate) fn note(&mut self, session: &DevelopSession) {
        let version = session.version();
        if self
            .versions
            .back()
            .is_some_and(|(last, _)| *last >= version)
        {
            return;
        }
        let Some(geometry) = session.geometry() else {
            return;
        };
        if self.versions.len() == LAYOUTS_KEPT {
            self.versions.pop_front();
        }
        self.versions.push_back((version, geometry));
    }

    /// The layout `version` had: its own, or that of the newest version
    /// before it that was seen. `None` when nothing that old is kept, which
    /// is drawn as the settings lay the output out now.
    pub(crate) fn of(&self, version: Option<u64>) -> Option<&Geometry> {
        let version = version?;
        self.versions
            .iter()
            .rev()
            .find(|(kept, _)| *kept <= version)
            .map(|(_, geometry)| geometry)
    }
}

/// The colour a mask's overlay takes where the mask covers fully.
pub(crate) const MASK_TINT: [u8; 3] = [236, 44, 40];
/// How opaque the overlay is where the mask covers fully, of 255.
const MASK_ALPHA: f32 = 150.0;

/// The red overlay of one mask over the canvas view.
pub(crate) struct MaskTexture {
    alpha: Arc<Plane>,
    handle: TextureHandle,
    /// The output rectangle the coverage is of.
    pub(crate) roi: geom::Rect,
    /// The settings version it was computed from.
    pub(crate) version: u64,
}

impl MaskTexture {
    fn of(ctx: &egui::Context, overlay: &MaskOverlay) -> Option<Self> {
        let image = mask_image(overlay.alpha())?;
        Some(Self {
            alpha: Arc::clone(overlay.alpha()),
            handle: ctx.load_texture("develop-mask-overlay", image, TextureOptions::LINEAR),
            roi: overlay.view().roi(),
            version: overlay.version(),
        })
    }

    /// The texture to draw.
    pub(crate) fn id(&self) -> TextureId {
        self.handle.id()
    }
}

/// A mask's coverage as its red overlay; `None` for a plane with no pixels.
fn mask_image(alpha: &Plane) -> Option<ColorImage> {
    if alpha.is_empty() {
        return None;
    }
    let [red, green, blue] = MASK_TINT;
    let pixels = alpha
        .as_slice()
        .iter()
        .map(|coverage| {
            let opacity = (coverage.clamp(0.0, 1.0) * MASK_ALPHA).round() as u8;
            Color32::from_rgba_unmultiplied(red, green, blue, opacity)
        })
        .collect();
    Some(ColorImage::new([alpha.width(), alpha.height()], pixels))
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
        self.layouts.note(session);
        match session.mask_overlay() {
            Some(overlay)
                if !self
                    .mask
                    .as_ref()
                    .is_some_and(|mask| Arc::ptr_eq(&mask.alpha, overlay.alpha())) =>
            {
                self.mask = MaskTexture::of(ctx, overlay);
            }
            Some(_) => {}
            None => self.mask = None,
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

/// Whether `frame` shows the whole photo of `output` pixels, and no more:
/// a frame the crop tool asked for reaches past the photo on every side,
/// and is not what the Navigator shows.
fn covers_whole(frame: &DevelopFrame, output: Option<(u32, u32)>) -> bool {
    let Some((width, height)) = output else {
        return false;
    };
    let roi = frame.view().roi();
    let near = |value: f32, edge: f32| (value - edge).abs() <= 0.5;
    near(roi.x0, 0.0)
        && near(roi.y0, 0.0)
        && near(roi.x1, width as f32)
        && near(roi.y1, height as f32)
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
    fn a_masks_overlay_is_red_where_it_covers_and_clear_where_it_does_not() {
        let alpha = Plane::from_vec(3, 1, vec![0.0, 0.5, 1.0]).unwrap();
        let image = mask_image(&alpha).unwrap();
        assert_eq!(image.size, [3, 1]);
        assert_eq!(image.pixels[0], Color32::TRANSPARENT);
        assert!(image.pixels[1].a() > 60 && image.pixels[1].a() < 90);
        let full = image.pixels[2];
        assert_eq!(full.a(), 150);
        assert!(
            full.r() > full.g() * 3 && full.r() > full.b() * 3,
            "{full:?}"
        );
        assert!(mask_image(&Plane::new(0, 0)).is_none());
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
