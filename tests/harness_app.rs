//! The app behind the agent harness: photos imported through the real
//! dialog, observed, rated by a key press, opened in the loupe, a slider
//! dragged, real GPU frames measured, and a sandbox that holds everything
//! the session writes.

mod support;

use std::collections::BTreeSet;
use std::path::Path;

use photo::harness::StopReason;
use photo_testkit::TempLibrary;
use serde_json::{Value, json};
use support::{Client, Live, listing};

/// The file names of the fixture library's photos.
fn names(library: &TempLibrary) -> BTreeSet<String> {
    library
        .entries()
        .iter()
        .map(|entry| {
            entry
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// Import the fixture library through the dialog and wait for it.
fn import(client: &mut Client, library: &TempLibrary) -> Value {
    let imported = client.ok(
        "import_folder",
        json!({ "path": library.root(), "recursive": true }),
    );
    client.ok("wait_idle", json!({ "timeout_ms": 120_000 }));
    imported
}

#[test]
fn a_folder_imported_through_the_dialog_is_observed_and_3_rates_the_active_photo() {
    let library = TempLibrary::new();
    let live = Live::start();
    let mut client = live.client();

    let before = client.ok("observe", json!({ "controls": false }));
    assert_eq!(before["photos"]["all"], 0);
    assert_eq!(before["module"], "library");
    assert_eq!(before["view"], "grid");

    let imported = import(&mut client, &library);
    assert_eq!(imported["found"], 6, "{imported}");
    assert_eq!(imported["ticked"], 6);
    assert_eq!(imported["import_started"], true);
    assert_eq!(imported["pressed"], "Import 6 Photos");

    let seen = client.ok("observe", json!({}));
    assert_eq!(seen["dialog_open"], false);
    assert_eq!(seen["photos"]["shown"], 6, "{}", seen["photos"]);
    assert_eq!(seen["source"]["kind"], "previous_import");
    let active = &seen["active_photo"];
    assert!(
        active["id"].is_i64(),
        "an import makes its first photo active: {active}"
    );
    assert_eq!(seen["idle"], true);
    let controls = seen["controls"].as_array().expect("controls are listed");
    let cells: BTreeSet<String> = controls
        .iter()
        .filter(|control| control["role"] == "ListBoxOption")
        .filter_map(|control| control["name"].as_str().map(str::to_owned))
        .filter(|name| names(&library).contains(name))
        .collect();
    assert_eq!(cells, names(&library), "every photo has a grid cell");
    let import_button = controls
        .iter()
        .find(|control| control["name"] == "Import…")
        .expect("the Import button is listed");
    assert_eq!(import_button["role"], "Button");
    assert_eq!(import_button["enabled"], true);
    assert_eq!(import_button["rect"].as_array().map(Vec::len), Some(4));

    let page = client.ok("observe_library", json!({ "offset": 0, "limit": 10 }));
    assert_eq!(page["total"], 6);
    let listed: BTreeSet<String> = page["photos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|photo| photo["file_name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed, names(&library));
    let second_page = client.ok("observe_library", json!({ "offset": 4, "limit": 10 }));
    assert_eq!(second_page["photos"].as_array().map(Vec::len), Some(2));

    client.ok("key", json!({ "name": "3" }));
    let rated = client.ok("observe", json!({ "controls": false }));
    assert_eq!(rated["active_photo"]["id"], active["id"]);
    assert_eq!(
        rated["active_photo"]["rating"], 3,
        "{}",
        rated["active_photo"]
    );
    let toasts: Vec<&str> = rated["toasts"]["recent"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|toast| toast["message"].as_str())
        .collect();
    assert!(toasts.iter().any(|toast| toast.contains('3')), "{toasts:?}");

    let again = client.ok(
        "import_folder",
        json!({ "path": library.root(), "recursive": true }),
    );
    assert_eq!(again["import_started"], false, "{again}");
    assert_eq!(again["already_in_catalog"], 6);
    assert_eq!(again["ticked"], 0);
    let after = client.ok("observe", json!({ "controls": false }));
    assert_eq!(after["dialog_open"], false, "the dialog was cancelled");
    assert_eq!(after["photos"]["all"], 6);
}

#[test]
fn a_screenshot_is_a_real_png_with_sane_statistics_and_no_difference_from_itself() {
    let live = Live::start();
    let mut client = live.client();
    let answer = client.call("screenshot", json!({}));
    if answer["ok"] == false {
        assert_eq!(answer["error"]["code"], "unavailable", "{answer}");
        eprintln!("skipped: no GPU adapter on this machine ({answer})");
        return;
    }
    let shot = &answer["data"];
    let relative = shot["path"].as_str().unwrap();
    assert!(relative.starts_with("artifacts/"), "{relative}");
    let file = live.run_dir.join(relative);
    let decoded = image::open(&file)
        .expect("the screenshot is an image")
        .to_rgba8();
    assert_eq!(
        (shot["width"].as_u64(), shot["height"].as_u64()),
        (Some(1600), Some(1000))
    );
    assert_eq!(decoded.dimensions(), (1600, 1000));
    assert_eq!(shot["pixels_per_point"], 1.0);

    let stats = client.ok("image_stats", json!({ "path": relative }));
    assert_eq!(stats["pixels"], 1_600_000);
    for channel in ["r", "g", "b", "luma"] {
        let c = &stats["channels"][channel];
        let order: Vec<f64> = ["min", "p1", "p5", "p50", "p95", "p99", "max"]
            .iter()
            .map(|key| c[*key].as_f64().unwrap())
            .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] <= pair[1]),
            "{channel}: {c}"
        );
        assert!(order[0] >= 0.0 && order[6] <= 255.0, "{channel}: {c}");
        for clip in ["clip_low_fraction", "clip_high_fraction"] {
            assert!(
                (0.0..=1.0).contains(&c[clip].as_f64().unwrap()),
                "{channel}: {c}"
            );
        }
    }
    let histogram: Vec<f64> = stats["luma_histogram"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bin| bin.as_f64().unwrap())
        .collect();
    assert_eq!(histogram.len(), 64);
    assert!((histogram.iter().sum::<f64>() - 1.0).abs() < 1e-3);
    // A neutral dark interface (design §8.8): mostly dark and grey.
    assert!(
        stats["channels"]["luma"]["mean"].as_f64().unwrap() < 100.0,
        "{stats}"
    );
    assert!(stats["mean_saturation"].as_f64().unwrap() < 0.2, "{stats}");

    let same = client.ok(
        "compare_images",
        json!({ "path_a": relative, "path_b": relative }),
    );
    assert_eq!(same["identical"], true);
    assert_eq!(same["differing_fraction"], 0.0);
    for channel in ["r", "g", "b"] {
        assert_eq!(same["channels"][channel]["max_abs_diff"], 0);
    }
    let manifest = live.json_file("manifest.json");
    assert_eq!(manifest["artifacts"][0]["path"], relative);
    assert_eq!(manifest["artifacts"][0]["kind"], "screenshot");
}

#[test]
fn a_sandboxed_session_writes_nothing_outside_its_run_directory() {
    let library = TempLibrary::new();
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
    let real_stores: Vec<_> = ["catalog", "edited"]
        .iter()
        .map(|name| checkout.join(name))
        .collect();
    let pictures = directories::UserDirs::new()
        .and_then(|dirs| dirs.picture_dir().map(|dir| dir.join("Photo")));
    let watched = || {
        let mut all = listing(library.root());
        for store in &real_stores {
            all.extend(listing(store));
        }
        if let Some(pictures) = &pictures {
            all.extend(listing(pictures));
        }
        all
    };
    let before = watched();
    assert!(
        before.len() >= 9,
        "the listing sees the library's two folders and seven files"
    );

    let mut live = Live::start();
    let mut client = live.client();
    import(&mut client, &library);
    client.ok("key", json!({ "name": "3" }));
    client.ok("key", json!({ "name": "p" }));
    let _ = client.call("screenshot", json!({}));
    let ping = client.ok("ping", json!({}));
    client.ok("quit", json!({}));
    assert_eq!(live.join(), Ok(StopReason::Quit));

    let storage = Path::new(ping["storage_root"].as_str().unwrap());
    assert!(
        storage.starts_with(&live.run_dir),
        "{} is outside the run",
        storage.display()
    );
    let catalog = storage.join("catalog").join("Photo Catalog.photocat");
    assert!(
        catalog.is_file(),
        "the catalog was written into the sandbox"
    );
    let previews = listing(&storage.join("catalog").join("Photo Catalog.previews"));
    assert!(
        previews.iter().any(|(_, size, _)| *size > 0),
        "previews were written into the sandbox"
    );
    let runs: Vec<_> = std::fs::read_dir(live.base.path().join("runs"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        runs,
        [live.run_dir.clone()],
        "the runs folder holds this run only"
    );
    assert_eq!(
        std::fs::read_dir(live.base.path()).unwrap().count(),
        1,
        "nothing beside the runs folder"
    );
    assert_eq!(
        watched(),
        before,
        "the photos' folder and the real stores are untouched"
    );
}

#[test]
fn open_shows_a_photo_in_the_loupe_at_startup_and_later_in_the_app_as_it_is() {
    let library = TempLibrary::new();
    let path_of = |name: &str| {
        library
            .entries()
            .iter()
            .find(|entry| entry.path.ends_with(name))
            .map(|entry| entry.path.clone())
            .unwrap()
    };
    let dawn = path_of("dawn.jpg");
    let live = Live::start_with(Some(dawn.clone()));
    let mut client = live.client();
    let seen = client.ok("observe", json!({ "controls": false }));
    assert_eq!(seen["module"], "library");
    assert_eq!(seen["view"], "loupe");
    assert_eq!(seen["active_photo"]["file_name"], "dawn.jpg");
    assert_eq!(seen["photos"]["all"], 1);

    // What starting the app again would lose: a step to undo and a panel
    // that was closed.
    client.ok("key", json!({ "name": "3" }));
    client.ok(
        "click",
        json!({ "name": "Folders", "role": "DisclosureTriangle" }),
    );
    let before = client.ok("observe", json!({ "controls": false }));
    assert_eq!(before["undo"]["undo"], "Set Rating to 3");
    let open_panels = |seen: &Value| -> Vec<String> {
        seen["panels"]["left"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|panel| panel.as_str().map(str::to_owned))
            .collect()
    };
    assert!(!open_panels(&before).contains(&"Folders".to_owned()));
    assert!(open_panels(&before).contains(&"Catalog".to_owned()));

    let opened = client.ok("open", json!({ "path": path_of("lake.png") }));
    assert_eq!(opened["opened"]["file_name"], "lake.png");
    assert_eq!(opened["opened"]["imported"], true);
    assert_eq!(opened["module"], "library");
    assert_eq!(opened["view"], "loupe");
    let after = client.ok("observe", json!({ "controls": false }));
    assert_eq!(after["active_photo"]["file_name"], "lake.png");
    assert_eq!(after["active_photo"]["id"], opened["opened"]["id"]);
    assert_eq!(after["selection"]["count"], 1);
    assert_eq!(
        after["undo"]["undo"], "Set Rating to 3",
        "the undo stack is as it was"
    );
    assert_eq!(after["panels"], before["panels"], "and so are the panels");

    let again = client.ok("open", json!({ "path": dawn }));
    assert_eq!(
        again["opened"]["imported"], false,
        "dawn.jpg was in the catalog already"
    );
    let seen = client.ok("observe", json!({ "controls": false }));
    assert_eq!(seen["view"], "loupe");
    assert_eq!(seen["active_photo"]["file_name"], "dawn.jpg");
    assert_eq!(seen["active_photo"]["rating"], 3);
    assert_eq!(seen["photos"]["all"], 2);
    assert_eq!(
        client.fails("open", json!({ "path": library.stray_file() })),
        "invalid_params"
    );
    assert_eq!(
        client.fails("open", json!({ "path": library.root().join("gone.jpg") })),
        "not_found"
    );
    let unchanged = client.ok("observe", json!({ "controls": false }));
    assert_eq!(unchanged["active_photo"]["file_name"], "dawn.jpg");

    // While developing, the photo opened is the one developed.
    client.ok("key", json!({ "name": "d" }));
    client.ok("wait_idle", json!({ "timeout_ms": 120_000 }));
    let developing = client.ok("open", json!({ "path": path_of("lake.png") }));
    assert_eq!(developing["module"], "develop");
    client.ok("wait_idle", json!({ "timeout_ms": 120_000 }));
    let seen = client.ok("observe", json!({ "controls": false }));
    assert_eq!(seen["module"], "develop");
    assert_eq!(seen["active_photo"]["file_name"], "lake.png");
    assert!(
        seen["active_photo"]["load"]["develop"]["state"].is_string(),
        "a develop session is open on it: {}",
        seen["active_photo"]
    );
}

#[test]
fn set_slider_drags_a_slider_to_its_value_and_lets_go() {
    let library = TempLibrary::new();
    let live = Live::start();
    let mut client = live.client();
    import(&mut client, &library);
    let set = client.ok(
        "set_slider",
        json!({ "name": "Thumbnail size", "value": 250 }),
    );
    let value = set["value"].as_f64().unwrap();
    assert!((value - 250.0).abs() < 1.0, "{set}");
    let seen = client.ok("observe", json!({}));
    let slider = seen["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|control| control["name"] == "Thumbnail size")
        .cloned()
        .unwrap();
    assert!(
        (slider["numeric"].as_f64().unwrap() - 250.0).abs() < 1.0,
        "{slider}"
    );
    assert_eq!(
        seen["pointer"].as_array().map(Vec::len),
        Some(2),
        "the pointer is where it let go"
    );
    let clamped = client.ok(
        "set_slider",
        json!({ "name": "Thumbnail size", "value": 10_000 }),
    );
    assert_eq!(clamped["target"], 320.0, "held to the slider's range");
    assert_eq!(
        client.fails("set_slider", json!({ "name": "Exposure", "value": 1 })),
        "not_found"
    );
}
