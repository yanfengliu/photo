//! Stored JSON this build cannot read (damaged, or written by a newer
//! version): reported, never replaced by defaults, and never allowed to take
//! the readable data down with it.
//!
//! The damage is done behind the catalog's back, through a second, plain
//! SQLite connection to the same file.

mod common;

use common::{import, raster};
use photo_catalog::{Catalog, CatalogError, PhotoQuery, SmartRules, Source};
use photo_core::develop::{DevelopSettings, GroupSet};
use photo_core::meta::PhotoId;
use rusqlite::Connection;

struct Fixture {
    _dir: tempfile::TempDir,
    catalog: Catalog,
    /// A second connection to the same file, for tampering.
    raw: Connection,
    photo: PhotoId,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let mut catalog = Catalog::open(&path).unwrap();
    let photo = import(&mut catalog, &[raster("/x/a.jpg"), raster("/x/b.jpg")])[0];
    let raw = Connection::open(&path).unwrap();
    Fixture {
        _dir: dir,
        catalog,
        raw,
        photo,
    }
}

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn is_corrupt<T>(result: Result<T, CatalogError>) -> bool {
    matches!(result, Err(CatalogError::Corrupt { .. }))
}

/// JSON a newer version might write: valid, but with a value this build has
/// no meaning for.
const FROM_THE_FUTURE: &str = r#"{"profile":"cinematic","tone":{"exposure":1.0}}"#;

#[test]
fn unreadable_develop_settings_are_reported_and_left_exactly_as_they_are() {
    let Fixture {
        _dir,
        mut catalog,
        raw,
        photo,
    } = fixture();
    catalog
        .set_develop_settings(photo, &exposure(0.5), None)
        .unwrap();
    let stored = |raw: &Connection| -> String {
        raw.query_row(
            "SELECT develop FROM photos WHERE id = ?1",
            [photo.0],
            |row| row.get(0),
        )
        .unwrap()
    };

    for damaged in [FROM_THE_FUTURE, "{\"tone\":", "", "null", "[1, 2, 3]"] {
        raw.execute(
            "UPDATE photos SET develop = ?1 WHERE id = ?2",
            (damaged, photo.0),
        )
        .unwrap();

        assert!(is_corrupt(catalog.develop_settings(photo)), "{damaged:?}");
        // Never the defaults, which the next save would then make permanent.
        assert_eq!(stored(&raw), damaged);
        // The rest of the photo is still there to be shown.
        assert_eq!(catalog.photo(photo).unwrap().file_name, "a.jpg");
        assert_eq!(catalog.query(&PhotoQuery::default()).unwrap().len(), 2);
    }

    // Writing new settings on purpose replaces the unreadable text, and
    // undoing that write puts the very same text back.
    let change = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("Fresh start"))
        .unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(1.0));
    catalog.undo_develop(std::slice::from_ref(&change)).unwrap();
    assert_eq!(stored(&raw), "[1, 2, 3]");
    assert!(is_corrupt(catalog.develop_settings(photo)));
}

#[test]
fn an_unreadable_history_step_cannot_be_gone_to_but_the_others_can() {
    let Fixture {
        _dir,
        mut catalog,
        raw,
        photo,
    } = fixture();
    let good = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("Good"))
        .unwrap();
    let bad = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("Bad"))
        .unwrap();
    let (good, bad) = (good.history_step().unwrap(), bad.history_step().unwrap());
    raw.execute(
        "UPDATE history SET settings = ?1 WHERE id = ?2",
        (FROM_THE_FUTURE, bad.0),
    )
    .unwrap();
    let version = catalog.photo(photo).unwrap().develop_version;

    // The list does not need the settings, so it still shows both steps.
    assert_eq!(catalog.history(photo).unwrap().len(), 2);
    assert!(is_corrupt(catalog.history_settings(bad)));
    assert!(is_corrupt(catalog.go_to_history_step(photo, bad)));
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(1.0));
    assert_eq!(catalog.photo(photo).unwrap().develop_version, version);

    catalog.go_to_history_step(photo, good).unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(0.5));
}

#[test]
fn an_unreadable_snapshot_or_preset_is_passed_over_and_kept() {
    let Fixture {
        _dir,
        mut catalog,
        raw,
        photo,
    } = fixture();
    let bad_snapshot = catalog
        .create_snapshot(photo, "Bad", &exposure(0.1))
        .unwrap();
    let good_snapshot = catalog
        .create_snapshot(photo, "Good", &exposure(0.2))
        .unwrap();
    let all = GroupSet::all();
    let bad_settings = catalog
        .create_preset("G", "Bad settings", &exposure(0.1), all)
        .unwrap();
    let bad_groups = catalog
        .create_preset("G", "Bad groups", &exposure(0.2), all)
        .unwrap();
    let good_preset = catalog
        .create_preset("G", "Good", &exposure(0.3), all)
        .unwrap();
    raw.execute(
        "UPDATE snapshots SET settings = ?1 WHERE id = ?2",
        (FROM_THE_FUTURE, bad_snapshot.0),
    )
    .unwrap();
    raw.execute(
        "UPDATE presets SET settings = ?1 WHERE id = ?2",
        (FROM_THE_FUTURE, bad_settings.0),
    )
    .unwrap();
    raw.execute(
        r#"UPDATE presets SET groups = '["tone","holograms"]' WHERE id = ?1"#,
        [bad_groups.0],
    )
    .unwrap();

    let snapshots = catalog.snapshots(photo).unwrap();
    assert_eq!(
        snapshots.iter().map(|s| s.id).collect::<Vec<_>>(),
        [good_snapshot]
    );
    let presets = catalog.presets().unwrap();
    assert_eq!(
        presets.iter().map(|p| p.id).collect::<Vec<_>>(),
        [good_preset]
    );
    assert_eq!(presets[0].settings, exposure(0.3));

    // Passed over, not thrown away: a build that can read them still will.
    let stats = catalog.stats().unwrap();
    assert_eq!((stats.snapshots, stats.presets), (2, 3));
    // And their names are still taken.
    assert!(matches!(
        catalog.create_preset("G", "Bad groups", &exposure(0.0), all),
        Err(CatalogError::NameTaken(_))
    ));
}

#[test]
fn unreadable_smart_rules_do_not_take_the_collections_panel_down() {
    let Fixture {
        _dir,
        mut catalog,
        raw,
        ..
    } = fixture();
    let future = catalog
        .create_smart_collection("From the future", None, &SmartRules::default())
        .unwrap();
    let damaged = catalog
        .create_smart_collection("Damaged", None, &SmartRules::default())
        .unwrap();
    let invalid = catalog
        .create_smart_collection("Invalid", None, &SmartRules::default())
        .unwrap();
    let healthy = catalog
        .create_smart_collection("Healthy", None, &SmartRules::default())
        .unwrap();
    let set_rules = |id: photo_core::meta::CollectionId, rules: &str| {
        raw.execute(
            "UPDATE collections SET rules = ?1 WHERE id = ?2",
            (rules, id.0),
        )
        .unwrap();
    };
    // A field this build does not know, text that is not JSON, and rules
    // that parse but pair a number field with a text operator.
    set_rules(
        future,
        r#"{"match_all":true,"rules":[{"field":"face_count","op":"at_least","value":2}]}"#,
    );
    set_rules(damaged, "{\"match_all\":tr");
    set_rules(
        invalid,
        r#"{"match_all":true,"rules":[{"field":"rating","op":"contains","value":"3"}]}"#,
    );

    let listed: Vec<(String, u64)> = catalog
        .collections()
        .unwrap()
        .into_iter()
        .map(|collection| (collection.name, collection.photo_count))
        .collect();
    assert_eq!(
        listed,
        [
            ("Damaged".to_string(), 0),
            ("From the future".to_string(), 0),
            ("Healthy".to_string(), 2),
            ("Invalid".to_string(), 0),
        ]
    );

    for id in [future, damaged] {
        assert!(is_corrupt(catalog.smart_rules(id)));
        assert!(is_corrupt(
            catalog.query(&PhotoQuery::from(Source::Collection(id)))
        ));
        assert!(is_corrupt(catalog.count(&Source::Collection(id))));
    }
    // Rules that can be read are returned, so they can be shown and fixed;
    // they are refused only when used.
    let readable = catalog.smart_rules(invalid).unwrap();
    assert!(matches!(
        readable.validate(),
        Err(CatalogError::InvalidRule(_))
    ));
    assert!(matches!(
        catalog.count(&Source::Collection(invalid)),
        Err(CatalogError::InvalidRule(_))
    ));

    // Each can be repaired by giving it new rules, or simply deleted.
    catalog
        .set_smart_rules(future, &SmartRules::default())
        .unwrap();
    assert_eq!(catalog.count(&Source::Collection(future)).unwrap(), 2);
    catalog.delete_collection(damaged).unwrap();
    catalog.delete_collection(invalid).unwrap();
    assert_eq!(catalog.collections().unwrap().len(), 2);
    let _ = healthy;
}
