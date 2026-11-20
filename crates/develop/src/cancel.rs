//! Cancelling a render that is no longer wanted.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A flag shared between whoever asked for a render and the renderer doing
/// it. Once [`Self::cancel`] has been called, every render the token (or a
/// clone of it) was passed to stops at its next check and returns `None`;
/// the renderer checks between stages and between row batches of its
/// expensive stages, so a stale render is abandoned within milliseconds.
///
/// Cloning shares the flag. A cancelled token stays cancelled: make a new
/// one for the next render.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask every render holding this token (or a clone of it) to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// True once [`Self::cancel`] has been called on this token or a clone.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Some(())` while the render may go on, `None` once cancelled, so a
    /// stage can bail out with `?`.
    pub(crate) fn proceed(&self) -> Option<()> {
        (!self.is_cancelled()).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled() && token.proceed().is_some());
        token.cancel();
        assert!(clone.is_cancelled() && clone.proceed().is_none());
    }
}
