//! The tone-curve editor: a plot of a curve with draggable control points.
//!
//! * Press the curve to add a point there (and keep dragging to place it).
//! * Drag a point to move it. It cannot pass its neighbours.
//! * Double-click a point, or drag it well outside the plot, to remove it.
//!
//! The editor changes the control points of a `photo-core` [`PointCurve`];
//! what the curve *looks* like between them is the pipeline's business, so
//! the caller supplies a function that evaluates it.

use egui::epaint::Shape;
use egui::{
    Color32, CornerRadius, PointerButton, Pos2, Rect, Response, Role, Sense, Stroke, Ui, Vec2,
    WidgetInfo, lerp, pos2, remap_clamp,
};
use photo_core::develop::{CurvePoint, PointCurve};

use super::histogram::{BINS, area_mesh, grid_color};
use super::{end_click_burst, in_click_burst, recall, remember};
use crate::theme::{self, palette, radius};

/// How close to a control point a press must be to grab it.
const GRAB_REACH: f32 = 9.0;
/// How close to the curve a press must be to add a point.
const ADD_REACH: f32 = 12.0;
/// How far outside the plot a dragged point is let go of and removed.
const REMOVE_MARGIN: f32 = 24.0;
/// The least horizontal distance kept between neighbouring points: a little
/// more than `PointCurve` itself insists on, so no point is ever dropped.
const GAP: f32 = PointCurve::MIN_GAP * 2.0;
/// Radius of a control point's handle.
const HANDLE: f32 = 3.5;

/// What a curve editor reports for one frame.
#[derive(Clone, Debug)]
pub struct CurveResponse {
    /// The plot's response. Its rect is the plot area.
    pub response: Response,
    /// The control points changed this frame.
    pub changed: bool,
    /// An edit ended this frame: a drag that changed the curve was released,
    /// or a point was removed with a double-click. Fires once per drag, also
    /// for one that ended as it began (a point dragged away and back), so
    /// compare with the last recorded state before adding an undo step.
    pub finished: bool,
}

/// A point being dragged, remembered between frames.
#[derive(Clone)]
struct Drag {
    /// The control points when the drag began. Every frame is computed from
    /// these, so a point dragged off and back on returns exactly.
    original: Vec<CurvePoint>,
    /// Which of them is being dragged.
    index: usize,
    /// Handle position minus pointer position at the press.
    grab_offset: Vec2,
    /// Whether the gesture has changed the curve so far.
    changed: bool,
}

/// A tone-curve editor. See the [module documentation](self).
#[must_use = "call `.show(ui)`"]
pub struct CurveEditor<'a> {
    name: &'a str,
    curve: &'a mut PointCurve,
    sample: &'a dyn Fn(f32) -> f32,
    backdrop: Option<&'a [u32; BINS]>,
    color: Color32,
    size: Option<Vec2>,
}

impl<'a> CurveEditor<'a> {
    /// An editor for the control points of `curve`, named `name` for
    /// accessibility. `sample` evaluates the curve to draw: output (0 to 1)
    /// for an input (0 to 1). It is called before any edit of this frame is
    /// applied, so it should evaluate the curve as it was handed in.
    pub fn new(name: &'a str, curve: &'a mut PointCurve, sample: &'a dyn Fn(f32) -> f32) -> Self {
        Self {
            name,
            curve,
            sample,
            backdrop: None,
            color: palette::TEXT_STRONG,
            size: None,
        }
    }

    /// Draw these histogram counts faintly behind the curve.
    pub fn backdrop(mut self, counts: &'a [u32; BINS]) -> Self {
        self.backdrop = Some(counts);
        self
    }

    /// The colour of the curve and its points (for the per-channel curves).
    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    /// The size of the plot. The default is a square as wide as the space
    /// available.
    pub fn size(mut self, size: impl Into<Vec2>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Show the editor.
    pub fn show(mut self, ui: &mut Ui) -> CurveResponse {
        let size = self
            .size
            .unwrap_or_else(|| Vec2::splat(ui.available_width()));
        // An id from the name, not from the position among the widgets around
        // it: a drag must survive those appearing and disappearing.
        let id = ui.make_persistent_id(("curve_editor", self.name));
        let (_, rect) = ui.allocate_space(size);
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let before = self.curve.clone();
        let finished = self.interact(ui, &response, rect);
        let changed = *self.curve != before;
        if changed {
            // The caller's sampler still describes the old curve.
            ui.ctx().request_repaint();
        }

        let (enabled, summary) = (ui.is_enabled(), describe(self.curve.points()));
        response.widget_info(|| WidgetInfo {
            label: Some(self.name.to_owned()),
            current_text_value: Some(summary.clone()),
            enabled,
            ..WidgetInfo::new(Role::Canvas)
        });
        if ui.is_rect_visible(rect) {
            self.paint(ui, &response, rect);
        }
        CurveResponse {
            response,
            changed,
            finished,
        }
    }

    /// Apply this frame's pointer input. Returns whether an edit finished.
    fn interact(&mut self, ui: &Ui, response: &Response, rect: Rect) -> bool {
        let mut drag: Option<Drag> = recall(ui.ctx(), response.id);
        // Only the primary button edits: a context click leaves the curve be.
        let (pressed, primary) = ui.input(|input| {
            let pointer = &input.pointer;
            (pointer.primary_pressed(), pointer.primary_down())
        });
        let down = primary && response.is_pointer_button_down_on();
        let released = response.clicked() || response.drag_stopped_by(PointerButton::Primary);
        let burst = in_click_burst(ui.ctx(), response.id);
        let mut finished = false;

        if response.double_clicked() {
            drag = None;
            let points = self.curve.points();
            let hit = response
                .interact_pointer_pos()
                .and_then(|pointer| nearest(points, rect, pointer));
            if let Some(index) = hit.filter(|_| points.len() > 2) {
                *self.curve = PointCurve::new(without(points, index));
                finished = true;
                end_click_burst(ui.ctx(), response.id);
            }
        } else if let Some(pointer) = response.interact_pointer_pos()
            && (down || released)
        {
            // Only the press itself can grab or add a point: a pointer that
            // wanders onto the curve with the button already down does not.
            // Nor does the click that follows a removing double-click.
            if pressed && !burst {
                response.request_focus();
                drag = self.begin(rect, pointer);
            }
            if let Some(drag) = &mut drag {
                let removable = drag.original.len() > 2;
                let dropped = removable && !rect.expand(REMOVE_MARGIN).contains(pointer);
                let points = if dropped {
                    without(&drag.original, drag.index)
                } else {
                    let target = point_at(rect, pointer + drag.grab_offset);
                    moved(&drag.original, drag.index, target)
                };
                if points != self.curve.points() {
                    *self.curve = PointCurve::new(points);
                    drag.changed = true;
                }
            }
        }

        if let Some(ended) = &drag
            && (released || !down)
        {
            finished |= ended.changed;
            drag = None;
        }
        remember(ui.ctx(), response.id, drag);
        finished
    }

    /// Start a drag at `pointer`: grab the point there, or add one if the
    /// press is on the curve.
    fn begin(&mut self, rect: Rect, pointer: Pos2) -> Option<Drag> {
        let points = self.curve.points().to_vec();
        if let Some(index) = nearest(&points, rect, pointer) {
            let grab_offset = point_pos(rect, points[index]) - pointer;
            return Some(Drag {
                original: points,
                index,
                grab_offset,
                changed: false,
            });
        }
        let x = point_at(rect, pointer).x;
        let on_curve = CurvePoint::new(x, (self.sample)(x).clamp(0.0, 1.0));
        let handle = point_pos(rect, on_curve);
        let has_room = points.iter().all(|point| (point.x - x).abs() >= GAP);
        if !has_room || (handle.y - pointer.y).abs() > ADD_REACH {
            return None;
        }
        let index = points.partition_point(|point| point.x < x);
        let mut original = points;
        original.insert(index, on_curve);
        *self.curve = PointCurve::new(original.iter().copied());
        Some(Drag {
            original,
            index,
            grab_offset: handle - pointer,
            changed: true,
        })
    }

    fn paint(&self, ui: &Ui, response: &Response, rect: Rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(radius::MD), palette::WELL);
        let guide = Stroke::new(1.0, grid_color());
        for quarter in 1..4 {
            let t = quarter as f32 / 4.0;
            painter.vline(lerp(rect.x_range(), t), rect.y_range(), guide);
            painter.hline(rect.x_range(), lerp(rect.y_range(), t), guide);
        }
        if let Some(counts) = self.backdrop {
            let peak = counts[1..BINS - 1].iter().copied().max().unwrap_or(0);
            let fill = theme::mix(palette::WELL, palette::OUTLINE, 0.45);
            painter.add(Shape::mesh(area_mesh(rect, counts, peak, fill)));
        }
        painter.line_segment([rect.left_bottom(), rect.right_top()], guide);

        let steps = (rect.width() / 2.0).ceil().max(2.0) as usize;
        let line: Vec<Pos2> = (0..=steps)
            .map(|step| {
                let x = step as f32 / steps as f32;
                point_pos(rect, CurvePoint::new(x, (self.sample)(x).clamp(0.0, 1.0)))
            })
            .collect();
        painter.add(Shape::line(line, Stroke::new(1.5, self.color)));

        let points = self.curve.points();
        let lit = if response.is_pointer_button_down_on() {
            recall::<Drag>(ui.ctx(), response.id).map(|drag| drag.index)
        } else {
            response
                .hover_pos()
                .and_then(|pointer| nearest(points, rect, pointer))
        };
        for (index, point) in points.iter().enumerate() {
            let center = point_pos(rect, *point);
            let fill = if lit == Some(index) {
                self.color
            } else {
                palette::WELL
            };
            painter.circle(center, HANDLE, fill, Stroke::new(1.5, self.color));
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                CornerRadius::same(radius::MD),
                Stroke::new(1.0, palette::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Where the control point `point` is drawn in a plot occupying `rect`.
///
/// The plot rect is the rect of the editor's response, so a script can find
/// a point on screen from the accessibility tree alone.
pub fn point_pos(rect: Rect, point: CurvePoint) -> Pos2 {
    pos2(
        lerp(rect.x_range(), point.x),
        lerp(rect.bottom()..=rect.top(), point.y),
    )
}

/// The curve coordinates under `pos`, clamped to the plot.
fn point_at(rect: Rect, pos: Pos2) -> CurvePoint {
    CurvePoint::new(
        remap_clamp(pos.x, rect.x_range(), 0.0..=1.0),
        remap_clamp(pos.y, rect.bottom()..=rect.top(), 0.0..=1.0),
    )
}

/// The control point closest to `pos`, if one is within reach.
fn nearest(points: &[CurvePoint], rect: Rect, pos: Pos2) -> Option<usize> {
    points
        .iter()
        .map(|point| point_pos(rect, *point).distance(pos))
        .enumerate()
        .filter(|(_, distance)| *distance <= GRAB_REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// `points` with the one at `index` moved to `target`, kept between its
/// neighbours and inside the plot.
fn moved(points: &[CurvePoint], index: usize, target: CurvePoint) -> Vec<CurvePoint> {
    let floor = match index.checked_sub(1) {
        Some(previous) => points[previous].x + GAP,
        None => 0.0,
    };
    let ceiling = match points.get(index + 1) {
        Some(next) => next.x - GAP,
        None => 1.0,
    };
    let mut points = points.to_vec();
    points[index] = CurvePoint::new(
        target.x.clamp(floor, ceiling.max(floor)),
        target.y.clamp(0.0, 1.0),
    );
    points
}

/// `points` without the one at `index`.
fn without(points: &[CurvePoint], index: usize) -> Vec<CurvePoint> {
    let mut points = points.to_vec();
    points.remove(index);
    points
}

/// The control points as text, for the accessibility tree: `x,y` pairs.
fn describe(points: &[CurvePoint]) -> String {
    let pairs: Vec<String> = points
        .iter()
        .map(|point| format!("{:.2},{:.2}", point.x, point.y))
        .collect();
    pairs.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot() -> Rect {
        Rect::from_min_max(pos2(100.0, 50.0), pos2(300.0, 250.0))
    }

    fn identity() -> Vec<CurvePoint> {
        PointCurve::default().points().to_vec()
    }

    #[test]
    fn the_plot_runs_left_to_right_and_bottom_to_top() {
        assert_eq!(
            point_pos(plot(), CurvePoint::new(0.0, 0.0)),
            pos2(100.0, 250.0)
        );
        assert_eq!(
            point_pos(plot(), CurvePoint::new(1.0, 1.0)),
            pos2(300.0, 50.0)
        );
        assert_eq!(
            point_pos(plot(), CurvePoint::new(0.5, 0.25)),
            pos2(200.0, 200.0)
        );
    }

    #[test]
    fn screen_positions_map_back_to_curve_coordinates_and_clamp() {
        let point = CurvePoint::new(0.3, 0.8);
        let back = point_at(plot(), point_pos(plot(), point));
        assert!((back.x - 0.3).abs() < 1.0e-5 && (back.y - 0.8).abs() < 1.0e-5);
        assert_eq!(
            point_at(plot(), pos2(0.0, 999.0)),
            CurvePoint::new(0.0, 0.0)
        );
    }

    #[test]
    fn only_a_point_within_reach_is_grabbed_and_the_closest_wins() {
        let points = [
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.5),
            CurvePoint::new(0.52, 0.5),
        ];
        assert_eq!(nearest(&points, plot(), pos2(203.0, 150.0)), Some(2));
        assert_eq!(nearest(&points, plot(), pos2(199.0, 150.0)), Some(1));
        assert_eq!(nearest(&points, plot(), pos2(150.0, 150.0)), None);
    }

    #[test]
    fn a_moved_point_cannot_pass_its_neighbours_or_leave_the_plot() {
        let points = [
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.5),
            CurvePoint::new(1.0, 1.0),
        ];
        let right = moved(&points, 1, CurvePoint::new(2.0, 2.0));
        assert_eq!(right[1], CurvePoint::new(1.0 - GAP, 1.0));
        let left = moved(&points, 1, CurvePoint::new(-1.0, -1.0));
        assert_eq!(left[1], CurvePoint::new(GAP, 0.0));
        let end = moved(&points, 2, CurvePoint::new(0.8, 0.9));
        assert_eq!(end[2], CurvePoint::new(0.8, 0.9));
        // Whatever the drag, the result is a curve PointCurve accepts whole.
        assert_eq!(PointCurve::new(right).points().len(), 3);
    }

    #[test]
    fn the_description_lists_the_points() {
        assert_eq!(describe(&identity()), "0.00,0.00 1.00,1.00");
        assert_eq!(without(&identity(), 0).len(), 1);
    }
}
