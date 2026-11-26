//! The shell around the work area: the top bar (app name, activity and the
//! module picker) and the thin strips at the window's edges that hide and
//! show each edge (design §8.1).

use std::path::Path;

use egui::{Align, Button, Layout, Margin, ProgressBar, Rect, RichText, Ui, Vec2, vec2};
use photo_engine::{ExportOutcome, TaskKind, TaskState};

use super::Cx;
use super::actions::Action;
use super::dispatch;
use super::state::Module;
use crate::theme::{self, palette, space};
use crate::widgets::{Icon, IconButton};

/// Height of the top bar.
const TOP_BAR: f32 = 32.0;
/// Thickness of an edge strip.
const EDGE: f32 = 12.0;
/// Width of one task in the activity indicator.
const TASK_WIDTH: f32 = 150.0;

/// A window edge that can be hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Top,
    Left,
    Right,
    Bottom,
}

impl Edge {
    /// The icon and name of the button that hides or shows the edge, which
    /// say what it will do.
    pub(crate) fn button(self, shown: bool) -> (Icon, &'static str) {
        match (self, shown) {
            (Self::Top, true) => (Icon::ChevronUp, "Hide top bar"),
            (Self::Top, false) => (Icon::ChevronDown, "Show top bar"),
            (Self::Left, true) => (Icon::ChevronLeft, "Hide left panels"),
            (Self::Left, false) => (Icon::ChevronRight, "Show left panels"),
            (Self::Right, true) => (Icon::ChevronRight, "Hide right panels"),
            (Self::Right, false) => (Icon::ChevronLeft, "Show right panels"),
            (Self::Bottom, true) => (Icon::ChevronDown, "Hide filmstrip"),
            (Self::Bottom, false) => (Icon::ChevronUp, "Show filmstrip"),
        }
    }
}

/// The button that hides or shows `edge`, in a thin strip of its own.
/// Flips `shown` when clicked.
pub(crate) fn edge_strip(ui: &mut Ui, edge: Edge, shown: &mut bool) {
    let id = ("edge", format!("{edge:?}"));
    let frame = egui::Frame::new().fill(palette::WORK_AREA);
    let panel = match edge {
        Edge::Top => egui::Panel::top(id),
        Edge::Left => egui::Panel::left(id),
        Edge::Right => egui::Panel::right(id),
        Edge::Bottom => egui::Panel::bottom(id),
    };
    panel
        .exact_size(EDGE)
        .resizable(false)
        .show_separator_line(false)
        .frame(frame)
        .show(ui, |ui| {
            let (icon, name) = edge.button(*shown);
            let button = Rect::from_center_size(ui.max_rect().center(), Vec2::splat(EDGE));
            if ui
                .put(button, IconButton::new(icon, name).side(EDGE))
                .clicked()
            {
                *shown = !*shown;
            }
        });
}

/// The top bar, or its strip while it is hidden.
pub(crate) fn top_bar(ui: &mut Ui, cx: &mut Cx<'_>) {
    if !cx.ui.layout.top_bar {
        edge_strip(ui, Edge::Top, &mut cx.ui.layout.top_bar);
        return;
    }
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(12, 0));
    egui::Panel::top("top-bar")
        .exact_size(TOP_BAR)
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let name = RichText::new("Photo")
                    .font(theme::title_font())
                    .color(palette::TEXT_STRONG);
                ui.label(name);
                ui.add_space(space::LG);
                activity(ui, cx);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (icon, label) = Edge::Top.button(true);
                    if ui.add(IconButton::new(icon, label)).clicked() {
                        cx.ui.layout.top_bar = false;
                    }
                    ui.add_space(space::LG);
                    module_picker(ui, cx);
                });
            });
        });
}

/// Library and Develop: the module shown is lit.
fn module_picker(ui: &mut Ui, cx: &mut Cx<'_>) {
    ui.spacing_mut().item_spacing.x = space::SM;
    let current = cx.ui.module;
    // Right to left: the last module is added first.
    for (module, name) in [(Module::Develop, "Develop"), (Module::Library, "Library")] {
        let selected = current == module;
        let color = if selected {
            palette::TEXT_STRONG
        } else {
            palette::TEXT
        };
        let button = Button::selectable(selected, RichText::new(name).color(color))
            .min_size(vec2(64.0, 22.0));
        let hint = match module {
            Module::Library => "Organise the photos (G)",
            Module::Develop => "Develop the selected photo (D)",
        };
        if ui.add(button).on_hover_text(hint).clicked() && !selected {
            match module {
                Module::Library => {
                    let view = cx.ui.view;
                    dispatch::show_library(cx, view);
                }
                Module::Develop => dispatch::open_develop(cx),
            }
        }
    }
}

/// The running background tasks: what each is doing, how far it is, and a
/// button that cancels it. Scans are shown by the import dialog instead.
fn activity(ui: &mut Ui, cx: &mut Cx<'_>) {
    let running: Vec<_> = cx
        .engine
        .tasks()
        .iter()
        .filter(|task| task.state == TaskState::Running && task.kind != TaskKind::Scan)
        .cloned()
        .collect();
    for task in running {
        ui.label(RichText::new(&task.label).color(palette::TEXT_DIM).small());
        let fraction = if task.total == 0 {
            0.0
        } else {
            task.done as f32 / task.total as f32
        };
        let text = format!("{} of {}", task.done, task.total);
        ui.add(
            ProgressBar::new(fraction)
                .desired_width(TASK_WIDTH)
                .desired_height(12.0)
                .text(RichText::new(text).small()),
        );
        let cancel = format!("Cancel {}", task.label);
        if ui
            .add(IconButton::new(Icon::Close, &cancel).side(18.0))
            .clicked()
        {
            cx.act(Action::CancelTask(task.id));
        }
        ui.add_space(space::LG);
    }
    export_notice(ui, cx);
}

/// The export that ended last, until its notice is dismissed: how many
/// files it wrote, and a way to the folder they are in.
fn export_notice(ui: &mut Ui, cx: &mut Cx<'_>) {
    let ended = cx
        .engine
        .tasks()
        .iter()
        .rev()
        .find(|task| task.kind == TaskKind::Export && task.state.is_over())
        .map(|task| (task.id, task.total));
    let Some((task, total)) = ended else {
        return;
    };
    if cx.ui.dismissed_export == Some(task) {
        return;
    }
    let results = cx.engine.export_results(task).unwrap_or_default();
    let written: Vec<&Path> = results
        .iter()
        .filter_map(|exported| match &exported.result {
            Ok(ExportOutcome::Written { path, .. }) => Some(path.as_path()),
            _ => None,
        })
        .collect();
    let folder = written
        .first()
        .and_then(|path| path.parent())
        .map(Path::to_path_buf);
    let text = format!("Exported {} of {total}", written.len());
    ui.label(RichText::new(text).color(palette::TEXT_DIM).small());
    if let Some(folder) = folder
        && ui
            .add(Button::new(RichText::new("Show Exported Files").small()))
            .on_hover_text(format!("Open {}", folder.display()))
            .clicked()
    {
        (cx.reveal)(&folder);
    }
    if ui
        .add(IconButton::new(Icon::Close, "Dismiss the export notice").side(18.0))
        .clicked()
    {
        cx.ui.dismissed_export = Some(task);
    }
}
