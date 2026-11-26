//! Pacing the draft edits of a control being dragged.
//!
//! The engine's render worker cancels the render it is running whenever a
//! newer one is asked for. A drag changes its value every frame, so asking
//! for every value would cancel every render and leave the canvas still
//! until the hand stops. Instead one draft is in flight at a time: a value
//! waits here (newer values of the same control replacing it), and at the
//! end of each frame it goes to the engine if the frame of the draft before
//! has arrived, or if nothing is being rendered that could bring it (the
//! file is still loading, or a render failed). The panels show the waiting
//! value meanwhile, so the control never lags the hand, and letting go
//! sends what waits before the commit.

use photo_core::develop::{Adj, DevelopSettings};
use photo_core::meta::FileKind;

use super::actions::DevelopAction;
use crate::app::Cx;
use crate::app::actions::Action;

/// A draft edit waiting for its turn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pending {
    /// One slider's value.
    Adjust(Adj, f32),
    /// Several values at once (a curve, a wheel).
    Settings(Box<DevelopSettings>),
    /// What a tool made of the settings (a crop handle, a spot, a mask):
    /// part of the tool's own step, committed when the tool is.
    Tool(Box<DevelopSettings>),
}

impl Pending {
    /// The request that hands the edit to the engine.
    pub(crate) fn into_action(self) -> DevelopAction {
        match self {
            Self::Adjust(adj, value) => DevelopAction::Adjust(adj, value),
            Self::Settings(settings) => DevelopAction::Edit(settings),
            Self::Tool(settings) => DevelopAction::ToolEdit(settings),
        }
    }

    /// Whether a tool made the edit.
    pub(crate) fn is_tool(&self) -> bool {
        matches!(self, Self::Tool(_))
    }

    /// Whether `self` replaces `other`: the same slider, or settings for
    /// settings. Anything else must not be lost, so goes first.
    fn replaces(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Adjust(adj, _), Self::Adjust(other, _)) => adj == other,
            (Self::Settings(_), Self::Settings(_)) | (Self::Tool(_), Self::Tool(_)) => true,
            _ => false,
        }
    }
}

/// `settings` as they will be once `pending` reaches the engine.
pub(crate) fn overlay(settings: &mut DevelopSettings, kind: FileKind, pending: Option<&Pending>) {
    match pending {
        Some(Pending::Adjust(adj, value)) => adj.set(settings, adj.spec_for(kind).clamp(*value)),
        Some(Pending::Settings(waiting) | Pending::Tool(waiting)) => settings.clone_from(waiting),
        None => {}
    }
}

/// Whether the next draft may go: the frame of the open photo's newest
/// settings is on hand, or nothing is being rendered that could bring it.
fn turn(cx: &Cx<'_>) -> bool {
    let Some(session) = cx.engine.develop() else {
        return true;
    };
    let current = session
        .frame()
        .is_some_and(|frame| frame.version() == session.version());
    current || cx.engine.pending_renders() == 0
}

/// A control moved to `edit`: it waits for the end of the frame, replacing
/// a waiting value of the same control. A waiting value of another control
/// goes at once, so that it is not lost.
pub(crate) fn draft(cx: &mut Cx<'_>, edit: Pending) {
    if let Some(waiting) = &cx.ui.develop.pending
        && !edit.replaces(waiting)
    {
        flush(cx);
    }
    cx.ui.develop.pending = Some(edit);
}

/// Send what waits, if its turn has come. Call once a frame, after every
/// control has drawn; the frame a draft waits for wakes the window when it
/// arrives.
pub(crate) fn pace(cx: &mut Cx<'_>) {
    if cx.ui.develop.pending.is_some() && turn(cx) {
        flush(cx);
    }
}

/// Send what waits now: the control was let go, or another moved.
pub(crate) fn flush(cx: &mut Cx<'_>) {
    if let Some(edit) = cx.ui.develop.pending.take() {
        cx.act(Action::Develop(edit.into_action()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_replaces_only_a_waiting_value_of_the_same_control() {
        let exposure = Pending::Adjust(Adj::Exposure, 1.0);
        assert!(Pending::Adjust(Adj::Exposure, 2.0).replaces(&exposure));
        assert!(!Pending::Adjust(Adj::Contrast, 2.0).replaces(&exposure));
        let curve = Pending::Settings(Box::default());
        assert!(Pending::Settings(Box::default()).replaces(&curve));
        assert!(!exposure.replaces(&curve));
        let crop = Pending::Tool(Box::default());
        assert!(Pending::Tool(Box::default()).replaces(&crop));
        assert!(
            !curve.replaces(&crop) && !crop.replaces(&curve),
            "a panel's edit and a tool's are different steps"
        );
        assert!(crop.is_tool() && !curve.is_tool());
    }

    #[test]
    fn the_overlay_shows_the_waiting_value_held_to_the_range() {
        let mut settings = DevelopSettings::default();
        overlay(
            &mut settings,
            FileKind::Raster,
            Some(&Pending::Adjust(Adj::Exposure, 9.0)),
        );
        assert_eq!(settings.tone.exposure, 5.0);
        let mut waiting = DevelopSettings::default();
        waiting.presence.clarity = 12.0;
        overlay(
            &mut settings,
            FileKind::Raster,
            Some(&Pending::Settings(Box::new(waiting.clone()))),
        );
        assert_eq!(settings, waiting);
    }
}
