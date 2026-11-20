//! The brief confirmations the UI shows after an action ("Set Rating to 3").

use std::collections::VecDeque;

/// How many toasts are kept when nobody drains them.
const LIMIT: usize = 64;

/// Whether a toast confirms something or reports a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToastKind {
    /// An action happened.
    Info,
    /// Something failed; the message says what.
    Error,
}

/// One message for the UI to show briefly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// What happened, as a short sentence.
    pub message: String,
    /// Whether it is a confirmation or a failure.
    pub kind: ToastKind,
}

/// Toasts waiting to be drained, oldest first. Bounded: when nobody drains
/// them, the oldest are dropped.
#[derive(Debug, Default)]
pub(crate) struct Toasts {
    queue: VecDeque<Toast>,
}

impl Toasts {
    pub(crate) fn info(&mut self, message: impl Into<String>) {
        self.push(message.into(), ToastKind::Info);
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.push(message.into(), ToastKind::Error);
    }

    fn push(&mut self, message: String, kind: ToastKind) {
        if self.queue.len() == LIMIT {
            self.queue.pop_front();
        }
        self.queue.push_back(Toast { message, kind });
    }

    pub(crate) fn drain(&mut self) -> Vec<Toast> {
        self.queue.drain(..).collect()
    }

    pub(crate) fn pending(&self) -> impl Iterator<Item = &Toast> {
        self.queue.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toasts_drain_in_order_and_the_queue_is_bounded() {
        let mut toasts = Toasts::default();
        for index in 0..LIMIT + 3 {
            toasts.info(format!("toast {index}"));
        }
        toasts.error("failed");
        let drained = toasts.drain();
        assert_eq!(drained.len(), LIMIT);
        assert_eq!(drained[0].message, "toast 4");
        assert_eq!(drained.last().map(|t| t.kind), Some(ToastKind::Error));
        assert!(toasts.drain().is_empty());
    }
}
