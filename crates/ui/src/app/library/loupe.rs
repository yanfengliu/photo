//! The loupe: the active photo alone, fitted, filling the work area, or at
//! 1:1. Drag to pan a photo larger than the work area; click to go between
//! the fitted zoom and 1:1 at the point clicked.
//!
//! It shows the standard preview, with the thumbnail standing in (at the
//! same size, so nothing moves) until that arrives. Until the develop
//! pipeline renders full resolution, 1:1 is the standard preview at its own
//! resolution; the toolbar says so.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use photo_engine::{PhotoSummary, PreviewSize};

use crate::app::Cx;
use crate::app::state::Zoom;
use crate::app::text;
use crate::app::textures::Shown;
use crate::theme::{self, palette};

/// The longest edge of a standard preview, in pixels.
const STANDARD_EDGE: f32 = PreviewSize::Standard.long_edge() as f32;
/// Room kept around a fitted photo.
const MARGIN: f32 = 16.0;
/// Height of the line of information above the photo.
const INFO: f32 = 26.0;

/// Show the active photo.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let area = ui.max_rect();
    let Some(photo) = cx
        .engine
        .active_photo()
        .and_then(|photo| cx.engine.summary(photo).cloned())
    else {
        cx.ui.loupe_region = None;
        message(ui, area, "Select a photo to see it here.");
        return;
    };
    let standard = cx
        .textures
        .photo(ui.ctx(), cx.engine, photo.id, PreviewSize::Standard);
    let thumbnail = cx
        .textures
        .photo(ui.ctx(), cx.engine, photo.id, PreviewSize::Thumbnail);
    let shown = standard.or(thumbnail);
    let natural = natural_size(&photo, standard, thumbnail);
    let pixels_per_point = ui.ctx().pixels_per_point();
    let stage = area.with_min_y(area.top() + INFO);
    // A fitted photo keeps a margin; a larger one uses the whole stage.
    let view_for = |zoom| if zoom == Zoom::Fit { stage.shrink(MARGIN) } else { stage };

    let response = ui.interact(stage, ui.make_persistent_id("loupe"), Sense::click_and_drag());
    let loupe = &mut cx.ui.loupe;
    if response.is_pointer_button_down_on() {
        loupe.pan += ui.input(|input| input.pointer.delta());
    }
    if response.hovered() && loupe.zoom != Zoom::Fit {
        loupe.pan += ui.input(|input| input.smooth_scroll_delta);
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let view = view_for(loupe.zoom);
        let size = image_size(loupe.zoom, natural, view.size(), pixels_per_point);
        let image = placed(view, size, loupe.pan, pixels_per_point);
        zoom_at(loupe, image, pointer, natural, view, pixels_per_point);
    }
    let view = view_for(loupe.zoom);
    let size = image_size(loupe.zoom, natural, view.size(), pixels_per_point);
    loupe.pan = clamp_pan(loupe.pan, size, view.size());
    let image = placed(view, size, loupe.pan, pixels_per_point);
    cx.ui.loupe_region = region(image, view);

    let painter = ui.painter_at(stage);
    match shown {
        Some(shown) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(shown.id, image, uv, Color32::WHITE);
        }
        None => {
            painter.rect_filled(image, 0.0, palette::WELL);
        }
    }
    painter.rect_stroke(
        image,
        0.0,
        Stroke::new(1.0, palette::SEPARATOR),
        StrokeKind::Outside,
    );
    let band = Rect::from_min_max(area.min, pos2(area.right(), stage.top()));
    info_line(ui, cx, &photo, band);
}

/// The size, in pixels, of the photo at 1:1: the standard preview's own
/// size, or while only the thumbnail is there, the size the standard
/// preview will have.
fn natural_size(photo: &PhotoSummary, standard: Option<Shown>, thumbnail: Option<Shown>) -> Vec2 {
    if let Some(standard) = standard {
        return standard.pixels;
    }
    let aspect = thumbnail.map_or(photo.width as f32 / photo.height.max(1) as f32, |t| t.aspect);
    let long = (photo.width.max(photo.height) as f32).clamp(1.0, STANDARD_EDGE);
    if aspect >= 1.0 {
        vec2(long, long / aspect)
    } else {
        vec2(long * aspect, long)
    }
}

/// The photo's size on screen, in points, at `zoom` in a view of
/// `view` points.
fn image_size(zoom: Zoom, natural: Vec2, view: Vec2, pixels_per_point: f32) -> Vec2 {
    let natural = natural.max(Vec2::splat(1.0));
    match zoom {
        Zoom::Fit => natural * (view.x / natural.x).min(view.y / natural.y),
        Zoom::Fill => natural * (view.x / natural.x).max(view.y / natural.y),
        Zoom::OneToOne => natural / pixels_per_point,
    }
}

/// Where a photo of `size` sits in `view`, panned by `pan`, its corner on a
/// whole physical pixel so that 1:1 stays sharp.
fn placed(view: Rect, size: Vec2, pan: Vec2, pixels_per_point: f32) -> Rect {
    let min = (view.center() + pan - size / 2.0).to_vec2();
    let min = (min * pixels_per_point).round() / pixels_per_point;
    Rect::from_min_size(min.to_pos2(), size)
}

/// Keep a photo larger than the view covering it; centre a smaller one.
fn clamp_pan(pan: Vec2, size: Vec2, view: Vec2) -> Vec2 {
    let room = ((size - view) / 2.0).max(Vec2::ZERO);
    vec2(pan.x.clamp(-room.x, room.x), pan.y.clamp(-room.y, room.y))
}

/// A click at `pointer` on the photo drawn at `image`: from a fitted zoom
/// to 1:1 with the clicked point kept under the pointer, and from 1:1 back
/// to the fitted zoom.
fn zoom_at(
    loupe: &mut crate::app::state::LoupeState,
    image: Rect,
    pointer: Pos2,
    natural: Vec2,
    view: Rect,
    pixels_per_point: f32,
) {
    if loupe.zoom == Zoom::OneToOne {
        loupe.toggle();
        return;
    }
    let fraction = ((pointer - image.min) / image.size()).clamp(Vec2::ZERO, Vec2::splat(1.0));
    loupe.toggle();
    let size = image_size(Zoom::OneToOne, natural, view.size(), pixels_per_point);
    let min = pointer - fraction * size;
    loupe.pan = (min + size / 2.0) - view.center();
}

/// The part of the photo in view, as fractions of its width and height;
/// `None` when all of it is.
fn region(image: Rect, view: Rect) -> Option<Rect> {
    let seen = image.intersect(view);
    if seen.width() >= image.width() - 0.5 && seen.height() >= image.height() - 0.5 {
        return None;
    }
    let to_unit = |point: Pos2| pos2(
        (point.x - image.left()) / image.width(),
        (point.y - image.top()) / image.height(),
    );
    Some(Rect::from_min_max(to_unit(seen.min), to_unit(seen.max)))
}

/// The loupe's status line, above the photo: the file name and the
/// capture and exposure readings at the left; at 1:1, what 1:1 shows.
fn info_line(ui: &Ui, cx: &mut Cx<'_>, photo: &PhotoSummary, band: Rect) {
    let mut parts = vec![text::photo_name(photo)];
    if let Some(time) = &photo.capture_time {
        parts.push(text::date_time(time));
    }
    let generation = cx.generation;
    if let Some(record) = cx.ui.record.get(cx.engine, photo.id, generation) {
        parts.extend(text::exposure(&record.metadata));
        parts.extend(record.metadata.iso.map(|iso| format!("ISO {iso}")));
    }
    let painter = ui.painter_at(band);
    let note = (cx.ui.loupe.zoom == Zoom::OneToOne)
        .then_some("1:1 shows the standard preview (up to 2048 pixels) until Develop renders full size");
    let note_width = note.map_or(0.0, |note| {
        let galley = painter.layout_no_wrap(note.to_owned(), theme::small_font(), palette::TEXT_DIM);
        let width = galley.size().x;
        let at = Align2::RIGHT_CENTER.anchor_size(band.right_center() - vec2(12.0, 0.0), galley.size());
        painter.galley(at.min, galley, palette::TEXT_DIM);
        width + 24.0
    });
    let room = band.width() - note_width - 24.0;
    let galley = text::fitted(&painter, &parts.join("    "), theme::small_font(), palette::TEXT, room);
    let at = Align2::LEFT_CENTER.anchor_size(band.left_center() + vec2(12.0, 0.0), galley.size());
    painter.galley(at.min, galley, palette::TEXT);
}

/// A line of text in the middle of the work area.
fn message(ui: &Ui, area: Rect, text: &str) {
    ui.painter().text(
        area.center(),
        Align2::CENTER_CENTER,
        text,
        theme::body_font(),
        palette::TEXT_DIM,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shows_everything_fill_covers_and_one_to_one_is_one_pixel_per_pixel() {
        let natural = vec2(2048.0, 1024.0);
        let view = vec2(1000.0, 800.0);
        assert_eq!(image_size(Zoom::Fit, natural, view, 1.0), vec2(1000.0, 500.0));
        assert_eq!(image_size(Zoom::Fill, natural, view, 1.0), vec2(1600.0, 800.0));
        assert_eq!(image_size(Zoom::OneToOne, natural, view, 2.0), vec2(1024.0, 512.0));
    }

    #[test]
    fn a_large_photo_cannot_be_panned_off_the_view_and_a_small_one_stays_centred() {
        let view = vec2(1000.0, 800.0);
        let pan = clamp_pan(vec2(900.0, -900.0), vec2(1600.0, 800.0), view);
        assert_eq!(pan, vec2(300.0, 0.0));
        assert_eq!(clamp_pan(vec2(50.0, 50.0), vec2(400.0, 300.0), view), Vec2::ZERO);
    }

    #[test]
    fn a_click_zooms_in_keeping_the_clicked_point_under_the_pointer() {
        let view = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0));
        let natural = vec2(2000.0, 1000.0);
        let mut loupe = crate::app::state::LoupeState::default();
        let fit = image_size(Zoom::Fit, natural, view.size(), 1.0);
        let fitted = placed(view, fit, Vec2::ZERO, 1.0);
        let pointer = pos2(750.0, 125.0);
        zoom_at(&mut loupe, fitted, pointer, natural, view, 1.0);
        assert_eq!(loupe.zoom, Zoom::OneToOne);
        let zoomed = placed(view, natural, loupe.pan, 1.0);
        let fraction = (pointer - zoomed.min) / zoomed.size();
        assert!((fraction - vec2(0.75, 0.25)).length() < 1.0e-4, "{fraction:?}");
    }

    #[test]
    fn the_region_in_view_is_reported_only_when_part_is_hidden() {
        let view = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let whole = Rect::from_center_size(view.center(), vec2(80.0, 60.0));
        assert_eq!(region(whole, view), None);
        let wide = Rect::from_center_size(view.center(), vec2(200.0, 100.0));
        let seen = region(wide, view).unwrap();
        assert!((seen.left() - 0.25).abs() < 1.0e-5 && (seen.right() - 0.75).abs() < 1.0e-5);
    }
}
