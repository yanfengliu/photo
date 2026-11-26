//! The crop tool: the rectangle and its aspect ratio, the straighten angle
//! (a slider, and a line drawn along what should be level), quarter turns
//! and mirrors. While it is open the canvas shows the whole photo with the
//! rectangle over it (see `canvas`); everything it changes is one step,
//! recorded when the tool is put away.

mod aspect;
pub(crate) mod canvas;
pub(crate) mod rect;

use egui::{Button, ComboBox, DragValue, Pos2, RichText, Ui, Vec2, vec2};
use photo_core::develop::{Adj, Crop, CropConstraint, CropRect, DevelopSettings, Turn};
use photo_engine::Geometry;

use self::aspect::{ASPECTS, Aspect};
use self::rect::{Handle, Upright};
use super::{edit, editing};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::app::develop::canvas::map::{frame_geometry, turned};
use crate::app::develop::right::Editing;
use crate::theme::{palette, size, space};
use crate::widgets::{AdjSlider, Icon, IconButton};

/// A handle being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drag {
    pub(crate) handle: Handle,
    /// The rectangle when the drag began.
    pub(crate) start: Upright,
    /// Where the pointer was pressed, in points.
    pub(crate) pressed: Pos2,
    /// Where the pointer was when the rectangle was last worked out.
    pub(crate) at: Pos2,
}

/// What the crop tool remembers between frames.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CropTool {
    /// The ratio typed into the Custom fields.
    pub(crate) custom: (f32, f32),
    /// The menu entry chosen while the tool is open; a stored constraint
    /// names its own.
    pub(crate) chosen: Option<Aspect>,
    /// The straighten tool is armed: the next drag draws a line to level.
    pub(crate) leveling: bool,
    /// The line being drawn, in points.
    pub(crate) line: Option<(Pos2, Pos2)>,
    pub(crate) drag: Option<Drag>,
    /// The rectangle the angle slider's drag began with: its centre in the
    /// frame (from the frame's centre, not turned) and its size.
    pub(crate) turning: Option<(Vec2, Vec2)>,
}

impl Default for CropTool {
    fn default() -> Self {
        Self {
            custom: (3.0, 2.0),
            chosen: None,
            leveling: false,
            line: None,
            drag: None,
            turning: None,
        }
    }
}

impl CropTool {
    /// Forget the gestures (the tool closed, or another photo opened).
    pub(crate) fn forget(&mut self) {
        *self = Self {
            custom: self.custom,
            ..Self::default()
        };
    }

    /// Whether the guide lines are those of straightening.
    pub(crate) fn straightening(&self) -> bool {
        self.leveling || self.turning.is_some()
    }
}

/// The frame the crop is cut from, as the settings shown lay it out.
pub(crate) struct Frame {
    pub(crate) geometry: Geometry,
    pub(crate) size: Vec2,
}

impl Frame {
    /// The frame of `settings` for the open photo; `None` until it is
    /// decoded.
    pub(crate) fn of(cx: &Cx<'_>, settings: &DevelopSettings) -> Option<Self> {
        let source = cx.engine.develop()?.geometry()?.source_size();
        let geometry = frame_geometry(settings, source);
        let (width, height) = geometry.frame_size();
        Some(Self {
            geometry,
            size: vec2(width as f32, height as f32),
        })
    }

    /// The least a rectangle's sides may be.
    pub(crate) fn min(&self) -> Vec2 {
        self.size * CropRect::MIN_SIDE
    }

    /// The most a side can be: the frame's diagonal.
    fn cap(&self) -> Vec2 {
        Vec2::splat(self.size.length())
    }

    /// Whether a rectangle shows only the photo when the frame is
    /// straightened by `angle`.
    pub(crate) fn fits(&self, angle: f32) -> impl Fn(Upright) -> bool + '_ {
        move |upright| rect::shows_photo(upright, angle, &self.geometry)
    }

    /// The largest rectangle of `ratio` about `center` that shows only the
    /// photo, no larger than `cap`; about the frame's centre when `center`
    /// leaves no room.
    fn largest(&self, center: Vec2, ratio: f32, cap: Vec2, angle: f32) -> Option<Upright> {
        let fits = self.fits(angle);
        rect::largest(center, ratio, self.min(), cap, &fits)
            .or_else(|| rect::largest(Vec2::ZERO, ratio, self.min(), cap, &fits))
    }
}

/// Store `upright` at `angle` as the crop of `settings` and hand them to the
/// engine as a draft of the tool's step.
pub(crate) fn set_rect(
    cx: &mut Cx<'_>,
    editing: &Editing,
    frame: &Frame,
    upright: Upright,
    angle: f32,
) {
    let mut settings = editing.settings.clone();
    settings.crop = rect::with_rect(&settings.crop, upright, angle, frame.size);
    edit(cx, settings);
}

/// Straighten by `angle`. During one drag of the slider the rectangle is
/// worked out from the one the drag began with, so going back gives it
/// back.
pub(crate) fn set_angle(
    cx: &mut Cx<'_>,
    editing: &Editing,
    frame: &Frame,
    angle: f32,
    finished: bool,
) {
    let crop = editing.settings.crop;
    let angle = Adj::CropAngle.spec().clamp(angle);
    let tool = &mut cx.ui.develop.tools.crop;
    let (center, size) = *tool.turning.get_or_insert_with(|| {
        let current = rect::upright_of(&crop, frame.size);
        (turned(current.center, crop.angle), current.size)
    });
    if finished {
        tool.turning = None;
    }
    let fits = frame.fits(angle);
    let upright = rect::straightened(center, size, angle, frame.min(), &fits);
    set_rect(cx, editing, frame, upright, angle);
}

/// Hold the rectangle to `constraint`: the largest of that shape about
/// where it is.
fn constrain(cx: &mut Cx<'_>, editing: &Editing, frame: &Frame, constraint: CropConstraint) {
    let mut settings = editing.settings.clone();
    settings.crop.constraint = constraint;
    if let Some(ratio) = aspect::locked_ratio(constraint, frame.size) {
        let angle = settings.crop.angle;
        let current = rect::upright_of(&settings.crop, frame.size);
        if let Some(upright) = frame.largest(current.center, ratio, frame.cap(), angle) {
            settings.crop = rect::with_rect(&settings.crop, upright, angle, frame.size);
        }
    }
    edit(cx, settings);
}

/// An entry of the aspect menu was chosen.
fn choose_aspect(cx: &mut Cx<'_>, editing: &Editing, frame: &Frame, aspect: Aspect) {
    let current = rect::upright_of(&editing.settings.crop, frame.size);
    let tool = &mut cx.ui.develop.tools.crop;
    tool.chosen = Some(aspect);
    let landscape = current.size.x >= current.size.y;
    let constraint = aspect.constraint(frame.size, landscape, tool.custom);
    constrain(cx, editing, frame, constraint);
}

/// The lock: off frees the rectangle's shape, on holds the shape it has.
fn set_lock(cx: &mut Cx<'_>, editing: &Editing, frame: &Frame, locked: bool) {
    let mut settings = editing.settings.clone();
    if locked {
        let crop = settings.crop;
        let current = rect::upright_of(&crop, frame.size);
        let constraint = if crop.is_identity() {
            CropConstraint::Original
        } else {
            CropConstraint::Ratio {
                width: current.size.x.round().max(1.0),
                height: current.size.y.round().max(1.0),
            }
        };
        let tool = &mut cx.ui.develop.tools.crop;
        tool.chosen = Aspect::of(constraint, frame.size);
        if let (Some(Aspect::Custom), CropConstraint::Ratio { width, height }) =
            (tool.chosen, constraint)
        {
            tool.custom = (width, height);
        }
        settings.crop.constraint = constraint;
    } else {
        settings.crop.constraint = CropConstraint::Free;
    }
    edit(cx, settings);
}

/// X: lay the rectangle the other way, a locked ratio with it.
pub(crate) fn swap(cx: &mut Cx<'_>) {
    let Some(editing) = editing(cx) else {
        return;
    };
    let Some(frame) = Frame::of(cx, &editing.settings) else {
        return;
    };
    let mut settings = editing.settings.clone();
    let angle = settings.crop.angle;
    let current = rect::upright_of(&settings.crop, frame.size);
    let swapped = Upright {
        center: current.center,
        size: vec2(current.size.y, current.size.x),
    };
    settings.crop.constraint = match settings.crop.constraint {
        CropConstraint::Free => CropConstraint::Free,
        CropConstraint::Original => CropConstraint::Ratio {
            width: frame.size.y,
            height: frame.size.x,
        },
        CropConstraint::Ratio { width, height } => CropConstraint::Ratio {
            width: height,
            height: width,
        },
    };
    let tool = &mut cx.ui.develop.tools.crop;
    tool.custom = (tool.custom.1, tool.custom.0);
    if tool.chosen == Some(Aspect::Original) {
        tool.chosen = Aspect::of(settings.crop.constraint, frame.size);
    }
    let upright = frame
        .largest(swapped.center, swapped.aspect(), swapped.size, angle)
        .unwrap_or(current);
    settings.crop = rect::with_rect(&settings.crop, upright, angle, frame.size);
    edit(cx, settings);
}

/// A quarter turn or a mirror, the crop carried with it: part of the crop
/// tool's step while that is open, a step of its own otherwise. Returns
/// false when no photo is open to turn here.
pub(crate) fn turn(cx: &mut Cx<'_>, way: Turn) -> bool {
    let Some(editing) = editing(cx).filter(Editing::editable) else {
        return false;
    };
    let mut settings = editing.settings.clone();
    settings.turn(way);
    let tool = &mut cx.ui.develop.tools.crop;
    tool.drag = None;
    tool.turning = None;
    if way.is_quarter() {
        tool.custom = (tool.custom.1, tool.custom.0);
    }
    if cx.ui.develop.tools.open == Some(super::Tool::Crop) {
        edit(cx, settings);
    } else {
        cx.act(Action::Develop(DevelopAction::Apply(Box::new(settings))));
    }
    true
}

/// The menu entry shown for the crop as it is.
fn shown_aspect(tool: &CropTool, crop: &Crop, frame: &Frame) -> Aspect {
    let stored = Aspect::of(crop.constraint, frame.size);
    match (tool.chosen, stored) {
        // What was chosen, while it still says what is stored.
        (Some(chosen), Some(_))
            if chosen.constraint(frame.size, true, tool.custom) == crop.constraint
                || chosen.constraint(frame.size, false, tool.custom) == crop.constraint =>
        {
            chosen
        }
        (_, Some(stored)) => stored,
        (Some(chosen), None) => chosen,
        (None, None) => Aspect::Original,
    }
}

/// The tool's panel: aspect and lock, the angle, the straighten tool, turns
/// and mirrors, and the size the photo will have.
pub(crate) fn panel(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let Some(frame) = Frame::of(cx, &editing.settings) else {
        ui.label(RichText::new("The photo is still being read.").color(palette::TEXT_DIM));
        return;
    };
    let crop = editing.settings.crop;
    let locked = crop.constraint != CropConstraint::Free;
    let shown = shown_aspect(&cx.ui.develop.tools.crop, &crop, &frame);

    ui.horizontal(|ui| {
        ui.allocate_ui(vec2(size::SLIDER_LABEL, size::ROW), |ui| {
            ui.set_min_width(size::SLIDER_LABEL);
            ui.label(RichText::new("Aspect").color(palette::TEXT));
        });
        let mut chosen = shown;
        ComboBox::from_id_salt("crop-aspect")
            .selected_text(shown.label())
            .width(104.0)
            .show_ui(ui, |ui| {
                for (label, aspect) in ASPECTS {
                    ui.selectable_value(&mut chosen, aspect, label);
                }
            })
            .response
            .accessible_name("Crop aspect");
        if chosen != shown {
            choose_aspect(cx, editing, &frame, chosen);
        }
        let (icon, hint) = if locked {
            (Icon::Lock, "The shape is held: click to free it")
        } else {
            (Icon::Unlock, "The shape is free: click to hold it")
        };
        let lock = IconButton::new(icon, "Lock aspect").selected(locked);
        if ui.add(lock).on_hover_text(hint).clicked() {
            set_lock(cx, editing, &frame, !locked);
        }
        if ui
            .add(Button::new("Swap").min_size(vec2(0.0, size::ROW)))
            .on_hover_text("Lay the rectangle the other way (X)")
            .clicked()
        {
            swap(cx);
        }
    });
    if shown == Aspect::Custom {
        custom_fields(ui, cx, editing, &frame);
    }

    let mut angle = crop.angle;
    let slider = AdjSlider::new(Adj::CropAngle.spec(), &mut angle, 0.0)
        .name_prefix("Crop")
        .show(ui);
    if slider.changed {
        set_angle(cx, editing, &frame, angle, slider.finished);
    } else if !slider.response.dragged() {
        cx.ui.develop.tools.crop.turning = None;
    }

    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        let leveling = cx.ui.develop.tools.crop.leveling;
        if ui
            .add(Button::selectable(leveling, "Straighten").min_size(vec2(0.0, size::ROW)))
            .on_hover_text("Draw a line along something that should be level or upright")
            .clicked()
        {
            cx.ui.develop.tools.crop.leveling = !leveling;
        }
        ui.add_space(space::MD);
        for (icon, name, way) in [
            (Icon::RotateCcw, "Rotate left", Turn::Left),
            (Icon::RotateCw, "Rotate right", Turn::Right),
            (Icon::Flip, "Flip horizontal", Turn::FlipHorizontal),
            (Icon::FlipVertical, "Flip vertical", Turn::FlipVertical),
        ] {
            if ui.add(IconButton::new(icon, name)).clicked() {
                turn(cx, way);
            }
        }
    });

    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        // The size is the engine's: the one rectangle every view and the
        // export are cut to.
        let size = cx
            .engine
            .develop()
            .and_then(|session| session.output_size());
        if let Some((width, height)) = size {
            ui.label(
                RichText::new(format!("{width} x {height} px"))
                    .color(palette::TEXT_DIM)
                    .small(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!crop.is_identity(), Button::new("Reset Crop"))
                .on_hover_text("The whole photo, not turned")
                .clicked()
            {
                let mut settings = editing.settings.clone();
                settings.crop = Crop {
                    constraint: crop.constraint,
                    ..Crop::default()
                };
                cx.ui.develop.tools.crop.turning = None;
                edit(cx, settings);
            }
        });
    });
}

/// The two numbers of a custom ratio.
fn custom_fields(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing, frame: &Frame) {
    let mut custom = cx.ui.develop.tools.crop.custom;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(size::SLIDER_LABEL + space::MD);
        for (value, name) in [
            (&mut custom.0, "Custom aspect width"),
            (&mut custom.1, "Custom aspect height"),
        ] {
            let field = DragValue::new(value).range(0.1..=100.0).speed(0.05);
            changed |= ui.add(field).accessible_name(name).changed();
            if name.ends_with("width") {
                ui.label(RichText::new(":").color(palette::TEXT_DIM));
            }
        }
    });
    if changed {
        cx.ui.develop.tools.crop.custom = custom;
        choose_aspect(cx, editing, frame, Aspect::Custom);
    }
}
