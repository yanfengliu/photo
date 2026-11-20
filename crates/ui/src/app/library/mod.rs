//! The Library module (design §8.2): the panel columns, the filter bar,
//! the four views, the toolbar and the filmstrip, laid out in the shell.
//!
//! ```text
//! ┌──────────────── top bar ─────────────────┐
//! │ left   │ filter bar          │ right     │
//! │ panels │ grid / loupe /      │ panels    │
//! │        │ compare / survey    │           │
//! │        │ toolbar             │           │
//! ├────────┴─────────────────────┴───────────┤
//! │ source line · filmstrip                  │
//! └──────────────────────────────────────────┘
//! ```

pub(crate) mod compare;
mod drag;
mod filmstrip;
pub(crate) mod filter_bar;
mod filter_metadata;
mod grid;
mod left;
mod loupe;
mod right;
mod survey;
mod toolbar;

use egui::{Rect, Ui};
use photo_core::meta::{ColorLabel, Flag, PhotoId, Rating};
use photo_engine::{PhotoSummary, PreviewSize};

pub(crate) use self::drag::DraggedPhotos;
pub(crate) use self::left::LeftState;
pub(crate) use self::right::RightState;
use super::Cx;
use super::shell::{Edge, edge_strip};
use super::state::View;
use super::text;
use crate::theme;
use crate::widgets::{Badges, SelectionTier, ThumbImage, Thumbnail};

/// Lay the Library out in what the top bar left of `ui`. Returns the work
/// area's rect, which toasts are shown over.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) -> Rect {
    // Only the loupe shows part of a photo for the navigator to outline.
    if cx.ui.view != View::Loupe {
        cx.ui.loupe_region = None;
    }
    // The filmstrip spans the whole width, under the columns.
    if cx.ui.layout.filmstrip {
        filmstrip::show(ui, cx);
    } else {
        edge_strip(ui, Edge::Bottom, &mut cx.ui.layout.filmstrip);
    }
    edge_strip(ui, Edge::Left, &mut cx.ui.layout.left);
    edge_strip(ui, Edge::Right, &mut cx.ui.layout.right);
    if cx.ui.layout.left {
        left::show(ui, cx);
    }
    if cx.ui.layout.right {
        right::show(ui, cx);
    }
    if cx.ui.layout.toolbar {
        toolbar::show(ui, cx);
    }
    if cx.ui.view == View::Grid {
        filter_bar::show(ui, cx);
    }
    let work_area = egui::CentralPanel::default()
        .frame(theme::work_area_frame())
        .show(ui, |ui| {
            match cx.ui.view {
                View::Grid => grid::show(ui, cx),
                View::Loupe => loupe::show(ui, cx),
                View::Compare => compare::show(ui, cx),
                View::Survey => survey::show(ui, cx),
            }
            ui.max_rect()
        })
        .inner;
    // The grid and the filmstrip have both scrolled to a newly active photo.
    cx.ui.grid.reveal_active = false;
    drag::preview(ui.ctx());
    work_area
}

/// How selected `photo` is.
fn tier(cx: &Cx<'_>, photo: PhotoId) -> SelectionTier {
    if cx.engine.active_photo() == Some(photo) {
        SelectionTier::Active
    } else if cx.engine.selection().is_selected(photo) {
        SelectionTier::Selected
    } else {
        SelectionTier::None
    }
}

/// The badges a photo's cell carries.
fn badges(photo: &PhotoSummary) -> Badges {
    Badges {
        edited: photo.has_edits,
        virtual_copy: photo.is_virtual_copy(),
        missing: photo.missing,
        quick_collection: photo.in_quick_collection,
    }
}

/// A cell for `photo` of `size` points named `name`, showing the preview
/// of `preview` size (a thumbnail, or a standard preview for large cells).
fn cell<'a>(
    ui: &Ui,
    cx: &mut Cx<'_>,
    photo: &PhotoSummary,
    name: &'a str,
    size: egui::Vec2,
    preview: PreviewSize,
) -> Thumbnail<'a> {
    let mut shown = cx.textures.photo(ui.ctx(), cx.engine, photo.id, preview);
    if shown.is_none() && preview == PreviewSize::Standard {
        // The thumbnail stands in while the larger preview is made.
        shown = cx
            .textures
            .photo(ui.ctx(), cx.engine, photo.id, PreviewSize::Thumbnail);
    }
    let image = shown.map_or(ThumbImage::Placeholder, |shown| ThumbImage::Texture {
        id: shown.id,
        aspect: shown.aspect,
    });
    // The shape the photo is shown in, rotated and cropped, so that the
    // cell keeps its shape when the thumbnail arrives.
    let (width, height) = cx
        .engine
        .displayed_size(photo.id)
        .unwrap_or((photo.width, photo.height));
    Thumbnail::new(name, size)
        .image(image)
        .placeholder_aspect(width as f32 / height.max(1) as f32)
        .tier(tier(cx, photo.id))
        .rating(photo.rating)
        .flag(photo.flag)
        .label(photo.label)
        .badges(badges(photo))
}

/// The rating, flag and label of the active photo, for the toolbar's
/// controls; the defaults when no photo is active.
fn active_marks(cx: &Cx<'_>) -> Option<(Rating, Flag, ColorLabel)> {
    let active = cx.engine.active_photo()?;
    let summary = cx.engine.summary(active)?;
    Some((summary.rating, summary.flag, summary.label))
}

/// The summaries of the photos `ids` names that are in the list, in the
/// order given.
fn summaries(cx: &Cx<'_>, ids: &[PhotoId]) -> Vec<PhotoSummary> {
    ids.iter()
        .filter_map(|id| cx.engine.summary(*id).cloned())
        .collect()
}

/// The accessible name of a photo's cell in a view whose cells carry a
/// prefix ("Filmstrip dawn.jpg").
fn prefixed(prefix: &str, photo: &PhotoSummary) -> String {
    format!("{prefix} {}", text::photo_name(photo))
}
