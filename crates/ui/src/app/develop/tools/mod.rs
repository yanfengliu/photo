//! The Develop tools (design §8.3): crop and straighten (R), healing (Q)
//! and masking (M a linear gradient, Shift+M a radial one, K the brush).
//!
//! One tool is open at a time, from the strip under the histogram or its
//! key. What a tool changes goes to the engine as drafts of one step: Enter
//! (or the tool's key again, another tool, Done) records it, Escape drops
//! it. The tools keep nothing of the photo themselves: each frame they read
//! the crop, the spots and the masks from the settings shown, so whatever
//! else changes those (undo, a preset, another photo) is what they show.
//!
//! A change made elsewhere while a tool has drafts out (a slider of a
//! panel, a preset) first records the tool's step, so the two stay two
//! steps (see `PhotoApp::apply_actions`).

pub(crate) mod crop;
pub(crate) mod heal;
pub(crate) mod mask;

use egui::{Align, Button, Layout, Margin, Painter, Pos2, Response, Ui, vec2};
use photo_core::develop::{DevelopSettings, Mask};
use photo_core::meta::PhotoId;

use self::mask::Armed;
use super::actions::DevelopAction;
use super::canvas::map::CanvasMap;
use super::drafts::{self, Pending};
use super::right::Editing;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::theme::{self, palette, space};
use crate::widgets::{Icon, IconButton};

/// A Develop tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Crop and straighten (R).
    Crop,
    /// Healing spots (Q).
    Heal,
    /// Masks (M, Shift+M, K).
    Mask,
}

/// The kinds of shape a mask is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaskKind {
    /// A linear gradient (M).
    Linear,
    /// A radial gradient (Shift+M).
    Radial,
    /// Brush strokes (K).
    Brush,
}

/// What a tool key, or a button of the strip, asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ToolKey {
    Crop,
    Heal,
    Mask(MaskKind),
}

/// The strip's buttons: what each asks for, its icon, its name and its key.
const STRIP: [(ToolKey, Icon, &str, &str); 5] = [
    (ToolKey::Crop, Icon::Crop, "Crop", "Crop and straighten (R)"),
    (ToolKey::Heal, Icon::Heal, "Healing", "Remove spots (Q)"),
    (
        ToolKey::Mask(MaskKind::Linear),
        Icon::LinearMask,
        "Linear gradient mask",
        "Mask with a linear gradient (M)",
    ),
    (
        ToolKey::Mask(MaskKind::Radial),
        Icon::RadialMask,
        "Radial gradient mask",
        "Mask with a radial gradient (Shift+M)",
    ),
    (
        ToolKey::Mask(MaskKind::Brush),
        Icon::Brush,
        "Brush mask",
        "Mask with the brush (K)",
    ),
];

/// What the tools remember between frames.
#[derive(Default)]
pub(crate) struct Tools {
    pub(crate) open: Option<Tool>,
    /// A tool has changed the photo and its step is not recorded yet.
    pub(crate) dirty: bool,
    pub(crate) crop: crop::CropTool,
    pub(crate) heal: heal::HealTool,
    pub(crate) mask: mask::MaskTool,
    /// The photo the selections belong to.
    photo: Option<PhotoId>,
    /// Where the crop rectangle was drawn in the last frame, in points.
    pub(crate) crop_rect: Option<egui::Rect>,
}

/// What the tools edit: the open photo's settings as shown, a draft that
/// waits for its turn included. `None` when the active photo is not open.
pub(crate) fn editing(cx: &Cx<'_>) -> Option<Editing> {
    let pending = cx.ui.develop.pending.as_ref();
    cx.engine
        .develop()
        .filter(|session| Some(session.photo()) == cx.engine.active_photo())
        .map(|session| Editing::of(session, pending))
}

/// The tool that takes the canvas, if one is open and the photo is decoded
/// and can be edited.
pub(crate) fn active(cx: &Cx<'_>) -> Option<Tool> {
    let ready = cx.engine.develop().is_some_and(|session| {
        session.output_size().is_some()
            && session.read_only_reason().is_none()
            && Some(session.photo()) == cx.engine.active_photo()
    });
    cx.ui.develop.tools.open.filter(|_| ready)
}

/// Hand `settings` to the engine as a draft of the open tool's step.
pub(crate) fn edit(cx: &mut Cx<'_>, settings: DevelopSettings) {
    cx.ui.develop.tools.dirty = true;
    drafts::draft(cx, Pending::Tool(Box::new(settings)));
}

/// Record what the open tool changed as one step. The tool stays open.
pub(crate) fn commit(cx: &mut Cx<'_>) {
    if cx.ui.develop.tools.open.is_none() {
        return;
    }
    drafts::flush(cx);
    if std::mem::take(&mut cx.ui.develop.tools.dirty) {
        cx.act(Action::Develop(DevelopAction::ToolCommit));
    }
}

/// Drop what the open tool changed and has not recorded. The tool stays
/// open. Returns whether there was anything to drop.
pub(crate) fn revert(cx: &mut Cx<'_>) -> bool {
    let develop = &mut cx.ui.develop;
    if develop.pending.as_ref().is_some_and(Pending::is_tool) {
        develop.pending = None;
    }
    develop.tools.crop.forget();
    develop.tools.heal.drag = None;
    develop.tools.mask.drag = None;
    let dirty = std::mem::take(&mut develop.tools.dirty);
    if dirty {
        cx.act(Action::Develop(DevelopAction::ToolCancel));
    }
    dirty
}

/// Close the open tool without recording or dropping anything.
fn close(cx: &mut Cx<'_>) {
    let tools = &mut cx.ui.develop.tools;
    tools.open = None;
    tools.crop.forget();
    tools.crop_rect = None;
    tools.heal.drag = None;
    (tools.mask.armed, tools.mask.drag) = (None, None);
}

/// Enter, Done, the tool's key again or another tool: record the tool's
/// step and put it away.
pub(crate) fn put_away(cx: &mut Cx<'_>) {
    commit(cx);
    close(cx);
}

/// Escape, Discard: drop what the tool changed and put it away.
pub(crate) fn discard(cx: &mut Cx<'_>) {
    revert(cx);
    close(cx);
}

/// A tool key or a button of the strip: open the tool (putting away the
/// one open), or put it away if it is the one open. A mask key arms the
/// shape the next drag on the photo makes.
pub(crate) fn press(cx: &mut Cx<'_>, key: ToolKey) {
    // A photo only just made active has no session yet; the tool opens for
    // it all the same, and is put away again if it turns out read-only.
    let read_only = cx
        .engine
        .develop()
        .is_some_and(|session| session.read_only_reason().is_some());
    if read_only || cx.engine.photo_count() == 0 {
        cx.say("Open a photo that can be edited to use the tools.");
        return;
    }
    let (tool, kind) = match key {
        ToolKey::Crop => (Tool::Crop, None),
        ToolKey::Heal => (Tool::Heal, None),
        ToolKey::Mask(kind) => (Tool::Mask, Some(kind)),
    };
    let tools = &cx.ui.develop.tools;
    let armed = kind.map(|kind| Armed { kind, into: None });
    let again = tools.open == Some(tool) && (armed.is_none() || tools.mask.armed == armed);
    if again {
        put_away(cx);
        return;
    }
    if tools.open != Some(tool) {
        put_away(cx);
        cx.ui.develop.tools.open = Some(tool);
        cx.ui.develop.picker = false;
    }
    if armed.is_some() {
        cx.ui.develop.tools.mask.armed = armed;
    }
}

/// O: the red overlay of the selected mask on or off.
pub(crate) fn toggle_overlay(cx: &mut Cx<'_>) {
    let tool = &mut cx.ui.develop.tools.mask;
    tool.overlay = !tool.overlay;
}

/// Delete with a tool open: the selected spot or mask. Returns false when
/// no tool takes the key.
pub(crate) fn delete_selected(cx: &mut Cx<'_>) -> bool {
    match cx.ui.develop.tools.open {
        Some(Tool::Heal) => heal::delete_selected(cx),
        Some(Tool::Mask) => mask::delete_selected(cx),
        Some(Tool::Crop) | None => return false,
    }
    true
}

/// Once a frame, before anything is drawn: forget what belonged to another
/// photo, put the tools away from a photo that cannot be edited, and keep
/// the engine showing the mask overlay that is wanted.
pub(crate) fn sync(cx: &mut Cx<'_>) {
    let session = cx.engine.develop();
    let photo = session.map(|session| session.photo());
    let read_only = session.is_some_and(|session| session.read_only_reason().is_some());
    let drafted = session.is_some_and(|session| session.is_editing());
    let waiting = cx.ui.develop.pending.as_ref().is_some_and(Pending::is_tool);
    let tools = &mut cx.ui.develop.tools;
    if tools.photo != photo {
        tools.photo = photo;
        tools.crop.forget();
        tools.heal.forget();
        tools.mask.forget();
        tools.dirty = false;
    }
    if tools.dirty && !waiting && !drafted {
        // The engine holds nothing uncommitted: it was recorded or dropped
        // from elsewhere (another photo was opened, an undo).
        tools.dirty = false;
    }
    if read_only && tools.open.is_some() {
        close(cx);
    }
    let masking = cx.ui.develop.tools.open == Some(Tool::Mask);
    mask::tell_overlay(cx, masking);
}

/// The strip of tool buttons, under the histogram.
pub(crate) fn strip(ui: &mut Ui, cx: &mut Cx<'_>, editing: Option<&Editing>) {
    let enabled = editing.is_some_and(Editing::editable);
    let masks: &[Mask] = editing.map_or(&[], |editing| &editing.settings.masks);
    let mut pressed = None;
    egui::Frame::new()
        .fill(palette::HEADER)
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::MD;
                for (key, icon, name, hint) in STRIP {
                    let tools = &cx.ui.develop.tools;
                    let lit = match key {
                        ToolKey::Crop => tools.open == Some(Tool::Crop),
                        ToolKey::Heal => tools.open == Some(Tool::Heal),
                        ToolKey::Mask(kind) => {
                            tools.open == Some(Tool::Mask) && tools.mask.kind(masks) == Some(kind)
                        }
                    };
                    let button = IconButton::new(icon, name).selected(lit);
                    if ui
                        .add_enabled(enabled, button)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        pressed = Some(key);
                    }
                }
            });
        });
    if let Some(key) = pressed {
        press(cx, key);
    }
}

/// The open tool's panel, under the strip: its controls, then Discard and
/// Done.
pub(crate) fn panel(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let Some(tool) = cx.ui.develop.tools.open else {
        return;
    };
    let title = match tool {
        Tool::Crop => "Crop & Straighten",
        Tool::Heal => "Healing",
        Tool::Mask => "Masking",
    };
    let margin = Margin {
        left: space::LG as i8,
        right: space::LG as i8,
        top: space::MD as i8,
        bottom: space::LG as i8,
    };
    egui::Frame::new()
        .fill(theme::mix(palette::PANEL, palette::WELL, 0.45))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(space::MD, space::XS);
            ui.label(theme::header_text(title, palette::TEXT));
            ui.add_space(space::SM);
            ui.add_enabled_ui(editing.editable(), |ui| match tool {
                Tool::Crop => crop::panel(ui, cx, editing),
                Tool::Heal => heal::panel(ui, cx, editing),
                Tool::Mask => mask::panel(ui, cx, editing),
            });
            ui.add_space(space::LG);
            // In a row of its own, or the right-to-left layout would take
            // all the height the column has left.
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let done = Button::new("Done")
                        .fill(palette::ACCENT_DIM)
                        .min_size(vec2(64.0, 22.0));
                    if ui
                        .add(done)
                        .on_hover_text("Keep what the tool changed, as one step (Enter)")
                        .clicked()
                    {
                        put_away(cx);
                    }
                    if ui
                        .add(Button::new("Discard").min_size(vec2(64.0, 22.0)))
                        .on_hover_text("Drop what the tool changed (Escape)")
                        .clicked()
                    {
                        discard(cx);
                    }
                });
            });
        });
}

/// Take the canvas's input for the open tool (the crop tool has a canvas of
/// its own).
pub(crate) fn interact(
    ui: &Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    map: &CanvasMap,
    response: &Response,
) {
    match cx.ui.develop.tools.open {
        Some(Tool::Heal) => heal::interact(ui, cx, editing, map, response),
        Some(Tool::Mask) => mask::interact(ui, cx, editing, map, response),
        Some(Tool::Crop) | None => {}
    }
}

/// Paint the open tool's marks over the photo.
pub(crate) fn paint(
    painter: &Painter,
    cx: &Cx<'_>,
    editing: &Editing,
    map: &CanvasMap,
    pointer: Option<Pos2>,
) {
    let tools = &cx.ui.develop.tools;
    match tools.open {
        Some(Tool::Heal) => heal::paint(painter, &tools.heal, &editing.settings.spots, map),
        Some(Tool::Mask) => {
            mask::paint(painter, &tools.mask, &editing.settings.masks, map, pointer);
        }
        Some(Tool::Crop) | None => {}
    }
}
