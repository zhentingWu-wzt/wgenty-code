//! Shared harness for daemon HTTP integration tests.
//!
//! Boots the real Axum router (`routes::create_routers`) on an ephemeral
//! loopback port with a tempdir-backed `DaemonState`, so tests exercise the
//! actual HTTP + SSE stack (auth, routing, serialization) instead of calling
//! handlers in-process. Everything is single-process: no real daemon binary,
//! no developer `~/.wgenty-code` state.

#![allow(dead_code)] // helpers are used by a subset of the test modules

use std::sync::Arc;
use wgenty_code::config::Settings;
use wgenty_code::daemon::routes;
use wgenty_code::daemon::state::DaemonState;
use wgenty_code::state::AppState;

pub const TEST_TOKEN: &str = "integration-test-token";

pub struct TestDaemon {
    /// Base URL including the `/api/v1` prefix.
    pub base: String,
    pub state: Arc<DaemonState>,
    /// Client with the bearer token pre-configured.
    pub client: reqwest::Client,
    /// Keep alive: sessions/config land under this dir.
    pub _temp: tempfile::TempDir,
    pub _server: tokio::task::JoinHandle<()>,
}

/// Boot a daemon with default test settings (tempdir working dir).
pub async fn spawn_daemon() -> TestDaemon {
    spawn_daemon_custom(|_| {}, |_| {}).await
}

/// Boot a daemon, customizing `Settings` before `DaemonState` construction and
/// the `DaemonState` itself (e.g. shrinking the session event hub) before it
/// is shared with the router.
pub async fn spawn_daemon_custom(
    configure_settings: impl FnOnce(&mut Settings),
    tweak_state: impl FnOnce(&mut DaemonState),
) -> TestDaemon {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    configure_settings(&mut settings);

    let mut state = DaemonState::new(AppState::new(settings)).await;
    // Isolate the registry from the developer's real projects.json.
    state.projects = wgenty_code::daemon::projects::ProjectRegistry::load(
        temp.path().to_path_buf(),
        temp.path().join("projects.json"),
    );
    tweak_state(&mut state);
    let state = Arc::new(state);

    let (health, protected) = routes::create_routers(state.clone(), TEST_TOKEN.to_string());
    let app = health.merge(protected);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test daemon");
    });

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {TEST_TOKEN}").parse().expect("auth header"),
    );
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("reqwest client");

    TestDaemon {
        base: format!("http://{addr}/api/v1"),
        state,
        client,
        _temp: temp,
        _server: server,
    }
}

/// Create a session via `POST /sessions` and return its id.
pub async fn create_session(d: &TestDaemon, name: &str) -> String {
    let resp = d
        .client
        .post(format!("{}/sessions", d.base))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .expect("create session request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "create session");
    let body: serde_json::Value = resp.json().await.expect("session body");
    body["id"].as_str().expect("session id").to_string()
}

/// Incremental parser for an SSE response body: yields the payload of each
/// `data:` frame, skipping keep-alive comments. Frames are delimited by a
/// blank line; chunks may split a frame anywhere.
pub struct SseReader {
    stream:
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    buf: String,
}

impl SseReader {
    /// Connect to an SSE endpoint; the HTTP response headers must already be
    /// received, which guarantees the server-side subscription exists (the
    /// handlers subscribe before responding).
    pub async fn connect(client: &reqwest::Client, url: &str) -> Self {
        let resp = client.get(url).send().await.expect("SSE connect");
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::OK,
            "SSE endpoint must accept the subscription"
        );
        Self {
            stream: Box::pin(resp.bytes_stream()),
            buf: String::new(),
        }
    }

    /// Next `data:` frame payload (multi-line `data:` joined with `\n`).
    pub async fn next_data(&mut self) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(10), self.next_data_inner())
            .await
            .expect("SSE frame within 10s")
    }

    async fn next_data_inner(&mut self) -> String {
        use futures::StreamExt as _;
        loop {
            // Drain complete frames already buffered.
            while let Some(end) = self.buf.find("\n\n") {
                let frame: String = self.buf.drain(..end + 2).collect();
                let data: Vec<&str> = frame
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .map(|l| l.trim_start())
                    .collect();
                if !data.is_empty() {
                    return data.join("\n");
                }
                // Comment-only frame (keep-alive): keep waiting.
            }
            let chunk = self
                .stream
                .next()
                .await
                .expect("SSE stream open")
                .expect("SSE chunk ok");
            self.buf
                .push_str(std::str::from_utf8(&chunk).expect("utf8 SSE chunk"));
        }
    }

    /// Next frame parsed as a JSON value.
    pub async fn next_json(&mut self) -> serde_json::Value {
        serde_json::from_str(&self.next_data().await).expect("SSE frame is JSON")
    }
}
