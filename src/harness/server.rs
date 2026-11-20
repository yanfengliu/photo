//! The TCP side: one client at a time on 127.0.0.1, a token first, then
//! JSON lines. Between requests the app keeps running frames, so engine
//! work progresses and what the UI shows stays current.

use std::io::{self, BufRead as _, BufReader, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use photo_ui::AppOptions;
use serde_json::{Value, json};

use super::HarnessConfig;
use super::commands::{self, Outcome, Session};
use super::driver::{Driver, Look};
use super::protocol::{CmdError, Code, Params, error_response, ok_response, parse_request};
use super::run::{RunDir, SessionInfo, new_token};

/// How long one wait for a client's line lasts before the app gets a
/// frame: a frame's length at 60 frames a second.
const TICK: Duration = Duration::from_millis(16);
/// How long writing a response may block on a client that does not read.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest request line accepted.
const MAX_LINE: usize = 1 << 20;

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// A client sent `quit`.
    Quit,
}

/// A harness session bound to its port, ready to start the app and serve.
pub struct Server {
    listener: TcpListener,
    run: RunDir,
    info: SessionInfo,
    options: AppOptions,
    open: Option<PathBuf>,
    look: Look,
    paths_base: PathBuf,
}

impl Server {
    /// Listen on 127.0.0.1 at the configured port (0 picks a free one) and
    /// write `session.json`. The app starts in [`Server::run`], on the
    /// thread that serves.
    pub fn bind(config: HarnessConfig) -> Result<Self, String> {
        let HarnessConfig {
            port,
            mut run,
            storage,
            sandboxed,
            open,
            look,
            paths_base,
        } = config;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .map_err(|error| format!("cannot listen on 127.0.0.1:{port}: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("cannot listen without blocking: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("cannot read the port listened on: {error}"))?
            .port();
        let info = SessionInfo {
            port,
            token: new_token(),
            pid: std::process::id(),
            run_id: run.id().to_owned(),
            run_dir: run.path().to_path_buf(),
            started_at: run.started_at(),
            storage_root: storage.root.clone(),
            sandboxed,
            in_checkout: run.in_checkout(),
        };
        run.write_session(&info)
            .map_err(|error| format!("cannot write {}: {error}", run.path().display()))?;
        run.log(json!({
            "kind": "start",
            "port": port,
            "pid": info.pid,
            "storage_root": info.storage_root,
            "sandboxed": sandboxed,
            "open": open,
        }));
        let options = AppOptions {
            catalog_path: storage.catalog,
            previews_dir: storage.previews,
            export_dir: storage.exports,
        };
        Ok(Self {
            listener,
            run,
            info,
            options,
            open,
            look,
            paths_base,
        })
    }

    /// How to reach the session.
    pub fn session(&self) -> &SessionInfo {
        &self.info
    }

    /// Start the app and serve until a client sends `quit`.
    pub fn run(mut self) -> Result<StopReason, String> {
        let (mut driver, opened) =
            match Driver::start(self.options.clone(), self.look, self.open.as_deref()) {
                Ok(started) => started,
                Err(error) => {
                    self.run
                        .log(json!({ "kind": "start_failed", "error": error }));
                    let _ = self.run.write_manifest(Some("startup_failed"));
                    return Err(error);
                }
            };
        if let Some(opened) = opened {
            self.run.log(json!({
                "kind": "opened",
                "id": opened.id,
                "path": opened.path,
                "imported": opened.imported,
            }));
        }
        let mut client: Option<Client> = None;
        loop {
            self.accept(&mut client);
            match client.as_mut().map(Client::read) {
                None => std::thread::sleep(TICK),
                Some(Read::Waiting) => {}
                Some(Read::Closed(reason)) => {
                    self.run
                        .log(json!({ "kind": "disconnect", "reason": reason }));
                    client = None;
                }
                Some(Read::Line(line)) => {
                    let Some(open) = client.as_mut() else {
                        continue;
                    };
                    if self.serve_line(&mut driver, open, &line) {
                        self.refuse_the_rest(open);
                        return Ok(StopReason::Quit);
                    }
                    if open.dropped {
                        client = None;
                    }
                }
            }
            driver.tick();
        }
    }

    /// Take waiting connections: the first becomes the client, any other
    /// is told that one is connected and closed.
    fn accept(&mut self, client: &mut Option<Client>) {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    // A client that has just gone (as `harnessctl` does
                    // after each answer) makes way for the next at once.
                    if client.as_ref().is_some_and(Client::is_gone) {
                        self.run.log(json!({ "kind": "disconnect", "reason": "the client closed the connection" }));
                        *client = None;
                    }
                    if client.is_some() {
                        let error = CmdError::unavailable(
                            "another client is connected; the harness serves one at a time",
                        );
                        let _ = send_on(&stream, &error_response(&Value::Null, &error));
                        self.run
                            .log(json!({ "kind": "refused", "peer": peer.to_string() }));
                        continue;
                    }
                    match Client::new(stream, peer) {
                        Ok(new) => {
                            self.run
                                .log(json!({ "kind": "connect", "peer": peer.to_string() }));
                            *client = Some(new);
                        }
                        Err(error) => {
                            log::warn!("cannot set up the connection from {peer}: {error}")
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    if error.kind() != io::ErrorKind::WouldBlock {
                        log::warn!("cannot accept a connection: {error}");
                    }
                    return;
                }
            }
        }
    }

    /// Answer one line from the client. Returns true when it was `quit`.
    fn serve_line(&mut self, driver: &mut Driver, client: &mut Client, line: &str) -> bool {
        if !client.authenticated {
            if line.trim() == self.info.token {
                client.authenticated = true;
                self.run.log(json!({ "kind": "token", "accepted": true }));
            } else {
                let error = CmdError::new(
                    Code::BadToken,
                    "the first line of a connection must be the token in session.json",
                );
                let _ = client.send(&error_response(&Value::Null, &error));
                self.run.log(json!({ "kind": "token", "accepted": false }));
                client.dropped = true;
            }
            return false;
        }
        let (mut response, quit) = self.answer(driver, line);
        if quit {
            // Finalised before the answer goes, so a client that reads the
            // manifest once `quit` is answered finds it complete.
            self.run.log(json!({ "kind": "stop", "reason": "quit" }));
            if let Err(error) = self.run.write_manifest(Some("quit")) {
                let error = CmdError::io(format!("cannot write the manifest: {error}"));
                response = error_response(&response["id"], &error);
            }
        }
        if let Err(error) = client.send(&response) {
            self.run
                .log(json!({ "kind": "disconnect", "reason": format!("cannot answer: {error}") }));
            client.dropped = true;
        }
        quit
    }

    /// The response to one request line, and whether it ends the session.
    fn answer(&mut self, driver: &mut Driver, line: &str) -> (Value, bool) {
        let started = Instant::now();
        let request = match parse_request(line) {
            Ok(request) => request,
            Err((id, error)) => {
                self.run.log(json!({ "kind": "unparsed", "line": line }));
                let response = error_response(&id, &error);
                self.log_response(&response, started);
                return (response, false);
            }
        };
        self.run.log(json!({
            "kind": "request",
            "id": request.id,
            "cmd": request.cmd,
            "params": request.params,
        }));
        let params = Params::new(&request.params);
        let mut session = Session {
            driver,
            run: &mut self.run,
            info: &self.info,
            paths_base: &self.paths_base,
            request_id: &request.id,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            commands::run(&request.cmd, &params, &mut session)
        }));
        let (response, quit) = match outcome {
            Ok(Ok(Outcome::Answer(data))) => (ok_response(&request.id, data), false),
            Ok(Ok(Outcome::Quit(data))) => (ok_response(&request.id, data), true),
            Ok(Err(error)) => (error_response(&request.id, &error), false),
            Err(panic) => {
                let error =
                    CmdError::internal(format!("the command panicked: {}", panic_text(&*panic)));
                (error_response(&request.id, &error), false)
            }
        };
        self.log_response(&response, started);
        (response, quit)
    }

    fn log_response(&mut self, response: &Value, started: Instant) {
        let mut record = response.clone();
        record["kind"] = json!("response");
        record["duration_ms"] = json!(started.elapsed().as_millis() as u64);
        self.run.log(record);
    }

    /// After `quit`, answer the lines the client already sent with
    /// `quitting`.
    fn refuse_the_rest(&mut self, client: &mut Client) {
        let _ = client
            .stream
            .set_read_timeout(Some(Duration::from_millis(1)));
        while let Read::Line(line) = client.read() {
            let id = parse_request(&line).map_or_else(|(id, _)| id, |request| request.id);
            let error = CmdError::new(Code::Quitting, "the session is shutting down");
            if client.send(&error_response(&id, &error)).is_err() {
                return;
            }
        }
    }
}

/// What the client sent, as far as it has got.
enum Read {
    /// A complete line, without its line break.
    Line(String),
    /// Nothing complete yet.
    Waiting,
    /// The connection is over, for this reason.
    Closed(String),
}

/// The connected client.
struct Client {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    pending: Vec<u8>,
    authenticated: bool,
    /// The connection is to be closed after this line.
    dropped: bool,
}

impl Client {
    fn new(stream: TcpStream, _peer: SocketAddr) -> io::Result<Self> {
        // A socket accepted from a non-blocking listener can inherit that.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(TICK))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        stream.set_nodelay(true)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            pending: Vec::new(),
            authenticated: false,
            dropped: false,
        })
    }

    /// The next complete line, waiting at most one tick for it. A part of a
    /// line is kept for the next call.
    fn read(&mut self) -> Read {
        loop {
            match self.reader.read_until(b'\n', &mut self.pending) {
                Ok(0) if self.pending.is_empty() => {
                    return Read::Closed("the client closed the connection".to_owned());
                }
                // A line, or the last bytes before the client closed.
                Ok(_) => return Read::Line(self.take_line()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if self.pending.len() > MAX_LINE {
                        return Read::Closed(format!("a line was longer than {MAX_LINE} bytes"));
                    }
                    return Read::Waiting;
                }
                Err(error) => return Read::Closed(error.to_string()),
            }
        }
    }

    /// Whether the client has closed the connection with nothing left
    /// unread, found without waiting.
    fn is_gone(&self) -> bool {
        if !(self.pending.is_empty() && self.reader.buffer().is_empty()) {
            return false;
        }
        if self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let mut byte = [0_u8; 1];
        let gone = match self.stream.peek(&mut byte) {
            Ok(0) => true,
            Ok(_) => false,
            Err(error) => error.kind() != io::ErrorKind::WouldBlock,
        };
        let _ = self.stream.set_nonblocking(false);
        gone
    }

    fn take_line(&mut self) -> String {
        let bytes = std::mem::take(&mut self.pending);
        let text = String::from_utf8_lossy(&bytes);
        text.trim_end_matches(['\n', '\r']).to_owned()
    }

    fn send(&mut self, value: &Value) -> io::Result<()> {
        send_on(&self.stream, value)
    }
}

/// Write one response line.
fn send_on(mut stream: &TcpStream, value: &Value) -> io::Result<()> {
    let mut line = value.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

/// What a panic said.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned())
}
