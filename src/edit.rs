//! Image editing state and undo/redo history.
//! All adjustment math lives here — both the data model and CPU-side
//! processing for full-resolution save.

// -- Data model --

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EditState {
    pub exposure: f32,    // -5.0 to +5.0 (stops)
    pub contrast: f32,    // -100 to +100
    pub highlights: f32,  // -100 to +100
    pub shadows: f32,     // -100 to +100
    pub whites: f32,      // -100 to +100
    pub blacks: f32,      // -100 to +100
    pub temperature: f32, // -100 to +100
    pub tint: f32,        // -100 to +100
    pub vibrance: f32,    // -100 to +100
    pub saturation: f32,  // -100 to +100
    pub clarity: f32,     // -100 to +100
    pub dehaze: f32,      // -100 to +100
    pub lens_correction: bool,
}

impl EditState {
    /// Returns true if all adjustments are at their defaults (no edits).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug)]
pub struct UndoHistory {
    undo_stack: Vec<EditState>,
    redo_stack: Vec<EditState>,
    /// The last committed (stable) state. On commit(), this is pushed to the
    /// undo stack and then updated to `current`.
    committed: EditState,
    pub current: EditState,
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            committed: EditState::default(),
            current: EditState::default(),
        }
    }

    /// Call when a slider drag ends. Pushes the pre-edit (committed) state to
    /// the undo stack and marks the current state as the new committed baseline.
    pub fn commit(&mut self) {
        self.undo_stack.push(self.committed);
        self.committed = self.current;
        self.redo_stack.clear();
    }

    /// Undo: restore previous state. Returns true if undo was performed.
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.committed);
            self.committed = prev;
            self.current = prev;
            true
        } else {
            false
        }
    }

    /// Redo: restore next state. Returns true if redo was performed.
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.committed);
            self.committed = next;
            self.current = next;
            true
        } else {
            false
        }
    }

    /// Reset all adjustments to default. This is an undoable action.
    pub fn reset_all(&mut self) {
        self.undo_stack.push(self.committed);
        self.redo_stack.clear();
        self.committed = EditState::default();
        self.current = EditState::default();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_edit_state_is_zeroed() {
        let s = EditState::default();
        assert_eq!(s.exposure, 0.0);
        assert_eq!(s.contrast, 0.0);
        assert_eq!(s.highlights, 0.0);
        assert_eq!(s.shadows, 0.0);
        assert_eq!(s.whites, 0.0);
        assert_eq!(s.blacks, 0.0);
        assert_eq!(s.temperature, 0.0);
        assert_eq!(s.tint, 0.0);
        assert_eq!(s.vibrance, 0.0);
        assert_eq!(s.saturation, 0.0);
        assert_eq!(s.clarity, 0.0);
        assert_eq!(s.dehaze, 0.0);
        assert!(!s.lens_correction);
        assert!(s.is_default());
    }

    #[test]
    fn is_default_false_when_modified() {
        let mut s = EditState::default();
        s.exposure = 1.0;
        assert!(!s.is_default());
    }

    #[test]
    fn undo_redo_basic_flow() {
        let mut h = UndoHistory::new();
        assert!(!h.can_undo());
        assert!(!h.can_redo());

        // Make an edit
        h.current.exposure = 1.5;
        h.commit();
        assert!(h.can_undo());
        assert!(!h.can_redo());

        // Make another edit
        h.current.contrast = 50.0;
        h.commit();

        // Undo once — should restore exposure=1.5, contrast=0
        assert!(h.undo());
        assert_eq!(h.current.exposure, 1.5);
        assert_eq!(h.current.contrast, 0.0);
        assert!(h.can_redo());

        // Redo — should restore contrast=50
        assert!(h.redo());
        assert_eq!(h.current.contrast, 50.0);
    }

    #[test]
    fn undo_on_empty_returns_false() {
        let mut h = UndoHistory::new();
        assert!(!h.undo());
    }

    #[test]
    fn redo_on_empty_returns_false() {
        let mut h = UndoHistory::new();
        assert!(!h.redo());
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut h = UndoHistory::new();
        h.current.exposure = 1.0;
        h.commit();
        h.current.exposure = 2.0;
        h.commit();

        // Undo
        h.undo();
        assert!(h.can_redo());

        // New edit should clear redo
        h.current.exposure = 3.0;
        h.commit();
        assert!(!h.can_redo());
    }

    #[test]
    fn reset_all_is_undoable() {
        let mut h = UndoHistory::new();
        h.current.exposure = 2.5;
        h.current.contrast = -30.0;
        h.commit();

        h.reset_all();
        assert!(h.current.is_default());
        assert!(h.can_undo());

        // Undo the reset
        h.undo();
        assert_eq!(h.current.exposure, 2.5);
        assert_eq!(h.current.contrast, -30.0);
    }
}
