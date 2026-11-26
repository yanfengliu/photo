//! The engine's last moments.
//!
//! Dropping the engine stops every render first (exports leave no file
//! behind). Then an edit still open in Develop is committed, as closing the
//! photo commits it: nothing in the application has a save step, so
//! quitting keeps the work. That comes before anything that can wait: a
//! worker stuck on a drive that has stalled can keep the workers from
//! stopping for as long as the drive stalls, and the edit is in the catalog
//! by then. Then the workers are stopped and waited for, what they had
//! finished is brought in as a last poll would, and every file an import
//! has read and copied gets its record, so that closing during an import
//! leaves no copy the catalog does not know.
//!
//! Closing does not wait long for the catalog. [`CLOSING_PATIENCE`] is one
//! budget for all of closing's catalog writes together: each waits for a
//! catalog another connection has locked only for what is left of it, and
//! once it is spent no write is tried at all (the catalog refuses them; see
//! `Catalog::set_deadline`). Waiting for the workers to stop is not waiting
//! for the catalog, and spends none of it. A catalog that cannot be written
//! is not asked again photo by photo. An edit that could not be saved is
//! named in the log by its photo's path. What could not be recorded of an
//! import is left where it is and named in the log, file by file: the next
//! import of its folder finds it.
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
        if std::thread::panicking() {
            self.pool.shut_down();
            return;
        }
        self.imports.closing = true;
        let started = Instant::now();
        self.give_the_catalog_until(started + self.closing_patience);
        self.keep_open_edit();
        let left = self.closing_patience.saturating_sub(started.elapsed());
        self.pool.shut_down();
        self.give_the_catalog_until(Instant::now() + left);
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
        self.record_ready_imports();
    }
}

impl Engine {
    /// Commit the edit still open in Develop, if there is one: one catalog
    /// write. An edit the catalog could not take is named in the log by
    /// its photo's path, and closing goes on.
    fn keep_open_edit(&mut self) {
        let editing = self
            .develop
            .as_ref()
            .filter(|session| session.is_editing())
            .map(|session| session.path.clone());
        let Some(path) = editing else {
            return;
        };
        if let Err(error) = self.commit_edit() {
            log::error!(
                "the edit of {} still open as the application closed could not be saved: {error}",
                path.display()
            );
        }
    }

    /// Let every catalog write from now on wait for a locked catalog only
    /// until `deadline`, and none be tried after it.
    fn give_the_catalog_until(&mut self, deadline: Instant) {
        if let Err(error) = self.catalog.set_deadline(Some(deadline)) {
            log::warn!("cannot limit the wait for the catalog: {error}");
        }
    }
}
