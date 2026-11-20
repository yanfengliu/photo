//! The Develop module's dialogs (design §8.4): Copy Settings and
//! Synchronize Settings (the settings-group checklist), New Preset (a name,
//! a group and the checklist), naming a snapshot, renaming a preset, and
//! the confirmations before a preset or snapshot is deleted, which cannot
//! be undone.

use egui::{Button, Checkbox, RichText, TextEdit, Ui};
use photo_core::develop::{GroupSet, SettingsGroup};
use photo_core::meta::{PhotoId, PresetId, SnapshotId};

use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::theme::{palette, space};
use crate::widgets::{Dialog, DialogOutcome};

/// The group a new preset goes into unless the user says otherwise.
pub(crate) const USER_GROUP: &str = "User Presets";

/// A Develop dialog.
pub(crate) enum DevelopDialog {
    Groups(GroupsDialog),
    Snapshot(SnapshotDialog),
    RenamePreset(PresetDialog),
    Delete(DeleteDialog),
}

impl DevelopDialog {
    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        match self {
            Self::Groups(dialog) => dialog.show(ctx, cx),
            Self::Snapshot(dialog) => dialog.show(ctx, cx),
            Self::RenamePreset(dialog) => dialog.show(ctx, cx),
            Self::Delete(dialog) => dialog.show(ctx, cx),
        }
    }
}

/// What a checklist dialog does with the groups ticked.
pub(crate) enum GroupsPurpose {
    Copy,
    Sync,
    NewPreset { name: String, group: String },
}

/// The settings-group checklist.
pub(crate) struct GroupsDialog {
    purpose: GroupsPurpose,
    groups: GroupSet,
    focus: bool,
}

impl GroupsDialog {
    pub(crate) fn new(purpose: GroupsPurpose, groups: GroupSet) -> Self {
        Self {
            purpose,
            groups,
            focus: true,
        }
    }

    fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let (title, confirm) = match self.purpose {
            GroupsPurpose::Copy => ("Copy Settings", "Copy"),
            GroupsPurpose::Sync => ("Synchronize Settings", "Synchronize"),
            GroupsPurpose::NewPreset { .. } => ("New Preset", "Create"),
        };
        let named = match &self.purpose {
            GroupsPurpose::NewPreset { name, .. } => !name.trim().is_empty(),
            _ => true,
        };
        let focus = std::mem::take(&mut self.focus);
        let response = Dialog::new(title)
            .confirm_label(confirm)
            .can_confirm(named && !self.groups.is_empty())
            .width(460.0)
            .show(ctx, |ui| {
                if let GroupsPurpose::NewPreset { name, group } = &mut self.purpose {
                    preset_fields(ui, name, group, focus);
                    ui.add_space(space::LG);
                    ui.label(RichText::new("Settings to include").color(palette::TEXT_DIM));
                } else if let GroupsPurpose::Sync = self.purpose {
                    ui.label(
                        RichText::new(
                            "Give the other selected photos these settings of the active one.",
                        )
                        .color(palette::TEXT_DIM),
                    );
                }
                ui.add_space(space::SM);
                checklist(ui, &mut self.groups);
            });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                let groups = self.groups;
                let action = match &self.purpose {
                    GroupsPurpose::Copy => {
                        cx.ui.develop.copy_groups = groups;
                        DevelopAction::CopySettings(groups)
                    }
                    GroupsPurpose::Sync => DevelopAction::SyncSettings(groups),
                    GroupsPurpose::NewPreset { name, group } => DevelopAction::CreatePreset {
                        group: group_or_default(group),
                        name: name.trim().to_owned(),
                        groups,
                    },
                };
                cx.act(Action::Develop(action));
                false
            }
        }
    }
}

/// The group typed, or the user group when nothing was.
fn group_or_default(group: &str) -> String {
    let group = group.trim();
    if group.is_empty() {
        USER_GROUP.to_owned()
    } else {
        group.to_owned()
    }
}

/// The name and group fields of a preset.
fn preset_fields(ui: &mut Ui, name: &mut String, group: &mut String, focus: bool) {
    ui.label(RichText::new("Name").color(palette::TEXT_DIM));
    let field = ui
        .add(
            TextEdit::singleline(name)
                .id_salt("preset-name")
                .desired_width(f32::INFINITY),
        )
        .accessible_name("Preset name");
    if focus {
        field.request_focus();
    }
    ui.add_space(space::MD);
    ui.label(RichText::new("Group").color(palette::TEXT_DIM));
    ui.add(
        TextEdit::singleline(group)
            .id_salt("preset-group")
            .hint_text(USER_GROUP)
            .desired_width(f32::INFINITY),
    )
    .accessible_name("Preset group");
}

/// Every settings group as a tick box in two columns, with Check All and
/// Check None above them.
fn checklist(ui: &mut Ui, groups: &mut GroupSet) {
    ui.horizontal(|ui| {
        if ui.add(Button::new("Check All")).clicked() {
            *groups = GroupSet::all();
        }
        if ui.add(Button::new("Check None")).clicked() {
            *groups = GroupSet::NONE;
        }
    });
    ui.add_space(space::SM);
    let half = SettingsGroup::ALL.len().div_ceil(2);
    ui.columns(2, |columns| {
        for (index, group) in SettingsGroup::ALL.into_iter().enumerate() {
            let column = &mut columns[usize::from(index >= half)];
            let mut ticked = groups.contains(group);
            if column
                .add(Checkbox::new(&mut ticked, group.label()))
                .changed()
            {
                groups.set(group, ticked);
            }
        }
    });
}

/// Naming a new snapshot of a photo, or a snapshot anew.
pub(crate) struct SnapshotDialog {
    photo: PhotoId,
    renaming: Option<SnapshotId>,
    name: String,
    /// The name a new snapshot gets when none is typed.
    suggested: String,
    focus: bool,
}

impl SnapshotDialog {
    /// Name a new snapshot of `photo`, which has `existing` snapshots.
    pub(crate) fn new(photo: PhotoId, existing: usize) -> Self {
        Self {
            photo,
            renaming: None,
            name: String::new(),
            suggested: format!("Snapshot {}", existing + 1),
            focus: true,
        }
    }

    /// Give the snapshot `id` of `photo` a new name.
    pub(crate) fn rename(photo: PhotoId, id: SnapshotId, name: &str) -> Self {
        Self {
            photo,
            renaming: Some(id),
            name: name.to_owned(),
            suggested: name.to_owned(),
            focus: true,
        }
    }

    fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let (title, confirm) = match self.renaming {
            None => ("New Snapshot", "Create"),
            Some(_) => ("Rename Snapshot", "Rename"),
        };
        let focus = std::mem::take(&mut self.focus);
        let response = Dialog::new(title).confirm_label(confirm).show(ctx, |ui| {
            ui.label(RichText::new("Name").color(palette::TEXT_DIM));
            let field = ui
                .add(
                    TextEdit::singleline(&mut self.name)
                        .id_salt("snapshot-name")
                        .hint_text(self.suggested.as_str())
                        .desired_width(f32::INFINITY),
                )
                .accessible_name("Snapshot name");
            if focus {
                field.request_focus();
            }
        });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                let typed = self.name.trim();
                let name = if typed.is_empty() {
                    self.suggested.clone()
                } else {
                    typed.to_owned()
                };
                cx.act(Action::Develop(match self.renaming {
                    None => DevelopAction::CreateSnapshot(self.photo, name),
                    Some(id) => DevelopAction::RenameSnapshot(id, name),
                }));
                false
            }
        }
    }
}

/// Renaming a preset: its name and group.
pub(crate) struct PresetDialog {
    id: PresetId,
    name: String,
    group: String,
    focus: bool,
}

impl PresetDialog {
    pub(crate) fn new(id: PresetId, group: &str, name: &str) -> Self {
        Self {
            id,
            name: name.to_owned(),
            group: group.to_owned(),
            focus: true,
        }
    }

    fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let focus = std::mem::take(&mut self.focus);
        let response = Dialog::new("Rename Preset")
            .confirm_label("Rename")
            .can_confirm(!self.name.trim().is_empty())
            .show(ctx, |ui| {
                preset_fields(ui, &mut self.name, &mut self.group, focus)
            });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                cx.act(Action::Develop(DevelopAction::RenamePreset {
                    id: self.id,
                    group: group_or_default(&self.group),
                    name: self.name.trim().to_owned(),
                }));
                false
            }
        }
    }
}

/// What a delete confirmation is about.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Doomed {
    Preset(PresetId),
    Snapshot(SnapshotId),
}

/// Asking before a preset or snapshot is deleted.
pub(crate) struct DeleteDialog {
    doomed: Doomed,
    name: String,
}

impl DeleteDialog {
    pub(crate) fn new(doomed: Doomed, name: &str) -> Self {
        Self {
            doomed,
            name: name.to_owned(),
        }
    }

    fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let (title, what) = match self.doomed {
            Doomed::Preset(_) => ("Delete Preset", "preset"),
            Doomed::Snapshot(_) => ("Delete Snapshot", "snapshot"),
        };
        let response = Dialog::new(title).confirm_label("Delete").show(ctx, |ui| {
            let question = format!("Delete the {what} \"{}\"?", self.name);
            ui.label(RichText::new(question).color(palette::TEXT_STRONG));
            ui.add_space(space::SM);
            ui.label(
                RichText::new("Photos keep their settings. This cannot be undone.")
                    .color(palette::TEXT_DIM),
            );
        });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                cx.act(Action::Develop(match self.doomed {
                    Doomed::Preset(id) => DevelopAction::DeletePreset(id),
                    Doomed::Snapshot(id) => DevelopAction::DeleteSnapshot(id),
                }));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_without_a_group_goes_into_the_user_group() {
        assert_eq!(group_or_default("  "), USER_GROUP);
        assert_eq!(group_or_default(" Travel "), "Travel");
    }

    #[test]
    fn a_new_snapshot_is_offered_the_next_number_as_its_name() {
        let dialog = SnapshotDialog::new(PhotoId(7), 2);
        assert_eq!(dialog.suggested, "Snapshot 3");
        assert!(dialog.name.is_empty());
    }
}
