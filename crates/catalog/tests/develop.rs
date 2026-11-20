//! Develop settings and history.

mod common;

use common::{import, memory, raster, raw};
use photo_catalog::{Catalog, CatalogError, DevelopEdit, Entity, HistoryStep};
use photo_core::develop::{CurvePoint, DevelopSettings, PointCurve};
use photo_core::meta::{FileKind, HistoryId, PhotoId};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn one_raster() -> (Catalog, PhotoId) {
    let mut catalog = memory();
    let id = import(&mut catalog, &[raster("/d/photo.jpg")])[0];
    (catalog, id)
}

fn labels(steps: &[HistoryStep]) -> Vec<(&str, bool)> {
    steps
        .iter()
        .map(|step| (step.label.as_str(), step.is_current))
        .collect()
}

/// Everything observable about a photo's develop state.
fn state(catalog: &Catalog, photo: PhotoId) -> (DevelopSettings, bool, Vec<HistoryStep>) {
    (
        catalog.develop_settings(photo).unwrap(),
        catalog.photo(photo).unwrap().has_edits,
        catalog.history(photo).unwrap(),
    )
}

#[test]
fn a_photo_that_was_never_edited_reads_as_the_default_for_its_kind() {
    let mut catalog = memory();
    let ids = import(&mut catalog, &[raw("/d/a.arw"), raster("/d/a.jpg")]);
    assert_eq!(
        catalog.develop_settings(ids[0]).unwrap(),
        DevelopSettings::default_for(FileKind::Raw)
    );
    assert_eq!(
        catalog.develop_settings(ids[1]).unwrap(),
        DevelopSettings::default_for(FileKind::Raster)
    );
    assert_ne!(
        catalog.develop_settings(ids[0]).unwrap(),
        catalog.develop_settings(ids[1]).unwrap()
    );
    assert!(catalog.history(ids[0]).unwrap().is_empty());
    assert!(matches!(
        catalog.develop_settings(PhotoId(404)),
        Err(CatalogError::NotFound {
            entity: Entity::Photo,
            id: 404
        })
    ));
}

#[test]
fn settings_round_trip_exactly() {
    let (mut catalog, id) = one_raster();
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 1.0 / 3.0;
    settings.presence.clarity = -17.25;
    settings.tone_curve.rgb = PointCurve::new([
        CurvePoint::new(0.0, 0.03),
        CurvePoint::new(0.4, 0.55),
        CurvePoint::new(1.0, 1.0),
    ]);
    settings.crop.angle = 1.5;

    catalog.set_develop_settings(id, &settings, None).unwrap();
    assert_eq!(catalog.develop_settings(id).unwrap(), settings);
}

#[test]
fn every_write_bumps_the_develop_version_and_stamps_the_edit_time() {
    let (mut catalog, id) = one_raster();
    let version = |catalog: &Catalog| catalog.photo(id).unwrap().develop_version;
    assert_eq!(version(&catalog), 0);
    assert_eq!(catalog.photo(id).unwrap().edited_at, None);

    catalog
        .set_develop_settings(id, &exposure(0.5), None)
        .unwrap();
    assert_eq!(version(&catalog), 1);
    let change = catalog
        .set_develop_settings(id, &exposure(1.0), Some("Exposure"))
        .unwrap();
    assert_eq!(version(&catalog), 2);
    // Writing the same settings again is still a write.
    catalog
        .set_develop_settings(id, &exposure(1.0), None)
        .unwrap();
    assert_eq!(version(&catalog), 3);
    catalog
        .go_to_history_step(id, change.history_step().unwrap())
        .unwrap();
    assert_eq!(version(&catalog), 4);
    // A rendering made for version 2 must never be mistaken for a later one,
    // so undo and redo move forward too. (The two writes in between left the
    // same settings at the same history step, so the change still fits.)
    catalog.undo_develop(std::slice::from_ref(&change)).unwrap();
    assert_eq!(version(&catalog), 5);
    catalog.redo_develop(std::slice::from_ref(&change)).unwrap();
    assert_eq!(version(&catalog), 6);

    assert!(catalog.photo(id).unwrap().edited_at.unwrap() > 1_600_000_000);
}

#[test]
fn has_edits_tracks_whether_settings_differ_from_the_default_for_the_kind() {
    let mut catalog = memory();
    let ids = import(&mut catalog, &[raw("/d/a.arw"), raster("/d/a.jpg")]);
    let (raw_photo, raster_photo) = (ids[0], ids[1]);
    let edited = |catalog: &Catalog, id| catalog.photo(id).unwrap().has_edits;

    catalog
        .set_develop_settings(raw_photo, &exposure(0.3), None)
        .unwrap();
    assert!(edited(&catalog, raw_photo));
    // Back at the RAW starting point: stored, but no longer an edit.
    catalog
        .set_develop_settings(
            raw_photo,
            &DevelopSettings::default_for(FileKind::Raw),
            None,
        )
        .unwrap();
    assert!(!edited(&catalog, raw_photo));
    // The neutral settings are an edit for RAW (capture sharpening removed)...
    catalog
        .set_develop_settings(raw_photo, &DevelopSettings::default(), None)
        .unwrap();
    assert!(edited(&catalog, raw_photo));

    // ...and exactly the starting point for a raster file.
    catalog
        .set_develop_settings(raster_photo, &DevelopSettings::default(), None)
        .unwrap();
    assert!(!edited(&catalog, raster_photo));
    catalog
        .set_develop_settings(
            raster_photo,
            &DevelopSettings::default_for(FileKind::Raw),
            None,
        )
        .unwrap();
    assert!(edited(&catalog, raster_photo));
}

#[test]
fn only_labelled_writes_add_history_steps_and_they_are_listed_oldest_first() {
    let (mut catalog, id) = one_raster();
    let first = catalog
        .set_develop_settings(id, &exposure(0.5), Some("Exposure +0.50"))
        .unwrap();
    let dragging = catalog
        .set_develop_settings(id, &exposure(0.7), None)
        .unwrap();
    let second = catalog
        .set_develop_settings(id, &exposure(1.0), Some("Exposure +1.00"))
        .unwrap();
    assert_eq!(dragging.history_step(), None);
    assert_eq!(first.photo(), id);

    let history = catalog.history(id).unwrap();
    assert_eq!(
        labels(&history),
        [("Exposure +0.50", false), ("Exposure +1.00", true)]
    );
    assert_eq!(history[0].id, first.history_step().unwrap());
    assert_eq!(history[1].id, second.history_step().unwrap());
    assert!(history[0].id < history[1].id);
    assert!(history[0].created_at > 1_600_000_000);
    assert_eq!(
        catalog.history_settings(history[0].id).unwrap(),
        exposure(0.5)
    );
    assert_eq!(
        catalog.history_settings(history[1].id).unwrap(),
        exposure(1.0)
    );
    assert!(matches!(
        catalog.history_settings(HistoryId(404)),
        Err(CatalogError::NotFound {
            entity: Entity::HistoryStep,
            id: 404
        })
    ));
}

#[test]
fn going_to_a_step_restores_its_settings_and_keeps_the_later_steps_until_the_next_edit() {
    let (mut catalog, id) = one_raster();
    for (stops, label) in [(0.5, "A"), (1.0, "B"), (1.5, "C")] {
        catalog
            .set_develop_settings(id, &exposure(stops), Some(label))
            .unwrap();
    }
    let steps = catalog.history(id).unwrap();

    let jump = catalog.go_to_history_step(id, steps[0].id).unwrap();
    assert_eq!(jump.history_step(), None, "going to a step adds none");
    assert_eq!(catalog.develop_settings(id).unwrap(), exposure(0.5));
    assert_eq!(
        labels(&catalog.history(id).unwrap()),
        [("A", true), ("B", false), ("C", false)]
    );

    // The later steps can still be returned to...
    catalog.go_to_history_step(id, steps[2].id).unwrap();
    assert_eq!(catalog.develop_settings(id).unwrap(), exposure(1.5));
    catalog.go_to_history_step(id, steps[0].id).unwrap();

    // ...until a new edit is made from the earlier state, which replaces them.
    catalog
        .set_develop_settings(id, &exposure(-1.0), Some("D"))
        .unwrap();
    assert_eq!(
        labels(&catalog.history(id).unwrap()),
        [("A", false), ("D", true)]
    );
    assert!(matches!(
        catalog.history_settings(steps[1].id),
        Err(CatalogError::NotFound { .. })
    ));
}

#[test]
fn a_step_can_only_be_gone_to_on_its_own_photo() {
    let mut catalog = memory();
    let ids = import(&mut catalog, &[raster("/d/a.jpg"), raster("/d/b.jpg")]);
    let step = catalog
        .set_develop_settings(ids[0], &exposure(1.0), Some("A"))
        .unwrap()
        .history_step()
        .unwrap();

    let error = catalog.go_to_history_step(ids[1], step).unwrap_err();
    assert!(matches!(
        error,
        CatalogError::NotFound {
            entity: Entity::HistoryStep,
            ..
        }
    ));
    assert!(!catalog.photo(ids[1]).unwrap().has_edits);
    assert_eq!(catalog.photo(ids[1]).unwrap().develop_version, 0);
}

#[test]
fn undoing_an_edit_restores_settings_and_history_exactly_and_redo_reapplies_it() {
    let (mut catalog, id) = one_raster();
    let untouched = state(&catalog, id);
    let first = catalog
        .set_develop_settings(id, &exposure(0.5), Some("A"))
        .unwrap();
    let after_first = state(&catalog, id);
    let second = catalog
        .set_develop_settings(id, &exposure(1.0), Some("B"))
        .unwrap();
    let after_second = state(&catalog, id);

    catalog.undo_develop(std::slice::from_ref(&second)).unwrap();
    assert_eq!(state(&catalog, id), after_first);
    catalog.undo_develop(std::slice::from_ref(&first)).unwrap();
    assert_eq!(state(&catalog, id), untouched);
    assert_eq!(catalog.photo(id).unwrap().edited_at, None);

    catalog.redo_develop(std::slice::from_ref(&first)).unwrap();
    assert_eq!(state(&catalog, id), after_first);
    catalog.redo_develop(std::slice::from_ref(&second)).unwrap();
    assert_eq!(
        state(&catalog, id),
        after_second,
        "redo brings back the same step ids"
    );
}

#[test]
fn undoing_an_edit_that_replaced_later_steps_brings_those_steps_back() {
    let (mut catalog, id) = one_raster();
    for (stops, label) in [(0.5, "A"), (1.0, "B"), (1.5, "C")] {
        catalog
            .set_develop_settings(id, &exposure(stops), Some(label))
            .unwrap();
    }
    let first = catalog.history(id).unwrap()[0].id;
    let jump = catalog.go_to_history_step(id, first).unwrap();
    let before_edit = state(&catalog, id);
    let edit = catalog
        .set_develop_settings(id, &exposure(-1.0), Some("D"))
        .unwrap();
    let after_edit = state(&catalog, id);
    assert_eq!(labels(&after_edit.2), [("A", false), ("D", true)]);

    catalog.undo_develop(std::slice::from_ref(&edit)).unwrap();
    assert_eq!(state(&catalog, id), before_edit);
    assert_eq!(
        labels(&before_edit.2),
        [("A", true), ("B", false), ("C", false)]
    );

    // The jump itself undoes back to the newest step being current.
    catalog.undo_develop(std::slice::from_ref(&jump)).unwrap();
    assert_eq!(catalog.develop_settings(id).unwrap(), exposure(1.5));
    assert_eq!(
        labels(&catalog.history(id).unwrap()),
        [("A", false), ("B", false), ("C", true)]
    );

    catalog.redo_develop(&[jump, edit]).unwrap();
    assert_eq!(state(&catalog, id), after_edit);
}

#[test]
fn a_batch_of_edits_is_one_transaction_and_undoes_as_one() {
    let mut catalog = memory();
    let ids = import(&mut catalog, &[raster("/d/a.jpg"), raster("/d/b.jpg")]);
    let before: Vec<_> = ids.iter().map(|id| state(&catalog, *id)).collect();
    let (base, look) = (DevelopSettings::default(), exposure(0.8));
    fn edit<'a>(photo: PhotoId, settings: &'a DevelopSettings, label: &'a str) -> DevelopEdit<'a> {
        DevelopEdit {
            photo,
            settings,
            history_label: Some(label),
        }
    }

    // Two steps for the first photo and one for the second, in one go.
    let edits = [
        edit(ids[0], &base, "Import"),
        edit(ids[0], &look, "Paste"),
        edit(ids[1], &look, "Paste"),
    ];
    let changes = catalog.apply_develop_edits(&edits).unwrap();
    assert_eq!(changes.len(), 3);
    assert_eq!(
        labels(&catalog.history(ids[0]).unwrap()),
        [("Import", false), ("Paste", true)]
    );
    assert_eq!(catalog.develop_settings(ids[1]).unwrap(), look);
    let after: Vec<_> = ids.iter().map(|id| state(&catalog, *id)).collect();

    catalog.undo_develop(&changes).unwrap();
    assert_eq!(
        ids.iter()
            .map(|id| state(&catalog, *id))
            .collect::<Vec<_>>(),
        before
    );
    catalog.redo_develop(&changes).unwrap();
    assert_eq!(
        ids.iter()
            .map(|id| state(&catalog, *id))
            .collect::<Vec<_>>(),
        after
    );

    // One bad edit and none of the batch is kept.
    let versions = |catalog: &Catalog| -> Vec<u64> {
        ids.iter()
            .map(|id| catalog.photo(*id).unwrap().develop_version)
            .collect()
    };
    let unchanged = versions(&catalog);
    let bad = [
        edit(ids[1], &base, "Reset"),
        edit(PhotoId(404), &base, "Reset"),
    ];
    assert!(matches!(
        catalog.apply_develop_edits(&bad),
        Err(CatalogError::NotFound { .. })
    ));
    assert_eq!(versions(&catalog), unchanged);
    assert_eq!(catalog.develop_settings(ids[1]).unwrap(), look);
}

#[test]
fn settings_that_could_not_be_read_back_are_refused_and_nothing_changes() {
    let (mut catalog, id) = one_raster();
    catalog
        .set_develop_settings(id, &exposure(0.5), Some("A"))
        .unwrap();
    let before = state(&catalog, id);
    let version = catalog.photo(id).unwrap().develop_version;

    let error = catalog
        .set_develop_settings(id, &exposure(f32::NAN), Some("Broken"))
        .unwrap_err();
    assert!(matches!(error, CatalogError::Unstorable { .. }));
    assert_eq!(state(&catalog, id), before);
    assert_eq!(catalog.photo(id).unwrap().develop_version, version);

    let run = catalog.begin_import().unwrap();
    let mut photo = raster("/d/broken.jpg");
    photo.develop = Some(exposure(f32::INFINITY));
    assert!(matches!(
        catalog.insert_photo(run, &photo),
        Err(CatalogError::Unstorable { .. })
    ));
    assert_eq!(catalog.find_by_path("/d/broken.jpg").unwrap(), None);
}

#[test]
fn deleting_a_photo_takes_its_history_and_snapshots_with_it() {
    let (mut catalog, id) = one_raster();
    catalog
        .set_develop_settings(id, &exposure(0.5), Some("A"))
        .unwrap();
    catalog
        .set_develop_settings(id, &exposure(1.0), Some("B"))
        .unwrap();
    catalog
        .create_snapshot(id, "Keeper", &exposure(1.0))
        .unwrap();
    let stats = catalog.stats().unwrap();
    assert_eq!((stats.history, stats.snapshots), (2, 1));

    catalog.remove_photos(&[id]).unwrap();
    let stats = catalog.stats().unwrap();
    assert_eq!((stats.photos, stats.history, stats.snapshots), (0, 0, 0));
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}
