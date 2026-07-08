//! Harness action commands: sliders, clicks, and keys.
//!
//! Each action dispatches the exact `Message` values the equivalent real user
//! interaction produces — never direct state mutation. Also owns the slider
//! name vocabulary shared by the dispatcher and the observation builder.

use super::*;
use crate::harness::HarnessResponse;

impl App {
    pub(super) fn respond_unknown_slider(&mut self, id: u64, kind: &str) {
        self.respond_harness(HarnessResponse::failure(
            id,
            "invalid_params",
            &format!(
                "unknown slider {kind:?}; valid kinds: {}",
                ALL_SLIDER_KINDS
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    pub(super) fn set_harness_slider(
        &mut self,
        id: u64,
        kind_name: &str,
        value: f32,
    ) -> Task<Message> {
        let Some(kind) = parse_slider_kind(kind_name) else {
            self.respond_unknown_slider(id, kind_name);
            return Task::none();
        };
        if self.tab != Tab::Detail || self.current_image_path.is_none() {
            self.respond_harness(HarnessResponse::failure(
                id,
                "unavailable",
                "no image open in Detail — sliders only exist there",
            ));
            return Task::none();
        }
        let (min, max) = slider_range(kind);
        let value = value.clamp(min, max);
        // Two on_change events then a release is exactly what a real drag
        // produces (the first change only registers drag detection). Clearing
        // the double-click-release memory first keeps consecutive harness
        // set_slider calls from triggering the reset-to-zero double-click
        // affordance — the agent is pacing deliberate drags, not double-clicking.
        self.last_slider_release = None;
        let first = self.update(Message::SliderChanged(kind, value));
        let second = self.update(Message::SliderChanged(kind, value));
        let release = self.update(Message::SliderReleased(kind));
        self.respond_harness(HarnessResponse::success(
            id,
            serde_json::json!({"accepted": true, "kind": kind_name, "value": value}),
        ));
        Task::batch([first, second, release])
    }

    pub(super) fn click_harness_control(
        &mut self,
        id: u64,
        control: &str,
        value: Option<String>,
    ) -> Task<Message> {
        // A user cannot click a control that is not interactively available;
        // the gate uses the same predicate the `observe` controls list
        // advertises, so list and behavior cannot drift.
        if let Some(false) = self.harness_control_enabled(control) {
            self.respond_harness(HarnessResponse::failure(
                id,
                "unavailable",
                &format!(
                    "control {control:?} is currently disabled — check `enabled` in the observe controls list"
                ),
            ));
            return Task::none();
        }
        let message = match control {
            "save" => Message::SaveEdited,
            "back" => {
                if self.collection_nav.is_some() {
                    Message::ExitCollectionDetail
                } else {
                    Message::SwitchTab(Tab::Library)
                }
            }
            "rotate_cw" => Message::RotateClockwise,
            "rotate_ccw" => Message::RotateCounterclockwise,
            "lens_correction" => Message::ToggleLensCorrection,
            "crop" => Message::ToggleCropMode,
            "crop_clear" => Message::ClearCrop,
            "reset_all" => Message::ResetAll,
            "crop_aspect" => match value.as_deref() {
                Some("Freeform") => Message::CropAspectSelected(CropAspect::Freeform),
                Some("Square") => Message::CropAspectSelected(CropAspect::Square),
                other => {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        &format!(
                            "crop_aspect needs value \"Freeform\" or \"Square\", got {other:?}"
                        ),
                    ));
                    return Task::none();
                }
            },
            "lens_profile" => match value {
                Some(name) => Message::LensProfileSelected(name),
                None => {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        "lens_profile needs a value (\"Auto\", \"None\", or \"<Maker> <Model>\")",
                    ));
                    return Task::none();
                }
            },
            "add_folder" | "add_files" => {
                self.respond_harness(HarnessResponse::failure(
                    id,
                    "unsupported",
                    "opens a native file dialog the harness cannot drive; use import_files / import_folder",
                ));
                return Task::none();
            }
            other => {
                self.respond_harness(HarnessResponse::failure(
                    id,
                    "invalid_params",
                    &format!("unknown control {other:?}; see the controls list in `observe`"),
                ));
                return Task::none();
            }
        };
        let task = self.update(message);
        self.respond_harness_accepted(id);
        task
    }

    pub(super) fn press_harness_key(
        &mut self,
        id: u64,
        name: &str,
        mods: &[String],
    ) -> Task<Message> {
        let mut modifiers = keyboard::Modifiers::empty();
        for modifier in mods {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "command" => modifiers |= keyboard::Modifiers::CTRL,
                "shift" => modifiers |= keyboard::Modifiers::SHIFT,
                "alt" => modifiers |= keyboard::Modifiers::ALT,
                other => {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        &format!("unknown modifier {other:?}; use ctrl, shift, or alt"),
                    ));
                    return Task::none();
                }
            }
        }

        use keyboard::key::Named;
        let key: keyboard::Key = match name.to_ascii_lowercase().as_str() {
            "escape" | "esc" => keyboard::Key::Named(Named::Escape),
            "left" | "arrowleft" => keyboard::Key::Named(Named::ArrowLeft),
            "right" | "arrowright" => keyboard::Key::Named(Named::ArrowRight),
            "space" => keyboard::Key::Named(Named::Space),
            "backspace" => keyboard::Key::Named(Named::Backspace),
            "home" => keyboard::Key::Named(Named::Home),
            single if single.chars().count() == 1 => keyboard::Key::Character(single.into()),
            other => {
                self.respond_harness(HarnessResponse::failure(
                    id,
                    "invalid_params",
                    &format!(
                        "unknown key {other:?}; use a single character or escape/left/right/space/backspace/home"
                    ),
                ));
                return Task::none();
            }
        };

        // Ctrl+O opens a native dialog the harness cannot drive; refuse it the
        // same way dialog-opening clicks are refused.
        if modifiers.command() && matches!(&key, keyboard::Key::Character(c) if c.as_str() == "o") {
            self.respond_harness(HarnessResponse::failure(
                id,
                "unsupported",
                "ctrl+o opens a native file dialog the harness cannot drive; use the open command",
            ));
            return Task::none();
        }

        let task = self.handle_key(key, modifiers);
        self.respond_harness_accepted(id);
        task
    }
}

pub(crate) const ALL_SLIDER_KINDS: &[(&str, SliderKind)] = &[
    ("exposure", SliderKind::Exposure),
    ("contrast", SliderKind::Contrast),
    ("highlights", SliderKind::Highlights),
    ("shadows", SliderKind::Shadows),
    ("whites", SliderKind::Whites),
    ("blacks", SliderKind::Blacks),
    ("temperature", SliderKind::Temperature),
    ("tint", SliderKind::Tint),
    ("vibrance", SliderKind::Vibrance),
    ("saturation", SliderKind::Saturation),
    ("clarity", SliderKind::Clarity),
    ("dehaze", SliderKind::Dehaze),
];

pub(crate) fn parse_slider_kind(name: &str) -> Option<SliderKind> {
    let lowered = name.to_ascii_lowercase();
    ALL_SLIDER_KINDS
        .iter()
        .find(|(candidate, _)| *candidate == lowered)
        .map(|(_, kind)| *kind)
}

pub(crate) fn slider_label(kind: SliderKind) -> &'static str {
    match kind {
        SliderKind::Exposure => "Exposure",
        SliderKind::Contrast => "Contrast",
        SliderKind::Highlights => "Highlights",
        SliderKind::Shadows => "Shadows",
        SliderKind::Whites => "Whites",
        SliderKind::Blacks => "Blacks",
        SliderKind::Temperature => "Temp",
        SliderKind::Tint => "Tint",
        SliderKind::Vibrance => "Vibrance",
        SliderKind::Saturation => "Saturation",
        SliderKind::Clarity => "Clarity",
        SliderKind::Dehaze => "Dehaze",
    }
}
