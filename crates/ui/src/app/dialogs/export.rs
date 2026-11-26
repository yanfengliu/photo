//! The Export dialog (design §8.4): where the files go and what happens
//! when a name is taken, how they are named (with the name the first photo
//! would get, as the engine plans it), then the file settings of
//! `export_fields`, and saved presets of all of it.
//!
//! Export hands the settings to the engine and closes; progress and Cancel
//! show in the top bar, and when it ends so does a way to the folder.
//! Nothing here can write over an original: the engine refuses any name
//! that belongs to a photo in the catalog, and replacing existing files is
//! not offered at all beside the originals, where a file of the same name
//! may be one the catalog does not know.

use std::path::{Path, PathBuf};

use egui::{Button, ComboBox, Key, Modifiers, RichText, TextEdit, Ui};
use photo_engine::{ExistingFile, ExportSettings, ExtensionCase, Folder, Target};

use super::export_fields::{self, Kept};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::text;
use crate::theme::{self, palette, space};
use crate::widgets::{Dialog, DialogOutcome};

/// The tokens a name template takes, with what each stands for.
const TOKENS: [(&str, &str); 4] = [
    ("{name}", "The original file's name"),
    (
        "{seq}",
        "The photo's number in the export ({seq:3} pads it to three digits)",
    ),
    ("{date}", "The capture date, as 2024-03-15"),
    ("{text}", "The custom text"),
];

/// What the menu calls saving no preset's settings.
const NO_PRESET: &str = "(none)";

/// The Export dialog while it is open.
pub(crate) struct ExportDialog {
    settings: ExportSettings,
    target: Target,
    /// How many photos the target holds.
    count: usize,
    /// The folder typed, kept while the files go beside their originals.
    folder: String,
    /// The subfolder typed, kept while none is used.
    subfolder: String,
    presets: Vec<(String, ExportSettings)>,
    /// When the presets were last read.
    read_at: Option<u64>,
    /// The preset whose settings were loaded last.
    chosen: Option<String>,
    /// The name being typed for a preset to save.
    naming: Option<String>,
    /// Where the first photo would go, for the settings it was planned for.
    planned: Option<(ExportSettings, Option<PathBuf>)>,
    /// Values kept while their control is switched off.
    kept: Kept,
}

impl ExportDialog {
    /// Open for the photos `target` stands for, with the settings of the
    /// export before, or the engine's defaults.
    pub(crate) fn open(cx: &mut Cx<'_>, target: Target, count: usize) -> Self {
        let settings = cx
            .ui
            .export_settings
            .clone()
            .unwrap_or_else(|| cx.engine.default_export_settings());
        let mut dialog = Self {
            settings: ExportSettings::default(),
            target,
            count,
            folder: cx.engine.export_dir().display().to_string(),
            subfolder: String::new(),
            presets: Vec::new(),
            read_at: None,
            chosen: None,
            naming: None,
            planned: None,
            kept: Kept::default(),
        };
        dialog.load(settings);
        dialog
    }

    /// Take `settings` as the dialog's, the fields kept beside them
    /// following.
    fn load(&mut self, settings: ExportSettings) {
        if let Folder::Specific(folder) = &settings.location.folder {
            self.folder = folder.display().to_string();
        }
        if !settings.location.subfolder.is_empty() {
            self.subfolder.clone_from(&settings.location.subfolder);
        }
        self.kept.follow(&settings);
        self.settings = settings;
    }

    /// Whether the files go beside their originals.
    fn beside(&self) -> bool {
        self.settings.location.folder == Folder::SameAsOriginal
    }

    /// Why the export cannot start yet, if it cannot.
    fn problem(&self) -> Option<&'static str> {
        if self.count == 0 {
            Some("No photo is selected.")
        } else if !self.beside() && self.folder.trim().is_empty() {
            Some("Choose the folder the files go to.")
        } else {
            None
        }
    }

    /// Where the first photo would be written, as the engine plans it now.
    fn example(&mut self, cx: &Cx<'_>) -> Option<&Path> {
        let fresh = self
            .planned
            .as_ref()
            .is_some_and(|(settings, _)| *settings == self.settings);
        if !fresh {
            let path = cx
                .engine
                .planned_export_path(self.target.clone(), &self.settings)
                .unwrap_or_else(|error| {
                    log::warn!("cannot plan the export's name: {error}");
                    None
                });
            self.planned = Some((self.settings.clone(), path));
        }
        self.planned.as_ref()?.1.as_deref()
    }

    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        if self.read_at != Some(cx.generation) {
            self.read_at = Some(cx.generation);
            self.presets = cx.engine.export_presets().unwrap_or_else(|error| {
                log::warn!("cannot read the export presets: {error}");
                Vec::new()
            });
        }
        let confirm = format!("Export {}", text::count(self.count, "Photo", "Photos"));
        let problem = self.problem();
        let response = Dialog::new("Export")
            .confirm_label(&confirm)
            .can_confirm(problem.is_none())
            .width(640.0)
            .show(ctx, |ui| {
                self.preset_rows(ui, cx);
                section(ui, "Location");
                self.location_rows(ui);
                section(ui, "File Naming");
                self.naming_rows(ui, cx);
                export_fields::show(ui, &mut self.settings, &mut self.kept);
                ui.add_space(space::LG);
                ui.label(
                    RichText::new(
                        "Originals are never written. Every export is a new file, and a name that belongs to a photo in the catalog is refused.",
                    )
                    .color(palette::TEXT_DIM)
                    .small(),
                );
                if let Some(problem) = problem {
                    ui.label(RichText::new(problem).color(palette::WARNING));
                }
            });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => {
                cx.ui.export_settings = Some(self.settings.clone());
                false
            }
            DialogOutcome::Confirmed => {
                cx.ui.export_settings = Some(self.settings.clone());
                cx.act(Action::StartExport(
                    self.target.clone(),
                    Box::new(self.settings.clone()),
                ));
                false
            }
        }
    }

    /// The saved presets: load one, save the settings as one, delete one.
    fn preset_rows(&mut self, ui: &mut Ui, cx: &mut Cx<'_>) {
        row(ui, "Preset", |ui| {
            // The preset loaded last, and whether the settings are still
            // its own.
            let saved = self
                .chosen
                .as_ref()
                .and_then(|name| self.presets.iter().find(|(other, _)| other == name));
            let shown = match (&self.chosen, saved) {
                (Some(name), Some((_, settings))) if *settings != self.settings => {
                    format!("{name} (changed)")
                }
                (Some(name), _) => name.clone(),
                (None, _) => NO_PRESET.to_owned(),
            };
            let mut picked = None;
            ComboBox::from_id_salt("export-preset")
                .selected_text(shown)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (name, _) in &self.presets {
                        let current = self.chosen.as_deref() == Some(name.as_str());
                        if ui.selectable_label(current, name.as_str()).clicked() {
                            picked = Some(name.clone());
                        }
                    }
                    if self.presets.is_empty() {
                        ui.label(RichText::new("No presets saved yet").color(palette::TEXT_DIM));
                    }
                })
                .response
                .accessible_name("Export preset");
            if let Some(name) = picked
                && let Some((_, settings)) = self.presets.iter().find(|(other, _)| *other == name)
            {
                let settings = settings.clone();
                self.load(settings);
                self.chosen = Some(name);
            }
            if ui.button("Save Preset…").clicked() {
                self.naming = Some(self.chosen.clone().unwrap_or_default());
            }
            let deletable = self
                .chosen
                .as_ref()
                .is_some_and(|name| self.presets.iter().any(|(other, _)| other == name));
            if ui
                .add_enabled(deletable, Button::new("Delete Preset"))
                .clicked()
                && let Some(name) = self.chosen.take()
            {
                cx.act(Action::DeleteExportPreset(name));
            }
        });
        let Some(name) = &mut self.naming else {
            return;
        };
        let mut save = false;
        let mut close = false;
        row(ui, "Save as", |ui| {
            let field = ui
                .add(
                    TextEdit::singleline(name)
                        .id_salt("export-preset-name")
                        .hint_text("A name for these settings")
                        .desired_width(220.0),
                )
                .accessible_name("Export preset name");
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if entered {
                // Enter here saves the preset; it does not export.
                ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            }
            let named = !name.trim().is_empty();
            save = (ui.add_enabled(named, Button::new("Save")).clicked() || entered) && named;
            close = ui.button("Don't Save").clicked();
        });
        if save {
            let name = name.trim().to_owned();
            cx.act(Action::SaveExportPreset(
                name.clone(),
                Box::new(self.settings.clone()),
            ));
            self.chosen = Some(name);
        }
        if save || close {
            self.naming = None;
        }
    }

    /// The folder, the subfolder and what happens to a name that is taken.
    fn location_rows(&mut self, ui: &mut Ui) {
        let mut beside = self.beside();
        row(ui, "Export to", |ui| {
            ui.radio_value(&mut beside, false, "A folder");
            ui.radio_value(&mut beside, true, "Beside each original");
        });
        row(ui, "Folder", |ui| {
            ui.add_enabled(
                !beside,
                TextEdit::singleline(&mut self.folder)
                    .id_salt("export-folder")
                    .hint_text("The folder the files go to")
                    .desired_width(400.0),
            )
            .accessible_name("Export folder");
            if ui.add_enabled(!beside, Button::new("Choose…")).clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose the folder to export to")
                    .pick_folder()
            {
                self.folder = folder.display().to_string();
            }
        });
        let location = &mut self.settings.location;
        location.folder = if beside {
            Folder::SameAsOriginal
        } else {
            Folder::Specific(PathBuf::from(self.folder.trim()))
        };
        row(ui, "", |ui| {
            let mut used = !location.subfolder.is_empty();
            let toggled = ui.checkbox(&mut used, "Put in a subfolder").changed();
            let typed = ui
                .add_enabled(
                    used,
                    TextEdit::singleline(&mut self.subfolder)
                        .id_salt("export-subfolder")
                        .hint_text("Its name")
                        .desired_width(240.0),
                )
                .accessible_name("Export subfolder")
                .changed();
            if toggled && used && self.subfolder.trim().is_empty() {
                self.subfolder = "Export".to_owned();
            }
            if toggled || typed {
                location.subfolder = if used {
                    self.subfolder.trim().to_owned()
                } else {
                    String::new()
                };
            }
        });
        // Beside the originals a file of the same name may be an original
        // the catalog does not know, so replacing is not offered there.
        if beside && location.existing == ExistingFile::Overwrite {
            location.existing = ExistingFile::UniqueName;
        }
        row(ui, "Existing files", |ui| {
            let label = |policy: ExistingFile| match policy {
                ExistingFile::UniqueName => "Give the new file another name",
                ExistingFile::Skip => "Skip the photo",
                ExistingFile::Overwrite => "Replace the earlier export",
            };
            ComboBox::from_id_salt("export-existing")
                .selected_text(label(location.existing))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for policy in [
                        ExistingFile::UniqueName,
                        ExistingFile::Skip,
                        ExistingFile::Overwrite,
                    ] {
                        if policy == ExistingFile::Overwrite && beside {
                            continue;
                        }
                        ui.selectable_value(&mut location.existing, policy, label(policy));
                    }
                })
                .response
                .accessible_name("Existing files");
            if beside {
                ui.label(
                    RichText::new("Nothing beside an original is replaced.")
                        .color(palette::TEXT_DIM)
                        .small(),
                );
            }
        });
    }

    /// The name template, its tokens, and the name it gives the first
    /// photo.
    fn naming_rows(&mut self, ui: &mut Ui, cx: &Cx<'_>) {
        let naming = &mut self.settings.naming;
        row(ui, "Name", |ui| {
            ui.add(
                TextEdit::singleline(&mut naming.template)
                    .id_salt("export-template")
                    .hint_text("{name}")
                    .desired_width(240.0),
            )
            .accessible_name("File name template");
            for (token, hint) in TOKENS {
                if ui.button(token).on_hover_text(hint).clicked() {
                    naming.template.push_str(token);
                }
            }
        });
        row(ui, "Custom text", |ui| {
            ui.add(
                TextEdit::singleline(&mut naming.custom_text)
                    .id_salt("export-custom-text")
                    .hint_text("What {text} stands for")
                    .desired_width(240.0),
            )
            .accessible_name("Custom text");
            let label = |case: ExtensionCase| match case {
                ExtensionCase::Lower => "Lower-case extension",
                ExtensionCase::Upper => "Upper-case extension",
            };
            ComboBox::from_id_salt("export-extension")
                .selected_text(label(naming.extension_case))
                .width(170.0)
                .show_ui(ui, |ui| {
                    for case in [ExtensionCase::Lower, ExtensionCase::Upper] {
                        ui.selectable_value(&mut naming.extension_case, case, label(case));
                    }
                })
                .response
                .accessible_name("Extension case");
        });
        let example = self.example(cx).map(Path::to_path_buf);
        row(ui, "Example", |ui| {
            // Flowing like text: a long folder goes on to the next line
            // instead of widening the dialog.
            ui.horizontal_wrapped(|ui| match &example {
                Some(path) => {
                    ui.label(RichText::new(text::file_name(path)).color(palette::TEXT_STRONG));
                    if let Some(folder) = path.parent() {
                        ui.label(
                            RichText::new(format!("in {}", folder.display()))
                                .color(palette::TEXT_DIM)
                                .small(),
                        );
                    }
                }
                None => {
                    ui.label(RichText::new("No photo to name").color(palette::TEXT_DIM));
                }
            });
        });
    }
}

/// A section's heading.
pub(super) fn section(ui: &mut Ui, title: &str) {
    ui.add_space(space::LG);
    ui.label(theme::header_text(title, palette::TEXT_DIM));
    ui.add_space(space::XS);
}

/// A row of the dialog: its caption at the left, its controls after it.
pub(super) fn row(ui: &mut Ui, caption: &str, controls: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(96.0, 20.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| ui.label(RichText::new(caption).color(palette::TEXT_DIM)),
        );
        controls(ui);
    });
}
