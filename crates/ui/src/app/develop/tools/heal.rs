//! The healing tool. A click on the photo places a spot the size the panel
//! says, a drag places one as large as the drag; either takes the source the
//! engine suggests for it. Drag a spot to move it, its source to move the
//! source; Delete removes the selected one. Each spot is drawn as its
//! target and its source joined by a line, through the photo's geometry, so
//! a spot stays on its content however the photo is turned or cropped.

use egui::epaint::Shape;
use egui::{Button, Color32, CursorIcon, Painter, Pos2, Response, RichText, Stroke, Ui, Vec2, vec2};
use photo_core::develop::{AdjSpec, SettingsGroup, Spot, SpotMode};
use photo_core::geom::Point;

use super::{edit, editing};
use crate::app::Cx;
use crate::app::develop::canvas::map::CanvasMap;
use crate::app::develop::right::Editing;
use crate::theme::{palette, size, space};
use crate::widgets::AdjSlider;

/// The radius slider: a percentage of the photo's longer side.
const SIZE: AdjSpec = AdjSpec {
    key: "spot_size",
    label: "Size",
    min: 0.1,
    max: 25.0,
    step: 0.1,
    decimals: 1,
    group: SettingsGroup::Healing,
};
const FEATHER: AdjSpec = AdjSpec {
    key: "spot_feather",
    label: "Feather",
    min: 0.0,
    max: 100.0,
    step: 1.0,
    decimals: 0,
    group: SettingsGroup::Healing,
};
const OPACITY: AdjSpec = AdjSpec {
    key: "spot_opacity",
    label: "Opacity",
    min: 0.0,
    max: 100.0,
    step: 1.0,
    decimals: 0,
    group: SettingsGroup::Healing,
};

/// The least a spot can be grabbed by, in points.
const GRIP: f32 = 7.0;

/// A gesture on the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SpotDrag {
    /// A new spot being sized: its centre (normalized source) and its
    /// radius so far (a fraction of the longer side).
    Sizing { target: Point, radius: f32 },
    /// A spot's target, or its source, being moved. `grip` is from the
    /// pointer to the centre held, in source pixels.
    Moving {
        index: usize,
        source: bool,
        grip: Vec2,
    },
}

/// What the healing tool remembers between frames.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HealTool {
    pub(crate) selected: Option<usize>,
    /// What new spots get: how they fill, their radius as a percentage of
    /// the longer side, feather and opacity.
    pub(crate) mode: SpotMode,
    pub(crate) size: f32,
    pub(crate) feather: f32,
    pub(crate) opacity: f32,
    pub(crate) drag: Option<SpotDrag>,
}

impl Default for HealTool {
    fn default() -> Self {
        Self {
            selected: None,
            mode: SpotMode::Heal,
            size: 2.0,
            feather: 50.0,
            opacity: 100.0,
            drag: None,
        }
    }
}

impl HealTool {
    /// Forget what belongs to the photo that was open.
    pub(crate) fn forget(&mut self) {
        self.selected = None;
        self.drag = None;
    }
}

fn offset(from: Point, to: Point) -> Vec2 {
    vec2(to.x - from.x, to.y - from.y)
}

/// The spot under a source position (pixels): its index, and whether it is
/// the source circle. The selected spot is looked at first, then the newest.
fn hit(
    spots: &[Spot],
    selected: Option<usize>,
    map: &CanvasMap,
    at: Point,
    reach: f32,
) -> Option<(usize, bool)> {
    let long = map.long_edge();
    let order = selected
        .into_iter()
        .chain((0..spots.len()).rev())
        .filter(|index| *index < spots.len());
    for index in order {
        let spot = &spots[index];
        let radius = (spot.radius * long).max(reach);
        if map.pixels(spot.target).distance(at) <= radius {
            return Some((index, false));
        }
        if map.pixels(spot.source).distance(at) <= radius {
            return Some((index, true));
        }
    }
    None
}

/// Add a spot at `target` (normalized source) of `radius`, with the source
/// the engine suggests, and select it.
fn place(cx: &mut Cx<'_>, editing: &Editing, target: Point, radius: f32) {
    if !(0.0..=1.0).contains(&target.x) || !(0.0..=1.0).contains(&target.y) {
        return;
    }
    let radius = radius.clamp(Spot::MIN_RADIUS, Spot::MAX_RADIUS);
    let source = match cx.engine.suggest_heal_source(target, radius) {
        Ok(source) => source,
        Err(error) => {
            log::warn!("no source suggested for the spot: {error}");
            Point::new(target.x + 2.5 * radius, target.y)
        }
    };
    let tool = &mut cx.ui.develop.tools.heal;
    let spot = Spot {
        mode: tool.mode,
        target,
        source,
        radius,
        feather: tool.feather,
        opacity: tool.opacity,
    };
    let mut settings = editing.settings.clone();
    settings.spots.push(spot);
    tool.selected = Some(settings.spots.len() - 1);
    edit(cx, settings);
}

/// Delete: remove the selected spot.
pub(crate) fn delete_selected(cx: &mut Cx<'_>) {
    let Some(editing) = editing(cx) else {
        return;
    };
    let Some(index) = cx
        .ui
        .develop
        .tools
        .heal
        .selected
        .filter(|index| *index < editing.settings.spots.len())
    else {
        return;
    };
    let mut settings = editing.settings.clone();
    settings.spots.remove(index);
    let tool = &mut cx.ui.develop.tools.heal;
    tool.selected = None;
    tool.drag = None;
    edit(cx, settings);
}

/// Take the canvas's input for the healing tool.
pub(crate) fn interact(
    ui: &Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    map: &CanvasMap,
    response: &Response,
) {
    let spots = &editing.settings.spots;
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let pressed = ui.input(|input| input.pointer.press_origin());
    let selected = cx.ui.develop.tools.heal.selected;
    let reach = |at: Pos2| GRIP * map.source_per_point(at);
    let under = |at: Pos2| hit(spots, selected, map, map.screen_to_source(at), reach(at));

    if response.hovered() {
        let over = pointer.and_then(under).is_some();
        ui.ctx().set_cursor_icon(if over {
            CursorIcon::Grab
        } else {
            CursorIcon::Crosshair
        });
    }
    if response.drag_started()
        && let Some(from) = pressed
    {
        let at = map.screen_to_source(from);
        let tool = &mut cx.ui.develop.tools.heal;
        tool.drag = Some(match under(from) {
            Some((index, source)) => {
                tool.selected = Some(index);
                let spot = &spots[index];
                let held = map.pixels(if source { spot.source } else { spot.target });
                SpotDrag::Moving {
                    index,
                    source,
                    grip: offset(at, held),
                }
            }
            None => SpotDrag::Sizing {
                target: map.normalized(at),
                radius: Spot::MIN_RADIUS,
            },
        });
    }
    if response.dragged()
        && let (Some(drag), Some(at)) = (cx.ui.develop.tools.heal.drag, pointer)
    {
        let at = map.screen_to_source(at);
        match drag {
            SpotDrag::Sizing { target, .. } => {
                let radius = map.pixels(target).distance(at) / map.long_edge();
                cx.ui.develop.tools.heal.drag = Some(SpotDrag::Sizing {
                    target,
                    radius: radius.clamp(Spot::MIN_RADIUS, Spot::MAX_RADIUS),
                });
            }
            SpotDrag::Moving {
                index,
                source,
                grip,
            } if index < spots.len() => {
                let moved = map.normalized(Point::new(at.x + grip.x, at.y + grip.y));
                let mut settings = editing.settings.clone();
                let spot = &mut settings.spots[index];
                let held = if source {
                    &mut spot.source
                } else {
                    &mut spot.target
                };
                if *held != moved {
                    *held = moved;
                    edit(cx, settings);
                }
            }
            SpotDrag::Moving { .. } => {}
        }
    }
    if response.drag_stopped()
        && let Some(SpotDrag::Sizing { target, radius }) = cx.ui.develop.tools.heal.drag.take()
    {
        cx.ui.develop.tools.heal.size = radius * 100.0;
        place(cx, editing, target, radius);
    }
    if response.clicked()
        && let Some(at) = pointer
    {
        match under(at) {
            Some((index, _)) => cx.ui.develop.tools.heal.selected = Some(index),
            None => {
                let radius = cx.ui.develop.tools.heal.size / 100.0;
                place(cx, editing, map.to_normalized(at), radius);
            }
        }
    }
}

/// A ring that reads on any photo: light on a dark edge.
fn ring(painter: &Painter, points: &[Pos2], width: f32, color: Color32) {
    if points.len() < 3 {
        return;
    }
    let shadow = Stroke::new(width + 2.0, Color32::from_black_alpha(140));
    painter.add(Shape::closed_line(points.to_vec(), shadow));
    painter.add(Shape::closed_line(points.to_vec(), Stroke::new(width, color)));
}

/// How far a ring's points are from `center`, on average.
fn ring_radius(points: &[Pos2], center: Pos2) -> f32 {
    let sum: f32 = points.iter().map(|point| point.distance(center)).sum();
    sum / points.len().max(1) as f32
}

/// Paint the spots: each target and source circle, joined by a line.
pub(crate) fn paint(painter: &Painter, tool: &HealTool, spots: &[Spot], map: &CanvasMap) {
    let long = map.long_edge();
    for (index, spot) in spots.iter().enumerate() {
        let selected = tool.selected == Some(index);
        let radius = spot.radius * long;
        let target = map.circle(map.pixels(spot.target), radius);
        let source = map.circle(map.pixels(spot.source), radius);
        let (strong, quiet) = if selected {
            (palette::TEXT_STRONG, palette::ACCENT_STRONG)
        } else {
            (palette::TEXT, palette::TEXT_DIM)
        };
        if let (Some(from), Some(to)) = (map.to_screen(spot.source), map.to_screen(spot.target)) {
            let along = to - from;
            let gap = ring_radius(&source, from) + ring_radius(&target, to);
            if along.length() > gap + 1.0 {
                let unit = along.normalized();
                let ends = [
                    from + unit * ring_radius(&source, from),
                    to - unit * ring_radius(&target, to),
                ];
                painter.line_segment(ends, Stroke::new(3.0, Color32::from_black_alpha(140)));
                painter.line_segment(ends, Stroke::new(1.0, quiet));
            }
        }
        ring(painter, &source, 1.0, quiet);
        ring(painter, &target, if selected { 2.0 } else { 1.5 }, strong);
    }
    if let Some(SpotDrag::Sizing { target, radius }) = tool.drag {
        let sizing = map.circle(map.pixels(target), radius * long);
        ring(painter, &sizing, 1.5, palette::TEXT_STRONG);
    }
}

/// The tool's panel: heal or clone, size, feather and opacity (of the
/// selected spot, and of the spots placed next), and Delete.
pub(crate) fn panel(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let spots = &editing.settings.spots;
    let selected = cx
        .ui
        .develop
        .tools
        .heal
        .selected
        .filter(|index| *index < spots.len());
    let tool = &cx.ui.develop.tools.heal;
    let was = match selected {
        Some(index) => {
            let spot = &spots[index];
            (
                spot.mode,
                spot.radius * 100.0,
                spot.feather,
                spot.opacity,
            )
        }
        None => (tool.mode, tool.size, tool.feather, tool.opacity),
    };
    let (mut mode, mut radius, mut feather, mut opacity) = was;

    ui.horizontal(|ui| {
        ui.allocate_ui(vec2(size::SLIDER_LABEL, size::ROW), |ui| {
            ui.set_min_width(size::SLIDER_LABEL);
            ui.label(RichText::new("Mode").color(palette::TEXT));
        });
        for (choice, label, hint) in [
            (
                SpotMode::Heal,
                "Heal",
                "Copy the source's texture, matched to what surrounds the spot",
            ),
            (SpotMode::Clone, "Clone", "Copy the source as it is"),
        ] {
            let button = Button::selectable(mode == choice, label).min_size(vec2(0.0, size::ROW));
            if ui.add(button).on_hover_text(hint).clicked() {
                mode = choice;
            }
        }
    });
    AdjSlider::new(SIZE, &mut radius, 2.0)
        .name_prefix("Spot")
        .show(ui);
    AdjSlider::new(FEATHER, &mut feather, 50.0)
        .name_prefix("Spot")
        .show(ui);
    AdjSlider::new(OPACITY, &mut opacity, 100.0)
        .name_prefix("Spot")
        .show(ui);

    if (mode, radius, feather, opacity) != was {
        let tool = &mut cx.ui.develop.tools.heal;
        (tool.mode, tool.size, tool.feather, tool.opacity) = (mode, radius, feather, opacity);
        if let Some(index) = selected {
            let mut settings = editing.settings.clone();
            let spot = &mut settings.spots[index];
            spot.mode = mode;
            spot.radius = (radius / 100.0).clamp(Spot::MIN_RADIUS, Spot::MAX_RADIUS);
            spot.feather = feather;
            spot.opacity = opacity;
            edit(cx, settings);
        }
    }

    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        let count = crate::app::text::count(spots.len(), "spot", "spots");
        ui.label(RichText::new(count).color(palette::TEXT_DIM).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(selected.is_some(), Button::new("Delete Spot"))
                .on_hover_text("Remove the selected spot (Delete)")
                .clicked()
            {
                delete_selected(cx);
            }
        });
    });
    ui.label(
        RichText::new(
            "Click the photo to place a spot, or drag to size one. Drag a spot or its source to move it.",
        )
        .color(palette::TEXT_DIM)
        .small(),
    );
}
