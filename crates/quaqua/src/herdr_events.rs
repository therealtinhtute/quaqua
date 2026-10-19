//! Herdr event subscription activity client.
//!
//! Connects to the local herdr Unix domain socket and subscribes to lifecycle
//! and pane status events (`pane.agent_status_changed`, `pane.exited`) over
//! newline-delimited JSON. Implements the engine [`Activity`] trait.
//!
//! # Protocol & Contract
//! - Transport: Newline-delimited JSON over a local Unix domain socket.
//! - Subscriptions: Pane-level events (`pane.agent_status_changed`, `pane.exited`)
//!   require a valid pane ID (such as `"w1:p1"`). The `agent` parameter in
//!   [`HerdrEventRequest::Subscribe`] must be a valid pane ID.
//! - Bounded Handshake: `subscribe()` enforces an overall deadline equal to `read_timeout`
//!   and caps early buffered events to prevent hanging on chattering non-ack peers.
//! - Bounded Buffering: Incoming events are buffered in a FIFO queue capped at 10,000
//!   events. If unpolled events exceed this cap, the oldest events are dropped to prevent
//!   unbounded memory growth. Unpolled events are cleared on re-subscribe or unsubscribe.
//! - Disconnection & EOF: If the herdr server closes the socket connection, any remaining
//!   buffered events are drained first; once empty, subsequent polls return
//!   [`ActivityError::ExecutionFailed`] to alert the caller that the subscription is dead.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::activity::{
    Activity, ActivityError, ActivityRequest, ActivityResult, HerdrEventRequest, HerdrEventResult,
};

static REQ_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_BUFFERED_EVENTS: usize = 10_000;

fn next_req_id() -> String {
    let id = REQ_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sub_req_{id}")
}

// ----------------------------------------------------------------------------
// JSON-RPC DTOs per `herdr api schema --json`
// ----------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RpcRequest<P> {
    id: String,
    method: String,
    params: P,
}

#[derive(Debug, Serialize)]
struct EventsSubscribeParams {
    subscriptions: Vec<SubscriptionEntry>,
}

#[derive(Debug, Serialize)]
struct SubscriptionEntry {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pane_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcRawResponse {
    id: Option<String>,
    #[allow(dead_code)]
    result: Option<serde_json::Value>,
    error: Option<RpcErrorBody>,
}

#[derive(Debug, Deserialize)]
struct RpcErrorBody {
    code: Option<String>,
    message: Option<String>,
}

// ----------------------------------------------------------------------------
// HerdrEvents Inner State
// ----------------------------------------------------------------------------

#[derive(Default)]
struct HerdrEventsInner {
    stream: Option<UnixStream>,
    active_agent: Option<String>,
    connection_closed: bool,
    read_buf: Vec<u8>,
    buffered_events: VecDeque<String>,
}

// ----------------------------------------------------------------------------
// HerdrEvents Activity Implementation
// ----------------------------------------------------------------------------

/// Client for herdr event streaming over Unix domain socket.
///
/// Implements [`Activity`] for [`HerdrEventRequest`].
pub struct HerdrEvents {
    socket_path_override: Option<PathBuf>,
    read_timeout: Duration,
    state: Mutex<HerdrEventsInner>,
}

impl HerdrEvents {
    /// Creates a new `HerdrEvents` activity client resolving `HERDR_SOCKET_PATH` at call time.
    pub fn new() -> Self {
        Self {
            socket_path_override: None,
            read_timeout: DEFAULT_TIMEOUT,
            state: Mutex::new(HerdrEventsInner::default()),
        }
    }

    /// Creates an activity client pointed at a specific socket path (used by tests).
    pub fn with_socket_path<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            socket_path_override: Some(path.into()),
            read_timeout: DEFAULT_TIMEOUT,
            state: Mutex::new(HerdrEventsInner::default()),
        }
    }

    /// Creates an activity client with a specific socket path and read timeout.
    pub fn with_socket_path_and_timeout<P: Into<PathBuf>>(path: P, timeout: Duration) -> Self {
        Self {
            socket_path_override: Some(path.into()),
            read_timeout: timeout,
            state: Mutex::new(HerdrEventsInner::default()),
        }
    }

    fn resolve_socket_path(&self) -> Result<PathBuf, ActivityError> {
        if let Some(path) = &self.socket_path_override {
            return Ok(path.clone());
        }
        match std::env::var_os("HERDR_SOCKET_PATH") {
            Some(val) if !val.is_empty() => Ok(PathBuf::from(val)),
            _ => Err(ActivityError::NotFound(
                "HERDR_SOCKET_PATH environment variable not set or empty".to_string(),
            )),
        }
    }

    /// Subscribes to herdr events for the given agent and event types.
    pub fn subscribe(&self, agent: &str, event_types: &[String]) -> Result<(), ActivityError> {
        if agent.trim().is_empty() {
            return Err(ActivityError::ExecutionFailed(
                "agent identifier must not be empty (required for herdr pane subscription)"
                    .to_string(),
            ));
        }

        let socket_path = self.resolve_socket_path()?;
        let mut stream = match UnixStream::connect(&socket_path) {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("failed to connect to socket {}: {e}", socket_path.display());
                return match e.kind() {
                    std::io::ErrorKind::NotFound => Err(ActivityError::NotFound(msg)),
                    _ => Err(ActivityError::ExecutionFailed(msg)),
                };
            }
        };

        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|e| {
                ActivityError::ExecutionFailed(format!("failed to set read timeout: {e}"))
            })?;
        stream
            .set_write_timeout(Some(self.read_timeout))
            .map_err(|e| {
                ActivityError::ExecutionFailed(format!("failed to set write timeout: {e}"))
            })?;

        let target_types = if event_types.is_empty() {
            vec![
                "pane.agent_status_changed".to_string(),
                "pane.exited".to_string(),
            ]
        } else {
            event_types.to_vec()
        };

        let subscriptions: Vec<SubscriptionEntry> = target_types
            .into_iter()
            .map(|t| {
                let pane_id = if t.starts_with("pane.") {
                    Some(agent.to_string())
                } else {
                    None
                };
                SubscriptionEntry {
                    event_type: t,
                    pane_id,
                }
            })
            .collect();

        let req_id = next_req_id();
        let rpc_req = RpcRequest {
            id: req_id.clone(),
            method: "events.subscribe".to_string(),
            params: EventsSubscribeParams { subscriptions },
        };

        let mut json_bytes = serde_json::to_vec(&rpc_req)
            .map_err(|e| ActivityError::ExecutionFailed(format!("serialization error: {e}")))?;
        json_bytes.push(b'\n');

        stream.write_all(&json_bytes).map_err(|e| {
            ActivityError::ExecutionFailed(format!("failed to write subscribe request: {e}"))
        })?;
        stream.flush().map_err(|e| {
            ActivityError::ExecutionFailed(format!("failed to flush subscribe request: {e}"))
        })?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut early_events = Vec::new();
        let deadline = Instant::now() + self.read_timeout;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(ActivityError::Timeout(format!(
                    "timed out waiting for subscription response after {:?}",
                    self.read_timeout
                )));
            }
            let remaining = deadline.saturating_duration_since(now);
            let _ = reader.get_ref().set_read_timeout(Some(remaining));

            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    return Err(ActivityError::ExecutionFailed(
                        "connection closed by server before subscription response".to_string(),
                    ));
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(resp) = serde_json::from_str::<RpcRawResponse>(trimmed) {
                        if resp.id.as_deref() == Some(&req_id) {
                            if let Some(err) = resp.error {
                                let code = err.code.unwrap_or_else(|| "unknown_error".to_string());
                                let message =
                                    err.message.unwrap_or_else(|| "no message".to_string());
                                return Err(ActivityError::ExecutionFailed(format!(
                                    "herdr subscription error ({code}): {message}"
                                )));
                            }
                            break;
                        }
                    }
                    if early_events.len() >= MAX_BUFFERED_EVENTS {
                        return Err(ActivityError::ExecutionFailed(
                            "exceeded maximum buffered early events while waiting for subscription ack"
                                .to_string(),
                        ));
                    }
                    early_events.push(trimmed.to_string());
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock
                    {
                        return Err(ActivityError::Timeout(format!(
                            "timed out waiting for subscription response after {:?}",
                            self.read_timeout
                        )));
                    }
                    return Err(ActivityError::ExecutionFailed(format!(
                        "socket read error: {e}"
                    )));
                }
            }
        }

        let unconsumed = reader.buffer().to_vec();
        let read_stream = reader.into_inner();
        read_stream.set_nonblocking(true).map_err(|e| {
            ActivityError::ExecutionFailed(format!("failed to set nonblocking mode: {e}"))
        })?;

        let mut inner = self.state.lock().map_err(|_| {
            ActivityError::ExecutionFailed("internal state mutex poisoned".to_string())
        })?;

        if let Some(old_stream) = inner.stream.take() {
            let _ = old_stream.shutdown(std::net::Shutdown::Both);
        }
        inner.read_buf.clear();
        inner.buffered_events.clear();
        inner.connection_closed = false;

        if !unconsumed.is_empty() {
            inner.read_buf.extend_from_slice(&unconsumed);
        }
        for ev in early_events {
            inner.buffered_events.push_back(ev);
        }
        inner.stream = Some(read_stream);
        inner.active_agent = Some(agent.to_string());

        Ok(())
    }

    /// Drains all buffered socket events non-blocking.
    ///
    /// If the connection was closed by the server (EOF), any remaining buffered events
    /// are returned first. Once buffered events are exhausted, subsequent calls return
    /// [`ActivityError::ExecutionFailed`] to notify the caller of the closed connection.
    pub fn poll_events(&self) -> Result<Vec<String>, ActivityError> {
        let mut inner = self.state.lock().map_err(|_| {
            ActivityError::ExecutionFailed("internal state mutex poisoned".to_string())
        })?;
        let HerdrEventsInner {
            stream,
            active_agent,
            connection_closed,
            read_buf,
            buffered_events,
        } = &mut *inner;

        let mut hit_eof = false;

        if let Some(s) = stream {
            let mut buf = [0u8; 8192];
            loop {
                match s.read(&mut buf) {
                    Ok(0) => {
                        hit_eof = true;
                        break;
                    }
                    Ok(n) => {
                        read_buf.extend_from_slice(&buf[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                        continue;
                    }
                    Err(e) => {
                        return Err(ActivityError::ExecutionFailed(format!(
                            "socket read error: {e}"
                        )));
                    }
                }
            }
        }

        if hit_eof {
            *stream = None;
            *connection_closed = true;
        }

        while let Some(pos) = read_buf.iter().position(|&b| b == b'\n') {
            let line_bytes = &read_buf[..pos];
            let line = String::from_utf8_lossy(line_bytes).trim().to_string();
            read_buf.drain(..=pos);
            if !line.is_empty() {
                if buffered_events.len() >= MAX_BUFFERED_EVENTS {
                    buffered_events.pop_front();
                }
                buffered_events.push_back(line);
            }
        }

        if !buffered_events.is_empty() {
            let events: Vec<String> = buffered_events.drain(..).collect();
            return Ok(events);
        }

        if *connection_closed {
            return Err(ActivityError::ExecutionFailed(
                "herdr socket connection closed by server (EOF)".to_string(),
            ));
        }

        if active_agent.is_none() {
            return Ok(Vec::new());
        }

        Ok(Vec::new())
    }

    /// Unsubscribes and closes the active socket connection.
    pub fn unsubscribe(&self, _agent: &str) -> Result<(), ActivityError> {
        let mut inner = self.state.lock().map_err(|_| {
            ActivityError::ExecutionFailed("internal state mutex poisoned".to_string())
        })?;

        if let Some(stream) = inner.stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        inner.active_agent = None;
        inner.connection_closed = false;
        inner.read_buf.clear();
        inner.buffered_events.clear();
        Ok(())
    }
}

impl Default for HerdrEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl Activity for HerdrEvents {
    fn execute(&self, req: &ActivityRequest) -> Result<ActivityResult, ActivityError> {
        match req {
            ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe { agent, event_types }) => {
                self.subscribe(agent, event_types)?;
                Ok(ActivityResult::HerdrEvents(HerdrEventResult::Subscribed {
                    agent: agent.clone(),
                }))
            }
            ActivityRequest::HerdrEvents(HerdrEventRequest::Unsubscribe { agent }) => {
                self.unsubscribe(agent)?;
                Ok(ActivityResult::HerdrEvents(
                    HerdrEventResult::Unsubscribed {
                        agent: agent.clone(),
                    },
                ))
            }
            ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents) => {
                let events = self.poll_events()?;
                Ok(ActivityResult::HerdrEvents(HerdrEventResult::Events(
                    events,
                )))
            }
            other => Err(ActivityError::ExecutionFailed(format!(
                "unsupported activity request: {other:?}"
            ))),
        }
    }
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    static TEST_SOCK_COUNTER: AtomicUsize = AtomicUsize::new(1);

    struct TestSocketGuard(PathBuf);

    impl Drop for TestSocketGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp_socket_path() -> (PathBuf, TestSocketGuard) {
        let n = TEST_SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "quaqua_test_herdr_ev_{}_{}_{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            n
        ));
        let _ = std::fs::remove_file(&path);
        let guard = TestSocketGuard(path.clone());
        (path, guard)
    }

    fn poll_until_events(client: &HerdrEvents, timeout: Duration) -> Vec<String> {
        let start = Instant::now();
        let poll_req = ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents);
        let mut all_events = Vec::new();
        while start.elapsed() < timeout {
            match client.execute(&poll_req) {
                Ok(ActivityResult::HerdrEvents(HerdrEventResult::Events(evs))) => {
                    all_events.extend(evs);
                    if !all_events.is_empty() {
                        break;
                    }
                }
                Ok(other) => panic!("unexpected result: {other:?}"),
                Err(e) => panic!("poll failed with error: {e:?}"),
            }
            thread::sleep(Duration::from_millis(10));
        }
        all_events
    }

    #[test]
    fn test_stub_socket_subscribe_and_poll() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            // Verify subscribe request contains method and params with pane_id
            let req_val: serde_json::Value =
                serde_json::from_str(&line).expect("valid json request");
            assert_eq!(req_val["method"], "events.subscribe");
            let subs = req_val["params"]["subscriptions"].as_array().unwrap();
            for sub in subs {
                assert_eq!(sub["pane_id"], "w1:p1");
            }
            let req_id = req_val["id"].as_str().unwrap().to_string();

            // Reply with success response AND event in one write/flush to avoid timing race
            let resp = serde_json::json!({
                "id": req_id,
                "result": { "type": "subscription_started" }
            });
            let event = serde_json::json!({
                "event": "pane.agent_status_changed",
                "data": {
                    "pane_id": "w1:p1",
                    "workspace_id": "w1",
                    "agent": "test-agent",
                    "agent_status": "working"
                }
            });
            stream
                .write_all(format!("{}\n{}\n", resp, event).as_bytes())
                .expect("write response and event");
            stream.flush().expect("flush response and event");

            // Keep connection open until client closes it
            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
        });

        let client = HerdrEvents::with_socket_path(&sock_path);

        // 1. Subscribe
        let sub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
            agent: "w1:p1".to_string(),
            event_types: vec!["pane.agent_status_changed".to_string()],
        });
        let sub_res = client.execute(&sub_req).expect("subscribe succeeds");
        assert_eq!(
            sub_res,
            ActivityResult::HerdrEvents(HerdrEventResult::Subscribed {
                agent: "w1:p1".to_string()
            })
        );

        // 2. Poll events: should return the pushed event
        let events = poll_until_events(&client, Duration::from_secs(2));
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("pane.agent_status_changed"));
        assert!(events[0].contains("working"));

        // 3. Poll again: should be drained (empty)
        let poll_req = ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents);
        let poll_res2 = client.execute(&poll_req).expect("poll succeeds");
        match poll_res2 {
            ActivityResult::HerdrEvents(HerdrEventResult::Events(evs)) => {
                assert_eq!(evs.len(), 0);
            }
            other => panic!("unexpected poll result: {other:?}"),
        }

        // 4. Unsubscribe
        let unsub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Unsubscribe {
            agent: "w1:p1".to_string(),
        });
        let unsub_res = client.execute(&unsub_req).expect("unsubscribe succeeds");
        assert_eq!(
            unsub_res,
            ActivityResult::HerdrEvents(HerdrEventResult::Unsubscribed {
                agent: "w1:p1".to_string()
            })
        );

        let _ = handle.join();
    }

    #[test]
    fn test_stub_socket_multiple_events() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            let req_val: serde_json::Value =
                serde_json::from_str(&line).expect("valid json request");
            let subs = req_val["params"]["subscriptions"].as_array().unwrap();
            for sub in subs {
                assert_eq!(sub["pane_id"], "w1:p1");
            }
            let req_id = req_val["id"].as_str().unwrap().to_string();

            let resp = serde_json::json!({
                "id": req_id,
                "result": { "type": "subscription_started" }
            });
            let ev1 = serde_json::json!({
                "event": "pane.agent_status_changed",
                "data": { "pane_id": "w1:p1", "agent_status": "working" }
            });
            let ev2 = serde_json::json!({
                "event": "pane.exited",
                "data": { "pane_id": "w1:p1" }
            });

            stream
                .write_all(format!("{}\n{}\n{}\n", resp, ev1, ev2).as_bytes())
                .expect("write response and events");
            stream.flush().expect("flush events");

            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
        });

        let client = HerdrEvents::with_socket_path(&sock_path);
        let sub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Subscribe {
                agent: "w1:p1".to_string(),
                event_types: vec![],
            },
        ));
        assert!(sub_res.is_ok());

        let events = poll_until_events(&client, Duration::from_secs(2));
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("pane.agent_status_changed"));
        assert!(events[1].contains("pane.exited"));

        let unsub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Unsubscribe {
                agent: "w1:p1".to_string(),
            },
        ));
        assert!(unsub_res.is_ok());

        let _ = handle.join();
    }

    #[test]
    fn test_subscribe_empty_agent_rejected() {
        let client = HerdrEvents::with_socket_path("/tmp/not_used.sock");
        let sub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
            agent: "   ".to_string(),
            event_types: vec![],
        });
        let res = client.execute(&sub_req);
        assert!(res.is_err());
        match res.err().unwrap() {
            ActivityError::ExecutionFailed(msg) => {
                assert!(msg.contains("agent identifier must not be empty"));
            }
            other => panic!("expected ExecutionFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_stub_socket_eof_returns_error() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            let req_val: serde_json::Value =
                serde_json::from_str(&line).expect("valid json request");
            let req_id = req_val["id"].as_str().unwrap().to_string();

            let resp = serde_json::json!({
                "id": req_id,
                "result": { "type": "subscription_started" }
            });
            stream
                .write_all(format!("{}\n", resp).as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");

            // Close the server end immediately to trigger EOF
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });

        let client = HerdrEvents::with_socket_path(&sock_path);
        let sub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Subscribe {
                agent: "w1:p1".to_string(),
                event_types: vec![],
            },
        ));
        assert!(sub_res.is_ok());

        // Poll should detect EOF and return error
        let poll_req = ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents);
        let poll_res = client.execute(&poll_req);
        assert!(poll_res.is_err(), "EOF must not be silently swallowed");
        match poll_res.err().unwrap() {
            ActivityError::ExecutionFailed(msg) => {
                assert!(msg.contains("closed by server (EOF)"));
            }
            other => panic!("expected ExecutionFailed, got: {other:?}"),
        }

        let _ = handle.join();
    }

    #[test]
    fn test_subscribe_overall_deadline_chatter() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            // Server keeps sending non-ack lines rapidly (every 10ms)
            for i in 0..100 {
                let chatter = serde_json::json!({
                    "id": format!("wrong_id_{i}"),
                    "event": "some.noise",
                    "data": {}
                });
                if stream
                    .write_all(format!("{}\n", chatter).as_bytes())
                    .is_err()
                {
                    break;
                }
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(10));
            }
        });

        // 100ms overall timeout
        let client =
            HerdrEvents::with_socket_path_and_timeout(&sock_path, Duration::from_millis(100));
        let start = Instant::now();
        let res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Subscribe {
                agent: "w1:p1".to_string(),
                event_types: vec![],
            },
        ));
        let elapsed = start.elapsed();

        assert!(
            res.is_err(),
            "chattering non-ack peer must hit overall deadline"
        );
        assert!(
            matches!(res.err().unwrap(), ActivityError::Timeout(_)),
            "expected ActivityError::Timeout"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "overall deadline should trigger promptly, took {:?}",
            elapsed
        );

        let _ = handle.join();
    }

    #[test]
    fn test_multibyte_utf8_across_read_boundaries() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        // Non-ASCII string with Vietnamese multi-byte characters
        let title = "✳ Chuyển đổi ompz sang pi — Tiêu đề dài";

        let title_clone = title.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            let req_val: serde_json::Value =
                serde_json::from_str(&line).expect("valid json request");
            let req_id = req_val["id"].as_str().unwrap().to_string();

            let resp = serde_json::json!({
                "id": req_id,
                "result": { "type": "subscription_started" }
            });
            stream
                .write_all(format!("{}\n", resp).as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");

            let event = serde_json::json!({
                "event": "pane.agent_status_changed",
                "data": {
                    "pane_id": "w1:p1",
                    "title": title_clone
                }
            });
            let event_bytes = format!("{}\n", event).into_bytes();

            // Split the event bytes in the middle of a multi-byte sequence (chunk sizes 3, 5, rest)
            stream.write_all(&event_bytes[..3]).expect("write chunk 1");
            stream.flush().expect("flush 1");
            thread::sleep(Duration::from_millis(15));
            stream.write_all(&event_bytes[3..8]).expect("write chunk 2");
            stream.flush().expect("flush 2");
            thread::sleep(Duration::from_millis(15));
            stream.write_all(&event_bytes[8..]).expect("write chunk 3");
            stream.flush().expect("flush 3");

            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
        });

        let client = HerdrEvents::with_socket_path(&sock_path);
        let sub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Subscribe {
                agent: "w1:p1".to_string(),
                event_types: vec![],
            },
        ));
        assert!(sub_res.is_ok());

        let events = poll_until_events(&client, Duration::from_secs(2));
        assert_eq!(events.len(), 1);
        assert!(
            events[0].contains(title),
            "multibyte UTF-8 must not be corrupted across chunk boundaries. Got: {}",
            events[0]
        );
        assert!(
            !events[0].contains('\u{FFFD}'),
            "no replacement char U+FFFD should appear"
        );

        let unsub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Unsubscribe {
                agent: "w1:p1".to_string(),
            },
        ));
        assert!(unsub_res.is_ok());

        let _ = handle.join();
    }

    #[test]
    fn test_dead_unreachable_socket_path() {
        let bad_path = std::env::temp_dir().join("quaqua_nonexistent_dead_socket_123456.sock");
        let _ = std::fs::remove_file(&bad_path);

        let client = HerdrEvents::with_socket_path(&bad_path);
        let sub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
            agent: "w1:p1".to_string(),
            event_types: vec![],
        });

        let res = client.execute(&sub_req);
        assert!(
            res.is_err(),
            "connecting to dead socket must fail with ActivityError"
        );
        let err = res.err().unwrap();
        assert!(
            matches!(
                err,
                ActivityError::NotFound(_) | ActivityError::ExecutionFailed(_)
            ),
            "unexpected error variant: {err:?}"
        );
    }

    #[test]
    fn test_silent_socket_timeout() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            thread::sleep(Duration::from_millis(300));
            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
        });

        let client =
            HerdrEvents::with_socket_path_and_timeout(&sock_path, Duration::from_millis(80));
        let sub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
            agent: "w1:p1".to_string(),
            event_types: vec![],
        });

        let start = Instant::now();
        let res = client.execute(&sub_req);
        let elapsed = start.elapsed();

        assert!(res.is_err(), "silent socket must time out rather than hang");
        let err = res.err().unwrap();
        assert!(
            matches!(err, ActivityError::Timeout(_)),
            "expected ActivityError::Timeout, got: {err:?}"
        );
        assert!(
            elapsed < Duration::from_millis(600),
            "timeout should occur promptly within deadline"
        );

        let _ = handle.join();
    }

    #[test]
    fn test_server_error_response() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            let req_val: serde_json::Value =
                serde_json::from_str(&line).expect("valid json request");
            let req_id = req_val["id"].as_str().unwrap().to_string();

            let err_resp = serde_json::json!({
                "id": req_id,
                "error": {
                    "code": "pane_not_found",
                    "message": "pane w1:p99 not found"
                }
            });
            stream
                .write_all(format!("{}\n", err_resp).as_bytes())
                .expect("write error response");
            stream.flush().expect("flush error response");
        });

        let client = HerdrEvents::with_socket_path(&sock_path);
        let sub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
            agent: "w1:p99".to_string(),
            event_types: vec!["pane.agent_status_changed".to_string()],
        });

        let res = client.execute(&sub_req);
        assert!(res.is_err());
        let err = res.err().unwrap();
        match err {
            ActivityError::ExecutionFailed(msg) => {
                assert!(msg.contains("pane_not_found"));
                assert!(msg.contains("pane w1:p99 not found"));
            }
            other => panic!("expected ExecutionFailed, got: {other:?}"),
        }

        let _ = handle.join();
    }

    #[test]
    fn test_unsupported_activity_request() {
        let client = HerdrEvents::new();
        let req = ActivityRequest::Cost(crate::engine::activity::CostRequest::QuerySpend {
            session_pattern: "test".to_string(),
        });
        let res = client.execute(&req);
        assert!(res.is_err());
        match res.err().unwrap() {
            ActivityError::ExecutionFailed(msg) => {
                assert!(msg.contains("unsupported activity request"));
            }
            other => panic!("expected ExecutionFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_trait_object_safety() {
        let (sock_path, _guard) = temp_socket_path();
        let client = HerdrEvents::with_socket_path(&sock_path);
        let activity_dyn: Box<dyn Activity> = Box::new(client);

        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<HerdrEvents>();

        let poll_req = ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents);
        let res = activity_dyn.execute(&poll_req);
        assert!(res.is_ok());
    }

    #[test]
    fn test_env_resolution_missing() {
        let client = HerdrEvents::new();
        let orig = std::env::var_os("HERDR_SOCKET_PATH");
        std::env::remove_var("HERDR_SOCKET_PATH");

        let sub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe {
            agent: "w1:p1".to_string(),
            event_types: vec![],
        });
        let res = client.execute(&sub_req);
        assert!(res.is_err());
        assert!(matches!(res.err().unwrap(), ActivityError::NotFound(_)));

        if let Some(val) = orig {
            std::env::set_var("HERDR_SOCKET_PATH", val);
        }
    }

    #[test]
    fn test_poll_and_unsubscribe_when_unconnected() {
        let client = HerdrEvents::with_socket_path("/tmp/not_connected.sock");

        let poll_req = ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents);
        let poll_res = client
            .execute(&poll_req)
            .expect("poll unconnected succeeds");
        assert_eq!(
            poll_res,
            ActivityResult::HerdrEvents(HerdrEventResult::Events(vec![]))
        );

        let unsub_req = ActivityRequest::HerdrEvents(HerdrEventRequest::Unsubscribe {
            agent: "agent_none".to_string(),
        });
        let unsub_res = client
            .execute(&unsub_req)
            .expect("unsubscribe unconnected succeeds");
        assert_eq!(
            unsub_res,
            ActivityResult::HerdrEvents(HerdrEventResult::Unsubscribed {
                agent: "agent_none".to_string(),
            })
        );
    }

    #[test]
    fn test_stub_socket_early_event_before_ack() {
        let (sock_path, _guard) = temp_socket_path();
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read subscribe request");

            let req_val: serde_json::Value =
                serde_json::from_str(&line).expect("valid json request");
            let req_id = req_val["id"].as_str().unwrap().to_string();

            // Send an event frame BEFORE the ack line
            let early_ev = serde_json::json!({
                "event": "pane.agent_status_changed",
                "data": { "pane_id": "w1:p1", "agent_status": "idle" }
            });
            stream
                .write_all(format!("{}\n", early_ev).as_bytes())
                .expect("write early event");
            stream.flush().expect("flush early event");

            // Then send the ack response
            let resp = serde_json::json!({
                "id": req_id,
                "result": { "type": "subscription_started" }
            });
            stream
                .write_all(format!("{}\n", resp).as_bytes())
                .expect("write response");
            stream.flush().expect("flush response");

            let mut buf = [0u8; 16];
            let _ = stream.read(&mut buf);
        });

        let client = HerdrEvents::with_socket_path(&sock_path);
        let sub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Subscribe {
                agent: "w1:p1".to_string(),
                event_types: vec!["pane.agent_status_changed".to_string()],
            },
        ));
        assert!(sub_res.is_ok());

        let events = poll_until_events(&client, Duration::from_secs(2));
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("idle"));

        let unsub_res = client.execute(&ActivityRequest::HerdrEvents(
            HerdrEventRequest::Unsubscribe {
                agent: "w1:p1".to_string(),
            },
        ));
        assert!(unsub_res.is_ok());

        let _ = handle.join();
    }
}
