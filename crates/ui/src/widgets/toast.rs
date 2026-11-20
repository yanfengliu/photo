//! The toast overlay: brief confirmations ("Set Rating to 3") that appear
//! over the work area and go away on their own.

use egui::{Align2, Color32, CornerRadius, Frame, Id, Margin, Order, Rect, RichText, Role, Stroke};

use crate::theme::{palette, radius, space};

/// Seconds a toast stays up.
const LIFETIME: f64 = 2.2;
/// Seconds a toast takes to fade in, and to fade out at the end of its life.
const FADE: f64 = 0.2;
/// The most toasts shown at once. Older ones make way.
const MAX_VISIBLE: usize = 3;
/// Gap between the toasts and the bottom of the area they are shown over.
const LIFT: f32 = 28.0;

#[derive(Clone, Debug)]
struct Toast {
    text: String,
    /// When it first appeared on screen; `None` until then, so a toast queued
    /// while nothing was drawn still gets its full time.
    shown_at: Option<f64>,
}

/// The queue of toasts and their timing.
///
/// Keep one per window, [`push`](Self::push) a message whenever something
/// worth confirming happens, and call [`show`](Self::show) every frame.
#[derive(Clone, Debug, Default)]
pub struct Toasts {
    toasts: Vec<Toast>,
}

impl Toasts {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a message. If too many are up already, the oldest goes.
    pub fn push(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            shown_at: None,
        });
        if self.toasts.len() > MAX_VISIBLE {
            self.toasts.remove(0);
        }
    }

    /// Whether nothing is queued or showing.
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// The messages currently queued or showing, oldest first.
    pub fn messages(&self) -> impl Iterator<Item = &str> {
        self.toasts.iter().map(|toast| toast.text.as_str())
    }

    /// Draw the toasts stacked at the bottom centre of `over` (the work area)
    /// and drop the ones whose time is up. They do not take pointer input.
    pub fn show(&mut self, ctx: &egui::Context, over: Rect) {
        let now = ctx.input(|input| input.time);
        for toast in &mut self.toasts {
            toast.shown_at.get_or_insert(now);
        }
        self.toasts.retain(|toast| age(toast, now) < LIFETIME);
        if self.toasts.is_empty() {
            return;
        }

        egui::Area::new(Id::unique("photo-toasts"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(over.center_bottom() - egui::vec2(0.0, LIFT))
            .interactable(false)
            .role(Role::Alert)
            .accessible_name("Notifications")
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = space::SM;
                ui.vertical_centered(|ui| {
                    for toast in &self.toasts {
                        ui.scope(|ui| {
                            ui.set_opacity(opacity(age(toast, now)));
                            Frame::new()
                                .fill(Color32::from_black_alpha(224))
                                .stroke(Stroke::new(1.0, palette::OUTLINE))
                                .corner_radius(CornerRadius::same(radius::LG))
                                .inner_margin(Margin::symmetric(14, 6))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(&toast.text).color(palette::TEXT_STRONG),
                                    );
                                });
                        });
                    }
                });
            });

        // Animate the fades; otherwise sleep until the next toast starts to go.
        let next_change = self
            .toasts
            .iter()
            .map(|toast| {
                let age = age(toast, now);
                let steady = FADE..LIFETIME - FADE;
                if steady.contains(&age) {
                    LIFETIME - FADE - age
                } else {
                    0.0
                }
            })
            .fold(f64::INFINITY, f64::min);
        if next_change <= 0.0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after_secs(next_change as f32);
        }
    }
}

fn age(toast: &Toast, now: f64) -> f64 {
    toast.shown_at.map_or(0.0, |shown_at| now - shown_at)
}

/// How opaque a toast of this age is: it fades in, holds, and fades out.
fn opacity(age: f64) -> f32 {
    let fade_in = age / FADE;
    let fade_out = (LIFETIME - age) / FADE;
    fade_in.min(fade_out).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toast_fades_in_holds_and_fades_out() {
        assert_eq!(opacity(0.0), 0.0);
        assert!(opacity(FADE / 2.0) > 0.4 && opacity(FADE / 2.0) < 0.6);
        assert_eq!(opacity(LIFETIME / 2.0), 1.0);
        assert!(opacity(LIFETIME - FADE / 2.0) < 0.6);
        assert_eq!(opacity(LIFETIME), 0.0);
        assert_eq!(opacity(LIFETIME + 5.0), 0.0);
    }

    #[test]
    fn the_queue_keeps_only_the_newest_few() {
        let mut toasts = Toasts::new();
        assert!(toasts.is_empty());
        for n in 1..=5 {
            toasts.push(format!("Set Rating to {n}"));
        }
        let messages: Vec<&str> = toasts.messages().collect();
        assert_eq!(
            messages,
            ["Set Rating to 3", "Set Rating to 4", "Set Rating to 5"]
        );
    }
}
