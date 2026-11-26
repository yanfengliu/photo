//! What a list re-query costs at 20,000 photos. Ignored by default; run
//! with `cargo test -p photo-engine --test scenarios -- --ignored --nocapture`.
//!
//! The printed medians are the result. The ceilings only catch a
//! pathological regression.

use std::time::{Duration, Instant};

use photo_catalog::{Catalog, NewPhoto};
use photo_core::meta::{FileKind, Rating};
use photo_core::time::DateTime;
use photo_engine::{
    Comparison, Engine, Filter, RatingFilter, Sort, SortDirection, SortKey, Target,
};

use crate::common::config;

const PHOTOS: usize = 20_000;

fn photo(index: usize) -> NewPhoto {
    let path = format!(
        "/library/{}/{:02}/IMG_{index}.jpg",
        2015 + index % 10,
        1 + index % 12
    );
    let mut photo = NewPhoto::new(path, FileKind::Raster);
    photo.file_size = 10_000_000 + index as u64;
    photo.metadata.width = 6000;
    photo.metadata.height = 4000;
    photo.metadata.capture_time = DateTime::new(
        2015 + (index % 10) as i32,
        1 + (index % 12) as u8,
        1 + (index % 28) as u8,
        (index % 24) as u8,
        (index % 60) as u8,
        0,
    );
    photo.metadata.camera_model = Some(["Z8", "X-T5", "ILCE-7RM5"][index % 3].to_string());
    photo.title = format!("study {}", index % 50);
    photo.keywords = vec![format!("trip {}", index % 40)];
    photo
}

/// The median time of `runs` calls, printed.
fn median(label: &str, runs: usize, mut work: impl FnMut(usize)) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|run| {
            let start = Instant::now();
            work(run);
            start.elapsed()
        })
        .collect();
    times.sort();
    let middle = times[runs / 2];
    println!("{label:<64} {:>8.2} ms", middle.as_secs_f64() * 1000.0);
    assert!(middle < Duration::from_secs(5), "{label} took {middle:?}");
    middle
}

#[test]
#[ignore = "timing report"]
fn requery_at_twenty_thousand_photos() {
    let dir = tempfile::tempdir().unwrap();
    let settings = config(dir.path(), 2);
    {
        let mut catalog = Catalog::open(&settings.catalog_path).unwrap();
        let run = catalog.begin_import().unwrap();
        for batch in 0..4 {
            let photos: Vec<NewPhoto> = (batch * 5_000..(batch + 1) * 5_000).map(photo).collect();
            catalog.insert_photos(run, &photos).unwrap();
        }
    }
    let mut engine = Engine::open(settings).unwrap();
    assert_eq!(engine.photos().len(), PHOTOS);
    println!("\n{PHOTOS} photos, file-backed catalog");

    median("one catalog write alone (a UI setting)", 9, |run| {
        engine.set_ui_setting("timing", &run.to_string()).unwrap();
    });
    median("full re-query: sort order changed", 9, |run| {
        let direction = if run % 2 == 0 {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
        engine
            .set_sort(Sort {
                key: SortKey::FileName,
                direction,
            })
            .unwrap();
    });
    median("full re-query: text filter typed", 9, |run| {
        let filter = Filter {
            text: format!("study {}", run % 5),
            ..Filter::default()
        };
        engine.set_filter(filter).unwrap();
    });
    engine.set_filter(Filter::default()).unwrap();
    let photos: Vec<_> = engine
        .photos()
        .iter()
        .take(9)
        .map(|photo| photo.id)
        .collect();
    median(
        "rating keystroke, list updated in place (one write)",
        9,
        |run| {
            let stars = Rating::new(1 + (run % 5) as u8);
            engine
                .set_rating(Target::Photos(vec![photos[run]]), stars)
                .unwrap();
        },
    );
    engine
        .set_filter(Filter {
            rating: Some(RatingFilter {
                comparison: Comparison::AtMost,
                rating: Rating::new(5),
            }),
            ..Filter::default()
        })
        .unwrap();
    median(
        "rating keystroke under a rating filter, full re-query (one write)",
        9,
        |run| {
            let stars = Rating::new((run % 5) as u8);
            engine
                .set_rating(Target::Photos(vec![photos[run]]), stars)
                .unwrap();
        },
    );
    assert_eq!(engine.photos().len(), PHOTOS);
}

/// The develop session's interactive loop: one slider update to its draft
/// frame, for a 2.5-megapixel view of a 24-megapixel photo.
#[test]
#[ignore = "timing report"]
fn settings_update_to_draft_frame_at_two_and_a_half_megapixels() {
    use photo_core::develop::Adj;
    use photo_engine::{CanvasView, DevelopLoad, Quality};

    let (_files, files) = crate::common::many_jpegs(1, 6000, 4000);
    let mut studio = crate::common::Studio::with_workers(4);
    studio.import(files);
    let photo = studio.ids()[0];
    studio.engine.open_develop(photo).unwrap();
    studio.engine.set_canvas_view(CanvasView::Fit {
        width: 1936,
        height: 1291,
    });
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Rendered);
    let view = session.frame().unwrap().view();
    let megapixels = f64::from(view.width() * view.height()) / 1e6;
    println!(
        "\nview {}x{} ({megapixels:.2} MP) of a 6000x4000 photo",
        view.width(),
        view.height()
    );
    assert!((2.4..2.6).contains(&megapixels));

    let draft = median("settings update to its draft frame", 15, |run| {
        let value = if run % 2 == 0 { 0.5 } else { -0.5 };
        studio
            .engine
            .adjust(Adj::Exposure, value + run as f32 / 100.0)
            .unwrap();
        crate::common::settle(&mut studio.engine);
        let session = studio.engine.develop().unwrap();
        let frame = session.frame().unwrap();
        assert_eq!(
            (frame.version(), frame.quality()),
            (session.version(), Quality::Draft)
        );
    });
    let stages = studio
        .engine
        .develop()
        .unwrap()
        .frame()
        .unwrap()
        .stage_times()
        .to_vec();
    for (stage, time) in stages {
        println!("  {stage:<62} {:>8.2} ms", time.as_secs_f64() * 1000.0);
    }
    studio.engine.commit_edit().unwrap();
    median("release: commit to its full-quality frame", 5, |run| {
        studio
            .engine
            .adjust(Adj::Contrast, run as f32 + 1.0)
            .unwrap();
        studio.engine.commit_edit().unwrap();
        crate::common::settle(&mut studio.engine);
    });
    assert!(draft < Duration::from_secs(1), "{draft:?}");
}

/// Setting the keywords of many photos is one catalog write, and so are its
/// undo and redo: none of them grows with a commit per photo.
#[test]
#[ignore = "timing report"]
fn keywords_set_on_many_photos_are_one_catalog_write() {
    const MANY: usize = 500;
    let dir = tempfile::tempdir().unwrap();
    let settings = config(dir.path(), 2);
    {
        let mut catalog = Catalog::open(&settings.catalog_path).unwrap();
        let run = catalog.begin_import().unwrap();
        let photos: Vec<NewPhoto> = (0..MANY).map(photo).collect();
        catalog.insert_photos(run, &photos).unwrap();
    }
    let mut engine = Engine::open(settings).unwrap();
    let photos: Vec<_> = engine.photos().iter().map(|photo| photo.id).collect();
    assert_eq!(photos.len(), MANY);
    println!("\n{MANY} photos, file-backed catalog");
    let set = median("set two keywords on every photo", 3, |run| {
        let names = [format!("batch {run}"), "kept".to_string()];
        engine
            .set_keywords(Target::Photos(photos.clone()), &names)
            .unwrap();
    });
    let undo = median("undo it", 1, |_| {
        engine.undo().unwrap();
    });
    let redo = median("redo it", 1, |_| {
        engine.redo().unwrap();
    });
    for (what, took) in [("set", set), ("undo", undo), ("redo", redo)] {
        assert!(took < Duration::from_secs(1), "{what} took {took:?}");
    }
}
