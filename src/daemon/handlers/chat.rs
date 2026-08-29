//! Chat streaming and subagent trace SSE handlers.

use super::*;

pub async fn chat_stream(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ChatStreamRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Clone from the live handle so a `/model` switch takes effect on the
    // very next turn. The shared pooled HTTP clients below keep their
    // keep-alive pool + TLS session cache across requests.
    let settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    let client = ApiClient::with_clients(
        settings,
        state.http_client.clone(),
        state.http_client_stream.clone(),
    );

    // Build messages and tools
    let messages = body.messages;
    let tools: Option<Vec<ToolDefinition>> = if body.plan_mode.unwrap_or(false) {
        None
    } else {
        let defs = state.tool_executor.tool_definitions();
        if defs.is_empty() {
            None
        } else {
            Some(defs)
        }
    };

    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();

    tokio::spawn(async move {
        // Make the API call
        let response = match client.chat_stream(messages, tools).await {
            Ok(r) => r,
            Err(e) => {
                let error_json = serde_json::json!({"error": e.to_string()}).to_string();
                let _ = tx.send(Ok(Event::default().data(error_json)));
                return;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let error_json = serde_json::json!({
                "error": crate::api::format_api_error(status, &body)
            })
            .to_string();
            let _ = tx.send(Ok(Event::default().data(error_json)));
            return;
        }

        // Stream SSE chunks back to the client.
        // Use a buffer to handle chunk boundaries — a TCP chunk may split an SSE line
        // in the middle, and String::lines() would discard the partial fragment.
        let mut stream = response.bytes_stream();
        let mut stream_error: Option<String> = None;
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    // Extract complete lines; keep the trailing partial line in buffer
                    while let Some(idx) = buffer.find('\n') {
                        let line = buffer[..idx].trim().to_string();
                        buffer = buffer[idx + 1..].to_string();
                        if line.is_empty() {
                            continue;
                        }
                        // Upstream already formats SSE as "data: {...}" or "[DONE]";
                        // strip prefix so we don't double-wrap.
                        let payload = line.strip_prefix("data: ").unwrap_or(&line);
                        let _ = tx.send(Ok(Event::default().data(payload)));
                    }
                }
                Err(e) => {
                    // reqwest's `Display` only prints the outer kind ("error
                    // decoding response body") and drops the real cause
                    // (timeout vs. connection reset vs. h2 error) that lives in
                    // `Error::source()`. Walk the chain so it's visible in both
                    // the log and the SSE error payload sent to the client.
                    let chain = format_error_chain(&e);
                    error!(error = ?e, chain = %chain, "stream chunk error");
                    stream_error = Some(format!("Upstream stream interrupted: {}", chain));
                    break;
                }
            }
        }
        // Flush any remaining data in the buffer
        let remainder = buffer.trim().to_string();
        if !remainder.is_empty() && stream_error.is_none() {
            let payload = remainder.strip_prefix("data: ").unwrap_or(&remainder);
            let _ = tx.send(Ok(Event::default().data(payload)));
        }

        // Signal done or error (not normal end — lets the TS side detect incomplete streams)
        if let Some(error_msg) = stream_error {
            let error_json = serde_json::json!({"error": error_msg}).to_string();
            let _ = tx.send(Ok(Event::default().data(error_json)));
        } else {
            let _ = tx.send(Ok(Event::default().data("[DONE]")));
        }
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

// ── Subagent Trace Stream (SSE) ──────────────────────────────────────────────

/// Query parameters for `GET /api/v1/subagents/trace/stream`.
#[derive(Debug, serde::Deserialize)]
pub struct TraceStreamQuery {
    /// Filter to a single session. When omitted, the stream is global (all
    /// sessions, live-only -- no cold-start replay).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Unix epoch milliseconds. Replayed headers and live events at or before
    /// this timestamp are skipped.
    #[serde(default)]
    pub since: Option<i64>,
}

/// `GET /api/v1/subagents/trace/replay` — one-shot JSON (non-SSE) replay of a
/// session's persisted transcript headers. Lets thin clients recover terminal
/// subagent results after a refresh/reconnect WITHOUT holding a second
/// long-lived SSE connection (every permanent same-origin connection counts
/// against the browser's per-origin HTTP/1.1 limit).
pub async fn subagent_trace_replay(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<TraceStreamQuery>,
) -> Json<Vec<crate::teams::trace_sink::TraceEvent>> {
    let events = match (q.session_id.as_deref(), state.transcript_store.clone()) {
        (Some(sid), Some(store)) => replay_session_events(&store, sid, q.since.unwrap_or(0)),
        _ => Vec::new(),
    };
    Json(events)
}

/// `GET /api/v1/subagents/trace/stream` -- SSE stream of live subagent trace
/// events with optional cold-start replay.
///
/// On connect (when `session_id` is given) the endpoint replays persisted
/// transcript headers for that session from the global transcript store, then
/// streams live redacted events from the process-global trace hub. A slow
/// subscriber observes `Lagged` (drop-oldest); file persistence is unaffected.
/// Requires the standard bearer token (`require_auth`). See design D3 / Q5.
pub async fn subagent_trace_stream(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<TraceStreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();

    // Subscribe to the global live hub BEFORE cold-start replay so events
    // emitted during replay are buffered in the receiver (avoids a race that
    // would drop events between replay and subscribe).
    let live = crate::teams::trace_sink::trace_hub_subscribe();

    let session_id = q.session_id;
    let since = q.since.unwrap_or(0);
    let store = state.transcript_store.clone();

    tokio::spawn(async move {
        let mut live = live;

        // 1. Cold-start replay from the global transcript store. Only when a
        //    session is requested: a global (no session_id) subscription has
        //    no single persisted history to replay and starts live.
        if let Some(sid) = session_id.as_deref() {
            if let Some(store) = store.as_ref() {
                for ev in replay_session_events(store, sid, since) {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
            }
        }

        // 2. Live stream from the global hub.
        loop {
            match live.recv().await {
                Ok(ev) => {
                    if !should_emit_live(&ev, session_id.as_deref(), since) {
                        continue;
                    }
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "trace SSE subscriber lagged; oldest events dropped for this subscriber"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

/// Reconstruct persisted transcript headers for `session_id` into trace events
/// for cold-start SSE replay. Headers newer than `since` (by `started_at`, unix
/// ms) are emitted in chronological (ascending) order. Per-step detail
/// (round/tool/params) is not stored at the header level, so each replayed
/// event carries the run's terminal state; live events provide per-step detail
/// going forward. `pub(crate)` for unit testing.
pub(crate) fn replay_session_events(
    store: &crate::transcript::SubagentTranscriptStore,
    session_id: &str,
    since: i64,
) -> Vec<crate::teams::trace_sink::TraceEvent> {
    match store.list_by_session(session_id) {
        Ok(headers) => {
            // list_by_session returns DESC by started_at; emit ASC so the
            // client observes chronological order.
            let mut ordered: Vec<_> = headers
                .into_iter()
                .filter(|h| h.started_at > since)
                .collect();
            ordered.reverse();
            ordered
                .into_iter()
                .map(|h| trace_event_from_header(&h))
                .collect()
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                session_id = session_id,
                "cold-start replay: list_by_session failed"
            );
            Vec::new()
        }
    }
}

/// Whether a live trace event should be emitted to a subscriber filtered by an
/// optional `session_id` and a `since` (unix ms) watermark. `since` is
/// inclusive-skip: events with `ts <= since` are dropped. `pub(crate)` for unit
/// testing.
pub(crate) fn should_emit_live(
    ev: &crate::teams::trace_sink::TraceEvent,
    session_id: Option<&str>,
    since: i64,
) -> bool {
    if let Some(sid) = session_id {
        if ev.session_id != sid {
            return false;
        }
    }
    ev.ts > since
}

/// Reconstruct a `TraceEvent` summary from a persisted transcript header.
///
/// The `status` string is the raw DB value (lowercase, e.g. "completed"); live
/// events use the runtime `SubagentStatus` serde name (PascalCase, e.g.
/// "Completed"). Consumers should treat both case-insensitively. The `error`
/// object carries the persisted message + denormalized `root_cause`.
pub(super) fn trace_event_from_header(
    h: &crate::transcript::SubagentTranscriptHeader,
) -> crate::teams::trace_sink::TraceEvent {
    use crate::teams::trace_sink::TraceEvent;
    let error = h.error_message.as_ref().map(|m| {
        let mut obj = serde_json::Map::new();
        obj.insert("message".to_string(), serde_json::Value::String(m.clone()));
        obj.insert(
            "root_cause".to_string(),
            serde_json::to_value(&h.root_cause).unwrap_or(serde_json::Value::Null),
        );
        serde_json::Value::Object(obj)
    });
    TraceEvent {
        ts: h.started_at,
        session_id: h.session_id.clone(),
        node_id: h.id.clone(),
        parent_id: h.parent_id.clone(),
        label: h.label.clone(),
        status: h.status.clone(),
        round: Some(h.actual_rounds as usize),
        current_tool: None,
        current_params: None,
        elapsed_ms: 0,
        progress_delta: None,
        token_budget_k: None,
        cumulative_tokens: h.total_tokens,
        error,
        // Replay the persisted summary as the terminal result text so
        // reconnecting SSE clients get the finished result too.
        result: h.summary.clone(),
        kind: crate::teams::trace_sink::TraceEventKind::Progress,
        permission: None,
        question: None,
    }
}

// ── Tools ────────────────────────────────────────────────────────────────────
