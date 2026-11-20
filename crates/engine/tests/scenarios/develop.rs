//! The develop session: loading, editing, committing, undo.

use std::time::{Duration, Instant};

use photo_core::develop::{Adj, LocalAdj, Mask, MaskShape};
use photo_core::geom::Point;
use photo_core::meta::PhotoId;
use photo_engine::{CanvasView, DevelopLoad, EngineError, PreviewSize, Quality};
use photo_testkit::TempLibrary;

use crate::common::{Studio, mean_luma, open_rendered};

const FIT: CanvasView = CanvasView::Fit {
    width: 320,
    height: 240,
};

fn studio() -> (TempLibrary, Studio) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    (library, studio)
}

/// The largest develop version of any photo: the catalog-wide counter.
fn newest_version(studio: &Studio) -> u64 {
    studio
        .engine
        .photos()
        .iter()
        .map(|photo| photo.develop_version)
        .max()
        .unwrap_or(0)
}

#[test]
fn a_raw_photo_shows_its_embedded_preview_then_decodes_then_renders() {
    let (_library, mut studio) = studio();
    let harbour = studio.id("harbour.dng");
    studio.engine.open_develop(harbour).unwrap();
    assert_eq!(
        studio.engine.develop().unwrap().load_state(),
        &DevelopLoad::Nothing
    );
    studio.engine.set_canvas_view(FIT);
    studio.settle();

    let session = studio.engine.develop().unwrap();
    assert_eq!(
        session.load_history(),
        [
            DevelopLoad::Nothing,
            DevelopLoad::EmbeddedPreview,
            DevelopLoad::Decoded,
            DevelopLoad::Rendered,
        ]
    );
    assert!(session.embedded_preview().is_some());
    assert_eq!(session.output_size(), Some((96, 64)));
    let frame = session.frame().unwrap();
    assert_eq!(frame.version(), session.version());
    assert_eq!(frame.quality(), Quality::Full);
    let image = frame.image();
    assert_eq!(
        (image.width, image.height),
        (frame.view().width(), frame.view().height())
    );
    assert_eq!(
        frame.histogram().pixel_count(),
        u64::from(image.width * image.height)
    );
    let (kelvin, _) = session.as_shot_white_balance().unwrap();
    assert!((4000.0..6500.0).contains(&kelvin), "{kelvin}");
    assert_eq!(
        session.shown_white_balance(),
        session.as_shot_white_balance()
    );
}

#[test]
fn a_raster_photo_decodes_then_renders_and_follows_the_canvas() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    let session = studio.engine.develop().unwrap();
    assert_eq!(
        session.load_history(),
        [
            DevelopLoad::Nothing,
            DevelopLoad::Decoded,
            DevelopLoad::Rendered
        ]
    );
    assert_eq!(session.shown_white_balance(), Some((0.0, 0.0)));
    let fitted = session.frame().unwrap().view();
    assert_eq!(
        (fitted.width(), fitted.height()),
        (96, 64),
        "never enlarged"
    );

    studio.engine.set_canvas_view(CanvasView::ActualSize {
        center: Point::new(48.0, 32.0),
        width: 40,
        height: 20,
    });
    studio.settle();
    let session = studio.engine.develop().unwrap();
    let view = session.frame().unwrap().view();
    assert_eq!((view.width(), view.height()), (40, 20));
    assert_eq!(view.scale(), 1.0);
    assert_eq!(session.view(), Some(view));
    assert_eq!(session.geometry().unwrap().output_size(), (96, 64));
}

#[test]
fn a_drag_renders_drafts_and_its_release_commits_once() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    let steps = studio.engine.history(dawn).unwrap().len();
    let newest = newest_version(&studio);
    let stored = studio.engine.summary(dawn).unwrap().develop_version;
    let requested = studio.engine.develop().unwrap().render_counts().requested;
    studio.toasts();

    for step in 1..=60 {
        studio
            .engine
            .adjust(Adj::Exposure, step as f32 / 100.0)
            .unwrap();
    }
    let session = studio.engine.develop().unwrap();
    assert!(session.is_editing());
    assert_eq!(session.settings().tone.exposure, 0.6);
    assert_eq!(
        studio.engine.history(dawn).unwrap().len(),
        steps,
        "nothing written yet"
    );
    assert_eq!(studio.engine.summary(dawn).unwrap().develop_version, stored);
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().tone.exposure,
        0.0
    );
    assert_eq!(studio.engine.undo_label(), None);

    assert!(studio.engine.commit_edit().unwrap());
    studio.settle();
    let session = studio.engine.develop().unwrap();
    let frame = session.frame().unwrap();
    assert_eq!(
        frame.version(),
        session.version(),
        "the newest render arrived"
    );
    assert_eq!(frame.quality(), Quality::Full);
    let counts = session.render_counts();
    assert_eq!(
        counts.requested - requested,
        61,
        "sixty drafts and one full render"
    );
    assert!(
        counts.superseded > 0,
        "stale drafts were dropped or cancelled: {counts:?}"
    );
    assert_eq!(counts.rendered + counts.superseded, counts.requested);
    assert_eq!(counts.failed, 0);

    let history = studio.engine.history(dawn).unwrap();
    assert_eq!(history.len(), steps + 1, "one history step");
    assert_eq!(history.last().unwrap().label, "Exposure +0.60");
    assert_eq!(
        studio.engine.summary(dawn).unwrap().develop_version,
        newest + 1,
        "one catalog write"
    );
    assert_eq!(studio.engine.undo_label(), Some("Exposure +0.60"));
    assert!(studio.toasts().is_empty(), "slider commits are not toasted");

    studio.engine.undo().unwrap();
    assert_eq!(
        studio.engine.develop().unwrap().settings().tone.exposure,
        0.0,
        "one undo entry"
    );
    assert_eq!(studio.engine.undo_label(), None);
    assert_eq!(
        studio.engine.history(dawn).unwrap().len(),
        steps,
        "undo takes the step back"
    );
}

#[test]
fn undo_and_redo_restore_the_exact_settings_and_render_again() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Exposure, 0.37).unwrap();
    studio.engine.commit_edit().unwrap();
    let first = studio.engine.develop().unwrap().settings().clone();
    studio.engine.adjust(Adj::Contrast, 23.0).unwrap();
    studio.engine.adjust(Adj::Clarity, -12.0).unwrap();
    studio.engine.commit_edit().unwrap();
    let second = studio.engine.develop().unwrap().settings().clone();
    studio.settle();

    studio.engine.undo().unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.settings(), &first);
    assert_eq!(studio.engine.develop_settings(dawn).unwrap(), first);
    assert_eq!(session.frame().unwrap().version(), session.version());
    let undone = mean_luma(session.frame().unwrap().image());

    studio.engine.redo().unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.settings(), &second);
    assert_eq!(session.frame().unwrap().version(), session.version());
    assert_ne!(mean_luma(session.frame().unwrap().image()), undone);
}

#[test]
fn a_thumbnail_changes_after_an_edit() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    studio.engine.thumbnail(dawn);
    studio.settle();
    let before = studio.engine.thumbnail(dawn).ready().cloned().unwrap();

    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Exposure, 2.0).unwrap();
    studio.engine.commit_edit().unwrap();
    assert!(studio.engine.thumbnail(dawn).ready().is_none());
    studio.settle();
    let after = studio.engine.thumbnail(dawn).ready().cloned().unwrap();
    assert!(after.version() > before.version());
    assert!(
        mean_luma(after.image()) > mean_luma(before.image()) + 20.0,
        "{} -> {}",
        mean_luma(before.image()),
        mean_luma(after.image())
    );
    let standard = studio.engine.standard_preview(dawn);
    assert!(standard.ready().is_none());
    studio.settle();
    assert_eq!(
        studio.engine.load_state(dawn, PreviewSize::Standard),
        photo_engine::LoadState::Ready {
            origin: photo_engine::PreviewOrigin::Rendered
        }
    );
}

#[test]
fn discrete_edits_masks_and_cancelling() {
    let (_library, mut studio) = studio();
    let dawn = studio.id("dawn.jpg");
    open_rendered(&mut studio, dawn, FIT);
    let mut settings = studio.engine.develop().unwrap().settings().clone();
    settings.masks.push(Mask::from_shape(
        MaskShape::Linear {
            start: Point::new(0.5, 0.0),
            end: Point::new(0.5, 0.6),
        },
        1,
    ));
    studio.engine.apply_settings(settings).unwrap();
    assert_eq!(
        studio.engine.history(dawn).unwrap().last().unwrap().label,
        "Add Mask"
    );

    studio
        .engine
        .adjust_local(0, LocalAdj::Exposure, 9.0)
        .unwrap();
    studio.engine.commit_edit().unwrap();
    let held = studio.engine.develop_settings(dawn).unwrap().masks[0]
        .adjust
        .exposure;
    assert_eq!(held, 4.0, "held to the local slider's range");
    assert!(matches!(
        studio.engine.adjust_local(3, LocalAdj::Exposure, 1.0),
        Err(EngineError::NoSuchMask(3))
    ));

    studio.engine.adjust(Adj::Shadows, 40.0).unwrap();
    assert!(studio.engine.cancel_edit());
    let session = studio.engine.develop().unwrap();
    assert!(!session.is_editing());
    assert_eq!(session.settings().tone.shadows, 0.0);
    assert!(
        !studio.engine.commit_edit().unwrap(),
        "nothing left to commit"
    );

    studio.engine.adjust(Adj::Temperature, 30.0).unwrap();
    let balance = studio.engine.develop().unwrap().settings().white_balance;
    assert_eq!((balance.temp, balance.tint), (30.0, 0.0));
    assert_eq!(balance.preset, photo_core::develop::WbPreset::Custom);
    studio.engine.close_develop();
    assert!(studio.engine.develop().is_none());
    assert_eq!(
        studio.engine.develop_settings(dawn).unwrap().white_balance,
        balance,
        "closing commits"
    );
}

#[test]
fn opening_another_photo_commits_and_remembers_the_previous_one() {
    let (_library, mut studio) = studio();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    open_rendered(&mut studio, dawn, FIT);
    studio.engine.adjust(Adj::Vibrance, 25.0).unwrap();
    open_rendered(&mut studio, lake, FIT);
    assert_eq!(
        studio
            .engine
            .develop_settings(dawn)
            .unwrap()
            .presence
            .vibrance,
        25.0
    );
    assert_eq!(studio.engine.previous_photo(), Some(dawn));
    assert!(matches!(
        Studio::new().engine.adjust(Adj::Exposure, 1.0),
        Err(EngineError::NoDevelopSession)
    ));
}

#[test]
fn dropping_the_engine_with_renders_in_flight_returns_promptly() {
    let (_files, files) = crate::common::many_jpegs(1, 1600, 1200);
    let mut studio = Studio::new();
    studio.import(files);
    let photo: PhotoId = studio.ids()[0];
    studio.engine.open_develop(photo).unwrap();
    studio.engine.set_canvas_view(CanvasView::Fit {
        width: 1600,
        height: 1200,
    });
    studio.engine.wait_idle(Duration::from_millis(20));
    for step in 0..10 {
        let _ = studio.engine.adjust(Adj::Exposure, step as f32 / 10.0);
    }
    let start = Instant::now();
    drop(studio);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
}
