//! The engine's last moments.
//!
//! Dropping the engine stops every render first (exports leave no file
//! behind), then the workers. An edit still open in Develop is committed,
//! as closing the photo commits it: nothing in the application has a save
//! step, so quitting keeps the work. Then what the workers had finished is
//! brought in as a last poll would, and every file an import has read and
//! copied gets its record, so that closing during an import leaves no copy
//! the catalog does not know.
//!
//! Closing does not wait long for the catalog. One that another connection
//! has locked is waited for no longer than [`CLOSING_PATIENCE`] in all, and
//! one that cannot be written is not asked again photo by photo. An edit
//! that could not be saved is named in the log by its photo's path. What
//! could not be recorded of an import is left where it is and named in the
//! log: the next import of its folder finds it.
//!
//! None of this is done while a panic unwinds: the catalog is left as it
//! is.

use std::time::{Duration, Instant};

use super::Engine;

/// How long, in all, a closing engine waits for a catalog that another
/// connection has locked.
pub(super) const CLOSING_PATIENCE: Duration = Duration::from_secs(2);

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.exports.cancel_all();
        self.pool.shut_down();
        if std::thread::panicking() {
            return;
        }
        let deadline = Instant::now() + self.closing_patience;
        self.imports.closing = Some(deadline);
        self.keep_open_edit(deadline);
        while let Ok(event) = self.events.try_recv() {
            self.wait_for_catalog_until(deadline);
            self.handle(event);
        }
        self.record_ready_imports();
    }
}

impl Engine {
    /// Commit the edit still open in Develop, if there is one: one catalog
    /// write, which waits for a locked catalog until `deadline` and no
    /// longer. An edit the catalog could not take is named in the log by
    /// its photo's path, and closing goes on.
    fn keep_open_edit(&mut self, deadline: Instant) {
        let editing = self
            .develop
            .as_ref()
            .filter(|session| session.is_editing())
            .map(|session| session.path.clone());
        let Some(path) = editing else {
            return;
        };
        self.wait_for_catalog_until(deadline);
        if let Err(error) = self.commit_edit() {
            log::error!(
                "the edit of {} still open as the application closed could not be saved: {error}",
                path.display()
            );
        }
    }

    /// Let a change wait for a locked catalog until `deadline` and no
    /// longer.
    pub(crate) fn wait_for_catalog_until(&self, deadline: Instant) {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.catalog.set_busy_timeout(left) {
            log::warn!("cannot shorten the wait for the catalog: {error}");
        }
    }
}
