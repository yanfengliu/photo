//! The toolbar under the work area: the view buttons, then what the view
//! offers (sort order, the marks of the photos keys apply to, rotation,
//! the thumbnail size, the loupe's zoom, Compare's swap).
//!
//! The view's own controls are laid out first, from the right edge; the
//! rest fill what is left from the left edge, and the groups that matter
//! least (sort order, then rotation, then the colour labels, which keys
//! also set) give way in a narrow window, so no control ever lies on
//! another.

use egui::{
    Align, Button, ComboBox, Label, Layout, Margin, RichText, Slider, Stroke, Ui, UiBuilder, vec2,
};
use photo_core::meta::{ColorLabel, Flag, Rating};
use photo_engine::{Sort, SortDirection, SortKey};

use super::{active_marks, compare, survey};
use crate::app::actions::Action;
use crate::app::state::{THUMB_SIZES, View, Zoom};
use crate::app::{Cx, dispatch, text};
use crate::theme::{self, palette, size, space};
use crate::widgets::{Icon, IconButton, color_labels, flag_toggle, rating_stars};

/// Height of the toolbar.
const TOOLBAR: f32 = 36.0;
/// Below this width the view's controls drop their words.
const ROOMY: f32 = 760.0;
/// Width of the views, flag and stars, which are always shown.
const ALWAYS: f32 = 264.0;
/// Width the colour labels add.
const LABELS: f32 = 96.0;
/// Width the rotate buttons add.
const ROTATE: f32 = 64.0;
/// Width the sort menu adds.
const SORT: f32 = 196.0;
/// Widest the names of the two photos in Compare are shown.
const NAMES: f32 = 320.0;

/// The sort keys, as the sort menu lists them.
const SORT_KEYS: [(SortKey, &str); 9] = [
    (SortKey::CaptureTime, "Capture Time"),
    (SortKey::Added, "Added Order"),
    (SortKey::FileName, "File Name"),
    (SortKey::Rating, "Rating"),
    (SortKey::Pick, "Pick"),
    (SortKey::Label, "Label"),
    (SortKey::EditTime, "Edit Time"),
    (SortKey::FileType, "File Type"),
    (SortKey::Custom, "User Order"),
];

/// Show the toolbar panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(8, 0));
    egui::Panel::bottom("toolbar")
        .exact_size(TOOLBAR)
        .resizable(false)
        .frame(frame)
        .show(ui, |ui| {
            let bar = ui.max_rect();
            let roomy = bar.width() >= ROOMY;
            let mut right = ui.new_child(
                UiBuilder::new()
                    .max_rect(bar)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            right.spacing_mut().item_spacing.x = space::SM;
            match cx.ui.view {
                View::Grid => thumbnail_size(&mut right, cx, roomy),
                View::Loupe => zoom(&mut right, cx),
                View::Compare => compare_controls(&mut right, cx, roomy),
                View::Survey if roomy => survey_note(&mut right, cx),
                View::Survey => {}
            }
            let taken = right.min_rect().width();
            let rest = bar.with_max_x(bar.right() - taken - space::LG);
            let mut left = ui.new_child(
                UiBuilder::new()
                    .max_rect(rest)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            // Whatever does not fit is cut off rather than drawn on the
            // view's controls.
            left.set_clip_rect(rest.intersect(ui.clip_rect()));
            left.spacing_mut().item_spacing.x = space::XS;
            views(&mut left, cx);
            divider(&mut left);
            let room = rest.width();
            if cx.ui.view == View::Grid && room >= ALWAYS + LABELS + ROTATE + SORT {
                sort(&mut left, cx);
                divider(&mut left);
            }
            marks(&mut left, cx, room);
        });
}

fn views(ui: &mut Ui, cx: &mut Cx<'_>) {
    for (view, icon, label) in [
        (View::Grid, Icon::Grid, "Grid view"),
        (View::Loupe, Icon::Loupe, "Loupe view"),
        (View::Compare, Icon::Compare, "Compare view"),
        (View::Survey, Icon::Survey, "Survey view"),
    ] {
        let button = IconButton::new(icon, label).selected(cx.ui.view == view);
        if ui.add(button).clicked() {
            dispatch::open_view(cx, view);
        }
    }
}

/// The sort menu and its direction.
fn sort(ui: &mut Ui, cx: &mut Cx<'_>) {
    let current = cx.engine.sort();
    ui.label(RichText::new("Sort").color(palette::TEXT_DIM));
    let name = SORT_KEYS
        .iter()
        .find(|(key, _)| *key == current.key)
        .map_or("Capture Time", |(_, name)| *name);
    let mut chosen = current.key;
    ComboBox::from_id_salt("sort-key")
        .selected_text(name)
        .width(116.0)
        .show_ui(ui, |ui| {
            for (key, name) in SORT_KEYS {
                ui.selectable_value(&mut chosen, key, name);
            }
        })
        .response
        .accessible_name("Sort order");
    let (icon, label, flipped) = match current.direction {
        SortDirection::Ascending => (Icon::ChevronUp, "Sort ascending", SortDirection::Descending),
        SortDirection::Descending => (
            Icon::ChevronDown,
            "Sort descending",
            SortDirection::Ascending,
        ),
    };
    let mut direction = current.direction;
    if ui.add(IconButton::new(icon, label)).clicked() {
        direction = flipped;
    }
    let sort = Sort {
        key: chosen,
        direction,
    };
    if sort != current {
        cx.act(Action::SetSort(sort));
    }
}

/// Flag, rating and label of the photos keys apply to, and rotation, as
/// far as `room` (the width left of the view's controls) allows.
fn marks(ui: &mut Ui, cx: &mut Cx<'_>, room: f32) {
    let target = cx.ui.view.target();
    let marks = active_marks(cx);
    let (mut rating, mut flag, mut label) =
        marks.unwrap_or((Rating::UNRATED, Flag::Unflagged, ColorLabel::None));
    ui.add_enabled_ui(marks.is_some(), |ui| {
        if flag_toggle(ui, "Flag", &mut flag).changed() {
            cx.act(Action::SetFlag(target.clone(), flag));
        }
        divider(ui);
        if rating_stars(ui, "Rating", &mut rating).changed() {
            cx.act(Action::SetRating(target.clone(), rating));
        }
        if room < ALWAYS + LABELS {
            return;
        }
        divider(ui);
        if color_labels(ui, "Label", &mut label).changed() {
            // A swatch clicked on gives every photo its label; the swatch
            // of the label shown, clicked off, clears them all.
            cx.act(Action::SetLabel(target.clone(), label));
        }
        if room < ALWAYS + LABELS + ROTATE {
            return;
        }
        divider(ui);
        if ui
            .add(IconButton::new(Icon::RotateCcw, "Rotate left"))
            .clicked()
        {
            cx.act(Action::RotateLeft(target.clone()));
        }
        if ui
            .add(IconButton::new(Icon::RotateCw, "Rotate right"))
            .clicked()
        {
            cx.act(Action::RotateRight(target.clone()));
        }
    });
}

fn thumbnail_size(ui: &mut Ui, cx: &mut Cx<'_>, roomy: bool) {
    ui.add(
        Slider::new(&mut cx.ui.thumb_size, THUMB_SIZES)
            .show_value(false)
            .trailing_fill(true),
    )
    .accessible_name("Thumbnail size");
    if roomy {
        ui.label(RichText::new("Thumbnails").color(palette::TEXT_DIM));
    }
}

fn zoom(ui: &mut Ui, cx: &mut Cx<'_>) {
    // Right to left: the last choice is added first.
    for (zoom, label) in [
        (Zoom::OneToOne, "1:1"),
        (Zoom::Fill, "Fill"),
        (Zoom::Fit, "Fit"),
    ] {
        let selected = cx.ui.loupe.zoom == zoom;
        if ui
            .add(Button::selectable(selected, label).min_size(vec2(36.0, size::ROW)))
            .clicked()
        {
            cx.ui.loupe.set(zoom);
        }
    }
}

fn compare_controls(ui: &mut Ui, cx: &mut Cx<'_>, roomy: bool) {
    if ui.button("Make Select").clicked() {
        compare::make_select(cx);
    }
    if ui.button("Swap").clicked() {
        compare::swap(cx);
    }
    if !roomy {
        return;
    }
    let (select, candidate) = compare::names(cx);
    let names = match (select, candidate) {
        (Some(select), Some(candidate)) => format!("{select}  vs  {candidate}"),
        (Some(select), None) => select,
        _ => String::new(),
    };
    // Long file names are cut short rather than crowd out the marks.
    ui.allocate_ui(vec2(NAMES, size::ROW), |ui| {
        ui.add(Label::new(RichText::new(names).color(palette::TEXT_DIM)).truncate());
    });
}

fn survey_note(ui: &mut Ui, cx: &Cx<'_>) {
    let count = cx.engine.selection().selected().len();
    let shown = count.min(survey::MAX_TILES);
    let note = format!("Surveying {}", text::count(shown, "photo", "photos"));
    ui.label(RichText::new(note).color(palette::TEXT_DIM));
}

/// A thin vertical line between groups of controls.
fn divider(ui: &mut Ui) {
    let (_, rect) = ui.allocate_space(vec2(9.0, size::ROW));
    let line = Stroke::new(1.0, palette::OUTLINE);
    ui.painter()
        .vline(rect.center().x, rect.y_range().shrink(3.0), line);
}
