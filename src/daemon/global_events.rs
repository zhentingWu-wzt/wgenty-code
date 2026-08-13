//! Global (daemon-wide, cross-project) event bus: todos changes, background
//! results, permission-mode / model switches, task-group results. Separate
//! envelope and seq space from the per-session `SessionEventHub` so
//! high-frequency session deltas can't starve global events (design §3.1).
//! v1 is live-only — clients realign via the existing GET endpoints.

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// One event on the global bus. `seq` is monotonic across the daemon process
/// for client dedup/ordering; it is NOT resumable after a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalEvent {
    pub seq: u64,
    pub kind: GlobalEventKind,
    /// Kind-specific payload. Cross-project events carry project/session
    /// dimension fields so clients can filter (design §10).
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GlobalEventKind {
    /// Full todos snapshot (small; YAGNI: no incremental diff).
    TodosChanged,
    BackgroundResult,
    ModeChanged,
    ModelChanged,
    TaskGroupResult,
    /// Transport-level config changed (max_tokens / timeout / streaming / api_base).
    ConfigChanged,
}

pub type GlobalEventHub = tokio::sync::broadcast::Sender<GlobalEvent>;

/// Hub channel capacity; aligned with the session event hub.
pub const GLOBAL_EVENT_HUB_CAPACITY: usize = 1024;

pub fn new_global_event_hub() -> GlobalEventHub {
    tokio::sync::broadcast::channel(GLOBAL_EVENT_HUB_CAPACITY).0
}

/// GET /api/v1/events — global SSE stream (live-only, v1, design §3.4).
///
/// Contract:
/// - Arrival order is NOT guaranteed to equal `seq` order: with multiple
///   concurrent publishers the broadcasts interleave, so clients MUST
///   sort/dedup by `seq` on receipt.
/// - Global events are low-frequency and the stream carries no replay: when
///   events lag (or after any reconnect) the client realigns by re-reading
///   the plain GET endpoints (GET /todos, GET /background/results, ...).
///
/// Clients that disconnect re-subscribe and realign the same way. Keep-alive
/// every 15s.
pub(crate) async fn get_global_events(
    State(state): State<Arc<crate::daemon::state::DaemonState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    // Subscribe before responding so no event is missed between connect and
    // stream start (mirrors get_session_events).
    let mut live = state.global_event_hub.subscribe();

    tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Global events are low-frequency and clients realign via
                    // the GET endpoints, so a warn is enough here — no
                    // per-connection sync_lost frame (unlike run_loop.rs).
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "global events SSE subscriber lagged; it should realign via GET endpoints"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::state::DaemonState;

    /// Mirrors the construction path used by handlers.rs tests: tempdir-backed
    /// settings so nothing touches the developer's real project state.
    async fn test_daemon_state() -> DaemonState {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        // All construction-time I/O finishes inside `DaemonState::new`; the
        // broadcast assertions below don't touch the working dir, so the
        // tempdir can drop when this helper returns.
        DaemonState::new(crate::state::AppState::new(settings)).await
    }

    #[tokio::test]
    async fn global_events_stream_is_live_only() {
        let hub = new_global_event_hub();
        // 订阅前的事件不可见。
        let _ = hub.send(GlobalEvent {
            seq: 1,
            kind: GlobalEventKind::ModeChanged,
            data: serde_json::json!({}),
        });
        let mut rx = hub.subscribe();
        let _ = hub.send(GlobalEvent {
            seq: 2,
            kind: GlobalEventKind::ModelChanged,
            data: serde_json::json!({}),
        });
        let got = rx.recv().await.expect("live event");
        assert_eq!(got.seq, 2);
    }

    /// The SSE handler streams events published after the connection as
    /// `data:` frames carrying the serialized `GlobalEvent` envelope.
    #[tokio::test]
    async fn get_global_events_streams_live_events_as_sse_data() {
        use axum::response::IntoResponse;
        use futures::StreamExt;
        use std::sync::Arc;

        let state = Arc::new(test_daemon_state().await);
        // The handler subscribes before returning, so an event published
        // right after connect must not be missed.
        let response = get_global_events(axum::extract::State(state.clone()))
            .await
            .into_response();
        state.broadcast_global(
            GlobalEventKind::TodosChanged,
            serde_json::json!({"todos": []}),
        );
        let mut body = response.into_body().into_data_stream();
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
            .await
            .expect("SSE frame within timeout")
            .expect("stream open")
            .expect("frame ok");
        let text = String::from_utf8(frame.to_vec()).expect("utf8 frame");
        assert!(
            text.starts_with("data:"),
            "expected SSE data frame, got: {text}"
        );
        assert!(text.contains("todos_changed"), "frame: {text}");
        assert!(text.contains("\"seq\":1"), "frame: {text}");
    }

    #[tokio::test]
    async fn broadcast_global_assigns_monotonic_seq_to_all_subscribers() {
        let state = test_daemon_state().await;
        let mut a = state.global_event_hub.subscribe();
        let mut b = state.global_event_hub.subscribe();
        state.broadcast_global(
            GlobalEventKind::ModeChanged,
            serde_json::json!({"mode": "yolo"}),
        );
        state.broadcast_global(
            GlobalEventKind::ModelChanged,
            serde_json::json!({"profile": "p1"}),
        );
        for expected_seq in [1u64, 2] {
            let ea = a.recv().await.expect("subscriber a");
            let eb = b.recv().await.expect("subscriber b");
            assert_eq!(ea.seq, expected_seq);
            assert_eq!(eb.seq, expected_seq);
        }
    }
}
