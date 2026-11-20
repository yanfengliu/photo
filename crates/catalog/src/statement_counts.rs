//! Every read takes the same number of SQL statements whatever the number of
//! photos: none runs a query per row. Statements are counted, not timed, so
//! the test is exact and quick enough to run every time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use photo_core::develop::DevelopSettings;
use photo_core::meta::{CollectionId, ColorLabel, FileKind, Flag, KeywordId, PhotoId, Rating};
use rusqlite::trace::{TraceEvent, TraceEventCodes};

use crate::{
    Catalog, Comparison, Facet, Filter, NewPhoto, PhotoQuery, RatingFilter, SmartField, SmartOp,
    SmartRule, SmartRules, Sort, SortDirection, SortKey, Source,
};

/// A catalog of `count` photos in four folders and three keywords, with
/// edits, a virtual copy, an album holding every photo and a smart
/// collection. Only the number of photos depends on `count`.
struct Library {
    catalog: Catalog,
    ids: Vec<PhotoId>,
    album: CollectionId,
    smart: CollectionId,
    keyword: KeywordId,
}

fn library(count: usize) -> Library {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let photos: Vec<NewPhoto> = (0..count)
        .map(|index| {
            let kind = [FileKind::Raw, FileKind::Raster][index % 2];
            let mut photo = NewPhoto::new(format!("/lib/{}/{index}.x", index % 4), kind);
            photo.rating = Rating::new((index % 6) as u8);
            photo.flag = [Flag::Unflagged, Flag::Picked][index % 2];
            photo.label = ColorLabel::ALL[index % 3];
            photo.title = format!("study {index}");
            photo.metadata.camera_model = Some(["Z8", "X-T5"][index % 2].into());
            photo.metadata.iso = Some(100 << (index % 4));
            photo.keywords = vec![format!("subject {}", index % 3)];
            photo
        })
        .collect();
    let run = catalog.begin_import().unwrap();
    let ids = catalog.insert_photos(run, &photos).unwrap();
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 0.5;
    for id in ids.iter().step_by(3) {
        catalog
            .set_develop_settings(*id, &settings, Some("Edit"))
            .unwrap();
    }
    catalog.create_virtual_copy(ids[0]).unwrap();
    let album = catalog.create_collection("Album", None).unwrap();
    catalog.add_to_collection(album, &ids).unwrap();
    catalog
        .add_to_collection(catalog.quick_collection(), &ids[..2])
        .unwrap();
    let rules = SmartRules {
        match_all: true,
        rules: vec![
            SmartRule {
                field: SmartField::Rating,
                op: SmartOp::AtLeast,
                value: 2.into(),
            },
            SmartRule {
                field: SmartField::Keyword,
                op: SmartOp::Contains,
                value: "subject".into(),
            },
        ],
    };
    let smart = catalog
        .create_smart_collection("Rated", None, &rules)
        .unwrap();
    let keyword = catalog.keywords().unwrap()[0].id;
    Library {
        catalog,
        ids,
        album,
        smart,
        keyword,
    }
}

/// How many statements `read` runs.
fn statements(library: &mut Library, read: &dyn Fn(&Library)) -> usize {
    let counted = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&counted);
    library
        .catalog
        .conn
        .trace_v2(
            TraceEventCodes::SQLITE_TRACE_STMT,
            Some(move |event: TraceEvent<'_>| {
                if let TraceEvent::Stmt(..) = event {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            }),
        )
        .unwrap();
    read(library);
    library
        .catalog
        .conn
        .trace_v2(
            TraceEventCodes::SQLITE_TRACE_STMT,
            None::<fn(TraceEvent<'_>)>,
        )
        .unwrap();
    counted.load(Ordering::Relaxed)
}

/// A filter that uses every list and range, and the free text.
fn busy_filter() -> Filter {
    Filter {
        text: "study".into(),
        flags: vec![Flag::Picked, Flag::Unflagged],
        rating: Some(RatingFilter {
            comparison: Comparison::AtLeast,
            rating: Rating::new(1),
        }),
        labels: ColorLabel::ALL.to_vec(),
        cameras: vec!["Z8".into(), "X-T5".into()],
        ..Filter::default()
    }
}

type Read = Box<dyn Fn(&Library)>;

fn query(source: impl Fn(&Library) -> Source + 'static, key: SortKey) -> Read {
    Box::new(move |library| {
        let query = PhotoQuery {
            source: source(library),
            filter: Filter::default(),
            sort: Sort {
                key,
                direction: SortDirection::Descending,
            },
        };
        library.catalog.query(&query).unwrap();
    })
}

fn reads() -> Vec<(&'static str, Read)> {
    let mut reads: Vec<(&'static str, Read)> = vec![
        (
            "all photos",
            query(|_| Source::AllPhotos, SortKey::CaptureTime),
        ),
        (
            "by file name",
            query(|_| Source::AllPhotos, SortKey::FileName),
        ),
        (
            "a folder and its subfolders",
            query(
                |library| Source::Folder {
                    id: library.catalog.folders().unwrap()[0].id,
                    include_subfolders: true,
                },
                SortKey::Rating,
            ),
        ),
        (
            "album in its order",
            query(|l| Source::Collection(l.album), SortKey::Custom),
        ),
        (
            "smart collection",
            query(|l| Source::Collection(l.smart), SortKey::Pick),
        ),
        (
            "keyword",
            query(|l| Source::Keyword(l.keyword), SortKey::Label),
        ),
        (
            "quick collection",
            query(|_| Source::QuickCollection, SortKey::EditTime),
        ),
        (
            "previous import",
            query(|_| Source::PreviousImport, SortKey::FileType),
        ),
        (
            "filtered",
            Box::new(|library| {
                let query = PhotoQuery {
                    filter: busy_filter(),
                    ..PhotoQuery::default()
                };
                library.catalog.query(&query).unwrap();
                library.catalog.query_ids(&query).unwrap();
                library
                    .catalog
                    .count_matching(&query.source, &query.filter)
                    .unwrap();
            }),
        ),
        (
            "cells and records by id",
            Box::new(|library| {
                library.catalog.summaries(&library.ids).unwrap();
                library.catalog.photos(&library.ids).unwrap();
            }),
        ),
        (
            "panels",
            Box::new(|library| {
                let catalog = &library.catalog;
                catalog.folders().unwrap();
                catalog.keywords().unwrap();
                catalog.collections().unwrap();
                catalog.imports().unwrap();
                catalog.stats().unwrap();
                catalog.collection_photos(library.album).unwrap();
                catalog.count(&Source::AllPhotos).unwrap();
            }),
        ),
    ];
    for facet in [Facet::Camera, Facet::Lens, Facet::Iso, Facet::CaptureDate] {
        reads.push((
            "facet",
            Box::new(move |library| {
                library
                    .catalog
                    .facet_values(&Source::AllPhotos, &busy_filter(), facet)
                    .unwrap();
            }),
        ));
    }
    reads
}

#[test]
fn no_read_runs_more_statements_for_more_photos() {
    let (mut small, mut large) = (library(12), library(240));
    for (name, read) in reads() {
        let few = statements(&mut small, read.as_ref());
        assert!(few > 0, "{name}: nothing was counted");
        assert_eq!(statements(&mut large, read.as_ref()), few, "{name}");
    }
}
