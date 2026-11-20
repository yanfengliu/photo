//! The track of a develop slider: pointer, keyboard and accessibility input,
//! and its painting.

use egui::accesskit::{Action, ActionData};
use egui::{
    CornerRadius, EventFilter, Id, Key, Modifiers, PointerButton, Rangef, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, WidgetInfo, lerp, pos2, remap_clamp, vec2,
};
use photo_core::develop::AdjSpec;

use super::{Edit, Scale, round_to_decimals};
use crate::theme::{palette, radius, size};
use crate::widgets::{end_click_burst, in_click_burst, meant_double_click, recall, remember};

/// How far from the handle's centre a press still counts as grabbing the
/// handle (rather than asking the handle to jump to the pointer).
const GRAB_REACH: f32 = 6.0;

/// A pointer gesture on the track, remembered between frames.
#[derive(Clone, Copy)]
struct Gesture {
    /// Handle centre minus pointer, at the press. Kept for the whole drag so
    /// the handle does not jump under the pointer when it is grabbed.
    grab_offset: f32,
    /// Where a press grabbed the handle, and the value then. While the
    /// pointer is on that spot the value is left exactly as it was, so
    /// pressing the handle never moves it, not even onto the step grid.
    held: Option<(f32, f32)>,
    /// The press ended the typing of a number and does nothing else.
    inert: bool,
    /// Whether the gesture has changed the value so far.
    changed: bool,
}

/// Everything the track needs to know about its slider.
pub(super) struct Track<'a> {
    pub id: Id,
    pub name: &'a str,
    pub spec: AdjSpec,
    pub scale: Scale,
    pub default: f32,
    /// A number is being typed into the slider's field. A press on the track
    /// then only ends the typing (which commits the number); taking it as an
    /// edit too would put the handle back under the pointer.
    pub typing: bool,
}

impl Track<'_> {
    /// Handle input for the track in `rect`, update `value`, and paint it.
    pub fn show(&self, ui: &Ui, rect: Rect, value: &mut f32, edit: &mut Edit) -> Response {
        let response = ui.interact(rect, self.id, Sense::click_and_drag());
        let half_handle = size::HANDLE[0] / 2.0;
        let centers = Rangef::new(rect.left() + half_handle, rect.right() - half_handle);

        self.pointer(ui, &response, centers, value, edit);
        self.keyboard(ui, &response, value, edit);
        self.accessibility(ui, &response, value, edit);

        if ui.is_rect_visible(rect) {
            self.paint(ui, &response, rect, centers, *value);
        }
        response
    }

    fn x_of(&self, centers: Rangef, value: f32) -> f32 {
        let AdjSpec { min, max, .. } = self.spec;
        lerp(centers, self.scale.position(value, min, max))
    }

    /// The value under `x`, on the slider's step grid.
    fn value_at(&self, centers: Rangef, x: f32) -> f32 {
        let AdjSpec { min, max, step, .. } = self.spec;
        let position = remap_clamp(x, centers, 0.0..=1.0);
        let raw = self.scale.value(position, min, max);
        let on_grid = min + ((raw - min) / step).round() * step;
        self.spec.clamp(round_to_decimals(&self.spec, on_grid))
    }

    fn pointer(
        &self,
        ui: &Ui,
        response: &Response,
        centers: Rangef,
        value: &mut f32,
        edit: &mut Edit,
    ) {
        let mut gesture: Option<Gesture> = recall(ui.ctx(), self.id);
        // Only the primary button edits: a context click leaves the value be.
        let primary = ui.input(|input| input.pointer.primary_down());
        let down = primary && response.is_pointer_button_down_on();
        let released = response.clicked() || response.drag_stopped_by(PointerButton::Primary);
        let burst = in_click_burst(ui.ctx(), self.id);

        if meant_double_click(response, burst) {
            gesture = None;
            edit.commit(value, self.default);
            end_click_burst(ui.ctx(), self.id);
        } else if let Some(pointer) = response.interact_pointer_pos()
            && (down || released)
            && (gesture.is_some() || !burst)
        {
            let gesture = gesture.get_or_insert_with(|| {
                response.request_focus();
                let handle = self.x_of(centers, *value);
                let grabbed = (pointer.x - handle).abs() <= GRAB_REACH;
                Gesture {
                    grab_offset: if grabbed { handle - pointer.x } else { 0.0 },
                    held: grabbed.then_some((pointer.x, *value)),
                    inert: self.typing,
                    changed: false,
                }
            });
            let target = match gesture.held {
                _ if gesture.inert => *value,
                Some((x, held)) if x == pointer.x => held,
                _ => self.value_at(centers, pointer.x + gesture.grab_offset),
            };
            if target != *value {
                *value = target;
                edit.changed = true;
                gesture.changed = true;
            }
        }

        if let Some(ended) = gesture
            && (released || !down)
        {
            edit.finished |= ended.changed;
            gesture = None;
        }
        remember(ui.ctx(), self.id, gesture);
    }

    /// Arrow keys nudge by one step, or ten with Shift, while the track has
    /// keyboard focus. The keys are consumed so nothing else acts on them.
    fn keyboard(&self, ui: &Ui, response: &Response, value: &mut f32, edit: &mut Edit) {
        if !response.has_focus() {
            return;
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                self.id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let steps = ui.input_mut(|input| {
            let mut steps = 0.0;
            for (key, direction) in [
                (Key::ArrowRight, 1.0),
                (Key::ArrowUp, 1.0),
                (Key::ArrowLeft, -1.0),
                (Key::ArrowDown, -1.0),
            ] {
                // Shift first: the plain pattern also matches shifted presses.
                let coarse = input.count_and_consume_key(Modifiers::SHIFT, key);
                let fine = input.count_and_consume_key(Modifiers::NONE, key);
                steps += direction * (10.0 * coarse as f32 + fine as f32);
            }
            steps
        });
        self.nudge(steps, value, edit);
    }

    /// Assistive technology (and the test harness) can set, raise and lower
    /// the value through the accessibility tree.
    fn accessibility(&self, ui: &Ui, response: &Response, value: &mut f32, edit: &mut Edit) {
        let (set_to, steps) = ui.input(|input| {
            let set_to = input
                .accesskit_action_requests(self.id, Action::SetValue)
                .filter_map(|request| match &request.data {
                    Some(ActionData::NumericValue(number)) => Some(*number as f32),
                    _ => None,
                })
                .last();
            let up = input.num_accesskit_action_requests(self.id, Action::Increment);
            let down = input.num_accesskit_action_requests(self.id, Action::Decrement);
            (set_to, up as f32 - down as f32)
        });
        let enabled = ui.is_enabled();
        if enabled {
            if let Some(number) = set_to.filter(|number| number.is_finite()) {
                let target = self.spec.clamp(round_to_decimals(&self.spec, number));
                edit.commit(value, target);
            }
            self.nudge(steps, value, edit);
        }

        let AdjSpec { min, max, step, .. } = self.spec;
        let current = f64::from(*value);
        response.widget_info(|| WidgetInfo::slider(enabled, current, self.name));
        ui.ctx().accesskit_node_builder(self.id, |node| {
            node.set_min_numeric_value(f64::from(min));
            node.set_max_numeric_value(f64::from(max));
            node.set_numeric_value_step(f64::from(step));
            node.add_action(Action::SetValue);
            if current < f64::from(max) {
                node.add_action(Action::Increment);
            }
            if current > f64::from(min) {
                node.add_action(Action::Decrement);
            }
        });
    }

    fn nudge(&self, steps: f32, value: &mut f32, edit: &mut Edit) {
        if steps != 0.0 {
            let moved = self.spec.clamp(*value) + steps * self.spec.step;
            let target = self.spec.clamp(round_to_decimals(&self.spec, moved));
            edit.commit(value, target);
        }
    }

    fn paint(&self, ui: &Ui, response: &Response, rect: Rect, centers: Rangef, value: f32) {
        let painter = ui.painter();
        let y = rect.center().y;
        let half_track = size::TRACK / 2.0;
        let rounding = CornerRadius::same(radius::SM);
        let engaged = response.is_pointer_button_down_on();
        let lit = engaged || response.hovered();

        let rail = Rect::from_x_y_ranges(rect.x_range(), y - half_track..=y + half_track);
        painter.rect_filled(rail, rounding, palette::OUTLINE);

        // The filled part runs from the default to the current value, so a
        // slider at its default shows no fill at all.
        let handle_x = self.x_of(centers, value);
        let origin_x = self.x_of(centers, self.default);
        if (handle_x - origin_x).abs() > 0.5 {
            let fill = Rect::from_x_y_ranges(
                handle_x.min(origin_x)..=handle_x.max(origin_x),
                y - half_track..=y + half_track,
            );
            let color = if lit {
                palette::ACCENT_STRONG
            } else {
                palette::ACCENT
            };
            painter.rect_filled(fill, rounding, color);
        }

        // Signed ranges get a tick at zero, the centre of the range.
        if self.spec.min < 0.0 && self.spec.max > 0.0 {
            let zero_x = self.x_of(centers, 0.0);
            let tick = Rect::from_center_size(pos2(zero_x, y), vec2(1.0, 7.0));
            painter.rect_filled(tick, 0.0, palette::TEXT_DIM);
        }

        let handle = Rect::from_center_size(pos2(handle_x, y), size::HANDLE.into());
        let fill = if lit {
            palette::TEXT_STRONG
        } else {
            palette::TEXT
        };
        let outline = if response.has_focus() || engaged {
            palette::ACCENT_STRONG
        } else {
            palette::SEPARATOR
        };
        painter.rect(
            handle,
            rounding,
            fill,
            Stroke::new(1.0, outline),
            StrokeKind::Outside,
        );
    }
}
