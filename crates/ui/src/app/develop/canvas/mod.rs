//! The Develop canvas: the open photo on near-black, fitted, filled or at
//! 1:1, 2:1 or 4:1. Drag to pan; click, Space or Z to go between the fitted
//! zoom and 1:1 around the pointer. Before and after in place, side by side
//! or split; clipping overlays; the white-balance picker.
//!
//! The engine is told what the canvas shows ([`Placement::canvas_view`]), so
//! every render is sized to the screen. Until the first render arrives the
//! canvas shows what it has: the camera's embedded preview, or the
//! Library's preview of the photo; each is drawn where the rendered frame
//! will be, so nothing moves when it arrives.

mod draw;
pub(crate) mod mapping;

use egui::{Align2, CursorIcon, Rect, Sense, Ui, pos2, vec2};
use photo_core::geom::Point;
use photo_engine::{DevelopLoad, PhotoSummary};

pub use self::mapping::CanvasZoom;
pub(crate) use self::mapping::{CanvasState, Placement};
use self::mapping::to_pixels;
use super::actions::DevelopAction;
use super::state::BeforeAfter;
use crate::app::actions::Action;
use crate::app::{Cx, text};
use crate::theme::{self, palette};

/// Height of the line of information above the photo.
const INFO: f32 = 26.0;
/// Gap between the two halves of a side-by-side view.
const GAP: f32 = 6.0;

/// Which picture a box of the canvas shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// The edit.
    After,
    /// The before view.
    Before,
    /// The before view left of the middle, the edit right of it.
    Split,
}

/// Show the canvas in `ui`'s whole rect.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let area = ui.max_rect();
    let Some(photo) = cx
        .engine
        .active_photo()
        .and_then(|photo| cx.engine.summary(photo).cloned())
    else {
        cx.ui.develop.seen = None;
        cx.ui.develop.last_placement = None;
        message(ui, area, "Select a photo in the filmstrip to develop it.");
        return;
    };
    let stage = area.with_min_y(area.top() + INFO);
    let ppp = ui.ctx().pixels_per_point();
    let Some(output) = output_size(cx, &photo) else {
        message(ui, stage, "The photo's size is not known yet.");
        return;
    };
    let boxes = boxes(stage, cx.ui.develop.before_after);
    let state = cx.ui.develop.canvas;
    let place = |rect: Rect, state: &CanvasState| {
        Placement::new(state.zoom, state.center, output, to_pixels(rect, ppp))
    };

    // Input over the whole stage, with the placement of the box under the
    // pointer.
    let response = ui.interact(stage, ui.make_persistent_id("develop-canvas"), Sense::click_and_drag());
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let under = pointer
        .and_then(|at| boxes.iter().find(|(rect, _)| rect.contains(at)))
        .or(boxes.last())
        .map_or(stage, |(rect, _)| *rect);
    let placement = place(under, &state);
    let pointer_px = pointer.map(|at| (at.to_vec2() * ppp).to_pos2());
    let develop = &mut cx.ui.develop;
    if develop.picker && pointer_px.is_some_and(|at| placement.photo().contains(at)) {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    if response.dragged() && !placement.fitted {
        let delta = ui.input(|input| input.pointer.delta()) * ppp;
        develop.canvas.pan(&placement, delta);
    }
    if response.hovered() && !placement.fitted {
        let scroll = ui.input(|input| input.smooth_scroll_delta) * ppp;
        develop.canvas.pan(&placement, scroll);
    }
    let mut picked = None;
    if response.clicked() {
        if develop.picker {
            picked = pointer_px
                .filter(|at| placement.photo().contains(*at))
                .map(|at| placement.to_output(at));
        } else {
            develop.canvas.toggle_at(&placement, pointer_px);
        }
    }
    if let Some(at) = picked {
        develop.picker = false;
        cx.act(Action::Develop(DevelopAction::PickWhiteBalance(Point::new(at.x, at.y))));
    }

    // What the engine is asked to render: the box of the edit.
    let state = cx.ui.develop.canvas;
    let main = boxes.last().map_or(stage, |(rect, _)| *rect);
    let main_placement = place(main, &state);
    let view = main_placement.canvas_view();
    if cx.ui.develop.told.canvas != Some(view)
        && cx.engine.develop().is_some_and(|session| session.photo() == photo.id)
    {
        cx.ui.develop.told.canvas = Some(view);
        cx.act(Action::Develop(DevelopAction::SetCanvasView(view)));
    }
    cx.ui.develop.seen = main_placement.seen_fraction();
    cx.ui.develop.last_placement = Some(main_placement);

    let mut shown = draw::Shown::default();
    let paired = boxes.len() > 1;
    for (rect, side) in &boxes {
        let placement = place(*rect, &state);
        let painter = ui.painter_at(*rect);
        let here = draw::picture(&painter, cx, &photo, &placement, ppp, *side);
        if *side != Side::Before {
            shown = here;
        }
        draw::labels(&painter, *rect, &placement, ppp, *side, paired);
    }
    cx.frames.shows = shown.into_shows(photo.id);
    let band = Rect::from_min_max(area.min, pos2(area.right(), stage.top()));
    info_line(ui, cx, &photo, band);
}

/// The boxes of the stage and what each shows, the edit's last.
fn boxes(stage: Rect, mode: BeforeAfter) -> Vec<(Rect, Side)> {
    match mode {
        BeforeAfter::Off => vec![(stage, Side::After)],
        BeforeAfter::Before => vec![(stage, Side::Before)],
        BeforeAfter::Split => vec![(stage, Side::Split)],
        BeforeAfter::SideBySide => {
            let half = ((stage.width() - GAP) / 2.0).floor();
            let left = Rect::from_min_size(stage.min, vec2(half, stage.height()));
            let right = Rect::from_min_max(pos2(stage.right() - half, stage.top()), stage.max);
            vec![(left, Side::Before), (right, Side::After)]
        }
    }
}

/// The photo's full-resolution size as shown (turned and cropped): the
/// open session's once its file is decoded, before that the catalog's.
fn output_size(cx: &mut Cx<'_>, photo: &PhotoSummary) -> Option<egui::Vec2> {
    let decoded = cx
        .engine
        .develop()
        .filter(|session| session.photo() == photo.id)
        .and_then(|session| session.output_size());
    let (width, height) = decoded.or_else(|| cx.engine.displayed_size(photo.id))?;
    (width > 0 && height > 0).then(|| vec2(width as f32, height as f32))
}

/// Space, Z: between the fitted zoom and 1:1 around the pointer.
pub(crate) fn toggle_zoom(cx: &mut Cx<'_>, ctx: &egui::Context) {
    let develop = &mut cx.ui.develop;
    let Some(placement) = develop.last_placement else {
        develop.canvas.set(match develop.canvas.zoom {
            CanvasZoom::Fit | CanvasZoom::Fill => CanvasZoom::OneToOne,
            _ => develop.canvas.fitted,
        });
        return;
    };
    let ppp = ctx.pixels_per_point();
    let pointer = ctx
        .input(|input| input.pointer.hover_pos())
        .map(|at| (at.to_vec2() * ppp).to_pos2());
    develop.canvas.toggle_at(&placement, pointer);
}

/// Choose a zoom from the toolbar, around the canvas's centre.
pub(crate) fn set_zoom(cx: &mut Cx<'_>, zoom: CanvasZoom) {
    let develop = &mut cx.ui.develop;
    match develop.last_placement {
        Some(placement) => develop.canvas.zoom_to(zoom, &placement, None),
        None => develop.canvas.set(zoom),
    }
}

/// The line above the photo: the file name and its readings at the left;
/// at the right, what is shown while the photo loads, or why it cannot be.
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
    let session = cx.engine.develop().filter(|session| session.photo() == photo.id);
    let (note, color) = match session.map(|session| (session.load_state(), session.read_only_reason())) {
        None | Some((DevelopLoad::Nothing, _)) => ("Loading…".to_owned(), palette::TEXT_DIM),
        Some((DevelopLoad::EmbeddedPreview, _)) => (
            "Camera preview: the raw file is being read".to_owned(),
            palette::WARNING,
        ),
        Some((DevelopLoad::Decoded, _)) => ("Rendering…".to_owned(), palette::TEXT_DIM),
        Some((DevelopLoad::Failed(reason), _)) => {
            (format!("Could not read the file: {reason}"), palette::ERROR)
        }
        Some((DevelopLoad::Rendered, Some(_))) => ("Read only".to_owned(), palette::WARNING),
        Some((DevelopLoad::Rendered, None)) => (String::new(), palette::TEXT_DIM),
    };
    let painter = ui.painter_at(band);
    let note_width = if note.is_empty() {
        0.0
    } else {
        let galley = painter.layout_no_wrap(note, theme::small_font(), color);
        let width = galley.size().x;
        let at = Align2::RIGHT_CENTER.anchor_size(band.right_center() - vec2(12.0, 0.0), galley.size());
        painter.galley(at.min, galley, color);
        width + 24.0
    };
    let room = band.width() - note_width - 24.0;
    let galley = text::fitted(
        &painter,
        &parts.join("    "),
        theme::small_font(),
        palette::TEXT,
        room,
    );
    let at = Align2::LEFT_CENTER.anchor_size(band.left_center() + vec2(12.0, 0.0), galley.size());
    painter.galley(at.min, galley, palette::TEXT);
}

/// A line of text in the middle of `area`.
fn message(ui: &Ui, area: Rect, text: &str) {
    ui.painter().text(
        area.center(),
        Align2::CENTER_CENTER,
        text,
        theme::body_font(),
        palette::TEXT_DIM,
    );
}
