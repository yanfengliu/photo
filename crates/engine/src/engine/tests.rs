//! Tests that need to see inside the engine: what was decoded, when a
//! preview is looked at again, and what happens while its workers are held.

mod imports;
mod loading;
mod rechecks;
mod saves;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use photo_core::meta::PhotoId;
use tempfile::TempDir;

use super::{Engine, EngineConfig, EngineEvent};
use crate::develop::worker::{Delivery, Frame, Outcome, Purpose, Work};
use crate::develop::{CanvasView, DevelopLoad};
use crate::import::{ImportMode, ImportRequest};
use crate::jobs::Lane;
use crate::looks::{BuiltInPreset, PresetKey};
use crate::previews::PreviewState;
use crate::stamp::Stamp;

const PATIENCE: Duration = Duration::from_secs(60);

/// The engine of the catalog kept under `dir`, made if it is not there.
fn open_in(dir: &Path, workers: usize) -> Engine {
    let catalog = dir.join("catalog");
    fs::create_dir_all(&catalog).unwrap();
    Engine::open(EngineConfig {
        catalog_path: catalog.join("library.photocat"),
        previews_dir: catalog.join("library.previews"),
        worker_threads: workers,
        export_dir: dir.join("exports"),
    })
    .unwrap()
}

/// An engine on a new catalog with `files` imported in place.
fn engine_with(files: &[PathBuf], workers: usize) -> (TempDir, Engine, Vec<PhotoId>) {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = open_in(dir.path(), workers);
    let task = engine
        .start_import(ImportRequest {
            files: files.to_vec(),
            mode: ImportMode::Add,
            skip_suspected_duplicates: false,
            keywords: Vec::new(),
            read_sidecars: false,
        })
        .unwrap();
    assert!(engine.wait_idle(PATIENCE));
    let ids = engine.import_summary(task).unwrap().imported.clone();
    assert_eq!(ids.len(), files.len());
    (dir, engine, ids)
}

/// Keep one worker busy from now until the returned sender is used or
/// dropped. With one worker, everything submitted meanwhile waits in the
/// queue.
fn hold_worker(engine: &mut Engine) -> Sender<()> {
    let (started, has_started) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    engine.submit(
        Lane::Urgent,
        None,
        None,
        move |_| {
            started.send(()).unwrap();
            // An error is the sender dropped, which releases as well.
            let _ = released.recv();
            Box::new(|_: &mut Engine| false)
        },
        |_, _| false,
    );
    has_started.recv_timeout(PATIENCE).unwrap();
    release
}

/// Leave the release of a held worker to the queue: the worker stays busy
/// until the pool, shutting down, has dropped everything still queued. So
/// what is queued behind it never runs, and the pool posts nothing for it.
fn release_at_shutdown(engine: &mut Engine, release: Sender<()>) {
    engine.submit(
        Lane::Background,
        None,
        None,
        move |_| {
            drop(release);
            Box::new(|_: &mut Engine| false)
        },
        |_, _| false,
    );
}

/// Take the next thing a worker has posted, without applying it. With one
/// worker, results come in the order the jobs were queued.
fn next_event(engine: &Engine) -> EngineEvent {
    engine.events.recv_timeout(PATIENCE).unwrap()
}

/// What the calling thread logs while `work` runs.
fn logged(work: impl FnOnce()) -> Vec<String> {
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            LINES.with_borrow_mut(|lines| lines.push(record.args().to_string()));
        }
        fn flush(&self) {}
    }
    thread_local! {
        static LINES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
    LINES.with_borrow_mut(Vec::clear);
    work();
    LINES.take()
}

fn jpegs(count: usize, width: u32, height: u32) -> (TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let files = (0..count)
        .map(|index| {
            photo_testkit::write_jpeg(dir.path(), &format!("IMG_{index:04}.jpg"), width, height)
        })
        .collect();
    (dir, files)
}

#[test]
fn opening_photos_in_a_row_decodes_only_what_is_left_open() {
    let (_files_dir, files) = jpegs(6, 1200, 800);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    for id in &ids {
        engine.open_develop(*id).unwrap();
    }
    assert!(engine.wait_idle(PATIENCE));
    let session = engine.develop().unwrap();
    assert_eq!(session.photo(), ids[5]);
    assert_eq!(session.load_state(), &DevelopLoad::Decoded);
    let decoded = |file: &PathBuf| {
        let stamp = Stamp::read(file).unwrap();
        engine.sources.cached(file, stamp).is_some()
    };
    assert!(decoded(&files[5]), "the photo left open");
    let count = files.iter().filter(|file| decoded(file)).count();
    assert!(count <= 2, "{count} of 6 photos were decoded");
}

#[test]
fn a_failed_preview_is_tried_again_once_the_failure_is_old() {
    let (files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 2);
    let away = files_dir.path().join("away.jpg");
    fs::rename(&files[0], &away).unwrap();
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Failed(_)));

    // The drive is plugged back in.
    fs::rename(&away, &files[0]).unwrap();
    assert!(
        matches!(engine.thumbnail(ids[0]), PreviewState::Failed(_)),
        "a fresh failure is not tried again at every request"
    );
    assert!(engine.is_idle());
    engine.previews.retry_after = Duration::ZERO;
    assert!(
        matches!(engine.thumbnail(ids[0]), PreviewState::Pending),
        "an old one is"
    );
    assert!(engine.wait_idle(PATIENCE));
    assert!(engine.thumbnail(ids[0]).ready().is_some());
}

#[test]
fn a_preview_in_memory_is_made_again_when_its_file_was_replaced() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 2);
    engine.thumbnail(ids[0]);
    assert!(engine.wait_idle(PATIENCE));
    let first = engine.thumbnail(ids[0]).ready().cloned().unwrap();
    assert_eq!((first.image().width, first.image().height), (96, 64));

    // Another picture is saved under the same name.
    let other = tempfile::tempdir().unwrap();
    let replacement = photo_testkit::write_jpeg(other.path(), "IMG_0000.jpg", 64, 96);
    fs::copy(&replacement, &files[0]).unwrap();
    assert!(engine.thumbnail(ids[0]).ready().is_some());
    assert!(
        engine.is_idle(),
        "a preview just made is not looked at again"
    );
    engine.previews.recheck_after = Duration::ZERO;
    assert!(
        engine.thumbnail(ids[0]).ready().is_some(),
        "answered from memory while a worker looks"
    );
    assert!(engine.wait_idle(PATIENCE));
    assert!(matches!(engine.thumbnail(ids[0]), PreviewState::Pending));
    assert!(engine.wait_idle(PATIENCE));
    let second = engine.thumbnail(ids[0]).ready().cloned().unwrap();
    assert_eq!((second.image().width, second.image().height), (64, 96));
}

#[test]
fn a_late_preset_preview_is_not_kept_under_the_preset_chosen_since() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    engine.open_develop(ids[0]).unwrap();
    engine.set_canvas_view(CanvasView::Fit {
        width: 96,
        height: 64,
    });
    assert!(engine.wait_idle(PATIENCE));
    let (mono, vivid) = (
        PresetKey::BuiltIn(BuiltInPreset::MonoContrast),
        PresetKey::BuiltIn(BuiltInPreset::Vivid),
    );
    engine.preview_preset(Some(mono), 48, 48).unwrap();
    let asked_for_mono = engine.next_render;
    engine.preview_preset(Some(vivid), 48, 48).unwrap();
    let asked_for_vivid = engine.next_render;
    assert!(asked_for_vivid > asked_for_mono);

    // What the render worker hands back for a request: here by hand, so
    // that the first preset's frame is the one that comes in late.
    let session = engine.develop().unwrap();
    let (generation, version, view) = (
        session.generation,
        session.version(),
        session.view().unwrap(),
    );
    let delivery = |id| {
        let image = photo_core::image::Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 255],
            space: photo_core::colorimetry::RgbSpace::Srgb,
        };
        Delivery {
            id,
            purpose: Purpose::Look,
            generation,
            version,
            view,
            work: Work::Render(photo_develop::Quality::Full),
            outcome: Outcome::Rendered(Box::new(Frame {
                histogram: photo_develop::histogram(&image),
                image,
                stage_times: Vec::new(),
                render_time: Duration::ZERO,
            })),
        }
    };
    engine.render_delivered(delivery(asked_for_mono));
    assert!(
        engine.develop().unwrap().preset_preview().is_none(),
        "the first preset's frame is not shown as the second's"
    );
    engine.render_delivered(delivery(asked_for_vivid));
    let session = engine.develop().unwrap();
    assert_eq!(session.preset_preview().map(|(key, _)| *key), Some(vivid));
}
