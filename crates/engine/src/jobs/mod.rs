//! The worker pool: threads that run jobs away from the engine's thread and
//! post what they found back over a channel.
//!
//! Guarantees:
//!
//! - Every submitted job produces exactly one [`Event::Done`], after any
//!   [`Event::Progress`] it posted, unless the pool is dropped first.
//! - A job that panics is reported as [`Outcome::Panicked`]; the worker that
//!   ran it carries on with the next job.
//! - A job whose task was cancelled before it started is reported as
//!   [`Outcome::Skipped`] without running.
//! - Dropping the pool abandons queued jobs, asks running ones to stop (see
//!   [`JobContext::should_stop`]) and joins every thread before it returns.
//! - The wake callback runs after every event is posted, on the worker's
//!   thread; a panic inside it is contained.

mod queue;

use std::any::Any;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use photo_core::meta::PhotoId;

use crate::previews::PreviewSize;
use queue::Queue;

/// Identifies one submitted job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct JobId(u64);

/// Which line a job waits in; see [`queue`] for the order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    /// The photo open in Develop: ahead of everything, in order.
    Urgent,
    /// Pictures someone is waiting to see: newest request first.
    Interactive,
    /// Everything else: first come, first served.
    Background,
}

/// What an interactive job is for, so that a repeated request can find it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum JobKey {
    /// A preview of a catalogued photo.
    Preview(PhotoId, PreviewSize),
    /// The thumbnail of a file offered for import.
    Candidate(PathBuf),
}

/// A flag shared by the jobs of one task, asking them to stop.
#[derive(Clone, Debug, Default)]
pub(crate) struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Ask every job holding this token to stop.
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// True once [`Self::cancel`] has been called.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The callback that tells the UI results are waiting to be polled.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// What a worker posts to the engine.
pub(crate) enum Event<O, P> {
    /// Something a job found on its way; always before its `Done`.
    Progress(P),
    /// A job has ended.
    Done {
        /// The job.
        job: JobId,
        /// How it ended.
        outcome: Outcome<O>,
    },
}

/// How a job ended.
pub(crate) enum Outcome<O> {
    /// It ran to completion and returned this.
    Finished(O),
    /// It never ran: its task was cancelled, or the pool was shutting down.
    Skipped,
    /// It panicked, with this message.
    Panicked(String),
}

/// The code a job runs on a worker.
pub(crate) type Work<O, P> = Box<dyn FnOnce(&JobContext<'_, O, P>) -> O + Send>;

/// A job to submit.
pub(crate) struct Job<O, P> {
    /// Which line it waits in.
    pub(crate) lane: Lane,
    /// For an interactive job, what it is for: a later job with the same key
    /// replaces it while it is still queued.
    pub(crate) key: Option<JobKey>,
    /// The token of the task it belongs to, if any.
    pub(crate) cancel: Option<CancelToken>,
    /// What it does.
    pub(crate) work: Work<O, P>,
}

/// The result of [`Pool::submit`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Submitted {
    /// The new job.
    pub(crate) id: JobId,
    /// A queued job with the same key that was dropped in its favour; it
    /// will never post an event.
    pub(crate) replaced: Option<JobId>,
}

/// What a running job can do besides compute.
pub(crate) struct JobContext<'a, O, P> {
    events: &'a Sender<Event<O, P>>,
    shared: &'a Shared<O, P>,
    cancel: Option<&'a CancelToken>,
}

impl<O, P> JobContext<'_, O, P> {
    /// Post progress. It reaches the engine before the job's outcome.
    pub(crate) fn progress(&self, progress: P) {
        if self.events.send(Event::Progress(progress)).is_ok() {
            self.shared.wake();
        }
    }

    /// True once the job's task has been cancelled or the pool is shutting
    /// down. A long job checks this between steps and returns early.
    pub(crate) fn should_stop(&self) -> bool {
        self.shared.stopping.load(Ordering::SeqCst)
            || self.cancel.is_some_and(CancelToken::is_cancelled)
    }
}

struct Queued<O, P> {
    id: JobId,
    cancel: Option<CancelToken>,
    work: Work<O, P>,
}

struct State<O, P> {
    queue: Queue<Queued<O, P>, JobKey>,
    shutdown: bool,
}

struct Shared<O, P> {
    state: Mutex<State<O, P>>,
    available: Condvar,
    stopping: AtomicBool,
    wake: Mutex<Option<Wake>>,
}

impl<O, P> Shared<O, P> {
    /// Block until there is a job to run, or return `None` at shutdown.
    fn next(&self) -> Option<Queued<O, P>> {
        let mut state = lock(&self.state);
        loop {
            if state.shutdown {
                return None;
            }
            if let Some(job) = state.queue.pop() {
                return Some(job);
            }
            state = self
                .available
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn wake(&self) {
        let wake = lock(&self.wake).clone();
        if let Some(wake) = wake
            && catch_unwind(AssertUnwindSafe(|| wake())).is_err()
        {
            log::error!("the engine's wake callback panicked");
        }
    }

    fn run(&self, job: Queued<O, P>, events: &Sender<Event<O, P>>) -> Outcome<O> {
        let Queued { cancel, work, .. } = job;
        let cancelled = cancel.as_ref().is_some_and(CancelToken::is_cancelled);
        if cancelled || self.stopping.load(Ordering::SeqCst) {
            return Outcome::Skipped;
        }
        let context = JobContext {
            events,
            shared: self,
            cancel: cancel.as_ref(),
        };
        match catch_unwind(AssertUnwindSafe(|| work(&context))) {
            Ok(output) => Outcome::Finished(output),
            Err(payload) => Outcome::Panicked(panic_message(payload.as_ref())),
        }
    }
}

/// A fixed set of worker threads sharing one queue.
pub(crate) struct Pool<O, P> {
    shared: Arc<Shared<O, P>>,
    threads: Vec<JoinHandle<()>>,
    next_id: u64,
}

impl<O: Send + 'static, P: Send + 'static> Pool<O, P> {
    /// Start `threads` workers (at least one) that post to `events`.
    pub(crate) fn start(threads: usize, events: &Sender<Event<O, P>>) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: Queue::new(),
                shutdown: false,
            }),
            available: Condvar::new(),
            stopping: AtomicBool::new(false),
            wake: Mutex::new(None),
        });
        let mut pool = Self {
            shared,
            threads: Vec::new(),
            next_id: 0,
        };
        for index in 0..threads.max(1) {
            let shared = Arc::clone(&pool.shared);
            let events = events.clone();
            // On failure `pool` is dropped here, which stops and joins the
            // workers already started.
            let handle = thread::Builder::new()
                .name(format!("photo-engine-{index}"))
                .spawn(move || work_loop(&shared, &events))?;
            pool.threads.push(handle);
        }
        Ok(pool)
    }

    /// Queue a job.
    pub(crate) fn submit(&mut self, job: Job<O, P>) -> Submitted {
        self.next_id += 1;
        let id = JobId(self.next_id);
        let queued = Queued {
            id,
            cancel: job.cancel,
            work: job.work,
        };
        let replaced = {
            let mut state = lock(&self.shared.state);
            match job.lane {
                Lane::Urgent => {
                    state.queue.push_urgent(queued);
                    None
                }
                Lane::Interactive => state.queue.push_interactive(job.key, queued),
                Lane::Background => {
                    state.queue.push_background(queued);
                    None
                }
            }
        };
        self.shared.available.notify_one();
        Submitted {
            id,
            replaced: replaced.map(|job| job.id),
        }
    }

    /// Move the queued interactive job with this key to the front of its
    /// lane. Returns whether one was queued.
    pub(crate) fn promote(&self, key: &JobKey) -> bool {
        lock(&self.shared.state).queue.promote(key)
    }

    /// How many jobs are waiting for a worker.
    pub(crate) fn queued(&self) -> usize {
        lock(&self.shared.state).queue.len()
    }

    /// Replace the wake callback.
    pub(crate) fn set_wake(&self, wake: Option<Wake>) {
        *lock(&self.shared.wake) = wake;
    }
}

impl<O, P> Drop for Pool<O, P> {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let abandoned = {
            let mut state = lock(&self.shared.state);
            state.shutdown = true;
            state.queue.drain()
        };
        drop(abandoned);
        self.shared.available.notify_all();
        for handle in self.threads.drain(..) {
            if handle.join().is_err() {
                log::error!("an engine worker thread ended with a panic");
            }
        }
    }
}

fn work_loop<O, P>(shared: &Shared<O, P>, events: &Sender<Event<O, P>>) {
    while let Some(job) = shared.next() {
        let id = job.id;
        let outcome = shared.run(job, events);
        if events.send(Event::Done { job: id, outcome }).is_err() {
            return;
        }
        shared.wake();
    }
}

/// Lock a mutex, carrying on with the data if a panic poisoned it: nothing
/// here leaves shared state half-changed across a panic.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "no further detail".to_string()
    }
}

#[cfg(test)]
mod tests;
