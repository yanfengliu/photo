//! Harness artifact pipelines: screenshots, CPU render dumps, image
//! statistics, comparisons, and run-manifest finalization.
//!
//! Pixel work runs on blocking threads; results come back through
//! `HarnessMsg` completions. Artifact paths in reports are run-dir-relative
//! so session logs and manifests stay portable.

use super::*;
use crate::harness::{self, HarnessResponse, RenderReport, RenderSource, ScreenshotReport};

impl App {
    fn next_harness_artifact_path(&mut self, label: &str) -> Option<(PathBuf, String)> {
        let config = harness::config()?;
        self.harness_artifact_seq += 1;
        let relative = format!("artifacts/{:04}-{label}.png", self.harness_artifact_seq);
        Some((config.run_dir.join(&relative), relative))
    }

    pub(super) fn save_harness_screenshot(
        &mut self,
        request_id: u64,
        generation: u64,
        screenshot: iced::window::Screenshot,
    ) -> Task<Message> {
        let Some((absolute, relative)) = self.next_harness_artifact_path("screenshot") else {
            self.respond_harness(HarnessResponse::failure(
                request_id,
                "internal",
                "harness configuration missing",
            ));
            return Task::none();
        };
        let canvas_size = self.current_canvas_size();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let width = screenshot.size.width;
                    let height = screenshot.size.height;
                    let image =
                        image::RgbaImage::from_raw(width, height, screenshot.bytes.to_vec())
                            .ok_or_else(|| "screenshot byte length mismatch".to_string())?;
                    image
                        .save(&absolute)
                        .map_err(|e| format!("cannot write {}: {e}", absolute.display()))?;
                    Ok(ScreenshotReport {
                        path: relative,
                        width,
                        height,
                        scale_factor: screenshot.scale_factor,
                        canvas_size,
                    })
                })
                .await
                .map_err(|e| e.to_string())?
            },
            move |result| {
                Message::Harness(HarnessMsg::ScreenshotSaved {
                    request_id,
                    generation,
                    result,
                })
            },
        )
    }

    pub(super) fn dump_harness_render(
        &mut self,
        request_id: u64,
        source: RenderSource,
        max_dim: Option<u32>,
    ) -> Task<Message> {
        let generation = self.harness_connection_generation;
        let Some(image_data) = self.image.clone() else {
            self.respond_harness(HarnessResponse::failure(
                request_id,
                "unavailable",
                "no image loaded",
            ));
            return Task::none();
        };
        let (state, lens, label) = match source {
            RenderSource::Current => {
                let state = self.visible_edit_state();
                let lens = self.current_lens_correction(state.lens_correction);
                (state, lens, "render-current")
            }
            RenderSource::Original => (
                edit::EditState::default(),
                edit::LensCorrection::default(),
                "render-original",
            ),
        };
        let Some((absolute, relative)) = self.next_harness_artifact_path(label) else {
            self.respond_harness(HarnessResponse::failure(
                request_id,
                "internal",
                "harness configuration missing",
            ));
            return Task::none();
        };
        let load_stage = match self.detail_load.stage {
            DetailLoadStage::Idle => "idle",
            DetailLoadStage::Loading => "loading",
            DetailLoadStage::PreviewWhileLoading => "preview_while_loading",
            DetailLoadStage::PreviewOnly => "preview_only",
        }
        .to_string();
        let source_name = match source {
            RenderSource::Current => "current",
            RenderSource::Original => "original",
        }
        .to_string();
        let max_dim = max_dim.map(|dim| dim.clamp(64, 16_384));

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let rendered = edit::render_edited_image(
                        &image_data.pixels,
                        image_data.width,
                        image_data.height,
                        &state,
                        lens,
                    );
                    // Statistics describe the full-resolution render; the
                    // written PNG may be downscaled for cheap multimodal
                    // viewing.
                    let stats = harness::stats::image_stats(
                        &rendered.pixels,
                        rendered.width,
                        rendered.height,
                    )?;
                    let mut output = image::RgbaImage::from_raw(
                        rendered.width,
                        rendered.height,
                        rendered.pixels,
                    )
                    .ok_or_else(|| "rendered buffer length mismatch".to_string())?;
                    if let Some(max_dim) = max_dim {
                        let largest = output.width().max(output.height());
                        if largest > max_dim {
                            let scale = max_dim as f32 / largest as f32;
                            let new_width = (output.width() as f32 * scale).round().max(1.0) as u32;
                            let new_height =
                                (output.height() as f32 * scale).round().max(1.0) as u32;
                            output = image::imageops::resize(
                                &output,
                                new_width,
                                new_height,
                                image::imageops::FilterType::Triangle,
                            );
                        }
                    }
                    let (width, height) = (output.width(), output.height());
                    output
                        .save(&absolute)
                        .map_err(|e| format!("cannot write {}: {e}", absolute.display()))?;
                    Ok(RenderReport {
                        path: relative,
                        width,
                        height,
                        source: source_name,
                        load_stage,
                        stats,
                    })
                })
                .await
                .map_err(|e| e.to_string())?
            },
            move |result| {
                Message::Harness(HarnessMsg::RenderDumped {
                    request_id,
                    generation,
                    result,
                })
            },
        )
    }

    fn resolve_harness_input_path(path: &str) -> PathBuf {
        let candidate = PathBuf::from(path);
        if candidate.is_relative() {
            if let Some(config) = harness::config() {
                return config.run_dir.join(candidate);
            }
        }
        candidate
    }

    pub(super) fn compute_harness_image_stats(
        &mut self,
        request_id: u64,
        path: String,
    ) -> Task<Message> {
        let generation = self.harness_connection_generation;
        let resolved = Self::resolve_harness_input_path(&path);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let image = image::open(&resolved)
                        .map_err(|e| format!("cannot open {}: {e}", resolved.display()))?
                        .to_rgba8();
                    let report =
                        harness::stats::image_stats(image.as_raw(), image.width(), image.height())?;
                    serde_json::to_value(report).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())?
            },
            move |result| {
                Message::Harness(HarnessMsg::StatsComputed {
                    request_id,
                    generation,
                    result,
                })
            },
        )
    }

    pub(super) fn compute_harness_compare(
        &mut self,
        request_id: u64,
        path_a: String,
        path_b: String,
    ) -> Task<Message> {
        let generation = self.harness_connection_generation;
        let resolved_a = Self::resolve_harness_input_path(&path_a);
        let resolved_b = Self::resolve_harness_input_path(&path_b);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let image_a = image::open(&resolved_a)
                        .map_err(|e| format!("cannot open {}: {e}", resolved_a.display()))?
                        .to_rgba8();
                    let image_b = image::open(&resolved_b)
                        .map_err(|e| format!("cannot open {}: {e}", resolved_b.display()))?
                        .to_rgba8();
                    let report = harness::stats::compare_images(
                        image_a.as_raw(),
                        image_a.width(),
                        image_a.height(),
                        image_b.as_raw(),
                        image_b.width(),
                        image_b.height(),
                    )?;
                    serde_json::to_value(report).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())?
            },
            move |result| {
                Message::Harness(HarnessMsg::StatsComputed {
                    request_id,
                    generation,
                    result,
                })
            },
        )
    }

    // -----------------------------------------------------------------------
    // Manifest
    // -----------------------------------------------------------------------

    pub(super) fn finalize_harness_manifest(&mut self, stop_reason: &str) {
        let Some(config) = harness::config() else {
            return;
        };
        let mut manifest = harness::RunManifest::started(config.run_id.clone(), config.sandboxed);
        manifest.finalize(stop_reason, self.harness_artifacts.clone());
        // Preserve the true start instant from the manifest written at launch.
        if let Ok(body) = std::fs::read_to_string(config.run_dir.join("manifest.json")) {
            if let Ok(original) = serde_json::from_str::<harness::RunManifest>(&body) {
                manifest.started_at_epoch_ms = original.started_at_epoch_ms;
                manifest.started_at_utc = original.started_at_utc;
            }
        }
        if let Err(e) = harness::write_manifest(&config.run_dir, &manifest) {
            log::error!("cannot finalize harness manifest: {e}");
        }
    }
}
