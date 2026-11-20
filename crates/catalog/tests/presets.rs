//! Snapshots (named settings of one photo) and presets (named settings for
//! any photo).

mod common;

use common::{import_paths, memory};
use photo_catalog::{CatalogError, Entity, Preset, Snapshot};
use photo_core::develop::{DevelopSettings, GroupSet, SettingsGroup};
use photo_core::meta::{PhotoId, PresetId, SnapshotId};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn snapshot_names(snapshots: &[Snapshot]) -> Vec<&str> {
    snapshots
        .iter()
        .map(|snapshot| snapshot.name.as_str())
        .collect()
}

fn preset_names(presets: &[Preset]) -> Vec<(&str, &str)> {
    presets
        .iter()
        .map(|preset| (preset.group.as_str(), preset.name.as_str()))
        .collect()
}

#[test]
fn snapshots_belong_to_one_photo_and_keep_the_settings_they_were_given() {
    let mut catalog = memory();
    let ids = import_paths(&mut catalog, &["/s/a.jpg", "/s/b.jpg"]);
    let first = catalog
        .create_snapshot(ids[0], "  Before crop ", &exposure(0.5))
        .unwrap();
    let second = catalog
        .create_snapshot(ids[0], "Final", &exposure(1.0))
        .unwrap();
    catalog
        .create_snapshot(ids[1], "Other photo", &exposure(-1.0))
        .unwrap();

    let snapshots = catalog.snapshots(ids[0]).unwrap();
    assert_eq!(snapshot_names(&snapshots), ["Before crop", "Final"]);
    assert_eq!(
        snapshots
            .iter()
            .map(|snapshot| snapshot.id)
            .collect::<Vec<_>>(),
        [first, second]
    );
    assert_eq!(snapshots[0].settings, exposure(0.5));
    assert_eq!(snapshots[1].settings, exposure(1.0));
    assert!(
        snapshots
            .iter()
            .all(|snapshot| snapshot.photo == ids[0] && snapshot.created_at > 1_600_000_000)
    );

    // A snapshot is a copy: later edits to the photo do not reach it, and
    // taking one does not change the photo.
    catalog
        .set_develop_settings(ids[0], &exposure(3.0), None)
        .unwrap();
    assert_eq!(
        catalog.snapshots(ids[0]).unwrap()[1].settings,
        exposure(1.0)
    );
    assert_eq!(catalog.photo(ids[1]).unwrap().develop_version, 0);
}

#[test]
fn snapshots_can_be_renamed_and_deleted() {
    let mut catalog = memory();
    let id = import_paths(&mut catalog, &["/s/a.jpg"])[0];
    let first = catalog.create_snapshot(id, "One", &exposure(0.5)).unwrap();
    let second = catalog.create_snapshot(id, "Two", &exposure(1.0)).unwrap();

    catalog.rename_snapshot(first, "Uno").unwrap();
    // Unlike presets, two snapshots may share a name.
    catalog.rename_snapshot(second, "Uno").unwrap();
    assert_eq!(
        snapshot_names(&catalog.snapshots(id).unwrap()),
        ["Uno", "Uno"]
    );

    catalog.delete_snapshot(first).unwrap();
    assert_eq!(catalog.snapshots(id).unwrap().len(), 1);
    assert_eq!(catalog.snapshots(id).unwrap()[0].id, second);
}

#[test]
fn snapshot_errors_are_typed() {
    let mut catalog = memory();
    let id = import_paths(&mut catalog, &["/s/a.jpg"])[0];
    assert!(matches!(
        catalog.create_snapshot(id, " ", &exposure(0.0)),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.create_snapshot(PhotoId(404), "x", &exposure(0.0)),
        Err(CatalogError::NotFound {
            entity: Entity::Photo,
            ..
        })
    ));
    assert!(matches!(
        catalog.snapshots(PhotoId(404)),
        Err(CatalogError::NotFound { .. })
    ));
    for result in [
        catalog.rename_snapshot(SnapshotId(404), "x"),
        catalog.delete_snapshot(SnapshotId(404)),
    ] {
        assert!(matches!(
            result,
            Err(CatalogError::NotFound {
                entity: Entity::Snapshot,
                id: 404
            })
        ));
    }
    assert!(matches!(
        catalog.create_snapshot(id, "NaN", &exposure(f32::NAN)),
        Err(CatalogError::Unstorable { .. })
    ));
    assert_eq!(catalog.stats().unwrap().snapshots, 0);
}

#[test]
fn presets_keep_their_settings_and_the_groups_they_apply() {
    let mut catalog = memory();
    let tone_only = GroupSet::NONE
        .with(SettingsGroup::Tone)
        .with(SettingsGroup::ToneCurve);
    let id = catalog
        .create_preset("Portraits", "Soft light", &exposure(0.3), tone_only)
        .unwrap();
    catalog
        .create_preset("Portraits", "Everything", &exposure(-0.3), GroupSet::all())
        .unwrap();
    catalog
        .create_preset("B&W", "Nothing", &exposure(0.0), GroupSet::NONE)
        .unwrap();

    let presets = catalog.presets().unwrap();
    assert_eq!(
        preset_names(&presets),
        [
            ("B&W", "Nothing"),
            ("Portraits", "Everything"),
            ("Portraits", "Soft light")
        ]
    );
    let soft = presets.iter().find(|preset| preset.id == id).unwrap();
    assert_eq!(soft.settings, exposure(0.3));
    assert_eq!(soft.groups, tone_only);
    assert_eq!(presets[1].groups, GroupSet::all());
    assert!(presets[0].groups.is_empty());
}

#[test]
fn presets_are_listed_in_natural_order_by_group_then_name() {
    let mut catalog = memory();
    for (group, name) in [
        ("user", "Look 10"),
        ("User", "look 9"),
        ("Film", "b"),
        ("film 2", "a"),
    ] {
        catalog
            .create_preset(group, name, &exposure(0.0), GroupSet::all())
            .unwrap();
    }
    assert_eq!(
        preset_names(&catalog.presets().unwrap()),
        [
            ("Film", "b"),
            ("film 2", "a"),
            ("User", "look 9"),
            ("user", "Look 10")
        ]
    );
}

#[test]
fn preset_names_are_unique_within_a_group_ignoring_case() {
    let mut catalog = memory();
    let id = catalog
        .create_preset("Mine", "Punchy", &exposure(0.5), GroupSet::all())
        .unwrap();
    let clash = catalog
        .create_preset("MINE", "punchy", &exposure(0.1), GroupSet::all())
        .unwrap_err();
    assert!(matches!(clash, CatalogError::NameTaken(name) if name == "punchy"));
    catalog
        .create_preset("Other", "Punchy", &exposure(0.1), GroupSet::all())
        .unwrap();

    // Renaming can move a preset to another group, but not onto another preset.
    assert!(matches!(
        catalog.rename_preset(id, "other", "PUNCHY"),
        Err(CatalogError::NameTaken(_))
    ));
    catalog.rename_preset(id, " Other ", " Punchier ").unwrap();
    catalog.rename_preset(id, "Other", "PUNCHIER").unwrap();
    assert_eq!(
        preset_names(&catalog.presets().unwrap()),
        [("Other", "PUNCHIER"), ("Other", "Punchy")]
    );
    assert_eq!(catalog.presets().unwrap()[0].settings, exposure(0.5));
}

#[test]
fn presets_can_be_deleted_and_errors_are_typed() {
    let mut catalog = memory();
    let id = catalog
        .create_preset("Mine", "Punchy", &exposure(0.5), GroupSet::all())
        .unwrap();
    assert!(matches!(
        catalog.create_preset("Mine", "", &exposure(0.0), GroupSet::all()),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.create_preset(" ", "x", &exposure(0.0), GroupSet::all()),
        Err(CatalogError::EmptyName)
    ));
    assert!(matches!(
        catalog.create_preset("Mine", "NaN", &exposure(f32::NAN), GroupSet::all()),
        Err(CatalogError::Unstorable { .. })
    ));

    catalog.delete_preset(id).unwrap();
    assert!(catalog.presets().unwrap().is_empty());
    for result in [
        catalog.delete_preset(id),
        catalog.rename_preset(PresetId(404), "a", "b"),
    ] {
        assert!(matches!(
            result,
            Err(CatalogError::NotFound {
                entity: Entity::Preset,
                ..
            })
        ));
    }
}
