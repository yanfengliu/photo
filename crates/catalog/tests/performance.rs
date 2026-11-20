//! Timing reports. Ignored by default; run with
//! `cargo test -p photo-catalog --test performance -- --ignored --nocapture`.
//!
//! The assertions are generous ceilings that only catch pathological
//! slowness; the printed numbers are the real result. That no read runs a
//! query per row is pinned exactly, and on every test run, by the
//! statement-counting unit test in `src/statement_counts.rs`.

mod common;

use std::time::{Duration, Instant};

use common::date;
use photo_catalog::{
    Catalog, Comparison, Facet, Filter, NewPhoto, PhotoQuery, RatingFilter, SmartField, SmartOp,
    SmartRule, SmartRules, Sort, SortDirection, SortKey, Source,
};
use photo_core::meta::{ColorLabel, FileKind, Flag, PhotoId, Rating};

const CAMERAS: [&str; 3] = ["ILCE-7RM5", "Z8", "X-T5"];
const LENSES: [&str; 5] = [
    "24-70mm F2.8",
    "50mm F1.2",
    "85mm F1.8",
    "16-35mm F4",
    "100-400mm F5.6",
];
const SUBJECTS: [&str; 8] = [
    "harbour", "street", "portrait", "forest", "summit", "market", "night", "garden",
];

/// A varied, deterministic photo: 100 folders three levels deep, a spread of
/// cameras, lenses, ISOs, ratings, flags, labels and keywords.
fn photo(index: usize) -> NewPhoto {
    let folder = index % 100;
    let (extension, kind) = if index.is_multiple_of(3) {
        ("jpg", FileKind::Raster)
    } else {
        ("arw", FileKind::Raw)
    };
    let path = format!(
        "/library/{}/{:02}/set{}/IMG_{}.{extension}",
        2015 + folder % 10,
        1 + folder % 12,
        folder,
        index
    );
    let mut photo = NewPhoto::new(path, kind);
    photo.file_size = 20_000_000 + index as u64;
    photo.file_mtime = 1_600_000_000 + index as i64;
    photo.metadata.width = 6000;
    photo.metadata.height = 4000;
    photo.metadata.capture_time = date(
        2015 + (index % 10) as i32,
        1 + (index % 12) as u8,
        1 + (index % 28) as u8,
    )
    .into();
    photo.metadata.camera_model = Some(CAMERAS[index % CAMERAS.len()].into());
    photo.metadata.lens = Some(LENSES[index % LENSES.len()].into());
    photo.metadata.iso = Some(100 << (index % 8));
    photo.metadata.aperture = Some(1.4 + (index % 9) as f32);
    photo.metadata.focal_length_mm = Some(16.0 + (index % 200) as f32);
    photo.rating = Rating::new((index % 6) as u8);
    photo.flag = [
        Flag::Unflagged,
        Flag::Picked,
        Flag::Unflagged,
        Flag::Rejected,
    ][index % 4];
    photo.label = ColorLabel::ALL[index % ColorLabel::ALL.len()];
    photo.title = format!("{} study {}", SUBJECTS[index % SUBJECTS.len()], index % 50);
    photo.keywords = vec![
        SUBJECTS[index % SUBJECTS.len()].to_string(),
        format!("trip {}", index % 40),
        format!("client {}", index % 25),
    ];
    photo
}

fn timed<T>(label: &str, ceiling: Duration, work: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = work();
    let elapsed = start.elapsed();
    println!("{label:<58} {:>9.2} ms", elapsed.as_secs_f64() * 1000.0);
    assert!(
        elapsed < ceiling,
        "{label} took {elapsed:?}, ceiling {ceiling:?}"
    );
    value
}

fn insert(catalog: &mut Catalog, range: std::ops::Range<usize>) -> Vec<PhotoId> {
    let photos: Vec<NewPhoto> = range.map(photo).collect();
    let run = catalog.begin_import().unwrap();
    catalog.insert_photos(run, &photos).unwrap()
}

/// The queries the Library runs, each timed against the catalog as it is.
fn time_queries(catalog: &mut Catalog, total: usize, ceiling: Duration) {
    let all = timed("query: all photos by capture time", ceiling, || {
        catalog.query(&PhotoQuery::default()).unwrap()
    });
    assert_eq!(all.len(), total);

    let filtered = PhotoQuery {
        source: Source::AllPhotos,
        filter: Filter {
            text: "harbour trip".into(),
            flags: vec![Flag::Picked, Flag::Unflagged],
            rating: Some(RatingFilter {
                comparison: Comparison::AtLeast,
                rating: Rating::new(3),
            }),
            cameras: vec!["Z8".into(), "X-T5".into()],
            ..Filter::default()
        },
        sort: Sort {
            key: SortKey::FileName,
            direction: SortDirection::Descending,
        },
    };
    let found = timed(
        "query: text + flags + rating + camera, by file name",
        ceiling,
        || catalog.query(&filtered).unwrap(),
    );
    assert!(!found.is_empty() && found.len() < total / 4);
    timed("count: the same filter", ceiling, || {
        catalog
            .count_matching(&filtered.source, &filtered.filter)
            .unwrap()
    });
    timed("ids only: all photos by rating", ceiling, || {
        let by_rating = PhotoQuery {
            sort: Sort {
                key: SortKey::Rating,
                direction: SortDirection::Descending,
            },
            ..PhotoQuery::default()
        };
        catalog.query_ids(&by_rating).unwrap()
    });
    timed("facets: cameras and lenses", ceiling, || {
        let filter = Filter::default();
        let cameras = catalog
            .facet_values(&Source::AllPhotos, &filter, Facet::Camera)
            .unwrap();
        (
            cameras,
            catalog
                .facet_values(&Source::AllPhotos, &filter, Facet::Lens)
                .unwrap(),
        )
    });

    let year = catalog
        .folders()
        .unwrap()
        .into_iter()
        .find(|folder| folder.path.ends_with("2015/01/set0"))
        .unwrap();
    let root = catalog
        .ensure_folder(std::path::Path::new("/library/2015"))
        .unwrap();
    timed("query: one folder", ceiling, || {
        let source = Source::Folder {
            id: year.id,
            include_subfolders: false,
        };
        catalog.query(&PhotoQuery::from(source)).unwrap()
    });
    let in_year = timed("query: a folder with all its subfolders", ceiling, || {
        let source = Source::Folder {
            id: root,
            include_subfolders: true,
        };
        catalog.query(&PhotoQuery::from(source)).unwrap()
    });
    assert_eq!(in_year.len(), total / 10);

    let rules = SmartRules {
        match_all: true,
        rules: vec![
            SmartRule {
                field: SmartField::Rating,
                op: SmartOp::AtLeast,
                value: 4.into(),
            },
            SmartRule {
                field: SmartField::Keyword,
                op: SmartOp::Contains,
                value: "TRIP 1".into(),
            },
            SmartRule {
                field: SmartField::Title,
                op: SmartOp::DoesNotContain,
                value: "market".into(),
            },
        ],
    };
    let smart = catalog
        .create_smart_collection("timing", None, &rules)
        .unwrap();
    timed(
        "query: smart collection (rating, keyword, title)",
        ceiling,
        || {
            catalog
                .query(&PhotoQuery::from(Source::Collection(smart)))
                .unwrap()
        },
    );
    timed(
        "list: folders, keywords and collections with counts",
        ceiling,
        || {
            (
                catalog.folders().unwrap(),
                catalog.keywords().unwrap(),
                catalog.collections().unwrap(),
            )
        },
    );
    catalog.delete_collection(smart).unwrap();
}

#[test]
#[ignore = "timing report"]
fn five_thousand_photos() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path().join("timing.photocat")).unwrap();
    println!("\n5,000 photos, file-backed catalog");
    let ids = timed(
        "insert: one batch of 5,000 (3 keywords each)",
        Duration::from_secs(10),
        || insert(&mut catalog, 0..5_000),
    );
    time_queries(&mut catalog, 5_000, Duration::from_secs(2));

    let some: Vec<PhotoId> = ids.iter().copied().step_by(10).collect();
    timed("set ratings on 500 photos", Duration::from_secs(2), || {
        let ratings: Vec<(PhotoId, Rating)> = some.iter().map(|id| (*id, Rating::new(2))).collect();
        catalog.set_ratings(&ratings).unwrap();
    });
    let removed = timed("remove 500 photos", Duration::from_secs(2), || {
        catalog.remove_photos(&some).unwrap()
    });
    timed("restore 500 photos", Duration::from_secs(2), || {
        catalog.restore_photos(&removed).unwrap()
    });
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 5_000);
}

#[test]
#[ignore = "timing report"]
fn one_hundred_thousand_photos() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path().join("timing.photocat")).unwrap();
    println!("\n100,000 photos, file-backed catalog");
    timed(
        "insert: 20 batches of 5,000",
        Duration::from_secs(120),
        || {
            for batch in 0..20 {
                insert(&mut catalog, batch * 5_000..(batch + 1) * 5_000);
            }
        },
    );
    time_queries(&mut catalog, 100_000, Duration::from_secs(10));
    timed("reopen", Duration::from_secs(5), || {
        drop(std::mem::replace(
            &mut catalog,
            Catalog::open_in_memory().unwrap(),
        ));
        catalog = Catalog::open(dir.path().join("timing.photocat")).unwrap();
    });
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 100_000);
}
