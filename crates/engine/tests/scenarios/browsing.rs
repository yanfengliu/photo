//! Sources, filters, sorts, facets and the panel lists.

use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;
use photo_engine::{
    Comparison, CopyFilter, DateRange, Facet, Filter, IsoRange, RatingFilter, SmartField, SmartOp,
    SmartRule, SmartRules, Sort, SortDirection, SortKey, Source, Target, TextField,
};
use photo_testkit::TempLibrary;

use crate::common::Studio;

/// The fixture library, imported and marked up:
/// dawn 3 stars and "coast", ridge 5 stars and rotated, harbour rejected
/// and "coast", fireworks picked, lake red, meadow titled; then a virtual
/// copy of dawn. Shown as all photos.
fn marked_up() -> (TempLibrary, Studio) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let id = |name: &str| studio.id(name);
    let (dawn, ridge, harbour) = (id("dawn.jpg"), id("ridge.jpg"), id("harbour.dng"));
    let (fireworks, lake, meadow) = (id("fireworks.jpg"), id("lake.png"), id("meadow.tif"));
    let engine = &mut studio.engine;
    engine
        .set_rating(Target::Photos(vec![dawn]), Rating::new(3))
        .unwrap();
    engine
        .set_rating(Target::Photos(vec![ridge]), Rating::new(5))
        .unwrap();
    engine.rotate_right(Target::Photos(vec![ridge])).unwrap();
    engine
        .set_flag(Target::Photos(vec![harbour]), Flag::Rejected)
        .unwrap();
    engine
        .set_flag(Target::Photos(vec![fireworks]), Flag::Picked)
        .unwrap();
    engine
        .toggle_label(Target::Photos(vec![lake]), ColorLabel::Red)
        .unwrap();
    engine
        .set_text(
            Target::Photos(vec![meadow]),
            TextField::Title,
            "Green meadow",
        )
        .unwrap();
    engine
        .add_keywords(Target::Photos(vec![dawn, harbour]), &["coast"])
        .unwrap();
    engine
        .create_virtual_copies(Target::Photos(vec![dawn]))
        .unwrap();
    engine.set_source(Source::AllPhotos).unwrap();
    (library, studio)
}

fn filtered(studio: &mut Studio, filter: Filter) -> Vec<String> {
    studio.engine.set_filter(filter).unwrap();
    let mut names = studio.names();
    names.sort();
    names
}

#[test]
fn every_filter_and_sort_the_catalog_offers_is_reachable() {
    let (_library, mut studio) = marked_up();
    assert_eq!(studio.engine.photos().len(), 7);
    let filter = |change: fn(&mut Filter)| {
        let mut filter = Filter::default();
        change(&mut filter);
        filter
    };

    let cases: Vec<(Filter, Vec<&str>)> = vec![
        (filter(|f| f.text = "meadow".into()), vec!["meadow.tif"]),
        (
            filter(|f| f.text = "COAST".into()),
            vec!["dawn.jpg", "dawn.jpg", "harbour.dng"],
        ),
        (
            filter(|f| f.flags = vec![Flag::Picked]),
            vec!["fireworks.jpg"],
        ),
        (
            filter(|f| {
                f.rating = Some(RatingFilter {
                    comparison: Comparison::AtLeast,
                    rating: Rating::new(4),
                })
            }),
            vec!["ridge.jpg"],
        ),
        (
            filter(|f| f.labels = vec![ColorLabel::Red]),
            vec!["lake.png"],
        ),
        (
            filter(|f| f.copies = Some(CopyFilter::VirtualCopies)),
            vec!["dawn.jpg"],
        ),
        (
            filter(|f| f.kind = Some(FileKind::Raw)),
            vec!["harbour.dng"],
        ),
        (filter(|f| f.edited = Some(true)), vec!["ridge.jpg"]),
        (
            filter(|f| {
                f.capture = Some(DateRange {
                    from: DateTime::new(2024, 7, 1, 0, 0, 0),
                    to: None,
                })
            }),
            vec!["fireworks.jpg"],
        ),
    ];
    for (filter, expected) in cases {
        assert_eq!(
            filtered(&mut studio, filter.clone()),
            expected,
            "{filter:?}"
        );
    }

    // The metadata facets list what the photos hold, and each value filters
    // to exactly its count.
    studio.engine.set_filter(Filter::default()).unwrap();
    let cameras = studio.engine.facet_values(Facet::Camera).unwrap();
    let camera = cameras
        .iter()
        .find(|value| !value.value.is_empty())
        .unwrap()
        .clone();
    let lenses = studio.engine.facet_values(Facet::Lens).unwrap();
    let lens = lenses
        .iter()
        .find(|value| !value.value.is_empty())
        .unwrap()
        .clone();
    let isos = studio.engine.facet_values(Facet::Iso).unwrap();
    let iso = isos
        .iter()
        .find(|value| !value.value.is_empty())
        .unwrap()
        .clone();
    let days = studio.engine.facet_values(Facet::CaptureDate).unwrap();
    assert!(
        days.iter()
            .any(|value| value.value == "2024-07-04" && value.count == 1)
    );
    let cameras_filter = Filter {
        cameras: vec![camera.value.clone()],
        ..Filter::default()
    };
    assert_eq!(
        filtered(&mut studio, cameras_filter).len() as u64,
        camera.count
    );
    let lens_filter = Filter {
        lenses: vec![lens.value.clone()],
        ..Filter::default()
    };
    assert_eq!(filtered(&mut studio, lens_filter).len() as u64, lens.count);
    let iso_value: u32 = iso.value.parse().unwrap();
    let iso_filter = Filter {
        iso: Some(IsoRange {
            min: Some(iso_value),
            max: Some(iso_value),
        }),
        ..Filter::default()
    };
    assert_eq!(filtered(&mut studio, iso_filter).len() as u64, iso.count);
    // A facet's own condition does not narrow its own column.
    assert_eq!(studio.engine.facet_values(Facet::Iso).unwrap(), isos);

    studio.engine.set_filter(Filter::default()).unwrap();
    let keys = [
        SortKey::CaptureTime,
        SortKey::Added,
        SortKey::FileName,
        SortKey::Rating,
        SortKey::Pick,
        SortKey::Label,
        SortKey::EditTime,
        SortKey::FileType,
        SortKey::Custom,
    ];
    for key in keys {
        let sort = |direction| Sort { key, direction };
        studio
            .engine
            .set_sort(sort(SortDirection::Ascending))
            .unwrap();
        let ascending = studio.ids();
        studio
            .engine
            .set_sort(sort(SortDirection::Descending))
            .unwrap();
        let mut descending = studio.ids();
        descending.reverse();
        assert_eq!(ascending, descending, "{key:?}");
        assert_eq!(ascending.len(), 7);
    }
    studio
        .engine
        .set_sort(Sort {
            key: SortKey::Rating,
            direction: SortDirection::Descending,
        })
        .unwrap();
    assert_eq!(studio.names()[0], "ridge.jpg");
    studio
        .engine
        .set_sort(Sort {
            key: SortKey::FileName,
            direction: SortDirection::Ascending,
        })
        .unwrap();
    assert_eq!(studio.names()[..2], ["dawn.jpg", "dawn.jpg"]);
    assert_eq!(studio.engine.sort().key, SortKey::FileName);
}

#[test]
fn every_source_is_reachable_and_an_unknown_one_is_refused() {
    let (_library, mut studio) = marked_up();
    let folder = studio.engine.folders()[1].id;
    let coast = studio.engine.keywords()[0].id;
    let sources = [
        (Source::AllPhotos, 7),
        (Source::PreviousImport, 7),
        (Source::QuickCollection, 0),
        (
            Source::Folder {
                id: folder,
                include_subfolders: false,
            },
            3,
        ),
        (Source::Keyword(coast), 3),
    ];
    for (source, count) in sources {
        studio.engine.set_source(source).unwrap();
        assert_eq!(studio.engine.photos().len(), count, "{source:?}");
        assert_eq!(studio.engine.source(), &source);
    }
    let unknown = Source::Collection(photo_core::meta::CollectionId(999));
    assert!(studio.engine.set_source(unknown).is_err());
    assert_eq!(
        studio.engine.source(),
        &Source::Keyword(coast),
        "the list stays as it was"
    );
}

#[test]
fn folder_collection_and_keyword_counts_update_after_changes() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let dawn = studio.id("dawn.jpg");
    let lake = studio.id("lake.png");
    let engine = &mut studio.engine;

    let trip = engine.create_collection("Trip", None).unwrap();
    let count = |engine: &photo_engine::Engine| {
        engine
            .collections()
            .iter()
            .find(|collection| collection.id == trip)
            .map(|collection| collection.photo_count)
    };
    assert_eq!(count(engine), Some(0));
    engine
        .add_to_collection(trip, Target::Photos(vec![dawn, lake]))
        .unwrap();
    assert_eq!(count(engine), Some(2));

    engine
        .add_keywords(Target::Photos(vec![dawn]), &["coast"])
        .unwrap();
    let keyword = &engine.keywords()[0];
    assert_eq!((keyword.name.as_str(), keyword.photo_count), ("coast", 1));

    engine
        .toggle_quick_collection(Target::Photos(vec![dawn, lake]))
        .unwrap();
    assert_eq!(engine.source_counts().quick_collection, 2);
    assert!(engine.summary(dawn).unwrap().in_quick_collection);
    engine
        .toggle_quick_collection(Target::Photos(vec![dawn, lake]))
        .unwrap();
    assert_eq!(engine.source_counts().quick_collection, 0);

    engine
        .create_virtual_copies(Target::Photos(vec![dawn]))
        .unwrap();
    let spring: Vec<u64> = engine.folders().iter().map(|f| f.photo_count).collect();
    assert_eq!(spring, [4, 3]);
    assert_eq!(engine.source_counts().all_photos, 7);

    engine
        .remove_from_catalog(Target::Photos(vec![lake]))
        .unwrap();
    assert_eq!(count(engine), Some(1));
    let folders: Vec<u64> = engine.folders().iter().map(|f| f.photo_count).collect();
    assert_eq!(folders, [4, 2]);
    assert_eq!(engine.source_counts().all_photos, 6);
}

#[test]
fn a_smart_collection_reflects_a_rating_change_without_being_touched() {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    let rules = SmartRules {
        match_all: true,
        rules: vec![SmartRule {
            field: SmartField::Rating,
            op: SmartOp::AtLeast,
            value: 4.into(),
        }],
    };
    let engine = &mut studio.engine;
    let best = engine
        .create_smart_collection("Best", None, &rules)
        .unwrap();
    let count = |engine: &photo_engine::Engine| engine.collections()[0].photo_count;
    assert_eq!(count(engine), 0);

    engine
        .set_rating(Target::Photos(vec![dawn, lake]), Rating::new(4))
        .unwrap();
    assert_eq!(count(engine), 2);
    engine.set_source(Source::Collection(best)).unwrap();
    assert_eq!(studio.ids().len(), 2);

    let engine = &mut studio.engine;
    engine
        .set_rating(Target::Photos(vec![dawn]), Rating::new(1))
        .unwrap();
    assert_eq!(studio.ids(), [lake]);
    assert!(studio.engine.undo().unwrap());
    assert_eq!(studio.ids().len(), 2);
    assert_eq!(studio.engine.smart_rules(best).unwrap(), rules);
}
