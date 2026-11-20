//! A file-backed catalog gives back everything that was written to it.

mod common;

use common::{date, import, raster, raw};
use photo_catalog::{
    Catalog, CollectionInfo, FolderInfo, HistoryStep, ImportRun, KeywordInfo, PhotoQuery,
    PhotoRecord, PhotoSummary, Preset, SmartField, SmartOp, SmartRule, SmartRules, Snapshot, Sort,
    SortDirection, SortKey, Source, TextField,
};
use photo_core::develop::{DevelopSettings, GroupSet, SettingsGroup};
use photo_core::meta::{ColorLabel, Flag, PhotoId, Rating};

fn exposure(stops: f32) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = stops;
    settings
}

/// The develop state of one photo.
#[derive(Debug, PartialEq)]
struct Develop {
    settings: DevelopSettings,
    history: Vec<HistoryStep>,
    history_settings: Vec<DevelopSettings>,
    snapshots: Vec<Snapshot>,
}

/// Everything the public API reports about a catalog.
#[derive(Debug, PartialEq)]
struct Dump {
    records: Vec<PhotoRecord>,
    develop: Vec<Develop>,
    photo_keywords: Vec<Vec<KeywordInfo>>,
    cells: Vec<PhotoSummary>,
    folders: Vec<FolderInfo>,
    keywords: Vec<KeywordInfo>,
    collections: Vec<CollectionInfo>,
    members: Vec<Vec<PhotoId>>,
    rules: Vec<SmartRules>,
    presets: Vec<Preset>,
    imports: Vec<ImportRun>,
    settings: Vec<Option<String>>,
    stats: photo_catalog::CatalogStats,
}

fn dump(catalog: &Catalog) -> Dump {
    let by_added = PhotoQuery {
        sort: Sort {
            key: SortKey::Added,
            direction: SortDirection::Ascending,
        },
        ..PhotoQuery::default()
    };
    let cells = catalog.query(&by_added).unwrap();
    let ids: Vec<PhotoId> = cells.iter().map(|cell| cell.id).collect();
    let collections = catalog.collections().unwrap();
    let holds_photos = |c: &&CollectionInfo| c.kind == photo_catalog::CollectionKind::Regular;
    let is_smart = |c: &&CollectionInfo| c.kind == photo_catalog::CollectionKind::Smart;
    let mut members: Vec<Vec<PhotoId>> = collections
        .iter()
        .filter(holds_photos)
        .map(|c| catalog.collection_photos(c.id).unwrap())
        .collect();
    members.push(
        catalog
            .collection_photos(catalog.quick_collection())
            .unwrap(),
    );
    Dump {
        records: catalog.photos(&ids).unwrap(),
        develop: ids
            .iter()
            .map(|id| {
                let history = catalog.history(*id).unwrap();
                Develop {
                    settings: catalog.develop_settings(*id).unwrap(),
                    history_settings: history
                        .iter()
                        .map(|step| catalog.history_settings(step.id).unwrap())
                        .collect(),
                    history,
                    snapshots: catalog.snapshots(*id).unwrap(),
                }
            })
            .collect(),
        photo_keywords: ids
            .iter()
            .map(|id| catalog.photo_keywords(*id).unwrap())
            .collect(),
        folders: catalog.folders().unwrap(),
        keywords: catalog.keywords().unwrap(),
        rules: collections
            .iter()
            .filter(is_smart)
            .map(|c| catalog.smart_rules(c.id).unwrap())
            .collect(),
        members,
        collections,
        presets: catalog.presets().unwrap(),
        imports: catalog.imports().unwrap(),
        settings: ["ui.module", "missing.key"]
            .map(|key| catalog.setting(key).unwrap())
            .to_vec(),
        stats: catalog.stats().unwrap(),
        cells,
    }
}

/// Use every part of the catalog at least once.
fn populate(catalog: &mut Catalog) {
    let mut first = raw("/lib/2024/DSC_0001.ARW");
    first.file_size = 60_000_000;
    first.file_mtime = 1_720_000_000;
    first.metadata.width = 9504;
    first.metadata.height = 6336;
    first.metadata.capture_time = Some(date(2024, 7, 4));
    first.metadata.camera_make = Some("SONY".into());
    first.metadata.camera_model = Some("ILCE-7RM5".into());
    first.metadata.lens = Some("FE 50mm F1.2 GM".into());
    first.metadata.iso = Some(100);
    first.metadata.aperture = Some(1.2);
    first.rating = Rating::new(5);
    first.flag = Flag::Picked;
    first.label = ColorLabel::Blue;
    first.keywords = vec!["fireworks".into(), "night".into()];
    let ids = import(
        catalog,
        &[
            first,
            raster("/lib/2024/scan.png"),
            raster("/lib/2023/old.jpg"),
        ],
    );
    let late = import(catalog, &[raster("/lib/2025/new.jpg")]);

    catalog
        .set_text(TextField::Title, &[(ids[0], "Fourth of July")])
        .unwrap();
    catalog
        .set_text(TextField::Caption, &[(ids[0], "Long exposure")])
        .unwrap();
    catalog
        .set_text(TextField::Creator, &[(ids[1], "Scanner")])
        .unwrap();
    catalog
        .set_text(TextField::Copyright, &[(ids[1], "Public domain")])
        .unwrap();
    catalog.set_missing(&[ids[2]], true).unwrap();

    catalog
        .set_develop_settings(ids[0], &exposure(0.4), Some("Exposure"))
        .unwrap();
    catalog
        .set_develop_settings(ids[0], &exposure(0.8), Some("Exposure"))
        .unwrap();
    let first_step = catalog.history(ids[0]).unwrap()[0].id;
    catalog.go_to_history_step(ids[0], first_step).unwrap();
    catalog
        .create_snapshot(ids[0], "Keeper", &exposure(0.8))
        .unwrap();
    let copy = catalog.create_virtual_copy(ids[0]).unwrap();
    catalog
        .set_develop_settings(copy, &exposure(-0.5), Some("Darker"))
        .unwrap();

    let places = catalog.create_keyword("Places", None).unwrap();
    let boston = catalog.create_keyword("Boston", Some(places)).unwrap();
    catalog.assign_keyword(boston, &[ids[0], copy]).unwrap();

    let set = catalog.create_collection_set("Holidays", None).unwrap();
    let album = catalog.create_collection("Fourth", Some(set)).unwrap();
    catalog
        .add_to_collection(album, &[late[0], ids[0], copy])
        .unwrap();
    catalog
        .set_collection_order(album, &[copy, late[0], ids[0]])
        .unwrap();
    catalog
        .add_to_collection(catalog.quick_collection(), &[ids[1]])
        .unwrap();
    let rules = SmartRules {
        match_all: false,
        rules: vec![
            SmartRule {
                field: SmartField::Rating,
                op: SmartOp::AtLeast,
                value: 4.into(),
            },
            SmartRule {
                field: SmartField::Keyword,
                op: SmartOp::Contains,
                value: "night".into(),
            },
        ],
    };
    catalog
        .create_smart_collection("Best or night", None, &rules)
        .unwrap();

    let groups = GroupSet::NONE
        .with(SettingsGroup::Tone)
        .with(SettingsGroup::Presence);
    catalog
        .create_preset("Mine", "Bright", &exposure(1.0), groups)
        .unwrap();
    catalog.set_setting("ui.module", "develop").unwrap();
}

#[test]
fn reopening_a_file_backed_catalog_returns_everything_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");

    let written = {
        let mut catalog = Catalog::open(&path).unwrap();
        populate(&mut catalog);
        dump(&catalog)
    };
    // Guard against a dump that is trivially equal because it is empty.
    assert_eq!(written.records.len(), 5);
    assert_eq!(written.develop[0].history.len(), 2);
    assert_eq!(written.develop[0].snapshots.len(), 1);
    assert_eq!(written.collections.len(), 3);
    assert_eq!(
        written.members.iter().map(Vec::len).collect::<Vec<_>>(),
        [3, 1]
    );
    assert_eq!(written.rules.len(), 1);
    assert_eq!(
        (
            written.presets.len(),
            written.imports.len(),
            written.keywords.len()
        ),
        (1, 2, 4)
    );
    assert!(
        written
            .cells
            .iter()
            .any(|cell| cell.is_virtual_copy() && cell.has_edits)
    );

    let reopened = Catalog::open(&path).unwrap();
    assert_eq!(dump(&reopened), written);
    assert_eq!(reopened.check_integrity().unwrap(), Vec::<String>::new());
    assert_eq!(reopened.count(&Source::PreviousImport).unwrap(), 1);
}

#[test]
fn edits_made_after_reopening_continue_where_the_last_session_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let (photo, version, step, edited_at) = {
        let mut catalog = Catalog::open(&path).unwrap();
        let photo = import(&mut catalog, &[raster("/lib/a.jpg"), raster("/lib/b.jpg")])[0];
        let change = catalog
            .set_develop_settings(photo, &exposure(0.4), Some("Exposure"))
            .unwrap();
        let record = catalog.photo(photo).unwrap();
        (
            photo,
            record.develop_version,
            change.history_step().unwrap(),
            record.edited_at.unwrap(),
        )
    };

    let mut catalog = Catalog::open(&path).unwrap();
    let by_edit_time = PhotoQuery {
        sort: Sort {
            key: SortKey::EditTime,
            direction: SortDirection::Ascending,
        },
        ..PhotoQuery::default()
    };
    let other = catalog.find_by_path("/lib/b.jpg").unwrap().unwrap();
    assert_eq!(
        catalog.query_ids(&by_edit_time).unwrap(),
        [other, photo],
        "never edited sorts first"
    );

    // An edit in this session is later than any edit of the last one.
    let change = catalog
        .set_develop_settings(other, &exposure(0.1), Some("Exposure"))
        .unwrap();
    assert_eq!(catalog.query_ids(&by_edit_time).unwrap(), [photo, other]);
    assert!(
        change.history_step().unwrap() > step,
        "history ids keep counting up across sessions"
    );

    // Develop versions come from one counter for the whole catalog, and it
    // carried over from the last session too.
    let latest = catalog.photo(other).unwrap().develop_version;
    assert!(latest > version);
    catalog
        .set_develop_settings(photo, &exposure(0.9), None)
        .unwrap();
    assert_eq!(catalog.query_ids(&by_edit_time).unwrap(), [other, photo]);
    assert_eq!(catalog.photo(photo).unwrap().develop_version, latest + 1);
    assert!(catalog.photo(photo).unwrap().edited_at.unwrap() >= edited_at);
    assert!(catalog.begin_import().unwrap().0 > 1);
}

#[test]
fn a_mutation_that_fails_leaves_the_file_backed_catalog_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.photocat");
    let mut catalog = Catalog::open(&path).unwrap();
    populate(&mut catalog);
    let before = dump(&catalog);

    let run = catalog.begin_import().unwrap();
    assert!(
        catalog
            .insert_photos(run, &[raster("/lib/x/1.jpg"), raster("/lib/2024/scan.png")])
            .is_err()
    );
    assert!(catalog.remove_photos(&[PhotoId(1), PhotoId(404)]).is_err());
    assert!(
        catalog
            .set_ratings(&[(PhotoId(1), Rating::new(1)), (PhotoId(404), Rating::new(1))])
            .is_err()
    );
    assert!(
        catalog
            .set_develop_settings(PhotoId(1), &exposure(f32::NAN), Some("NaN"))
            .is_err()
    );

    drop(catalog);
    let reopened = Catalog::open(&path).unwrap();
    let mut after = dump(&reopened);
    // The import run that was begun is the one thing that did happen.
    assert_eq!(after.imports.len(), before.imports.len() + 1);
    after.imports.remove(0);
    after.stats.imports -= 1;
    assert_eq!(after, before);
}
