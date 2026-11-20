//! `harnessctl`: send commands to a running `photo --harness` session.
//!
//! ```text
//! harnessctl [--run-dir DIR] <command> [params-json]
//! harnessctl [--run-dir DIR] script <file.jsonl> [--keep-going]
//! ```
//!
//! The session is the newest `session.json` under the checkout's
//! `tmp/harness-runs/` (or the system temporary directory's runs folder
//! outside a checkout); `--run-dir` names a run directory, or a folder of
//! them, instead. Each response is printed as one JSON line. A script holds
//! one command object per line (`{"cmd": "key", "params": {"name": "3"}}`);
//! lines starting with `#` are comments, ids are assigned, and a session's
//! own `session.jsonl` replays as a script (only its requests are sent).
//!
//! Exit codes: 0 when every response was ok, 1 when one was not, 2 for a
//! usage or connection error.

use std::fs;
use std::io::{BufRead as _, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::harness::run::SESSION_FILE;
use crate::repo::{self, HarnessRuns};

/// Every response was ok.
pub const EXIT_OK: u8 = 0;
/// A response was not ok.
pub const EXIT_FAILED: u8 = 1;
/// The command line was wrong, or the session could not be reached.
pub const EXIT_USAGE: u8 = 2;

/// How the command line is used.
pub const USAGE: &str = "usage: harnessctl [--run-dir DIR] <command> [params-json]\n       harnessctl [--run-dir DIR] script <file.jsonl> [--keep-going]";

/// How long to wait for the session to accept a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for one response: `wait_idle` and imports can be long.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// A session to talk to, as its `session.json` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFile {
    /// Its local port.
    pub port: u16,
    /// The token each connection starts with.
    pub token: String,
    /// The run directory it belongs to.
    pub run_dir: PathBuf,
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq)]
enum Invocation {
    One { cmd: String, params: Value },
    Script { file: PathBuf, keep_going: bool },
    Help,
}

/// Run `harnessctl` with the arguments after the program name, printing
/// responses to `out` and complaints to `err`; returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let (run_dir, invocation) = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            let _ = writeln!(err, "{message}\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let keep_going = matches!(
        invocation,
        Invocation::Script {
            keep_going: true,
            ..
        }
    );
    let commands = match invocation {
        Invocation::Help => {
            let _ = writeln!(out, "{USAGE}");
            return EXIT_OK;
        }
        Invocation::One { cmd, params } => vec![(cmd, params)],
        Invocation::Script { ref file, .. } => match fs::read_to_string(file)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))
            .and_then(|text| parse_script(&text))
        {
            Ok(commands) => commands,
            Err(message) => {
                let _ = writeln!(err, "{message}");
                return EXIT_USAGE;
            }
        },
    };
    let session = match find_session(run_dir.as_deref(), &repo::harness_runs()) {
        Ok(session) => session,
        Err(message) => {
            let _ = writeln!(err, "{message}");
            return EXIT_USAGE;
        }
    };
    let mut connection = match Connection::open(&session) {
        Ok(connection) => connection,
        Err(message) => {
            let _ = writeln!(err, "{message}");
            return EXIT_USAGE;
        }
    };
    let mut code = EXIT_OK;
    for (index, (cmd, params)) in commands.into_iter().enumerate() {
        let response = match connection.request(index as u64 + 1, &cmd, params) {
            Ok(response) => response,
            Err(message) => {
                let _ = writeln!(err, "{message}");
                return EXIT_USAGE;
            }
        };
        let _ = writeln!(out, "{response}");
        if response.get("ok") != Some(&Value::Bool(true)) {
            code = EXIT_FAILED;
            if !keep_going {
                break;
            }
        }
    }
    code
}

fn parse_args(args: &[String]) -> Result<(Option<PathBuf>, Invocation), String> {
    let mut run_dir = None;
    let mut keep_going = false;
    let mut positional = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run-dir" => {
                let dir = args.next().ok_or("--run-dir needs a directory")?;
                run_dir = Some(PathBuf::from(dir));
            }
            "--keep-going" => keep_going = true,
            "--help" | "-h" => return Ok((run_dir, Invocation::Help)),
            other => match other.strip_prefix("--run-dir=") {
                Some(dir) => run_dir = Some(PathBuf::from(dir)),
                None if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
                None => positional.push(other.to_owned()),
            },
        }
    }
    let invocation = match positional.as_slice() {
        [] => return Err("no command was given".to_owned()),
        [script, file] if script == "script" => Invocation::Script {
            file: PathBuf::from(file),
            keep_going,
        },
        [script] if script == "script" => return Err("script needs a file".to_owned()),
        _ if keep_going => return Err("--keep-going only goes with script".to_owned()),
        [cmd] => Invocation::One {
            cmd: cmd.clone(),
            params: json!({}),
        },
        [cmd, params] => {
            let params: Value = serde_json::from_str(params)
                .map_err(|error| format!("the parameters are not JSON: {error}"))?;
            if !params.is_object() {
                return Err("the parameters must be a JSON object".to_owned());
            }
            Invocation::One {
                cmd: cmd.clone(),
                params,
            }
        }
        _ => return Err("too many arguments".to_owned()),
    };
    Ok((run_dir, invocation))
}

/// The commands of a script: one object per line with `cmd` and optional
/// `params`; blank lines and lines starting with `#` are skipped, and so are
/// a session log's records other than its requests.
pub fn parse_script(text: &str) -> Result<Vec<(String, Value)>, String> {
    let mut commands = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |message: String| format!("line {}: {message}", index + 1);
        let value: Value =
            serde_json::from_str(line).map_err(|error| at(format!("not JSON: {error}")))?;
        let Value::Object(mut object) = value else {
            return Err(at("a command is a JSON object".to_owned()));
        };
        match object.get("kind").and_then(Value::as_str) {
            None | Some("request") => {}
            Some(_) => continue,
        }
        let cmd = match object.remove("cmd") {
            Some(Value::String(cmd)) => cmd,
            _ => return Err(at("the command has no `cmd` string".to_owned())),
        };
        let params = match object.remove("params") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(params @ Value::Object(_)) => params,
            Some(_) => return Err(at("`params` must be an object".to_owned())),
        };
        commands.push((cmd, params));
    }
    Ok(commands)
}

/// The session to talk to: the one `run_dir` names (a run directory, or a
/// folder of them), else the newest in `runs`.
pub fn find_session(run_dir: Option<&Path>, runs: &HarnessRuns) -> Result<SessionFile, String> {
    let dir = match run_dir {
        Some(dir) if dir.join(SESSION_FILE).is_file() => dir.to_path_buf(),
        Some(dir) => newest_run(dir).ok_or_else(|| {
            format!(
                "there is no {SESSION_FILE} in {} or the run directories in it",
                dir.display()
            )
        })?,
        None => newest_run(&runs.dir).ok_or_else(|| {
            format!(
                "no harness session was found in {}; start one with `photo --harness`",
                runs.dir.display()
            )
        })?,
    };
    read_session(&dir)
}

/// The run directory in `base` with the greatest name (run ids sort by
/// time) that holds a session file.
pub fn newest_run(base: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(base).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join(SESSION_FILE).is_file())
        .max_by(|a, b| a.file_name().cmp(&b.file_name()))
}

/// Read a run directory's `session.json`.
pub fn read_session(run_dir: &Path) -> Result<SessionFile, String> {
    let path = run_dir.join(SESSION_FILE);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not JSON: {error}", path.display()))?;
    let port = value["port"]
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| format!("{} has no port", path.display()))?;
    let token = value["token"]
        .as_str()
        .ok_or_else(|| format!("{} has no token", path.display()))?
        .to_owned();
    Ok(SessionFile {
        port,
        token,
        run_dir: run_dir.to_path_buf(),
    })
}

/// A connection that has sent its token.
struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    run_dir: PathBuf,
}

impl Connection {
    fn open(session: &SessionFile) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, session.port));
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|error| {
            format!(
                "cannot reach the session of {} on port {}: {error}; is `photo --harness` still running?",
                session.run_dir.display(),
                session.port
            )
        })?;
        let setup = |stream: &TcpStream| {
            stream.set_read_timeout(Some(RESPONSE_TIMEOUT))?;
            stream.set_nodelay(true)
        };
        setup(&stream).map_err(|error| format!("cannot set up the connection: {error}"))?;
        let reader = stream
            .try_clone()
            .map(BufReader::new)
            .map_err(|error| format!("cannot set up the connection: {error}"))?;
        let mut connection = Self {
            reader,
            writer: stream,
            run_dir: session.run_dir.clone(),
        };
        connection.send_line(&session.token)?;
        Ok(connection)
    }

    fn send_line(&mut self, line: &str) -> Result<(), String> {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .and_then(|()| self.writer.flush())
            .map_err(|error| {
                format!(
                    "cannot send to the session of {}: {error}",
                    self.run_dir.display()
                )
            })
    }

    /// Send one request and read its response.
    fn request(&mut self, id: u64, cmd: &str, params: Value) -> Result<Value, String> {
        let request = json!({ "id": id, "cmd": cmd, "params": params });
        self.send_line(&request.to_string())?;
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err(format!(
                "the session of {} closed the connection before answering `{cmd}`",
                self.run_dir.display()
            )),
            Ok(_) => serde_json::from_str(line.trim_end())
                .map_err(|error| format!("the session's answer is not JSON: {error}")),
            Err(error) => Err(format!("no answer to `{cmd}`: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn a_command_takes_json_parameters_and_a_script_a_file() {
        let (dir, one) = parse_args(&args(&[
            "--run-dir",
            "/runs/x",
            "click",
            r#"{"name": "OK"}"#,
        ]))
        .unwrap();
        assert_eq!(dir, Some(PathBuf::from("/runs/x")));
        assert_eq!(
            one,
            Invocation::One {
                cmd: "click".to_owned(),
                params: json!({ "name": "OK" })
            }
        );
        let (_, script) = parse_args(&args(&["script", "s.jsonl", "--keep-going"])).unwrap();
        assert_eq!(
            script,
            Invocation::Script {
                file: PathBuf::from("s.jsonl"),
                keep_going: true
            }
        );
        for bad in [
            &[][..],
            &["ping", "[1]"],
            &["ping", "{"],
            &["ping", "--keep-going"],
            &["--fast", "ping"],
            &["script"],
            &["a", "b", "c"],
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_script_skips_comments_and_a_session_logs_other_records() {
        let text = "# warm up\n{\"cmd\": \"ping\"}\n\n{\"kind\": \"request\", \"id\": 4, \"cmd\": \"key\", \"params\": {\"name\": \"3\"}}\n{\"kind\": \"response\", \"id\": 4, \"ok\": true}\n{\"kind\": \"connect\"}\n";
        let commands = parse_script(text).unwrap();
        assert_eq!(
            commands,
            vec![
                ("ping".to_owned(), json!({})),
                ("key".to_owned(), json!({ "name": "3" }))
            ]
        );
        let error = parse_script("{\"cmd\": \"ping\"}\n{\"params\": {}}\n").unwrap_err();
        assert!(error.starts_with("line 2:"), "{error}");
        assert!(parse_script("not json").is_err());
    }
}
