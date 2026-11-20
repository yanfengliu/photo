//! Background tasks as the UI's activity indicator lists them.

use std::collections::HashMap;
use std::fmt;

use crate::jobs::CancelToken;

/// How many finished tasks are kept for the activity list.
const FINISHED_KEPT: usize = 16;

/// Identifies one background task for the life of the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What a task does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskKind {
    /// Listing a folder's importable files.
    Scan,
    /// Importing files into the catalog.
    Import,
    /// Writing XMP sidecars.
    SaveMetadata,
    /// Reading XMP sidecars back into the catalog.
    ReadMetadata,
}

/// Where a task is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// Still working (a cancelled task stays here until its jobs have
    /// stopped).
    Running,
    /// Done; per-item failures are counted in [`TaskInfo::errors`].
    Finished,
    /// Stopped at the user's request; what it had completed is kept.
    Cancelled,
    /// Gave up as a whole, for the reason given.
    Failed(String),
}

impl TaskState {
    /// True for every state but [`TaskState::Running`].
    pub fn is_over(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// One task, as the activity indicator shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskInfo {
    /// The task's id.
    pub id: TaskId,
    /// What it does.
    pub kind: TaskKind,
    /// A short description, such as "Importing 120 photos".
    pub label: String,
    /// Items completed so far (succeeded, skipped or failed).
    pub done: u64,
    /// Items in all; can grow while a scan discovers files.
    pub total: u64,
    /// Where it is.
    pub state: TaskState,
    /// Items that failed so far.
    pub errors: u64,
}

/// Running tasks and the most recently finished ones, in start order.
#[derive(Debug, Default)]
pub(crate) struct Tasks {
    list: Vec<TaskInfo>,
    tokens: HashMap<TaskId, CancelToken>,
    next: u64,
}

impl Tasks {
    /// Register a running task and return its id and cancel token.
    pub(crate) fn start(
        &mut self,
        kind: TaskKind,
        label: impl Into<String>,
        total: u64,
    ) -> (TaskId, CancelToken) {
        self.next += 1;
        let id = TaskId(self.next);
        let token = CancelToken::default();
        self.tokens.insert(id, token.clone());
        self.list.push(TaskInfo {
            id,
            kind,
            label: label.into(),
            done: 0,
            total,
            state: TaskState::Running,
            errors: 0,
        });
        (id, token)
    }

    pub(crate) fn list(&self) -> &[TaskInfo] {
        &self.list
    }

    pub(crate) fn get(&self, id: TaskId) -> Option<&TaskInfo> {
        self.list.iter().find(|task| task.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: TaskId) -> Option<&mut TaskInfo> {
        self.list.iter_mut().find(|task| task.id == id)
    }

    /// Ask a running task to stop. Returns false for an unknown or finished
    /// task.
    pub(crate) fn cancel(&mut self, id: TaskId) -> bool {
        let running = self.get(id).is_some_and(|task| !task.state.is_over());
        match self.tokens.get(&id) {
            Some(token) if running => {
                token.cancel();
                true
            }
            _ => false,
        }
    }

    /// The cancel token of a running task.
    pub(crate) fn token(&self, id: TaskId) -> Option<CancelToken> {
        self.tokens.get(&id).cloned()
    }

    /// True when the task was asked to stop.
    pub(crate) fn is_cancelled(&self, id: TaskId) -> bool {
        self.tokens.get(&id).is_some_and(CancelToken::is_cancelled)
    }

    /// Put a task in its final state and forget the oldest finished tasks
    /// beyond the number kept.
    pub(crate) fn finish(&mut self, id: TaskId, state: TaskState) {
        if let Some(task) = self.get_mut(id) {
            task.state = state;
        }
        self.tokens.remove(&id);
        let finished = self.list.iter().filter(|task| task.state.is_over()).count();
        let mut excess = finished.saturating_sub(FINISHED_KEPT);
        self.list.retain(|task| {
            let drop = excess > 0 && task.state.is_over();
            if drop {
                excess -= 1;
            }
            !drop
        });
    }

    /// How many tasks are still running.
    pub(crate) fn running(&self) -> usize {
        self.list
            .iter()
            .filter(|task| !task.state.is_over())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_flags_the_token_and_only_running_tasks_can_be_cancelled() {
        let mut tasks = Tasks::default();
        let (id, token) = tasks.start(TaskKind::Import, "Importing", 3);
        assert!(!tasks.is_cancelled(id));
        assert!(tasks.cancel(id));
        assert!(token.is_cancelled() && tasks.is_cancelled(id));
        tasks.finish(id, TaskState::Cancelled);
        assert!(!tasks.cancel(id));
        assert_eq!(
            tasks.get(id).map(|task| &task.state),
            Some(&TaskState::Cancelled)
        );
        assert_eq!(tasks.running(), 0);
    }

    #[test]
    fn only_the_newest_finished_tasks_are_kept() {
        let mut tasks = Tasks::default();
        let (running, _) = tasks.start(TaskKind::Scan, "Scanning", 0);
        let ids: Vec<TaskId> = (0..FINISHED_KEPT + 4)
            .map(|_| tasks.start(TaskKind::SaveMetadata, "Saving", 1).0)
            .collect();
        for id in &ids {
            tasks.finish(*id, TaskState::Finished);
        }
        assert_eq!(tasks.list().len(), FINISHED_KEPT + 1);
        assert_eq!(tasks.list()[0].id, running);
        assert!(tasks.get(ids[3]).is_none());
        assert!(tasks.get(ids[4]).is_some());
    }
}
