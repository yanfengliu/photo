//! Opening, waiting and dropping.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use photo_engine::{Engine, EngineError, PreviewState};

use crate::common::{PATIENCE, Studio, config, many_jpegs};

#[test]
fn wait_idle_is_true_after_work_and_false_on_timeout_with_work_outstanding() {
    let (_files_dir, files) = many_jpegs(30, 640, 480);
    let mut studio = Studio::with_workers(1);
    studio.import(files);
    assert!(studio.engine.is_idle());

    for photo in studio.ids() {
        assert!(matches!(
            studio.engine.standard_preview(photo),
            PreviewState::Pending
        ));
    }
    assert!(!studio.engine.is_idle());
    assert!(
        !studio.engine.wait_idle(Duration::ZERO),
        "thirty previews cannot all be ready at once"
    );
    let pending = studio.engine.pending_work();
    assert!(pending.jobs > 0 && pending.previews > 0, "{pending:?}");

    assert!(studio.engine.wait_idle(PATIENCE));
    assert!(studio.engine.is_idle());
    assert_eq!(studio.engine.pending_work().jobs, 0);
    for photo in studio.ids() {
        assert!(studio.engine.standard_preview(photo).ready().is_some());
    }
}

#[test]
fn dropping_the_engine_with_jobs_queued_returns_promptly_and_leaves_no_thread() {
    let (_files_dir, files) = many_jpegs(40, 640, 480);
    let mut studio = Studio::with_workers(1);
    studio.import(files);
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&woken);
    studio.engine.set_wake(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    for photo in studio.ids() {
        studio.engine.standard_preview(photo);
    }
    assert!(studio.engine.pending_work().queued > 0);

    let start = Instant::now();
    let Studio { dir, engine } = studio;
    drop(engine);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        Arc::strong_count(&woken),
        1,
        "a worker thread still holds the wake callback"
    );

    // Nothing is left holding the catalog either.
    let mut again = Engine::open(config(dir.path(), 1)).unwrap();
    assert_eq!(again.photos().len(), 40);
    assert!(again.wait_idle(PATIENCE));
}

#[test]
fn opening_needs_a_usable_previews_folder() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("previews");
    std::fs::write(&blocked, b"a file, not a folder").unwrap();
    let mut settings = config(dir.path(), 1);
    settings.previews_dir = blocked;
    assert!(matches!(
        Engine::open(settings),
        Err(EngineError::PreviewsFolder { .. })
    ));
}

#[test]
fn ui_settings_persist_with_the_catalog() {
    let mut studio = Studio::new();
    assert_eq!(studio.engine.ui_setting("panel.left").unwrap(), None);
    studio
        .engine
        .set_ui_setting("panel.left", "collapsed")
        .unwrap();
    let studio = studio.reopen();
    assert_eq!(
        studio.engine.ui_setting("panel.left").unwrap().as_deref(),
        Some("collapsed")
    );
}
