//! A harness run's directory, `tmp/harness-runs/<run-id>/`:
//!
//! * `session.json`: how to reach the session (port, token) and what it is
//!   (process id, run id, start time, storage root).
//! * `manifest.json`: the run's record, rewritten as artifacts appear and
//!   finalised on `quit` with the stop reason.
//! * `session.jsonl`: every request and response, appended as they happen.
//!   `harnessctl script` replays it: it runs the `request` lines.
//! * `artifacts/`: the screenshots and renders the session made.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::protocol::PROTOCOL_VERSION;
use crate::repo::HarnessRuns;

/// The version of the files' layout, written into each.
pub const SCHEMA_VERSION: u32 = 1;

/// The name of the connection file.
pub const SESSION_FILE: &str = "session.json";
/// The name of the run's record.
pub const MANIFEST_FILE: &str = "manifest.json";
/// The name of the request log.
pub const LOG_FILE: &str = "session.jsonl";
/// The name of the folder of screenshots and renders.
pub const ARTIFACTS_DIR: &str = "artifacts";
/// The name of the folder a sandboxed session keeps its stores in.
pub const STORAGE_DIR: &str = "storage";

/// How to reach a running session, as `session.json` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// The local TCP port.
    pub port: u16,
    /// The line each connection must start with.
    pub token: String,
    /// The harness's process.
    pub pid: u32,
    /// The run's id.
    pub run_id: String,
    /// The run directory.
    pub run_dir: PathBuf,
    /// When the session started, in UTC.
    pub started_at: String,
    /// Where the app keeps its catalog and exports.
    pub storage_root: PathBuf,
    /// False when the session uses the user's real stores.
    pub sandboxed: bool,
    /// False when the run directory is in the system temporary directory.
    pub in_checkout: bool,
}

impl SessionInfo {
    fn to_json(&self) -> Value {
        let mut value = json!({
            "schema_version": SCHEMA_VERSION,
            "protocol_version": PROTOCOL_VERSION,
            "app_version": env!("CARGO_PKG_VERSION"),
            "port": self.port,
            "token": self.token,
            "pid": self.pid,
            "run_id": self.run_id,
            "run_dir": self.run_dir,
            "started_at": self.started_at,
            "storage_root": self.storage_root,
            "sandboxed": self.sandboxed,
            "runs_location": runs_location(self.in_checkout),
        });
        if !self.in_checkout {
            value["note"] = json!(
                "Photo is not running from a checkout, so this run is kept in the system temporary directory."
            );
        }
        value
    }
}

fn runs_location(in_checkout: bool) -> &'static str {
    if in_checkout {
        "checkout"
    } else {
        "system_temp"
    }
}

/// One run's directory and the records kept in it.
pub struct RunDir {
    path: PathBuf,
    id: String,
    in_checkout: bool,
    started_at: SystemTime,
    started: Instant,
    log: Option<File>,
    artifacts: Vec<Value>,
    next_artifact: u32,
    requests: u64,
    storage: Option<(PathBuf, bool)>,
}

impl RunDir {
    /// Make a new run directory, with its `artifacts/` folder, under
    /// `runs`. The id is the start time in UTC (so ids sort by time) and the
    /// process id.
    pub fn create(runs: &HarnessRuns) -> io::Result<Self> {
        fs::create_dir_all(&runs.dir)?;
        let now = SystemTime::now();
        let stem = format!("{}-{}", compact_stamp(now), std::process::id());
        let (id, path) = (0..1000)
            .map(|n| match n {
                0 => stem.clone(),
                n => format!("{stem}-{n}"),
            })
            .map(|id| {
                let path = runs.dir.join(&id);
                (id, path)
            })
            .find(|(_, path)| !path.exists())
            .ok_or_else(|| io::Error::other("no free run directory name"))?;
        fs::create_dir(&path)?;
        fs::create_dir(path.join(ARTIFACTS_DIR))?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.join(LOG_FILE))?;
        Ok(Self {
            path,
            id,
            in_checkout: runs.in_checkout,
            started_at: now,
            started: Instant::now(),
            log: Some(log),
            artifacts: Vec::new(),
            next_artifact: 1,
            requests: 0,
            storage: None,
        })
    }

    /// The run directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The run's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether the run directory is in a checkout.
    pub fn in_checkout(&self) -> bool {
        self.in_checkout
    }

    /// When the run started, in UTC.
    pub fn started_at(&self) -> String {
        iso8601(self.started_at)
    }

    /// Milliseconds since the run started.
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// A path a request gave: relative ones are taken from the run
    /// directory.
    pub fn resolve(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.path.join(path)
        }
    }

    /// Write `session.json` and the first `manifest.json`.
    pub fn write_session(&mut self, info: &SessionInfo) -> io::Result<()> {
        self.storage = Some((info.storage_root.clone(), info.sandboxed));
        write_json(&self.path.join(SESSION_FILE), &info.to_json())?;
        self.write_manifest(None)
    }

    /// Append one record to `session.jsonl`, stamped with the time since the
    /// run started. A log that cannot be written is reported once and then
    /// left alone; the session goes on.
    pub fn log(&mut self, mut record: Value) {
        if record.get("kind").is_some_and(|kind| kind == "request") {
            self.requests += 1;
        }
        if let Value::Object(fields) = &mut record {
            fields.insert("at_ms".to_owned(), json!(self.elapsed_ms()));
        }
        let Some(file) = self.log.as_mut() else {
            return;
        };
        let line = format!("{record}\n");
        if let Err(error) = file.write_all(line.as_bytes()).and_then(|()| file.flush()) {
            log::error!(
                "cannot write {}: {error}",
                self.path.join(LOG_FILE).display()
            );
            self.log = None;
        }
    }

    /// Where the next artifact of `kind` goes, absolute and relative to the
    /// run directory: `artifacts/0001-screenshot.png` and so on, numbered in
    /// the order they are made.
    pub fn next_artifact(&mut self, kind: &str) -> (PathBuf, String) {
        let relative = format!("{ARTIFACTS_DIR}/{:04}-{kind}.png", self.next_artifact);
        self.next_artifact += 1;
        (self.path.join(&relative), relative)
    }

    /// Record an artifact that was written, and rewrite the manifest so it
    /// lists it even if the session never ends cleanly.
    pub fn add_artifact(&mut self, mut entry: Value) {
        if let Value::Object(fields) = &mut entry {
            fields.insert("created_at".to_owned(), json!(iso8601(SystemTime::now())));
        }
        self.artifacts.push(entry);
        if let Err(error) = self.write_manifest(None) {
            log::error!("cannot update the manifest: {error}");
        }
    }

    /// The artifacts written so far.
    pub fn artifacts(&self) -> &[Value] {
        &self.artifacts
    }

    /// Write `manifest.json`; with `stop`, as the run's final record.
    pub fn write_manifest(&self, stop: Option<&str>) -> io::Result<()> {
        let (root, sandboxed) = self
            .storage
            .clone()
            .map_or((Value::Null, Value::Null), |(root, sandboxed)| {
                (json!(root), json!(sandboxed))
            });
        let manifest = json!({
            "schema_version": SCHEMA_VERSION,
            "protocol_version": PROTOCOL_VERSION,
            "app_version": env!("CARGO_PKG_VERSION"),
            "run_id": self.id,
            "started_at": self.started_at(),
            "ended_at": stop.map(|_| iso8601(SystemTime::now())),
            "stop_reason": stop,
            "storage": { "root": root, "sandboxed": sandboxed },
            "runs_location": runs_location(self.in_checkout),
            "requests": self.requests,
            "artifacts": self.artifacts,
        });
        write_json(&self.path.join(MANIFEST_FILE), &manifest)
    }
}

/// Write `value` to `path` whole: to a temporary file beside it first, so a
/// reader never sees half a file.
fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let partial = path.with_extension("json.partial");
    fs::write(&partial, format!("{text}\n"))?;
    fs::rename(&partial, path)
}

/// A random hex string for the session token. `RandomState` is seeded from
/// the operating system's randomness, which is enough for a token that only
/// keeps other local programs from driving the app by accident.
pub fn new_token() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    (0..2_u64)
        .map(|round| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(round);
            hasher.write_u128(nanos);
            hasher.write_u32(std::process::id());
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// `time` in UTC as `2026-11-20T05:30:12.345Z`.
pub fn iso8601(time: SystemTime) -> String {
    let (date, (hour, minute, second), millis) = utc(time);
    format!(
        "{:04}-{:02}-{:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z",
        date.0, date.1, date.2
    )
}

/// `time` in UTC as `20261120T053012345Z`: sortable and safe in a file
/// name on every platform.
fn compact_stamp(time: SystemTime) -> String {
    let (date, (hour, minute, second), millis) = utc(time);
    format!(
        "{:04}{:02}{:02}T{hour:02}{minute:02}{second:02}{millis:03}Z",
        date.0, date.1, date.2
    )
}

/// The calendar date, the time of day and the milliseconds of `time`, in
/// UTC. Times before 1970 are taken as 1970.
fn utc(time: SystemTime) -> ((i64, u32, u32), (u64, u64, u64), u64) {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let days = (seconds / 86_400) as i64;
    let of_day = seconds % 86_400;
    (
        civil_from_days(days),
        (of_day / 3600, of_day / 60 % 60, of_day % 60),
        u64::from(since.subsec_millis()),
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn times_are_written_in_utc() {
        assert_eq!(iso8601(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let known = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(iso8601(known), "2023-11-14T22:13:20.123Z");
        assert_eq!(compact_stamp(known), "20231114T221320123Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(iso8601(leap), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn run_ids_sort_by_time_and_never_collide() {
        let base = tempfile::tempdir().unwrap();
        let runs = HarnessRuns {
            dir: base.path().join("runs"),
            in_checkout: true,
        };
        let first = RunDir::create(&runs).unwrap();
        let second = RunDir::create(&runs).unwrap();
        assert_ne!(first.id(), second.id());
        assert!(
            first.id() < second.id(),
            "{} then {}",
            first.id(),
            second.id()
        );
        assert!(first.path().join(ARTIFACTS_DIR).is_dir());
        assert!(first.path().join(LOG_FILE).is_file());
    }

    #[test]
    fn the_manifest_lists_artifacts_and_says_why_the_run_stopped() {
        let base = tempfile::tempdir().unwrap();
        let runs = HarnessRuns {
            dir: base.path().to_path_buf(),
            in_checkout: false,
        };
        let mut run = RunDir::create(&runs).unwrap();
        let (absolute, relative) = run.next_artifact("screenshot");
        assert_eq!(relative, "artifacts/0001-screenshot.png");
        assert_eq!(
            absolute,
            run.path().join("artifacts").join("0001-screenshot.png")
        );
        run.add_artifact(json!({ "path": relative, "kind": "screenshot" }));
        run.log(json!({ "kind": "request", "cmd": "ping" }));
        run.write_manifest(Some("quit")).unwrap();
        let text = fs::read_to_string(run.path().join(MANIFEST_FILE)).unwrap();
        let manifest: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest["stop_reason"], "quit");
        assert!(manifest["ended_at"].is_string());
        assert_eq!(manifest["requests"], 1);
        assert_eq!(
            manifest["artifacts"][0]["path"],
            "artifacts/0001-screenshot.png"
        );
        assert_eq!(manifest["runs_location"], "system_temp");
        let log = fs::read_to_string(run.path().join(LOG_FILE)).unwrap();
        let record: Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
        assert_eq!(record["cmd"], "ping");
        assert!(record["at_ms"].is_u64());
    }

    #[test]
    fn tokens_are_long_hex_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn relative_paths_are_taken_from_the_run_directory() {
        let base = tempfile::tempdir().unwrap();
        let runs = HarnessRuns {
            dir: base.path().to_path_buf(),
            in_checkout: true,
        };
        let run = RunDir::create(&runs).unwrap();
        assert_eq!(
            run.resolve("artifacts/a.png"),
            run.path().join("artifacts/a.png")
        );
        let absolute = base.path().join("elsewhere.png");
        assert_eq!(run.resolve(absolute.to_str().unwrap()), absolute);
    }
}
