//! Server-side agent loop bridge.
//!
//! Maps daemon `SessionEvent`s (from `GET /sessions/:id/events` SSE) into TUI
//! `AppEvent`s, so the TUI can observe a daemon-owned agent run the same way
//! the Web client does. Foundation for migrating the TUI from a client-side
//! loop to a server-side loop.

use crate::agent::progress::{SubagentProgress, SubagentStatus};
use crate::daemon::run_loop::{SessionEvent, SessionEventKind};
use crate::teams::trace_sink::{TraceEvent, TraceEventKind};
use crate::tui::app::types::AppEvent;
use crate::tui::client::{DaemonClient, GlobalEventWire};
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// A daemon turn error or rejected run request is terminal: surface the
/// error, then release the running-turn gate. Transport disconnects recover
/// in `spawn_session_event_reader` and do not use this path.
fn stream_termination_events(message: String) -> Vec<AppEvent> {
    vec![
        AppEvent::StreamError(message),
        AppEvent::ServerTurnTerminated,
    ]
}

pub(super) fn send_stream_termination(
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    message: impl Into<String>,
) {
    for event in stream_termination_events(message.into()) {
        if event_tx.send(event).is_err() {
            return;
        }
    }
}

/// Map a daemon `SessionEvent` into zero or more TUI `AppEvent`s.
///
/// The TUI's `AppEvent` enum already mirrors the daemon's `SessionEventKind`
/// (ContentDelta/ToolStart/ToolResult/...), so this is a 1:1 shape translation
/// plus `data` JSON payload extraction.
pub(crate) fn session_event_to_app_events(ev: SessionEvent) -> Vec<AppEvent> {
    match ev.kind {
        SessionEventKind::ContentDelta => ev
            .data
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| vec![AppEvent::ContentDelta(t.to_string())])
            .unwrap_or_default(),
        SessionEventKind::ReasoningDelta => ev
            .data
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| vec![AppEvent::ReasoningDelta(t.to_string())])
            .unwrap_or_default(),
        SessionEventKind::ToolStart => {
            let name = ev
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = ev
                .data
                .get("args")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            vec![AppEvent::ToolStart { name, args }]
        }
        SessionEventKind::ToolResult => {
            let name = ev
                .data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = ev
                .data
                .get("args")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let content = ev
                .data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            vec![AppEvent::ToolResult {
                name,
                args,
                content,
            }]
        }
        SessionEventKind::TurnDone => {
            let finish_reason = ev
                .data
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("stop")
                .to_string();
            // A tool_calls finish_reason ends an LLM round, not the turn:
            // commit the round's streamed text (StreamDone) so it lands as its
            // own assistant bubble, but don't signal TurnComplete - tools and
            // further rounds still follow. Only a terminal reason completes.
            if finish_reason == "tool_calls" {
                vec![AppEvent::StreamDone { finish_reason }]
            } else {
                vec![
                    AppEvent::StreamDone { finish_reason },
                    AppEvent::TurnComplete,
                ]
            }
        }
        SessionEventKind::TurnError => {
            let msg = ev
                .data
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            stream_termination_events(msg)
        }
        SessionEventKind::PermissionRequired => {
            let request_id = ev
                .data
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool = ev
                .data
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let reason = ev
                .data
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let rule = ev
                .data
                .get("rule")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            vec![AppEvent::ServerPermissionRequired {
                request_id,
                tool,
                reason,
                rule,
            }]
        }
        SessionEventKind::AskUser => {
            let request_id = ev
                .data
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let question = ev
                .data
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let options = ev
                .data
                .get("options")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            let multi_select = ev
                .data
                .get("multi_select")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            vec![AppEvent::ServerQuestionAsked {
                request_id,
                question,
                options,
                multi_select,
            }]
        }
        SessionEventKind::Save => Vec::new(),
        // Sequence recovery is a rendering concern. The daemon keeps run and
        // continuation ownership; the TUI only reports that older UI events
        // were unavailable and resumes from the advertised cursor.
        SessionEventKind::SyncLost => vec![AppEvent::SystemNotice(format!(
            "Session event history before sequence {} is unavailable; resumed from the latest daemon state.",
            ev.data
                .get("latest_seq")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        ))],
        // TurnContext is consumed by web/Tauri inspector panels; the TUI has
        // its own in-process TurnContext via AppEvent::TurnContextCaptured.
        SessionEventKind::TurnContext => Vec::new(),
    }
}

/// Spawn a background task that subscribes to the daemon's session-event SSE
/// stream and forwards mapped `AppEvent`s into `event_tx`. Returns the task's
/// `JoinHandle` so callers can abort/respawn it when the session switches.
///
/// The connect is retried silently with backoff until it succeeds (or
/// `shutdown` is set). This matters because the TUI generates its session id
/// locally and the daemon only learns about a session on its first PUT save —
/// an early connect legitimately 404s, and treating that as a fatal "lost
/// connection" spammed a spurious error on every TUI launch. Once connected,
/// `ready` (optional) fires so callers can start a run only after the
/// subscription is live.
///
/// After an established connection drops, the reader reconnects with
/// `after=<last_seq>` so the daemon buffer replays anything emitted across
/// the gap. It does not release the active-run gate while recovery is in
/// progress. Exits when the event channel is dropped or `shutdown` is set.
pub(crate) fn spawn_session_event_reader(
    client: DaemonClient,
    session_id: String,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    shutdown: Arc<AtomicBool>,
    mut ready: Option<oneshot::Sender<()>>,
    last_seq: Arc<std::sync::atomic::AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let initial_backoff = std::time::Duration::from_millis(250);
        let max_backoff = std::time::Duration::from_secs(2);
        let mut backoff = initial_backoff;
        let mut cursor = last_seq.load(Ordering::Acquire);
        let mut connected_once = cursor > 0;
        loop {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            let after = connected_once.then_some(cursor);
            let resp = match client.session_events_after(&session_id, after).await {
                Ok(response) => response,
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        session_id,
                        after,
                        "session events SSE connect failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                    continue;
                }
            };
            connected_once = true;
            backoff = initial_backoff;
            if let Some(tx) = ready.take() {
                let _ = tx.send(());
            }
            let lines = crate::tui::client::sse_data_lines(resp);
            tokio::pin!(lines);
            while !shutdown.load(Ordering::SeqCst) {
                match lines.next().await {
                    Some(Ok(json_str)) => match serde_json::from_str::<SessionEvent>(&json_str) {
                        Ok(ev) => {
                            if ev.kind == SessionEventKind::SyncLost {
                                cursor = ev
                                    .data
                                    .get("latest_seq")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0);
                                last_seq.store(cursor, Ordering::Release);
                            }
                            if ev.seq != 0 {
                                if ev.seq <= cursor {
                                    continue;
                                }
                                cursor = ev.seq;
                                last_seq.store(cursor, Ordering::Release);
                            }
                            for app_ev in session_event_to_app_events(ev) {
                                if event_tx.send(app_ev).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::trace!(
                                error = %e,
                                "skip unparseable session event line"
                            );
                        }
                    },
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, last_seq = cursor, "session events SSE read error; reconnecting");
                        let _ = event_tx.send(AppEvent::StreamError(
                            "lost connection to daemon event stream; reconnecting".to_string(),
                        ));
                        break;
                    }
                    None => {
                        tracing::warn!(
                            session_id,
                            last_seq = cursor,
                            "session events SSE stream closed; reconnecting"
                        );
                        let _ = event_tx.send(AppEvent::StreamError(
                            "lost connection to daemon event stream; reconnecting".to_string(),
                        ));
                        break;
                    }
                }
            }
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff * 2, max_backoff);
        }
    })
}

/// Map a daemon `TraceEvent` (subagent trace stream) into zero or more
/// `AppEvent`s:
/// - `progress` -> [`AppEvent::SubagentTraceProgress`] (live subagent-tree
///   update; applied via `upsert` in server-side mode).
/// - `permission_pending` / `question_pending` -> server-side popup.
/// - `permission_resolved` / `question_resolved` -> dismiss the matching
///   popup (multi-device sync: another device resolved it).
fn trace_event_to_app_events(ev: TraceEvent) -> Vec<AppEvent> {
    match ev.kind {
        TraceEventKind::Progress => {
            vec![AppEvent::SubagentTraceProgress(Box::new(
                trace_event_to_progress(&ev),
            ))]
        }
        TraceEventKind::PermissionPending => ev
            .permission
            .map(|p| {
                vec![AppEvent::ServerPermissionRequired {
                    request_id: p.request_id,
                    tool: p.tool,
                    reason: p.policy_reason,
                    rule: p.session_rule,
                }]
            })
            .unwrap_or_default(),
        TraceEventKind::PermissionResolved => ev
            .permission
            .map(|p| {
                vec![AppEvent::ServerPermissionResolved {
                    request_id: p.request_id,
                }]
            })
            .unwrap_or_default(),
        TraceEventKind::QuestionPending => ev
            .question
            .map(|q| {
                vec![AppEvent::ServerQuestionAsked {
                    request_id: q.request_id,
                    question: q.question,
                    options: serde_json::to_value(&q.options).unwrap_or(serde_json::Value::Null),
                    multi_select: q.multi_select,
                }]
            })
            .unwrap_or_default(),
        TraceEventKind::QuestionResolved => ev
            .question
            .map(|q| {
                vec![AppEvent::ServerQuestionResolved {
                    request_id: q.request_id,
                }]
            })
            .unwrap_or_default(),
    }
}

/// Reconstruct a [`SubagentProgress`] from a trace `Progress` event for
/// `SubagentTree::upsert`. `TraceEvent` is the redacted, serialized projection
/// of `SubagentProgress`, so fields not carried on the wire (`action_log`,
/// `text_snapshot`, `messages`, …) default to empty; the tree only displays
/// the fields that survive the round-trip.
fn trace_event_to_progress(ev: &TraceEvent) -> SubagentProgress {
    // `status` round-trips through serde: TraceEvent stores the serialized
    // variant name (e.g. "Running"); parse it back, falling back to Running.
    let status =
        serde_json::from_value::<SubagentStatus>(serde_json::Value::String(ev.status.clone()))
            .unwrap_or(SubagentStatus::Running);
    // current_params: TraceEvent stores redacted JSON or a summary string;
    // SubagentProgress expects a human-readable summary string.
    let current_params = ev.current_params.as_ref().map(|v| match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    });
    let error_details = ev
        .error
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    SubagentProgress {
        node_id: ev.node_id.clone(),
        parent_id: ev.parent_id.clone(),
        label: ev.label.clone(),
        status,
        round: ev.round,
        max_rounds: None,
        current_tool: ev.current_tool.clone(),
        current_params,
        action_log: Vec::new(),
        // Terminal trace events carry the subagent's final result text; the
        // SubagentTree preserves it on upsert for the focus view.
        text_snapshot: ev.result.clone(),
        started_at: ev.ts,
        elapsed_ms: ev.elapsed_ms,
        metadata: None,
        progress_delta: ev.progress_delta,
        token_budget_k: ev.token_budget_k,
        cumulative_tokens: ev.cumulative_tokens,
        error_details,
        events: Vec::new(),
        messages: Vec::new(),
    }
}

/// Spawn a background task that subscribes to the daemon's subagent trace
/// SSE stream and forwards mapped `AppEvent`s (permission/question) into
/// `event_tx`. Returns the task's `JoinHandle` so callers can abort/respawn it
/// when the session switches. Mirrors `spawn_session_event_reader`'s silent
/// connect retry (the trace endpoint never 404s for unknown sessions — it
/// streams an empty replay — so failures here are transient network hiccups).
pub(crate) fn spawn_trace_event_reader(
    client: DaemonClient,
    session_id: String,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    shutdown: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = std::time::Duration::from_millis(250);
        let max_backoff = std::time::Duration::from_secs(2);
        let resp = loop {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            match client.trace_stream(&session_id).await {
                Ok(r) => break r,
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        session_id,
                        "trace stream SSE connect failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                }
            }
        };
        let lines = crate::tui::client::sse_data_lines(resp);
        tokio::pin!(lines);
        while !shutdown.load(Ordering::SeqCst) {
            match lines.next().await {
                Some(Ok(json_str)) => match serde_json::from_str::<TraceEvent>(&json_str) {
                    Ok(ev) => {
                        for app_ev in trace_event_to_app_events(ev) {
                            if event_tx.send(app_ev).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::trace!(
                            error = %e,
                            "skip unparseable trace event line"
                        );
                    }
                },
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "trace stream SSE read error");
                    return;
                }
                None => return,
            }
        }
    })
}

/// Poll interval for the todos fallback while the global event stream is
/// down (same 500ms cadence the panel used before subscription).
const TODOS_FALLBACK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
/// How often to retry `subscribe_events` while running on the polling
/// fallback.
const TODOS_RESUBSCRIBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Map one global daemon event into its TUI events. Background results are
/// session-scoped; todos snapshots remain global so the plan panel retains its
/// existing live-update behavior.
pub(crate) fn global_event_to_app_events(
    event: GlobalEventWire,
    session_id: &str,
) -> Vec<AppEvent> {
    match event.kind.as_str() {
        "background_result" => event
            .data
            .get("result")
            .cloned()
            .and_then(|value| {
                serde_json::from_value::<crate::tools::execution::BackgroundResult>(value).ok()
            })
            .filter(|result| result.session_id.as_deref() == Some(session_id))
            .map(AppEvent::BackgroundTaskCompleted)
            .into_iter()
            .collect(),
        "todos_changed" => event
            .data
            .get("items")
            .cloned()
            .and_then(|value| {
                serde_json::from_value::<Vec<crate::tui::client::TodoItem>>(value).ok()
            })
            .map(AppEvent::TodosSnapshot)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// Convert the daemon's retained inbox snapshot into display-only recovery
/// notifications for the active session. Model delivery remains daemon-owned.
pub(crate) fn background_results_to_app_events(
    results: Vec<serde_json::Value>,
    session_id: &str,
) -> Vec<AppEvent> {
    results
        .into_iter()
        .filter_map(|value| {
            serde_json::from_value::<crate::tools::execution::BackgroundResult>(value).ok()
        })
        .filter(|result| result.session_id.as_deref() == Some(session_id))
        .map(AppEvent::BackgroundTaskRecovered)
        .collect()
}

/// Spawn a background task that drives the plan/todos panel from the daemon's
/// global event stream (`GET /api/v1/events`). Session-independent: spawned
/// once at app startup and lives until `shutdown` is set.
///
/// Modes:
/// - **Subscribed**: `todos_changed` events carry a full snapshot; `data.items`
///   replaces local state directly. Arrival order is not guaranteed to equal
///   `seq` order (see daemon `global_events.rs`), so stale/duplicate snapshots
///   (`seq <= last applied`) are dropped. The stream is live-only, so every
///   (re)subscribe first realigns via one `GET /api/v1/todos`.
/// - **Fallback**: on connect failure or mid-stream disconnect, poll
///   `GET /api/v1/todos` every 500ms and retry the subscription every ~5s.
pub(crate) fn spawn_global_event_reader(
    client: DaemonClient,
    session_id: String,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    shutdown: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    /// Forward a full todos snapshot unless it is identical to the last one
    /// forwarded. Replacing local state with an equal snapshot is a no-op
    /// anyway, and the dedup keeps a quiet (or empty) daemon todos bus from
    /// repeatedly stomping the panel while the `update_plan` tool-arg path
    /// also writes it. Returns false only when the event channel is closed
    /// (app shutting down).
    fn forward_snapshot(
        event_tx: &mpsc::UnboundedSender<AppEvent>,
        last: &mut Option<Vec<crate::tui::client::TodoItem>>,
        items: Vec<crate::tui::client::TodoItem>,
    ) -> bool {
        if last.as_ref() == Some(&items) {
            return true;
        }
        if event_tx
            .send(AppEvent::TodosSnapshot(items.clone()))
            .is_err()
        {
            return false;
        }
        *last = Some(items);
        true
    }

    /// Fetch one full todos snapshot via GET and forward it. Returns false
    /// only when the event channel is closed (app shutting down).
    async fn realign_via_get(
        client: &DaemonClient,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
        last: &mut Option<Vec<crate::tui::client::TodoItem>>,
    ) -> bool {
        match client.get_todos().await {
            Ok(resp) => forward_snapshot(event_tx, last, resp.items),
            Err(e) => {
                tracing::debug!(error = %e, "todos GET failed; retrying next cycle");
                true
            }
        }
    }

    async fn recover_background_results(
        client: &DaemonClient,
        session_id: &str,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) -> bool {
        match client.get_background_results().await {
            Ok(results) => {
                for event in background_results_to_app_events(results, session_id) {
                    if event_tx.send(event).is_err() {
                        return false;
                    }
                }
                true
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    session_id,
                    "background-result snapshot recovery failed"
                );
                true
            }
        }
    }

    tokio::spawn(async move {
        let mut last_snapshot: Option<Vec<crate::tui::client::TodoItem>> = None;
        loop {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            match client.subscribe_events().await {
                Ok(stream) => {
                    // Live-only stream: realign once via GET before trusting
                    // events, so nothing published before the subscribe is missed.
                    if !realign_via_get(&client, &event_tx, &mut last_snapshot).await {
                        return;
                    }
                    if !recover_background_results(&client, &session_id, &event_tx).await {
                        return;
                    }
                    let mut last_seq = 0u64;
                    tokio::pin!(stream);
                    loop {
                        if shutdown.load(Ordering::SeqCst) {
                            return;
                        }
                        match stream.next().await {
                            Some(Ok(ev)) => {
                                // Preserve the todos stream's sequence guard;
                                // result events are independent and must never
                                // advance it.
                                if ev.kind == "todos_changed" {
                                    if ev.seq <= last_seq {
                                        continue;
                                    }
                                    last_seq = ev.seq;
                                }
                                for app_event in global_event_to_app_events(ev, &session_id) {
                                    match app_event {
                                        AppEvent::TodosSnapshot(items) => {
                                            if !forward_snapshot(
                                                &event_tx,
                                                &mut last_snapshot,
                                                items,
                                            ) {
                                                return;
                                            }
                                        }
                                        app_event => {
                                            if event_tx.send(app_event).is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                tracing::warn!(
                                    error = %e,
                                    "global events SSE read error; falling back to polling"
                                );
                                break;
                            }
                            None => {
                                tracing::warn!(
                                    "global events SSE stream closed; falling back to polling"
                                );
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        "global events SSE connect failed; using polling fallback"
                    );
                }
            }
            // Polling fallback until the next resubscribe attempt. The first
            // `interval` tick fires immediately, so a failed connect still
            // populates the panel right away.
            let mut poll = tokio::time::interval(TODOS_FALLBACK_POLL_INTERVAL);
            let resubscribe = tokio::time::sleep(TODOS_RESUBSCRIBE_INTERVAL);
            tokio::pin!(resubscribe);
            loop {
                tokio::select! {
                    _ = poll.tick() => {
                        if shutdown.load(Ordering::SeqCst) {
                            return;
                        }
                        if !realign_via_get(&client, &event_tx, &mut last_snapshot).await {
                            return;
                        }
                    }
                    _ = &mut resubscribe => break,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Query, State};
    use axum::response::sse::{Event, Sse};
    use axum::routing::get;
    use axum::Router;
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    async fn disconnect_then_replay_turn_done(
        State(connections): State<Arc<AtomicUsize>>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
        let connection = connections.fetch_add(1, Ordering::SeqCst);
        let events = if connection == 0 {
            assert_eq!(query.get("after").map(String::as_str), Some("41"));
            Vec::new()
        } else {
            assert_eq!(query.get("after").map(String::as_str), Some("41"));
            vec![ev(
                42,
                SessionEventKind::TurnDone,
                serde_json::json!({"finish_reason": "stop"}),
            )]
        };
        Sse::new(futures::stream::iter(events.into_iter().map(|event| {
            let json = serde_json::to_string(&event).expect("serialize session event");
            Ok(Event::default().data(json))
        })))
    }

    async fn sync_lost_then_resume_from_latest(
        State(connections): State<Arc<AtomicUsize>>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
        let connection = connections.fetch_add(1, Ordering::SeqCst);
        let events = if connection == 0 {
            assert_eq!(query.get("after").map(String::as_str), Some("4"));
            vec![ev(
                0,
                SessionEventKind::SyncLost,
                serde_json::json!({"latest_seq": 77}),
            )]
        } else {
            assert_eq!(query.get("after").map(String::as_str), Some("77"));
            vec![ev(
                78,
                SessionEventKind::TurnDone,
                serde_json::json!({"finish_reason": "stop"}),
            )]
        };
        Sse::new(futures::stream::iter(events.into_iter().map(|event| {
            let json = serde_json::to_string(&event).expect("serialize session event");
            Ok(Event::default().data(json))
        })))
    }

    #[tokio::test]
    async fn session_reader_reconnects_with_replay_after_established_disconnect() {
        let connections = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route(
                "/api/v1/sessions/:id/events",
                get(disconnect_then_replay_turn_done),
            )
            .with_state(connections.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind replay server");
        let address = listener.local_addr().expect("replay server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve replay server");
        });
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let cursor = Arc::new(std::sync::atomic::AtomicU64::new(41));
        let reader = spawn_session_event_reader(
            DaemonClient::new(format!("http://{address}")),
            "session-a".to_string(),
            event_tx,
            shutdown.clone(),
            None,
            cursor.clone(),
        );
        let mut saw_terminal = false;

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match event_rx.recv().await.expect("reader event channel open") {
                    AppEvent::ServerTurnTerminated => saw_terminal = true,
                    AppEvent::TurnComplete => break,
                    _ => {}
                }
            }
        })
        .await
        .expect("replayed TurnDone must reach the app");
        assert!(!saw_terminal, "disconnect recovery must keep the run gate");
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        assert_eq!(cursor.load(Ordering::Acquire), 42);

        shutdown.store(true, Ordering::SeqCst);
        reader.abort();
        server.abort();
    }

    #[tokio::test]
    async fn session_reader_realigns_cursor_after_sync_lost() {
        let connections = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route(
                "/api/v1/sessions/:id/events",
                get(sync_lost_then_resume_from_latest),
            )
            .with_state(connections.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind sync-lost server");
        let address = listener.local_addr().expect("sync-lost server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve sync-lost server");
        });
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let cursor = Arc::new(std::sync::atomic::AtomicU64::new(4));
        let reader = spawn_session_event_reader(
            DaemonClient::new(format!("http://{address}")),
            "session-a".to_string(),
            event_tx,
            shutdown.clone(),
            None,
            cursor.clone(),
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            let mut saw_sync_lost_notice = false;
            loop {
                match event_rx.recv().await.expect("reader event channel open") {
                    AppEvent::SystemNotice(notice) if notice.contains("sequence 77") => {
                        saw_sync_lost_notice = true
                    }
                    AppEvent::TurnComplete => {
                        assert!(saw_sync_lost_notice);
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("reader must reconnect from the sync-lost latest sequence");
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        assert_eq!(cursor.load(Ordering::Acquire), 78);

        shutdown.store(true, Ordering::SeqCst);
        reader.abort();
        server.abort();
    }

    fn background_result_event(session_id: Option<&str>) -> crate::tui::client::GlobalEventWire {
        crate::tui::client::GlobalEventWire {
            seq: 1,
            kind: "background_result".to_string(),
            data: serde_json::json!({
                "result": {
                    "task_id": "bg_a",
                    "session_id": session_id,
                    "result_type": "command",
                    "command": "true",
                    "stdout": "done",
                    "stderr": "",
                    "exit_code": 0,
                    "success": true,
                    "sandbox_bypassed": false,
                    "permission_mode": null,
                    "sandbox_level": null
                }
            }),
        }
    }

    #[test]
    fn retained_background_snapshot_maps_to_display_events_only_for_active_session() {
        let results = vec![
            serde_json::json!({
                "task_id": "bg_a",
                "session_id": "session-a",
                "result_type": "command",
                "command": "true",
                "stdout": "done",
                "stderr": "",
                "exit_code": 0,
                "success": true,
                "sandbox_bypassed": false,
                "permission_mode": null,
                "sandbox_level": null
            }),
            serde_json::json!({
                "task_id": "bg_b",
                "session_id": "session-b",
                "result_type": "command",
                "command": "true",
                "stdout": "foreign",
                "stderr": "",
                "exit_code": 0,
                "success": true,
                "sandbox_bypassed": false,
                "permission_mode": null,
                "sandbox_level": null
            }),
            serde_json::json!({
                "task_id": "bg_legacy",
                "result_type": "command",
                "command": "true",
                "stdout": "legacy",
                "stderr": "",
                "exit_code": 0,
                "success": true,
                "sandbox_bypassed": false,
                "permission_mode": null,
                "sandbox_level": null
            }),
        ];

        let events = background_results_to_app_events(results, "session-a");

        assert!(matches!(
            events.as_slice(),
            [AppEvent::BackgroundTaskRecovered(result)] if result.task_id == "bg_a"
        ));
    }

    fn ev(seq: u64, kind: SessionEventKind, data: serde_json::Value) -> SessionEvent {
        SessionEvent {
            seq,
            session_id: "s".into(),
            run_id: "r".into(),
            kind,
            data,
        }
    }

    #[test]
    fn content_delta_maps() {
        let apps = session_event_to_app_events(ev(
            1,
            SessionEventKind::ContentDelta,
            serde_json::json!({"text": "hello"}),
        ));
        assert_eq!(apps.len(), 1);
        assert!(matches!(&apps[0], AppEvent::ContentDelta(t) if t == "hello"));
    }

    #[test]
    fn reasoning_delta_maps() {
        let apps = session_event_to_app_events(ev(
            2,
            SessionEventKind::ReasoningDelta,
            serde_json::json!({"text": "thinking"}),
        ));
        assert_eq!(apps.len(), 1);
        assert!(matches!(
            &apps[0],
            AppEvent::ReasoningDelta(t) if t == "thinking"
        ));
    }

    #[test]
    fn tool_start_maps() {
        let apps = session_event_to_app_events(ev(
            3,
            SessionEventKind::ToolStart,
            serde_json::json!({"name": "file_read", "args": {"path": "x"}}),
        ));
        assert_eq!(apps.len(), 1);
        assert!(matches!(&apps[0], AppEvent::ToolStart { name, .. } if name == "file_read"));
    }

    #[test]
    fn tool_result_maps() {
        let apps = session_event_to_app_events(ev(
            4,
            SessionEventKind::ToolResult,
            serde_json::json!({"name": "file_read", "args": {}, "content": "ok"}),
        ));
        assert_eq!(apps.len(), 1);
        assert!(matches!(
            &apps[0],
            AppEvent::ToolResult { name, content, .. } if name == "file_read" && content == "ok"
        ));
    }

    #[test]
    fn turn_done_maps_to_stream_done_and_complete() {
        let apps = session_event_to_app_events(ev(
            5,
            SessionEventKind::TurnDone,
            serde_json::json!({"finish_reason": "stop"}),
        ));
        assert_eq!(apps.len(), 2);
        assert!(matches!(
            &apps[0],
            AppEvent::StreamDone { finish_reason } if finish_reason == "stop"
        ));
        assert!(matches!(&apps[1], AppEvent::TurnComplete));
    }

    #[test]
    fn turn_done_tool_calls_is_round_boundary_not_complete() {
        // A tool_calls finish ends an LLM round, not the turn: commit the
        // streamed text via StreamDone but don't signal TurnComplete (tools and
        // further rounds still follow). Regression for web/desktop round-split.
        let apps = session_event_to_app_events(ev(
            9,
            SessionEventKind::TurnDone,
            serde_json::json!({"finish_reason": "tool_calls"}),
        ));
        assert_eq!(apps.len(), 1);
        assert!(matches!(
            &apps[0],
            AppEvent::StreamDone { finish_reason } if finish_reason == "tool_calls"
        ));
    }

    #[test]
    fn turn_error_maps() {
        let apps = session_event_to_app_events(ev(
            6,
            SessionEventKind::TurnError,
            serde_json::json!({"message": "boom"}),
        ));
        assert_eq!(apps.len(), 2);
        assert!(matches!(&apps[0], AppEvent::StreamError(m) if m == "boom"));
        assert!(matches!(&apps[1], AppEvent::ServerTurnTerminated));
    }

    #[test]
    fn run_failure_maps_to_error_and_terminal_lifecycle() {
        let apps = stream_termination_events("run failed".to_string());

        assert_eq!(apps.len(), 2);
        assert!(matches!(&apps[0], AppEvent::StreamError(message) if message == "run failed"));
        assert!(matches!(&apps[1], AppEvent::ServerTurnTerminated));
    }

    #[test]
    fn save_is_noop() {
        let apps =
            session_event_to_app_events(ev(7, SessionEventKind::Save, serde_json::Value::Null));
        assert!(apps.is_empty());
    }

    #[test]
    fn sync_lost_is_reported_without_taking_run_ownership() {
        let apps = session_event_to_app_events(ev(
            0,
            SessionEventKind::SyncLost,
            serde_json::json!({"latest_seq": 77}),
        ));
        assert!(matches!(
            apps.as_slice(),
            [AppEvent::SystemNotice(notice)] if notice.contains("sequence 77")
        ));
    }

    #[test]
    fn missing_text_field_is_dropped() {
        let apps = session_event_to_app_events(ev(
            8,
            SessionEventKind::ContentDelta,
            serde_json::json!({}),
        ));
        assert!(apps.is_empty());
    }

    #[test]
    fn matching_background_result_maps_to_completed_event() {
        let apps =
            global_event_to_app_events(background_result_event(Some("session-a")), "session-a");

        assert!(matches!(
            apps.as_slice(),
            [AppEvent::BackgroundTaskCompleted(result)] if result.task_id == "bg_a"
        ));
    }

    #[test]
    fn foreign_or_legacy_background_result_is_dropped() {
        assert!(global_event_to_app_events(
            background_result_event(Some("session-b")),
            "session-a"
        )
        .is_empty());
        assert!(global_event_to_app_events(background_result_event(None), "session-a").is_empty());
    }

    fn trace_ev(kind: TraceEventKind) -> TraceEvent {
        TraceEvent {
            ts: 1700000000,
            session_id: "s".into(),
            node_id: "n1".into(),
            parent_id: None,
            label: "explore".into(),
            status: "Running".into(),
            round: Some(2),
            current_tool: Some("grep".into()),
            current_params: Some(serde_json::Value::String("src/".into())),
            elapsed_ms: 500,
            progress_delta: Some(0.4),
            token_budget_k: Some(10),
            cumulative_tokens: 1234,
            error: None,
            result: None,
            kind,
            permission: None,
            question: None,
        }
    }

    #[test]
    fn trace_progress_maps_to_subagent_trace_progress() {
        let apps = trace_event_to_app_events(trace_ev(TraceEventKind::Progress));
        assert_eq!(apps.len(), 1);
        match &apps[0] {
            AppEvent::SubagentTraceProgress(p) => {
                assert_eq!(p.node_id, "n1");
                assert_eq!(p.label, "explore");
                assert_eq!(p.status, SubagentStatus::Running);
                assert_eq!(p.round, Some(2));
                assert_eq!(p.current_tool.as_deref(), Some("grep"));
                assert_eq!(p.current_params.as_deref(), Some("src/"));
                assert_eq!(p.cumulative_tokens, 1234);
                assert!(p.action_log.is_empty());
                assert!(p.messages.is_empty());
            }
            other => panic!("expected SubagentTraceProgress, got {other:?}"),
        }
    }

    #[test]
    fn trace_progress_current_params_json_serialized_to_string() {
        let mut ev = trace_ev(TraceEventKind::Progress);
        ev.current_params = Some(serde_json::json!({"path": "a.rs"}));
        let apps = trace_event_to_app_events(ev);
        match &apps[0] {
            AppEvent::SubagentTraceProgress(p) => {
                // Non-string JSON is stringified for the summary field.
                assert!(p.current_params.as_deref().unwrap().contains("a.rs"));
            }
            other => panic!("expected SubagentTraceProgress, got {other:?}"),
        }
    }

    #[test]
    fn trace_progress_unknown_status_falls_back_to_running() {
        let mut ev = trace_ev(TraceEventKind::Progress);
        ev.status = "BogusVariant".into();
        let apps = trace_event_to_app_events(ev);
        match &apps[0] {
            AppEvent::SubagentTraceProgress(p) => {
                assert_eq!(p.status, SubagentStatus::Running);
            }
            other => panic!("expected SubagentTraceProgress, got {other:?}"),
        }
    }
}
