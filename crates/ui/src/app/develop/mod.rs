//! The Develop module (design §8.3): the photo canvas between the left
//! column (Navigator, Presets, Snapshots, History; Copy and Paste) and the
//! right (Histogram and the editing panels; Previous and Reset), with the
//! toolbar under the canvas and the filmstrip across the bottom.
//!
//! ```text
//! ┌──────────────── top bar ─────────────────┐
//! │ Navigator │ canvas             │ Histogram│
//! │ Presets   │                    │ Basic    │
//! │ Snapshots │                    │ Tone ... │
//! │ History   │ toolbar            │          │
//! │ Copy Paste│                    │ Prev Rst │
//! ├───────────┴────────────────────┴──────────┤
//! │ source line · filmstrip                   │
//! └───────────────────────────────────────────┘
//! ```
//!
//! The module edits the active photo: when the active photo changes (the
//! filmstrip, the arrow keys), the engine is asked to open it, which
//! commits any edit of the photo before.

pub(crate) mod actions;
pub(crate) mod canvas;
pub(crate) mod drafts;
pub(crate) mod frames;
mod left;
pub(crate) mod right;
pub(crate) mod state;
mod toolbar;
pub(crate) mod tools;

use egui::{Color32, Id, Key, Order, Rect, Sense, Ui, pos2};

use self::actions::DevelopAction;
pub use self::canvas::CanvasZoom;
pub(crate) use self::frames::Frames;
pub use self::frames::{CanvasImage, CanvasShows};
pub(crate) use self::state::DevelopState;
pub use self::state::{BeforeAfter, Lights};
pub use self::tools::{MaskKind, Tool};
use self::state::{LEFT_KEY, RIGHT_KEY, Told, encode_panels};
use super::Cx;
use super::actions::Action;
use super::library::filmstrip;
use super::shell::{Edge, edge_strip};
use crate::theme;

/// Lay the Develop module out in what the top bar left of `ui`. Returns the
/// work area's rect, which toasts are shown over.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) -> Rect {
    if cx.ui.develop.picker && ui.input(|input| input.key_pressed(Key::Escape)) {
        cx.ui.develop.picker = false;
    }
    follow_active(cx);
    tools::sync(cx);
    cx.frames.sync(ui.ctx(), cx.engine.develop());
    let open = cx.engine.develop().map(|session| session.photo());
    let generation = cx.generation;
    cx.ui.develop.lists.refresh(cx.engine, open, generation);

    if cx.ui.layout.filmstrip {
        filmstrip::show(ui, cx);
    } else {
        edge_strip(ui, Edge::Bottom, &mut cx.ui.layout.filmstrip);
    }
    edge_strip(ui, Edge::Left, &mut cx.ui.layout.left);
    edge_strip(ui, Edge::Right, &mut cx.ui.layout.right);
    if cx.ui.layout.left {
        left::show(ui, cx);
    } else {
        cx.ui.develop.hovered_preset = None;
    }
    if cx.ui.layout.right {
        right::show(ui, cx);
    }
    if cx.ui.layout.toolbar {
        toolbar::show(ui, cx);
    }
    let work_area = egui::CentralPanel::default()
        .frame(theme::work_area_frame())
        .show(ui, |ui| {
            canvas::show(ui, cx);
            ui.max_rect()
        })
        .inner;
    cx.ui.grid.reveal_active = false;
    lights_out(ui.ctx(), cx);
    drafts::pace(cx);
    tell(cx);
    store_panels(cx);
    work_area
}

/// L: everything but the photo dimmed or black. The dark takes the
/// pointer, and a click on it turns the lights back on.
fn lights_out(ctx: &egui::Context, cx: &mut Cx<'_>) {
    let alpha = match cx.ui.develop.lights {
        Lights::On => return,
        Lights::Dim => 215,
        Lights::Out => 255,
    };
    let screen = ctx.content_rect();
    let hole = match (cx.ui.develop.photo_rect, cx.ui.develop.stage) {
        (Some(photo), Some(stage)) => photo.intersect(stage),
        _ => Rect::NOTHING,
    };
    let parts = if hole.is_positive() {
        vec![
            Rect::from_min_max(screen.min, pos2(screen.right(), hole.top())),
            Rect::from_min_max(pos2(screen.left(), hole.bottom()), screen.max),
            Rect::from_min_max(
                pos2(screen.left(), hole.top()),
                pos2(hole.left(), hole.bottom()),
            ),
            Rect::from_min_max(
                pos2(hole.right(), hole.top()),
                pos2(screen.right(), hole.bottom()),
            ),
        ]
    } else {
        vec![screen]
    };
    let dark = Color32::from_black_alpha(alpha);
    egui::Area::new(Id::unique("develop-lights-out"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            for (index, part) in parts.into_iter().enumerate() {
                if !part.is_positive() {
                    continue;
                }
                ui.painter().rect_filled(part, 0.0, dark);
                let id = Id::unique(("develop-lights-out", index));
                if ui.interact(part, id, Sense::click()).clicked() {
                    cx.ui.develop.lights = Lights::On;
                }
            }
        });
}

/// Open the active photo if it is not the one open, or close the session
/// when no photo is active. The engine commits an edit of the photo that
/// was open before.
pub(crate) fn follow_active(cx: &mut Cx<'_>) {
    let active = cx.engine.active_photo();
    let open = cx.engine.develop().map(|session| session.photo());
    let told = cx.ui.develop.told.photo;
    if active == open {
        if told != open {
            cx.ui.develop.photo_changed();
            cx.ui.develop.told = Told::fresh(open);
        }
        return;
    }
    if told == active {
        // Asked already (and refused, or not applied yet).
        return;
    }
    // A value still waiting belongs to the photo open now, which commits
    // it when the next one opens.
    drafts::flush(cx);
    cx.ui.develop.photo_changed();
    cx.ui.develop.told = Told::fresh(active);
    match active {
        Some(photo) => cx.act(Action::Develop(DevelopAction::Open(photo))),
        None => cx.act(Action::Develop(DevelopAction::Close)),
    }
}

/// Close the session, for leaving the module.
pub(crate) fn close(cx: &mut Cx<'_>) {
    tools::put_away(cx);
    drafts::flush(cx);
    cx.frames.forget();
    cx.ui.develop.lights = Lights::On;
    cx.ui.develop.photo_changed();
    cx.ui.develop.told = Told::default();
    if cx.engine.develop().is_some() {
        cx.act(Action::Develop(DevelopAction::Close));
    }
}

/// Tell the engine what the before view should show and which preset to
/// preview, when that changed.
fn tell(cx: &mut Cx<'_>) {
    let open = cx.engine.develop().map(|session| session.photo());
    if open.is_none() || open != cx.ui.develop.told.photo {
        return;
    }
    let develop = &mut cx.ui.develop;
    let before = develop
        .before_after
        .shows_before()
        .then_some(develop.before_choice);
    let look = develop.hovered_preset;
    let (width, height) = develop.navigator_box;
    let mut asks = Vec::new();
    if develop.told.before != Some(before) {
        develop.told.before = Some(before);
        asks.push(DevelopAction::SetBefore(before));
    }
    if develop.told.look != Some(look) {
        develop.told.look = Some(look);
        asks.push(DevelopAction::PreviewPreset(look, width, height));
    }
    for ask in asks {
        cx.act(Action::Develop(ask));
    }
}

/// Keep the panel columns' open state in the catalog, written when it
/// changes.
fn store_panels(cx: &mut Cx<'_>) {
    let develop = &mut cx.ui.develop;
    let left = encode_panels(&develop.left_panels);
    let right = encode_panels(&develop.right_panels);
    for (key, now, stored) in [
        (LEFT_KEY, left, &mut develop.stored_panels.0),
        (RIGHT_KEY, right, &mut develop.stored_panels.1),
    ] {
        if now != *stored {
            if let Err(error) = cx.engine.set_ui_setting(key, &now) {
                log::info!("the panel layout is not kept: {error}");
            }
            *stored = now;
        }
    }
}
