//! The masking tool: a list of masks, each one or more shapes (linear and
//! radial gradients, brush strokes) added together or cut from each other,
//! with the adjustments it applies where it covers the photo.
//!
//! A new shape is armed first (M, Shift+M, K, or the panel's buttons) and
//! made by the next drag on the photo. With nothing armed a drag takes a
//! handle of the selected shape, or paints when that is a brush; a click on
//! a pin selects its mask. The red overlay (O) is the engine's own coverage
//! of the selected mask.

pub(crate) mod brush;
mod panel;
pub(crate) mod shapes;

use egui::{CursorIcon, Painter, Pos2, Response, Ui, Vec2, vec2};
use photo_core::develop::{DevelopSettings, Mask, MaskComponent, MaskShape};
use photo_core::geom::Point;

use self::brush::BrushOptions;
pub(crate) use self::panel::panel;
use super::{MaskKind, edit, editing};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::app::develop::canvas::map::CanvasMap;
use crate::app::develop::right::Editing;

/// How near a pin a click selects it, in points.
const PIN_REACH: f32 = 10.0;
/// The feather a new radial gradient has.
const RADIAL_FEATHER: f32 = 50.0;

/// The shape the next drag on the photo makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Armed {
    pub(crate) kind: MaskKind,
    /// Into the selected mask: `Some(false)` adds to it, `Some(true)` cuts
    /// from it. `None` makes a new mask.
    pub(crate) into: Option<bool>,
}

/// A gesture on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Drag {
    /// A handle of a gradient, held `grip` source pixels from the pointer;
    /// `start` is the shape when the drag began.
    Handle {
        mask: usize,
        component: usize,
        handle: shapes::Handle,
        grip: Vec2,
        start: MaskShape,
    },
    /// A new radial gradient being pulled out from where the pointer was
    /// pressed.
    Radial {
        mask: usize,
        component: usize,
        pressed: Pos2,
    },
    /// A brush stroke being painted.
    Stroke { mask: usize, component: usize },
}

/// What the masking tool remembers between frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MaskTool {
    /// The selected mask.
    pub(crate) selected: Option<usize>,
    /// The shape of it whose handles show.
    pub(crate) component: usize,
    pub(crate) armed: Option<Armed>,
    /// The red overlay is wanted.
    pub(crate) overlay: bool,
    /// The overlay the engine was last asked to show.
    pub(crate) told: Option<Option<usize>>,
    pub(crate) brush: BrushOptions,
    pub(crate) drag: Option<Drag>,
}

impl MaskTool {
    /// Forget what belongs to the photo that was open.
    pub(crate) fn forget(&mut self) {
        *self = Self {
            overlay: self.overlay,
            brush: self.brush,
            ..Self::default()
        };
    }

    /// The selected shape.
    pub(crate) fn shape<'a>(&self, masks: &'a [Mask]) -> Option<&'a MaskShape> {
        let mask = masks.get(self.selected?)?;
        Some(&mask.components.get(self.component)?.shape)
    }

    /// The kind of shape the tool is working with: the one armed, else the
    /// selected shape's.
    pub(crate) fn kind(&self, masks: &[Mask]) -> Option<MaskKind> {
        match self.armed {
            Some(armed) => Some(armed.kind),
            None => self.shape(masks).map(kind_of),
        }
    }

    /// Whether a drag on the photo paints.
    pub(crate) fn painting(&self, masks: &[Mask]) -> bool {
        self.kind(masks) == Some(MaskKind::Brush)
    }
}

/// The kind of a shape.
pub(crate) fn kind_of(shape: &MaskShape) -> MaskKind {
    match shape {
        MaskShape::Linear { .. } => MaskKind::Linear,
        MaskShape::Radial { .. } => MaskKind::Radial,
        MaskShape::Brush { .. } => MaskKind::Brush,
    }
}

/// Put `shape` into the settings: into the selected mask when `into` says
/// so and one is selected, otherwise as a new mask named after it. The
/// shape becomes the selected one; returns its mask and its place in it.
fn add_shape(
    settings: &mut DevelopSettings,
    tool: &mut MaskTool,
    shape: MaskShape,
    into: Option<bool>,
) -> (usize, usize) {
    let selected = tool.selected.filter(|index| *index < settings.masks.len());
    let placed = match (into, selected) {
        (Some(subtract), Some(mask)) => {
            let components = &mut settings.masks[mask].components;
            components.push(MaskComponent { shape, subtract });
            (mask, components.len() - 1)
        }
        _ => {
            let label = shape.label();
            let alike = settings
                .masks
                .iter()
                .filter(|mask| mask.name.starts_with(label))
                .count();
            settings.masks.push(Mask::from_shape(shape, alike + 1));
            // A new mask does nothing yet: its overlay shows where it is.
            tool.overlay = true;
            (settings.masks.len() - 1, 0)
        }
    };
    (tool.selected, tool.component) = (Some(placed.0), placed.1);
    placed
}

/// A press on the photo became a drag at `from`.
fn begin(cx: &mut Cx<'_>, editing: &Editing, map: &CanvasMap, from: Pos2, erase: bool) {
    let at = map.to_normalized(from);
    let mut settings = editing.settings.clone();
    let tool = &mut cx.ui.develop.tools.mask;
    match tool.armed.take() {
        Some(Armed {
            kind: MaskKind::Linear,
            into,
        }) => {
            let shape = MaskShape::Linear { start: at, end: at };
            let (mask, component) = add_shape(&mut settings, tool, shape.clone(), into);
            tool.drag = Some(Drag::Handle {
                mask,
                component,
                handle: shapes::Handle::LinearEnd,
                grip: Vec2::ZERO,
                start: shape,
            });
        }
        Some(Armed {
            kind: MaskKind::Radial,
            into,
        }) => {
            let shape = shapes::pulled_radial(map, from, from, RADIAL_FEATHER);
            let (mask, component) = add_shape(&mut settings, tool, shape, into);
            tool.drag = Some(Drag::Radial {
                mask,
                component,
                pressed: from,
            });
        }
        Some(Armed {
            kind: MaskKind::Brush,
            into,
        }) => {
            let mut shape = MaskShape::Brush {
                strokes: Vec::new(),
            };
            brush::begin(&mut shape, &tool.brush, at, erase);
            let (mask, component) = add_shape(&mut settings, tool, shape, into);
            tool.drag = Some(Drag::Stroke { mask, component });
        }
        None => {
            let Some(mask) = tool.selected.filter(|index| *index < settings.masks.len()) else {
                return;
            };
            let component = tool.component;
            let Some(target) = settings.masks[mask].components.get_mut(component) else {
                return;
            };
            if brush::begin(&mut target.shape, &tool.brush, at, erase) {
                tool.drag = Some(Drag::Stroke { mask, component });
            } else {
                // A handle is only taken hold of: nothing has moved yet.
                if let Some((handle, held)) = shapes::hit(&target.shape, map, from) {
                    let pointer = map.screen_to_source(from);
                    tool.drag = Some(Drag::Handle {
                        mask,
                        component,
                        handle,
                        grip: vec2(held.x - pointer.x, held.y - pointer.y),
                        start: target.shape.clone(),
                    });
                }
                return;
            }
        }
    }
    edit(cx, settings);
}

/// The drag went on to `at`.
fn carry(cx: &mut Cx<'_>, editing: &Editing, map: &CanvasMap, at: Pos2) {
    let Some(drag) = cx.ui.develop.tools.mask.drag.clone() else {
        return;
    };
    let (mask, component) = match &drag {
        Drag::Handle {
            mask, component, ..
        }
        | Drag::Radial {
            mask, component, ..
        }
        | Drag::Stroke { mask, component } => (*mask, *component),
    };
    let mut settings = editing.settings.clone();
    let Some(target) = settings
        .masks
        .get_mut(mask)
        .and_then(|mask| mask.components.get_mut(component))
    else {
        return;
    };
    let before = target.shape.clone();
    match &drag {
        Drag::Handle {
            handle,
            grip,
            start,
            ..
        } => {
            let pointer = map.screen_to_source(at);
            let to = Point::new(pointer.x + grip.x, pointer.y + grip.y);
            target.shape = shapes::moved(start, *handle, to, map);
        }
        Drag::Radial { pressed, .. } => {
            target.shape = shapes::pulled_radial(map, *pressed, at, RADIAL_FEATHER);
        }
        Drag::Stroke { .. } => {
            brush::extend(&mut target.shape, map, map.to_normalized(at));
        }
    }
    if target.shape != before {
        edit(cx, settings);
    }
}

/// A click on the photo: a dab of the brush, or a pin's mask selected.
fn click(cx: &mut Cx<'_>, editing: &Editing, map: &CanvasMap, at: Pos2, erase: bool) {
    let masks = &editing.settings.masks;
    if cx.ui.develop.tools.mask.painting(masks) {
        begin(cx, editing, map, at, erase);
        cx.ui.develop.tools.mask.drag = None;
        return;
    }
    let tool = &mut cx.ui.develop.tools.mask;
    if tool.armed.is_some() {
        return;
    }
    for (index, mask) in masks.iter().enumerate() {
        for (place, component) in mask.components.iter().enumerate() {
            let pin = shapes::pin(&component.shape).and_then(|pin| map.to_screen(pin));
            if pin.is_some_and(|pin| pin.distance(at) <= PIN_REACH) {
                (tool.selected, tool.component) = (Some(index), place);
                return;
            }
        }
    }
}

/// Take the canvas's input for the masking tool.
pub(crate) fn interact(
    ui: &Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    map: &CanvasMap,
    response: &Response,
) {
    let masks = &editing.settings.masks;
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let pressed = ui.input(|input| input.pointer.press_origin());
    let erase = ui.input(|input| input.modifiers.alt);
    let tool = &cx.ui.develop.tools.mask;
    if response.hovered() || response.dragged() {
        let over_handle = || {
            let shape = tool.shape(masks)?;
            shapes::hit(shape, map, pointer?)
        };
        let icon = if tool.painting(masks) {
            // The brush's own outline is the pointer.
            CursorIcon::None
        } else if tool.armed.is_some() {
            CursorIcon::Crosshair
        } else if tool.drag.is_some() || over_handle().is_some() {
            CursorIcon::Grab
        } else {
            CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(icon);
    }
    if response.drag_started() {
        if let Some(from) = pressed {
            begin(cx, editing, map, from, erase);
        }
    } else if response.dragged()
        && let Some(at) = pointer
    {
        // Not in the frame the drag began: what `begin` made is not in
        // the settings this frame was handed.
        carry(cx, editing, map, at);
    }
    if response.drag_stopped() {
        cx.ui.develop.tools.mask.drag = None;
    }
    if response.clicked()
        && let Some(at) = pointer
    {
        click(cx, editing, map, at, erase);
    }
}

/// Paint the masks: a pin for every shape, the selected mask's outlines,
/// the selected shape's handles, and the brush's outline under `pointer`.
pub(crate) fn paint(
    painter: &Painter,
    tool: &MaskTool,
    masks: &[Mask],
    map: &CanvasMap,
    pointer: Option<Pos2>,
) {
    for (index, mask) in masks.iter().enumerate() {
        let of_selected = tool.selected == Some(index);
        for (place, component) in mask.components.iter().enumerate() {
            let selected = of_selected && tool.component == place;
            if of_selected {
                shapes::paint(painter, &component.shape, map, selected);
            }
            if let Some(at) = shapes::pin(&component.shape).and_then(|pin| map.to_screen(pin)) {
                shapes::paint_pin(painter, at, selected);
            }
        }
    }
    if tool.painting(masks)
        && let Some(at) = pointer
    {
        brush::paint_cursor(painter, map, at, &tool.brush);
    }
}

/// Delete: remove the selected mask.
pub(crate) fn delete_selected(cx: &mut Cx<'_>) {
    let Some(editing) = editing(cx) else {
        return;
    };
    let tool = &mut cx.ui.develop.tools.mask;
    let Some(index) = tool
        .selected
        .filter(|index| *index < editing.settings.masks.len())
    else {
        return;
    };
    let mut settings = editing.settings.clone();
    settings.masks.remove(index);
    (tool.selected, tool.component, tool.drag) = (None, 0, None);
    edit(cx, settings);
}

/// Keep the engine showing the overlay the tool wants: the selected mask's
/// while the tool is `open` with the overlay on, none otherwise.
pub(crate) fn tell_overlay(cx: &mut Cx<'_>, open: bool) {
    let Some(session) = cx.engine.develop() else {
        cx.ui.develop.tools.mask.told = None;
        return;
    };
    let held = session.settings().masks.len();
    let tool = &mut cx.ui.develop.tools.mask;
    let asked = if open && tool.overlay {
        tool.selected
    } else {
        None
    };
    // A mask just made is not the engine's until its draft is; wait for it.
    if asked.is_some_and(|index| index >= held) {
        return;
    }
    if tool.told != Some(asked) {
        tool.told = Some(asked);
        cx.act(Action::Develop(DevelopAction::ShowMaskOverlay(asked)));
    }
}
