//! Painting one box of the canvas: whatever stands in for the photo, the
//! newest frame on top of it where it covers, the clipping overlay, a
//! mask's red overlay, and the Before and After captions.
//!
//! Once the photo is decoded every frame is painted through the canvas's
//! map, by the layout of the settings version it was rendered for; until
//! then there is only the box the photo will take.

use egui::{Align2, Color32, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use photo_core::meta::PhotoId;
use photo_engine::{PhotoSummary, PreviewSize};

use super::Side;
use super::map::CanvasMap;
use super::mapping::{Placement, to_points};
use crate::app::Cx;
use crate::app::develop::frames::{CanvasImage, CanvasShows, Frames, Layouts, Uploaded};
use crate::theme::{self, palette};
use crate::widgets::ClippingToggles;

/// The whole texture.
const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// What a box showed of the edit.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Shown {
    image: CanvasImage,
    version: Option<u64>,
    current: bool,
}

impl Shown {
    pub(crate) fn into_shows(self, photo: PhotoId) -> CanvasShows {
        CanvasShows {
            photo: Some(photo),
            image: self.image,
            version: self.version,
            current: self.current,
        }
    }
}

/// Where a box's frames go.
struct Place<'a> {
    placement: &'a Placement,
    ppp: f32,
    map: Option<&'a CanvasMap>,
    layouts: &'a Layouts,
    toggles: ClippingToggles,
}

/// Paint the picture of one box. `map` is the mapping through the photo's
/// geometry, once it is decoded. Returns what the box shows of the edit.
pub(super) fn picture(
    painter: &Painter,
    cx: &mut Cx<'_>,
    photo: &PhotoSummary,
    placement: &Placement,
    map: Option<&CanvasMap>,
    side: Side,
) -> Shown {
    let ppp = painter.ctx().pixels_per_point();
    let photo_rect = to_points(placement.photo(), ppp);
    let session = cx
        .engine
        .develop()
        .filter(|session| session.photo() == photo.id);
    let (ours, version) = (session.is_some(), session.map(|session| session.version()));
    let ctx = painter.ctx().clone();
    let mut shown = Shown::default();
    // Frames are cut to the photo's box: one rendered for the crop tool
    // reaches past it.
    let inside = painter.with_clip_rect(painter.clip_rect().intersect(photo_rect.expand(0.5)));
    let underlaid = ours && cx.frames.whole.is_some();
    let stand_in = if side == Side::Before || underlaid {
        None
    } else {
        library_preview(&ctx, cx, photo.id)
    };
    let toggles = cx.ui.develop.clipping;
    let Frames {
        main,
        whole,
        before,
        mask,
        layouts,
        ..
    } = &mut *cx.frames;
    let place = Place {
        placement,
        ppp,
        map,
        layouts,
        toggles,
    };

    if side == Side::Before {
        painter.rect_filled(photo_rect, 0.0, palette::WELL);
        let before = ours.then_some(before.as_mut()).flatten();
        if frame(&inside, &ctx, before, &place).is_none() {
            painter.text(
                photo_rect.intersect(painter.clip_rect()).center(),
                Align2::CENTER_CENTER,
                "Rendering the before view…",
                theme::small_font(),
                palette::TEXT_DIM,
            );
        }
        outline(painter, photo_rect);
        return shown;
    }

    // The underlay: the whole photo, from the best source there is.
    match ours.then_some(whole.as_ref()).flatten() {
        Some(whole) => {
            match (map, whole.roi) {
                (Some(map), Some(roi)) => {
                    map.paint(&inside, whole.id(), roi, layouts.of(whole.version));
                }
                _ => {
                    painter.image(whole.id(), photo_rect, UV, Color32::WHITE);
                }
            }
            shown.image = if whole.version.is_some() {
                CanvasImage::Rendered
            } else {
                CanvasImage::EmbeddedPreview
            };
            shown.version = whole.version;
        }
        None => match stand_in {
            Some(id) => {
                painter.image(id, photo_rect, UV, Color32::WHITE);
                shown.image = CanvasImage::LibraryPreview;
            }
            None => {
                painter.rect_filled(photo_rect, 0.0, palette::WELL);
            }
        },
    }
    if ours {
        if let Some(drawn) = frame(&inside, &ctx, main.as_mut(), &place) {
            shown.image = CanvasImage::Rendered;
            shown.version = Some(drawn);
        }
        if let (Some(map), Some(mask)) = (map, mask.as_ref()) {
            map.paint(&inside, mask.id(), mask.roi, layouts.of(Some(mask.version)));
        }
        if side == Side::Split {
            let middle = painter.clip_rect().center().x;
            let left = inside.with_clip_rect(inside.clip_rect().with_max_x(middle));
            left.rect_filled(photo_rect, 0.0, palette::WELL);
            frame(&left, &ctx, before.as_mut(), &place);
            let seen = photo_rect.intersect(painter.clip_rect());
            painter.vline(
                middle,
                seen.y_range(),
                Stroke::new(1.0, palette::TEXT_STRONG),
            );
        }
    }
    shown.current = shown.version.is_some() && shown.version == version;
    outline(painter, photo_rect);
    shown
}

/// Paint the edit through `map` alone, with nothing cut away: what the crop
/// tool shows, the frame's own surroundings included.
pub(crate) fn edit(painter: &Painter, cx: &mut Cx<'_>, map: &CanvasMap) -> Shown {
    let version = cx.engine.develop().map(|session| session.version());
    let ctx = painter.ctx().clone();
    let toggles = cx.ui.develop.clipping;
    let Frames {
        main,
        whole,
        layouts,
        ..
    } = &mut *cx.frames;
    let mut shown = Shown::default();
    if let Some(whole) = whole.as_ref()
        && let Some(roi) = whole.roi
    {
        map.paint(painter, whole.id(), roi, layouts.of(whole.version));
        shown.image = CanvasImage::Rendered;
        shown.version = whole.version;
    }
    if let Some(main) = main.as_mut()
        && let Some(roi) = main.roi
    {
        let rendered = layouts.of(main.version);
        map.paint(painter, main.id(), roi, rendered);
        if let Some(overlay) = main.overlay(&ctx, toggles) {
            map.paint(painter, overlay, roi, rendered);
        }
        shown.image = CanvasImage::Rendered;
        shown.version = main.version;
    }
    shown.current = shown.version.is_some() && shown.version == version;
    shown
}

/// The hairline that seats the photo on the canvas.
fn outline(painter: &Painter, photo: Rect) {
    painter.rect_stroke(
        photo,
        0.0,
        Stroke::new(1.0, palette::SEPARATOR),
        StrokeKind::Outside,
    );
}

/// Paint `uploaded` where its pixels belong, with its clipping overlay.
/// Returns its version when a rendered frame was painted.
fn frame(
    painter: &Painter,
    ctx: &egui::Context,
    uploaded: Option<&mut Uploaded>,
    place: &Place<'_>,
) -> Option<u64> {
    let uploaded = uploaded?;
    let overlay = uploaded.overlay(ctx, place.toggles);
    match (place.map, uploaded.roi) {
        (Some(map), Some(roi)) => {
            let rendered = place.layouts.of(uploaded.version);
            map.paint(painter, uploaded.id(), roi, rendered);
            if let Some(overlay) = overlay {
                map.paint(painter, overlay, roi, rendered);
            }
        }
        (_, roi) => {
            let rect = match roi {
                Some(roi) => to_points(place.placement.on_screen(roi), place.ppp),
                None => to_points(place.placement.photo(), place.ppp),
            };
            painter.image(uploaded.id(), rect, UV, Color32::WHITE);
            if let Some(overlay) = overlay {
                painter.image(overlay, rect, UV, Color32::WHITE);
            }
        }
    }
    uploaded.version
}

/// The Library's preview of the photo (its standard preview, or while that
/// is made its thumbnail), shown while the session has nothing yet.
fn library_preview(
    ctx: &egui::Context,
    cx: &mut Cx<'_>,
    photo: PhotoId,
) -> Option<egui::TextureId> {
    let standard = cx
        .textures
        .photo(ctx, cx.engine, photo, PreviewSize::Standard);
    standard
        .or_else(|| {
            cx.textures
                .photo(ctx, cx.engine, photo, PreviewSize::Thumbnail)
        })
        .map(|shown| shown.id)
}

/// The Before and After captions of a box. `paired` is true when the edit
/// shares the canvas with a before view, so it is captioned too.
pub(super) fn labels(
    painter: &Painter,
    rect: Rect,
    placement: &Placement,
    ppp: f32,
    side: Side,
    paired: bool,
) {
    let photo = to_points(placement.photo(), ppp).intersect(rect);
    let top = photo.top() + 8.0;
    match side {
        Side::After if paired => caption(
            painter,
            pos2(photo.left() + 8.0, top),
            "After",
            Align2::LEFT_TOP,
        ),
        Side::After => {}
        Side::Before => caption(
            painter,
            pos2(photo.left() + 8.0, top),
            "Before",
            Align2::LEFT_TOP,
        ),
        Side::Split => {
            let middle = rect.center().x;
            caption(
                painter,
                pos2(middle - 8.0, top),
                "Before",
                Align2::RIGHT_TOP,
            );
            caption(painter, pos2(middle + 8.0, top), "After", Align2::LEFT_TOP);
        }
    }
}

/// A small caption on a dark pill, so it reads on any photo.
fn caption(painter: &Painter, at: Pos2, text: &str, align: Align2) {
    let galley = painter.layout_no_wrap(text.to_owned(), theme::small_font(), palette::TEXT_STRONG);
    let rect = align.anchor_size(at, galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(rect, 3.0, Color32::from_black_alpha(160));
    painter.galley(rect.min + vec2(6.0, 3.0), galley, palette::TEXT_STRONG);
}
