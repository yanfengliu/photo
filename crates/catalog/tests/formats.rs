//! Settings and smart rules written by a newer version are never rewritten
//! by this one, which would silently drop what it does not know. They carry
//! their format number; reading one that is too new is a typed error, and
//! so is any change that would replace it, which then changes nothing.

mod common;

use std::path::{Path, PathBuf};

use common::{import_paths, raster};
use photo_catalog::{
    Catalog, CatalogError, DevelopEdit, Entity, SmartField, SmartOp, SmartRule, SmartRules, Source,
};
use photo_core::develop::{DevelopSettings, GroupSet, SETTINGS_FORMAT};
use rusqlite::Connection;

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn newer_settings(error: &CatalogError) -> bool {
    matches!(
        error,
        CatalogError::NewerFormat { what: "develop settings", found, supported }
            if *found == SETTINGS_FORMAT + 1 && *supported == SETTINGS_FORMAT
    )
}

/// Run raw SQL on the file, as another version of the program would.
fn tamper(path: &Path, sql: &str) {
    Connection::open(path).unwrap().execute_batch(sql).unwrap();
}

/// A text column of one row, read raw.
fn raw(path: &Path, sql: &str) -> String {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn catalog_file(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("library.photocat")
}

#[test]
fn every_stored_copy_carries_the_format_it_was_written_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_file(&dir);
    {
        let mut catalog = Catalog::open(&path).unwrap();
        let mut photo = raster("/f/a.jpg");
        photo.develop = Some(exposure(0.1));
        let run = catalog.begin_import().unwrap();
        let photo = catalog.insert_photo(run, &photo).unwrap();
        catalog
            .set_develop_settings(photo, &exposure(0.5), Some("Edit"))
            .unwrap();
        catalog
            .create_snapshot(photo, "Snap", &exposure(0.5))
            .unwrap();
        catalog
            .create_preset("Mine", "Look", &exposure(0.5), GroupSet::all())
            .unwrap();
        catalog
            .create_smart_collection("Smart", None, &SmartRules::default())
            .unwrap();
    }
    let format = |sql| raw(&path, sql);
    let settings = SETTINGS_FORMAT.to_string();
    assert_eq!(
        format("SELECT CAST(develop_format AS TEXT) FROM photos"),
        settings
    );
    assert_eq!(
        format("SELECT CAST(MAX(settings_format) AS TEXT) FROM history"),
        settings
    );
    assert_eq!(
        format("SELECT CAST(settings_format AS TEXT) FROM snapshots"),
        settings
    );
    assert_eq!(
        format("SELECT CAST(settings_format AS TEXT) FROM presets"),
        settings
    );
    assert_eq!(
        format("SELECT CAST(rules_format AS TEXT) FROM collections WHERE kind = 2"),
        SmartRules::FORMAT.to_string()
    );
}

#[test]
fn a_photos_newer_settings_are_reported_and_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_file(&dir);
    let (photo, other, step) = {
        let mut catalog = Catalog::open(&path).unwrap();
        let ids = import_paths(&mut catalog, &["/f/a.jpg", "/f/b.jpg"]);
        let change = catalog
            .set_develop_settings(ids[0], &exposure(0.5), Some("Edit"))
            .unwrap();
        (ids[0], ids[1], change.history_step().unwrap())
    };
    // A newer version edited the photo with a tool this one does not know.
    tamper(
        &path,
        &format!(
            "UPDATE photos SET develop = json_set(develop, '$.future', 7), develop_format = {}
             WHERE id = {}",
            SETTINGS_FORMAT + 1,
            photo.0
        ),
    );
    let stored = || {
        raw(
            &path,
            &format!("SELECT develop FROM photos WHERE id = {}", photo.0),
        )
    };
    let before = stored();

    let mut catalog = Catalog::open(&path).unwrap();
    assert!(newer_settings(
        &catalog.develop_settings(photo).unwrap_err()
    ));
    assert!(newer_settings(
        &catalog
            .set_develop_settings(photo, &exposure(1.0), None)
            .unwrap_err()
    ));
    assert!(newer_settings(
        &catalog.go_to_history_step(photo, step).unwrap_err()
    ));
    let settings = exposure(1.0);
    let edits = [
        DevelopEdit {
            photo: other,
            settings: &settings,
            history_label: None,
        },
        DevelopEdit {
            photo,
            settings: &settings,
            history_label: Some("Paste"),
        },
    ];
    assert!(newer_settings(
        &catalog.apply_develop_edits(&edits).unwrap_err()
    ));
    assert_eq!(
        catalog.photo(other).unwrap().develop_version,
        0,
        "all or nothing"
    );

    // Copying, removing and restoring keep the settings exactly as written.
    let copy = catalog.create_virtual_copy(photo).unwrap();
    assert!(newer_settings(&catalog.develop_settings(copy).unwrap_err()));
    let removed = catalog.remove_photos(&[photo]).unwrap();
    catalog.restore_photos(&removed).unwrap();
    assert!(newer_settings(
        &catalog.develop_settings(photo).unwrap_err()
    ));
    assert!(catalog.photo(photo).unwrap().has_edits);
    drop(catalog);
    assert_eq!(stored(), before);
}

#[test]
fn newer_history_steps_are_not_gone_to_or_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_file(&dir);
    let (photo, first, second) = {
        let mut catalog = Catalog::open(&path).unwrap();
        let photo = import_paths(&mut catalog, &["/f/a.jpg"])[0];
        let first = catalog
            .set_develop_settings(photo, &exposure(0.5), Some("First"))
            .unwrap();
        let second = catalog
            .set_develop_settings(photo, &exposure(1.0), Some("Second"))
            .unwrap();
        (
            photo,
            first.history_step().unwrap(),
            second.history_step().unwrap(),
        )
    };
    tamper(
        &path,
        &format!(
            "UPDATE history SET settings_format = {} WHERE id = {}",
            SETTINGS_FORMAT + 1,
            second.0
        ),
    );

    let mut catalog = Catalog::open(&path).unwrap();
    assert!(newer_settings(
        &catalog.history_settings(second).unwrap_err()
    ));
    catalog.go_to_history_step(photo, first).unwrap();
    assert!(newer_settings(
        &catalog.go_to_history_step(photo, second).unwrap_err()
    ));
    // A new step from here would replace the newer one after it.
    let steps = catalog.history(photo).unwrap();
    assert!(newer_settings(
        &catalog
            .set_develop_settings(photo, &exposure(2.0), Some("Third"))
            .unwrap_err()
    ));
    assert_eq!(catalog.history(photo).unwrap(), steps);
    // An edit that keeps the history is fine.
    catalog
        .set_develop_settings(photo, &exposure(2.0), None)
        .unwrap();
}

#[test]
fn newer_snapshots_and_presets_are_passed_over_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_file(&dir);
    let photo = {
        let mut catalog = Catalog::open(&path).unwrap();
        let photo = import_paths(&mut catalog, &["/f/a.jpg"])[0];
        for name in ["Old", "New"] {
            catalog
                .create_snapshot(photo, name, &exposure(0.5))
                .unwrap();
            catalog
                .create_preset("Mine", name, &exposure(0.5), GroupSet::all())
                .unwrap();
        }
        photo
    };
    let newer = SETTINGS_FORMAT + 1;
    tamper(
        &path,
        &format!(
            "UPDATE snapshots SET settings_format = {newer} WHERE name = 'New';
             UPDATE presets SET settings_format = {newer} WHERE name = 'New';"
        ),
    );

    let catalog = Catalog::open(&path).unwrap();
    let listed = |names: Vec<String>| names == ["Old"];
    assert!(listed(
        catalog
            .snapshots(photo)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect()
    ));
    assert!(listed(
        catalog
            .presets()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect()
    ));
    assert_eq!(catalog.stats().unwrap().snapshots, 2);
    assert_eq!(catalog.stats().unwrap().presets, 2);
}

#[test]
fn newer_smart_rules_are_reported_and_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = catalog_file(&dir);
    let rules = SmartRules {
        match_all: true,
        rules: vec![SmartRule {
            field: SmartField::Rating,
            op: SmartOp::AtLeast,
            value: 3.into(),
        }],
    };
    let smart = {
        let mut catalog = Catalog::open(&path).unwrap();
        import_paths(&mut catalog, &["/f/a.jpg"]);
        catalog
            .create_smart_collection("Smart", None, &rules)
            .unwrap()
    };
    tamper(
        &path,
        &format!(
            "UPDATE collections SET rules_format = {} WHERE id = {}",
            SmartRules::FORMAT + 1,
            smart.0
        ),
    );
    let stored = || {
        raw(
            &path,
            &format!("SELECT rules FROM collections WHERE id = {}", smart.0),
        )
    };
    let before = stored();
    let newer_rules = |error: CatalogError| {
        matches!(
            error,
            CatalogError::NewerFormat { what: "smart collection rules", found, supported }
                if found == SmartRules::FORMAT + 1 && supported == SmartRules::FORMAT
        )
    };

    let mut catalog = Catalog::open(&path).unwrap();
    assert!(newer_rules(catalog.smart_rules(smart).unwrap_err()));
    assert!(newer_rules(
        catalog.count(&Source::Collection(smart)).unwrap_err()
    ));
    assert!(newer_rules(
        catalog
            .set_smart_rules(smart, &SmartRules::default())
            .unwrap_err()
    ));
    // The panel still lists it, with nothing in it.
    let listed = catalog.collections().unwrap();
    assert_eq!(
        listed.iter().find(|c| c.id == smart).unwrap().photo_count,
        0
    );
    drop(catalog);
    assert_eq!(stored(), before);

    // Deleting it is still the user's call.
    let mut catalog = Catalog::open(&path).unwrap();
    catalog.delete_collection(smart).unwrap();
    assert!(matches!(
        catalog.smart_rules(smart),
        Err(CatalogError::NotFound {
            entity: Entity::Collection,
            ..
        })
    ));
}
