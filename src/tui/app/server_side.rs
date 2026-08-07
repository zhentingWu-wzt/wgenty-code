//! Server-side agent loop bridge.
//!
//! Maps daemon `SessionEvent`s (from `GET /sessions/:id/events` SSE) into TUI
//! `AppEvent`s, so the TUI can observe a daemon-owned agent run the same way
//! the Web client does. Foundation for migrating the TUI from a client-side
//! loop to a server-side loop.

use crate::daemon::run_loop::{SessionEvent, SessionEventKind};
use crate::tui::app::types::AppEvent;
use crate::tui::client::DaemonClient;
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

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
        SessionEventKind::Save => Vec::new(),
    }
}

/// Spawn a background task that subscribes to the daemon's session-event SSE
/// stream and forwards mapped `AppEvent`s into `event_tx`.
///
/// Runs until the SSE stream closes, the event channel is dropped (app
/// shutting down), or `shutdown` is set.
pub(crate) fn spawn_session_event_reader(
    client: DaemonClient,
    session_id: String,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    shutdown: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let resp = match client.session_events(&session_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "session events SSE connect failed");
                let _ = event_tx.send(AppEvent::StreamError(format!(
                    "lost connection to daemon event stream: {e}"
                )));
                return;
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
                    break;
                }
                None => break,
            }
        }
    });
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
}
