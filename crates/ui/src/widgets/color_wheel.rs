//! The colour wheel of the Color Grading panel: a hue and saturation disc
//! with a draggable handle, and a luminance slider under it.
//!
//! Hue is the angle (0 degrees at the right, increasing counter-clockwise),
//! saturation the distance from the centre. Double-click the disc to remove
//! the tint. With keyboard focus, the horizontal arrows turn the hue and the
//! vertical ones change the saturation (ten units with Shift).

use egui::epaint::{Mesh, Shape};
use egui::{
    Align2, Color32, EventFilter, Key, Modifiers, PointerButton, Pos2, Rect, Response, Role, Sense,
    Stroke, Ui, Vec2, WidgetInfo, vec2,
};
use photo_core::develop::{AdjSpec, GradeWheel};

use super::adj_slider::AdjSlider;
use super::{end_click_burst, in_click_burst, meant_double_click, recall, remember};
use crate::theme::{self, palette, size, space};

/// Triangles in the disc.
const SEGMENTS: usize = 72;
/// How close to the handle a press must be to grab it.
const GRAB_REACH: f32 = 8.0;
/// Radius of the handle.
const HANDLE: f32 = 4.5;

/// What a colour wheel reports for one frame.
#[derive(Clone, Debug)]
pub struct WheelResponse {
    /// The disc's response. Its rect is the square around the disc.
    pub response: Response,
    /// Hue, saturation or luminance changed this frame.
    pub changed: bool,
    /// An edit ended this frame: a drag that moved the handle was released,
    /// or a one-step edit (a key, a reset, the luminance slider) was made.
    /// Fires once per drag, also for one that ended back where it began, so
    /// compare with the last recorded state before adding an undo step.
    pub finished: bool,
}

/// A drag on the disc, remembered between frames.
#[derive(Clone, Copy)]
struct Drag {
    /// Handle position minus pointer position at the press.
    grab_offset: Vec2,
    /// Where a press grabbed the handle, and the hue and saturation then.
    /// While the pointer is on that spot they are left exactly as they were,
    /// so pressing the handle never moves it, not even to whole units.
    held: Option<(Pos2, (f32, f32))>,
    /// Whether the drag has changed the wheel so far.
    changed: bool,
}

/// A colour wheel. See the [module documentation](self).
#[must_use = "call `.show(ui)`"]
pub struct ColorWheel<'a> {
    name: &'a str,
    wheel: &'a mut GradeWheel,
    luminance: AdjSpec,
    diameter: f32,
}

impl<'a> ColorWheel<'a> {
    /// A wheel editing `wheel`. `name` ("Shadows", "Midtones", ...) is shown
    /// above the disc and names the parts for accessibility: the disc is
    /// `"{name} wheel"` and the slider `"{name} Luminance"`. `luminance` is
    /// the spec of the luminance adjustment this wheel belongs to.
    pub fn new(name: &'a str, wheel: &'a mut GradeWheel, luminance: AdjSpec) -> Self {
        Self {
            name,
            wheel,
            luminance,
            diameter: 84.0,
        }
    }

    /// The diameter of the disc in points.
    pub fn diameter(mut self, diameter: f32) -> Self {
        self.diameter = diameter;
        self
    }

    /// Show the wheel: caption, disc and luminance slider, stacked.
    pub fn show(self, ui: &mut Ui) -> WheelResponse {
        let Self {
            name,
            wheel,
            luminance,
            diameter,
        } = self;
        ui.vertical(|ui| {
            let width = ui.available_width();
            let (_, caption) = ui.allocate_space(vec2(width, size::ROW - space::SM));
            if ui.is_rect_visible(caption) {
                ui.painter().text(
                    caption.center(),
                    Align2::CENTER_CENTER,
                    name,
                    theme::small_font(),
                    palette::TEXT_DIM,
                );
            }

            let (_, row) = ui.allocate_space(vec2(width, diameter));
            let square = Rect::from_center_size(row.center(), Vec2::splat(diameter));
            let id = ui.make_persistent_id(("color_wheel", name));
            let response = ui.interact(square, id, Sense::click_and_drag());
            let before = (wheel.hue, wheel.saturation);
            let mut finished = pointer(ui, &response, square, wheel);
            finished |= keyboard(ui, &response, wheel);
            let mut changed = (wheel.hue, wheel.saturation) != before;

            let (enabled, summary) = (ui.is_enabled(), describe(wheel));
            response.widget_info(|| WidgetInfo {
                label: Some(format!("{name} wheel")),
                current_text_value: Some(summary.clone()),
                enabled,
                ..WidgetInfo::new(Role::ColorWell)
            });
            if ui.is_rect_visible(square) {
                paint(ui, &response, square, wheel);
            }

            ui.add_space(space::SM);
            let slider = AdjSlider::new(luminance, &mut wheel.luminance, 0.0)
                .name_prefix(name)
                .compact()
                .show(ui);
            changed |= slider.changed;
            finished |= slider.finished;

            WheelResponse {
                response,
                changed,
                finished,
            }
        })
        .inner
    }
}

/// Where the handle for `hue` (degrees) and `saturation` (0 to 100) sits on
/// a disc inscribed in `square`.
///
/// `square` is the rect of the wheel's response, so a script can find the
/// handle on screen from the accessibility tree alone.
pub fn handle_pos(square: Rect, hue: f32, saturation: f32) -> Pos2 {
    let radius = square.width() / 2.0 * (saturation / 100.0).clamp(0.0, 1.0);
    let angle = hue.to_radians();
    square.center() + radius * vec2(angle.cos(), -angle.sin())
}

/// The hue (whole degrees, 0 to 359) and saturation (whole units, 0 to 100)
/// at `pos` on a disc inscribed in `square`.
fn hue_saturation_at(square: Rect, pos: Pos2) -> (f32, f32) {
    let offset = pos - square.center();
    let saturation = (offset.length() / (square.width() / 2.0) * 100.0)
        .round()
        .min(100.0);
    let hue = (-offset.y)
        .atan2(offset.x)
        .to_degrees()
        .rem_euclid(360.0)
        .round()
        % 360.0;
    (hue, saturation)
}

/// Move the handle to `(hue, saturation)`. At the centre there is no hue to
/// speak of, so the old one is kept.
fn set(wheel: &mut GradeWheel, (hue, saturation): (f32, f32)) {
    if saturation > 0.0 {
        wheel.hue = hue;
    }
    wheel.saturation = saturation;
}

/// Apply this frame's pointer input. Returns whether an edit finished.
fn pointer(ui: &Ui, response: &Response, square: Rect, wheel: &mut GradeWheel) -> bool {
    let mut drag: Option<Drag> = recall(ui.ctx(), response.id);
    // Only the primary button edits: a context click leaves the tint be.
    let primary = ui.input(|input| input.pointer.primary_down());
    let down = primary && response.is_pointer_button_down_on();
    let released = response.clicked() || response.drag_stopped_by(PointerButton::Primary);
    let burst = in_click_burst(ui.ctx(), response.id);
    let mut finished = false;

    if meant_double_click(response, burst) {
        drag = None;
        if wheel.saturation != 0.0 {
            wheel.saturation = 0.0;
            wheel.hue = 0.0;
            finished = true;
        }
        end_click_burst(ui.ctx(), response.id);
    } else if let Some(pointer) = response.interact_pointer_pos()
        && (down || released)
        && (drag.is_some() || !burst)
    {
        let drag = drag.get_or_insert_with(|| {
            response.request_focus();
            let handle = handle_pos(square, wheel.hue, wheel.saturation);
            let grabbed = handle.distance(pointer) <= GRAB_REACH;
            Drag {
                grab_offset: if grabbed {
                    handle - pointer
                } else {
                    Vec2::ZERO
                },
                held: grabbed.then_some((pointer, (wheel.hue, wheel.saturation))),
                changed: false,
            }
        });
        let target = match drag.held {
            Some((spot, held)) if spot == pointer => held,
            _ => hue_saturation_at(square, pointer + drag.grab_offset),
        };
        let before = (wheel.hue, wheel.saturation);
        set(wheel, target);
        drag.changed |= (wheel.hue, wheel.saturation) != before;
    }

    if let Some(ended) = drag
        && (released || !down)
    {
        finished |= ended.changed;
        drag = None;
    }
    remember(ui.ctx(), response.id, drag);
    finished
}

/// Arrow keys while the disc has focus. Returns whether an edit was made.
fn keyboard(ui: &Ui, response: &Response, wheel: &mut GradeWheel) -> bool {
    if !response.has_focus() {
        return false;
    }
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            response.id,
            EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
    let presses = |key: Key| {
        ui.input_mut(|input| {
            // Shift first: the plain pattern also matches shifted presses.
            let coarse = input.count_and_consume_key(Modifiers::SHIFT, key);
            let fine = input.count_and_consume_key(Modifiers::NONE, key);
            10.0 * coarse as f32 + fine as f32
        })
    };
    let turn = presses(Key::ArrowRight) - presses(Key::ArrowLeft);
    let push = presses(Key::ArrowUp) - presses(Key::ArrowDown);
    if turn == 0.0 && push == 0.0 {
        return false;
    }
    let before = (wheel.hue, wheel.saturation);
    wheel.hue = (wheel.hue + turn).rem_euclid(360.0);
    wheel.saturation = (wheel.saturation + push).clamp(0.0, 100.0);
    (wheel.hue, wheel.saturation) != before
}

fn describe(wheel: &GradeWheel) -> String {
    format!("hue {:.0}, saturation {:.0}", wheel.hue, wheel.saturation)
}

/// The colour shown on the disc for a hue (degrees) at a saturation (0 to 1).
/// The disc is kept well short of full saturation and brightness: it is a
/// control, not a light source, and must not pull the eye from the photo.
fn disc_color(hue: f32, saturation: f32) -> Color32 {
    let saturation = saturation.clamp(0.0, 1.0);
    hsv(hue, 0.66 * saturation, egui::lerp(0.46..=0.74, saturation))
}

/// A colour from hue (degrees), saturation and value (0 to 1), all in the
/// display's own (gamma) space so the numbers mean what they look like.
fn hsv(hue: f32, saturation: f32, value: f32) -> Color32 {
    let sector = hue.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let byte = |channel: f32| ((channel + value - chroma) * 255.0).round() as u8;
    Color32::from_rgb(byte(red), byte(green), byte(blue))
}

fn paint(ui: &Ui, response: &Response, square: Rect, wheel: &GradeWheel) {
    let painter = ui.painter();
    let (center, radius) = (square.center(), square.width() / 2.0);

    let mut disc = Mesh::default();
    disc.colored_vertex(center, disc_color(0.0, 0.0));
    for segment in 0..SEGMENTS {
        let hue = 360.0 * segment as f32 / SEGMENTS as f32;
        disc.colored_vertex(handle_pos(square, hue, 100.0), disc_color(hue, 1.0));
        let (this, next) = (segment as u32, ((segment + 1) % SEGMENTS) as u32);
        disc.add_triangle(0, 1 + this, 1 + next);
    }
    painter.add(Shape::mesh(disc));
    // A rim hides the hard edge of the mesh and seats the disc in the panel.
    let lit = response.hovered() || response.is_pointer_button_down_on();
    let rim = if response.has_focus() {
        palette::ACCENT
    } else if lit {
        palette::OUTLINE
    } else {
        palette::SEPARATOR
    };
    painter.circle_stroke(center, radius, Stroke::new(1.5, rim));
    painter.circle_filled(center, 1.0, palette::SEPARATOR);

    let handle = handle_pos(square, wheel.hue, wheel.saturation);
    if wheel.saturation > 0.0 {
        let spoke = Stroke::new(1.0, Color32::from_white_alpha(70));
        painter.line_segment([center, handle], spoke);
    }
    let fill = disc_color(wheel.hue, wheel.saturation / 100.0);
    let ring = if lit {
        palette::TEXT_STRONG
    } else {
        palette::TEXT
    };
    painter.circle(handle, HANDLE + 1.0, palette::SEPARATOR, Stroke::NONE);
    painter.circle(handle, HANDLE, fill, Stroke::new(1.5, ring));
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    fn square() -> Rect {
        Rect::from_center_size(pos2(200.0, 200.0), Vec2::splat(100.0))
    }

    #[test]
    fn hue_is_the_angle_counter_clockwise_from_the_right() {
        assert_eq!(handle_pos(square(), 0.0, 100.0), pos2(250.0, 200.0));
        let up = handle_pos(square(), 90.0, 100.0);
        assert!((up.x - 200.0).abs() < 1.0e-3 && (up.y - 150.0).abs() < 1.0e-3);
        let left = handle_pos(square(), 180.0, 100.0);
        assert!((left.x - 150.0).abs() < 1.0e-3 && (left.y - 200.0).abs() < 1.0e-3);
    }

    #[test]
    fn saturation_is_the_distance_from_the_centre() {
        assert_eq!(handle_pos(square(), 123.0, 0.0), square().center());
        assert_eq!(handle_pos(square(), 0.0, 40.0), pos2(220.0, 200.0));
        assert_eq!(
            handle_pos(square(), 0.0, 400.0),
            pos2(250.0, 200.0),
            "clamped to the rim"
        );
    }

    #[test]
    fn a_position_reads_back_as_whole_hue_and_saturation() {
        for (hue, saturation) in [(0.0, 100.0), (45.0, 60.0), (180.0, 25.0), (300.0, 80.0)] {
            let read = hue_saturation_at(square(), handle_pos(square(), hue, saturation));
            assert_eq!(read, (hue, saturation));
        }
        let outside = hue_saturation_at(square(), pos2(500.0, 200.0));
        assert_eq!(outside, (0.0, 100.0), "beyond the rim is full saturation");
        let nearly_a_turn = hue_saturation_at(square(), pos2(250.0, 200.2));
        assert_eq!(nearly_a_turn.0, 0.0, "359.8 degrees rounds to 0, not 360");
    }

    #[test]
    fn the_centre_keeps_the_hue() {
        let mut wheel = GradeWheel {
            hue: 200.0,
            saturation: 40.0,
            luminance: 0.0,
        };
        set(&mut wheel, hue_saturation_at(square(), square().center()));
        assert_eq!((wheel.hue, wheel.saturation), (200.0, 0.0));
    }

    #[test]
    fn hsv_gives_the_primaries_where_they_belong() {
        assert_eq!(hsv(0.0, 1.0, 1.0), Color32::from_rgb(255, 0, 0));
        assert_eq!(hsv(60.0, 1.0, 1.0), Color32::from_rgb(255, 255, 0));
        assert_eq!(hsv(120.0, 1.0, 1.0), Color32::from_rgb(0, 255, 0));
        assert_eq!(hsv(240.0, 1.0, 1.0), Color32::from_rgb(0, 0, 255));
        assert_eq!(hsv(360.0, 1.0, 1.0), hsv(0.0, 1.0, 1.0));
        assert_eq!(hsv(200.0, 0.0, 0.5), Color32::from_gray(128));
    }

    #[test]
    fn the_disc_is_neutral_at_the_centre_and_restrained_at_the_rim() {
        let center = disc_color(0.0, 0.0);
        assert!(center.r() == center.g() && center.g() == center.b());
        let rim = disc_color(0.0, 1.0);
        assert!(
            rim.r() > rim.g() && rim.r() < 200 && rim.g() > 40,
            "{rim:?}"
        );
    }
}
