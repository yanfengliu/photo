//! Agent-harness protocol contract.
//!
//! JSON-stable vocabulary for the agent control channel: requests, responses,
//! observations, run manifests, and the process-wide harness configuration.
//! Design and loop documentation: `docs/guides/agent-harness.md`; thread:
//! `docs/work/10_agent-harness/historical/threads/done/agent-harness/DESIGN.md`. Protocol changes bump
//! [`HARNESS_PROTOCOL_VERSION`].

pub(crate) mod run;
pub(crate) mod server;
pub(crate) mod stats;

pub(crate) use run::{
    epoch_ms_now, generate_run_id, generate_token, harness_run_dir, sandbox_storage_dirs,
    write_manifest, RunManifest, SessionInfo,
};

use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) const HARNESS_PROTOCOL_VERSION: u32 = 1;
pub(crate) const DEFAULT_HARNESS_PORT: u16 = 7878;
pub(crate) const RUN_MANIFEST_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct HarnessRequest {
    pub(crate) id: u64,
    #[serde(flatten)]
    pub(crate) command: HarnessCommand,
}

// Hand-rolled so `params` may be omitted for commands whose parameters all
// have defaults: serde's adjacently-tagged enums otherwise require the
// content field to be present.
impl<'de> serde::Deserialize<'de> for HarnessRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| D::Error::custom("missing or non-integer `id`"))?;
        if let Some(object) = value.as_object_mut() {
            object
                .entry("params")
                .or_insert_with(|| serde_json::json!({}));
        }
        let command = HarnessCommand::deserialize(value).map_err(D::Error::custom)?;
        Ok(HarnessRequest { id, command })
    }
}

/// The command vocabulary. Action commands map to the exact `Message` values
/// real user interaction produces — the harness never bypasses `update()`.
/// Every variant is a struct variant so a missing `params` object can be
/// injected uniformly during deserialization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "cmd", content = "params", rename_all = "snake_case")]
pub(crate) enum HarnessCommand {
    Ping {},
    Observe {},
    Screenshot {},
    DumpRender {
        #[serde(default)]
        source: RenderSource,
        #[serde(default)]
        max_dim: Option<u32>,
    },
    ObserveLibrary {
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: Option<usize>,
    },
    ImageStats {
        path: String,
    },
    CompareImages {
        path_a: String,
        path_b: String,
    },
    Open {
        path: String,
    },
    ImportFiles {
        paths: Vec<String>,
    },
    ImportFolder {
        path: String,
    },
    SetSlider {
        kind: String,
        value: f32,
    },
    ResetSlider {
        kind: String,
    },
    Click {
        control: String,
        #[serde(default)]
        value: Option<String>,
    },
    Key {
        name: String,
        #[serde(default)]
        mods: Vec<String>,
    },
    SetCrop {
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    },
    WaitIdle {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Quit {},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RenderSource {
    /// The current visible edit state applied to the loaded base image.
    #[default]
    Current,
    /// The loaded base image with a default (identity) edit state.
    Original,
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HarnessResponse {
    pub(crate) id: u64,
    pub(crate) ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<HarnessError>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HarnessError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl HarnessResponse {
    pub(crate) fn success(id: u64, data: impl serde::Serialize) -> Self {
        match serde_json::to_value(data) {
            Ok(value) => Self {
                id,
                ok: true,
                data: Some(value),
                error: None,
            },
            Err(e) => Self::failure(
                id,
                "internal",
                &format!("response serialization failed: {e}"),
            ),
        }
    }

    pub(crate) fn failure(id: u64, code: &str, message: &str) -> Self {
        Self {
            id,
            ok: false,
            data: None,
            error: Some(HarnessError {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Observation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Observation {
    pub(crate) protocol_version: u32,
    pub(crate) tab: String,
    pub(crate) crop_mode: bool,
    pub(crate) current_image: Option<CurrentImageReport>,
    pub(crate) edit_state: Option<EditStateReport>,
    pub(crate) pending: PendingReport,
    pub(crate) library_count: usize,
    pub(crate) collections: Vec<String>,
    pub(crate) controls: Vec<ControlSpec>,
    pub(crate) save_status: Option<String>,
    pub(crate) error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) screenshot: Option<ScreenshotReport>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CurrentImageReport {
    pub(crate) path: String,
    /// `idle` | `loading` | `preview_while_loading` | `preview_only`.
    pub(crate) load_stage: String,
    /// Source-logical dimensions (what the status bar reports).
    pub(crate) logical_width: u32,
    pub(crate) logical_height: u32,
    /// Loaded GPU/CPU buffer dimensions (may be downscaled from logical).
    pub(crate) buffer_width: u32,
    pub(crate) buffer_height: u32,
    pub(crate) zoom_percent: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct EditStateReport {
    pub(crate) sliders: Vec<SliderReport>,
    pub(crate) lens_correction: bool,
    pub(crate) rotation_quarter_turns: u8,
    /// Normalized `[left, top, right, bottom]`, absent when uncropped.
    pub(crate) crop: Option<[f32; 4]>,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SliderReport {
    pub(crate) kind: String,
    pub(crate) value: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingReport {
    pub(crate) detail_loading: bool,
    pub(crate) exif_loading: bool,
    pub(crate) persist_in_flight: bool,
    pub(crate) persist_queued: usize,
    pub(crate) save_in_flight: bool,
    pub(crate) owed_bakes: usize,
    pub(crate) import_warm_queue: usize,
    /// The `wait_idle` condition: no detail load, EXIF read, or edit persist
    /// in flight. Import cache warming is deliberately excluded — it is a
    /// background optimization that never blocks interaction.
    pub(crate) idle: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ControlSpec {
    pub(crate) id: String,
    /// `slider` | `button` | `toggle` | `select` | `key`.
    pub(crate) kind: String,
    pub(crate) label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) step: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) options: Option<Vec<String>>,
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ScreenshotReport {
    pub(crate) path: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_factor: f64,
    /// Logical size of the image canvas region (the shader viewport), for
    /// relating window pixels to the photo area.
    pub(crate) canvas_size: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RenderReport {
    pub(crate) path: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) source: String,
    pub(crate) load_stage: String,
    pub(crate) stats: stats::ImageStatsReport,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LibraryEntryReport {
    pub(crate) path: String,
    pub(crate) filename: String,
    pub(crate) has_thumbnail: bool,
    /// `original` | `persisted_local_edit` — thumbnail base provenance.
    pub(crate) thumbnail_base: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LibraryPage {
    pub(crate) total: usize,
    pub(crate) offset: usize,
    pub(crate) entries: Vec<LibraryEntryReport>,
}

// ---------------------------------------------------------------------------
// Runtime configuration and server events
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct HarnessConfig {
    pub(crate) port: u16,
    pub(crate) run_dir: PathBuf,
    pub(crate) run_id: String,
    pub(crate) token: String,
    pub(crate) sandboxed: bool,
}

static CONFIG: OnceLock<HarnessConfig> = OnceLock::new();

pub(crate) fn config() -> Option<&'static HarnessConfig> {
    CONFIG.get()
}

/// Events the control channel feeds into the app's update loop.
#[derive(Debug, Clone)]
pub(crate) enum HarnessEvent {
    Connected {
        responder: tokio::sync::mpsc::UnboundedSender<HarnessResponse>,
    },
    Request(HarnessRequest),
    ClientDisconnected,
    ListenFailed {
        error: String,
    },
}

/// Creates the run directory tree, applies the storage sandbox, writes the
/// initial manifest, and publishes the process-wide harness configuration.
/// Called once from `main()` before the app starts; tests never call it (it
/// mutates process-global state).
pub(crate) fn prepare_runtime(launch: &crate::launch::HarnessLaunch) -> Result<(), String> {
    let base_root = crate::repo::photo_repo_root()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "cannot resolve a base directory for harness runs".to_string())?;
    let run_id = generate_run_id(epoch_ms_now(), std::process::id());
    let run_dir = harness_run_dir(&base_root, &run_id);

    std::fs::create_dir_all(run_dir.join("artifacts"))
        .map_err(|e| format!("cannot create harness run dir {}: {e}", run_dir.display()))?;

    let sandboxed = !launch.real_storage;
    if sandboxed {
        let storage = sandbox_storage_dirs(&run_dir);
        std::fs::create_dir_all(&storage.appdata)
            .map_err(|e| format!("cannot create sandbox appdata dir: {e}"))?;
        std::fs::create_dir_all(&storage.repo)
            .map_err(|e| format!("cannot create sandbox repo dir: {e}"))?;
        crate::library::set_runtime_app_storage_dir(storage.appdata);
        crate::repo::set_runtime_photo_repo_root(storage.repo);
    }

    let manifest = RunManifest::started(run_id.clone(), sandboxed);
    write_manifest(&run_dir, &manifest)?;

    let token = generate_token(epoch_ms_now(), std::process::id());
    CONFIG
        .set(HarnessConfig {
            port: launch.port,
            run_dir,
            run_id,
            token,
            sandboxed,
        })
        .map_err(|_| "harness configuration already initialized".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_with_flattened_command() {
        let request = HarnessRequest {
            id: 7,
            command: HarnessCommand::SetSlider {
                kind: "exposure".to_string(),
                value: 1.5,
            },
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"cmd\":\"set_slider\""), "{json}");
        assert!(json.contains("\"params\""), "{json}");
        let back: HarnessRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn parameterless_commands_need_no_params() {
        let request: HarnessRequest = serde_json::from_str(r#"{"id":1,"cmd":"ping"}"#).unwrap();
        assert_eq!(request.command, HarnessCommand::Ping {});

        let request: HarnessRequest = serde_json::from_str(r#"{"id":2,"cmd":"quit"}"#).unwrap();
        assert_eq!(request.command, HarnessCommand::Quit {});

        // Explicit empty params stay accepted.
        let request: HarnessRequest =
            serde_json::from_str(r#"{"id":3,"cmd":"ping","params":{}}"#).unwrap();
        assert_eq!(request.command, HarnessCommand::Ping {});

        // A request without an id is rejected outright.
        assert!(serde_json::from_str::<HarnessRequest>(r#"{"cmd":"ping"}"#).is_err());
    }

    #[test]
    fn optional_params_default() {
        let request: HarnessRequest = serde_json::from_str(r#"{"id":3,"cmd":"observe"}"#).unwrap();
        assert_eq!(request.command, HarnessCommand::Observe {});

        let request: HarnessRequest =
            serde_json::from_str(r#"{"id":4,"cmd":"dump_render"}"#).unwrap();
        assert_eq!(
            request.command,
            HarnessCommand::DumpRender {
                source: RenderSource::Current,
                max_dim: None
            }
        );

        let request: HarnessRequest =
            serde_json::from_str(r#"{"id":5,"cmd":"wait_idle","params":{"timeout_ms":250}}"#)
                .unwrap();
        assert_eq!(
            request.command,
            HarnessCommand::WaitIdle {
                timeout_ms: Some(250)
            }
        );
    }

    #[test]
    fn unknown_command_is_a_parse_error() {
        let result = serde_json::from_str::<HarnessRequest>(r#"{"id":1,"cmd":"explode"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn response_success_and_failure_shapes() {
        let ok = HarnessResponse::success(4, serde_json::json!({"pong": true}));
        let json = serde_json::to_string(&ok).unwrap();
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(!json.contains("error"), "{json}");

        let err = HarnessResponse::failure(5, "bad_request", "nope");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"code\":\"bad_request\""), "{json}");
        assert!(!json.contains("data"), "{json}");
        let back: HarnessResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, err);
    }
}
