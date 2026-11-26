//! Naming and placing: template tokens, sequence padding, forbidden
//! characters, folders, the existing-file policies, and `planned_path`
//! agreeing with what `export` writes.

use std::fs;
use std::path::Path;

use photo_core::time::DateTime;
use photo_develop::CancelToken;
use photo_export::{
    ExistingFile, ExportOutcome, ExportSettings, ExtensionCase, Folder, Format, export,
    planned_path,
};

use crate::common::{Fixture, names, open, written};

/// Exports `fixture` and checks that `planned_path` said where beforehand.
fn export_as_planned(fixture: &Fixture, settings: &ExportSettings) -> ExportOutcome {
    let planned = planned_path(&fixture.item(), settings);
    let outcome = export(&fixture.item(), settings, &CancelToken::new()).unwrap();
    assert_eq!(outcome.path(), planned);
    outcome
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn tokens_fill_the_template() {
    let mut fixture = Fixture::named("IMG 0042.png", 40, 30);
    fixture.metadata.capture_time = DateTime::new(2024, 3, 15, 7, 42, 10);
    fixture.sequence = 7;
    let mut settings = fixture.settings_out();
    settings.naming.template = "{date}_{text}-{name}_{seq:4}".into();
    settings.naming.custom_text = "Trip".into();
    settings.naming.extension_case = ExtensionCase::Upper;
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(file_name(&path), "2024-03-15_Trip-IMG 0042_0007.JPG");
    assert_eq!(path.parent().unwrap(), fixture.out_dir());

    settings.naming.template = "{seq}-{SEQ:2}-{seq:3}-{ name }".into();
    settings.naming.extension_case = ExtensionCase::Lower;
    settings.format = Format::Tiff {
        depth: Default::default(),
    };
    fixture.sequence = 12_345;
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(
        file_name(&path),
        "12345-12345-12345-IMG 0042.tif",
        "padding never truncates"
    );
}

#[test]
fn unknown_tokens_stay_and_missing_values_fall_back_to_the_original_name() {
    let fixture = Fixture::named("beach.png", 40, 30);
    let mut settings = fixture.settings_out();
    for (template, custom, expected) in [
        ("{nope}-{seq:0}-{seq:10}", "", "{nope}-{seq_0}-{seq_10}.jpg"),
        ("{text}", "", "beach.jpg"),
        ("{date}", "", "beach.jpg"),
        ("...", "", "beach.jpg"),
        ("{text}", "Holiday", "Holiday.jpg"),
        ("{name", "", "{name.jpg"),
    ] {
        settings.naming.template = template.into();
        settings.naming.custom_text = custom.into();
        let (path, _, _) = written(&export_as_planned(&fixture, &settings));
        assert_eq!(file_name(&path), expected, "{template:?}");
        fs::remove_file(path).unwrap();
    }
}

/// A token without a value takes a neighbouring separator with it: the name
/// neither starts nor ends with a separator nor has two in a row because of
/// it. The export dialog shows `planned_path` as its example, and the file
/// written has the name it showed.
#[test]
fn an_empty_token_leaves_no_stray_separator_in_the_planned_or_the_written_name() {
    let mut fixture = Fixture::named("IMG 0042.png", 40, 30);
    let mut settings = fixture.settings_out();
    settings.format = Format::Tiff {
        depth: Default::default(),
    };
    let date = DateTime::new(2024, 3, 15, 7, 42, 10);
    for (template, custom, dated, expected) in [
        // The dialog's example: no capture date.
        ("{date}-{text}-{seq:3}", "coast", false, "coast-001.tif"),
        // With every value it is as typed.
        (
            "{date}-{text}-{seq:3}",
            "coast",
            true,
            "2024-03-15-coast-001.tif",
        ),
        ("{date}-{text}-{seq:3}", "", true, "2024-03-15-001.tif"),
        ("{date}-{text}-{seq:3}", "", false, "001.tif"),
        ("{name}_{text}", "", false, "IMG 0042.tif"),
        ("{name}__{text}__{seq}", "x", false, "IMG 0042__x__1.tif"),
        // No token has a value: the original's name, not an empty one.
        ("{date}-{text}", "", false, "IMG 0042.tif"),
    ] {
        settings.naming.template = template.into();
        settings.naming.custom_text = custom.into();
        fixture.metadata.capture_time = if dated { date } else { None };
        let planned = planned_path(&fixture.item(), &settings);
        assert_eq!(file_name(&planned), expected, "{template:?}");
        let (path, _, _) = written(&export_as_planned(&fixture, &settings));
        assert_eq!(path, planned, "{template:?}");
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn characters_no_file_name_may_hold_are_replaced() {
    let fixture = Fixture::named("shot.png", 40, 30);
    let mut settings = fixture.settings_out();
    settings.naming.template = "a/b\\c:d*e?f\"g<h>i|j\t{text}. ".into();
    settings.naming.custom_text = "..x/../y".into();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(file_name(&path), "a_b_c_d_e_f_g_h_i_j_..x_.._y.jpg");
    assert_eq!(
        path.parent().unwrap(),
        fixture.out_dir(),
        "nothing escapes the folder"
    );
    settings.naming.template = "nul".into();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(file_name(&path), "nul_.jpg");
}

#[test]
fn files_go_to_the_chosen_folder_or_beside_the_original_with_a_subfolder() {
    let fixture = Fixture::named("pic.png", 40, 30);
    let mut settings = fixture.settings_out();
    settings.location.subfolder = "web/large".into();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(
        path,
        fixture.out_dir().join("web").join("large").join("pic.jpg")
    );

    settings.location.folder = Folder::SameAsOriginal;
    settings.location.subfolder = "../escape/Exports".into();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    let beside = fixture.original.parent().unwrap();
    assert_eq!(path, beside.join("escape").join("Exports").join("pic.jpg"));

    settings.location.subfolder = String::new();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(path, beside.join("pic.jpg"));
}

#[test]
fn the_unique_name_policy_counts_up_past_every_taken_name() {
    let fixture = Fixture::named("pic.png", 40, 30);
    let settings = fixture.settings_out();
    let mut paths = Vec::new();
    for _ in 0..3 {
        paths.push(written(&export_as_planned(&fixture, &settings)).0);
    }
    let names_written: Vec<String> = paths.iter().map(|p| file_name(p)).collect();
    assert_eq!(names_written, ["pic.jpg", "pic-2.jpg", "pic-3.jpg"]);
    // A gap is filled, and a name taken by a folder counts as taken.
    fs::remove_file(&paths[1]).unwrap();
    fs::create_dir(fixture.out_dir().join("pic-4.jpg")).unwrap();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(file_name(&path), "pic-2.jpg");
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(file_name(&path), "pic-5.jpg");
}

#[test]
fn overwrite_replaces_the_file_and_skip_leaves_it_alone() {
    let fixture = Fixture::named("pic.png", 40, 30);
    let mut settings = fixture.settings_out();
    fs::create_dir_all(fixture.out_dir()).unwrap();
    let existing = fixture.out_dir().join("pic.jpg");
    fs::write(&existing, b"an older export").unwrap();

    settings.location.existing = ExistingFile::Skip;
    let outcome = export_as_planned(&fixture, &settings);
    assert_eq!(
        outcome,
        ExportOutcome::Skipped {
            path: existing.clone()
        }
    );
    assert_eq!(fs::read(&existing).unwrap(), b"an older export");

    settings.location.existing = ExistingFile::Overwrite;
    let (path, width, height) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(path, existing);
    let decoded = open(&path);
    assert_eq!((decoded.width(), decoded.height()), (width, height));
    assert_eq!(names(&fixture.out_dir()), ["pic.jpg"]);

    // Skipping a name nothing has writes it.
    settings.location.existing = ExistingFile::Skip;
    settings.naming.template = "fresh".into();
    let (path, _, _) = written(&export_as_planned(&fixture, &settings));
    assert_eq!(file_name(&path), "fresh.jpg");
}
