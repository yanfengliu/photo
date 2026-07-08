//! Harness observation builders: UI state, typed controls, and library pages.
//!
//! Read-only projections of `App` for the agent. The controls list is
//! generated from the same enum and range sources the view uses, so what the
//! agent discovers cannot drift from what the panel renders.

use super::harness_actions::{slider_label, ALL_SLIDER_KINDS};
use super::*;
use crate::harness::{
    self, ControlSpec, CurrentImageReport, EditStateReport, LibraryEntryReport, LibraryPage,
    Observation, PendingReport, SliderReport,
};
use crate::loading::BaseImageSource;

impl App {
    pub(crate) fn build_harness_observation(&self) -> Observation {
        let current_image = self.current_image_path.as_ref().map(|path| {
            let (buffer_width, buffer_height) = self
                .image
                .as_ref()
                .map(|image| (image.width, image.height))
                .unwrap_or((0, 0));
            let (logical_width, logical_height) = self
                .current_image_source_dimensions
                .unwrap_or((buffer_width, buffer_height));
            CurrentImageReport {
                path: path.display().to_string(),
                load_stage: match self.detail_load.stage {
                    DetailLoadStage::Idle => "idle",
                    DetailLoadStage::Loading => "loading",
                    DetailLoadStage::PreviewWhileLoading => "preview_while_loading",
                    DetailLoadStage::PreviewOnly => "preview_only",
                }
                .to_string(),
                logical_width,
                logical_height,
                buffer_width,
                buffer_height,
                zoom_percent: self.zoom * 100.0,
            }
        });

        let edit_state = self.current_image_path.as_ref().map(|path| {
            let history = self.edit_histories.get(path);
            let state = history.map(|h| h.current).unwrap_or_default();
            EditStateReport {
                sliders: ALL_SLIDER_KINDS
                    .iter()
                    .map(|(name, kind)| SliderReport {
                        kind: name.to_string(),
                        value: get_slider_field(&state, *kind),
                    })
                    .collect(),
                lens_correction: state.lens_correction,
                rotation_quarter_turns: state.rotation.as_u8(),
                crop: state
                    .crop
                    .map(|crop| [crop.left, crop.top, crop.right, crop.bottom]),
                can_undo: history.is_some_and(edit::UndoHistory::can_undo),
                can_redo: history.is_some_and(edit::UndoHistory::can_redo),
            }
        });

        Observation {
            protocol_version: harness::HARNESS_PROTOCOL_VERSION,
            tab: match self.tab {
                Tab::Library => "library",
                Tab::Detail => "detail",
            }
            .to_string(),
            crop_mode: self.crop_mode,
            current_image,
            edit_state,
            pending: self.build_harness_pending_report(),
            library_count: self.library.len(),
            collections: self
                .collection_store
                .collections
                .iter()
                .map(|collection| collection.name.clone())
                .collect(),
            controls: self.build_harness_controls(),
            save_status: self.save_status.clone(),
            error: self.error.clone(),
            screenshot: None,
        }
    }

    pub(super) fn build_harness_pending_report(&self) -> PendingReport {
        PendingReport {
            detail_loading: self.detail_load.is_loading(),
            exif_loading: self.detail_load.exif_loading,
            persist_in_flight: self.local_edit_persist_in_flight.is_some(),
            persist_queued: self.pending_local_edit_persist_requests.len(),
            save_in_flight: self.save_in_flight,
            owed_bakes: self.owed_local_edit_bakes.len(),
            import_warm_queue: self.pending_import_cache_warm_paths.len()
                + usize::from(self.import_cache_warm_in_flight.is_some()),
            idle: self.harness_is_idle(),
        }
    }

    pub(crate) fn harness_is_idle(&self) -> bool {
        !self.detail_load.is_loading()
            && !self.detail_load.exif_loading
            && !self.save_in_flight
            && self.local_edit_persist_in_flight.is_none()
            && self.pending_local_edit_persist_requests.is_empty()
            && self.owed_local_edit_bakes.is_empty()
    }

    /// The single availability predicate behind both the `observe` controls
    /// list and the `click` gate. `None` means the control id is unknown.
    pub(super) fn harness_control_enabled(&self, control: &str) -> Option<bool> {
        let in_detail_with_image = self.tab == Tab::Detail && self.current_image_path.is_some();
        match control {
            "save" => Some(self.current_save_request().is_some()),
            "back" => Some(self.tab == Tab::Detail),
            "rotate_cw" | "rotate_ccw" | "lens_correction" | "crop" | "crop_clear"
            | "reset_all" | "crop_aspect" | "lens_profile" => Some(in_detail_with_image),
            // Dialog-openers are permanently unavailable to the harness but
            // recognized, so `click` can explain the import_* alternative.
            "add_folder" | "add_files" => Some(true),
            _ => None,
        }
    }

    fn build_harness_controls(&self) -> Vec<ControlSpec> {
        let in_detail_with_image = self.tab == Tab::Detail && self.current_image_path.is_some();
        let state = self.visible_edit_state();
        let mut controls = Vec::with_capacity(ALL_SLIDER_KINDS.len() + 12);

        for (name, kind) in ALL_SLIDER_KINDS {
            let (min, max) = slider_range(*kind);
            controls.push(ControlSpec {
                id: name.to_string(),
                kind: "slider".to_string(),
                label: slider_label(*kind).to_string(),
                min: Some(min),
                max: Some(max),
                step: Some(slider_step(*kind)),
                value: Some(serde_json::json!(get_slider_field(&state, *kind))),
                options: None,
                enabled: in_detail_with_image,
            });
        }

        let button = |id: &str, label: &str, enabled: bool| ControlSpec {
            id: id.to_string(),
            kind: "button".to_string(),
            label: label.to_string(),
            min: None,
            max: None,
            step: None,
            value: None,
            options: None,
            enabled,
        };

        let enabled = |id: &str| self.harness_control_enabled(id).unwrap_or(false);

        controls.push(button("save", "Save", enabled("save")));
        controls.push(button("back", "Back to Library", enabled("back")));
        controls.push(button("rotate_cw", "Rotate +90°", enabled("rotate_cw")));
        controls.push(button("rotate_ccw", "Rotate -90°", enabled("rotate_ccw")));
        controls.push(button(
            "crop",
            if self.crop_mode {
                "Finish Crop"
            } else {
                "Crop"
            },
            enabled("crop"),
        ));
        controls.push(button("crop_clear", "Clear Crop", enabled("crop_clear")));
        controls.push(button("reset_all", "Reset All", enabled("reset_all")));

        controls.push(ControlSpec {
            id: "lens_correction".to_string(),
            kind: "toggle".to_string(),
            label: "Lens Correction".to_string(),
            min: None,
            max: None,
            step: None,
            value: Some(serde_json::json!(state.lens_correction)),
            options: None,
            enabled: enabled("lens_correction"),
        });
        controls.push(ControlSpec {
            id: "crop_aspect".to_string(),
            kind: "select".to_string(),
            label: "Crop Aspect".to_string(),
            min: None,
            max: None,
            step: None,
            value: Some(serde_json::json!(self.crop_aspect.to_string())),
            options: Some(vec!["Freeform".to_string(), "Square".to_string()]),
            enabled: enabled("crop_aspect"),
        });
        controls.push(ControlSpec {
            id: "lens_profile".to_string(),
            kind: "select".to_string(),
            label: format!(
                "Lens Profile ({} database profiles; pass the exact \"Maker Model\" string)",
                self.lens_db.profiles.len()
            ),
            min: None,
            max: None,
            step: None,
            value: Some(serde_json::json!(self
                .lens_override_name
                .clone()
                .unwrap_or_else(|| "Auto".to_string()))),
            options: Some(vec!["Auto".to_string(), "None".to_string()]),
            enabled: enabled("lens_profile"),
        });

        let key = |id: &str, label: &str, enabled: bool| ControlSpec {
            id: id.to_string(),
            kind: "key".to_string(),
            label: label.to_string(),
            min: None,
            max: None,
            step: None,
            value: None,
            options: None,
            enabled,
        };
        controls.push(key("escape", "Back / cancel crop / dismiss", true));
        controls.push(key(
            "right",
            "Next image (also: space)",
            self.tab == Tab::Detail,
        ));
        controls.push(key(
            "left",
            "Previous image (also: backspace)",
            self.tab == Tab::Detail,
        ));
        controls.push(key("z+ctrl", "Undo", in_detail_with_image));
        controls.push(key("y+ctrl", "Redo", in_detail_with_image));
        controls.push(key("s+ctrl", "Save edited copy", in_detail_with_image));
        controls.push(key(
            "f",
            "Fit to window (also: 0, home)",
            self.tab == Tab::Detail,
        ));
        controls.push(key("1", "Actual size", self.tab == Tab::Detail));
        controls.push(key("=", "Zoom in (also: +)", self.tab == Tab::Detail));
        controls.push(key("-", "Zoom out (also: _)", self.tab == Tab::Detail));

        controls
    }

    pub(super) fn build_harness_library_page(
        &self,
        offset: usize,
        limit: Option<usize>,
    ) -> LibraryPage {
        let limit = limit.unwrap_or(100).min(1000);
        let entries = self
            .library
            .iter()
            .skip(offset)
            .take(limit)
            .map(|entry| LibraryEntryReport {
                path: entry.path.display().to_string(),
                filename: entry.filename.clone(),
                has_thumbnail: entry.thumbnail_handle.is_some(),
                thumbnail_base: match entry.thumbnail_base_source {
                    BaseImageSource::Original => "original",
                    BaseImageSource::PersistedLocalEdit => "persisted_local_edit",
                }
                .to_string(),
            })
            .collect();
        LibraryPage {
            total: self.library.len(),
            offset,
            entries,
        }
    }
}
