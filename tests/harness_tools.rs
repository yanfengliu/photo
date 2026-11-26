//! The Develop tools behind the agent harness: `observe` says where the
//! photo is on the canvas and which tool is open, a drag paints a brush
//! mask and the same drag with Alt held rubs it out, a drag of a corner
//! crops, a click places a healing spot, and Escape leaves no history step.

mod support;

use photo_testkit::TempLibrary;
use serde_json::{Value, json};
use support::{Client, Live};

/// The fixture photo called `name`.
fn fixture(library: &TempLibrary, name: &str) -> std::path::PathBuf {
    library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with(name))
        .map(|entry| entry.path.clone())
        .expect("the fixture is in the library")
}

/// The mean luma of the active photo's render at full resolution, with the
/// settings as they are now.
fn luma(client: &mut Client) -> f64 {
    client.ok("wait_idle", json!({}));
    let dumped = client.ok("dump_render", json!({}));
    dumped["stats"]["channels"]["luma"]["mean"]
        .as_f64()
        .unwrap_or_else(|| panic!("the render has statistics: {dumped}"))
}

/// What `observe` says without the controls.
fn seen(client: &mut Client) -> Value {
    client.ok("observe", json!({ "controls": false }))
}

/// A rectangle `observe` gave, as `[left, top, right, bottom]`.
fn edges(rect: &Value) -> [f64; 4] {
    let edges: Vec<f64> = rect
        .as_array()
        .unwrap_or_else(|| panic!("a rectangle, not {rect}"))
        .iter()
        .map(|edge| edge.as_f64().expect("a number"))
        .collect();
    edges.try_into().expect("four edges")
}

/// The point at fractions `(u, v)` of a rectangle, as a command takes it.
fn within(rect: [f64; 4], u: f64, v: f64) -> Value {
    json!([
        rect[0] + u * (rect[2] - rect[0]),
        rect[1] + v * (rect[3] - rect[1])
    ])
}

#[test]
fn a_brush_mask_is_painted_by_a_drag_and_rubbed_out_by_the_same_drag_with_alt_held() {
    let library = TempLibrary::new();
    let live = Live::start_with(Some(fixture(&library, "lake.png")));
    let mut client = live.client();
    assert!(
        seen(&mut client)["develop"].is_null(),
        "the Library has no canvas to describe"
    );

    client.ok("key", json!({ "name": "d" }));
    client.ok("wait_idle", json!({}));
    let develop = seen(&mut client);
    assert_eq!(develop["module"], "develop");
    assert_eq!(develop["develop"]["zoom"], "fit");
    assert!(
        develop["develop"]["tool"].is_null(),
        "{}",
        develop["develop"]
    );
    let rect = edges(&develop["develop"]["photo_rect"]);
    assert!(rect[2] > rect[0] && rect[3] > rect[1], "{rect:?}");
    // A point of the photo, as fractions of its width and height.
    let at = |u: f64, v: f64| within(rect, u, v);
    let before = luma(&mut client);

    // The brush, wide and hard-edged: a stroke across the middle, and two
    // stops more light under it.
    client.ok("key", json!({ "name": "k" }));
    let sized = client.ok("set_slider", json!({ "name": "Brush Size", "value": 15 }));
    assert_eq!(sized["exact"], true, "{sized}");
    client.ok("set_slider", json!({ "name": "Brush Feather", "value": 0 }));
    let painted = client.ok("drag", json!({ "from": at(0.2, 0.5), "to": at(0.8, 0.5) }));
    assert_eq!(painted["mods"], json!([]));
    let masking = seen(&mut client);
    assert_eq!(masking["develop"]["tool"], "mask");
    assert_eq!(masking["develop"]["selected_mask"], 0);
    assert_eq!(
        masking["develop"]["mask_overlay"], true,
        "a new mask shows its overlay"
    );
    client.ok("set_slider", json!({ "name": "Mask Exposure", "value": 2 }));
    let lit = luma(&mut client);
    assert!(lit > before + 8.0, "{before} became {lit}");

    // The same stroke with Alt held takes the mask away again.
    let rubbed = client.ok(
        "drag",
        json!({ "from": at(0.2, 0.5), "to": at(0.8, 0.5), "mods": ["alt"] }),
    );
    assert_eq!(rubbed["mods"], json!(["alt"]));
    let after = luma(&mut client);
    assert!(
        (after - before).abs() < (lit - before) * 0.1,
        "{before}, painted {lit}, rubbed out {after}"
    );

    // Escape puts the tool away and records nothing.
    client.ok("key", json!({ "name": "escape" }));
    let done = seen(&mut client);
    assert!(done["develop"]["tool"].is_null(), "{}", done["develop"]);
    assert!(done["undo"]["undo"].is_null(), "{}", done["undo"]);
    assert_eq!(luma(&mut client), before);
}

#[test]
fn the_crop_tool_says_where_its_rectangle_is_and_a_drag_of_a_corner_crops() {
    let library = TempLibrary::new();
    let live = Live::start_with(Some(fixture(&library, "dawn.jpg")));
    let mut client = live.client();
    client.ok("key", json!({ "name": "d" }));
    client.ok("wait_idle", json!({}));
    let whole = client.ok("dump_render", json!({}));
    assert_eq!(whole["render"]["width"], 96);
    assert_eq!(whole["render"]["height"], 64);

    client.ok("key", json!({ "name": "r" }));
    client.ok("wait_idle", json!({}));
    let cropping = seen(&mut client);
    assert_eq!(cropping["develop"]["tool"], "crop");
    let rect = edges(&cropping["develop"]["crop_rect"]);
    let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
    assert!(
        (width / height - 1.5).abs() < 0.02,
        "the whole photo: {rect:?}"
    );

    // The bottom right corner, pulled in by a quarter each way.
    client.ok(
        "drag",
        json!({
            "from": [rect[2], rect[3]],
            "to": [rect[2] - width / 4.0, rect[3] - height / 4.0],
        }),
    );
    client.ok("key", json!({ "name": "enter" }));
    client.ok("wait_idle", json!({}));
    let done = seen(&mut client);
    assert!(done["develop"]["tool"].is_null());
    assert!(done["develop"]["crop_rect"].is_null());
    assert_eq!(done["undo"]["undo"], "Crop Rectangle");
    let cropped = client.ok("dump_render", json!({}));
    let (w, h) = (
        cropped["render"]["width"].as_u64().unwrap(),
        cropped["render"]["height"].as_u64().unwrap(),
    );
    assert!(
        (70..=74).contains(&w) && (46..=50).contains(&h),
        "{w} x {h}"
    );
}

#[test]
fn a_click_on_the_photo_places_a_healing_spot_and_delete_takes_it_away() {
    let library = TempLibrary::new();
    let live = Live::start_with(Some(fixture(&library, "dawn.jpg")));
    let mut client = live.client();
    client.ok("key", json!({ "name": "d" }));
    client.ok("wait_idle", json!({}));
    client.ok("key", json!({ "name": "q" }));
    let healing = seen(&mut client);
    assert_eq!(healing["develop"]["tool"], "heal");
    assert!(healing["develop"]["selected_spot"].is_null());

    // A drag that goes nowhere is a click.
    let at = within(edges(&healing["develop"]["photo_rect"]), 0.4, 0.6);
    client.ok("drag", json!({ "from": at, "to": at }));
    let placed = client.ok("observe", json!({}));
    assert_eq!(
        placed["develop"]["selected_spot"], 0,
        "{}",
        placed["develop"]
    );
    let size = placed["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|control| control["name"] == "Spot Size" && control["role"] == "Slider")
        .expect("the spot's size is in the panel");
    assert_eq!(size["enabled"], true);

    client.ok("key", json!({ "name": "delete" }));
    assert!(seen(&mut client)["develop"]["selected_spot"].is_null());
    client.ok("key", json!({ "name": "escape" }));
    let done = seen(&mut client);
    assert!(done["develop"]["tool"].is_null(), "{}", done["develop"]);
    assert!(done["undo"]["undo"].is_null(), "{}", done["undo"]);
}
