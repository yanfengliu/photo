//! The develop slider: a label, a slim track and an editable number, built
//! from a `photo-core` [`AdjSpec`].
//!
//! ```text
//! Exposure   ────────┃━━━━━━▮──────   +0.35
//! label      track: tick at zero,     value
//!            filled from the default
//! ```
//!
//! * Drag the track, or click it to move the handle there. Pressing the
//!   handle itself moves nothing.
//! * Double-click the label or the track to reset to the default. On the bare
//!   track that is two edits: the first click moves the handle, the second
//!   resets it.
//! * With keyboard focus, arrow keys nudge by `step` (ten steps with Shift).
//! * Click the number to type one; Enter or leaving the field commits it,
//!   clamped to the range. A press on the track while typing only ends the
//!   typing. A field that is left without typing changes nothing.
//!
//! The caller owns the value. [`SliderResponse::changed`] is true on every
//! frame the value moved; [`SliderResponse::finished`] is true once when an
//! edit ends, which is when to record an undo step.

mod scale;
mod track;
mod value_field;

use egui::{Align2, Rect, Response, Sense, Ui, vec2};
use photo_core::develop::AdjSpec;

pub use scale::Scale;
use track::Track;
use value_field::ValueField;

use super::{end_click_burst, in_click_burst, meant_double_click};
use crate::theme::{self, palette, size, space};

/// What a slider reports for one frame.
#[derive(Clone, Debug)]
pub struct SliderResponse {
    /// The track's response, with the whole row as its rect.
    pub response: Response,
    /// The value changed this frame.
    pub changed: bool,
    /// An edit ended this frame: a drag that moved the value was released, or
    /// a one-step edit (a key, a reset, a typed number, an accessibility
    /// action) was made. Fires once per drag, also for a drag that ended back
    /// where it began (the preview it drove still wants a final render), so
    /// compare with the last recorded state before adding an undo step.
    pub finished: bool,
}

/// The outcome of one frame of editing, gathered from the slider's parts.
#[derive(Clone, Copy, Default)]
struct Edit {
    changed: bool,
    finished: bool,
}

impl Edit {
    /// A complete, one-step edit: set `value` to `target` and report it as
    /// both changed and finished. Does nothing if the value is already there.
    fn commit(&mut self, value: &mut f32, target: f32) {
        if *value != target {
            *value = target;
            self.changed = true;
            self.finished = true;
        }
    }
}

/// A develop slider. See the [module documentation](self).
#[must_use = "call `.show(ui)`"]
pub struct AdjSlider<'a> {
    spec: AdjSpec,
    value: &'a mut f32,
    default: f32,
    name_prefix: Option<&'a str>,
    scale: Scale,
    label_width: f32,
}

impl<'a> AdjSlider<'a> {
    /// A slider for `value` described by `spec`. `default` is what a
    /// double-click resets to and where the track's fill starts.
    pub fn new(spec: AdjSpec, value: &'a mut f32, default: f32) -> Self {
        Self {
            spec,
            value,
            default,
            name_prefix: None,
            scale: Scale::LINEAR,
            label_width: size::SLIDER_LABEL,
        }
    }

    /// Put `prefix` in front of the label to form the accessible name, for
    /// labels that repeat on screen: "Amount" under Sharpening becomes
    /// "Sharpening Amount". The prefix is not shown.
    pub fn name_prefix(mut self, prefix: &'a str) -> Self {
        self.name_prefix = Some(prefix);
        self
    }

    /// Spread the values along the track with `scale` instead of linearly.
    pub fn scale(mut self, scale: Scale) -> Self {
        self.scale = scale;
        self
    }

    /// Leave out the label column: just the track and the number. The
    /// accessible name is unchanged.
    pub fn compact(mut self) -> Self {
        self.label_width = 0.0;
        self
    }

    /// The name tests and assistive technology find this slider by.
    pub fn accessible_name(&self) -> String {
        match self.name_prefix {
            Some(prefix) => format!("{prefix} {}", self.spec.label),
            None => self.spec.label.to_owned(),
        }
    }

    /// Show the slider as one row across the available width.
    pub fn show(self, ui: &mut Ui) -> SliderResponse {
        let name = self.accessible_name();
        let Self {
            spec,
            value,
            default,
            name_prefix,
            scale,
            label_width,
        } = self;
        let id = ui.make_persistent_id(("adj_slider", spec.key, name_prefix));
        let (_, row) = ui.allocate_space(vec2(ui.available_width(), size::ROW));
        let parts = Parts::new(row, label_width);
        let mut edit = Edit::default();

        if let Some(label_rect) = parts.label {
            let label = ui.interact(label_rect, id.with("label"), Sense::CLICK);
            let burst = in_click_burst(ui.ctx(), label.id);
            if meant_double_click(&label, burst) {
                edit.commit(value, default);
                end_click_burst(ui.ctx(), label.id);
            }
            if ui.is_rect_visible(label_rect) {
                let color = if label.hovered() {
                    palette::TEXT_STRONG
                } else {
                    palette::TEXT
                };
                ui.painter_at(label_rect).text(
                    label_rect.left_center(),
                    Align2::LEFT_CENTER,
                    spec.label,
                    theme::body_font(),
                    color,
                );
            }
        }

        let field = ValueField {
            id: id.with("value"),
            name: &name,
            spec,
            default,
        };
        let track = Track {
            id,
            name: &name,
            spec,
            scale,
            default,
            typing: field.is_typing(ui),
        };
        let response = track.show(ui, parts.track, value, &mut edit);
        field.show(ui, parts.value, value, &mut edit);

        SliderResponse {
            response: response.with_new_rect(row),
            changed: edit.changed,
            finished: edit.finished,
        }
    }
}

/// Show a develop slider with the default options. See [`AdjSlider`].
pub fn adj_slider(ui: &mut Ui, spec: AdjSpec, value: &mut f32, default: f32) -> SliderResponse {
    AdjSlider::new(spec, value, default).show(ui)
}

/// The three parts of a slider row.
struct Parts {
    label: Option<Rect>,
    track: Rect,
    value: Rect,
}

impl Parts {
    fn new(row: Rect, label_width: f32) -> Self {
        let (label, rest) = if label_width > 0.0 {
            let (label, rest) = row.split_left_right_at_x(row.left() + label_width);
            (Some(label), rest.with_min_x(rest.left() + space::MD))
        } else {
            (None, row)
        };
        let (track, value) = rest.split_left_right_at_x(rest.right() - size::SLIDER_VALUE);
        Self {
            label,
            track: track.with_max_x(track.right() - space::SM),
            value,
        }
    }
}

/// `value` rounded to the number of decimals the adjustment shows.
fn round_to_decimals(spec: &AdjSpec, value: f32) -> f32 {
    let scale = 10.0_f32.powi(i32::from(spec.decimals));
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use egui::pos2;
    use photo_core::develop::Adj;

    use super::*;

    #[test]
    fn rounding_follows_the_decimals_of_the_adjustment() {
        assert_eq!(round_to_decimals(&Adj::Exposure.spec(), 0.3549), 0.35);
        assert_eq!(round_to_decimals(&Adj::Contrast.spec(), 12.6), 13.0);
        assert_eq!(round_to_decimals(&Adj::SharpenRadius.spec(), 1.26), 1.3);
    }

    #[test]
    fn a_commit_reports_nothing_when_the_value_is_already_there() {
        let (mut edit, mut value) = (Edit::default(), 5.0);
        edit.commit(&mut value, 5.0);
        assert!(!edit.changed && !edit.finished);
        edit.commit(&mut value, 6.0);
        assert!(edit.changed && edit.finished);
        assert_eq!(value, 6.0);
    }

    #[test]
    fn the_row_is_split_into_aligned_columns() {
        let row = Rect::from_min_max(pos2(10.0, 0.0), pos2(290.0, size::ROW));
        let parts = Parts::new(row, size::SLIDER_LABEL);
        let label = parts.label.unwrap();
        assert_eq!((label.left(), label.width()), (10.0, size::SLIDER_LABEL));
        assert_eq!(
            (parts.value.right(), parts.value.width()),
            (290.0, size::SLIDER_VALUE)
        );
        assert!(label.right() < parts.track.left());
        assert!(parts.track.right() < parts.value.left());

        let compact = Parts::new(row, 0.0);
        assert!(compact.label.is_none());
        assert_eq!(compact.track.left(), 10.0);
    }

    #[test]
    fn the_accessible_name_takes_the_prefix() {
        let mut value = 0.0;
        let plain = AdjSlider::new(Adj::SharpenAmount.spec(), &mut value, 0.0);
        assert_eq!(plain.accessible_name(), "Amount");
        let named = plain.name_prefix("Sharpening");
        assert_eq!(named.accessible_name(), "Sharpening Amount");
    }
}
