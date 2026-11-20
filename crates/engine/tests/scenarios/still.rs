//! Still renders, as the automation harness asks for them through a shared
//! reference to the engine.

use std::path::Path;

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{Adj, DevelopSettings};
use photo_core::image::{Rgb16Image, Rgba8Image};
use photo_core::meta::{FileKind, PhotoId};
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source};
use photo_engine::{EngineError, StillSettings, Target};

use crate::common::Studio;

/// The pipeline's own render of `file` with `settings`, at full size or
/// fitted inside a square of `edge`.
fn direct(file: &Path, settings: &DevelopSettings, edge: Option<u32>) -> (Rgba8Image, Rgb16Image) {
    let source = Source::new(photo_decode::decode(file).unwrap().image);
    let (width, height) = photo_develop::output_size(&source, settings);
    let (box_width, box_height) = edge.map_or((width, height), |edge| (edge, edge));
    let request = RenderRequest {
        source: &source,
        settings,
        view: photo_develop::fit_view(&source, settings, box_width, box_height),
        quality: Quality::Full,
        space: RgbSpace::Srgb,
    };
    let cancel = CancelToken::new();
    let rgba = Renderer::new().render_rgba8(&request, &cancel).unwrap();
    let rgb16 = Renderer::new().render_rgb16(&request, &cancel).unwrap();
    (rgba, rgb16)
}

#[test]
fn a_still_render_matches_the_pipelines_own_render_of_the_same_source_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    let file = photo_testkit::write_png8(dir.path(), "scene.png", 480, 320);
    let mut studio = Studio::new();
    studio.import(vec![file.clone()]);
    let photo = studio.ids()[0];
    let target = || Target::Photos(vec![photo]);
    studio
        .engine
        .quick_adjust(target(), Adj::Exposure, 0.7)
        .unwrap();
    studio.engine.rotate_right(target()).unwrap();
    let edited = studio.engine.develop_settings(photo).unwrap();

    let engine = &studio.engine;
    let full = engine
        .render_photo_rgba8(photo, StillSettings::Current, None)
        .unwrap();
    let (expected, expected_deep) = direct(&file, &edited, None);
    assert_eq!((full.width, full.height), (320, 480), "full size, turned");
    assert_eq!(full.pixels, expected.pixels);
    let deep = engine
        .render_photo_rgb16(photo, StillSettings::Current, None)
        .unwrap();
    assert_eq!((deep.width, deep.height), (320, 480));
    assert_eq!(deep.pixels, expected_deep.pixels);

    let defaults = DevelopSettings::default_for(FileKind::Raster);
    let small = engine
        .render_photo_rgba8(photo, StillSettings::Default, Some(120))
        .unwrap();
    assert_eq!((small.width, small.height), (120, 80), "capped, upright");
    assert_eq!(small.pixels, direct(&file, &defaults, Some(120)).0.pixels);
    assert!(matches!(
        engine.render_photo_rgba8(PhotoId(9999), StillSettings::Current, None),
        Err(EngineError::Catalog(_))
    ));

    studio.engine.open_develop(photo).unwrap();
    studio.engine.adjust(Adj::Contrast, 30.0).unwrap();
    let shown = studio.engine.develop().unwrap().settings().clone();
    assert_ne!(shown, edited, "an edit not yet committed");
    let live = studio
        .engine
        .render_photo_rgba8(photo, StillSettings::Current, Some(200))
        .unwrap();
    assert_eq!(live.pixels, direct(&file, &shown, Some(200)).0.pixels);
}
