//! The engine's last moments.
//!
//! Dropping the engine stops every render first (exports leave no file
//! behind), then the workers. What they had finished is brought in as a
//! last poll would, and every file an import has read and copied gets its
//! record, so that closing during an import leaves no copy the catalog
//! does not know.
//!
//! Closing does not wait long for the catalog. One that another connection
//! has locked is waited for no longer than [`CLOSING_PATIENCE`] in all, and
//! one that cannot be written is not asked again photo by photo. What could
//! not be recorded is left where it is and named in the log: the next
//! import of its folder finds it.

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
        while let Ok(event) = self.events.try_recv() {
            self.wait_for_catalog_until(deadline);
            self.handle(event);
        }
        self.record_ready_imports();
    }
}

impl Engine {
    /// Let a change wait for a locked catalog until `deadline` and no
    /// longer.
    pub(crate) fn wait_for_catalog_until(&self, deadline: Instant) {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.catalog.set_busy_timeout(left) {
            log::warn!("cannot shorten the wait for the catalog: {error}");
        }
    }
}
