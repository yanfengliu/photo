//! The Develop tools: the before view, auto tone, auto white balance and
//! the white-balance picker.

use std::sync::Arc;

use photo_core::develop::{Adj, Orientation, WbPreset};
use photo_core::geom::Point;
use photo_core::meta::PhotoId;
use photo_engine::{BeforeSettings, CanvasView, DevelopFrame, EngineError};
use tempfile::TempDir;

use crate::common::{Studio, mean_rgb, open_rendered};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const FIT: CanvasView = CanvasView::Fit {
    width: WIDTH,
    height: HEIGHT,
};

/// A studio holding one lossless 480 x 320 PNG of the scene, open in
/// Develop at 1:1.
fn studio_with_png() -> (TempDir, Studio, PhotoId) {
    let dir = tempfile::tempdir().unwrap();
    let file = photo_testkit::write_png8(dir.path(), "scene.png", WIDTH, HEIGHT);
    let mut studio = Studio::new();
    studio.import(vec![file]);
    let photo = studio.ids()[0];
    open_rendered(&mut studio, photo, FIT);
    (dir, studio, photo)
}

/// A studio holding one 480 x 320 DNG of the scene, open in Develop.
fn studio_with_dng() -> (TempDir, Studio, PhotoId) {
    let dir = tempfile::tempdir().unwrap();
    let file = photo_testkit::write_dng(dir.path(), "scene.dng", WIDTH, HEIGHT);
    let mut studio = Studio::new();
    studio.import(vec![file]);
    let photo = studio.ids()[0];
    open_rendered(&mut studio, photo, FIT);
    (dir, studio, photo)
}

fn frame(studio: &Studio) -> DevelopFrame {
    studio.engine.develop().unwrap().frame().unwrap().clone()
}

/// The centre of the grey ramp, where it is mid grey, in output pixels.
fn ramp_middle() -> Point {
    let (x, y, width, height) = photo_testkit::grey_ramp(WIDTH, HEIGHT);
    Point::new((x + width / 2) as f32, (y + height / 2) as f32)
}

/// The code value half the frame's pixels are darker than, by luma.
fn median(frame: &DevelopFrame) -> f64 {
    let histogram = frame.histogram();
    let half = histogram.pixel_count() / 2;
    let mut seen = 0;
    for (value, count) in histogram.luma.iter().enumerate() {
        seen += u64::from(*count);
        if seen > half {
            return value as f64;
        }
    }
    255.0
}

/// The red minus blue cast of the frame's mid-grey ramp, in code values.
fn cast(frame: &DevelopFrame) -> f64 {
    let at = ramp_middle();
    let [red, _, blue] = mean_rgb(frame.image(), at.x as u32 - 2, at.y as u32 - 2, 5, 5);
    red - blue
}

#[test]
fn the_before_view_keeps_its_frame_while_the_edit_changes_and_follows_its_geometry() {
    let (_dir, mut studio, photo) = studio_with_png();
    let untouched = frame(&studio);
    studio
        .engine
        .set_before(Some(BeforeSettings::Default))
        .unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert_eq!(session.before_settings(), Some(BeforeSettings::Default));
    let before = session.before_frame().unwrap().clone();
    assert_eq!(
        before.image().pixels,
        untouched.image().pixels,
        "the default settings"
    );

    studio.engine.adjust(Adj::Exposure, 1.0).unwrap();
    studio.engine.commit_edit().unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    assert!(
        Arc::ptr_eq(session.before_frame().unwrap().image(), before.image()),
        "not rendered again"
    );
    assert_ne!(
        session.frame().unwrap().image().pixels,
        before.image().pixels
    );

    let snapshot = studio.engine.create_snapshot(photo, "Bright").unwrap();
    let mut turned = studio.engine.develop().unwrap().settings().clone();
    turned.orientation = Orientation::new(1, false);
    turned.tone.exposure = -1.0;
    studio.engine.apply_settings(turned).unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    let (after, before) = (session.frame().unwrap(), session.before_frame().unwrap());
    assert_eq!(
        before.view(),
        after.view(),
        "the before view is laid out like the edit"
    );
    assert_eq!(
        (after.image().width, after.image().height),
        (213, 320),
        "turned"
    );

    studio
        .engine
        .set_before(Some(BeforeSettings::Snapshot(snapshot)))
        .unwrap();
    studio.settle();
    let session = studio.engine.develop().unwrap();
    let shown = session.before_frame().unwrap();
    let edited = session.frame().unwrap();
    let luma = |frame: &DevelopFrame| crate::common::mean_luma(frame.image());
    assert!(
        luma(shown) > luma(edited) + 20.0,
        "the snapshot is the brighter one"
    );

    let steps = studio.engine.history(photo).unwrap();
    let first = steps.first().unwrap().id;
    studio
        .engine
        .set_before(Some(BeforeSettings::HistoryStep(first)))
        .unwrap();
    studio.settle();
    assert!(studio.engine.develop().unwrap().before_frame().is_some());
    studio.engine.set_before(None).unwrap();
    assert!(studio.engine.develop().unwrap().before_frame().is_none());
}

#[test]
fn auto_tone_sets_the_tone_from_the_photo_and_commits_once() {
    let (_dir, mut studio, photo) = studio_with_png();
    studio.engine.adjust(Adj::Exposure, -2.0).unwrap();
    studio.engine.adjust(Adj::Vibrance, 25.0).unwrap();
    studio.engine.commit_edit().unwrap();
    studio.settle();
    let dark = frame(&studio);
    let before = studio.engine.develop_settings(photo).unwrap();

    studio.engine.auto_tone().unwrap();
    studio.settle();
    let after = studio.engine.develop_settings(photo).unwrap();
    assert!(after.tone.exposure > -2.0, "{:?}", after.tone);
    assert_eq!(
        after.tone.contrast, before.tone.contrast,
        "contrast is kept"
    );
    assert_eq!(after.presence, before.presence, "only the tone changes");
    let lifted = frame(&studio);
    println!("median {} -> {}", median(&dark), median(&lifted));
    assert!(
        median(&lifted) > median(&dark) + 40.0,
        "{} -> {}",
        median(&dark),
        median(&lifted)
    );
    assert!(
        (90.0..150.0).contains(&median(&lifted)),
        "near middle grey: {}",
        median(&lifted)
    );
    assert_eq!(
        studio.engine.history(photo).unwrap().last().unwrap().label,
        "Tone"
    );

    studio.engine.undo().unwrap();
    assert_eq!(studio.engine.develop_settings(photo).unwrap(), before);
}

#[test]
fn auto_white_balance_is_estimated_from_the_photo_whatever_the_setting() {
    let (dir, mut studio, photo) = studio_with_png();
    let decoded = photo_decode::decode(dir.path().join("scene.png")).unwrap();
    let estimate = photo_develop::auto_white_balance(&photo_develop::Source::new(decoded.image));
    assert_eq!(estimate.preset, WbPreset::Auto);
    for start in [60.0, -60.0] {
        studio.engine.adjust(Adj::Temperature, start).unwrap();
        studio.engine.adjust(Adj::Shadows, 10.0).unwrap();
        studio.engine.commit_edit().unwrap();
        let before = studio.engine.develop_settings(photo).unwrap();
        studio.engine.auto_white_balance().unwrap();
        let after = studio.engine.develop_settings(photo).unwrap();
        assert_eq!(after.white_balance, estimate);
        let mut expected = before.clone();
        expected.white_balance = estimate;
        assert_eq!(after, expected, "only the white balance changes");
        studio.engine.undo().unwrap();
        assert_eq!(studio.engine.develop_settings(photo).unwrap(), before);
    }
}

#[test]
fn auto_white_balance_takes_out_a_strong_cast() {
    let (_dir, mut studio, _photo) = studio_with_dng();
    let mut tungsten = studio.engine.develop().unwrap().settings().clone();
    let (temp, tint) = WbPreset::Tungsten.illuminant().unwrap();
    tungsten.white_balance = photo_core::develop::WhiteBalance {
        preset: WbPreset::Tungsten,
        temp,
        tint,
    };
    studio.engine.apply_settings(tungsten).unwrap();
    studio.settle();
    let blue = cast(&frame(&studio));
    studio.engine.auto_white_balance().unwrap();
    studio.settle();
    let corrected = cast(&frame(&studio));
    println!("tungsten cast {blue} -> auto {corrected}");
    assert!(
        blue < -40.0,
        "tungsten light set on daylight is blue: {blue}"
    );
    assert!(corrected.abs() < blue.abs() / 2.0, "{blue} -> {corrected}");
}

#[test]
fn the_picker_makes_the_picked_point_neutral() {
    let (_dir, mut studio, photo) = studio_with_png();
    studio.engine.adjust(Adj::Temperature, -50.0).unwrap();
    studio.engine.adjust(Adj::Tint, 30.0).unwrap();
    studio.engine.commit_edit().unwrap();
    studio.settle();
    let cool = cast(&frame(&studio));
    assert!(cool < -20.0, "{cool}");

    assert!(studio.engine.pick_white_balance(ramp_middle()).unwrap());
    studio.settle();
    let balance = studio.engine.develop_settings(photo).unwrap().white_balance;
    assert_eq!(balance.preset, WbPreset::Custom);
    let picked = frame(&studio);
    let at = ramp_middle();
    let [red, green, blue] = mean_rgb(picked.image(), at.x as u32 - 1, at.y as u32 - 1, 3, 3);
    assert!(
        (red - blue).abs() < 2.0 && (red - green).abs() < 2.0,
        "{red} {green} {blue}"
    );
    assert!(
        !studio
            .engine
            .pick_white_balance(Point::new(-10.0, 5.0))
            .unwrap(),
        "off the photo"
    );
}

#[test]
fn a_raw_photo_reads_its_as_shot_balance_and_the_picker_finds_it_again() {
    let (_dir, mut studio, _photo) = studio_with_dng();
    let session = studio.engine.develop().unwrap();
    let (kelvin, tint) = session.as_shot_white_balance().unwrap();
    assert!(
        (4500.0..5600.0).contains(&kelvin) && tint.abs() < 10.0,
        "{kelvin} {tint}"
    );
    let shot = cast(&frame(&studio));
    assert!(shot.abs() < 4.0, "as shot, grey is grey: {shot}");

    let mut tungsten = session.settings().clone();
    let (temp, tint) = WbPreset::Tungsten.illuminant().unwrap();
    tungsten.white_balance = photo_core::develop::WhiteBalance {
        preset: WbPreset::Tungsten,
        temp,
        tint,
    };
    studio.engine.apply_settings(tungsten).unwrap();
    studio.settle();
    assert!(
        cast(&frame(&studio)) < -20.0,
        "tungsten light set on daylight is blue"
    );

    studio.engine.pick_white_balance(ramp_middle()).unwrap();
    studio.settle();
    let picked = studio.engine.develop().unwrap().settings().white_balance;
    assert!(
        (picked.temp - kelvin).abs() < kelvin * 0.05,
        "{} vs {kelvin}",
        picked.temp
    );
    assert!(cast(&frame(&studio)).abs() < 3.0);
}

#[test]
fn the_tools_need_a_decoded_photo() {
    let mut studio = Studio::new();
    assert!(matches!(
        studio.engine.auto_tone(),
        Err(EngineError::NoDevelopSession)
    ));
    let (_dir, files) = crate::common::many_jpegs(1, 64, 48);
    studio.import(files);
    let photo = studio.ids()[0];
    studio.engine.open_develop(photo).unwrap();
    assert!(matches!(
        studio.engine.auto_white_balance(),
        Err(EngineError::StillLoading)
    ));
    studio.settle();
    studio.engine.auto_white_balance().unwrap();
}
