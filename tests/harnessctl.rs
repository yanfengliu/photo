//! `harnessctl`: finding the session, and the exit codes that tell a script
//! whether every answer was ok.

mod support;

use std::path::Path;
use std::process::Command;

use photo::harness::StopReason;
use photo::harnessctl::{self, EXIT_FAILED, EXIT_OK, EXIT_USAGE, find_session, newest_run};
use photo::repo::HarnessRuns;
use serde_json::{Value, json};
use support::Live;

/// Run `harnessctl` in this process; its exit code and the lines it
/// printed.
fn harnessctl(args: &[&str]) -> (u8, Vec<Value>, String) {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = harnessctl::run(&args, &mut out, &mut err);
    let lines = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| json!(line)))
        .collect();
    (code, lines, String::from_utf8(err).unwrap())
}

fn write_session(dir: &Path, port: u16) {
    std::fs::create_dir_all(dir).unwrap();
    let session = json!({ "port": port, "token": "t", "run_id": "x" });
    std::fs::write(dir.join("session.json"), session.to_string()).unwrap();
}

#[test]
fn the_newest_session_is_found_unless_a_run_directory_is_named() {
    let base = tempfile::tempdir().unwrap();
    let older = base.path().join("20260101T000000000Z-11");
    let newer = base.path().join("20261231T235959999Z-12");
    write_session(&older, 1001);
    write_session(&newer, 1002);
    std::fs::create_dir(base.path().join("20991231T000000000Z-13")).unwrap();
    let runs = HarnessRuns {
        dir: base.path().to_path_buf(),
        in_checkout: true,
    };
    assert_eq!(
        newest_run(base.path()),
        Some(newer.clone()),
        "a run without a session is passed over"
    );
    assert_eq!(find_session(None, &runs).unwrap().port, 1002);
    assert_eq!(find_session(Some(&older), &runs).unwrap().port, 1001);
    assert_eq!(find_session(Some(base.path()), &runs).unwrap().port, 1002);
    let empty = tempfile::tempdir().unwrap();
    assert!(find_session(Some(empty.path()), &runs).is_err());
    let nowhere = HarnessRuns {
        dir: empty.path().join("missing"),
        in_checkout: false,
    };
    let error = find_session(None, &nowhere).unwrap_err();
    assert!(error.contains("photo --harness"), "{error}");
}

#[test]
fn exit_codes_say_whether_every_answer_was_ok() {
    let mut live = Live::start();
    let run_dir = live.run_dir.display().to_string();
    let dir = run_dir.as_str();

    let (code, lines, _) = harnessctl(&["--run-dir", dir, "ping"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(lines[0]["ok"], true);
    assert_eq!(lines[0]["id"], 1);
    let (code, lines, _) = harnessctl(&["--run-dir", dir, "observe", r#"{"controls": false}"#]);
    assert_eq!(
        (code, lines[0]["data"]["module"].clone()),
        (EXIT_OK, json!("library"))
    );
    let (code, lines, _) = harnessctl(&["--run-dir", dir, "frobnicate"]);
    assert_eq!(code, EXIT_FAILED);
    assert_eq!(lines[0]["error"]["code"], "unsupported");

    let (code, lines, err) = harnessctl(&["--run-dir", dir, "click", "{not json"]);
    assert_eq!((code, lines.len()), (EXIT_USAGE, 0), "{err}");
    assert_eq!(harnessctl(&[]).0, EXIT_USAGE);
    let nowhere = tempfile::tempdir().unwrap();
    let (code, _, err) = harnessctl(&["--run-dir", &nowhere.path().display().to_string(), "ping"]);
    assert_eq!(code, EXIT_USAGE, "{err}");

    let script = live.base.path().join("script.jsonl");
    std::fs::write(
        &script,
        "# a comment\n{\"cmd\": \"ping\"}\n{\"cmd\": \"frobnicate\"}\n{\"cmd\": \"observe\", \"params\": {\"controls\": false}}\n",
    )
    .unwrap();
    let script = script.display().to_string();
    let (code, lines, _) = harnessctl(&["--run-dir", dir, "script", &script]);
    assert_eq!(
        (code, lines.len()),
        (EXIT_FAILED, 2),
        "it stops at the first failure"
    );
    assert_eq!(
        lines
            .iter()
            .map(|line| line["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    let (code, lines, _) = harnessctl(&["--run-dir", dir, "script", &script, "--keep-going"]);
    assert_eq!((code, lines.len()), (EXIT_FAILED, 3));

    // The session's own log replays as a script: only its requests are sent.
    let replay = live.base.path().join("replay.jsonl");
    let log = std::fs::read_to_string(live.run_dir.join("session.jsonl")).unwrap();
    let requests: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("\"kind\":\"request\"") && line.contains("\"cmd\":\"ping\""))
        .collect();
    std::fs::write(&replay, format!("{}\n{log}", requests[0])).unwrap();
    let (code, lines, _) = harnessctl(&[
        "--run-dir",
        dir,
        "script",
        &replay.display().to_string(),
        "--keep-going",
    ]);
    assert_eq!(code, EXIT_FAILED, "the log holds a frobnicate");
    assert!(lines.len() > 4);

    let binary = Command::new(env!("CARGO_BIN_EXE_harnessctl"))
        .args(["--run-dir", dir, "ping"])
        .output()
        .unwrap();
    assert_eq!(binary.status.code(), Some(i32::from(EXIT_OK)));
    let answer: Value = serde_json::from_slice(&binary.stdout).unwrap();
    assert_eq!(answer["data"]["protocol_version"], 1);
    let usage = Command::new(env!("CARGO_BIN_EXE_harnessctl"))
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(i32::from(EXIT_USAGE)));

    let (code, _, _) = harnessctl(&["--run-dir", dir, "quit"]);
    assert_eq!(code, EXIT_OK);
    assert_eq!(live.join(), Ok(StopReason::Quit));
    let (code, _, err) = harnessctl(&["--run-dir", dir, "ping"]);
    assert_eq!(
        code, EXIT_USAGE,
        "a session that has ended cannot be reached: {err}"
    );
}
