//! What each keyboard shortcut does: one engine action, or a change of what
//! is shown. Most keys mean the same in both modules; the photo keys apply
//! to the selection in the grid and to the photo looked at elsewhere, and
//! Develop has keys of its own.

use photo_core::meta::Rating;
use photo_engine::{CollectionKind, Movement, Source, Target};

use super::Cx;
use super::actions::Action;
use super::develop::actions::DevelopAction;
use super::develop::{self, BeforeAfter, canvas};
use super::dialogs;
use super::input::{Arrow, Shortcut};
use super::library::{compare, filter_bar};
use super::state::{Module, View};

/// Carry out `shortcut`.
pub(crate) fn shortcut(cx: &mut Cx<'_>, ctx: &egui::Context, shortcut: Shortcut) {
    let target = target(cx);
    let in_develop = cx.ui.module == Module::Develop;
    match shortcut {
        Shortcut::View(view) => show_library(cx, view),
        Shortcut::Develop => open_develop(cx),
        Shortcut::ToggleZoom if in_develop => canvas::toggle_zoom(cx, ctx),
        Shortcut::ToggleZoom => match cx.ui.view {
            View::Loupe => cx.ui.loupe.toggle(),
            View::Grid => open_view(cx, View::Loupe),
            View::Compare | View::Survey => {}
        },
        Shortcut::Rating(stars) => cx.act(Action::SetRating(target, Rating::new(stars))),
        Shortcut::RatingDown => cx.act(Action::DecreaseRating(target)),
        Shortcut::RatingUp => cx.act(Action::IncreaseRating(target)),
        Shortcut::Flag(flag) => cx.act(Action::SetFlag(target, flag)),
        Shortcut::Label(label) => cx.act(Action::ToggleLabel(target, label)),
        Shortcut::QuickCollection => cx.act(Action::ToggleQuickCollection(target)),
        Shortcut::Arrow { arrow, extend } => arrow_key(cx, arrow, extend),
        Shortcut::First => cx.act(Action::MoveActive {
            movement: Movement::First,
            extend: false,
        }),
        Shortcut::Last => cx.act(Action::MoveActive {
            movement: Movement::Last,
            extend: false,
        }),
        Shortcut::Undo => cx.act(Action::Undo),
        Shortcut::Redo => cx.act(Action::Redo),
        Shortcut::SelectAll => cx.act(Action::SelectAll),
        Shortcut::SelectNone => cx.act(Action::SelectNone),
        Shortcut::Delete => delete(cx),
        Shortcut::RotateLeft => cx.act(Action::RotateLeft(target)),
        Shortcut::RotateRight => cx.act(Action::RotateRight(target)),
        Shortcut::VirtualCopy => cx.act(Action::CreateVirtualCopies(target)),
        Shortcut::Import => dialogs::open_import(cx),
        Shortcut::New if in_develop => dialogs::open_new_snapshot(cx),
        Shortcut::New => dialogs::open_new_collection(cx, None),
        Shortcut::FindText => {
            show_library(cx, View::Grid);
            filter_bar::focus_text(cx.ui);
        }
        Shortcut::ToggleSides => cx.ui.layout.toggle_sides(),
        Shortcut::ToggleEdges => cx.ui.layout.toggle_edges(),
        Shortcut::ToggleToolbar => cx.ui.layout.toolbar = !cx.ui.layout.toolbar,
        Shortcut::Before => cx.ui.develop.toggle_before_after(BeforeAfter::Before),
        Shortcut::SideBySide => cx.ui.develop.toggle_before_after(BeforeAfter::SideBySide),
        Shortcut::Split => cx.ui.develop.toggle_before_after(BeforeAfter::Split),
        Shortcut::Clipping => cx.ui.develop.toggle_clipping(),
        Shortcut::WhiteBalancePicker => toggle_picker(cx),
        Shortcut::CopySettings => dialogs::open_copy_settings(cx),
        Shortcut::PasteSettings => {
            cx.act(Action::Develop(DevelopAction::PasteSettings(Target::Selected)));
        }
        Shortcut::ResetSettings => {
            let reset = if in_develop { Target::Active } else { target };
            cx.act(Action::Develop(DevelopAction::ResetSettings(reset)));
        }
    }
    ctx.request_repaint();
}

/// The photos a photo key applies to: the selection in the grid, the photo
/// looked at in the other views and in Develop.
fn target(cx: &Cx<'_>) -> Target {
    match cx.ui.module {
        Module::Library => cx.ui.view.target(),
        Module::Develop => Target::Active,
    }
}

/// D, or the module picker: develop the active photo (the first one when
/// none is active).
pub(crate) fn open_develop(cx: &mut Cx<'_>) {
    if cx.ui.module == Module::Develop {
        return;
    }
    if cx.engine.photo_count() == 0 {
        cx.say("Import photos to develop them.");
        return;
    }
    if cx.engine.active_photo().is_none() {
        cx.act(Action::MoveActive {
            movement: Movement::First,
            extend: false,
        });
    }
    cx.ui.module = Module::Develop;
    develop::follow_active(cx);
}

/// Show `view` of the Library, leaving Develop if it is shown.
pub(crate) fn show_library(cx: &mut Cx<'_>, view: View) {
    if cx.ui.module == Module::Develop {
        develop::close(cx);
        cx.ui.module = Module::Library;
    }
    open_view(cx, view);
}

/// Show `view`. A view of one photo with no photo active starts at the
/// first photo of the list.
pub(crate) fn open_view(cx: &mut Cx<'_>, view: View) {
    if view != View::Grid && cx.engine.active_photo().is_none() && cx.engine.photo_count() > 0 {
        cx.act(Action::MoveActive {
            movement: Movement::First,
            extend: false,
        });
    }
    if view == View::Compare && cx.ui.view != View::Compare {
        compare::reset(cx);
    }
    cx.ui.view = view;
}

/// W: the white-balance picker on or off, once the photo can be edited.
fn toggle_picker(cx: &mut Cx<'_>) {
    let ready = cx.engine.develop().is_some_and(|session| {
        session.output_size().is_some() && session.read_only_reason().is_none()
    });
    let develop = &mut cx.ui.develop;
    develop.picker = ready && !develop.picker;
}

fn arrow_key(cx: &mut Cx<'_>, arrow: Arrow, extend: bool) {
    let view = match cx.ui.module {
        Module::Library => cx.ui.view,
        // Develop's filmstrip moves the way the loupe's does.
        Module::Develop => View::Loupe,
    };
    let step = match (view, arrow) {
        (View::Compare, Arrow::Left | Arrow::Up) => return compare::step_candidate(cx, -1),
        (View::Compare, Arrow::Right | Arrow::Down) => return compare::step_candidate(cx, 1),
        (View::Survey, Arrow::Left | Arrow::Up) => return step_within_selection(cx, -1),
        (View::Survey, Arrow::Right | Arrow::Down) => return step_within_selection(cx, 1),
        (_, Arrow::Left) => -1,
        (_, Arrow::Right) => 1,
        (View::Grid, Arrow::Up) => -(cx.ui.grid.columns.max(1) as isize),
        (View::Grid, Arrow::Down) => cx.ui.grid.columns.max(1) as isize,
        (_, Arrow::Up) => -1,
        (_, Arrow::Down) => 1,
    };
    cx.act(Action::MoveActive {
        movement: Movement::Step(step),
        extend,
    });
}

/// Survey: make the selected photo before or after the active one active.
fn step_within_selection(cx: &mut Cx<'_>, step: isize) {
    let selected = cx.engine.selection().selected();
    let Some(at) = cx
        .engine
        .active_photo()
        .and_then(|active| selected.iter().position(|photo| *photo == active))
    else {
        return;
    };
    let next = at
        .saturating_add_signed(step)
        .min(selected.len().saturating_sub(1));
    if let Some(photo) = selected.get(next).copied() {
        cx.act(Action::MakeActive(photo));
    }
}

/// Delete: in a collection, take the photos out of it; anywhere else, ask
/// before removing them from the catalog.
fn delete(cx: &mut Cx<'_>) {
    let target = target(cx);
    let collection = match *cx.engine.source() {
        Source::QuickCollection => Some(cx.engine.quick_collection()),
        Source::Collection(id) => cx
            .engine
            .collections()
            .iter()
            .any(|collection| collection.id == id && collection.kind == CollectionKind::Regular)
            .then_some(id),
        _ => None,
    };
    match collection {
        Some(collection) => cx.act(Action::RemoveFromCollection(collection, target)),
        None => dialogs::open_remove_photos(cx, target),
    }
}
