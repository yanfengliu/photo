//! What the user asked of the Develop module and of Quick Develop, as
//! values: each is one engine call (see [`DevelopAction::apply`]).

use photo_core::develop::{Adj, DevelopSettings, GroupSet, WbPreset};
use photo_core::geom::Point;
use photo_core::meta::{HistoryId, PhotoId, PresetId, SnapshotId};
use photo_engine::{BeforeSettings, CanvasView, Engine, EngineError, PresetKey, Target};

/// One Develop request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DevelopAction {
    Open(PhotoId),
    Close,
    SetCanvasView(CanvasView),
    /// One slider moved: a draft, not yet committed.
    Adjust(Adj, f32),
    /// Several values moved together (a curve, a wheel): a draft.
    Edit(Box<DevelopSettings>),
    /// The control was let go: record what changed as one step.
    Commit,
    /// A discrete change, committed at once.
    Apply(Box<DevelopSettings>),
    AutoTone,
    AutoWhiteBalance,
    PickWhiteBalance(Point),
    SetBefore(Option<BeforeSettings>),
    PreviewPreset(Option<PresetKey>, u32, u32),
    ApplyPreset(PresetKey, Target),
    CreatePreset {
        group: String,
        name: String,
        groups: GroupSet,
    },
    RenamePreset {
        id: PresetId,
        group: String,
        name: String,
    },
    DeletePreset(PresetId),
    CreateSnapshot(PhotoId, String),
    RenameSnapshot(SnapshotId, String),
    DeleteSnapshot(SnapshotId),
    ApplySnapshot(PhotoId, SnapshotId),
    GoToHistoryStep(PhotoId, HistoryId),
    CopySettings(GroupSet),
    PasteSettings(Target),
    SyncSettings(GroupSet),
    PasteFromPrevious(Target),
    ResetSettings(Target),
    QuickAdjust(Target, Adj, f32),
    QuickWhiteBalance(Target, WbPreset),
    QuickAutoTone(Target),
}

impl DevelopAction {
    /// Hand the request to the engine.
    pub(crate) fn apply(self, engine: &mut Engine) -> Result<(), EngineError> {
        match self {
            Self::Open(photo) => engine.open_develop(photo)?,
            Self::Close => engine.close_develop(),
            Self::SetCanvasView(view) => engine.set_canvas_view(view),
            Self::Adjust(adj, value) => engine.adjust(adj, value)?,
            Self::Edit(settings) => engine.edit_settings(*settings)?,
            Self::Commit => {
                engine.commit_edit()?;
            }
            Self::Apply(settings) => engine.apply_settings(*settings)?,
            Self::AutoTone => engine.auto_tone()?,
            Self::AutoWhiteBalance => engine.auto_white_balance()?,
            Self::PickWhiteBalance(at) => {
                if !engine.pick_white_balance(at)? {
                    return Err(EngineError::NotEditable(
                        "there is no colour to neutralise there".to_owned(),
                    ));
                }
            }
            Self::SetBefore(choice) => engine.set_before(choice)?,
            Self::PreviewPreset(preset, width, height) => {
                engine.preview_preset(preset, width, height)?;
            }
            Self::ApplyPreset(preset, target) => engine.apply_preset(&preset, target)?,
            Self::CreatePreset {
                group,
                name,
                groups,
            } => {
                engine.create_preset(&group, &name, groups)?;
            }
            Self::RenamePreset { id, group, name } => engine.rename_preset(id, &group, &name)?,
            Self::DeletePreset(id) => engine.delete_preset(id)?,
            Self::CreateSnapshot(photo, name) => {
                engine.create_snapshot(photo, &name)?;
            }
            Self::RenameSnapshot(id, name) => engine.rename_snapshot(id, &name)?,
            Self::DeleteSnapshot(id) => engine.delete_snapshot(id)?,
            Self::ApplySnapshot(photo, id) => engine.apply_snapshot(photo, id)?,
            Self::GoToHistoryStep(photo, step) => engine.go_to_history_step(photo, step)?,
            Self::CopySettings(groups) => engine.copy_settings(groups)?,
            Self::PasteSettings(target) => engine.paste_settings(target)?,
            Self::SyncSettings(groups) => engine.sync_settings(groups)?,
            Self::PasteFromPrevious(target) => engine.paste_from_previous(target)?,
            Self::ResetSettings(target) => engine.reset_settings(target)?,
            Self::QuickAdjust(target, adj, delta) => engine.quick_adjust(target, adj, delta)?,
            Self::QuickWhiteBalance(target, preset) => {
                engine.quick_white_balance(target, preset)?;
            }
            Self::QuickAutoTone(target) => engine.quick_auto_tone(target)?,
        }
        Ok(())
    }

    /// What to tell the user when the engine refuses, before its reason.
    pub(crate) fn failure(&self) -> &'static str {
        match self {
            Self::Open(_) => "Could not open the photo in Develop",
            Self::Close | Self::SetCanvasView(_) | Self::SetBefore(_) => "Could not do that",
            Self::Adjust(..) | Self::Edit(_) | Self::Commit | Self::Apply(_) => {
                "Could not change the photo"
            }
            Self::AutoTone | Self::QuickAutoTone(_) => "Could not set the tone automatically",
            Self::AutoWhiteBalance => "Could not set the white balance automatically",
            Self::PickWhiteBalance(_) => "Could not pick the white balance",
            Self::PreviewPreset(..) => "Could not preview the preset",
            Self::ApplyPreset(..) => "Could not apply the preset",
            Self::CreatePreset { .. } => "Could not create the preset",
            Self::RenamePreset { .. } => "Could not rename the preset",
            Self::DeletePreset(_) => "Could not delete the preset",
            Self::CreateSnapshot(..) => "Could not create the snapshot",
            Self::RenameSnapshot(..) => "Could not rename the snapshot",
            Self::DeleteSnapshot(_) => "Could not delete the snapshot",
            Self::ApplySnapshot(..) => "Could not apply the snapshot",
            Self::GoToHistoryStep(..) => "Could not go to that step",
            Self::CopySettings(_) => "Could not copy the settings",
            Self::PasteSettings(_) => "Could not paste the settings",
            Self::SyncSettings(_) => "Could not synchronize the settings",
            Self::PasteFromPrevious(_) => "Could not take the previous photo's settings",
            Self::ResetSettings(_) => "Could not reset the settings",
            Self::QuickAdjust(..) | Self::QuickWhiteBalance(..) => "Could not change the photos",
        }
    }
}
