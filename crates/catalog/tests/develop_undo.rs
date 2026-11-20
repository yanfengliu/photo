//! Develop undo and redo in the corners: order, unlabelled edits, virtual
//! copies, edit times, and settings with every kind of content.

mod common;

use common::{import, import_paths, memory, raster, raw};
use photo_catalog::{Catalog, CatalogError, PhotoQuery, Sort, SortDirection, SortKey};
use photo_core::develop::{
    BrushStroke, CropConstraint, CropRect, CurvePoint, DevelopSettings, Mask, MaskComponent,
    MaskShape, Orientation, PointCurve, Spot, SpotMode,
};
use photo_core::geom::Point;
use photo_core::meta::{FileKind, PhotoId};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

fn one(catalog: &mut Catalog) -> PhotoId {
    import_paths(catalog, &["/u/photo.jpg"])[0]
}

fn labels(catalog: &Catalog, photo: PhotoId) -> Vec<(String, bool)> {
    let steps = catalog.history(photo).unwrap();
    steps
        .into_iter()
        .map(|step| (step.label, step.is_current))
        .collect()
}

fn by_edit_time(catalog: &Catalog) -> Vec<String> {
    let query = PhotoQuery {
        sort: Sort {
            key: SortKey::EditTime,
            direction: SortDirection::Ascending,
        },
        ..PhotoQuery::default()
    };
    common::names(catalog, &query)
}

/// Settings that use every kind of content the settings can hold: scalars,
/// arrays, curves, an optional crop with a ratio, spots, and masks with
/// nested shapes.
fn elaborate() -> DevelopSettings {
    let mut settings = DevelopSettings::default_for(FileKind::Raw);
    settings.tone.exposure = -1.0 / 3.0;
    settings.tone_curve.rgb = PointCurve::new([
        CurvePoint::new(0.0, 0.02),
        CurvePoint::new(0.35, 0.3),
        CurvePoint::new(1.0, 0.98),
    ]);
    settings.color_mixer.hue[3] = -17.5;
    settings.crop.rect = Some(CropRect {
        cx: 0.45,
        cy: 0.55,
        width: 0.6,
        height: 0.4,
    });
    settings.crop.angle = -2.25;
    settings.crop.constraint = CropConstraint::Ratio {
        width: 16.0,
        height: 9.0,
    };
    settings.orientation = Orientation::new(3, true);
    settings.spots.push(Spot {
        mode: SpotMode::Clone,
        target: Point::new(0.25, 0.75),
        source: Point::new(0.3, 0.7),
        radius: 0.02,
        feather: 35.0,
        opacity: 80.0,
    });
    let mut sky = Mask::from_shape(
        MaskShape::Linear {
            start: Point::new(0.5, 0.0),
            end: Point::new(0.5, 0.4),
        },
        1,
    );
    sky.name = "Sky \"north\" \u{2601}".into();
    sky.adjust.exposure = -0.7;
    sky.components.push(MaskComponent {
        shape: MaskShape::Brush {
            strokes: vec![BrushStroke {
                points: vec![
                    Point::new(0.1, 0.1),
                    Point::new(0.15, 0.12),
                    Point::new(0.2, 0.2),
                ],
                radius: 0.05,
                feather: 60.0,
                flow: 75.0,
                erase: true,
            }],
        },
        subtract: true,
    });
    settings.masks.push(sky);
    settings
}

#[test]
fn settings_with_every_kind_of_content_round_trip_through_every_store() {
    let mut catalog = memory();
    let photo = import(&mut catalog, &[raw("/u/a.arw")])[0];
    let settings = elaborate();

    let change = catalog
        .set_develop_settings(photo, &settings, Some("Everything"))
        .unwrap();
    let snapshot = catalog.create_snapshot(photo, "All", &settings).unwrap();
    catalog
        .create_preset(
            "Mine",
            "All",
            &settings,
            photo_core::develop::GroupSet::all(),
        )
        .unwrap();
    let copy = catalog.create_virtual_copy(photo).unwrap();

    assert_eq!(catalog.develop_settings(photo).unwrap(), settings);
    assert_eq!(
        catalog
            .history_settings(change.history_step().unwrap())
            .unwrap(),
        settings
    );
    let snapshots = catalog.snapshots(photo).unwrap();
    assert_eq!(
        (snapshots[0].id, &snapshots[0].settings),
        (snapshot, &settings)
    );
    assert_eq!(catalog.presets().unwrap()[0].settings, settings);
    assert_eq!(catalog.develop_settings(copy).unwrap(), settings);

    // Imported with these settings, too.
    let mut imported = raw("/u/b.arw");
    imported.develop = Some(settings.clone());
    let imported = import(&mut catalog, &[imported])[0];
    assert_eq!(catalog.develop_settings(imported).unwrap(), settings);
}

#[test]
fn a_bad_number_anywhere_in_the_settings_is_refused() {
    let mut catalog = memory();
    let photo = one(&mut catalog);
    let mut deep = elaborate();
    deep.masks[0].adjust.clarity = f32::NAN;
    let mut curve = elaborate();
    curve.spots[0].radius = f32::NEG_INFINITY;
    for bad in [deep, curve] {
        assert!(matches!(
            catalog.set_develop_settings(photo, &bad, Some("Bad")),
            Err(CatalogError::Unstorable { .. })
        ));
    }
    assert_eq!(catalog.photo(photo).unwrap().develop_version, 0);
    assert!(catalog.history(photo).unwrap().is_empty());
}

#[test]
fn going_to_a_step_recomputes_whether_the_photo_has_edits() {
    let mut catalog = memory();
    let photo = one(&mut catalog);
    // The first step records the untouched state, the second an edit.
    let untouched = catalog
        .set_develop_settings(photo, &DevelopSettings::default(), Some("Import"))
        .unwrap();
    let edited = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("Exposure"))
        .unwrap();
    let has_edits = |catalog: &Catalog| catalog.photo(photo).unwrap().has_edits;
    assert!(has_edits(&catalog));

    let back = catalog
        .go_to_history_step(photo, untouched.history_step().unwrap())
        .unwrap();
    assert!(!has_edits(&catalog), "the default settings are not an edit");
    let forward = catalog
        .go_to_history_step(photo, edited.history_step().unwrap())
        .unwrap();
    assert!(has_edits(&catalog));

    // Undoing the jumps restores the flag along with the settings.
    catalog
        .undo_develop(std::slice::from_ref(&forward))
        .unwrap();
    assert!(!has_edits(&catalog));
    catalog.undo_develop(std::slice::from_ref(&back)).unwrap();
    assert!(has_edits(&catalog));
}

#[test]
fn changes_can_only_be_undone_and_redone_in_order() {
    let mut catalog = memory();
    let photo = one(&mut catalog);
    let first = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    let second = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("B"))
        .unwrap();
    let version = catalog.photo(photo).unwrap().develop_version;
    let stale = |result: Result<(), CatalogError>| {
        assert!(matches!(result, Err(CatalogError::StaleChange(id)) if id == photo));
    };

    // Undoing the older change while the newer one is still applied would
    // leave step B in the history with settings from before step A.
    stale(catalog.undo_develop(std::slice::from_ref(&first)));
    // A change that is applied cannot be applied again.
    stale(catalog.redo_develop(std::slice::from_ref(&second)));
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(1.0));
    assert_eq!(
        labels(&catalog, photo),
        [("A".to_string(), false), ("B".to_string(), true)]
    );
    assert_eq!(
        catalog.photo(photo).unwrap().develop_version,
        version,
        "nothing happened"
    );

    // In order, everything works; and a list is all or nothing.
    catalog
        .undo_develop(&[first.clone(), second.clone()])
        .unwrap();
    stale(catalog.undo_develop(std::slice::from_ref(&first)));
    stale(catalog.redo_develop(&[second.clone(), first.clone()]));
    assert!(catalog.history(photo).unwrap().is_empty());
    catalog.redo_develop(&[first, second]).unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(1.0));
}

#[test]
fn a_jump_back_through_the_history_does_not_let_an_older_change_be_undone() {
    let mut catalog = memory();
    let photo = one(&mut catalog);
    let first = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    catalog
        .set_develop_settings(photo, &exposure(1.0), Some("B"))
        .unwrap();

    // Back at step A the settings and the current step are exactly what the
    // first change left behind, but step B is still in the history: undoing
    // the first change now would leave B behind with no step before it.
    catalog
        .go_to_history_step(photo, first.history_step().unwrap())
        .unwrap();
    let version = catalog.photo(photo).unwrap().develop_version;
    assert!(matches!(
        catalog.undo_develop(std::slice::from_ref(&first)),
        Err(CatalogError::StaleChange(id)) if id == photo
    ));
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(0.5));
    assert_eq!(
        labels(&catalog, photo),
        [("A".to_string(), true), ("B".to_string(), false)]
    );
    assert_eq!(catalog.photo(photo).unwrap().develop_version, version);
}

#[test]
fn a_jump_back_through_the_history_does_not_let_an_abandoned_change_be_redone() {
    let mut catalog = memory();
    let photo = one(&mut catalog);
    let first = catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    let abandoned = catalog
        .set_develop_settings(photo, &exposure(1.0), Some("B"))
        .unwrap();
    catalog
        .undo_develop(std::slice::from_ref(&abandoned))
        .unwrap();
    // A different edit takes the place of the undone one.
    catalog
        .set_develop_settings(photo, &exposure(1.5), Some("C"))
        .unwrap();

    // Back at step A everything looks as it did before B was first made,
    // except that C now follows A: redoing B would splice it in before C.
    catalog
        .go_to_history_step(photo, first.history_step().unwrap())
        .unwrap();
    assert!(matches!(
        catalog.redo_develop(std::slice::from_ref(&abandoned)),
        Err(CatalogError::StaleChange(id)) if id == photo
    ));
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(0.5));
    assert_eq!(
        labels(&catalog, photo),
        [("A".to_string(), true), ("C".to_string(), false)]
    );
}

#[test]
fn an_unlabelled_edit_undoes_and_redoes_without_touching_the_history() {
    let mut catalog = memory();
    let photo = one(&mut catalog);
    catalog
        .set_develop_settings(photo, &exposure(0.5), Some("A"))
        .unwrap();
    let history = catalog.history(photo).unwrap();
    let drag = catalog
        .set_develop_settings(photo, &exposure(0.8), None)
        .unwrap();
    assert_eq!(catalog.history(photo).unwrap(), history);

    catalog.undo_develop(std::slice::from_ref(&drag)).unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(0.5));
    assert_eq!(catalog.history(photo).unwrap(), history);
    catalog.redo_develop(std::slice::from_ref(&drag)).unwrap();
    assert_eq!(catalog.develop_settings(photo).unwrap(), exposure(0.8));
    assert_eq!(catalog.history(photo).unwrap(), history);
}

#[test]
fn undoing_an_edit_also_takes_back_its_place_in_edit_time_order() {
    let mut catalog = memory();
    let ids = import(
        &mut catalog,
        &[raster("/t/a.jpg"), raster("/t/b.jpg"), raster("/t/c.jpg")],
    );
    catalog
        .set_develop_settings(ids[0], &exposure(0.1), None)
        .unwrap();
    catalog
        .set_develop_settings(ids[1], &exposure(0.1), None)
        .unwrap();
    assert_eq!(by_edit_time(&catalog), ["c.jpg", "a.jpg", "b.jpg"]);

    // Editing a again makes it the most recent; undoing puts it back.
    let again = catalog
        .set_develop_settings(ids[0], &exposure(0.2), None)
        .unwrap();
    assert_eq!(by_edit_time(&catalog), ["c.jpg", "b.jpg", "a.jpg"]);
    catalog.undo_develop(std::slice::from_ref(&again)).unwrap();
    assert_eq!(by_edit_time(&catalog), ["c.jpg", "a.jpg", "b.jpg"]);
    catalog.redo_develop(std::slice::from_ref(&again)).unwrap();
    assert_eq!(by_edit_time(&catalog), ["c.jpg", "b.jpg", "a.jpg"]);

    // A first edit undone leaves the photo never edited.
    let first = catalog
        .set_develop_settings(ids[2], &exposure(0.3), None)
        .unwrap();
    assert_eq!(by_edit_time(&catalog), ["b.jpg", "a.jpg", "c.jpg"]);
    catalog.undo_develop(std::slice::from_ref(&first)).unwrap();
    assert_eq!(by_edit_time(&catalog), ["c.jpg", "b.jpg", "a.jpg"]);
}

#[test]
fn a_virtual_copy_has_its_own_undo_history() {
    let mut catalog = memory();
    let master = one(&mut catalog);
    let on_master = catalog
        .set_develop_settings(master, &exposure(0.5), Some("Master edit"))
        .unwrap();
    let copy = catalog.create_virtual_copy(master).unwrap();
    let on_copy = catalog
        .set_develop_settings(copy, &exposure(-1.0), Some("Copy edit"))
        .unwrap();

    // Each change only fits the photo it was made on.
    catalog
        .undo_develop(std::slice::from_ref(&on_copy))
        .unwrap();
    assert_eq!(
        catalog.develop_settings(copy).unwrap(),
        exposure(0.5),
        "back to what it was copied with"
    );
    assert!(catalog.history(copy).unwrap().is_empty());
    assert_eq!(catalog.develop_settings(master).unwrap(), exposure(0.5));
    assert_eq!(
        labels(&catalog, master),
        [("Master edit".to_string(), true)]
    );

    catalog
        .undo_develop(std::slice::from_ref(&on_master))
        .unwrap();
    assert!(!catalog.photo(master).unwrap().has_edits);
    assert_eq!(catalog.develop_settings(copy).unwrap(), exposure(0.5));
    assert!(catalog.photo(copy).unwrap().has_edits);

    catalog.redo_develop(&[on_master, on_copy]).unwrap();
    assert_eq!(catalog.develop_settings(master).unwrap(), exposure(0.5));
    assert_eq!(catalog.develop_settings(copy).unwrap(), exposure(-1.0));
}
