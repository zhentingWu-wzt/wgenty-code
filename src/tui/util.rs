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

/// Outcome of [`start_daemon`]: either an already-running daemon was reused
/// via the discovery file, or a new embedded daemon was spawned locally.
/// `Reused` carries no shutdown handles — the daemon is owned by another
/// process and must keep running after this TUI exits.
#[cfg(feature = "daemon")]
pub enum StartDaemonOutcome {
    Reused {
        base_url: String,
    },
    Spawned {
        base_url: String,
    },
    Embedded {
        base_url: String,
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    },
}

/// How long to wait for a spawned daemon to become healthy before giving up.
#[cfg(feature = "daemon")]
const HEALTH_POLL_TIMEOUT_SECS: u64 = 15;

/// Start the daemon, preferring an external process so it survives TUI exit.
///
/// Decision chain:
/// 1. **Discovery reuse**: an already-running daemon is found via
///    `daemon.json` -> connect to it (owned by another process).
/// 2. **External spawn**: spawn `current_exe() daemon --port 8371` as a
///    detached child process. The child writes its own token + discovery
///    file and keeps running after the TUI exits, so Web/Desktop can reuse it.
/// 3. **Retry discovery**: if the spawn failed (e.g. port taken), another UI
///    may have started a daemon meanwhile -> retry discovery.
/// 4. **Embedded fallback**: if everything above fails, start the daemon
///    in-process (legacy behavior; killed on TUI exit).
#[cfg(feature = "daemon")]
pub async fn start_daemon(app_state: crate::state::AppState) -> anyhow::Result<StartDaemonOutcome> {
    // 1. Reuse an already-running global daemon when the discovery file
    // checks out; fall through to spawning otherwise (design §6.3).
    if let Some(found) = crate::utils::discovery::discover_daemon() {
        tracing::info!(
            port = found.port,
            "reusing running daemon via discovery file"
        );
        return Ok(StartDaemonOutcome::Reused {
            base_url: format!("http://127.0.0.1:{}", found.port),
        });
    }

    // 2. Spawn an external daemon process. The child inherits the current
    // working directory (so the daemon's `working_dir` matches the TUI's
    // project root), writes its own token + discovery file, and survives
    // TUI exit. stdio is redirected to null so the child doesn't hold the
    // terminal.
    const DEFAULT_PORT: u16 = 8371;
    if let Some(base_url) = spawn_external_daemon(DEFAULT_PORT).await {
        tracing::info!(
            "daemon spawned as external process on port {}",
            DEFAULT_PORT
        );
        return Ok(StartDaemonOutcome::Spawned { base_url });
    }

    // 3. Retry discovery: a concurrent UI may have started a daemon while we
    // were polling health. If so, reuse it instead of spawning a duplicate.
    if let Some(found) = crate::utils::discovery::discover_daemon() {
        tracing::info!(
            port = found.port,
            "reusing daemon started by a concurrent UI (post-spawn-retry)"
        );
        return Ok(StartDaemonOutcome::Reused {
            base_url: format!("http://127.0.0.1:{}", found.port),
        });
    }

    // 4. Fallback: embedded daemon (in-process). This is killed on TUI exit,
    // so other UIs connected to it will lose connection. Log a warning.
    tracing::warn!(
        "external daemon spawn failed; falling back to embedded daemon \
         (will be killed on TUI exit, other UIs may lose connection)"
    );
    spawn_embedded_daemon(app_state).await
}

/// Spawn `current_exe() daemon --port <port>` as a detached child process
/// and poll until the daemon's health endpoint answers. Returns the base URL
/// on success, or `None` if the process couldn't spawn or the daemon never
/// came up (timeout / bind failure / crash / foreign process on the port).
#[cfg(feature = "daemon")]
async fn spawn_external_daemon(port: u16) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let child = std::process::Command::new(&exe)
        .arg("daemon")
        .arg("--port")
        .arg(port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    tracing::debug!(
        pid = child.id(),
        exe = %exe.display(),
        port,
        "spawned external daemon process"
    );
    // Drop the child handle so we don't hold a waitable reference; the
    // process is now fully independent and survives TUI exit.
    drop(child);

    // Poll the health endpoint until the daemon serves HTTP. A bare TCP
    // connect is not enough: the port may be held by a foreign process, and
    // attaching to it would surface later as auth (401) or protocol errors.
    let health_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .ok()?;
    let base_url = format!("http://127.0.0.1:{}", port);
    let mut interval_ms = 200u64;
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(HEALTH_POLL_TIMEOUT_SECS);
    loop {
        if std::time::Instant::now() > deadline {
            tracing::warn!(port, "external daemon health poll timed out");
            return None;
        }
        if crate::utils::http::probe_daemon_health(&health_client, &base_url)
            .await
            .is_some()
        {
            return Some(base_url);
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        interval_ms = (interval_ms * 2).min(2000);
    }
}

/// Fallback: start the daemon in a background tokio task (in-process).
/// The returned `shutdown_tx` kills the daemon when the TUI exits.
#[cfg(feature = "daemon")]
async fn spawn_embedded_daemon(
    app_state: crate::state::AppState,
) -> anyhow::Result<StartDaemonOutcome> {
    const DEFAULT_PORT: u16 = 8371;
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", DEFAULT_PORT)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0").await?,
    };
    let port = listener.local_addr()?.port();
    let base_url = format!("http://127.0.0.1:{}", port);
    crate::utils::startup_timing::mark("daemon: tcp bound");
    use crate::daemon::state::DaemonState;
    use crate::daemon::{auth, routes};
    use std::sync::Arc;
    let daemon_state = Arc::new(DaemonState::new(app_state).await);
    crate::utils::startup_timing::mark("daemon: DaemonState created");

    {
        let sm_state = Arc::clone(&daemon_state);
        tokio::spawn(async move {
            if let Err(e) = sm_state.session_manager.load_index().await {
                tracing::warn!(error = %e, "Failed to load session index into background daemon");
            }
            tracing::debug!("background session index load complete");
        });
    }

    let api_token = auth::generate_api_token();
    crate::utils::write_daemon_token(&api_token)?;
    crate::utils::discovery::spawn_discovery_writer(port, api_token.clone());
    let cleanup_token = api_token.clone();
    let shutdown_notify = daemon_state.shutdown_notify.clone();
    let (health, protected) = routes::create_routers(daemon_state, api_token);
    crate::utils::startup_timing::mark("daemon: routers created");
    let app = health.merge(protected).layer(
        tower_http::cors::CorsLayer::new()
            .allow_origin([
                "http://localhost:3000"
                    .parse()
                    .expect("invalid hardcoded URL literal"),
                "http://localhost:5173"
                    .parse()
                    .expect("invalid hardcoded URL literal"),
                "http://127.0.0.1:3000"
                    .parse()
                    .expect("invalid hardcoded URL literal"),
                "http://127.0.0.1:5173"
                    .parse()
                    .expect("invalid hardcoded URL literal"),
            ])
            .allow_methods([
                http::Method::GET,
                http::Method::POST,
                http::Method::PUT,
                http::Method::DELETE,
            ])
            .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE]),
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    // TUI exit path.
                    _ = shutdown_rx => {}
                    // POST /api/v1/shutdown (`wgenty-code daemon stop`).
                    () = shutdown_notify.notified() => {}
                }
            })
            .await
            .ok();
        // Ownership-checked cleanup: see daemon/mod.rs — a newer daemon may
        // already own these files by the time this embedded instance exits.
        let _ = crate::utils::remove_daemon_token_if_matches(&cleanup_token);
        let _ = crate::utils::discovery::remove_discovery_file_if_pid(std::process::id());
    });
    tokio::task::yield_now().await;
    tracing::info!("embedded daemon ready on port {}", port);
    Ok(StartDaemonOutcome::Embedded {
        base_url,
        shutdown_tx,
        handle,
    })
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

fn arg_str(args: &serde_json::Value, key: &str) -> String {
    args.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn arg_u64(args: &serde_json::Value, key: &str) -> String {
    args.get(key)
        .and_then(|value| value.as_u64())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn truncate_label(label: String) -> String {
    const MAX_LABEL_CHARS: usize = 80;
    if label.chars().count() <= MAX_LABEL_CHARS {
        label
    } else {
        let mut truncated = label.chars().take(MAX_LABEL_CHARS).collect::<String>();
        truncated.push('…');
        truncated
    }
}

pub fn tool_label(name: &str, args: &serde_json::Value) -> String {
    match name {
        "exec_command" | "execute_command" | "background" => arg_str(args, "command"),
        "file_read" | "read_file" | "file_write" | "file_edit" | "view" => args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "apply_patch" => {
            let workdir = arg_str(args, "workdir");
            if workdir.is_empty() {
                truncate_label(arg_str(args, "patch"))
            } else {
                workdir
            }
        }
        "grep" | "search" => args
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "glob_search" | "glob" | "list_files" => args
            .get("path")
            .or_else(|| args.get("pattern"))
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
        "ask_user_question" => truncate_label(arg_str(args, "question")),
        "checkpoint" => arg_str(args, "description"),
        "codegraph_node" | "lsp" => arg_str(args, "symbol"),
        "codegraph_explore" => arg_str(args, "query"),
        "git_operations" => {
            let operation = arg_str(args, "operation");
            let branch = arg_str(args, "branch");
            let path = arg_str(args, "path");
            if !branch.is_empty() {
                format!("{} {}", operation, branch)
            } else if !path.is_empty() && path != "." {
                format!("{} {}", operation, path)
            } else {
                operation
            }
        }
        "kill_session" | "write_stdin" => {
            let session_id = arg_u64(args, "session_id");
            if session_id.is_empty() {
                String::new()
            } else {
                format!("session {}", session_id)
            }
        }
        "load_skill" => {
            let skill_name = arg_str(args, "name");
            if skill_name.is_empty() {
                "available skills".to_string()
            } else {
                skill_name
            }
        }
        "run_script" => truncate_label(arg_str(args, "script")),
        "run_test" => {
            let file = arg_str(args, "file");
            let filter = arg_str(args, "filter");
            let framework = arg_str(args, "framework");
            [file, filter, framework]
                .into_iter()
                .find(|value| !value.is_empty())
                .unwrap_or_else(|| "auto".to_string())
        }
        "TodoWrite" | "update_plan" => {
            let item_key = if name == "TodoWrite" { "items" } else { "plan" };
            let item_count = args
                .get(item_key)
                .and_then(|value| value.as_array())
                .map(|items| items.len())
                .unwrap_or(0);
            format!(
                "{} item{}",
                item_count,
                if item_count == 1 { "" } else { "s" }
            )
        }
        "task" => {
            let desc = arg_str(args, "description");
            let sub_type = arg_str(args, "subagent_type");
            if sub_type.is_empty() {
                desc
            } else {
                format!("[{}] {}", sub_type, desc)
            }
        }
        "delegate" => truncate_label(arg_str(args, "task")),
        "task_management" => {
            let operation = arg_str(args, "operation");
            let subject = arg_str(args, "subject");
            let task_id = arg_str(args, "task_id");
            if !subject.is_empty() {
                format!("{} {}", operation, subject)
            } else if !task_id.is_empty() {
                format!("{} {}", operation, task_id)
            } else {
                operation
            }
        }
        "team_message" => {
            let action = arg_str(args, "action");
            let to = arg_str(args, "to");
            let from = arg_str(args, "from");
            if !to.is_empty() {
                format!("{} → {}", action, to)
            } else if !from.is_empty() {
                format!("{} {}", action, from)
            } else {
                action
            }
        }
        "think" => "scratchpad".to_string(),
        "undo" => "latest checkpoint".to_string(),
        "compact" => "conversation history".to_string(),
        _ => name.to_string(),
    }
}

/// Pure function: derive the next AgentPhase from a single AppEvent.
pub fn agent_phase_from_event(event: &AppEvent) -> Option<AgentPhase> {
    match event {
        AppEvent::Submit(_) => Some(AgentPhase::Thinking),
        AppEvent::Connecting {
            attempt,
            max_retries,
        } => Some(AgentPhase::Connecting {
            attempt: *attempt,
            max_retries: *max_retries,
        }),
        AppEvent::PreparingTools => Some(AgentPhase::PreparingTools),
        AppEvent::ContentDelta(_) | AppEvent::ReasoningDelta(_) => {
            Some(AgentPhase::StreamingResponse)
        }
        AppEvent::StreamDone { .. } => Some(AgentPhase::Thinking),
        AppEvent::ToolStart { name, args: _ } => {
            Some(AgentPhase::ExecutingTool { name: name.clone() })
        }
        AppEvent::ToolResult { .. } => Some(AgentPhase::Thinking),
        AppEvent::PermissionRequired {
            tool_name,
            reason,
            rule,
            ..
        } => Some(AgentPhase::AwaitingPermission {
            tool: tool_name.clone(),
            rule: if rule.is_empty() {
                reason.clone()
            } else {
                format!("{rule}: {reason}")
            },
        }),
        AppEvent::QuestionAsked { question, .. } => Some(AgentPhase::AwaitingUserInput {
            question: question.clone(),
        }),
        AppEvent::StreamError(_) => Some(AgentPhase::Errored("Stream error".to_string())),
        AppEvent::CompactionStarted => Some(AgentPhase::Compacting),
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
        | AppEvent::ToggleMemory
        | AppEvent::MemoryListLoaded(_)
        | AppEvent::CtrlCPressed
        | AppEvent::SessionListLoaded(_)
        | AppEvent::HistoryLoaded { .. }
        | AppEvent::PlanUpdate(_)
        | AppEvent::TodosSnapshot(_)
        | AppEvent::UndoResult(_)
        | AppEvent::SaveSession
        | AppEvent::DeleteSession(_)
        | AppEvent::DeleteMemory(_, _)
        | AppEvent::ReinforceMemory(_, _)
        | AppEvent::PenalizeMemory(_, _)
        | AppEvent::ToggleCollapseAll
        | AppEvent::ToggleCollapseLatest
        | AppEvent::TurnStarted { .. }
        | AppEvent::ServerTurnTerminated
        | AppEvent::ConfigChanged(_)
        | AppEvent::ContextCompacted { .. }
        | AppEvent::AgentLocalView { .. }
        | AppEvent::BackgroundTaskResult(_)
        | AppEvent::BackgroundTaskRecovered(_)
        | AppEvent::BackgroundTaskCompleted(_)
        | AppEvent::SessionClearFailed { .. }
        | AppEvent::AgentGenerationReset { .. }
        | AppEvent::NavigateAgent { .. }
        | AppEvent::AgentViewNavigated(_)
        | AppEvent::NavigateAgentBack
        | AppEvent::MemoriesReady(_)
        | AppEvent::GlobalMemoriesReady(_)
        | AppEvent::SkillsReady(_)
        | AppEvent::TurnContextCaptured(_)
        | AppEvent::SystemNotice(_)
        | AppEvent::UndoRequested { .. }
        | AppEvent::RefreshUndoFileCounts
        | AppEvent::UndoFileCountsReady(_)
        | AppEvent::RefreshModels
        | AppEvent::ModelsReady(_)
        | AppEvent::ModelSwitchRequested { .. }
        | AppEvent::ModelSwitched { .. }
        | AppEvent::SessionSwitched { .. }
        | AppEvent::SessionCleared { .. }
        | AppEvent::ServerPermissionRequired { .. }
        | AppEvent::ServerQuestionAsked { .. }
        | AppEvent::SubagentTraceProgress(..)
        | AppEvent::ServerPermissionResolved { .. }
        | AppEvent::ServerQuestionResolved { .. }
        | AppEvent::ModelSwitchFailed(_) => None,
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

/// Next index with wrap-around over `count` items. Returns 0 when `count` is 0
/// (no items to navigate). Used by subagent status bar and focus-view selector
/// keyboard navigation (↓).
pub fn wrap_next(current: usize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        (current + 1) % count
    }
}

/// Previous index with wrap-around over `count` items. Returns 0 when `count`
/// is 0. Used by subagent status bar and focus-view selector navigation (↑).
pub fn wrap_prev(current: usize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        (current + count - 1) % count
    }
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
            agent_phase_from_event(&AppEvent::CompactionStarted),
            Some(AgentPhase::Compacting)
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
        // Startup memory recall is non-phase - it must not change the agent
        // phase indicator.
        assert_eq!(
            agent_phase_from_event(&AppEvent::MemoriesReady(vec!["mem1".into()])),
            None
        );
        // /clear 切换会话不改变 agent phase（仍为 Idle）
        assert_eq!(
            agent_phase_from_event(&AppEvent::SessionSwitched {
                from_id: "old".into(),
                id: "s1".into(),
                name: "New Session".into()
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
        assert!(AgentPhase::Compacting.is_busy());
        assert!(!AgentPhase::Errored("e".into()).is_busy());
    }

    #[test]
    fn test_wrap_next_advances_and_wraps() {
        assert_eq!(wrap_next(0, 3), 1);
        assert_eq!(wrap_next(1, 3), 2);
        assert_eq!(wrap_next(2, 3), 0); // wraps around to start
    }

    #[test]
    fn test_wrap_prev_decrements_and_wraps() {
        assert_eq!(wrap_prev(0, 3), 2); // wraps around to end
        assert_eq!(wrap_prev(1, 3), 0);
        assert_eq!(wrap_prev(2, 3), 1);
    }

    #[test]
    fn test_wrap_zero_count_returns_zero() {
        // No items → no movement, no panic (guard against div-by-zero).
        assert_eq!(wrap_next(5, 0), 0);
        assert_eq!(wrap_prev(5, 0), 0);
    }

    #[test]
    fn test_wrap_single_element_stays_at_zero() {
        assert_eq!(wrap_next(0, 1), 0);
        assert_eq!(wrap_prev(0, 1), 0);
    }
}
