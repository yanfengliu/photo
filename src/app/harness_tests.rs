//! Harness command-execution tests: each command must be observationally
//! equivalent to the real user interaction it mirrors.

use super::tests::{detail_app_with_image, library_app_with_entries, setup_dir};
use super::*;
use crate::harness::{HarnessCommand, HarnessEvent, HarnessRequest, HarnessResponse};
use crate::repo::with_test_photo_repo_root;

type ResponseReceiver = tokio::sync::mpsc::UnboundedReceiver<HarnessResponse>;

fn connect_harness(app: &mut App) -> ResponseReceiver {
    let (responder, responses) = tokio::sync::mpsc::unbounded_channel();
    let _ = app.update(Message::Harness(HarnessMsg::Event(
        HarnessEvent::Connected { responder },
    )));
    responses
}

fn send(app: &mut App, id: u64, command: HarnessCommand) -> Task<Message> {
    app.update(Message::Harness(HarnessMsg::Event(HarnessEvent::Request(
        HarnessRequest { id, command },
    ))))
}

fn expect_response(responses: &mut ResponseReceiver, id: u64) -> HarnessResponse {
    let response = responses
        .try_recv()
        .expect("expected a harness response to be queued");
    assert_eq!(response.id, id, "response correlates to its request");
    response
}

fn expect_no_response(responses: &mut ResponseReceiver) {
    assert!(
        responses.try_recv().is_err(),
        "expected no queued harness response"
    );
}

fn data(response: &HarnessResponse) -> &serde_json::Value {
    response.data.as_ref().expect("ok response carries data")
}

fn error_code(response: &HarnessResponse) -> &str {
    response
        .error
        .as_ref()
        .map(|error| error.code.as_str())
        .expect("failure response carries an error")
}

#[test]
fn ping_reports_protocol_and_app_version() {
    let (mut app, _) = App::new();
    let mut responses = connect_harness(&mut app);
    let _ = send(&mut app, 1, HarnessCommand::Ping {});
    let response = expect_response(&mut responses, 1);
    assert!(response.ok);
    assert_eq!(data(&response)["pong"], serde_json::json!(true));
    assert_eq!(
        data(&response)["protocol_version"],
        serde_json::json!(crate::harness::HARNESS_PROTOCOL_VERSION)
    );
    assert_eq!(
        data(&response)["app_version"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn observe_reports_state_controls_and_sliders() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);
    let _ = send(&mut app, 2, HarnessCommand::Observe {});
    let response = expect_response(&mut responses, 2);
    assert!(response.ok);
    let observation = data(&response);

    assert_eq!(observation["tab"], "detail");
    assert_eq!(observation["crop_mode"], serde_json::json!(false));
    assert_eq!(
        observation["current_image"]["load_stage"],
        serde_json::json!("idle")
    );
    assert_eq!(
        observation["current_image"]["buffer_width"],
        serde_json::json!(8)
    );
    assert_eq!(observation["pending"]["idle"], serde_json::json!(true));

    let sliders = observation["edit_state"]["sliders"].as_array().unwrap();
    assert_eq!(sliders.len(), 12);

    let controls = observation["controls"].as_array().unwrap();
    let exposure = controls
        .iter()
        .find(|control| control["id"] == "exposure")
        .expect("exposure control listed");
    assert_eq!(exposure["kind"], "slider");
    assert_eq!(exposure["min"], serde_json::json!(-5.0));
    assert_eq!(exposure["max"], serde_json::json!(5.0));
    assert_eq!(exposure["enabled"], serde_json::json!(true));

    let save = controls
        .iter()
        .find(|control| control["id"] == "save")
        .expect("save control listed");
    assert_eq!(save["enabled"], serde_json::json!(true));

    // Dialog-opening controls are deliberately absent from the vocabulary.
    assert!(!controls
        .iter()
        .any(|control| control["id"] == "add_folder" || control["id"] == "add_files"));
}

#[test]
fn set_slider_applies_and_commits_like_a_real_drag() {
    let repo_root = tempfile::tempdir().unwrap();
    with_test_photo_repo_root(repo_root.path(), || {
        let (dir, paths) = setup_dir(&["a.png"]);
        let _keep = &dir;
        let mut app = detail_app_with_image(&paths[0], 8, 6);
        let mut responses = connect_harness(&mut app);

        let _ = send(
            &mut app,
            3,
            HarnessCommand::SetSlider {
                kind: "exposure".to_string(),
                value: 1.5,
            },
        );
        let response = expect_response(&mut responses, 3);
        assert!(response.ok);
        assert_eq!(data(&response)["value"], serde_json::json!(1.5));

        let history = app.edit_histories.get(&paths[0]).expect("history created");
        assert_eq!(history.current.exposure, 1.5);
        assert!(history.can_undo(), "drag release commits the edit");
    });
}

#[test]
fn consecutive_set_sliders_do_not_trigger_double_click_reset() {
    // A real user double-clicking a slider resets it to zero; two deliberate
    // harness drags in quick succession must not.
    let repo_root = tempfile::tempdir().unwrap();
    with_test_photo_repo_root(repo_root.path(), || {
        let (dir, paths) = setup_dir(&["a.png"]);
        let _keep = &dir;
        let mut app = detail_app_with_image(&paths[0], 8, 6);
        let mut responses = connect_harness(&mut app);

        let _ = send(
            &mut app,
            4,
            HarnessCommand::SetSlider {
                kind: "contrast".to_string(),
                value: 30.0,
            },
        );
        let _ = send(
            &mut app,
            5,
            HarnessCommand::SetSlider {
                kind: "contrast".to_string(),
                value: 60.0,
            },
        );
        assert!(expect_response(&mut responses, 4).ok);
        assert!(expect_response(&mut responses, 5).ok);

        let history = app.edit_histories.get(&paths[0]).expect("history created");
        assert_eq!(
            history.current.contrast, 60.0,
            "second drag lands its value instead of double-click-resetting to 0"
        );
    });
}

#[test]
fn set_slider_clamps_to_the_widget_range() {
    let repo_root = tempfile::tempdir().unwrap();
    with_test_photo_repo_root(repo_root.path(), || {
        let (dir, paths) = setup_dir(&["a.png"]);
        let _keep = &dir;
        let mut app = detail_app_with_image(&paths[0], 8, 6);
        let mut responses = connect_harness(&mut app);
        let _ = send(
            &mut app,
            6,
            HarnessCommand::SetSlider {
                kind: "exposure".to_string(),
                value: 99.0,
            },
        );
        let response = expect_response(&mut responses, 6);
        assert!(response.ok);
        assert_eq!(data(&response)["value"], serde_json::json!(5.0));
        assert_eq!(
            app.edit_histories.get(&paths[0]).unwrap().current.exposure,
            5.0
        );
    });
}

#[test]
fn set_slider_rejects_unknown_kind_and_missing_image() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);
    let _ = send(
        &mut app,
        7,
        HarnessCommand::SetSlider {
            kind: "sharpness".to_string(),
            value: 1.0,
        },
    );
    let response = expect_response(&mut responses, 7);
    assert!(!response.ok);
    assert_eq!(error_code(&response), "invalid_params");

    let (mut library_app, _) = App::new();
    let mut responses = connect_harness(&mut library_app);
    let _ = send(
        &mut library_app,
        8,
        HarnessCommand::SetSlider {
            kind: "exposure".to_string(),
            value: 1.0,
        },
    );
    let response = expect_response(&mut responses, 8);
    assert!(!response.ok);
    assert_eq!(error_code(&response), "unavailable");
}

#[test]
fn clicks_mirror_the_buttons_they_name() {
    let repo_root = tempfile::tempdir().unwrap();
    with_test_photo_repo_root(repo_root.path(), || {
        let (dir, paths) = setup_dir(&["a.png"]);
        let _keep = &dir;
        let mut app = detail_app_with_image(&paths[0], 8, 6);
        let mut responses = connect_harness(&mut app);

        let _ = send(
            &mut app,
            9,
            HarnessCommand::Click {
                control: "rotate_cw".to_string(),
                value: None,
            },
        );
        assert!(expect_response(&mut responses, 9).ok);
        assert_eq!(app.current_rotation().as_u8(), 1);

        let _ = send(
            &mut app,
            10,
            HarnessCommand::Click {
                control: "reset_all".to_string(),
                value: None,
            },
        );
        assert!(expect_response(&mut responses, 10).ok);
        assert_eq!(app.current_rotation().as_u8(), 0);
        assert!(app
            .edit_histories
            .get(&paths[0])
            .is_some_and(edit::UndoHistory::can_undo));

        let _ = send(
            &mut app,
            11,
            HarnessCommand::Click {
                control: "back".to_string(),
                value: None,
            },
        );
        assert!(expect_response(&mut responses, 11).ok);
        assert_eq!(app.tab, Tab::Library);
    });
}

#[test]
fn click_rejects_unknown_and_dialog_controls() {
    let (mut app, _) = App::new();
    let mut responses = connect_harness(&mut app);

    let _ = send(
        &mut app,
        12,
        HarnessCommand::Click {
            control: "warp_drive".to_string(),
            value: None,
        },
    );
    let response = expect_response(&mut responses, 12);
    assert_eq!(error_code(&response), "invalid_params");

    let _ = send(
        &mut app,
        13,
        HarnessCommand::Click {
            control: "add_folder".to_string(),
            value: None,
        },
    );
    let response = expect_response(&mut responses, 13);
    assert_eq!(error_code(&response), "unsupported");
}

#[test]
fn key_escape_backs_out_of_detail_and_ctrl_o_is_refused() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);

    let _ = send(
        &mut app,
        14,
        HarnessCommand::Key {
            name: "escape".to_string(),
            mods: vec![],
        },
    );
    assert!(expect_response(&mut responses, 14).ok);
    assert_eq!(app.tab, Tab::Library);

    let _ = send(
        &mut app,
        15,
        HarnessCommand::Key {
            name: "o".to_string(),
            mods: vec!["ctrl".to_string()],
        },
    );
    let response = expect_response(&mut responses, 15);
    assert_eq!(error_code(&response), "unsupported");

    let _ = send(
        &mut app,
        16,
        HarnessCommand::Key {
            name: "hyperspace".to_string(),
            mods: vec![],
        },
    );
    let response = expect_response(&mut responses, 16);
    assert_eq!(error_code(&response), "invalid_params");
}

#[test]
fn set_crop_requires_crop_mode_then_commits_like_a_drag() {
    let repo_root = tempfile::tempdir().unwrap();
    with_test_photo_repo_root(repo_root.path(), || {
        let (dir, paths) = setup_dir(&["a.png"]);
        let _keep = &dir;
        let mut app = detail_app_with_image(&paths[0], 8, 6);
        let mut responses = connect_harness(&mut app);

        let _ = send(
            &mut app,
            17,
            HarnessCommand::SetCrop {
                left: 0.1,
                top: 0.1,
                right: 0.9,
                bottom: 0.9,
            },
        );
        let response = expect_response(&mut responses, 17);
        assert_eq!(error_code(&response), "unavailable");

        let _ = send(
            &mut app,
            18,
            HarnessCommand::Click {
                control: "crop".to_string(),
                value: None,
            },
        );
        assert!(expect_response(&mut responses, 18).ok);
        assert!(app.crop_mode);

        let _ = send(
            &mut app,
            19,
            HarnessCommand::SetCrop {
                left: 0.1,
                top: 0.1,
                right: 0.9,
                bottom: 0.9,
            },
        );
        assert!(expect_response(&mut responses, 19).ok);
        assert!(!app.crop_mode, "committing a crop exits crop mode");
        let crop = app
            .edit_histories
            .get(&paths[0])
            .and_then(|history| history.current.crop)
            .expect("crop committed");
        assert!((crop.left - 0.1).abs() < 1e-6);
        assert!((crop.bottom - 0.9).abs() < 1e-6);
    });
}

#[test]
fn wait_idle_responds_immediately_when_idle() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);
    let _ = send(&mut app, 20, HarnessCommand::WaitIdle { timeout_ms: None });
    let response = expect_response(&mut responses, 20);
    assert!(response.ok);
    assert_eq!(data(&response)["idle"], serde_json::json!(true));
}

#[test]
fn wait_idle_defers_until_the_load_settles() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);

    app.detail_load.begin_request();
    let _ = send(
        &mut app,
        21,
        HarnessCommand::WaitIdle {
            timeout_ms: Some(60_000),
        },
    );
    expect_no_response(&mut responses);

    // Still loading at the first poll: waiter stays armed.
    let _ = app.update(Message::Harness(HarnessMsg::IdlePoll));
    expect_no_response(&mut responses);

    // The load settles; the next poll releases the waiter.
    let _ = app.detail_load.on_full_image_loaded();
    app.detail_load.finish_exif();
    let _ = app.update(Message::Harness(HarnessMsg::IdlePoll));
    let response = expect_response(&mut responses, 21);
    assert!(response.ok);
    assert_eq!(data(&response)["idle"], serde_json::json!(true));
}

#[test]
fn wait_idle_covers_the_async_save_task() {
    // Found by the first live harness run: an agent that saves and then
    // immediately reads the export raced the spawn_blocking save task.
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);

    let _ = app.update(Message::SaveEdited);
    let _ = send(&mut app, 40, HarnessCommand::WaitIdle { timeout_ms: None });
    expect_no_response(&mut responses);

    let _ = app.update(Message::SaveCompleted(Ok("saved".to_string())));
    let _ = app.update(Message::Harness(HarnessMsg::IdlePoll));
    let response = expect_response(&mut responses, 40);
    assert!(response.ok);
    assert_eq!(data(&response)["idle"], serde_json::json!(true));
    assert_eq!(
        data(&response)["pending"]["save_in_flight"],
        serde_json::json!(false)
    );
}

#[test]
fn wait_idle_times_out_with_the_pending_report() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);

    app.detail_load.begin_request();
    let _ = send(
        &mut app,
        22,
        HarnessCommand::WaitIdle {
            timeout_ms: Some(0),
        },
    );
    expect_no_response(&mut responses);
    let _ = app.update(Message::Harness(HarnessMsg::IdlePoll));
    let response = expect_response(&mut responses, 22);
    assert!(!response.ok);
    assert_eq!(error_code(&response), "timeout");
    assert!(response.error.unwrap().message.contains("detail_loading"));
}

#[test]
fn observe_library_pages_with_offset_and_limit() {
    let mut app = library_app_with_entries(5);
    let mut responses = connect_harness(&mut app);
    let _ = send(
        &mut app,
        23,
        HarnessCommand::ObserveLibrary {
            offset: 1,
            limit: Some(2),
        },
    );
    let response = expect_response(&mut responses, 23);
    assert!(response.ok);
    let page = data(&response);
    assert_eq!(page["total"], serde_json::json!(5));
    assert_eq!(page["offset"], serde_json::json!(1));
    let entries = page["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["filename"], "photo-1.png");
    assert_eq!(entries[1]["filename"], "photo-2.png");
}

#[test]
fn import_folder_adds_library_entries_like_the_dialog() {
    let (dir, _paths) = setup_dir(&["a.png", "b.jpg"]);
    let (mut app, _) = App::new();
    app.clear_library_entries();
    let mut responses = connect_harness(&mut app);
    let _ = send(
        &mut app,
        24,
        HarnessCommand::ImportFolder {
            path: dir.path().display().to_string(),
        },
    );
    assert!(expect_response(&mut responses, 24).ok);
    assert_eq!(app.library.len(), 2);

    let _ = send(
        &mut app,
        25,
        HarnessCommand::ImportFolder {
            path: dir.path().join("missing").display().to_string(),
        },
    );
    let response = expect_response(&mut responses, 25);
    assert_eq!(error_code(&response), "invalid_params");
}

#[test]
fn import_files_requires_paths_and_open_requires_existence() {
    let (mut app, _) = App::new();
    let mut responses = connect_harness(&mut app);

    let _ = send(&mut app, 26, HarnessCommand::ImportFiles { paths: vec![] });
    let response = expect_response(&mut responses, 26);
    assert_eq!(error_code(&response), "invalid_params");

    let _ = send(
        &mut app,
        27,
        HarnessCommand::Open {
            path: "definitely/not/a/real/file.png".to_string(),
        },
    );
    let response = expect_response(&mut responses, 27);
    assert_eq!(error_code(&response), "invalid_params");
}

#[test]
fn dump_render_without_an_image_is_unavailable() {
    let (mut app, _) = App::new();
    let mut responses = connect_harness(&mut app);
    let _ = send(
        &mut app,
        28,
        HarnessCommand::DumpRender {
            source: crate::harness::RenderSource::Current,
            max_dim: None,
        },
    );
    let response = expect_response(&mut responses, 28);
    assert_eq!(error_code(&response), "unavailable");
}

#[test]
fn quit_acknowledges_then_refuses_further_requests() {
    let (mut app, _) = App::new();
    let mut responses = connect_harness(&mut app);
    let _ = send(&mut app, 29, HarnessCommand::Quit {});
    let response = expect_response(&mut responses, 29);
    assert!(response.ok);
    assert_eq!(data(&response)["quitting"], serde_json::json!(true));

    let _ = send(&mut app, 30, HarnessCommand::Ping {});
    let response = expect_response(&mut responses, 30);
    assert_eq!(error_code(&response), "quitting");
}

#[test]
fn requests_without_a_client_do_not_panic() {
    let (mut app, _) = App::new();
    let _ = send(&mut app, 31, HarnessCommand::Ping {});
}

#[test]
fn click_refuses_controls_the_list_advertises_as_disabled() {
    // Library tab, no image: the controls list reports rotate_cw and back as
    // disabled, so clicking them must fail instead of silently dispatching
    // messages no real user could produce.
    let (mut app, _) = App::new();
    let mut responses = connect_harness(&mut app);
    let _ = send(
        &mut app,
        40,
        HarnessCommand::Click {
            control: "rotate_cw".to_string(),
            value: None,
        },
    );
    assert_eq!(
        error_code(&expect_response(&mut responses, 40)),
        "unavailable"
    );

    let _ = send(
        &mut app,
        41,
        HarnessCommand::Click {
            control: "back".to_string(),
            value: None,
        },
    );
    assert_eq!(
        error_code(&expect_response(&mut responses, 41)),
        "unavailable"
    );
}

#[test]
fn import_files_rejects_paths_no_dialog_could_produce() {
    let (dir, paths) = setup_dir(&["real.png"]);
    let (mut app, _) = App::new();
    app.clear_library_entries();
    let mut responses = connect_harness(&mut app);

    let phantom = dir.path().join("ghost.png");
    let _ = send(
        &mut app,
        42,
        HarnessCommand::ImportFiles {
            paths: vec![
                paths[0].display().to_string(),
                phantom.display().to_string(),
            ],
        },
    );
    let response = expect_response(&mut responses, 42);
    assert_eq!(error_code(&response), "invalid_params");
    assert!(response.error.unwrap().message.contains("ghost.png"));
    assert_eq!(
        app.library.len(),
        0,
        "nothing imports when any path is phantom"
    );

    let _ = send(
        &mut app,
        43,
        HarnessCommand::ImportFiles {
            paths: vec![paths[0].display().to_string()],
        },
    );
    assert!(expect_response(&mut responses, 43).ok);
    assert_eq!(app.library.len(), 1);
}

#[test]
fn stale_async_completions_are_dropped_not_misdelivered() {
    let (mut app, _) = App::new();
    let mut first_client = connect_harness(&mut app);
    assert_eq!(app.harness_connection_generation, 1);

    let report = crate::harness::RenderReport {
        path: "artifacts/0001-render-current.png".to_string(),
        width: 1,
        height: 1,
        source: "current".to_string(),
        load_stage: "idle".to_string(),
        stats: crate::harness::stats::image_stats(&[0, 0, 0, 255], 1, 1).unwrap(),
    };

    // A completion from a previous generation must not be answered — but its
    // artifact is real and stays recorded for the manifest.
    let _ = app.update(Message::Harness(HarnessMsg::RenderDumped {
        request_id: 1,
        generation: 0,
        result: Ok(report.clone()),
    }));
    expect_no_response(&mut first_client);
    assert_eq!(app.harness_artifacts.len(), 1);

    // The same completion under the live generation is delivered.
    let _ = app.update(Message::Harness(HarnessMsg::RenderDumped {
        request_id: 1,
        generation: 1,
        result: Ok(report.clone()),
    }));
    assert!(expect_response(&mut first_client, 1).ok);

    // Reconnect: a new client with reused ids must never see generation-1
    // leftovers.
    let _ = app.update(Message::Harness(HarnessMsg::Event(
        HarnessEvent::ClientDisconnected,
    )));
    let mut second_client = connect_harness(&mut app);
    assert_eq!(app.harness_connection_generation, 2);
    let _ = app.update(Message::Harness(HarnessMsg::RenderDumped {
        request_id: 1,
        generation: 1,
        result: Ok(report),
    }));
    expect_no_response(&mut second_client);

    let stats =
        serde_json::to_value(crate::harness::stats::image_stats(&[0, 0, 0, 255], 1, 1).unwrap())
            .unwrap();
    let _ = app.update(Message::Harness(HarnessMsg::StatsComputed {
        request_id: 2,
        generation: 1,
        result: Ok(stats),
    }));
    expect_no_response(&mut second_client);
}

#[test]
fn disconnect_clears_idle_waiters() {
    let (dir, paths) = setup_dir(&["a.png"]);
    let _keep = &dir;
    let mut app = detail_app_with_image(&paths[0], 8, 6);
    let mut responses = connect_harness(&mut app);

    app.detail_load.begin_request();
    let _ = send(
        &mut app,
        32,
        HarnessCommand::WaitIdle {
            timeout_ms: Some(60_000),
        },
    );
    expect_no_response(&mut responses);
    assert_eq!(app.harness_idle_waiters.len(), 1);

    let _ = app.update(Message::Harness(HarnessMsg::Event(
        HarnessEvent::ClientDisconnected,
    )));
    assert!(app.harness_idle_waiters.is_empty());
    assert!(app.harness_responder.is_none());
}
