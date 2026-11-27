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
//!
//! With a tool open the canvas is the tool's: a press and a drag go to it,
//! and it draws its marks over the photo. The crop tool has the whole frame
//! shown, straightened, in place of the output (`crop_stage`).

mod draw;
pub(crate) mod map;
pub(crate) mod mapping;

use egui::{
    Align, Align2, CursorIcon, Label, Layout, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2,
};
use photo_core::geom::Point;
use photo_engine::{DevelopLoad, PhotoSummary};

use self::map::{CanvasMap, Display, Straight};
pub use self::mapping::CanvasZoom;
use self::mapping::to_pixels;
pub(crate) use self::mapping::{CanvasState, Placement};
use super::actions::DevelopAction;
use super::right::Editing;
use super::state::BeforeAfter;
use super::tools::{self, Tool, crop};
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
        cx.ui.develop.photo_rect = None;
        cx.ui.develop.stage = None;
        cx.ui.develop.map = None;
        message(ui, area, "Select a photo in the filmstrip to develop it.");
        return;
    };
    let stage = area.with_min_y(area.top() + INFO);
    cx.ui.develop.stage = Some(stage);
    let ppp = ui.ctx().pixels_per_point();
    let Some(output) = output_size(cx, &photo) else {
        cx.ui.develop.map = None;
        message(ui, stage, "The photo's size is not known yet.");
        return;
    };
    let band = Rect::from_min_max(area.min, pos2(area.right(), stage.top()));
    let editing = tools::editing(cx).filter(|editing| editing.photo == photo.id);
    let tool = tools::active(cx).filter(|_| editing.is_some());
    if tool == Some(Tool::Crop)
        && let Some(editing) = &editing
        && crop_stage(ui, cx, &photo, stage, editing)
    {
        info_line(ui, cx, &photo, band);
        return;
    }
    cx.ui.develop.tools.crop_rect = None;
    // A tool works on the edit alone.
    let boxes = match tool {
        Some(_) => vec![(stage, Side::After)],
        None => boxes(stage, cx.ui.develop.before_after),
    };
    let state = cx.ui.develop.canvas;
    let place = |rect: Rect, state: &CanvasState| {
        Placement::new(state.zoom, state.center, output, to_pixels(rect, ppp))
    };

    // Input over the whole stage, with the placement of the box under the
    // pointer.
    let response = ui.interact(
        stage,
        ui.make_persistent_id("develop-canvas"),
        Sense::click_and_drag(),
    );
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    let under = pointer
        .and_then(|at| boxes.iter().find(|(rect, _)| rect.contains(at)))
        .or(boxes.last())
        .map_or(stage, |(rect, _)| *rect);
    let placement = place(under, &state);
    let pointer_px = pointer.map(|at| (at.to_vec2() * ppp).to_pos2());
    // The mapping through the photo's geometry, once it is decoded.
    let geometry = cx
        .engine
        .develop()
        .filter(|session| session.photo() == photo.id)
        .and_then(|session| session.geometry());
    let through = |placement: Placement| {
        geometry.clone().map(|geometry| CanvasMap {
            ppp,
            geometry,
            display: Display::Output(placement),
        })
    };
    let tooled = tool.and(editing.as_ref()).zip(through(placement));
    if let Some((editing, map)) = &tooled {
        // The tool has the pointer; the wheel still moves a zoomed photo.
        tools::interact(ui, cx, editing, map, &response);
        if response.hovered() && !placement.fitted {
            let scroll = ui.input(|input| input.smooth_scroll_delta) * ppp;
            cx.ui.develop.canvas.pan(&placement, scroll);
        }
    } else {
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
            cx.act(Action::Develop(DevelopAction::PickWhiteBalance(
                Point::new(at.x, at.y),
            )));
        }
    }

    // What the engine is asked to render: the box of the edit.
    let state = cx.ui.develop.canvas;
    let main = boxes.last().map_or(stage, |(rect, _)| *rect);
    let main_placement = place(main, &state);
    let view = main_placement.canvas_view();
    if cx.ui.develop.told.canvas != Some(view)
        && cx
            .engine
            .develop()
            .is_some_and(|session| session.photo() == photo.id)
    {
        cx.ui.develop.told.canvas = Some(view);
        cx.act(Action::Develop(DevelopAction::SetCanvasView(view)));
    }
    cx.ui.develop.seen = main_placement.seen_fraction();
    cx.ui.develop.last_placement = Some(main_placement);
    cx.ui.develop.photo_rect = Some(mapping::to_points(main_placement.photo(), ppp));

    let mut shown = draw::Shown::default();
    let paired = boxes.len() > 1;
    for (rect, side) in &boxes {
        let placement = place(*rect, &state);
        let painter = ui.painter_at(*rect);
        let map = through(placement);
        let here = draw::picture(&painter, cx, &photo, &placement, map.as_ref(), *side);
        if *side != Side::Before {
            shown = here;
        }
        draw::labels(&painter, *rect, &placement, ppp, *side, paired);
    }
    cx.frames.shows = shown.into_shows(photo.id);
    let map = through(main_placement);
    if let (Some(_), Some(map)) = (tool, &map)
        && let Some(editing) = tools::editing(cx)
    {
        // The marks are those of the settings as this frame left them.
        let pointer = response.hover_pos();
        tools::paint(&ui.painter_at(stage), cx, &editing, map, pointer);
    }
    cx.ui.develop.map = map;
    info_line(ui, cx, &photo, band);
}

/// The canvas of the crop tool: the whole frame, straightened, with the
/// rectangle over it. Returns false when the photo is not ready for it,
/// and nothing was drawn.
fn crop_stage(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    photo: &PhotoSummary,
    stage: Rect,
    editing: &Editing,
) -> bool {
    let ppp = ui.ctx().pixels_per_point();
    let Some(frame) = crop::Frame::of(cx, &editing.settings) else {
        return false;
    };
    let Some(session) = cx.engine.develop() else {
        return false;
    };
    let Some(geometry) = session.geometry() else {
        return false;
    };
    let angle = editing.settings.crop.angle;
    let source = geometry.source_size();
    let straight = Straight::new(&editing.settings, source, angle, to_pixels(stage, ppp));
    let limit = ui.ctx().input(|input| input.max_texture_side) as f32;
    let view = crop::canvas::view(session, straight.scale, limit);
    let map = CanvasMap {
        ppp,
        geometry,
        display: Display::Straight(Box::new(straight.clone())),
    };
    let response = ui.interact(
        stage,
        ui.make_persistent_id("develop-canvas"),
        Sense::click_and_drag(),
    );
    crop::canvas::interact(ui, cx, editing, &frame, &straight, &response);
    if let Some(view) = view
        && cx.ui.develop.told.canvas != Some(view)
    {
        cx.ui.develop.told.canvas = Some(view);
        cx.act(Action::Develop(DevelopAction::SetCanvasView(view)));
    }
    let develop = &mut cx.ui.develop;
    develop.seen = None;
    develop.last_placement = None;
    develop.photo_rect = Some(map.photo_rect());

    let painter = ui.painter_at(stage);
    let shown = draw::edit(&painter, cx, &map);
    if let Some(corners) = map.source_corners() {
        crop::canvas::cover_outside(&painter, corners);
    }
    // The rectangle as this frame's drag left it, unless the angle moved:
    // then the photo follows next frame, and the rectangle with it.
    let crop_now = tools::editing(cx)
        .map(|now| now.settings.crop)
        .filter(|crop| crop.angle == angle)
        .unwrap_or(editing.settings.crop);
    let upright = crop::rect::upright_of(&crop_now, frame.size);
    let rect = crop::canvas::on_screen(&straight, upright, ppp);
    cx.ui.develop.tools.crop_rect = Some(rect);
    crop::canvas::paint(&painter, &cx.ui.develop.tools.crop, rect);
    cx.frames.shows = shown.into_shows(photo.id);
    cx.ui.develop.map = Some(map);
    true
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

/// What the load indicator says while the photo is shown as `load`, and
/// in which colour; nothing once the rendered frame is up.
pub(crate) fn load_note(load: Option<&DevelopLoad>, read_only: bool) -> (String, egui::Color32) {
    match load {
        None | Some(DevelopLoad::Nothing) => ("Loading…".to_owned(), palette::TEXT_DIM),
        Some(DevelopLoad::EmbeddedPreview) => (
            "Camera preview: the raw file is being read".to_owned(),
            palette::WARNING,
        ),
        Some(DevelopLoad::Decoded) => ("Rendering…".to_owned(), palette::TEXT_DIM),
        Some(DevelopLoad::Failed(reason)) => {
            (format!("Could not read the file: {reason}"), palette::ERROR)
        }
        Some(DevelopLoad::Rendered) if read_only => ("Read only".to_owned(), palette::WARNING),
        Some(DevelopLoad::Rendered) => (String::new(), palette::TEXT_DIM),
    }
}

/// The line above the photo: the file name and its readings at the left;
/// at the right, what is shown while the photo loads, or why it cannot be.
fn info_line(ui: &mut Ui, cx: &mut Cx<'_>, photo: &PhotoSummary, band: Rect) {
    let mut parts = vec![text::photo_name(photo)];
    if let Some(time) = &photo.capture_time {
        parts.push(text::date_time(time));
    }
    let generation = cx.generation;
    if let Some(record) = cx.ui.record.get(cx.engine, photo.id, generation) {
        parts.extend(text::exposure(&record.metadata));
        parts.extend(record.metadata.iso.map(|iso| format!("ISO {iso}")));
    }
    let session = cx
        .engine
        .develop()
        .filter(|session| session.photo() == photo.id);
    let (note, color) = load_note(
        session.map(|session| session.load_state()),
        session.is_some_and(|session| session.read_only_reason().is_some()),
    );
    let inner = band.shrink2(vec2(12.0, 0.0));
    let mut right = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    if !note.is_empty() {
        right.label(RichText::new(note).color(color).small());
    }
    let taken = right.min_rect().width();
    let rest = inner.with_max_x(inner.right() - taken - 24.0);
    let mut left = ui.new_child(
        UiBuilder::new()
            .max_rect(rest)
            .layout(Layout::left_to_right(Align::Center)),
    );
    left.add(
        Label::new(
            RichText::new(parts.join("    "))
                .color(palette::TEXT)
                .small(),
        )
        .truncate(),
    );
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

#[cfg(test)]
mod tests {
    use photo_engine::DevelopLoad;

    use super::load_note;

    /// What the line above the photo says at each stage of loading. The
    /// camera-preview stage can be shorter than a frame on a fast machine,
    /// so its words are pinned here rather than only on screen.
    #[test]
    fn the_load_indicator_says_what_is_shown_at_each_stage() {
        let says = |load: Option<&DevelopLoad>, read_only| load_note(load, read_only).0;
        assert_eq!(says(None, false), "Loading…");
        assert_eq!(says(Some(&DevelopLoad::Nothing), false), "Loading…");
        assert_eq!(
            says(Some(&DevelopLoad::EmbeddedPreview), false),
            "Camera preview: the raw file is being read"
        );
        assert_eq!(says(Some(&DevelopLoad::Decoded), false), "Rendering…");
        assert_eq!(says(Some(&DevelopLoad::Rendered), false), "");
        assert_eq!(says(Some(&DevelopLoad::Rendered), true), "Read only");
    }
}
