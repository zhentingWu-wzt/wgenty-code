//! WebSocket push channel (task 2.1).
//!
//! Single-connection task multiplexing three downlink event sources (session
//! hub, global trace hub, global event bus), the upstream control channel, and
//! a 15s heartbeat into one outbound envelope stream (design §4). Every
//! downstream frame is serialized to a D2 envelope (design §3.2) on a dedicated
//! write half so serialization/sending never blocks a `select!` branch.
//!
//! Task 2.1 ships the skeleton only: the per-connection session subscription
//! table is an empty placeholder and subscribe/unsubscribe handling lands in
//! task 2.2; route registration and auth land in task 2.3.

use crate::daemon::global_events::GlobalEvent;
use crate::daemon::run_loop::SessionEvent;
use crate::daemon::state::DaemonState;
use crate::teams::trace_sink::TraceEvent;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
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

/// Per-connection session subscription table. Task 2.2 fills this in with the
/// real `HashMap<SessionId, SubState>` and subscribe/unsubscribe handling;
/// task 2.1 leaves it empty, so the pump forwards no session events yet.
#[derive(Default)]
struct SessionSubscriptions {
    _task_2_2: (),
}

/// `GET /api/v1/ws` handler (design §3.1/§4).
///
/// Task 2.1 leaves it `pub(crate)` and unregistered: route registration and
/// token auth land in tasks 2.3. The 16 MiB `max_message_size` matches design
/// §3.1 (SessionEvent may carry a large diff).
#[allow(dead_code)] // wired into routes + auth in task 2.3
pub(crate) async fn ws_handler(
    State(state): State<Arc<DaemonState>>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.max_message_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| connection_loop(state, socket))
}

/// Per-connection task: split the socket into a writer half (serialized
/// outbound envelopes) and a reader half (parsed control messages), then run
/// the five-way `select!` pump.
#[allow(dead_code)] // only reachable via ws_handler, wired in task 2.3
async fn connection_loop(state: Arc<DaemonState>, socket: WebSocket) {
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
        state.session_event_hub.subscribe(),
        crate::teams::trace_sink::trace_hub_subscribe(),
        state.global_event_hub.subscribe(),
        ctrl_rx,
        outbound_tx,
        Duration::from_secs(15),
    )
    .await;

    writer.abort();
    reader.abort();
}

/// The five-way `select!` event pump, kept free of socket types so it can be
/// unit-tested with plain channels (design §4).
async fn event_pump(
    mut session_rx: broadcast::Receiver<SessionEvent>,
    mut trace_rx: broadcast::Receiver<TraceEvent>,
    mut global_rx: broadcast::Receiver<GlobalEvent>,
    mut ctrl_rx: mpsc::Receiver<ClientMessage>,
    outbound_tx: mpsc::Sender<Message>,
    heartbeat: Duration,
) {
    let subscriptions = SessionSubscriptions::default();
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + heartbeat, heartbeat);

    loop {
        tokio::select! {
            res = session_rx.recv() => match res {
                Ok(ev) => {
                    // Task 2.2: filter by the subscription table and dedup
                    // `seq <= seam_seq`. Task 2.1 keeps the table empty, so
                    // every session event is dropped until subscribe lands.
                    let _ = (&subscriptions, &ev);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Task 2.2: single-connection sync_lost via plan_lagged_resync.
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            res = trace_rx.recv() => match res {
                Ok(ev) => {
                    if !forward(&outbound_tx, DownstreamEnvelope::Trace { event: Box::new(ev) })
                        .await
                    {
                        break;
                    }
                }
                // drop-oldest hub: skip silently; clients replay via REST.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            res = global_rx.recv() => match res {
                Ok(ev) => {
                    if !forward(&outbound_tx, DownstreamEnvelope::Global { event: ev }).await {
                        break;
                    }
                }
                // low-frequency; clients realign via the GET endpoints.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            ctrl = ctrl_rx.recv() => match ctrl {
                Some(ClientMessage::Subscribe { session_id, after }) => {
                    // Task 2.2: subscribe/unsubscribe update the table (replay
                    // runs synchronously). Task 2.1 parses only, no-op.
                    let _ = (session_id, after);
                }
                Some(ClientMessage::Unsubscribe { session_id }) => {
                    let _ = session_id;
                }
                None => break,
            },
            _ = tick.tick() => {
                if !forward(&outbound_tx, DownstreamEnvelope::Heartbeat).await {
                    break;
                }
            }
        }
    }
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
    use crate::teams::trace_sink::{TraceEvent, TraceEventKind};
    use axum::extract::ws::Message;
    use std::time::Duration;
    use tokio::sync::{broadcast, mpsc};

    fn session_event() -> SessionEvent {
        SessionEvent {
            seq: 1,
            session_id: "s1".to_string(),
            run_id: "r1".to_string(),
            kind: SessionEventKind::ContentDelta,
            data: serde_json::json!({ "delta": "hi" }),
        }
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
    /// mpsc/broadcast 驱动。trace → global 依发送顺序出队；session（空订阅表）
    /// 与 ctrl（2.2 才实现）为无操作，不产生出站消息。
    #[tokio::test]
    async fn event_pump_forwards_trace_then_global_in_order() {
        let (session_tx, session_rx) = broadcast::channel(16);
        let (trace_tx, trace_rx) = broadcast::channel(16);
        let (global_tx, global_rx) = broadcast::channel(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(16);
        let (outbound_tx, mut outbound_rx) = mpsc::channel(16);

        let pump = tokio::spawn(event_pump(
            session_rx,
            trace_rx,
            global_rx,
            ctrl_rx,
            outbound_tx,
            Duration::from_secs(3600),
        ));

        let trace = trace_event();
        trace_tx.send(trace.clone()).unwrap();
        assert_eq!(
            outbound_rx.recv().await.unwrap(),
            Message::Text(
                serde_json::to_string(&DownstreamEnvelope::Trace {
                    event: Box::new(trace)
                })
                .unwrap()
            )
        );

        let global = global_event();
        global_tx.send(global.clone()).unwrap();
        assert_eq!(
            outbound_rx.recv().await.unwrap(),
            Message::Text(
                serde_json::to_string(&DownstreamEnvelope::Global { event: global }).unwrap()
            )
        );

        // 上行控制消息（2.2 才实现 subscribe/unsubscribe）与空订阅表下的
        // session 事件都应被安静消费，不崩、不额外出站。
        ctrl_tx
            .send(ClientMessage::Subscribe {
                session_id: "s1".to_string(),
                after: None,
            })
            .await
            .unwrap();
        session_tx.send(session_event()).unwrap();

        drop(session_tx);
        drop(trace_tx);
        drop(global_tx);
        drop(ctrl_tx);
        pump.await.unwrap();
    }
}
