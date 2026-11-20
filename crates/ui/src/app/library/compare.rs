//! Compare: the select and a candidate side by side.
//!
//! On entering, the select is the active photo and the candidate the next
//! selected photo (or the next photo of the list). The arrow keys step the
//! candidate through the list; Swap exchanges the two; Make Select makes
//! the candidate the select and takes the next photo as candidate. Clicking
//! either makes it the active photo, the one rating keys apply to, and a
//! photo made active elsewhere (the filmstrip) becomes the candidate.

use egui::{Align2, Rect, Ui, UiBuilder, vec2};
use photo_core::meta::PhotoId;
use photo_engine::PreviewSize;

use super::{cell, prefixed};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::theme::{self, palette, space};

/// Gap between the two panes, and around them.
const GAP: f32 = 12.0;
/// Height of the caption above each pane.
const CAPTION: f32 = 20.0;

/// The two photos Compare shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ComparePair {
    pub(crate) select: Option<PhotoId>,
    pub(crate) candidate: Option<PhotoId>,
    /// The active photo when the pair was last shown, to tell when another
    /// photo is made active from outside Compare.
    pub(crate) seen_active: Option<PhotoId>,
}

/// The pair as it should be now: the remembered one while its photos are
/// still in the list, a fresh one otherwise; a photo newly made active that
/// is neither becomes the candidate.
fn current(cx: &Cx<'_>) -> ComparePair {
    let listed = |photo: Option<PhotoId>| photo.filter(|id| cx.engine.index_of(*id).is_some());
    let remembered = cx.ui.compare;
    let mut pair = ComparePair {
        select: listed(remembered.select),
        candidate: listed(remembered.candidate),
        seen_active: remembered.seen_active,
    };
    let active = cx.engine.active_photo();
    if pair.select.is_none() {
        pair.select = active;
    }
    if active != remembered.seen_active
        && let Some(active) = active
        && Some(active) != pair.select
    {
        pair.candidate = Some(active);
    }
    if pair.candidate.is_none() || pair.candidate == pair.select {
        pair.candidate = pair.select.and_then(|select| next_candidate(cx, select));
    }
    pair
}

/// Start comparing afresh: the active photo and its candidate.
pub(crate) fn reset(cx: &mut Cx<'_>) {
    cx.ui.compare = ComparePair {
        seen_active: cx.engine.active_photo(),
        ..ComparePair::default()
    };
}

/// The photo after `photo` in the list, or the one before at the end.
fn neighbour(cx: &Cx<'_>, photo: PhotoId) -> Option<PhotoId> {
    let at = cx.engine.index_of(photo)?;
    let photos = cx.engine.photos();
    photos
        .get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|before| photos.get(before)))
        .map(|photo| photo.id)
}

/// The photo to compare `select` with: the next selected photo, or failing
/// that the next photo of the list (the one before at the end).
fn next_candidate(cx: &Cx<'_>, select: PhotoId) -> Option<PhotoId> {
    let selected = cx.engine.selection().selected();
    if selected.len() > 1
        && let Some(at) = selected.iter().position(|photo| *photo == select)
    {
        return Some(selected[(at + 1) % selected.len()]);
    }
    neighbour(cx, select)
}

/// The arrow keys: move the candidate `step` photos along the list,
/// passing over the select.
pub(crate) fn step_candidate(cx: &mut Cx<'_>, step: isize) {
    let pair = current(cx);
    let (Some(select), Some(candidate)) = (pair.select, pair.candidate) else {
        return;
    };
    let Some(mut at) = cx.engine.index_of(candidate) else {
        return;
    };
    let last = cx.engine.photo_count().saturating_sub(1);
    loop {
        let next = at.saturating_add_signed(step).min(last);
        if next == at {
            return;
        }
        at = next;
        let Some(photo) = cx.engine.photos().get(at).map(|photo| photo.id) else {
            return;
        };
        if photo != select {
            cx.ui.compare = ComparePair {
                select: Some(select),
                candidate: Some(photo),
                ..pair
            };
            if cx.engine.active_photo() == Some(candidate) {
                cx.act(Action::MakeActive(photo));
            }
            return;
        }
    }
}

/// Swap: the candidate becomes the select and the select the candidate.
pub(crate) fn swap(cx: &mut Cx<'_>) {
    let pair = current(cx);
    cx.ui.compare = ComparePair {
        select: pair.candidate,
        candidate: pair.select,
        ..pair
    };
}

/// Make Select: the candidate becomes the select, and the photo after it
/// the candidate.
pub(crate) fn make_select(cx: &mut Cx<'_>) {
    let pair = current(cx);
    let Some(candidate) = pair.candidate else {
        return;
    };
    cx.ui.compare = ComparePair {
        select: Some(candidate),
        candidate: neighbour(cx, candidate),
        ..pair
    };
    cx.act(Action::MakeActive(candidate));
}

/// The names of the select and the candidate, for the toolbar.
pub(crate) fn names(cx: &Cx<'_>) -> (Option<String>, Option<String>) {
    let pair = current(cx);
    let name = |photo: Option<PhotoId>| {
        photo
            .and_then(|id| cx.engine.summary(id))
            .map(crate::app::text::photo_name)
    };
    (name(pair.select), name(pair.candidate))
}

/// Show the two panes.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let pair = ComparePair {
        seen_active: cx.engine.active_photo(),
        ..current(cx)
    };
    cx.ui.compare = pair;
    let area = ui.max_rect().shrink(GAP);
    let Some(select) = pair.select else {
        message(ui, "Select a photo to compare it with another.");
        return;
    };
    let half = (area.width() - GAP) / 2.0;
    let left = Rect::from_min_size(area.min, vec2(half, area.height()));
    let right = Rect::from_min_size(area.min + vec2(half + GAP, 0.0), vec2(half, area.height()));
    pane(ui, cx, left, "Select", select);
    match pair.candidate {
        Some(candidate) => pane(ui, cx, right, "Candidate", candidate),
        None => {
            ui.painter().text(
                right.center(),
                Align2::CENTER_CENTER,
                "There is no other photo to compare with.",
                theme::body_font(),
                palette::TEXT_DIM,
            );
        }
    }
}

/// One pane: its caption and the photo, which a click makes active.
fn pane(ui: &mut Ui, cx: &mut Cx<'_>, rect: Rect, role: &str, photo: PhotoId) {
    let Some(summary) = cx.engine.summary(photo).cloned() else {
        return;
    };
    ui.painter().text(
        rect.left_top() + vec2(space::XS, CAPTION / 2.0),
        Align2::LEFT_CENTER,
        role.to_uppercase(),
        theme::small_font(),
        palette::TEXT_DIM,
    );
    let cell_rect = rect.with_min_y(rect.top() + CAPTION);
    let name = prefixed(role, &summary);
    let builder = UiBuilder::new().max_rect(cell_rect).id_salt(("compare", role));
    let response = ui
        .scope_builder(builder, |ui| {
            cell(ui, cx, &summary, &name, cell_rect.size(), PreviewSize::Standard)
                .loose(true)
                .show(ui)
        })
        .inner;
    if response.clicked {
        cx.act(Action::MakeActive(photo));
    }
}

/// A line of text in the middle of the work area.
fn message(ui: &Ui, text: &str) {
    ui.painter().text(
        ui.max_rect().center(),
        Align2::CENTER_CENTER,
        text,
        theme::body_font(),
        palette::TEXT_DIM,
    );
}
