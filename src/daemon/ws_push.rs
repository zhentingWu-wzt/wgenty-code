//! WebSocket push channel (tasks 2.1/2.2).
//!
//! Single-connection task multiplexing three downlink event sources (session
//! hub, global trace hub, global event bus), the upstream control channel, and
//! a 15s heartbeat into one outbound envelope stream (design §4). Every
//! downstream frame is serialized to a D2 envelope (design §3.2) on a dedicated
//! write half so serialization/sending never blocks a `select!` branch.
//!
//! Task 2.1 ships the skeleton; task 2.2 lands the per-connection session
//! subscription table with `subscribe`/`unsubscribe` handling (design §3.3),
//! cursor replay, and `subscribed` acks. Route registration and auth land in
//! task 2.3.

use crate::daemon::global_events::GlobalEvent;
use crate::daemon::run_loop::{
    plan_catch_up, plan_lagged_resync, sync_lost_event, CatchUp, SessionEvent,
};
use crate::daemon::state::DaemonState;
use crate::teams::trace_sink::TraceEvent;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// Downstream (server → client) message envelope, design §3.2. Tagged by
/// `type`; each variant's payload matches the D2 table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum DownstreamEnvelope {
    /// 15s application-layer keep-alive; no payload.
    Heartbeat,
    /// Global trace stream; the client filters by session on its own.
    Trace { event: Box<TraceEvent> },
    /// Global event bus (the event carries its own `seq`).
    Global { event: GlobalEvent },
    /// Per-session event, pushed only to connections subscribed to that session.
    Session {
        session_id: String,
        event: SessionEvent,
    },
    /// `subscribe` ack; lets the client align its cursor.
    Subscribed { session_id: String, latest_seq: u64 },
    /// Recoverable control error (e.g. subscription limit); connection stays open.
    Error { message: String },
}

/// Upstream (client → server) control message, design §3.2. Tagged by `op`.
/// Unrecognized messages are ignored with a debug log by the connection loop.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum ClientMessage {
    /// Subscribe to a session's events; `after` is the optional replay cursor.
    Subscribe {
        session_id: String,
        #[serde(default)]
        after: Option<u64>,
    },
    /// Unsubscribe from a session.
    Unsubscribe { session_id: String },
}

/// Replay events forwarded per pump iteration before yielding to the hub
/// branches (MAJOR-1 fix: one synchronous 1024-event replay would starve the
/// shared hub receiver and trip `Lagged` under an event storm).
const REPLAY_BATCH: usize = 64;

/// Per-session subscription state for one connection (design §3.3).
///
/// `delivered` is the highest seq actually sent to the client (watermark);
/// `snapshot_tail` is the tail of the subscription-time replay snapshot (live
/// events at or below it are already covered by the snapshot); `pending` holds
/// the not-yet-forwarded replay backlog plus live events that arrived while
/// the backlog drains, strictly ordered.
#[derive(Debug)]
struct SubState {
    delivered: u64,
    snapshot_tail: u64,
    pending: VecDeque<SessionEvent>,
}

/// Maximum live session subscriptions per connection (design §3.3). Exceeding
/// it yields an [`DownstreamEnvelope::Error`] but keeps the connection alive.
const MAX_SUBSCRIPTIONS: usize = 64;

/// Pump tuning knobs grouped to keep [`event_pump`] within clippy's
/// argument-count limit.
#[derive(Debug, Clone)]
struct PumpConfig {
    heartbeat: Duration,
    replay_batch: usize,
    /// The token this connection authenticated with. Compared against the
    /// current expected token on every heartbeat tick; a mismatch (rotation)
    /// closes the connection with 4001 so the client refreshes and reconnects.
    auth_token: Arc<str>,
}

/// `GET /api/v1/ws` handler (design §3.1/§4).
///
/// Auth is in-handler (not the header-only middleware) because browser
/// WebSocket APIs cannot set headers: the token is taken from the `token`
/// query parameter first, falling back to `Authorization: Bearer` for
/// non-browser clients. Rejection is identical to `require_auth` (401 +
/// same body). An empty expected token (daemon not fully initialized)
/// rejects everything. The 16 MiB `max_message_size` matches design §3.1.
#[derive(serde::Deserialize)]
pub(crate) struct WsAuthQuery {
    token: Option<String>,
}

/// In-handler auth decision (design §3.1): query `token` first, then the
/// `Authorization: Bearer` fallback (non-browser clients). An empty expected
/// token — daemon not fully initialized — rejects everything so the
/// uninitialized window cannot be bypassed with an empty query token.
/// Extracted as a pure function so the decision matrix is unit-testable
/// without a live hyper server (the `WebSocketUpgrade` extractor rejects
/// oneshot requests before handler code runs); the wire-level handshake is
/// covered by the task-2.5 tokio-tungstenite integration tests.
pub(crate) fn authorize_ws(expected: &str, query_token: Option<&str>, headers: &HeaderMap) -> bool {
    if expected.is_empty() {
        return false;
    }
    let provided = query_token.or_else(|| {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
    });
    provided == Some(expected)
}

/// Counts an authenticated ws connection as an active thin client so the
/// idle-shutdown timer keeps deferring while the connection is open (task
/// 2.4, design §3.1). Created only after [`authorize_ws`] passes — a
/// rejected handshake is not a client. `Drop` unregisters on every exit
/// path (socket error, pump exit, panic unwind). When registration was
/// refused (daemon already shutting down) drop is a no-op, so the count
/// never underflows.
struct ActiveClientGuard {
    state: Arc<DaemonState>,
    registered: bool,
}

impl ActiveClientGuard {
    fn new(state: &Arc<DaemonState>) -> Self {
        let registered = state.active_clients.register_client();
        Self {
            state: state.clone(),
            registered,
        }
    }
}

impl Drop for ActiveClientGuard {
    fn drop(&mut self) {
        if self.registered {
            self.state.active_clients.unregister_client();
        }
    }
}

pub(crate) async fn ws_handler(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<WsAuthQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let expected = state.current_api_token();
    if !authorize_ws(&expected, query.token.as_deref(), &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            "unauthorized: missing or invalid bearer token",
        )
            .into_response();
    }

    let active_client = ActiveClientGuard::new(&state);
    ws.max_message_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| {
            // Held for the connection's lifetime; dropped when this future
            // ends, releasing the active-client count.
            let _active_client = active_client;
            connection_loop(state, Arc::from(expected.as_str()), socket)
        })
}

/// Per-connection task: split the socket into a writer half (serialized
/// outbound envelopes) and a reader half (parsed control messages), then run
/// the five-way `select!` pump.
async fn connection_loop(state: Arc<DaemonState>, auth_token: Arc<str>, socket: WebSocket) {
    let (mut socket_tx, mut socket_rx) = socket.split();

    // Outbound: envelopes are serialized and sent here so the pump's select
    // branches never block on a slow client.
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Message>(256);
    let writer = tokio::spawn(async move {
        while let Some(msg) = outbound_rx.recv().await {
            if socket_tx.send(msg).await.is_err() {
                break; // client gone
            }
        }
    });

    // Inbound: parse text frames into control messages for the pump.
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<ClientMessage>(64);
    let reader = tokio::spawn(async move {
        while let Some(msg) = socket_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(_) => break,
            };
            match msg {
                Message::Text(text) => match serde_json::from_str::<ClientMessage>(text.as_str()) {
                    Ok(cmd) => {
                        if ctrl_tx.send(cmd).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        tracing::debug!(
                            target: "wgenty::daemon",
                            text = %text.as_str(),
                            "ws: ignoring unrecognized control message"
                        );
                    }
                },
                Message::Close(_) => break,
                // Binary / ping / pong frames are ignored in v1.
                _ => {}
            }
        }
    });

    event_pump(
        state.clone(),
        state.session_event_hub.subscribe(),
        crate::teams::trace_sink::trace_hub_subscribe(),
        state.global_event_hub.subscribe(),
        ctrl_rx,
        outbound_tx,
        PumpConfig {
            heartbeat: Duration::from_secs(15),
            replay_batch: REPLAY_BATCH,
            auth_token,
        },
    )
    .await;

    writer.abort();
    reader.abort();
}

/// The five-way `select!` event pump, kept free of socket types so it can be
/// unit-tested with plain channels (design §4).
async fn event_pump(
    state: Arc<DaemonState>,
    mut session_rx: broadcast::Receiver<SessionEvent>,
    mut trace_rx: broadcast::Receiver<TraceEvent>,
    mut global_rx: broadcast::Receiver<GlobalEvent>,
    mut ctrl_rx: mpsc::Receiver<ClientMessage>,
    outbound_tx: mpsc::Sender<Message>,
    config: PumpConfig,
) {
    let PumpConfig {
        heartbeat,
        replay_batch,
        auth_token,
    } = config;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + heartbeat, heartbeat);
    // Per-connection subscription table (design §3.3); released when the pump
    // exits, so disconnect cleanup is implicit.
    let mut subs: HashMap<String, SubState> = HashMap::new();

    'pump: loop {
        // Replay backlog advances at most `replay_batch` events per
        // subscription per iteration (MAJOR-1: batches, not one burst), so
        // the hub branches below keep being polled between batches.
        if !drain_pending(&mut subs, &outbound_tx, replay_batch).await {
            break 'pump;
        }
        let has_pending = subs.values().any(|s| !s.pending.is_empty());

        tokio::select! {
            biased;
            res = session_rx.recv() => match res {
                Ok(ev) => {
                    let session_id = ev.session_id.clone();
                    let seq = ev.seq;
                    if let Some(sub) = subs.get_mut(&session_id) {
                        if seq > sub.snapshot_tail && seq > sub.delivered {
                            if sub.pending.is_empty() {
                                sub.delivered = seq;
                            if !forward(
                                &outbound_tx,
                                DownstreamEnvelope::Session { session_id, event: ev },
                            )
                            .await
                            {
                                break 'pump;
                            }
                            } else {
                                // Backlog still draining: buffer to keep the
                                // client-visible order strict (replay first).
                                sub.pending.push_back(ev);
                            }
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // This connection's single hub receiver fell behind: resync
                    // every active subscription to the latest buffered seq and
                    // drop any stale backlog (the client realigns via
                    // `sync_lost` and resubscribes).
                    for (session_id, sub) in subs.iter_mut() {
                        let buffer = state.session_buffer(session_id);
                        let mut watermark = sub.delivered;
                        let ev = plan_lagged_resync(session_id, &mut watermark, &buffer);
                        sub.delivered = watermark;
                        sub.snapshot_tail = watermark;
                        sub.pending.clear();
                        if !forward(
                            &outbound_tx,
                            DownstreamEnvelope::Session {
                                session_id: session_id.clone(),
                                event: ev,
                            },
                        )
                        .await
                        {
                            break 'pump;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break 'pump,
            },
            res = trace_rx.recv() => match res {
                Ok(ev) => {
                    if !forward(&outbound_tx, DownstreamEnvelope::Trace { event: Box::new(ev) })
                        .await
                    {
                        break 'pump;
                    }
                }
                // drop-oldest hub: skip silently; clients replay via REST.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break 'pump,
            },
            res = global_rx.recv() => match res {
                Ok(ev) => {
                    if !forward(&outbound_tx, DownstreamEnvelope::Global { event: ev }).await {
                        break 'pump;
                    }
                }
                // low-frequency; clients realign via the GET endpoints.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break 'pump,
            },
            ctrl = ctrl_rx.recv() => match ctrl {
                Some(ClientMessage::Subscribe { session_id, after }) => {
                    // New subscriptions are capped; a repeat subscribe refreshes
                    // the existing entry (and its seam) instead of failing.
                    if !subs.contains_key(&session_id) && subs.len() >= MAX_SUBSCRIPTIONS {
                        if !forward(
                            &outbound_tx,
                            DownstreamEnvelope::Error {
                                message: format!(
                                    "subscription limit reached ({MAX_SUBSCRIPTIONS} sessions)"
                                ),
                            },
                        )
                        .await
                        {
                            break 'pump;
                        }
                        continue 'pump;
                    }

                    // Unknown session: ack with latest_seq 0 and do not subscribe
                    // (design §3.2 — client recovers via GET /sessions/:id).
                    if state.resolve_session(&session_id).await.is_none() {
                        if !forward(
                            &outbound_tx,
                            DownstreamEnvelope::Subscribed {
                                session_id,
                                latest_seq: 0,
                            },
                        )
                        .await
                        {
                            break 'pump;
                        }
                        continue 'pump;
                    }

                    let buffer = state.session_buffer(&session_id);
                    let decision = {
                        let buf = buffer.read().expect("session buffer lock poisoned");
                        plan_catch_up(after, &buf)
                    };

                    let sub_state = match decision {
                        CatchUp::Replay(evs) => {
                            // MAJOR-1: the snapshot is NOT forwarded inline;
                            // it becomes the backlog drained in batches by
                            // `drain_pending` while the hubs keep flowing.
                            let snapshot_tail = evs.last().map(|e| e.seq).unwrap_or(0);
                            SubState {
                                delivered: 0,
                                snapshot_tail,
                                pending: evs.into(),
                            }
                        }
                        CatchUp::SyncLost { latest_seq } => {
                            let ev = sync_lost_event(&session_id, "evicted", latest_seq);
                            if !forward(
                                &outbound_tx,
                                DownstreamEnvelope::Session {
                                    session_id: session_id.clone(),
                                    event: ev,
                                },
                            )
                            .await
                            {
                                break 'pump;
                            }
                            SubState {
                                delivered: latest_seq,
                                snapshot_tail: latest_seq,
                                pending: VecDeque::new(),
                            }
                        }
                        CatchUp::LiveOnly | CatchUp::UpToDate => SubState {
                            delivered: after.unwrap_or(0),
                            snapshot_tail: 0,
                            pending: VecDeque::new(),
                        },
                    };

                    let latest_seq = buffer
                        .read()
                        .expect("session buffer lock poisoned")
                        .latest_seq()
                        .unwrap_or(0);
                    if !forward(
                        &outbound_tx,
                        DownstreamEnvelope::Subscribed {
                            session_id: session_id.clone(),
                            latest_seq,
                        },
                    )
                    .await
                    {
                        break 'pump;
                    }

                    // A repeat subscribe replaces the whole entry (fresh
                    // snapshot and watermarks).
                    subs.insert(session_id, sub_state);
                }
                Some(ClientMessage::Unsubscribe { session_id }) => {
                    subs.remove(&session_id);
                }
                None => break 'pump,
            },
            _ = tick.tick() => {
                // Token-rotation guard: close stale credentials with 4001 so
                // the client refreshes its token and reconnects (design §3.1).
                if state.current_api_token() != *auth_token {
                    let _ = outbound_tx
                        .send(Message::Close(Some(CloseFrame {
                            code: 4001,
                            reason: "token rotated".into(),
                        })))
                        .await;
                    break 'pump;
                }
                if !forward(&outbound_tx, DownstreamEnvelope::Heartbeat).await {
                    break 'pump;
                }
            }
            // Backlog ready: a zero sleep is immediately ready, so once the
            // higher-priority hub branches are idle the loop spins back to
            // `drain_pending` instead of parking on the hubs.
            _ = tokio::time::sleep(Duration::ZERO), if has_pending => {}
        }
    }
}

/// Forward at most `batch` pending events per subscription. Returns `false`
/// when the write half is gone (client disconnected) so the pump can stop.
async fn drain_pending(
    subs: &mut HashMap<String, SubState>,
    outbound_tx: &mpsc::Sender<Message>,
    batch: usize,
) -> bool {
    for (session_id, sub) in subs.iter_mut() {
        let mut sent = 0;
        while sent < batch {
            let Some(ev) = sub.pending.front() else { break };
            let env = DownstreamEnvelope::Session {
                session_id: session_id.clone(),
                event: ev.clone(),
            };
            if !forward(outbound_tx, env).await {
                return false;
            }
            sub.delivered = ev.seq;
            sub.pending.pop_front();
            sent += 1;
        }
    }
    true
}

/// Serialize an envelope and push it onto the outbound channel. Returns `false`
/// when the write half is gone (client disconnected) so the pump can stop.
async fn forward(outbound_tx: &mpsc::Sender<Message>, env: DownstreamEnvelope) -> bool {
    let json = serde_json::to_string(&env).unwrap_or_default();
    outbound_tx.send(Message::Text(json)).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::global_events::{GlobalEvent, GlobalEventKind};
    use crate::daemon::run_loop::{SessionEvent, SessionEventKind};
    use crate::daemon::state::DaemonState;
    use crate::teams::trace_sink::{TraceEvent, TraceEventKind};
    use axum::extract::ws::Message;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{broadcast, mpsc};

    fn ev(seq: u64, session_id: &str) -> SessionEvent {
        SessionEvent {
            seq,
            session_id: session_id.to_string(),
            run_id: format!("run-{seq}"),
            kind: SessionEventKind::ContentDelta,
            data: serde_json::json!({ "delta": seq }),
        }
    }

    fn session_event() -> SessionEvent {
        ev(1, "s1")
    }

    fn trace_event() -> TraceEvent {
        TraceEvent {
            ts: 1,
            session_id: "s1".to_string(),
            node_id: "n1".to_string(),
            parent_id: None,
            label: "step".to_string(),
            status: "running".to_string(),
            round: None,
            current_tool: None,
            current_params: None,
            elapsed_ms: 0,
            progress_delta: None,
            token_budget_k: None,
            cumulative_tokens: 0,
            error: None,
            result: None,
            kind: TraceEventKind::Progress,
            permission: None,
            question: None,
        }
    }

    fn global_event() -> GlobalEvent {
        GlobalEvent {
            seq: 7,
            kind: GlobalEventKind::ModeChanged,
            data: serde_json::json!({}),
        }
    }

    // ── Test state + pump harness ────────────────────────────────────────────

    async fn test_state() -> Arc<DaemonState> {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.keep();
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = root.clone();
        let mut state = DaemonState::new(crate::state::AppState::new(settings)).await;
        // Isolate from the developer's real projects.json (mirrors run_loop tests).
        state.projects = crate::daemon::projects::ProjectRegistry::load(
            root.clone(),
            root.join("projects.json"),
        );
        Arc::new(state)
    }

    async fn save_session(state: &DaemonState, id: &str) {
        state
            .session_manager
            .save(&crate::context::memory_session::Session::with_id(
                id.to_string(),
                None,
            ))
            .await
            .expect("save test session");
    }

    /// Dual-write like production publishers: buffer (replay) + hub (live).
    fn publish(state: &DaemonState, ev: SessionEvent) {
        state
            .session_buffer(&ev.session_id)
            .write()
            .expect("session buffer lock poisoned")
            .push(ev.clone());
        let _ = state.session_event_hub.send(ev);
    }

    struct Pump {
        session_tx: broadcast::Sender<SessionEvent>,
        trace_tx: broadcast::Sender<TraceEvent>,
        global_tx: broadcast::Sender<GlobalEvent>,
        ctrl_tx: mpsc::Sender<ClientMessage>,
        outbound_rx: mpsc::Receiver<Message>,
        pump: tokio::task::JoinHandle<()>,
    }

    async fn spawn_pump(state: Arc<DaemonState>) -> Pump {
        spawn_pump_with_batch(state, REPLAY_BATCH).await
    }

    /// `replay_batch` < production lets tests force many drain/yield cycles
    /// and assert the interleaving invariants on short sequences.
    async fn spawn_pump_with_batch(state: Arc<DaemonState>, replay_batch: usize) -> Pump {
        // Mirror production: the pump's session receiver taps the real
        // per-daemon hub; `session_tx` is a hub-sender clone so live events in
        // tests flow through the same channel `publish` uses.
        let session_tx = state.session_event_hub.clone();
        let session_rx = state.session_event_hub.subscribe();
        let (trace_tx, trace_rx) = broadcast::channel(64);
        let (global_tx, global_rx) = broadcast::channel(64);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(64);
        let (outbound_tx, outbound_rx) = mpsc::channel(64);
        let pump = tokio::spawn(event_pump(
            state,
            session_rx,
            trace_rx,
            global_rx,
            ctrl_rx,
            outbound_tx,
            PumpConfig {
                heartbeat: Duration::from_secs(3600),
                replay_batch,
                auth_token: Arc::from(""),
            },
        ));
        Pump {
            session_tx,
            trace_tx,
            global_tx,
            ctrl_tx,
            outbound_rx,
            pump,
        }
    }

    async fn shutdown(p: Pump) {
        drop(p.session_tx);
        drop(p.trace_tx);
        drop(p.global_tx);
        drop(p.ctrl_tx);
        p.pump.await.expect("pump exits cleanly");
    }

    async fn recv_envelope(rx: &mut mpsc::Receiver<Message>) -> DownstreamEnvelope {
        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("envelope within timeout")
            .expect("outbound channel still open");
        match msg {
            Message::Text(text) => serde_json::from_str(&text).expect("valid envelope JSON"),
            other => panic!("expected text frame, got {other:?}"),
        }
    }

    async fn recv_raw(rx: &mut mpsc::Receiver<Message>) -> Message {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("frame within timeout")
            .expect("outbound channel still open")
    }

    fn assert_session(env: DownstreamEnvelope, session_id: &str, seq: u64) {
        match env {
            DownstreamEnvelope::Session {
                session_id: sid,
                event,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(event.seq, seq);
            }
            other => panic!("expected session {session_id} seq {seq}, got {other:?}"),
        }
    }

    fn assert_subscribed(env: DownstreamEnvelope, session_id: &str, latest_seq: u64) {
        match env {
            DownstreamEnvelope::Subscribed {
                session_id: sid,
                latest_seq: latest,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(latest, latest_seq);
            }
            other => panic!("expected subscribed {session_id} latest {latest_seq}, got {other:?}"),
        }
    }

    fn assert_error(env: DownstreamEnvelope) -> String {
        match env {
            DownstreamEnvelope::Error { message } => message,
            other => panic!("expected error envelope, got {other:?}"),
        }
    }

    /// D2 信封 §3.2：`session` 下行信封的字段名稳定、可往返。
    #[test]
    fn session_envelope_roundtrips_with_stable_field_names() {
        let env = DownstreamEnvelope::Session {
            session_id: "s1".to_string(),
            event: session_event(),
        };
        let json = serde_json::to_string(&env).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "session");
        assert_eq!(v["session_id"], "s1");
        assert!(v["event"].is_object());
        assert_eq!(v["event"]["seq"], 1);

        let back: DownstreamEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    /// D2 信封 §3.2：五种下行 type tag 与关键字段名对齐设计表。
    #[test]
    fn downstream_envelope_tags_match_design() {
        assert_eq!(
            serde_json::to_string(&DownstreamEnvelope::Heartbeat).unwrap(),
            r#"{"type":"heartbeat"}"#
        );

        let v: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&DownstreamEnvelope::Trace {
                event: Box::new(trace_event()),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(v["type"], "trace");
        assert!(v["event"].is_object());

        let v: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&DownstreamEnvelope::Global {
                event: global_event(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(v["type"], "global");
        assert!(v["event"].is_object());

        let v: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&DownstreamEnvelope::Subscribed {
                session_id: "s2".to_string(),
                latest_seq: 42,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(v["type"], "subscribed");
        assert_eq!(v["session_id"], "s2");
        assert_eq!(v["latest_seq"], 42);
    }

    /// D2 信封 §3.2：上行 `subscribe`（含可选 `after`）与 `unsubscribe` 解析。
    #[test]
    fn upstream_subscribe_and_unsubscribe_parse() {
        let m: ClientMessage =
            serde_json::from_str(r#"{"op":"subscribe","session_id":"s1","after":3}"#).unwrap();
        match m {
            ClientMessage::Subscribe { session_id, after } => {
                assert_eq!(session_id, "s1");
                assert_eq!(after, Some(3));
            }
            other => panic!("expected subscribe, got {other:?}"),
        }

        let m: ClientMessage =
            serde_json::from_str(r#"{"op":"subscribe","session_id":"s1"}"#).unwrap();
        match m {
            ClientMessage::Subscribe { session_id, after } => {
                assert_eq!(session_id, "s1");
                assert_eq!(after, None);
            }
            other => panic!("expected subscribe, got {other:?}"),
        }

        let m: ClientMessage =
            serde_json::from_str(r#"{"op":"unsubscribe","session_id":"s1"}"#).unwrap();
        match m {
            ClientMessage::Unsubscribe { session_id } => assert_eq!(session_id, "s1"),
            other => panic!("expected unsubscribe, got {other:?}"),
        }
    }

    /// 未识别 type / op 必须优雅失败（返回 `Err` 而非 panic），由连接循环忽略。
    #[test]
    fn unknown_message_types_are_graceful_errors() {
        assert!(serde_json::from_str::<DownstreamEnvelope>(r#"{"type":"bogus"}"#).is_err());
        assert!(serde_json::from_str::<ClientMessage>(r#"{"op":"bogus"}"#).is_err());
    }

    /// 连接循环（select 五路）的构造 + 消息出队顺序：不起真 WS 连接，用
    /// mpsc/broadcast 驱动。trace → global 依发送顺序出队；subscribe 应答
    /// `subscribed`；未订阅 session 事件不产生出站消息。
    #[tokio::test]
    async fn event_pump_forwards_trace_then_global_in_order() {
        let state = test_state().await;
        let mut p = spawn_pump(state).await;

        let trace = trace_event();
        p.trace_tx.send(trace.clone()).unwrap();
        assert_eq!(
            recv_raw(&mut p.outbound_rx).await,
            Message::Text(
                serde_json::to_string(&DownstreamEnvelope::Trace {
                    event: Box::new(trace)
                })
                .unwrap()
            )
        );

        let global = global_event();
        p.global_tx.send(global.clone()).unwrap();
        assert_eq!(
            recv_raw(&mut p.outbound_rx).await,
            Message::Text(
                serde_json::to_string(&DownstreamEnvelope::Global { event: global }).unwrap()
            )
        );

        // subscribe 现在产生 `subscribed` 应答（未知 session → latest_seq: 0）。
        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s1".to_string(),
                after: None,
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "s1", 0);

        // 未订阅的 session 事件被丢弃，不产生出站消息。
        p.session_tx.send(session_event()).unwrap();

        shutdown(p).await;
    }

    /// 订阅后先 replay 缓冲（`after` 游标续传），再 `subscribed` 应答；seam
    /// 处重复 seq 被去重，seam 之后的事件不漏发。
    #[tokio::test]
    async fn subscribe_replays_buffer_then_acks_and_dedups_seam() {
        let state = test_state().await;
        save_session(&state, "s1").await;
        for seq in 1..=3 {
            publish(&state, ev(seq, "s1"));
        }

        let mut p = spawn_pump(state).await;

        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s1".to_string(),
                after: Some(0),
            })
            .await
            .unwrap();

        // 应答先行（订阅接受 + 游标锚点），replay 随后分批排空。
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "s1", 3);
        for seq in 1..=3 {
            assert_session(recv_envelope(&mut p.outbound_rx).await, "s1", seq);
        }

        // seam 去重：重放 seq 3 被丢弃，seq 4 正常转发。
        p.session_tx.send(ev(3, "s1")).unwrap();
        p.session_tx.send(ev(4, "s1")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s1", 4);

        shutdown(p).await;
    }

    /// 未订阅 session 的事件不转发；订阅后按 session_id 过滤并按 seq 去重。
    #[tokio::test]
    async fn unsubscribed_session_not_forwarded_then_subscribed_forwards_with_dedup() {
        let state = test_state().await;
        save_session(&state, "s2").await;
        let mut p = spawn_pump(state).await;

        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s2".to_string(),
                after: None,
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "s2", 0);

        // s1 未订阅（先发），s2 已订阅（后发）：只有 s2 的 seq 1 出站。
        p.session_tx.send(ev(1, "s1")).unwrap();
        p.session_tx.send(ev(1, "s2")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s2", 1);

        // 重复 seq 1 被去重，seq 2 正常转发。
        p.session_tx.send(ev(1, "s2")).unwrap();
        p.session_tx.send(ev(2, "s2")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s2", 2);

        shutdown(p).await;
    }

    /// unsubscribe 后不再收到该 session 的事件（用同 ctrl 通道的后续 subscribe
    /// 应答作为屏障，保证 unsubscribe 已被处理，避免时序抖动）。
    #[tokio::test]
    async fn unsubscribe_stops_forwarding() {
        let state = test_state().await;
        save_session(&state, "s3").await;
        save_session(&state, "s4").await;
        let mut p = spawn_pump(state).await;

        for sid in ["s3", "s4"] {
            p.ctrl_tx
                .send(ClientMessage::Subscribe {
                    session_id: sid.to_string(),
                    after: None,
                })
                .await
                .unwrap();
            assert_subscribed(recv_envelope(&mut p.outbound_rx).await, sid, 0);
        }

        p.session_tx.send(ev(1, "s3")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s3", 1);

        // 退订 s3；随后的 s4 重复 subscribe 应答是 ctrl 通道 FIFO 屏障。
        p.ctrl_tx
            .send(ClientMessage::Unsubscribe {
                session_id: "s3".to_string(),
            })
            .await
            .unwrap();
        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s4".to_string(),
                after: None,
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "s4", 0);

        // s3 已退订（先发）→ 丢弃；s4 仍订阅（后发）→ 转发。
        p.session_tx.send(ev(2, "s3")).unwrap();
        p.session_tx.send(ev(1, "s4")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s4", 1);

        shutdown(p).await;
    }

    /// 重复 subscribe 以新游标重新规划：replay 缓冲并刷新 seam。
    #[tokio::test]
    async fn repeat_subscribe_replans_and_refreshes_seam() {
        let state = test_state().await;
        save_session(&state, "s5").await;
        publish(&state, ev(1, "s5"));
        publish(&state, ev(2, "s5"));

        let mut p = spawn_pump(state.clone()).await;

        // live-only 订阅：无 replay，seam 从 0 开始。
        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s5".to_string(),
                after: None,
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "s5", 2);

        publish(&state, ev(3, "s5"));
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s5", 3);

        // 重复订阅（after=0）：应答先行，replay 1..=3 分批排空，水位刷新至 3。
        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s5".to_string(),
                after: Some(0),
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "s5", 3);
        for seq in 1..=3 {
            assert_session(recv_envelope(&mut p.outbound_rx).await, "s5", seq);
        }

        // 刷新后的 seam：seq 3 去重，seq 4 转发。
        p.session_tx.send(ev(3, "s5")).unwrap();
        p.session_tx.send(ev(4, "s5")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s5", 4);

        shutdown(p).await;
    }

    /// 订阅上限 64：第 65 个订阅回错误信封，连接保持、既有订阅不受影响。
    #[tokio::test]
    async fn subscription_limit_64_returns_error_and_keeps_connection() {
        let state = test_state().await;
        let mut p = spawn_pump(state.clone()).await;

        for i in 0..64 {
            let sid = format!("s{i}");
            save_session(&state, &sid).await;
            p.ctrl_tx
                .send(ClientMessage::Subscribe {
                    session_id: sid.clone(),
                    after: None,
                })
                .await
                .unwrap();
            assert_subscribed(recv_envelope(&mut p.outbound_rx).await, &sid, 0);
        }

        save_session(&state, "s64").await;
        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s64".to_string(),
                after: None,
            })
            .await
            .unwrap();
        let message = assert_error(recv_envelope(&mut p.outbound_rx).await);
        assert!(
            message.contains("64"),
            "error mentions the limit: {message}"
        );

        // 连接仍在：既有订阅继续转发 live 事件。
        p.session_tx.send(ev(1, "s0")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "s0", 1);

        shutdown(p).await;
    }

    /// 未知 session：`subscribed{latest_seq: 0}` 应答、无 sync_lost、不入订阅表。
    #[tokio::test]
    async fn unknown_session_acks_with_latest_zero() {
        let state = test_state().await;
        save_session(&state, "sk").await;
        let mut p = spawn_pump(state).await;

        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "sk".to_string(),
                after: None,
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "sk", 0);

        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "ghost".to_string(),
                after: Some(7),
            })
            .await
            .unwrap();
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "ghost", 0);

        // ghost 未入表：先发的 ghost 事件被丢弃，只有 sk 的事件出站。
        p.session_tx.send(ev(1, "ghost")).unwrap();
        p.session_tx.send(ev(1, "sk")).unwrap();
        assert_session(recv_envelope(&mut p.outbound_rx).await, "sk", 1);

        shutdown(p).await;
    }

    /// MAJOR-1 回归：大批 replay 与 live 突发并发时 —— 订阅应答先行，
    /// replay 分批让出泵（hub 分支持续被轮询），客户端最终收到严格递增、
    /// 无丢失、无重复的完整序列（replay 先于其后到达的 live）。
    #[tokio::test]
    async fn large_replay_during_live_burst_stays_ordered_and_lossless() {
        let state = test_state().await;
        save_session(&state, "sburst").await;
        // 预置 200 个事件（仅 buffer，泵尚未启动，hub 广播无人收到）。
        for seq in 1..=200u64 {
            publish(&state, ev(seq, "sburst"));
        }

        // 批次=2：强制 replay 走多轮让出，放大与 live 分支的交错窗口。
        let mut p = spawn_pump_with_batch(state.clone(), 2).await;

        p.ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "sburst".to_string(),
                after: Some(0),
            })
            .await
            .unwrap();
        // 应答先行（订阅已接受 + 游标锚点），replay 随后异步排空。
        assert_subscribed(recv_envelope(&mut p.outbound_rx).await, "sburst", 200);

        // replay 排空前注入 live 突发（仅 hub，绕过 buffer）。
        for seq in 201..=260u64 {
            p.session_tx.send(ev(seq, "sburst")).unwrap();
        }

        // 收集 session 信封直到见到 seq 260。
        let mut seen: Vec<u64> = Vec::new();
        loop {
            let env = recv_envelope(&mut p.outbound_rx).await;
            if let DownstreamEnvelope::Session { event, .. } = env {
                seen.push(event.seq);
                if event.seq == 260 {
                    break;
                }
            }
        }
        let expected: Vec<u64> = (1..=260).collect();
        assert_eq!(
            seen, expected,
            "replay + live 交错后应严格递增、无丢失、无重复"
        );

        shutdown(p).await;
    }

    // ── Task 2.3: handshake auth (design §3.1) ──────────────────────────────

    /// 认证矩阵：无凭证 / 错误 query token → 401（与受保护路由同构）；
    /// Authorization header 回退可用；query token 可用；未初始化 token 的
    /// daemon（空期望值）拒绝一切凭证（含空 token，杜绝空串绕过）。
    #[tokio::test]
    async fn ws_handler_auth_matrix() {
        let bearer = |token: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            headers
        };

        // 无凭证 → 拒绝
        assert!(!authorize_ws("tok123", None, &HeaderMap::new()));
        // 错误 query token → 拒绝
        assert!(!authorize_ws("tok123", Some("wrong"), &HeaderMap::new()));
        // 正确 query token → 通过
        assert!(authorize_ws("tok123", Some("tok123"), &HeaderMap::new()));
        // Authorization header 回退 → 通过
        assert!(authorize_ws("tok123", None, &bearer("tok123")));
        // query 优先于 header（header 错、query 对 → 通过）
        assert!(authorize_ws("tok123", Some("tok123"), &bearer("wrong")));
        // 未初始化（期望值为空）：即使凭证匹配也拒绝一切
        assert!(!authorize_ws("", Some(""), &HeaderMap::new()));
        assert!(!authorize_ws("", None, &bearer("")));
    }

    /// 2.4：guard 在连接存续期间计入 active_clients，任意退出路径（含
    /// panic unwind）Drop 解除；注册被拒（daemon 已在关闭中）时 Drop 不
    /// 下溢计数。
    #[tokio::test]
    async fn active_client_guard_counts_connection_lifetime() {
        let state = test_state().await;
        assert_eq!(state.active_clients.client_count(), 0);
        {
            let _guard = ActiveClientGuard::new(&state);
            assert_eq!(state.active_clients.client_count(), 1);
            let _second = ActiveClientGuard::new(&state);
            assert_eq!(state.active_clients.client_count(), 2);
        }
        assert_eq!(
            state.active_clients.client_count(),
            0,
            "两个 guard 全部 Drop 后归零"
        );

        state.active_clients.initiate_shutdown();
        {
            let refused = ActiveClientGuard::new(&state);
            assert!(!refused.registered, "关闭中的注册被拒");
            assert_eq!(state.active_clients.client_count(), 0);
        }
        assert_eq!(
            state.active_clients.client_count(),
            0,
            "被拒注册的 guard Drop 不得下溢"
        );
    }

    /// Wire-level regression (production path): axum `route_layer` wraps
    /// routes registered BEFORE the layer call, so `/api/v1/ws` sits behind
    /// the `require_auth` middleware like every protected route. A browser
    /// handshake carries the token only as `?token=` (no Authorization
    /// header), so the middleware must accept the query-token fallback
    /// (design §3.1) or the handshake 401's before the in-handler auth can
    /// run. Drives the real assembled router over a raw TCP socket; task
    /// 2.5 adds the full tungstenite coverage.
    #[tokio::test]
    async fn ws_query_token_handshake_over_real_wire() {
        use crate::daemon::routes;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let state = test_state().await;
        state.set_api_token("wstok".to_string());
        let (health, protected) = routes::create_routers(state, "wstok".to_string());
        let app = health.merge(protected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service()).await;
        });

        let handshake = |token: &str| {
            let path = if token.is_empty() {
                "/api/v1/ws".to_string()
            } else {
                format!("/api/v1/ws?token={token}")
            };
            async move {
                let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
                let request = format!(
                    "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\n\
                     Upgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                     Sec-WebSocket-Version: 13\r\n\r\n"
                );
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut buf = vec![0u8; 1024];
                let n = stream.read(&mut buf).await.unwrap();
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                head.split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(0)
            }
        };

        // Browser path: query token only → upgrade succeeds.
        assert_eq!(handshake("wstok").await, 101);
        // No credentials → rejected before upgrade.
        assert_eq!(handshake("").await, 401);
        // Wrong token → rejected with the same isomorphic 401.
        assert_eq!(handshake("wrong").await, 401);

        server.abort();
    }
}
