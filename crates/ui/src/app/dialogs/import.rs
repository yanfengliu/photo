//! The Import dialog: where the photos come from (a folder chosen with the
//! native dialog or typed), whether its subfolders count, whether the files
//! stay where they are or are copied into folders by date, what to skip,
//! keywords to give every photo, and whether to read XMP sidecars; below,
//! the candidates the scan finds, ticked, filled in as they are found.
//!
//! The folder is looked in when its path is committed: Enter, leaving the
//! field, a path set from outside (a script, assistive technology), or the
//! native dialog. Import hands the ticked files to the engine and closes;
//! progress shows in the top bar, and when it ends the Library shows the
//! import.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{Key, Modifiers, RichText, TextEdit, Ui};
use photo_core::time::DateTime;
use photo_engine::{
    CopyLayout, DatePattern, ImportCandidate, ImportMode, ImportRequest, ImportScan,
};

use super::import_grid;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::text;
use crate::theme::{palette, space};
use crate::widgets::{Dialog, DialogOutcome};

/// The date folders copies go into unless the user says otherwise.
const DEFAULT_PATTERN: &str = "YYYY/YYYY-MM-DD";
/// The date the folder example uses while no candidate has one.
const SAMPLE_DATE: DateTime = DateTime {
    year: 2024,
    month: 3,
    day: 15,
    hour: 12,
    minute: 0,
    second: 0,
};

/// The choices the dialog keeps from one import to the next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportSettings {
    pub(crate) source: String,
    pub(crate) recursive: bool,
    pub(crate) copy: bool,
    pub(crate) destination: String,
    pub(crate) pattern: String,
    pub(crate) skip_duplicates: bool,
    pub(crate) keywords: String,
    pub(crate) read_sidecars: bool,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            source: String::new(),
            recursive: true,
            copy: false,
            destination: String::new(),
            pattern: DEFAULT_PATTERN.to_owned(),
            skip_duplicates: true,
            keywords: String::new(),
            read_sidecars: true,
        }
    }
}

/// The Import dialog while it is open.
pub(crate) struct ImportDialog {
    settings: ImportSettings,
    /// The folder looked in, and whether with its subfolders.
    scanned: Option<(PathBuf, bool)>,
    /// Candidates the user unticked.
    unchecked: HashSet<PathBuf>,
    /// Why the typed folder cannot be looked in.
    problem: Option<String>,
}

impl ImportDialog {
    /// Open with last time's settings, looking in last time's folder again.
    pub(crate) fn open(cx: &mut Cx<'_>) -> Self {
        let mut dialog = Self {
            settings: cx.ui.import_settings.clone(),
            scanned: None,
            unchecked: HashSet::new(),
            problem: None,
        };
        if !dialog.settings.source.trim().is_empty() {
            dialog.scan(cx);
        }
        dialog
    }

    /// Look in the folder the settings name.
    fn scan(&mut self, cx: &mut Cx<'_>) {
        let root = PathBuf::from(self.settings.source.trim());
        if !root.is_dir() {
            self.problem = Some(format!("There is no folder at {}.", root.display()));
            self.scanned = None;
            return;
        }
        self.problem = None;
        self.unchecked.clear();
        let recursive = self.settings.recursive;
        self.scanned = Some((root.clone(), recursive));
        cx.act(Action::ScanFolder { root, recursive });
    }

    /// The scan of this dialog's folder, as far as it has got.
    fn scan_result<'a>(&self, cx: &'a Cx<'_>) -> Option<&'a ImportScan> {
        let (root, recursive) = self.scanned.as_ref()?;
        cx.engine
            .import_scan()
            .filter(|scan| &scan.root == root && scan.recursive == *recursive)
    }

    /// Whether a candidate goes into the import.
    pub(super) fn is_ticked(&self, candidate: &ImportCandidate) -> bool {
        candidate_ticked(candidate, &self.unchecked, self.settings.skip_duplicates)
    }

    /// Whether a click can tick or untick a candidate: not one the import
    /// would leave out anyway.
    pub(super) fn is_tickable(&self, candidate: &ImportCandidate) -> bool {
        !left_out(candidate, self.settings.skip_duplicates)
    }

    /// Tick or untick a candidate.
    pub(super) fn toggle(&mut self, path: &Path) {
        if !self.unchecked.remove(path) {
            self.unchecked.insert(path.to_path_buf());
        }
    }

    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let candidates: Vec<ImportCandidate> = self
            .scan_result(cx)
            .map(|scan| scan.candidates.clone())
            .unwrap_or_default();
        let ticked: Vec<PathBuf> = candidates
            .iter()
            .filter(|candidate| self.is_ticked(candidate))
            .map(|candidate| candidate.path.clone())
            .collect();
        let request = self.request(ticked.clone());
        let confirm = format!("Import {}", text::count(ticked.len(), "Photo", "Photos"));
        let response = Dialog::new("Import Photos")
            .confirm_label(&confirm)
            .can_confirm(request.is_ok())
            .width(880.0)
            .show(ctx, |ui| {
                self.source_rows(ui, cx);
                ui.add_space(space::MD);
                self.file_rows(ui, &candidates);
                ui.add_space(space::MD);
                ui.separator();
                self.candidate_header(ui, cx, &candidates, ticked.len());
                import_grid::show(ui, cx, self, &candidates);
            });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => {
                self.close(cx);
                false
            }
            DialogOutcome::Confirmed => {
                if let Ok(request) = request {
                    cx.act(Action::StartImport(request));
                }
                self.close(cx);
                false
            }
        }
    }

    /// Leave: keep the settings for next time, stop a scan still running
    /// and let the candidates' thumbnails go.
    fn close(&mut self, cx: &mut Cx<'_>) {
        cx.ui.import_settings = self.settings.clone();
        if let Some(scan) = self.scan_result(cx)
            && !scan.finished
        {
            let task = scan.task;
            cx.act(Action::CancelTask(task));
        }
        cx.textures.clear_candidates();
    }

    /// The import the dialog would start, or why it cannot yet.
    fn request(&self, files: Vec<PathBuf>) -> Result<ImportRequest, String> {
        if files.is_empty() {
            return Err("Nothing is ticked.".to_owned());
        }
        let mode = if self.settings.copy {
            let destination = self.settings.destination.trim();
            if destination.is_empty() {
                return Err("Choose where the copies go.".to_owned());
            }
            let pattern = DatePattern::new(self.settings.pattern.trim())
                .map_err(|error| error.to_string())?;
            ImportMode::Copy {
                destination: PathBuf::from(destination),
                layout: CopyLayout::ByDate(pattern),
            }
        } else {
            ImportMode::Add
        };
        Ok(ImportRequest {
            files,
            mode,
            skip_suspected_duplicates: self.settings.skip_duplicates,
            keywords: text::split_names(&self.settings.keywords),
            read_sidecars: self.settings.read_sidecars,
        })
    }

    /// The source folder, chosen or typed, and its subfolders switch.
    fn source_rows(&mut self, ui: &mut Ui, cx: &mut Cx<'_>) {
        ui.horizontal(|ui| {
            caption(ui, "From");
            let field = ui
                .add(
                    TextEdit::singleline(&mut self.settings.source)
                        .id_salt("import-source")
                        .hint_text("A folder of photos")
                        .desired_width(620.0),
                )
                .accessible_name("Source folder");
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if entered {
                // Enter here looks in the folder; it does not import.
                ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            }
            let committed = field.lost_focus() || (field.changed() && !field.has_focus());
            if ui.button("Choose…").clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose a folder to import from")
                    .pick_folder()
            {
                self.settings.source = folder.display().to_string();
                self.scan(cx);
            } else if committed {
                self.scan(cx);
            }
        });
        ui.horizontal(|ui| {
            caption(ui, "");
            if ui
                .checkbox(&mut self.settings.recursive, "Include subfolders")
                .changed()
                && self.scanned.is_some()
            {
                self.scan(cx);
            }
        });
        if let Some(problem) = &self.problem {
            ui.horizontal(|ui| {
                caption(ui, "");
                ui.label(RichText::new(problem).color(palette::WARNING));
            });
        }
    }

    /// Add or copy, the destination and its date folders, and the options.
    fn file_rows(&mut self, ui: &mut Ui, candidates: &[ImportCandidate]) {
        let settings = &mut self.settings;
        ui.horizontal(|ui| {
            caption(ui, "Files");
            ui.radio_value(&mut settings.copy, false, "Add in place");
            ui.radio_value(&mut settings.copy, true, "Copy to a folder by date");
        });
        if settings.copy {
            ui.horizontal(|ui| {
                caption(ui, "To");
                ui.add(
                    TextEdit::singleline(&mut settings.destination)
                        .id_salt("import-destination")
                        .hint_text("The folder the dated folders go in")
                        .desired_width(520.0),
                )
                .accessible_name("Destination folder");
                if ui.button("Choose…").clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title("Choose where the copies go")
                        .pick_folder()
                {
                    settings.destination = folder.display().to_string();
                }
            });
            ui.horizontal(|ui| {
                caption(ui, "Folders");
                ui.add(
                    TextEdit::singleline(&mut settings.pattern)
                        .id_salt("import-pattern")
                        .desired_width(180.0),
                )
                .accessible_name("Date folder pattern");
                let example = pattern_example(settings, candidates);
                ui.label(RichText::new(example).color(palette::TEXT_DIM));
            });
        }
        ui.horizontal(|ui| {
            caption(ui, "Options");
            ui.checkbox(&mut settings.skip_duplicates, "Skip suspected duplicates");
            ui.add_space(space::LG);
            ui.checkbox(
                &mut settings.read_sidecars,
                "Read metadata from XMP sidecars",
            );
        });
        ui.horizontal(|ui| {
            caption(ui, "Keywords");
            ui.add(
                TextEdit::singleline(&mut settings.keywords)
                    .id_salt("import-keywords")
                    .hint_text("Given to every photo, separated by commas")
                    .desired_width(520.0),
            )
            .accessible_name("Import keywords");
        });
    }

    /// "Scanning… 12 found" or "6 photos found, 6 ticked", and the buttons
    /// that tick and untick them all.
    fn candidate_header(
        &mut self,
        ui: &mut Ui,
        cx: &Cx<'_>,
        candidates: &[ImportCandidate],
        ticked: usize,
    ) {
        let scanning = self.scan_result(cx).is_some_and(|scan| !scan.finished);
        ui.horizontal(|ui| {
            let found = text::count(candidates.len(), "photo", "photos");
            let status = if self.scanned.is_none() {
                "Choose a folder to see its photos here.".to_owned()
            } else if scanning {
                format!("Looking… {found} found so far")
            } else {
                format!("{found} found, {ticked} ticked")
            };
            ui.label(RichText::new(status).color(palette::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Uncheck All").clicked() {
                    self.unchecked = candidates.iter().map(|c| c.path.clone()).collect();
                }
                if ui.button("Check All").clicked() {
                    self.unchecked.clear();
                }
            });
        });
    }
}

/// Whether a candidate goes into the import: not unticked, and not one the
/// import leaves out anyway.
pub(super) fn candidate_ticked(
    candidate: &ImportCandidate,
    unchecked: &HashSet<PathBuf>,
    skip_duplicates: bool,
) -> bool {
    !left_out(candidate, skip_duplicates) && !unchecked.contains(&candidate.path)
}

/// Whether the import leaves a candidate out whatever its tick: it is
/// already in the catalog, or a suspected duplicate while those are
/// skipped.
fn left_out(candidate: &ImportCandidate, skip_duplicates: bool) -> bool {
    candidate.already_in_catalog || (skip_duplicates && candidate.suspected_duplicate)
}

/// Where a copy would go: "e.g. …/2024/2024-03-15/dawn.jpg", from the first
/// candidate with a capture date.
fn pattern_example(settings: &ImportSettings, candidates: &[ImportCandidate]) -> String {
    let Ok(pattern) = DatePattern::new(settings.pattern.trim()) else {
        return "Use YYYY, YY, MM and DD, and / between folders.".to_owned();
    };
    let sample = candidates
        .iter()
        .find_map(|candidate| candidate.capture_time.map(|time| (time, &candidate.path)));
    let (date, name) = match sample {
        Some((date, path)) => (date, text::file_name(path)),
        None => (SAMPLE_DATE, "photo.jpg".to_owned()),
    };
    let folder = pattern.folder_for(date);
    format!("e.g. {}", folder.join(name).display())
}

/// The label at the left of a row of the dialog.
fn caption(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(64.0, 20.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| ui.label(RichText::new(text).color(palette::TEXT_DIM)),
    );
}

#[cfg(test)]
mod tests {
    use photo_core::meta::FileKind;

    use super::*;

    fn candidate(path: &str, already: bool, duplicate: bool) -> ImportCandidate {
        ImportCandidate {
            path: PathBuf::from(path),
            kind: FileKind::Raster,
            file_size: 1,
            modified: 0,
            capture_time: None,
            already_in_catalog: already,
            suspected_duplicate: duplicate,
        }
    }

    #[test]
    fn everything_new_is_ticked_unless_the_user_unticks_it() {
        let mut unchecked = HashSet::new();
        assert!(candidate_ticked(
            &candidate("/a.jpg", false, false),
            &unchecked,
            true
        ));
        assert!(!candidate_ticked(
            &candidate("/b.jpg", true, false),
            &unchecked,
            false
        ));
        assert!(!candidate_ticked(
            &candidate("/c.jpg", false, true),
            &unchecked,
            true
        ));
        assert!(candidate_ticked(
            &candidate("/c.jpg", false, true),
            &unchecked,
            false
        ));
        unchecked.insert(PathBuf::from("/a.jpg"));
        assert!(!candidate_ticked(
            &candidate("/a.jpg", false, false),
            &unchecked,
            true
        ));
    }

    #[test]
    fn the_example_shows_the_folders_a_copy_goes_into() {
        let settings = ImportSettings::default();
        let mut dawn = candidate("/shoot/dawn.jpg", false, false);
        dawn.capture_time = DateTime::new(2024, 7, 4, 21, 30, 0);
        let example = pattern_example(&settings, &[dawn]);
        let expected = Path::new("2024").join("2024-07-04").join("dawn.jpg");
        assert_eq!(example, format!("e.g. {}", expected.display()));
        let broken = ImportSettings {
            pattern: "../YYYY".to_owned(),
            ..ImportSettings::default()
        };
        assert!(pattern_example(&broken, &[]).starts_with("Use YYYY"));
    }
}
