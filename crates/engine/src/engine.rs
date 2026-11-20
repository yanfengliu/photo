//! The engine itself: opening, polling, waiting, and the plumbing between
//! the engine's thread and its workers.
//!
//! Workers never touch the catalog. A job computes on a worker and returns
//! an [`Apply`]: a closure that the engine's thread runs during
//! [`Engine::poll`] to fold the result into state and into catalog writes.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use photo_catalog::Catalog;
use photo_core::meta::PhotoId;

use crate::browse::{Attribute, Browser, Change, Refresh};
use crate::error::EngineError;
use crate::import::Imports;
use crate::jobs::{self, CancelToken, Job, JobContext, JobId, JobKey, Lane, Outcome, Pool};
use crate::previews::Previews;
use crate::selection::Selection;
use crate::sidecars::Sidecars;
use crate::tasks::Tasks;
use crate::toasts::Toasts;
use crate::undo::History;

/// What a finished job hands back: run on the engine's thread by
/// [`Engine::poll`], it returns whether anything visible changed.
pub(crate) type Apply = Box<dyn FnOnce(&mut Engine) -> bool + Send>;

/// What a job's owner does when the job did not run to completion.
type OnFailure = Box<dyn FnOnce(&mut Engine, JobFailure) -> bool>;

/// The context a job's work runs in.
pub(crate) type Context<'a> = JobContext<'a, Apply, Apply>;

type EngineEvent = jobs::Event<Apply, Apply>;

/// Why a job produced no result.
#[derive(Clone, Debug)]
pub(crate) enum JobFailure {
    /// It never ran: its task was cancelled first.
    Skipped,
    /// It panicked, with this message.
    Panicked(String),
}

/// Where an engine keeps its files and how much it may run in parallel.
/// Nothing here has a default: every path is chosen by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// The catalog file; created if it does not exist.
    pub catalog_path: PathBuf,
    /// The folder preview files are kept in; created if it does not exist.
    pub previews_dir: PathBuf,
    /// How many background worker threads to run (at least one is).
    pub worker_threads: usize,
}

/// Work the engine has not finished, for an automation driver to report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PendingWork {
    /// Jobs submitted whose results have not been polled in yet.
    pub jobs: usize,
    /// Of those, the jobs still waiting for a worker.
    pub queued: usize,
    /// Previews being made.
    pub previews: usize,
    /// Background tasks still running.
    pub tasks: usize,
}

/// The whole application without a user interface.
///
/// One `Engine` lives on one thread, which owns the catalog. Background work
/// runs on worker threads that never touch the catalog; call
/// [`Engine::poll`] (once per frame) to bring their results in. Dropping the
/// engine stops the workers and waits for them, so no thread outlives it.
pub struct Engine {
    pub(crate) catalog: Catalog,
    pub(crate) previews_dir: PathBuf,
    pub(crate) browser: Browser,
    pub(crate) selection: Selection,
    pub(crate) history: History,
    pub(crate) previews: Previews,
    pub(crate) imports: Imports,
    pub(crate) sidecars: Sidecars,
    pub(crate) tasks: Tasks,
    pub(crate) toasts: Toasts,
    /// Missing-file states found by workers, written in one go per poll.
    pub(crate) missing: Vec<(PhotoId, bool)>,
    jobs: HashMap<JobId, OnFailure>,
    pool: Pool<Apply, Apply>,
    events: Receiver<EngineEvent>,
}

impl Engine {
    /// Open the catalog at `config.catalog_path` (creating it if needed),
    /// make sure the previews folder exists, start the workers and load the
    /// photo list for all photos.
    pub fn open(config: EngineConfig) -> Result<Self, EngineError> {
        let catalog = Catalog::open(&config.catalog_path)?;
        fs::create_dir_all(&config.previews_dir).map_err(|source| EngineError::PreviewsFolder {
            path: config.previews_dir.clone(),
            source,
        })?;
        let (sender, events) = mpsc::channel();
        let pool = Pool::start(config.worker_threads, &sender).map_err(EngineError::Workers)?;
        let quick = catalog.quick_collection();
        let mut engine = Self {
            catalog,
            previews_dir: config.previews_dir,
            browser: Browser::new(quick),
            selection: Selection::default(),
            history: History::default(),
            previews: Previews::default(),
            imports: Imports::default(),
            sidecars: Sidecars::default(),
            tasks: Tasks::default(),
            toasts: Toasts::default(),
            missing: Vec::new(),
            jobs: HashMap::new(),
            pool,
            events,
        };
        engine.refresh(&Refresh::all())?;
        Ok(engine)
    }

    /// Bring in everything the workers have finished. Returns whether
    /// anything the UI shows may have changed. Never blocks.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            changed |= self.handle(event);
        }
        changed | self.settle()
    }

    /// True when no job is queued or running and every result has been
    /// polled in.
    pub fn is_idle(&self) -> bool {
        self.jobs.is_empty() && self.missing.is_empty() && self.tasks.running() == 0
    }

    /// Poll until the engine is idle or `timeout` has passed, blocking on
    /// the workers' results in between rather than sleeping. Returns
    /// whether it became idle.
    pub fn wait_idle(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.poll();
            if self.is_idle() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match self.events.recv_timeout(left) {
                Ok(event) => {
                    self.handle(event);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.poll();
                    return self.is_idle();
                }
            }
        }
    }

    /// Set the callback workers call whenever they have posted a result, so
    /// the UI can repaint and poll without a timer. It runs on a worker
    /// thread and should only schedule a repaint.
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.pool.set_wake(Some(wake));
    }

    /// Work that is still outstanding.
    pub fn pending_work(&self) -> PendingWork {
        PendingWork {
            jobs: self.jobs.len(),
            queued: self.pool.queued(),
            previews: self.previews.in_flight(),
            tasks: self.tasks.running(),
        }
    }

    /// The value the UI stored under `key`, if any. Kept in the catalog, in
    /// a namespace of its own.
    pub fn ui_setting(&self, key: &str) -> Result<Option<String>, EngineError> {
        Ok(self.catalog.setting(&ui_key(key))?)
    }

    /// Store a value for the UI under `key`. Every call is a catalog write,
    /// so store panel state when it changes, not every frame.
    pub fn set_ui_setting(&mut self, key: &str, value: &str) -> Result<(), EngineError> {
        Ok(self.catalog.set_setting(&ui_key(key), value)?)
    }

    /// Queue a job. `work` runs on a worker and returns what the engine's
    /// thread then applies; `on_failure` runs instead when the job was
    /// skipped or panicked.
    pub(crate) fn submit(
        &mut self,
        lane: Lane,
        key: Option<JobKey>,
        cancel: Option<CancelToken>,
        work: impl FnOnce(&Context<'_>) -> Apply + Send + 'static,
        on_failure: impl FnOnce(&mut Engine, JobFailure) -> bool + 'static,
    ) -> JobId {
        let submitted = self.pool.submit(Job {
            lane,
            key,
            cancel,
            work: Box::new(work),
        });
        if let Some(replaced) = submitted.replaced {
            self.jobs.remove(&replaced);
        }
        self.jobs.insert(submitted.id, Box::new(on_failure));
        submitted.id
    }

    /// Move a queued interactive job to the front of its lane.
    pub(crate) fn promote(&self, key: &JobKey) {
        self.pool.promote(key);
    }

    fn handle(&mut self, event: EngineEvent) -> bool {
        match event {
            jobs::Event::Progress { progress, .. } => progress(self),
            jobs::Event::Done { job, outcome } => {
                let on_failure = self.jobs.remove(&job);
                let failure = match outcome {
                    Outcome::Finished(apply) => return apply(self),
                    Outcome::Skipped => JobFailure::Skipped,
                    Outcome::Panicked(message) => {
                        log::error!("a background job panicked: {message}");
                        JobFailure::Panicked(message)
                    }
                };
                on_failure.is_some_and(|on_failure| on_failure(self, failure))
            }
        }
    }

    /// Catalog writes collected while handling events, done once per poll.
    fn settle(&mut self) -> bool {
        if self.missing.is_empty() {
            return false;
        }
        let marks = std::mem::take(&mut self.missing);
        let mut photos = Vec::new();
        for state in [true, false] {
            let batch: Vec<PhotoId> = marks
                .iter()
                .filter(|(_, missing)| *missing == state)
                .map(|(photo, _)| *photo)
                .collect();
            if batch.is_empty() {
                continue;
            }
            match self.catalog.set_missing(&batch, state) {
                Ok(()) => photos.extend(batch),
                Err(error) => self.toasts.error(format!("Could not record missing files: {error}")),
            }
        }
        self.refresh_after(&[Change::Attribute(Attribute::Missing, photos)]);
        true
    }

    /// Re-read what `refresh` asks for and carry the selection over.
    pub(crate) fn refresh(&mut self, refresh: &Refresh) -> Result<(), EngineError> {
        if let Some(previous) = self.browser.apply(&self.catalog, refresh)? {
            self.selection.retain(&previous, &self.browser);
        }
        Ok(())
    }

    /// Re-read what `changes` made stale. A failure to re-read is reported
    /// as a toast: the change itself has already happened.
    pub(crate) fn refresh_after(&mut self, changes: &[Change]) {
        let mut plan = Refresh::default();
        for change in changes {
            self.browser.plan(change, &mut plan);
        }
        if let Err(error) = self.refresh(&plan) {
            self.toasts.error(format!("Could not refresh the photo list: {error}"));
        }
    }
}

fn ui_key(key: &str) -> String {
    format!("ui.{key}")
}
