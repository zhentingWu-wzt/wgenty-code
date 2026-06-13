//! Utility functions extracted from app.rs — pure functions with no App state dependency.

use super::app::{AppEvent, DiffData, MessageRole};
use crate::state::agent_phase::{AgentPhase, TurnAbortReason};
use ratatui::layout::Rect;

/// Truncate a user message to a short session name (max ~50 chars, no newlines).
pub fn truncate_session_name(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("");
    let trimmed = first_line.trim();
    if trimmed.len() <= 50 {
        trimmed.to_string()
    } else {
        let end = trimmed
            .char_indices()
            .take(50)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        format!("{}...", &trimmed[..end])
    }
}

/// Start the daemon in a background tokio task and wait for it to be ready.
/// Returns the base URL (including port) and a shutdown sender.
#[cfg(feature = "daemon")]
pub async fn start_daemon(
    app_state: crate::state::AppState,
) -> anyhow::Result<(
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
)> {
    // Bind to a random available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let base_url = format!("http://127.0.0.1:{}", port);
    use crate::daemon::routes;
    use crate::daemon::state::DaemonState;
    use std::sync::Arc;
    use tower_http::cors::{Any, CorsLayer};
    let daemon_state = Arc::new(DaemonState::new(app_state));
    let app = routes::create_router(daemon_state).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });
    // Wait for daemon to be ready (poll health endpoint)
    let client = super::client::DaemonClient::new(base_url.clone());
    for _attempt in 0..50 {
        if client.health().await.is_ok() {
            tracing::info!("daemon ready on port {}", port);
            return Ok((base_url, shutdown_tx, handle));
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    anyhow::bail!("daemon did not become ready within 5 seconds");
}

/// Compute initial collapse state based on line-count thresholds.
/// Returns (content_collapsed, tool_collapsed) tuple.
pub fn compute_collapse_state(role: &MessageRole, content: &str) -> (bool, bool) {
    let line_count = content.lines().count();
    match role {
        MessageRole::Assistant => (line_count > 50, false),
        MessageRole::Tool => (false, true),
        _ => (false, false),
    }
}

/// Extract DiffData from tool result. Tries metadata first, then auto-detects
/// unified diff content (lines with @@ / +++ / --- markers).
pub fn extract_diff_data(
    _name: &str,
    args: &serde_json::Value,
    raw_json: &str,
) -> Option<DiffData> {
    let file_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Try structured metadata first
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw_json) {
        if let Some(metadata) = parsed.get("metadata") {
            if let (Some(old), Some(new)) = (
                metadata.get("old_content").and_then(|v| v.as_str()),
                metadata.get("new_content").and_then(|v| v.as_str()),
            ) {
                return Some(DiffData {
                    file_path,
                    old_content: old.to_string(),
                    new_content: new.to_string(),
                });
            }
        }
    }
    // Auto-detect unified diff in content
    let content = raw_json.trim();
    let has_diff_markers =
        content.contains("@@") && content.contains("+++") && content.contains("---");
    if has_diff_markers {
        let (old, new) = split_unified_diff(content);
        if !old.is_empty() || !new.is_empty() {
            return Some(DiffData {
                file_path,
                old_content: old,
                new_content: new,
            });
        }
    }
    None
}

/// Split a unified diff string into old and new content for diff rendering.
pub fn split_unified_diff(content: &str) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for line in content.lines() {
        if line.starts_with("@@") {
            continue;
        }
        if line.starts_with("---") {
            old.push_str(line.trim_start_matches("--- "));
            old.push('\n');
            continue;
        }
        if line.starts_with("+++") {
            new.push_str(line.trim_start_matches("+++ "));
            new.push('\n');
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            old.push_str(&line[1..]);
            old.push('\n');
        } else if line.starts_with('+') && !line.starts_with("+++") {
            new.push_str(&line[1..]);
            new.push('\n');
        } else {
            old.push_str(line);
            old.push('\n');
            new.push_str(line);
            new.push('\n');
        }
    }
    (old, new)
}

/// Extract execution metadata from a raw tool result JSON.
/// Returns the "metadata" sub-object if present, None otherwise.
pub fn extract_tool_metadata(raw_json: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    parsed.get("metadata").cloned()
}

/// Format a tool result for codex-style tree display. The header bullet is
/// rendered by chat.rs; this produces the content body with action verb,
/// key parameter, and indented output.
pub fn format_tool_result(_name: &str, _args: &serde_json::Value, raw_json: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(raw_json) {
        Ok(v) => v,
        Err(_) => return raw_json.trim_end().to_string(),
    };
    let error = parsed["error"].as_str().unwrap_or("");
    if !error.is_empty() {
        return error.to_string();
    }
    parsed["content"].as_str().unwrap_or("").to_string()
}

pub fn tool_label(name: &str, args: &serde_json::Value) -> String {
    match name {
        "exec_command" | "execute_command" => args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "file_read" | "read_file" => args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "file_write" | "file_edit" | "apply_patch" => args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "grep" | "search" => args
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "glob_search" | "glob" | "list_files" => args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "web_search" => args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "web_fetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Pure function: derive the next AgentPhase from a single AppEvent.
pub fn agent_phase_from_event(event: &AppEvent) -> Option<AgentPhase> {
    match event {
        AppEvent::Submit(_) => Some(AgentPhase::Thinking),
        AppEvent::PreparingTools => Some(AgentPhase::PreparingTools),
        AppEvent::ContentDelta(_) | AppEvent::ReasoningDelta(_) => {
            Some(AgentPhase::StreamingResponse)
        }
        AppEvent::StreamDone { .. } => Some(AgentPhase::Thinking),
        AppEvent::ToolStart { name, args: _ } => {
            Some(AgentPhase::ExecutingTool { name: name.clone() })
        }
        AppEvent::ToolResult { .. } => Some(AgentPhase::Thinking),
        AppEvent::PermissionRequired { reason, rule, .. } => Some(AgentPhase::AwaitingPermission {
            tool: rule.clone(),
            rule: reason.clone(),
        }),
        AppEvent::QuestionAsked { question, .. } => Some(AgentPhase::AwaitingUserInput {
            question: question.clone(),
        }),
        AppEvent::StreamError(_) => Some(AgentPhase::Errored("Stream error".to_string())),
        AppEvent::TurnComplete => Some(AgentPhase::Idle),
        AppEvent::TurnAborted { reason } => match reason {
            TurnAbortReason::TimedOut => {
                Some(AgentPhase::Errored("Agent loop timed out".to_string()))
            }
            _ => Some(AgentPhase::Idle),
        },
        // Events that don't change phase
        AppEvent::MouseScrolled(_)
        | AppEvent::Paste(_)
        | AppEvent::KeyEvent(_)
        | AppEvent::Tick
        | AppEvent::ToggleSessions
        | AppEvent::ToggleTaskPanel
        | AppEvent::CtrlCPressed
        | AppEvent::SessionListLoaded(_)
        | AppEvent::HistoryLoaded(_)
        | AppEvent::PlanUpdate(_)
        | AppEvent::UndoResult(_)
        | AppEvent::SaveSession
        | AppEvent::DeleteSession(_)
        | AppEvent::ToggleCollapseAll
        | AppEvent::ToggleCollapseLatest
        | AppEvent::TodosUpdated(_)
        | AppEvent::TurnStarted { .. }
        | AppEvent::ConfigChanged(_)
        | AppEvent::SubagentUpdate(_)
        | AppEvent::ToggleSubagentPanel => None,
    }
}

/// Helper: create a centered rectangle of the given percentage size within `area`.
/// Used by popup components (session).
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

#[cfg(test)]
mod tests {
    use super::super::app::AppEvent;
    use super::*;
    use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};

    #[test]
    fn test_phase_transitions() {
        assert_eq!(
            agent_phase_from_event(&AppEvent::Submit("hello".into())),
            Some(AgentPhase::Thinking)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::ContentDelta("text".into())),
            Some(AgentPhase::StreamingResponse)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::StreamDone {
                finish_reason: "stop".into()
            }),
            Some(AgentPhase::Thinking)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::ToolStart {
                name: "file_read".into(),
                args: serde_json::json!({})
            }),
            Some(AgentPhase::ExecutingTool {
                name: "file_read".into()
            })
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::ToolResult {
                name: "x".into(),
                args: serde_json::json!({}),
                content: "y".into()
            }),
            Some(AgentPhase::Thinking)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::StreamError("fail".into())),
            Some(AgentPhase::Errored("Stream error".into()))
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnComplete),
            Some(AgentPhase::Idle)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnAborted {
                reason: TurnAbortReason::TimedOut
            }),
            Some(AgentPhase::Errored("Agent loop timed out".into()))
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnAborted {
                reason: TurnAbortReason::Interrupted
            }),
            Some(AgentPhase::Idle)
        );
    }

    #[test]
    fn test_non_phase_events_return_none() {
        assert_eq!(agent_phase_from_event(&AppEvent::Tick), None);
        assert_eq!(agent_phase_from_event(&AppEvent::MouseScrolled(3)), None);
        assert_eq!(
            agent_phase_from_event(&AppEvent::Paste("test".into())),
            None
        );
        assert_eq!(agent_phase_from_event(&AppEvent::SaveSession), None);
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnStarted {
                turn_id: TurnId::new()
            }),
            None
        );
    }

    #[test]
    fn test_phase_is_busy() {
        assert!(!AgentPhase::Idle.is_busy());
        assert!(!AgentPhase::Completed.is_busy());
        assert!(AgentPhase::Thinking.is_busy());
        assert!(AgentPhase::StreamingResponse.is_busy());
        assert!(AgentPhase::ExecutingTool { name: "x".into() }.is_busy());
        assert!(!AgentPhase::Errored("e".into()).is_busy());
    }
}
