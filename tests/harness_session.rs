//! The agent harness's protocol, met the way an agent meets it: a session
//! started on port 0 in a temporary sandbox and driven over a real TCP
//! connection.

mod support;

use photo::harness::StopReason;
use serde_json::{Value, json};
use support::{Client, Live};

#[test]
fn a_wrong_token_is_refused_and_the_right_one_accepted() {
    let live = Live::start();
    let mut stranger = Client::raw(live.session.port);
    stranger.line("not the token");
    let refusal = stranger.read().expect("the refusal is answered");
    assert_eq!(refusal["ok"], false);
    assert_eq!(refusal["error"]["code"], "bad_token");
    assert!(
        stranger.read().is_none(),
        "the connection is closed after a bad token"
    );

    let mut client = live.client();
    let ping = client.ok("ping", json!({}));
    assert_eq!(ping["protocol_version"], 1);
    assert_eq!(ping["app_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(ping["run_id"], live.session.run_id);
    assert_eq!(ping["sandboxed"], true);
    assert!(ping["gpu"]["available"].is_boolean());
}

#[test]
fn one_client_at_a_time_and_a_client_may_come_back() {
    let live = Live::start();
    let mut first = live.client();
    first.ok("ping", json!({}));
    let mut second = live.client();
    let refusal = second.read().expect("the second client is answered");
    assert_eq!(refusal["error"]["code"], "unavailable");
    drop(first);
    // A client that has closed its connection makes way for the next at
    // once; should the close not have arrived yet, the next is refused and
    // tries again.
    for _ in 0..100 {
        let mut again = live.client();
        again.line(r#"{"id": 1, "cmd": "ping"}"#);
        let answer = again.read().expect("an answer");
        if answer["ok"] == true {
            assert_eq!(answer["id"], 1);
            return;
        }
        assert_eq!(answer["error"]["code"], "unavailable", "{answer}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("a client could not reconnect");
}

#[test]
fn the_statistics_of_a_known_image_are_exact() {
    let live = Live::start();
    let mut client = live.client();
    let mut image = image::RgbaImage::new(2, 2);
    for (x, y, colour) in [
        (0, 0, [0, 0, 0]),
        (1, 0, [255, 255, 255]),
        (0, 1, [255, 0, 0]),
        (1, 1, [0, 0, 255]),
    ] {
        image.put_pixel(x, y, image::Rgba([colour[0], colour[1], colour[2], 255]));
    }
    let known = live.run_dir.join("known.png");
    image.save(&known).unwrap();
    let stats = client.ok("image_stats", json!({ "path": "known.png" }));
    let red = &stats["channels"]["r"];
    assert_eq!(red["clip_high_fraction"], 0.5, "white and red");
    assert_eq!(red["clip_low_fraction"], 0.5, "black and blue");
    assert_eq!(red["mean"], 127.5);
    assert_eq!(stats["channels"]["luma"]["clip_low_fraction"], 0.25);
    assert_eq!(stats["channels"]["luma"]["clip_high_fraction"], 0.25);
    assert_eq!(stats["mean_saturation"], 0.5);

    let mut changed = image.clone();
    changed.put_pixel(0, 0, image::Rgba([0, 9, 0, 255]));
    changed.save(live.run_dir.join("changed.png")).unwrap();
    let diff = client.ok(
        "compare_images",
        json!({ "path_a": known, "path_b": "changed.png" }),
    );
    assert_eq!(diff["identical"], false);
    assert_eq!(diff["differing_fraction"], 0.25);
    assert_eq!(diff["channels"]["g"]["max_abs_diff"], 9);
    let wide = image::RgbaImage::new(3, 1);
    wide.save(live.run_dir.join("wide.png")).unwrap();
    assert_eq!(
        client.fails(
            "compare_images",
            json!({ "path_a": "wide.png", "path_b": "known.png" })
        ),
        "invalid_params",
        "images of different sizes are not compared"
    );
}

#[test]
fn unknown_commands_and_bad_parameters_get_their_codes() {
    let live = Live::start();
    let mut client = live.client();
    for (cmd, params, code) in [
        ("frobnicate", json!({}), "unsupported"),
        ("click", json!({}), "invalid_params"),
        ("click", json!({ "name": 3 }), "invalid_params"),
        ("click", json!({ "name": "No Such Control" }), "not_found"),
        ("observe", json!({ "bogus": true }), "invalid_params"),
        (
            "wait_idle",
            json!({ "timeout_ms": "soon" }),
            "invalid_params",
        ),
        ("key", json!({ "name": "hyper" }), "invalid_params"),
        (
            "key",
            json!({ "name": "a", "mods": ["hyper"] }),
            "invalid_params",
        ),
        (
            "drag",
            json!({ "from": [1, 1], "to": [99_999, 1] }),
            "invalid_params",
        ),
        ("image_stats", json!({ "path": "missing.png" }), "not_found"),
        (
            "import_folder",
            json!({ "path": "no/such/folder" }),
            "not_found",
        ),
        ("open", json!({ "path": "no/such/photo.jpg" }), "not_found"),
        (
            "dump_render",
            json!({ "source": "sideways" }),
            "invalid_params",
        ),
        ("dump_render", json!({}), "unavailable"),
        (
            "set_slider",
            json!({ "name": "Thumbnail size" }),
            "invalid_params",
        ),
    ] {
        assert_eq!(client.fails(cmd, params.clone()), code, "{cmd} {params}");
    }

    client.line("this is not json");
    let answer = client.read().unwrap();
    assert_eq!(
        (answer["ok"].clone(), answer["error"]["code"].clone()),
        (json!(false), json!("bad_request"))
    );
    client.line(r#"{"id": 77, "params": {}}"#);
    let answer = client.read().unwrap();
    assert_eq!(answer["id"], 77, "a bad request keeps its id");
    assert_eq!(answer["error"]["code"], "bad_request");
    client.ok("ping", json!({}));
}

#[test]
fn quit_finalises_the_manifest_and_ends_the_session() {
    let mut live = Live::start();
    let mut client = live.client();
    client.ok("ping", json!({}));
    let started = live.json_file("manifest.json");
    assert_eq!(started["stop_reason"], Value::Null, "running");
    let quit = client.ok("quit", json!({}));
    assert_eq!(quit["stopping"], true);
    let manifest = live.json_file("manifest.json");
    assert_eq!(manifest["stop_reason"], "quit", "{manifest}");
    assert!(manifest["ended_at"].is_string());
    assert_eq!(manifest["requests"], 2);
    assert_eq!(manifest["run_id"], live.session.run_id);
    assert!(client.read().is_none(), "the connection closes");
    assert_eq!(live.join(), Ok(StopReason::Quit));

    let log = std::fs::read_to_string(live.run_dir.join("session.jsonl")).unwrap();
    let records: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let commands: Vec<&str> = records
        .iter()
        .filter(|record| record["kind"] == "request")
        .map(|record| record["cmd"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["ping", "quit"]);
    assert_eq!(
        records
            .iter()
            .filter(|record| record["kind"] == "response")
            .count(),
        2
    );
    assert!(
        !log.contains(&live.session.token),
        "the token is not logged"
    );
    let session = live.json_file("session.json");
    assert_eq!(session["port"], live.session.port);
    assert_eq!(session["token"], live.session.token.as_str());
    assert_eq!(session["pid"], std::process::id());
    assert_eq!(session["runs_location"], "system_temp");
    assert!(
        session["note"].is_string(),
        "outside a checkout, it says so"
    );
}
