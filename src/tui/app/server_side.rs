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
use crate::tui::client::DaemonClient;
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

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
            vec![
                AppEvent::StreamDone { finish_reason },
                AppEvent::TurnComplete,
            ]
        }
        SessionEventKind::TurnError => {
            let msg = ev
                .data
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            vec![AppEvent::StreamError(msg)]
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
/// subscription is live (session events are live-only, no replay).
///
/// A drop *after* a successful connect is a genuine disconnect: a
/// `StreamError` is emitted and the task exits (the next server-side run
/// respawns it). Also exits when the event channel is dropped (app shutting
/// down) or `shutdown` is set.
pub(crate) fn spawn_session_event_reader(
    client: DaemonClient,
    session_id: String,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    shutdown: Arc<AtomicBool>,
    ready: Option<oneshot::Sender<()>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Exponential backoff for connect retries: 250ms, 500ms, 1s, 2s cap.
        let mut backoff = std::time::Duration::from_millis(250);
        let max_backoff = std::time::Duration::from_secs(2);
        let resp = loop {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            match client.session_events(&session_id).await {
                Ok(r) => break r,
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        session_id,
                        "session events SSE connect failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                }
            }
        };
        let _ = ready.map(|tx| tx.send(()));
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while !shutdown.load(Ordering::SeqCst) {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].trim().to_string();
                        buf = buf[pos + 1..].to_string();
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            match serde_json::from_str::<SessionEvent>(json_str) {
                                Ok(ev) => {
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
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "session events SSE read error");
                    let _ = event_tx.send(AppEvent::StreamError(
                        "lost connection to daemon event stream".to_string(),
                    ));
                    return;
                }
                None => {
                    tracing::warn!(session_id, "session events SSE stream closed");
                    let _ = event_tx.send(AppEvent::StreamError(
                        "lost connection to daemon event stream".to_string(),
                    ));
                    return;
                }
            }
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
        text_snapshot: None,
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
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while !shutdown.load(Ordering::SeqCst) {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].trim().to_string();
                        buf = buf[pos + 1..].to_string();
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            match serde_json::from_str::<TraceEvent>(json_str) {
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
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "trace stream SSE read error");
                    return;
                }
                None => return,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn turn_error_maps() {
        let apps = session_event_to_app_events(ev(
            6,
            SessionEventKind::TurnError,
            serde_json::json!({"message": "boom"}),
        ));
        assert_eq!(apps.len(), 1);
        assert!(matches!(&apps[0], AppEvent::StreamError(m) if m == "boom"));
    }

    #[test]
    fn save_is_noop() {
        let apps =
            session_event_to_app_events(ev(7, SessionEventKind::Save, serde_json::Value::Null));
        assert!(apps.is_empty());
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
