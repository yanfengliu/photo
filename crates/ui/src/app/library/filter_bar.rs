//! The Library filter bar above the grid: Text, Attribute, Metadata and
//! None. Each tab opens its own row of conditions; conditions set on one
//! tab stay in force while another is open, and None turns them all off.
//! The engine holds the filter; the bar only edits a copy of it and hands
//! the result back.

use egui::{Align, Button, ComboBox, Layout, Margin, RichText, TextEdit, Ui, vec2};
use photo_core::meta::{FileKind, Flag, Rating};
use photo_engine::{Comparison, CopyFilter, Filter, RatingFilter};

use super::filter_metadata;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::state::UiState;
use crate::theme::{self, palette, space};
use crate::widgets::{Icon, IconButton, color_label_set, rating_stars};

/// Height of the row of tabs.
const TABS: f32 = 28.0;
/// Height of a row of conditions.
const ROW: f32 = 34.0;
/// Below this width the attribute conditions take two rows.
const ATTRIBUTES_IN_ONE_ROW: f32 = 960.0;

/// The filter bar's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FilterTab {
    Text,
    Attribute,
    Metadata,
}

/// What the filter bar remembers between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilterBarState {
    /// The open tab, if any.
    pub(crate) tab: Option<FilterTab>,
    /// The search text as typed.
    pub(crate) text: String,
    /// Give the search field keyboard focus this frame.
    pub(crate) focus_text: bool,
}

/// Command-F: open the Text tab with the keyboard in its field.
pub(crate) fn focus_text(ui: &mut UiState) {
    ui.filter_bar.tab = Some(FilterTab::Text);
    ui.filter_bar.focus_text = true;
}

/// Show the filter bar.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let one_row = ui.available_width() >= ATTRIBUTES_IN_ONE_ROW;
    let height = TABS
        + match cx.ui.filter_bar.tab {
            None => 0.0,
            Some(FilterTab::Text) => ROW,
            Some(FilterTab::Attribute) if one_row => ROW,
            Some(FilterTab::Attribute) => 2.0 * ROW,
            Some(FilterTab::Metadata) => filter_metadata::HEIGHT,
        };
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(10, 0));
    egui::Panel::top("filter-bar")
        .exact_size(height)
        .resizable(false)
        .frame(frame)
        .show(ui, |ui| {
            tabs(ui, cx);
            let mut filter = cx.engine.filter().clone();
            match cx.ui.filter_bar.tab {
                None => {}
                Some(FilterTab::Text) => text(ui, cx, &mut filter),
                Some(FilterTab::Attribute) => attributes(ui, &mut filter, one_row),
                Some(FilterTab::Metadata) => filter_metadata::show(ui, cx, &mut filter),
            }
            if &filter != cx.engine.filter() {
                cx.act(Action::SetFilter(filter));
            }
        });
}

/// "Library Filter: Text Attribute Metadata None", centred.
fn tabs(ui: &mut Ui, cx: &mut Cx<'_>) {
    let filter = cx.engine.filter();
    let in_force = [
        (FilterTab::Text, !filter.text.trim().is_empty()),
        (FilterTab::Attribute, has_attributes(filter)),
        (
            FilterTab::Metadata,
            !filter.cameras.is_empty()
                || !filter.lenses.is_empty()
                || filter.iso.is_some()
                || filter.capture.is_some(),
        ),
    ];
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), TABS));
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let width = 320.0;
    row.add_space(((rect.width() - width) / 2.0).max(0.0));
    row.label(RichText::new("Library Filter:").color(palette::TEXT_DIM));
    row.add_space(space::SM);
    for (tab, active) in in_force {
        let (label, name) = match tab {
            FilterTab::Text => ("Text", "Text filter"),
            FilterTab::Attribute => ("Attribute", "Attribute filter"),
            FilterTab::Metadata => ("Metadata", "Metadata filter"),
        };
        let open = cx.ui.filter_bar.tab == Some(tab);
        let color = if open || active {
            palette::TEXT_STRONG
        } else {
            palette::TEXT
        };
        let text = if active {
            RichText::new(label).color(color).underline()
        } else {
            RichText::new(label).color(color)
        };
        if row
            .add(Button::selectable(open, text))
            .accessible_name(name)
            .clicked()
        {
            cx.ui.filter_bar.tab = if open { None } else { Some(tab) };
        }
    }
    let none = cx.ui.filter_bar.tab.is_none() && cx.engine.filter().is_empty();
    if row
        .add(Button::selectable(none, "None"))
        .accessible_name("No filter")
        .clicked()
    {
        cx.ui.filter_bar.tab = None;
        cx.ui.filter_bar.text.clear();
        cx.act(Action::SetFilter(Filter::default()));
    }
}

/// Whether any attribute condition is set.
fn has_attributes(filter: &Filter) -> bool {
    !filter.flags.is_empty()
        || filter.rating.is_some()
        || !filter.labels.is_empty()
        || filter.kind.is_some()
        || filter.copies.is_some()
        || filter.edited.is_some()
}

/// The search field: every word must be found in the file name, title,
/// caption, a keyword, the camera or the lens.
fn text(ui: &mut Ui, cx: &mut Cx<'_>, filter: &mut Filter) {
    ui.horizontal_centered(|ui| {
        ui.label(RichText::new("Search").color(palette::TEXT_DIM));
        let state = &mut cx.ui.filter_bar;
        let response = ui
            .add(
                TextEdit::singleline(&mut state.text)
                    .id_salt("filter-text")
                    .hint_text("File name, title, caption, keyword, camera or lens")
                    .desired_width(420.0),
            )
            .accessible_name("Search text");
        if std::mem::take(&mut state.focus_text) {
            response.request_focus();
        }
        if response.changed() {
            filter.text = state.text.clone();
        } else if !response.has_focus() && state.text != filter.text {
            // Changed elsewhere (None, undo of nothing, another window).
            state.text = filter.text.clone();
        }
    });
}

/// Flag, rating, colour label, file kind, copies and edits: in one row, or
/// in a narrow window, the marks in one row and the kinds in another.
fn attributes(ui: &mut Ui, filter: &mut Filter, one_row: bool) {
    if one_row {
        ui.horizontal_centered(|ui| {
            marks(ui, filter);
            ui.add_space(space::XL);
            kinds(ui, filter);
        });
        return;
    }
    let row = vec2(ui.available_width(), ROW);
    let layout = Layout::left_to_right(Align::Center);
    ui.allocate_ui_with_layout(row, layout, |ui| marks(ui, filter));
    ui.allocate_ui_with_layout(row, layout, |ui| kinds(ui, filter));
}

/// The flag, rating and colour-label conditions.
fn marks(ui: &mut Ui, filter: &mut Filter) {
    ui.label(RichText::new("Flag").color(palette::TEXT_DIM));
    for (flag, icon, name) in [
        (Flag::Picked, Icon::FlagPick, "Show picked"),
        (Flag::Unflagged, Icon::FlagNone, "Show unflagged"),
        (Flag::Rejected, Icon::FlagReject, "Show rejected"),
    ] {
        let on = filter.flags.contains(&flag);
        if ui.add(IconButton::new(icon, name).selected(on)).clicked() {
            if on {
                filter.flags.retain(|other| *other != flag);
            } else {
                filter.flags.push(flag);
            }
        }
    }
    ui.add_space(space::XL);
    rating(ui, filter);
    ui.add_space(space::XL);
    ui.label(RichText::new("Color").color(palette::TEXT_DIM));
    color_label_set(ui, "Filter label", &mut filter.labels);
}

/// The file kind, copies and edits conditions.
fn kinds(ui: &mut Ui, filter: &mut Filter) {
    choice(
        ui,
        "Kind",
        "File kind filter",
        &mut filter.kind,
        &[
            (None, "All"),
            (Some(FileKind::Raw), "RAW"),
            (Some(FileKind::Raster), "Raster"),
        ],
    );
    choice(
        ui,
        "Copies",
        "Copies filter",
        &mut filter.copies,
        &[
            (None, "All"),
            (Some(CopyFilter::Masters), "Masters"),
            (Some(CopyFilter::VirtualCopies), "Virtual copies"),
        ],
    );
    choice(
        ui,
        "Edits",
        "Edited filter",
        &mut filter.edited,
        &[
            (None, "All"),
            (Some(true), "Edited"),
            (Some(false), "Unedited"),
        ],
    );
}

/// "Rating [at least] ★★★☆☆": no stars is no rating condition.
fn rating(ui: &mut Ui, filter: &mut Filter) {
    ui.label(RichText::new("Rating").color(palette::TEXT_DIM));
    let current = filter.rating;
    let mut comparison = current.map_or(Comparison::AtLeast, |rating| rating.comparison);
    let mut stars = current.map_or(Rating::UNRATED, |rating| rating.rating);
    let name = |comparison| match comparison {
        Comparison::AtLeast => "at least",
        Comparison::AtMost => "at most",
        Comparison::Exactly => "exactly",
    };
    ComboBox::from_id_salt("rating-comparison")
        .selected_text(name(comparison))
        .width(76.0)
        .show_ui(ui, |ui| {
            for option in [Comparison::AtLeast, Comparison::AtMost, Comparison::Exactly] {
                ui.selectable_value(&mut comparison, option, name(option));
            }
        })
        .response
        .accessible_name("Rating comparison");
    rating_stars(ui, "Filter rating", &mut stars);
    filter.rating = (stars != Rating::UNRATED).then_some(RatingFilter {
        comparison,
        rating: stars,
    });
}

/// A labelled menu of a few `options` for one condition.
fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    label: &str,
    name: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    ui.label(RichText::new(label).color(palette::TEXT_DIM));
    let shown = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or("All", |(_, text)| *text);
    ComboBox::from_id_salt(name)
        .selected_text(shown)
        .width(96.0)
        .show_ui(ui, |ui| {
            for (option, text) in options {
                ui.selectable_value(value, *option, *text);
            }
        })
        .response
        .accessible_name(name);
    ui.add_space(space::MD);
}
