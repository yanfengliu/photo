//! Command-line client for the photo agent harness.
//!
//! Speaks the JSON-lines protocol documented in `docs/guides/agent-harness.md`
//! against an app launched with `photo --harness`. Connection details come
//! from the newest `tmp/harness-runs/*/session.json` under the current
//! directory (run it from the repo root) or an explicit `--run-dir`.
//!
//! Usage:
//!   harnessctl [--run-dir DIR] [--timeout-ms N] <cmd> [params-json]
//!   harnessctl [--run-dir DIR] [--timeout-ms N] [--keep-going] script <file.jsonl>
//!
//! Examples:
//!   harnessctl observe
//!   harnessctl set_slider '{"kind":"exposure","value":1.5}'
//!   harnessctl dump_render '{"max_dim":1600}'
//!   harnessctl script scenarios/tune-shadows.jsonl
//!
//! Exit codes: 0 = all responses ok, 1 = a response was ok:false,
//! 2 = usage/transport error.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Clone, PartialEq)]
struct Invocation {
    run_dir: Option<PathBuf>,
    timeout_ms: u64,
    keep_going: bool,
    mode: Mode,
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Command { cmd: String, params: Option<String> },
    Script { path: PathBuf },
}

fn main() {
    std::process::exit(run(&std::env::args().skip(1).collect::<Vec<_>>()));
}

fn run(args: &[String]) -> i32 {
    let invocation = match parse_args(args) {
        Ok(invocation) => invocation,
        Err(message) => {
            eprintln!("harnessctl: {message}");
            eprintln!("usage: harnessctl [--run-dir DIR] [--timeout-ms N] <cmd> [params-json]");
            eprintln!("       harnessctl [--run-dir DIR] [--keep-going] script <file.jsonl>");
            return 2;
        }
    };

    let session_path = match resolve_session_path(invocation.run_dir.as_deref()) {
        Ok(path) => path,
        Err(message) => {
            eprintln!("harnessctl: {message}");
            return 2;
        }
    };
    let (port, token) = match read_session(&session_path) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("harnessctl: {message}");
            return 2;
        }
    };

    let stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!(
                "harnessctl: cannot connect to 127.0.0.1:{port} (is the app running with --harness?): {e}"
            );
            return 2;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(invocation.timeout_ms)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(e) => {
            eprintln!("harnessctl: cannot clone stream: {e}");
            return 2;
        }
    });
    let mut writer = stream;
    if let Err(e) = writeln!(writer, "{token}") {
        eprintln!("harnessctl: cannot send token: {e}");
        return 2;
    }

    match invocation.mode {
        Mode::Command { cmd, params } => {
            let request = match compose_request(1, &cmd, params.as_deref()) {
                Ok(request) => request,
                Err(message) => {
                    eprintln!("harnessctl: {message}");
                    return 2;
                }
            };
            match exchange(&mut writer, &mut reader, 1, &request) {
                Ok(ok) => {
                    if ok {
                        0
                    } else {
                        1
                    }
                }
                Err(message) => {
                    eprintln!("harnessctl: {message}");
                    2
                }
            }
        }
        Mode::Script { path } => run_script(&path, invocation.keep_going, &mut writer, &mut reader),
    }
}

fn run_script(
    path: &Path,
    keep_going: bool,
    writer: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
) -> i32 {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("harnessctl: cannot read {}: {e}", path.display());
            return 2;
        }
    };
    let mut next_id = 0u64;
    let mut any_failed = false;
    for (line_number, line) in body.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        next_id += 1;
        let request = match compose_script_request(next_id, trimmed) {
            Ok(request) => request,
            Err(message) => {
                eprintln!("harnessctl: line {}: {message}", line_number + 1);
                return 2;
            }
        };
        match exchange(writer, reader, next_id, &request) {
            Ok(true) => {}
            Ok(false) => {
                any_failed = true;
                if !keep_going {
                    return 1;
                }
            }
            Err(message) => {
                eprintln!("harnessctl: line {}: {message}", line_number + 1);
                return 2;
            }
        }
    }
    if any_failed {
        1
    } else {
        0
    }
}

/// Sends one request line and reads lines until the response with the same id
/// arrives. Prints that response to stdout; returns its `ok` flag.
fn exchange(
    writer: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    id: u64,
    request: &str,
) -> Result<bool, String> {
    writeln!(writer, "{request}").map_err(|e| format!("send failed: {e}"))?;
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| format!("read failed (timeout?): {e}"))?;
        if read == 0 {
            return Err("connection closed before the response arrived".to_string());
        }
        let value: serde_json::Value = match serde_json::from_str(line.trim()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
            println!("{}", line.trim());
            return Ok(value
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false));
        }
    }
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let mut run_dir = None;
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    let mut keep_going = false;
    let mut positional: Vec<String> = Vec::new();

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--run-dir" {
            index += 1;
            run_dir = Some(PathBuf::from(
                args.get(index).ok_or("--run-dir needs a value")?,
            ));
        } else if let Some(value) = arg.strip_prefix("--run-dir=") {
            run_dir = Some(PathBuf::from(value));
        } else if arg == "--timeout-ms" {
            index += 1;
            timeout_ms = args
                .get(index)
                .ok_or("--timeout-ms needs a value")?
                .parse()
                .map_err(|_| "--timeout-ms needs an integer".to_string())?;
        } else if let Some(value) = arg.strip_prefix("--timeout-ms=") {
            timeout_ms = value
                .parse()
                .map_err(|_| "--timeout-ms needs an integer".to_string())?;
        } else if arg == "--keep-going" {
            keep_going = true;
        } else if arg.starts_with("--") {
            return Err(format!("unknown flag {arg:?}"));
        } else {
            positional.push(arg.clone());
        }
        index += 1;
    }

    let mode = match positional.first().map(String::as_str) {
        None => return Err("missing command".to_string()),
        Some("script") => {
            let path = positional.get(1).ok_or("script needs a file path")?.clone();
            if positional.len() > 2 {
                return Err("script takes exactly one file".to_string());
            }
            Mode::Script {
                path: PathBuf::from(path),
            }
        }
        Some(cmd) => {
            if positional.len() > 2 {
                return Err("too many arguments; params must be one JSON string".to_string());
            }
            Mode::Command {
                cmd: cmd.to_string(),
                params: positional.get(1).cloned(),
            }
        }
    };

    Ok(Invocation {
        run_dir,
        timeout_ms,
        keep_going,
        mode,
    })
}

/// `{"id":N,"cmd":"...","params":{...}}` from a command name and optional
/// params JSON.
fn compose_request(id: u64, cmd: &str, params: Option<&str>) -> Result<String, String> {
    let mut request = serde_json::json!({ "id": id, "cmd": cmd });
    if let Some(params) = params {
        let parsed: serde_json::Value =
            serde_json::from_str(params).map_err(|e| format!("params is not valid JSON: {e}"))?;
        if !parsed.is_object() {
            return Err("params must be a JSON object".to_string());
        }
        request["params"] = parsed;
    }
    Ok(request.to_string())
}

/// A script line is a request object without an id (`{"cmd":..., "params":...}`);
/// ids are assigned sequentially so scripts replay deterministically.
fn compose_script_request(id: u64, line: &str) -> Result<String, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
    let object = value
        .as_object_mut()
        .ok_or("script line must be a JSON object")?;
    if !object.contains_key("cmd") {
        return Err("script line needs a \"cmd\" field".to_string());
    }
    object.insert("id".to_string(), serde_json::json!(id));
    Ok(value.to_string())
}

/// `--run-dir` may point at a run directory (containing `session.json`) or at
/// a `harness-runs` parent; without it, the newest run under
/// `tmp/harness-runs/` is used. Run ids embed a UTC timestamp, so the largest
/// directory name is the newest run — no mtime races.
fn resolve_session_path(run_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(dir) = run_dir {
        let direct = dir.join("session.json");
        if direct.is_file() {
            return Ok(direct);
        }
        return newest_session_under(dir)
            .ok_or_else(|| format!("no session.json under {}", dir.display()));
    }
    let default_root = Path::new("tmp").join("harness-runs");
    newest_session_under(&default_root).ok_or_else(|| {
        format!(
            "no harness runs under {} — launch the app with --harness first (or pass --run-dir)",
            default_root.display()
        )
    })
}

fn newest_session_under(root: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join("session.json").is_file())
        .collect();
    candidates.sort();
    candidates.pop().map(|dir| dir.join("session.json"))
}

fn read_session(path: &Path) -> Result<(u16, String), String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("invalid session file {}: {e}", path.display()))?;
    let port = value
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("session file {} has no port", path.display()))?;
    let token = value
        .get("token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("session file {} has no token", path.display()))?;
    Ok((port as u16, token.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_single_command_with_params() {
        let invocation =
            parse_args(&args(&["set_slider", r#"{"kind":"exposure","value":1.5}"#])).unwrap();
        assert_eq!(invocation.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(
            invocation.mode,
            Mode::Command {
                cmd: "set_slider".to_string(),
                params: Some(r#"{"kind":"exposure","value":1.5}"#.to_string()),
            }
        );
    }

    #[test]
    fn parses_flags_and_script_mode() {
        let invocation = parse_args(&args(&[
            "--run-dir",
            "tmp/harness-runs/x",
            "--timeout-ms=5000",
            "--keep-going",
            "script",
            "steps.jsonl",
        ]))
        .unwrap();
        assert_eq!(
            invocation.run_dir,
            Some(PathBuf::from("tmp/harness-runs/x"))
        );
        assert_eq!(invocation.timeout_ms, 5000);
        assert!(invocation.keep_going);
        assert_eq!(
            invocation.mode,
            Mode::Script {
                path: PathBuf::from("steps.jsonl")
            }
        );
    }

    #[test]
    fn rejects_bad_usage() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&args(&["--wat", "observe"])).is_err());
        assert!(parse_args(&args(&["observe", "{}", "extra"])).is_err());
        assert!(parse_args(&args(&["script"])).is_err());
    }

    #[test]
    fn composes_requests() {
        assert_eq!(
            compose_request(7, "ping", None).unwrap(),
            r#"{"cmd":"ping","id":7}"#
        );
        let with_params =
            compose_request(8, "set_slider", Some(r#"{"kind":"exposure","value":2}"#)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&with_params).unwrap();
        assert_eq!(value["id"], 8);
        assert_eq!(value["params"]["kind"], "exposure");
        assert!(compose_request(9, "x", Some("not json")).is_err());
        assert!(compose_request(10, "x", Some("[1,2]")).is_err());
    }

    #[test]
    fn script_lines_get_sequential_ids() {
        let request = compose_script_request(3, r#"{"cmd":"observe"}"#).unwrap();
        let value: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(value["id"], 3);
        assert_eq!(value["cmd"], "observe");
        assert!(compose_script_request(1, r#"{"params":{}}"#).is_err());
        assert!(compose_script_request(1, "[]").is_err());
    }

    #[test]
    fn newest_session_is_lexicographically_last_run_dir() {
        let root = tempfile::tempdir().unwrap();
        for run in ["20260101-000000Z-1", "20260102-000000Z-1", "not-a-run"] {
            let dir = root.path().join(run);
            std::fs::create_dir_all(&dir).unwrap();
            if run != "not-a-run" {
                std::fs::write(dir.join("session.json"), "{}").unwrap();
            }
        }
        let newest = newest_session_under(root.path()).unwrap();
        assert!(newest.ends_with(Path::new("20260102-000000Z-1").join("session.json")));
    }

    #[test]
    fn session_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, r#"{"port":7878,"token":"abc","pid":1}"#).unwrap();
        assert_eq!(read_session(&path).unwrap(), (7878, "abc".to_string()));
        std::fs::write(&path, r#"{"token":"abc"}"#).unwrap();
        assert!(read_session(&path).is_err());
    }
}
