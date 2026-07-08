//! Harness TCP control channel.
//!
//! JSON-lines over localhost with a per-run token gate. This module owns the
//! socket, `session.json`, and the append-only `session.jsonl` request/response
//! log; command semantics live in `app/harness_exec.rs`. The channel reaches
//! the app as an iced subscription stream of [`HarnessEvent`]s, and responses
//! travel back through the per-connection sender handed over in
//! [`HarnessEvent::Connected`].

use super::{HarnessConfig, HarnessEvent, HarnessRequest, HarnessResponse, SessionInfo};
use iced::futures::channel::mpsc as futures_mpsc;
use iced::futures::{SinkExt, Stream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The subscription the app runs while harness mode is active. Reads the
/// process-wide config; yields nothing when the harness is not configured.
pub(crate) fn subscription() -> iced::Subscription<HarnessEvent> {
    iced::Subscription::run(harness_event_stream)
}

fn harness_event_stream() -> impl Stream<Item = HarnessEvent> {
    iced::stream::channel(64, |mut events| async move {
        let Some(config) = super::config() else {
            return;
        };
        match TcpListener::bind(("127.0.0.1", config.port)).await {
            Ok(listener) => serve_on(listener, config, events).await,
            Err(e) => {
                let _ = events
                    .send(HarnessEvent::ListenFailed {
                        error: format!("cannot bind 127.0.0.1:{}: {e}", config.port),
                    })
                    .await;
            }
        }
    })
}

/// Accepts one client at a time, forever. Factored from the subscription
/// closure so tests can drive it against an ephemeral listener.
pub(crate) async fn serve_on(
    listener: TcpListener,
    config: &HarnessConfig,
    mut events: futures_mpsc::Sender<HarnessEvent>,
) {
    let port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(config.port);
    let session = SessionInfo {
        port,
        token: config.token.clone(),
        pid: std::process::id(),
        run_id: config.run_id.clone(),
        protocol_version: super::HARNESS_PROTOCOL_VERSION,
    };
    if let Err(e) = write_session_info(&config.run_dir, &session) {
        let _ = events.send(HarnessEvent::ListenFailed { error: e }).await;
        return;
    }

    let log = match open_session_log(&config.run_dir) {
        Ok(log) => log,
        Err(e) => {
            let _ = events.send(HarnessEvent::ListenFailed { error: e }).await;
            return;
        }
    };

    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            return;
        };
        handle_connection(stream, &mut events, &config.token, &log).await;
        if events.send(HarnessEvent::ClientDisconnected).await.is_err() {
            return;
        }
    }
}

type SharedLog = Arc<Mutex<std::fs::File>>;

fn open_session_log(run_dir: &Path) -> Result<SharedLog, String> {
    let path = run_dir.join("session.jsonl");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map(|file| Arc::new(Mutex::new(file)))
        .map_err(|e| format!("cannot open {}: {e}", path.display()))
}

fn write_session_info(run_dir: &Path, session: &SessionInfo) -> Result<(), String> {
    let path = run_dir.join("session.json");
    let body = serde_json::to_string_pretty(session)
        .map_err(|e| format!("session info serialization failed: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn append_session_log(log: &SharedLog, direction: &str, payload: serde_json::Value) {
    use std::io::Write;
    let entry = serde_json::json!({
        "t_ms": super::epoch_ms_now(),
        "dir": direction,
        "payload": payload,
    });
    if let Ok(mut file) = log.lock() {
        let _ = writeln!(file, "{entry}");
    }
}

async fn handle_connection(
    stream: TcpStream,
    events: &mut futures_mpsc::Sender<HarnessEvent>,
    token: &str,
    log: &SharedLog,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    match lines.next_line().await {
        Ok(Some(line)) if line.trim() == token => {}
        _ => {
            let rejection = HarnessResponse::failure(
                0,
                "bad_token",
                "first line must be the session token from session.json",
            );
            let _ = write_response_line(&mut write_half, &rejection).await;
            return;
        }
    }

    let (responder, mut responses) = tokio::sync::mpsc::unbounded_channel::<HarnessResponse>();
    if events
        .send(HarnessEvent::Connected {
            responder: responder.clone(),
        })
        .await
        .is_err()
    {
        return;
    }

    let writer_log = Arc::clone(log);
    let writer = tokio::spawn(async move {
        while let Some(response) = responses.recv().await {
            append_session_log(
                &writer_log,
                "resp",
                serde_json::to_value(&response).unwrap_or(serde_json::Value::Null),
            );
            if write_response_line(&mut write_half, &response)
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<HarnessRequest>(trimmed) {
            Ok(request) => {
                append_session_log(
                    log,
                    "req",
                    serde_json::to_value(&request).unwrap_or(serde_json::Value::Null),
                );
                if events.send(HarnessEvent::Request(request)).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                append_session_log(log, "req", serde_json::json!({ "malformed": trimmed }));
                let id = extract_request_id(trimmed);
                let response =
                    HarnessResponse::failure(id, "bad_request", &format!("invalid request: {e}"));
                if responder.send(response).is_err() {
                    break;
                }
            }
        }
    }

    // Reader is done (EOF or transport error). Dropping our responder clone
    // leaves the app's clone as the only sender; the detached writer task
    // drains until the app releases it on ClientDisconnected, then exits on
    // channel close.
    drop(responder);
    drop(writer);
}

async fn write_response_line(
    write_half: &mut tokio::net::tcp::OwnedWriteHalf,
    response: &HarnessResponse,
) -> std::io::Result<()> {
    let mut line = serde_json::to_string(response).unwrap_or_else(|_| {
        r#"{"id":0,"ok":false,"error":{"code":"internal","message":"response serialization failed"}}"#
            .to_string()
    });
    line.push('\n');
    write_half.write_all(line.as_bytes()).await
}

/// Best-effort id recovery from a malformed request line so the client can
/// still correlate the error response.
fn extract_request_id(line: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("id").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HarnessCommand;
    use iced::futures::StreamExt;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    struct TestServer {
        port: u16,
        run_dir: tempfile::TempDir,
        events: futures_mpsc::Receiver<HarnessEvent>,
    }

    async fn start_test_server(token: &str) -> TestServer {
        let run_dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = HarnessConfig {
            port,
            run_dir: run_dir.path().to_path_buf(),
            run_id: "test-run".to_string(),
            token: token.to_string(),
            sandboxed: true,
        };
        let (events_tx, events_rx) = futures_mpsc::channel(64);
        tokio::spawn(async move {
            // Config lives for the whole test process; leaking keeps the
            // borrow simple without a process-global.
            let config: &'static HarnessConfig = Box::leak(Box::new(config));
            serve_on(listener, config, events_tx).await;
        });
        TestServer {
            port,
            run_dir,
            events: events_rx,
        }
    }

    async fn next_event(events: &mut futures_mpsc::Receiver<HarnessEvent>) -> HarnessEvent {
        tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .expect("timed out waiting for harness event")
            .expect("event stream ended unexpectedly")
    }

    async fn read_line(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte))
                .await
                .expect("timed out reading line")
                .expect("socket read failed");
            if n == 0 || byte[0] == b'\n' {
                break;
            }
            buffer.push(byte[0]);
        }
        String::from_utf8(buffer).unwrap()
    }

    #[tokio::test]
    async fn wrong_token_is_rejected_without_events() {
        let mut server = start_test_server("secret").await;
        let mut client = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();
        client.write_all(b"wrong\n").await.unwrap();
        let line = read_line(&mut client).await;
        let response: HarnessResponse = serde_json::from_str(&line).unwrap();
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "bad_token");

        // The rejected connection still surfaces a disconnect marker, but no
        // Connected/Request events.
        let event = next_event(&mut server.events).await;
        assert!(
            matches!(event, HarnessEvent::ClientDisconnected),
            "{event:?}"
        );
    }

    #[tokio::test]
    async fn request_and_response_round_trip_with_session_log() {
        let mut server = start_test_server("secret").await;
        let mut client = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();
        client.write_all(b"secret\n").await.unwrap();

        let responder = match next_event(&mut server.events).await {
            HarnessEvent::Connected { responder } => responder,
            other => panic!("expected Connected, got {other:?}"),
        };

        client
            .write_all(b"{\"id\":41,\"cmd\":\"ping\"}\n")
            .await
            .unwrap();
        match next_event(&mut server.events).await {
            HarnessEvent::Request(request) => {
                assert_eq!(request.id, 41);
                assert_eq!(request.command, HarnessCommand::Ping {});
            }
            other => panic!("expected Request, got {other:?}"),
        }

        responder
            .send(HarnessResponse::success(
                41,
                serde_json::json!({"pong": true}),
            ))
            .unwrap();
        let line = read_line(&mut client).await;
        let response: HarnessResponse = serde_json::from_str(&line).unwrap();
        assert!(response.ok);
        assert_eq!(response.id, 41);

        // session.json carries the real port + token; session.jsonl has both
        // directions.
        let session: SessionInfo = serde_json::from_str(
            &std::fs::read_to_string(server.run_dir.path().join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(session.port, server.port);
        assert_eq!(session.token, "secret");
        assert_eq!(session.run_id, "test-run");

        drop(client);
        let event = next_event(&mut server.events).await;
        assert!(
            matches!(event, HarnessEvent::ClientDisconnected),
            "{event:?}"
        );
        drop(responder);

        let log = std::fs::read_to_string(server.run_dir.path().join("session.jsonl")).unwrap();
        let directions: Vec<String> = log
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["dir"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(directions, vec!["req", "resp"]);
    }

    #[tokio::test]
    async fn malformed_json_gets_error_response_with_recovered_id() {
        let mut server = start_test_server("secret").await;
        let mut client = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();
        client.write_all(b"secret\n").await.unwrap();
        let _connected = next_event(&mut server.events).await;

        client
            .write_all(b"{\"id\":9,\"cmd\":\"explode\"}\n")
            .await
            .unwrap();
        let line = read_line(&mut client).await;
        let response: HarnessResponse = serde_json::from_str(&line).unwrap();
        assert!(!response.ok);
        assert_eq!(response.id, 9);
        assert_eq!(response.error.unwrap().code, "bad_request");

        client.write_all(b"not json at all\n").await.unwrap();
        let line = read_line(&mut client).await;
        let response: HarnessResponse = serde_json::from_str(&line).unwrap();
        assert_eq!(response.id, 0);
        assert!(!response.ok);
    }

    #[tokio::test]
    async fn client_can_reconnect_after_disconnect() {
        let mut server = start_test_server("secret").await;

        let mut first = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();
        first.write_all(b"secret\n").await.unwrap();
        let first_responder = match next_event(&mut server.events).await {
            HarnessEvent::Connected { responder } => responder,
            other => panic!("expected Connected, got {other:?}"),
        };
        drop(first);
        let event = next_event(&mut server.events).await;
        assert!(
            matches!(event, HarnessEvent::ClientDisconnected),
            "{event:?}"
        );
        drop(first_responder);

        let mut second = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();
        second.write_all(b"secret\n").await.unwrap();
        let responder = match next_event(&mut server.events).await {
            HarnessEvent::Connected { responder } => responder,
            other => panic!("expected Connected, got {other:?}"),
        };
        second
            .write_all(b"{\"id\":1,\"cmd\":\"ping\"}\n")
            .await
            .unwrap();
        match next_event(&mut server.events).await {
            HarnessEvent::Request(request) => assert_eq!(request.id, 1),
            other => panic!("expected Request, got {other:?}"),
        }
        responder
            .send(HarnessResponse::success(
                1,
                serde_json::json!({"pong": true}),
            ))
            .unwrap();
        let line = read_line(&mut second).await;
        assert!(line.contains("\"ok\":true"));
    }

    #[test]
    fn extract_request_id_recovers_when_possible() {
        assert_eq!(extract_request_id(r#"{"id":12,"cmd":"explode"}"#), 12);
        assert_eq!(extract_request_id("garbage"), 0);
        assert_eq!(extract_request_id(r#"{"cmd":"x"}"#), 0);
    }
}
