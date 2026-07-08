//! Harness command execution: the dispatcher bridging the harness protocol
//! and `App`.
//!
//! Every action command dispatches the exact `Message` values the equivalent
//! real user interaction produces (slider drag sequences, the keyboard path,
//! viewer crop commits) — never direct state mutation. Action responses mean
//! "the gesture was performed", not "all resulting async work finished";
//! `wait_idle` is the synchronization point. Actions live in
//! `harness_actions.rs`, observation builders in `harness_observe.rs`, and
//! artifact pipelines in `harness_artifacts.rs`.

use super::harness_actions::parse_slider_kind;
use super::*;
use crate::harness::{
    self, HarnessCommand, HarnessEvent, HarnessRequest, HarnessResponse, RenderReport,
    ScreenshotReport,
};
/// Harness-internal messages carried on `Message::Harness`.
///
/// Async completions carry the connection `generation` current when their
/// command was dispatched. A completion whose generation is no longer current
/// must not be answered: with reconnecting clients that reuse request ids, a
/// stale response would otherwise be silently accepted as the answer to a
/// DIFFERENT command on the next connection.
#[derive(Debug, Clone)]
pub(crate) enum HarnessMsg {
    Event(HarnessEvent),
    ScreenshotCaptured {
        request_id: u64,
        generation: u64,
        screenshot: iced::window::Screenshot,
    },
    ScreenshotSaved {
        request_id: u64,
        generation: u64,
        result: Result<ScreenshotReport, String>,
    },
    RenderDumped {
        request_id: u64,
        generation: u64,
        result: Result<RenderReport, String>,
    },
    StatsComputed {
        request_id: u64,
        generation: u64,
        result: Result<serde_json::Value, String>,
    },
    IdlePoll,
    QuitNow,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HarnessIdleWaiter {
    request_id: u64,
    deadline: Instant,
}

const IDLE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const DEFAULT_WAIT_IDLE_TIMEOUT_MS: u64 = 30_000;
const MAX_WAIT_IDLE_TIMEOUT_MS: u64 = 600_000;
/// Grace period for the socket writer to flush the `quit` response before the
/// process exits.
const QUIT_FLUSH_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

impl App {
    pub(crate) fn handle_harness(&mut self, msg: HarnessMsg) -> Task<Message> {
        match msg {
            HarnessMsg::Event(HarnessEvent::Connected { responder }) => {
                self.harness_connection_generation += 1;
                self.harness_responder = Some(responder);
                Task::none()
            }
            HarnessMsg::Event(HarnessEvent::ClientDisconnected) => {
                self.harness_responder = None;
                self.harness_idle_waiters.clear();
                Task::none()
            }
            HarnessMsg::Event(HarnessEvent::ListenFailed { error }) => {
                log::error!("harness control channel unavailable: {error}");
                Task::none()
            }
            HarnessMsg::Event(HarnessEvent::Request(request)) => self.execute_harness(request),
            HarnessMsg::ScreenshotCaptured {
                request_id,
                generation,
                screenshot,
            } => self.save_harness_screenshot(request_id, generation, screenshot),
            HarnessMsg::ScreenshotSaved {
                request_id,
                generation,
                result,
            } => {
                // The artifact exists on disk regardless of who asked for it;
                // the manifest stays truthful even when the response is stale.
                if let Ok(report) = &result {
                    self.harness_artifacts.push(report.path.clone());
                }
                self.respond_harness_result_if_current(request_id, generation, result);
                Task::none()
            }
            HarnessMsg::RenderDumped {
                request_id,
                generation,
                result,
            } => {
                if let Ok(report) = &result {
                    self.harness_artifacts.push(report.path.clone());
                }
                self.respond_harness_result_if_current(request_id, generation, result);
                Task::none()
            }
            HarnessMsg::StatsComputed {
                request_id,
                generation,
                result,
            } => {
                self.respond_harness_result_if_current(request_id, generation, result);
                Task::none()
            }
            HarnessMsg::IdlePoll => self.poll_harness_idle(),
            HarnessMsg::QuitNow => iced::exit(),
        }
    }

    fn execute_harness(&mut self, request: HarnessRequest) -> Task<Message> {
        let id = request.id;
        if self.harness_quitting {
            self.respond_harness(HarnessResponse::failure(id, "quitting", "app is quitting"));
            return Task::none();
        }
        match request.command {
            HarnessCommand::Ping {} => {
                self.respond_harness(HarnessResponse::success(
                    id,
                    serde_json::json!({
                        "pong": true,
                        "protocol_version": harness::HARNESS_PROTOCOL_VERSION,
                        "app_version": env!("CARGO_PKG_VERSION"),
                    }),
                ));
                Task::none()
            }
            HarnessCommand::Observe {} => {
                let observation = self.build_harness_observation();
                self.respond_harness(HarnessResponse::success(id, observation));
                Task::none()
            }
            HarnessCommand::Screenshot {} => {
                let generation = self.harness_connection_generation;
                window::get_latest().then(move |window_id| match window_id {
                    Some(window_id) => window::screenshot(window_id).map(move |screenshot| {
                        Message::Harness(HarnessMsg::ScreenshotCaptured {
                            request_id: id,
                            generation,
                            screenshot,
                        })
                    }),
                    None => Task::done(Message::Harness(HarnessMsg::ScreenshotSaved {
                        request_id: id,
                        generation,
                        result: Err("no window available".to_string()),
                    })),
                })
            }
            HarnessCommand::DumpRender { source, max_dim } => {
                self.dump_harness_render(id, source, max_dim)
            }
            HarnessCommand::ObserveLibrary { offset, limit } => {
                let page = self.build_harness_library_page(offset, limit);
                self.respond_harness(HarnessResponse::success(id, page));
                Task::none()
            }
            HarnessCommand::ImageStats { path } => self.compute_harness_image_stats(id, path),
            HarnessCommand::CompareImages { path_a, path_b } => {
                self.compute_harness_compare(id, path_a, path_b)
            }
            HarnessCommand::Open { path } => {
                let path = PathBuf::from(&path);
                if !path.exists() {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        &format!("path does not exist: {}", path.display()),
                    ));
                    return Task::none();
                }
                let task = self.update(Message::FileSelected(Some(path)));
                self.respond_harness_accepted(id);
                task
            }
            HarnessCommand::ImportFiles { paths } => {
                let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
                if paths.is_empty() {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        "no paths given",
                    ));
                    return Task::none();
                }
                // The real file dialog can only ever return existing files;
                // phantom paths would otherwise persist into library.txt with
                // no UI affordance to remove them.
                let missing: Vec<String> = paths
                    .iter()
                    .filter(|path| !path.is_file())
                    .map(|path| path.display().to_string())
                    .collect();
                if !missing.is_empty() {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        &format!(
                            "paths a file dialog could never produce (missing or not files): {}",
                            missing.join(", ")
                        ),
                    ));
                    return Task::none();
                }
                let task = self.update(Message::FilesPicked(Some(paths)));
                self.respond_harness_accepted(id);
                task
            }
            HarnessCommand::ImportFolder { path } => {
                let path = PathBuf::from(&path);
                if !path.is_dir() {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        &format!("not a directory: {}", path.display()),
                    ));
                    return Task::none();
                }
                let task = self.update(Message::FolderPicked(Some(path)));
                self.respond_harness_accepted(id);
                task
            }
            HarnessCommand::SetSlider { kind, value } => self.set_harness_slider(id, &kind, value),
            HarnessCommand::ResetSlider { kind } => match parse_slider_kind(&kind) {
                Some(kind) => {
                    let task = self.update(Message::ResetSlider(kind));
                    self.respond_harness_accepted(id);
                    task
                }
                None => {
                    self.respond_unknown_slider(id, &kind);
                    Task::none()
                }
            },
            HarnessCommand::Click { control, value } => {
                self.click_harness_control(id, &control, value)
            }
            HarnessCommand::Key { name, mods } => self.press_harness_key(id, &name, &mods),
            HarnessCommand::SetCrop {
                left,
                top,
                right,
                bottom,
            } => {
                if !self.crop_mode {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "unavailable",
                        "crop mode is off — click the 'crop' control first (a user can only drag a crop while crop mode is active)",
                    ));
                    return Task::none();
                }
                let rect = edit::CropRect::new(left, top, right, bottom);
                if rect.width() <= 0.0 || rect.height() <= 0.0 {
                    self.respond_harness(HarnessResponse::failure(
                        id,
                        "invalid_params",
                        "crop rectangle has zero area",
                    ));
                    return Task::none();
                }
                let task = self.update(Message::Viewer(ViewerEvent::CropCommitted { rect }));
                self.respond_harness_accepted(id);
                task
            }
            HarnessCommand::WaitIdle { timeout_ms } => self.wait_harness_idle(id, timeout_ms),
            HarnessCommand::Quit {} => {
                self.harness_quitting = true;
                self.finalize_harness_manifest("quit");
                self.respond_harness(HarnessResponse::success(
                    id,
                    serde_json::json!({"quitting": true}),
                ));
                // Async block keeps the sleep lazy: no tokio reactor exists in
                // unit tests, and dropped tasks must not touch the timer.
                Task::perform(async { tokio::time::sleep(QUIT_FLUSH_DELAY).await }, |_| {
                    Message::Harness(HarnessMsg::QuitNow)
                })
            }
        }
    }

    // -----------------------------------------------------------------------
    // Responses
    // -----------------------------------------------------------------------

    pub(super) fn respond_harness(&mut self, response: HarnessResponse) {
        if let Some(responder) = &self.harness_responder {
            if responder.send(response).is_err() {
                self.harness_responder = None;
            }
        } else {
            log::warn!(
                "harness response {} dropped: no client connected",
                response.id
            );
        }
    }

    pub(super) fn respond_harness_accepted(&mut self, id: u64) {
        self.respond_harness(HarnessResponse::success(
            id,
            serde_json::json!({"accepted": true}),
        ));
    }

    /// Responds only when `generation` is still the current connection's.
    /// A stale completion (its client disconnected; possibly a NEW client
    /// connected since) is dropped: request ids are per-connection, so
    /// delivering it would let the next client mis-correlate it with an
    /// unrelated request.
    fn respond_harness_result_if_current(
        &mut self,
        id: u64,
        generation: u64,
        result: Result<impl serde::Serialize, String>,
    ) {
        if generation != self.harness_connection_generation {
            log::info!(
                "harness response {id} dropped: its connection (generation {generation}) is gone"
            );
            return;
        }
        let response = match result {
            Ok(data) => HarnessResponse::success(id, data),
            Err(message) => HarnessResponse::failure(id, "io", &message),
        };
        self.respond_harness(response);
    }

    // -----------------------------------------------------------------------
    // wait_idle
    // -----------------------------------------------------------------------

    fn wait_harness_idle(&mut self, id: u64, timeout_ms: Option<u64>) -> Task<Message> {
        if self.harness_is_idle() {
            let pending = self.build_harness_pending_report();
            self.respond_harness(HarnessResponse::success(
                id,
                serde_json::json!({"idle": true, "pending": pending}),
            ));
            return Task::none();
        }
        let timeout_ms = timeout_ms
            .unwrap_or(DEFAULT_WAIT_IDLE_TIMEOUT_MS)
            .min(MAX_WAIT_IDLE_TIMEOUT_MS);
        self.harness_idle_waiters.push(HarnessIdleWaiter {
            request_id: id,
            deadline: Instant::now() + std::time::Duration::from_millis(timeout_ms),
        });
        self.arm_harness_idle_poll()
    }

    fn arm_harness_idle_poll(&mut self) -> Task<Message> {
        if self.harness_idle_poll_armed {
            return Task::none();
        }
        self.harness_idle_poll_armed = true;
        // Async block keeps the sleep lazy (see the quit handler note).
        Task::perform(
            async { tokio::time::sleep(IDLE_POLL_INTERVAL).await },
            |_| Message::Harness(HarnessMsg::IdlePoll),
        )
    }

    fn poll_harness_idle(&mut self) -> Task<Message> {
        self.harness_idle_poll_armed = false;
        let idle = self.harness_is_idle();
        let now = Instant::now();
        let pending = self.build_harness_pending_report();

        let waiters = std::mem::take(&mut self.harness_idle_waiters);
        for waiter in waiters {
            if idle {
                self.respond_harness(HarnessResponse::success(
                    waiter.request_id,
                    serde_json::json!({"idle": true, "pending": pending}),
                ));
            } else if now >= waiter.deadline {
                self.respond_harness(HarnessResponse::failure(
                    waiter.request_id,
                    "timeout",
                    &format!(
                        "not idle before the deadline; pending: {}",
                        serde_json::to_string(&pending).unwrap_or_default()
                    ),
                ));
            } else {
                self.harness_idle_waiters.push(waiter);
            }
        }

        if self.harness_idle_waiters.is_empty() {
            Task::none()
        } else {
            self.arm_harness_idle_poll()
        }
    }
}
