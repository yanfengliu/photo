//! Views asked for while the photo open in Develop is still loading. The
//! one worker is held, so the request certainly comes before the decode.

use super::{PATIENCE, engine_with, hold_worker, jpegs};
use crate::develop::{BeforeSettings, CanvasView, DevelopLoad};
use crate::looks::{BuiltInPreset, PresetKey};

const MONO: PresetKey = PresetKey::BuiltIn(BuiltInPreset::MonoContrast);
const CANVAS: CanvasView = CanvasView::Fit {
    width: 96,
    height: 64,
};

#[test]
fn a_preset_preview_asked_for_while_the_photo_loads_arrives_once_it_is_decoded() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    let release = hold_worker(&mut engine);
    engine.open_develop(ids[0]).unwrap();
    engine.set_canvas_view(CANVAS);
    engine.preview_preset(Some(MONO), 48, 48).unwrap();
    // A pointer resting on the preset asks again with every frame.
    engine.preview_preset(Some(MONO), 48, 48).unwrap();
    let session = engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Nothing);
    assert!(session.preset_preview().is_none());
    assert_eq!(
        engine.pending_renders(),
        0,
        "there is nothing to render yet"
    );

    release.send(()).unwrap();
    assert!(engine.wait_idle(PATIENCE));
    let session = engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Rendered);
    let (key, frame) = session
        .preset_preview()
        .expect("the preview asked for while the photo was loading");
    assert_eq!(*key, MONO);
    assert_eq!((frame.image().width, frame.image().height), (48, 32));

    // The same request once more asks for nothing and loses nothing.
    engine.preview_preset(Some(MONO), 48, 48).unwrap();
    assert!(engine.is_idle());
    assert!(engine.develop().unwrap().preset_preview().is_some());
}

#[test]
fn a_preset_preview_does_not_wait_for_the_canvas_to_have_a_view() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    engine.open_develop(ids[0]).unwrap();
    assert!(engine.wait_idle(PATIENCE));
    assert_eq!(
        engine.develop().unwrap().load_state(),
        &DevelopLoad::Decoded
    );
    engine.preview_preset(Some(MONO), 48, 48).unwrap();
    assert!(engine.wait_idle(PATIENCE));
    let session = engine.develop().unwrap();
    assert!(session.frame().is_none(), "the canvas has no view yet");
    assert!(
        session.preset_preview().is_some(),
        "the preview has a size of its own"
    );
}

#[test]
fn the_before_view_turned_on_while_the_photo_loads_arrives_once_it_is_decoded() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    let release = hold_worker(&mut engine);
    engine.open_develop(ids[0]).unwrap();
    engine.set_canvas_view(CANVAS);
    engine.set_before(Some(BeforeSettings::Default)).unwrap();
    engine.set_before(Some(BeforeSettings::Default)).unwrap();
    let session = engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Nothing);
    assert!(session.before_frame().is_none());
    assert_eq!(
        engine.pending_renders(),
        0,
        "there is nothing to render yet"
    );

    release.send(()).unwrap();
    assert!(engine.wait_idle(PATIENCE));
    let session = engine.develop().unwrap();
    assert_eq!(session.before_settings(), Some(BeforeSettings::Default));
    let before = session
        .before_frame()
        .expect("the before view turned on while the photo was loading");
    assert_eq!((before.image().width, before.image().height), (96, 64));
}

#[test]
fn the_before_view_turned_on_before_the_canvas_has_a_view_arrives_with_the_view() {
    let (_files_dir, files) = jpegs(1, 96, 64);
    let (_dir, mut engine, ids) = engine_with(&files, 1);
    let release = hold_worker(&mut engine);
    engine.open_develop(ids[0]).unwrap();
    engine.set_before(Some(BeforeSettings::Default)).unwrap();
    release.send(()).unwrap();
    assert!(engine.wait_idle(PATIENCE));
    let session = engine.develop().unwrap();
    assert_eq!(session.load_state(), &DevelopLoad::Decoded);
    assert!(session.before_frame().is_none(), "no view to render it for");

    engine.set_canvas_view(CANVAS);
    assert!(engine.wait_idle(PATIENCE));
    assert!(engine.develop().unwrap().before_frame().is_some());
}
