//! `dump_render`: the active photo through the develop pipeline, measured at
//! full resolution.

mod support;

use photo::harness::run::STORAGE_DIR;
use photo::harness::stats;
use photo::repo::Storage;
use photo_engine::StillSettings;
use photo_testkit::TempLibrary;
use photo_ui::AppOptions;
use serde_json::{Value, json};
use support::Live;

/// The fixture photo called `name`.
fn fixture(library: &TempLibrary, name: &str) -> std::path::PathBuf {
    library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with(name))
        .map(|entry| entry.path.clone())
        .expect("the fixture is in the library")
}

/// The render's size as `[width, height]`.
fn size(render: &Value) -> (u64, u64) {
    (
        render["render"]["width"].as_u64().unwrap(),
        render["render"]["height"].as_u64().unwrap(),
    )
}

#[test]
fn a_render_has_the_statistics_of_a_direct_render_at_full_resolution() {
    let library = TempLibrary::new();
    let live = Live::start_with(Some(fixture(&library, "dawn.jpg")));
    let mut client = live.client();
    let dumped = client.ok("dump_render", json!({ "source": "current", "max_dim": 32 }));
    assert_eq!(dumped["photo"]["file_name"], "dawn.jpg");
    assert_eq!(size(&dumped), (96, 64), "the full-resolution render");

    let artifact = live.run_dir.join(dumped["path"].as_str().unwrap());
    let png = image::open(&artifact).expect("the render's PNG").to_rgba8();
    assert_eq!(png.dimensions(), (32, 21), "scaled down to max_dim");
    assert_eq!(dumped["artifact"]["width"], 32);
    let manifest = live.json_file("manifest.json");
    assert_eq!(manifest["artifacts"][0]["kind"], "render");
    assert_eq!(manifest["artifacts"][0]["render_size"], json!([96, 64]));

    // The same photo rendered straight through the engine, on the
    // session's catalog, measured by the same statistics.
    let storage = Storage::under(&live.run_dir.join(STORAGE_DIR));
    let engine = AppOptions {
        catalog_path: storage.catalog,
        previews_dir: storage.previews,
        export_dir: storage.exports,
    }
    .open_engine()
    .expect("the session's catalog opens");
    let dawn = engine
        .photos()
        .iter()
        .find(|photo| photo.file_name == "dawn.jpg")
        .map(|photo| photo.id)
        .expect("dawn.jpg was imported");
    let direct = engine
        .render_photo_rgba8(dawn, StillSettings::Current, None)
        .expect("the photo renders");
    let direct = image::RgbaImage::from_raw(direct.width, direct.height, direct.pixels).unwrap();
    let expected = stats::image_stats(&direct).unwrap().to_json();
    assert_eq!(
        dumped["stats"], expected,
        "the statistics are the full render's"
    );
    assert_eq!(dumped["stats"]["pixels"], 96 * 64);

    let unscaled = client.ok("dump_render", json!({ "max_dim": 4096 }));
    assert_eq!(
        unscaled["artifact"]["width"], 96,
        "a small render is not enlarged"
    );
    assert_eq!(unscaled["source"], "current", "current is the default");
}

#[test]
fn the_original_differs_from_the_current_render_after_an_edit_through_the_ui() {
    let library = TempLibrary::new();
    let live = Live::start_with(Some(fixture(&library, "lake.png")));
    let mut client = live.client();
    let original = client.ok("dump_render", json!({ "source": "original" }));
    let current = client.ok("dump_render", json!({ "source": "current" }));
    assert_eq!(size(&original), (72, 48));
    assert_eq!(current["stats"], original["stats"], "nothing is edited yet");

    // Rotation is a develop edit; the toolbar button is how a user makes it.
    client.ok("click", json!({ "name": "Rotate right", "role": "Button" }));
    client.ok("wait_idle", json!({}));
    let turned = client.ok("dump_render", json!({ "source": "current" }));
    let untouched = client.ok("dump_render", json!({ "source": "original" }));
    assert_eq!(size(&turned), (48, 72), "the current settings are turned");
    assert_eq!(size(&untouched), (72, 48), "the defaults are not");
    assert_eq!(untouched["stats"], original["stats"]);
    let paths: Vec<&str> = [&turned, &untouched]
        .iter()
        .map(|render| render["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        client.fails(
            "compare_images",
            json!({ "path_a": paths[0], "path_b": paths[1] })
        ),
        "invalid_params",
        "a 48 x 72 render and a 72 x 48 one are not the same image"
    );
}
