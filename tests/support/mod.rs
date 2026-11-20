//! A harness session started in this process, on port 0, in a temporary
//! sandbox, and a client that talks to it over a real TCP connection.

#![allow(dead_code)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

use photo::harness::run::STORAGE_DIR;
use photo::harness::{HarnessConfig, Look, RunDir, Server, SessionInfo, StopReason};
use photo::repo::{HarnessRuns, Storage};
use serde_json::{Value, json};
use tempfile::TempDir;

/// How long a test waits for one answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);

/// A running session and the temporary folder that holds its runs.
pub struct Live {
    /// The folder of run directories; nothing else is in it.
    pub base: TempDir,
    /// The session's run directory.
    pub run_dir: PathBuf,
    /// How to reach it.
    pub session: SessionInfo,
    thread: Option<JoinHandle<Result<StopReason, String>>>,
}

impl Live {
    /// A sandboxed session with nothing open.
    pub fn start() -> Self {
        Self::start_with(None)
    }

    /// A sandboxed session, opening `open` at startup.
    pub fn start_with(open: Option<PathBuf>) -> Self {
        let base = tempfile::tempdir().expect("a temporary folder");
        let runs = HarnessRuns {
            dir: base.path().join("runs"),
            in_checkout: false,
        };
        let run = RunDir::create(&runs).expect("a run directory");
        let run_dir = run.path().to_path_buf();
        let storage = Storage::under(&run_dir.join(STORAGE_DIR));
        storage.create_dirs().expect("the storage folders");
        let config = HarnessConfig {
            port: 0,
            run,
            storage,
            sandboxed: true,
            open,
            look: Look::default(),
            paths_base: base.path().to_path_buf(),
        };
        let server = Server::bind(config).expect("the server binds");
        let session = server.session().clone();
        let thread = std::thread::spawn(move || server.run());
        Self {
            base,
            run_dir,
            session,
            thread: Some(thread),
        }
    }

    /// A client that has sent the token.
    pub fn client(&self) -> Client {
        let mut client = Client::raw(self.session.port);
        client.line(&self.session.token);
        client
    }

    /// Wait for the session to end (after a client sent `quit`).
    pub fn join(&mut self) -> Result<StopReason, String> {
        self.thread
            .take()
            .expect("the session is running")
            .join()
            .expect("the session thread does not panic")
    }

    /// A file of the run directory, read as JSON.
    pub fn json_file(&self, name: &str) -> Value {
        let text = std::fs::read_to_string(self.run_dir.join(name)).expect("the file is there");
        serde_json::from_str(&text).expect("the file is JSON")
    }
}

impl Drop for Live {
    /// A test that failed half way still stops its session: `quit` is sent
    /// once the session takes a new client.
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        for _ in 0..100 {
            if let Some(mut client) = Client::try_raw(self.session.port) {
                client.line(&self.session.token);
                client.line(r#"{"id": 0, "cmd": "quit"}"#);
                if client.read().is_some_and(|answer| answer["ok"] == true) {
                    let _ = thread.join();
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// One TCP connection to a session.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next: u64,
}

impl Client {
    /// A connection that has sent nothing yet.
    pub fn raw(port: u16) -> Self {
        Self::try_raw(port).expect("the session takes connections")
    }

    fn try_raw(port: u16) -> Option<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.set_read_timeout(Some(ANSWER_TIMEOUT)).ok()?;
        let reader = BufReader::new(stream.try_clone().ok()?);
        Some(Self {
            reader,
            writer: stream,
            next: 0,
        })
    }

    /// Send one line.
    pub fn line(&mut self, text: &str) {
        writeln!(self.writer, "{text}").expect("the line is sent");
    }

    /// Read one answer; `None` once the session has closed the connection.
    pub fn read(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&line).expect("an answer is JSON")),
        }
    }

    /// Send a request and read its answer, checking the id comes back.
    pub fn call(&mut self, cmd: &str, params: Value) -> Value {
        self.next += 1;
        let request = json!({ "id": self.next, "cmd": cmd, "params": params });
        self.line(&request.to_string());
        let answer = self.read().unwrap_or_else(|| panic!("no answer to {cmd}"));
        assert_eq!(answer["id"], json!(self.next), "{answer}");
        answer
    }

    /// The data of a request that must succeed.
    pub fn ok(&mut self, cmd: &str, params: Value) -> Value {
        let answer = self.call(cmd, params);
        assert_eq!(answer["ok"], true, "{cmd} failed: {answer}");
        answer["data"].clone()
    }

    /// The error code of a request that must fail.
    pub fn fails(&mut self, cmd: &str, params: Value) -> String {
        let answer = self.call(cmd, params);
        assert_eq!(answer["ok"], false, "{cmd} should have failed: {answer}");
        answer["error"]["code"]
            .as_str()
            .expect("an error has a code")
            .to_owned()
    }
}

/// Every file under `root` with its size and modification time, sorted:
/// what a folder looked like, to compare with later.
pub fn listing(root: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                folders.push(path.clone());
            }
            found.push((path, metadata.len(), metadata.modified().ok()));
        }
    }
    found.sort();
    found
}
