//! The Presets panel (the built-in looks and the user's presets, by group:
//! a click applies one, the pointer on one previews it in the Navigator,
//! the menu renames and deletes) and the Snapshots panel (a click gives the
//! photo a snapshot's settings; the menu renames, deletes, or shows it as
//! the before view).

use std::collections::HashMap;

use egui::{RichText, Ui};
use photo_engine::{BUILT_IN_GROUP, BeforeSettings, PresetInfo, PresetKey, Target};

use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::app::develop::right::Editing;
use crate::app::develop::state::BeforeAfter;
use crate::app::{Cx, dialogs};
use crate::theme::{palette, space};
use crate::widgets::{Icon, IconButton, ListRow};

/// The accessible name of each preset's row: "Preset {name}", with the
/// group after it where two groups have a preset of that name.
fn row_names(presets: &[PresetInfo]) -> Vec<String> {
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for preset in presets {
        *uses.entry(preset.name.as_str()).or_default() += 1;
    }
    presets
        .iter()
        .map(|preset| {
            if uses[preset.name.as_str()] > 1 {
                format!("Preset {} ({})", preset.name, preset.group)
            } else {
                format!("Preset {}", preset.name)
            }
        })
        .collect()
}

/// The "+" button of a panel, with a word beside it.
fn add_button(ui: &mut Ui, name: &str, enabled: bool) -> bool {
    ui.horizontal(|ui| {
        let clicked = ui
            .add_enabled(enabled, IconButton::new(Icon::Plus, name).side(20.0))
            .clicked();
        ui.label(RichText::new("New").color(palette::TEXT_DIM));
        clicked
    })
    .inner
}

/// The Presets panel. Returns the preset under the pointer.
pub(super) fn presets(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: Option<&Editing>,
) -> Option<PresetKey> {
    let editable = editing.is_some_and(Editing::editable);
    if add_button(ui, "New preset", editable) {
        dialogs::open_new_preset(cx);
    }
    ui.add_space(space::XS);
    let presets = cx.ui.develop.lists.presets.clone();
    let names = row_names(&presets);
    let mut groups: Vec<&str> = Vec::new();
    for preset in &presets {
        if !groups.contains(&preset.group.as_str()) {
            groups.push(&preset.group);
        }
    }
    let mut hovered = None;
    for group in groups {
        let open = !cx.ui.develop.closed_groups.contains(group);
        let header = ListRow::new(&format!("Preset group {group}"), group)
            .icon(Icon::Folder)
            .expanded(open)
            .show(ui);
        if header.toggled || header.clicked {
            if open {
                cx.ui.develop.closed_groups.insert(group.to_owned());
            } else {
                cx.ui.develop.closed_groups.remove(group);
            }
        }
        if !open {
            continue;
        }
        for (preset, name) in presets
            .iter()
            .zip(&names)
            .filter(|(preset, _)| preset.group == group)
        {
            let row = ui
                .add_enabled_ui(editable, |ui| {
                    ListRow::new(name, &preset.name).depth(1).show(ui)
                })
                .inner;
            if row.response.hovered() {
                hovered = Some(preset.key);
            }
            if row.clicked {
                cx.act(Action::Develop(DevelopAction::ApplyPreset(
                    preset.key,
                    Target::Active,
                )));
            }
            if let PresetKey::User(id) = preset.key {
                egui::Popup::context_menu(&row.response).show(|ui| {
                    if ui.button("Rename…").clicked() {
                        dialogs::open_rename_preset(cx, id, &preset.group, &preset.name);
                    }
                    if ui.button("Delete…").clicked() {
                        dialogs::open_delete_preset(cx, id, &preset.name);
                    }
                });
            }
        }
    }
    if presets.iter().all(|preset| preset.group == BUILT_IN_GROUP) {
        ui.add_space(space::XS);
        ui.label(
            RichText::new("Save a look of your own with the + button.")
                .color(palette::TEXT_DIM)
                .small(),
        );
    }
    hovered
}

/// The Snapshots panel.
pub(super) fn snapshots(ui: &mut Ui, cx: &mut Cx<'_>, editing: Option<&Editing>) {
    let editable = editing.is_some_and(Editing::editable);
    if add_button(ui, "New snapshot", editable) {
        dialogs::open_new_snapshot(cx);
    }
    let Some(editing) = editing else {
        return;
    };
    let snapshots = cx.ui.develop.lists.snapshots.clone();
    if snapshots.is_empty() {
        ui.label(
            RichText::new("A snapshot keeps the photo's settings as they are now, by name.")
                .color(palette::TEXT_DIM)
                .small(),
        );
        return;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for snapshot in &snapshots {
        let copy = seen.entry(snapshot.name.as_str()).or_default();
        *copy += 1;
        let name = if *copy > 1 {
            format!("Snapshot {} ({copy})", snapshot.name)
        } else {
            format!("Snapshot {}", snapshot.name)
        };
        let row = ui
            .add_enabled_ui(editable, |ui| ListRow::new(&name, &snapshot.name).show(ui))
            .inner;
        if row.clicked {
            cx.act(Action::Develop(DevelopAction::ApplySnapshot(
                editing.photo,
                snapshot.id,
            )));
        }
        egui::Popup::context_menu(&row.response).show(|ui| {
            if ui.button("Show as Before").clicked() {
                show_as_before(cx, BeforeSettings::Snapshot(snapshot.id));
            }
            if ui.button("Rename…").clicked() {
                dialogs::open_rename_snapshot(cx, snapshot.id, &snapshot.name);
            }
            if ui.button("Delete…").clicked() {
                dialogs::open_delete_snapshot(cx, snapshot.id, &snapshot.name);
            }
        });
    }
}

/// Make `choice` the before view, and show it beside the edit if no before
/// view is on screen.
pub(super) fn show_as_before(cx: &mut Cx<'_>, choice: BeforeSettings) {
    let develop = &mut cx.ui.develop;
    develop.before_choice = choice;
    if !develop.before_after.shows_before() {
        develop.before_after = BeforeAfter::SideBySide;
    }
}

#[cfg(test)]
mod tests {
    use photo_core::develop::GroupSet;
    use photo_engine::BuiltInPreset;

    use super::*;

    fn preset(group: &str, name: &str) -> PresetInfo {
        PresetInfo {
            key: PresetKey::BuiltIn(BuiltInPreset::Vivid),
            group: group.to_owned(),
            name: name.to_owned(),
            groups: GroupSet::all(),
        }
    }

    #[test]
    fn a_name_two_groups_share_is_told_apart_by_its_group() {
        let names = row_names(&[
            preset("Photo Looks", "Vivid Colour"),
            preset("Mine", "Warm"),
            preset("Travel", "Warm"),
        ]);
        assert_eq!(
            names,
            [
                "Preset Vivid Colour",
                "Preset Warm (Mine)",
                "Preset Warm (Travel)"
            ]
        );
    }
}
