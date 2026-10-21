//! Acceptance: WebSocket push channel (sse-to-websocket task 2.5).
//!
//! Drives the real router (`create_routers`) over the wire with a
//! tokio-tungstenite client: envelope-type completeness on one connection,
//! subscribe-cursor resume across a reconnect (no loss, no duplicates),
//! handshake auth rejection, and SSE/WS clients observing equivalent event
//! streams side by side.

use crate::daemon_harness::{
    create_session, spawn_daemon_custom, SseReader, TestDaemon, TEST_TOKEN,
};
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use wgenty_code::agent::runtime::{EventSink, RuntimeEvent};
use wgenty_code::daemon::run_loop::DaemonEventSink;
use wgenty_code::teams::trace_sink::{trace_hub, TraceEvent, TraceEventKind};

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsRx = futures::stream::SplitStream<WsStream>;
type WsTx = futures::stream::SplitSink<WsStream, Message>;

/// Boot the daemon with the in-handler ws token initialized to the same
/// value the middleware carries (mirrors `daemon::run` startup order).
async fn spawn_ws_daemon() -> TestDaemon {
    spawn_daemon_custom(|_| {}, |state| state.set_api_token(TEST_TOKEN.to_string())).await
}

fn ws_url(d: &TestDaemon, token: &str) -> String {
    format!("{}/ws?token={token}", d.base.replace("http://", "ws://"))
}

async fn ws_connect(d: &TestDaemon, token: &str) -> (WsTx, WsRx) {
    let (stream, _) = tokio_tungstenite::connect_async(ws_url(d, token))
        .await
        .expect("ws connect");
    stream.split()
}

/// The run loop's session-event write path (hub + replay buffer).
fn test_sink(d: &TestDaemon, session_id: &str, run_id: &str) -> DaemonEventSink {
    DaemonEventSink::new(
        session_id.to_string(),
        run_id.to_string(),
        d.state.session_event_hub.clone(),
        d.state.session_seq_counter(session_id),
        d.state.session_buffer(session_id),
    )
}

fn emit_deltas(sink: &DaemonEventSink, range: std::ops::RangeInclusive<u64>) {
    for i in range {
        sink.emit(RuntimeEvent::ContentDelta(format!("delta-{i}")));
    }
}

fn subscribe_msg(session_id: &str, after: Option<u64>) -> Message {
    Message::text(
        serde_json::json!({ "op": "subscribe", "session_id": session_id, "after": after })
            .to_string(),
    )
}

/// Next downstream envelope as JSON (10s deadline; heartbeats are 15s so
/// they never interleave with these assertions).
async fn next_env(rx: &mut WsRx) -> serde_json::Value {
    let msg = tokio::time::timeout(Duration::from_secs(10), rx.next())
        .await
        .expect("envelope within 10s")
        .expect("ws stream open")
        .expect("ws frame ok");
    match msg {
        Message::Text(text) => serde_json::from_str(&text).expect("envelope is JSON"),
        other => panic!("expected text envelope, got {other:?}"),
    }
}

/// Subscribe and wait for the ack. The ack doubles as a readiness barrier:
/// the pump taps all three hubs before its select loop runs, so having
/// processed a control message proves every hub receiver is live.
async fn subscribe(tx: &mut WsTx, rx: &mut WsRx, session_id: &str, after: Option<u64>) {
    tx.send(subscribe_msg(session_id, after))
        .await
        .expect("send subscribe");
    let ack = next_env(rx).await;
    assert_eq!(ack["type"], "subscribed", "expected ack, got {ack}");
    assert_eq!(ack["session_id"], session_id);
}

fn trace_event(session_id: &str) -> TraceEvent {
    TraceEvent {
        ts: 1,
        session_id: session_id.to_string(),
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

/// Envelope-type completeness: subscribed, session, trace, and global all
/// arrive as typed envelopes on ONE connection with the D2 field shapes.
#[tokio::test]
async fn ws_envelope_types_complete_over_one_connection() {
    let d = spawn_ws_daemon().await;
    let sid = create_session(&d, "envelopes").await;
    let sink = test_sink(&d, &sid, "run-1");

    let (mut tx, mut rx) = ws_connect(&d, TEST_TOKEN).await;
    subscribe(&mut tx, &mut rx, &sid, None).await;

    emit_deltas(&sink, 1..=1);
    let session = next_env(&mut rx).await;
    assert_eq!(session["type"], "session");
    assert_eq!(session["session_id"], sid);
    assert_eq!(session["event"]["seq"], 1);
    assert_eq!(session["event"]["kind"], "content_delta");

    let _ = trace_hub().send(trace_event(&sid));
    let trace = next_env(&mut rx).await;
    assert_eq!(trace["type"], "trace");
    assert_eq!(trace["event"]["session_id"], sid);
    assert_eq!(trace["event"]["node_id"], "n1");

    d.state.broadcast_global(
        wgenty_code::daemon::global_events::GlobalEventKind::ModeChanged,
        serde_json::json!({ "mode": "yolo" }),
    );
    let global = next_env(&mut rx).await;
    assert_eq!(global["type"], "global");
    assert!(
        global["event"]["seq"].is_u64(),
        "global envelope carries seq"
    );
    assert_eq!(global["event"]["kind"], "mode_changed");
}

/// Cursor resume across a reconnect: replayed misses only, then live —
/// exactly [4, 5, 6], no duplicates, no gaps (spec: Subscribe with cursor
/// resumes missed events; Reconnect restores subscriptions).
#[tokio::test]
async fn ws_resubscribe_after_reconnect_no_loss_no_dup() {
    let d = spawn_ws_daemon().await;
    let sid = create_session(&d, "resume").await;
    let sink = test_sink(&d, &sid, "run-1");

    // Backlog exists before the first subscriber (buffer write; hub broadcast
    // with no receivers is a no-op).
    emit_deltas(&sink, 1..=3);

    let (mut tx, mut rx) = ws_connect(&d, TEST_TOKEN).await;
    subscribe(&mut tx, &mut rx, &sid, Some(0)).await;
    for seq in 1..=3 {
        let env = next_env(&mut rx).await;
        assert_eq!(env["type"], "session");
        assert_eq!(env["event"]["seq"], seq);
    }

    // Disconnect; 4..=5 land only in the replay buffer.
    drop((tx, rx));
    emit_deltas(&sink, 4..=5);

    // Reconnect with the last seen cursor: exactly the misses, then live.
    let (mut tx, mut rx) = ws_connect(&d, TEST_TOKEN).await;
    subscribe(&mut tx, &mut rx, &sid, Some(3)).await;
    emit_deltas(&sink, 6..=6);
    let mut seqs = Vec::new();
    for _ in 0..3 {
        let env = next_env(&mut rx).await;
        assert_eq!(env["type"], "session");
        seqs.push(env["event"]["seq"].as_u64().expect("seq"));
    }
    assert_eq!(
        seqs,
        vec![4, 5, 6],
        "missed events replayed once, then live"
    );
}

/// Handshake auth: wrong or missing credentials are rejected before the
/// upgrade with the same 401 as every protected route.
#[tokio::test]
async fn ws_handshake_auth_rejected_before_upgrade() {
    let d = spawn_ws_daemon().await;

    for token in ["", "wrong-token"] {
        let url = if token.is_empty() {
            format!("{}/ws", d.base.replace("http://", "ws://"))
        } else {
            ws_url(&d, token)
        };
        let err = tokio_tungstenite::connect_async(url)
            .await
            .expect_err("handshake must fail");
        let status = match err {
            tokio_tungstenite::tungstenite::Error::Http(resp) => resp.status(),
            other => panic!("expected http-level rejection, got {other:?}"),
        };
        assert_eq!(status, 401, "isomorphic rejection for token={token:?}");
    }
}

/// SSE and WS clients coexist and observe equivalent global event streams
/// with identical seq sequences (spec: Legacy SSE client coexists).
#[tokio::test]
async fn sse_and_ws_clients_observe_equivalent_global_streams() {
    let d = spawn_ws_daemon().await;
    let sid = create_session(&d, "coexist").await;

    let mut sse = SseReader::connect(&d.client, &format!("{}/events", d.base)).await;
    let (mut tx, mut rx) = ws_connect(&d, TEST_TOKEN).await;
    // Barrier: ack proves the ws pump's hub receivers are live (the SSE
    // connect already awaited its server-side subscription).
    subscribe(&mut tx, &mut rx, &sid, None).await;

    d.state.broadcast_global(
        wgenty_code::daemon::global_events::GlobalEventKind::ModeChanged,
        serde_json::json!({ "mode": "accept_edits" }),
    );
    d.state.broadcast_global(
        wgenty_code::daemon::global_events::GlobalEventKind::ModelChanged,
        serde_json::json!({ "profile": "p2" }),
    );

    let mut sse_events = Vec::new();
    for _ in 0..2 {
        let ev = sse.next_json().await;
        sse_events.push((ev["seq"].as_u64(), ev["kind"].as_str().unwrap().to_string()));
    }

    let mut ws_events = Vec::new();
    while ws_events.len() < 2 {
        let env = next_env(&mut rx).await;
        if env["type"] == "global" {
            ws_events.push((
                env["event"]["seq"].as_u64(),
                env["event"]["kind"].as_str().unwrap().to_string(),
            ));
        }
    }

    assert_eq!(
        sse_events, ws_events,
        "identical (seq, kind) sequence on both channels"
    );
}
