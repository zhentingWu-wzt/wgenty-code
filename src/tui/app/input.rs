//! Input submission — slash commands and normal user input.

use super::types::*;
use super::{App, PendingInput};
use crate::api::ChatMessage;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason};

impl App {
    pub(super) fn push_system_message(&mut self, content: impl Into<String>) {
        self.committed_messages.push(UIMessage {
            role: MessageRole::System,
            content: content.into(),
            tool_name: None,
            content_collapsed: false,
            tool_collapsed: false,
            tool_running: false,
            tool_args: None,
            diff_data: None,
            tool_metadata: None,
        });
    }

    /// Submit user input, automatically queueing if a Turn is already running.
    pub(super) fn submit_input(&mut self, text: String) {
        // Bang commands: `! <command>` runs a shell command directly in the
        // local working directory and shows its output as a system message.
        // This bypasses the agent turn entirely (no LLM tokens, no history,
        // no permission/sandbox/hook chain), matching Claude Code's bash mode.
        // Must precede the `/` slash-command checks so `!` is never routed as
        // a slash command or ordinary message.
        if is_bang_input(&text) {
            match parse_bang_command(&text) {
                Some(command) => self.run_bang_command(command),
                None => self.push_system_message(
                    "Usage: ! <command> - run a shell command directly and show its output.",
                ),
            }
            return;
        }
        // Slash commands
        if text.trim() == "/clear" {
            // Empty session: nothing to save, and creating another empty
            // session would just clutter the session list.
            if self.committed_messages.is_empty() {
                self.push_system_message("会话为空，无需清除");
                return;
            }

            // Snapshot UI transcript synchronously (no await needed) before
            // clearing the display. History is snapshotted inside the spawn
            // below because the tokio Mutex cannot be locked from this sync ctx.
            let old_id = self.session_id.clone();
            let previous_suppress_phase_updates = self.suppress_phase_updates;
            let old_ui_messages: Vec<_> = self
                .committed_messages
                .iter()
                .map(UIMessage::to_session_ui_message)
                .collect();

            // Clear visible state immediately so /clear returns a clean slate.
            self.committed_messages.clear();
            self.streaming_content.clear();
            self.streaming_active = false;
            self.scroll_offset = 0;
            self.user_scrolled = false;
            self.sandbox_bypassed_session = false;
            // Automatic daemon continuation is intentionally invisible to
            // local run gates. Fence every old-session save before canceling.
            self.mark_daemon_owned_session_history();
            self.cancel_current_turn();
            // Keep a daemon-owned run gated until the async phase confirms
            // cancellation and emits the explicit reset/adoption event.
            // Reset phase immediately and suppress stale events from the
            // just-aborted turn so the status bar shows "Ready" instead of
            // lingering on "Thinking". Cleared by the follow-up
            // AgentGenerationReset (both success and failure paths spawn one).
            self.phase = AgentPhase::Idle;
            self.suppress_phase_updates = true;
            // Clear queued inputs: a fresh generation cancels obsolete work.
            self.pending_inputs.clear();

            // Async: snapshot+clear history, cancel daemon ownership, create a
            // new session, then switch. Never PUT the old local snapshot: the
            // daemon owns any final history produced by cancellation.
            let client = self.daemon_client.clone();
            let history = self.conversation_history.clone();
            let event_tx = self.event_tx.clone();
            tokio::spawn(async move {
                let old_history = {
                    let mut h = history.lock().await;
                    crate::api::types::sanitize_tool_call_pairing(&mut h);
                    let snapshot = h.clone();
                    h.clear();
                    snapshot
                };

                // 1. Terminate daemon ownership before switching. A
                // server-side run persists its own final history after
                // cancellation, so a stale TUI PUT must not race that save.
                if let Err(error) = client.cancel_run(&old_id).await {
                    tracing::warn!(
                        session_id = %old_id,
                        error = %error,
                        "failed to cancel daemon session during /clear"
                    );
                    {
                        let mut current = history.lock().await;
                        *current = old_history.clone();
                    }
                    let _ = event_tx.send(AppEvent::HistoryLoaded {
                        messages: old_history,
                        ui_messages: old_ui_messages,
                    });
                    let _ = event_tx.send(AppEvent::SessionClearFailed {
                        suppress_phase_updates: previous_suppress_phase_updates,
                    });
                    let _ = event_tx.send(AppEvent::SystemNotice(format!(
                        "⚠️ 无法取消当前会话，未创建新会话：{error}"
                    )));
                    return;
                }

                // 2. Create a new session and switch.
                match client.create_session(None).await {
                    Ok(resp) => {
                        let _ = event_tx.send(AppEvent::SessionCleared {
                            id: resp.id,
                            name: resp.name,
                        });
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "create_session failed after /clear");
                        let _ = event_tx.send(AppEvent::SystemNotice(format!(
                            "⚠️ 创建新会话失败：{error}（当前会话已清空，可继续使用）"
                        )));
                        // Fallback: reset generation under the old id so
                        // subagent state and suppress_phase_updates are
                        // cleaned up (session_id stays the old one).
                        match client.reset_agent_generation(&old_id).await {
                            Ok(generation) => {
                                let _ =
                                    event_tx.send(AppEvent::AgentGenerationReset { generation });
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = %error,
                                    "reset_agent_generation failed; retaining old generation"
                                );
                                let _ = event_tx.send(AppEvent::AgentGenerationReset {
                                    generation: u64::MAX,
                                });
                            }
                        }
                    }
                }
            });
            return;
        }
        if text.trim() == "/plan" {
            let is_plan = self.mode == AgentMode::PlanMode;
            if is_plan {
                // Leaving PlanMode: restore previous mode if saved
                self.mode = self.previous_mode.take().unwrap_or(AgentMode::Normal);
            } else {
                // Entering PlanMode: save current mode for restore
                self.previous_mode = Some(self.mode);
                self.mode = AgentMode::PlanMode;
            }
            let msg = if !is_plan {
                "Plan mode enabled"
            } else {
                "Plan mode disabled"
            };
            self.sync_permission_mode_to_daemon();
            self.apply_mode_to_prompt_permissions();
            self.phase = AgentPhase::Idle;
            self.committed_messages.push(UIMessage {
                role: MessageRole::System,
                content: msg.to_string(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_running: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            return;
        }
        if text.trim() == "/continue" {
            if let Some(ref reason) = self.last_abort_reason {
                let label = match reason {
                    TurnAbortReason::MaxRoundsExceeded => "max rounds limit",
                    TurnAbortReason::TimedOut => "timeout",
                    _ => "recoverable error",
                };
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("\u{267B}\u{FE0F} Continuing after {}...", label),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    tool_running: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
                // Inject system message into conversation history
                let history = self.conversation_history.clone();
                let label_clone = label.to_string();
                tokio::spawn(async move {
                    let mut h = history.lock().await;
                    h.push(ChatMessage::system(format!(
                        "[User pressed /continue after {}. Continue working on the previous task from where you left off.]",
                        label_clone
                    )));
                });
                self.last_abort_reason = None;
                self.pending_inputs.push_back(PendingInput::new(
                    "Continue the current task from where you left off.".to_string(),
                ));
                if !self.has_running_turn() {
                    self.start_next_turn();
                }
            } else {
                self.phase = AgentPhase::Idle;
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: "No interrupted turn to continue. The last turn completed normally."
                        .to_string(),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    tool_running: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            return;
        }
        if text.trim() == "/init" {
            self.push_system_message(
                "🔄 Running /init — 正在分析代码库以生成 WGENTY.md 和 AGENTS.md...",
            );
            if !self.has_running_turn() {
                let init_prompt = crate::prompts::get_init_prompt().to_string();
                self.spawn_agent_turn(init_prompt, true);
            }
            return;
        }
        if text.trim() == "/compact" {
            if self.has_running_turn() {
                self.push_system_message(
                    "⏳ Please wait for the current task to finish before compacting.",
                );
                return;
            }
            self.push_system_message("🔄 Compacting conversation history...");
            self.spawn_compact_turn();
            return;
        }
        // Session / memory browsers (slash-only; no Ctrl bindings).
        // Accept common plurals as aliases.
        let slash = text.trim();
        if matches!(slash, "/session" | "/sessions") {
            let _ = self.event_tx.send(AppEvent::ToggleSessions);
            return;
        }
        if matches!(slash, "/memory" | "/memories") {
            let _ = self.event_tx.send(AppEvent::ToggleMemory);
            return;
        }
        if matches!(slash, "/undo") {
            self.open_undo_picker();
            return;
        }
        // /model            → open the model picker popup
        // /model <profile>  → switch directly to that profile (no popup)
        if slash == "/model" {
            let rest = text.trim().strip_prefix("/model").unwrap_or("").trim();
            if rest.is_empty() {
                self.open_model_picker();
            } else {
                self.switch_model_direct(rest);
            }
            return;
        }
        if slash == "/server-side" {
            self.server_side_loop = !self.server_side_loop;
            self.push_system_message(format!(
                "server-side loop: {}",
                if self.server_side_loop { "ON" } else { "OFF" }
            ));
            self.phase = AgentPhase::Idle;
            return;
        }
        if text.trim() == "/help" {
            let commands = crate::tui::completion::CompletionEngine::default_builtin_commands()
                .into_iter()
                .map(|command| format!("/{} — {}", command.name, command.description))
                .collect::<Vec<_>>()
                .join("\n");
            self.push_system_message(format!(
                "Available commands:\n{}\n\n! <command> - Run a shell command directly and show its output",
                commands
            ));
            self.phase = AgentPhase::Idle;
            return;
        }

        // NOTE: UserPromptSubmit hook is now fired inside AgentLoop::process_input_inner
        // (await + 10s timeout), so injected fragments can flow into the per-turn
        // <system-reminder> block. The previous fire-and-forget tokio::spawn was
        // removed in §3 of the system-reminder-channel change.

        // Route unrecognized slash commands via CommandRouter.
        // This catches workflow invocations like /comet or /verify
        // that are not handled by the built-in checks above.
        let trimmed = text.trim();
        if trimmed.starts_with('/') {
            if let Some(ref router) = self.command_router {
                match router.route(&text) {
                    crate::runtime::command::RouteResult::Workflow {
                        name,
                        command,
                        args,
                    } => {
                        // Fire SlashCommand hooks asynchronously
                        {
                            let hm = self.hook_manager.clone();
                            let sid = self.session_id.clone();
                            let cmd = command.clone();
                            let a = args.clone();
                            let cwd = std::env::current_dir().unwrap_or_default();
                            tokio::spawn(async move {
                                let ctx = crate::runtime::hooks::HookContext {
                                    event: "SlashCommand".to_string(),
                                    tool_name: Some(cmd.clone()),
                                    tool_input: Some(serde_json::json!({
                                        "command": cmd,
                                        "args": a,
                                    })),
                                    tool_result: None,
                                    session_id: Some(sid),
                                    working_directory: cwd.to_string_lossy().to_string(),
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                    comet_phase: None,
                                    workflow_state: None,
                                    variables: Default::default(),
                                };
                                hm.fire(
                                    &crate::runtime::hooks::HookEvent::SlashCommand,
                                    &ctx,
                                    None,
                                    None,
                                )
                                .await;
                            });
                        }
                        // Show friendly status message
                        self.push_system_message(format!("Starting {} workflow...", name));
                        let agent_input = crate::runtime::command::workflow_invocation_prompt(
                            &name, &command, &args, &text,
                        );
                        self.pending_inputs
                            .push_back(PendingInput::internal(text.clone(), agent_input));
                        if !self.has_running_turn() {
                            self.start_next_turn();
                        }
                        return;
                    }
                    crate::runtime::command::RouteResult::Unknown {
                        command,
                        suggestions,
                    } => {
                        let msg = if suggestions.is_empty() {
                            format!(
                                "Unknown command: /{}. Type /help for available commands.",
                                command
                            )
                        } else {
                            format!(
                                "Unknown command: /{}. Did you mean: /{}?",
                                command,
                                suggestions.join(", /")
                            )
                        };
                        self.push_system_message(msg);
                        return;
                    }
                    // BuiltIn — already handled above
                    crate::runtime::command::RouteResult::BuiltIn => {}
                    // NotSlash — can't happen here (already checked starts_with('/'))
                    crate::runtime::command::RouteResult::NotSlash => {}
                }
            }
        }
        if self.mode == AgentMode::PlanMode {
            self.phase = AgentPhase::Thinking;
            self.pending_inputs.push_back(PendingInput::new(text));
            self.start_next_turn();
            // PlanMode now persists across turns — the agent detects plan
            // confirmation replies and skips re-planning automatically.
            return;
        }
        self.pending_inputs.push_back(PendingInput::new(text));
        if !self.has_running_turn() {
            self.start_next_turn();
        }
    }

    /// Spawn a bang command (`! <command>`) for direct local execution.
    ///
    /// Shows an immediate "running" system message, then runs the command via
    /// the platform shell (with captured stdio) in the current working
    /// directory with a 120s timeout. The result is delivered back to the UI
    /// as a `BackgroundTaskResult` system message (the existing channel for
    /// background-task notifications), so no new event variant or render
    /// branch is needed.
    fn run_bang_command(&mut self, command: String) {
        // Immediate feedback so the user sees the command was accepted. The
        // command line is shown here once; the result message below carries
        // only the output (stdout/stderr/exit) to avoid duplication.
        self.push_system_message(format!("$ {}", command));

        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let cwd = std::env::current_dir().unwrap_or_default();
            let mut cmd = crate::sandbox::shell_command_captured(&command);
            cmd.current_dir(&cwd);
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(BANG_COMMAND_TIMEOUT_SECS),
                cmd.output(),
            )
            .await;

            let message = match result {
                Ok(Ok(output)) => format_bang_output(&output),
                Ok(Err(e)) => format!("Failed to execute command: {}", e),
                Err(_) => format!("Command timed out ({}s)", BANG_COMMAND_TIMEOUT_SECS),
            };
            let _ = tx.send(AppEvent::BackgroundTaskResult(message));
        });
    }
}

/// Default timeout for a bang command, in seconds. Matches the minimum
/// enforced by the `execute_command` agent tool.
const BANG_COMMAND_TIMEOUT_SECS: u64 = 120;

/// Returns true if the (trimmed) first line starts with `!`, i.e. the user
/// intends a bang command - even if the command body is empty (bare `!`).
fn is_bang_input(text: &str) -> bool {
    text.lines()
        .next()
        .unwrap_or("")
        .trim_start()
        .starts_with('!')
}

/// Parse a bang command from user input.
///
/// Returns the command body when the (trimmed) first line starts with `!`.
/// The leading `!` and any spaces immediately after it are stripped, so
/// `!ls`, `! ls`, and `!  ls -la` all yield `ls -la`. Returns `None` for
/// non-`!` input or a bare `!` with no command body.
fn parse_bang_command(text: &str) -> Option<String> {
    let first_line = text.lines().next().unwrap_or("").trim_start();
    let rest = first_line.strip_prefix('!')?;
    let command = rest.trim_start();
    if command.is_empty() {
        return None;
    }
    Some(command.to_string())
}

/// Format the output of a bang command into a single system message.
///
/// The command line itself is shown separately by `run_bang_command` as
/// immediate feedback, so this carries only stdout/stderr/exit status.
fn format_bang_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code();

    let mut parts: Vec<String> = Vec::new();
    if !stdout.is_empty() {
        parts.push(stdout.trim_end().to_string());
    }
    if !stderr.is_empty() {
        parts.push(format!("[stderr]\n{}", stderr.trim_end()));
    }
    // Only surface the exit code when the command did not succeed; a trailing
    // "exit code 0" on every successful command is noise. An empty result with
    // a zero exit is reported explicitly so success is still visible.
    if !output.status.success() {
        if let Some(code) = exit_code {
            parts.push(format!("[exit code {}]", code));
        } else {
            parts.push("[terminated by signal]".to_string());
        }
    } else if parts.is_empty() {
        parts.push("(no output)".to_string());
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::watcher::SettingsHandle;
    use crate::config::Settings;
    use crate::tui::client::DaemonClient;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::{post, put};
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    #[derive(Clone, Default)]
    struct ClearCapture {
        cancellations: Arc<AtomicUsize>,
        saves: Arc<AtomicUsize>,
        creations: Arc<AtomicUsize>,
        reject_cancel: bool,
        create_delay: Duration,
    }

    async fn capture_cancel(State(capture): State<ClearCapture>) -> StatusCode {
        capture.cancellations.fetch_add(1, Ordering::SeqCst);
        if capture.reject_cancel {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::NO_CONTENT
        }
    }

    async fn accept_save(State(capture): State<ClearCapture>) -> StatusCode {
        capture.saves.fetch_add(1, Ordering::SeqCst);
        StatusCode::NO_CONTENT
    }

    async fn create_cleared_session(
        State(capture): State<ClearCapture>,
    ) -> Json<serde_json::Value> {
        tokio::time::sleep(capture.create_delay).await;
        capture.creations.fetch_add(1, Ordering::SeqCst);
        Json(serde_json::json!({
            "id": "cleared-session",
            "name": "New Session",
            "created_at": "",
            "updated_at": "",
            "messages": [],
            "ui_messages": []
        }))
    }

    async fn assert_clear_cancels_and_adopts(local_run_gate: bool) {
        let capture = ClearCapture {
            create_delay: Duration::default(),
            ..ClearCapture::default()
        };
        let router = Router::new()
            .route("/api/v1/sessions", post(create_cleared_session))
            .route("/api/v1/sessions/:id", put(accept_save))
            .route("/api/v1/sessions/:id/cancel", post(capture_cancel))
            .with_state(capture.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind clear protocol server");
        let address = listener.local_addr().expect("clear server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve clear protocol server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "active-session".to_string(),
            settings,
        );
        app.committed_messages.push(UIMessage {
            role: MessageRole::User,
            content: "running".to_string(),
            tool_name: None,
            content_collapsed: false,
            tool_collapsed: false,
            tool_running: false,
            tool_args: None,
            diff_data: None,
            tool_metadata: None,
        });
        app.server_side_loop = true;
        app.server_side_turn_active = local_run_gate;

        app.submit_input("/clear".to_string());

        tokio::time::timeout(Duration::from_secs(2), async {
            while app.session_id != "cleared-session" {
                let event = app
                    .event_rx
                    .recv()
                    .await
                    .expect("app event channel remains open");
                app.handle_event(event).await;
            }
        })
        .await
        .expect("clear must adopt the session it creates");
        assert_eq!(capture.cancellations.load(Ordering::SeqCst), 1);
        assert_eq!(capture.creations.load(Ordering::SeqCst), 1);
        assert_eq!(
            capture.saves.load(Ordering::SeqCst),
            0,
            "daemon-owned history must not be overwritten during clear"
        );
        assert!(!app.server_side_turn_active);
        assert!(app.pending_inputs.is_empty());

        server.abort();
    }

    #[tokio::test]
    async fn clear_cancels_active_server_run_and_adopts_created_session() {
        assert_clear_cancels_and_adopts(true).await;
    }

    #[tokio::test]
    async fn clear_cancels_daemon_continuation_without_a_local_run_gate() {
        assert_clear_cancels_and_adopts(false).await;
    }

    #[tokio::test]
    async fn clear_does_not_orphan_new_session_when_server_cancel_fails() {
        let capture = ClearCapture {
            reject_cancel: true,
            ..ClearCapture::default()
        };
        let router = Router::new()
            .route("/api/v1/sessions", post(create_cleared_session))
            .route("/api/v1/sessions/:id", put(accept_save))
            .route("/api/v1/sessions/:id/cancel", post(capture_cancel))
            .with_state(capture.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind failed-clear protocol server");
        let address = listener.local_addr().expect("failed-clear server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve failed-clear protocol server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "active-session".to_string(),
            settings,
        );
        app.committed_messages.push(UIMessage {
            role: MessageRole::User,
            content: "running".to_string(),
            tool_name: None,
            content_collapsed: false,
            tool_collapsed: false,
            tool_running: false,
            tool_args: None,
            diff_data: None,
            tool_metadata: None,
        });
        app.server_side_loop = true;
        app.server_side_turn_active = true;

        app.submit_input("/clear".to_string());

        tokio::time::timeout(Duration::from_secs(2), async {
            while !app
                .committed_messages
                .iter()
                .any(|message| message.content.contains("无法取消当前会话"))
            {
                let event = app
                    .event_rx
                    .recv()
                    .await
                    .expect("app event channel remains open");
                app.handle_event(event).await;
            }
        })
        .await
        .expect("cancel failure must restore the old session");
        assert_eq!(app.session_id, "active-session");
        assert_eq!(capture.cancellations.load(Ordering::SeqCst), 1);
        assert_eq!(capture.creations.load(Ordering::SeqCst), 0);
        assert!(app.server_side_turn_active);
        assert!(app.pending_inputs.is_empty());

        server.abort();
    }

    // ── is_bang_input ──────────────────────────────────────────

    #[test]
    fn is_bang_input_plain_command() {
        assert!(is_bang_input("!ls"));
        assert!(is_bang_input("! ls -la"));
        assert!(is_bang_input("  !echo hi"));
    }

    #[test]
    fn is_bang_input_bare_bang() {
        // Bare `!` is still a bang input (so we can show a usage hint).
        assert!(is_bang_input("!"));
    }

    #[test]
    fn is_bang_input_non_bang() {
        assert!(!is_bang_input("ls"));
        assert!(!is_bang_input("/clear"));
        assert!(!is_bang_input("hello world"));
        assert!(!is_bang_input(""));
    }

    #[test]
    fn is_bang_input_multiline_first_line_wins() {
        // Only the first line matters.
        assert!(is_bang_input("!ls\necho hi"));
        assert!(!is_bang_input("echo hi\n!ls"));
    }

    // ── parse_bang_command ─────────────────────────────────────

    #[test]
    fn parse_bang_no_space() {
        assert_eq!(parse_bang_command("!ls"), Some("ls".to_string()));
    }

    #[test]
    fn parse_bang_with_space() {
        assert_eq!(parse_bang_command("! ls -la"), Some("ls -la".to_string()));
    }

    #[test]
    fn parse_bang_multiple_spaces() {
        assert_eq!(
            parse_bang_command("!  cargo build"),
            Some("cargo build".to_string())
        );
    }

    #[test]
    fn parse_bang_bare_bang_returns_none() {
        assert_eq!(parse_bang_command("!"), None);
    }

    #[test]
    fn parse_bang_only_spaces_after_bang() {
        assert_eq!(parse_bang_command("!   "), None);
    }

    #[test]
    fn parse_bang_non_bang_returns_none() {
        assert_eq!(parse_bang_command("/clear"), None);
        assert_eq!(parse_bang_command("hello"), None);
        assert_eq!(parse_bang_command(""), None);
    }

    #[test]
    fn parse_bang_with_leading_whitespace() {
        // Leading whitespace is stripped by trim_start outside `!`.
        assert_eq!(
            parse_bang_command("  !cargo build"),
            Some("cargo build".to_string())
        );
    }

    // ── format_bang_output ─────────────────────────────────────

    /// Run a trivial shell command and return its Output. Uses real
    /// subprocesses so the tests work cross-platform without requiring
    /// `ExitStatusExt` (which is Unix-only).
    ///
    /// Pins the subprocess cwd to the OS temp dir so a concurrent test that
    /// briefly points the *process* cwd at a dropped TempDir cannot make the
    /// child shell's getcwd() fail (which would pollute stderr and break the
    /// "(no output)" assertion).
    fn shell_output(command: &str) -> std::process::Output {
        crate::sandbox::std_shell_command(command)
            .current_dir(std::env::temp_dir())
            .output()
            .expect("test helper shell command should succeed")
    }

    #[test]
    fn format_bang_output_success_no_stderr_no_exit_line() {
        let output = shell_output("echo hello");
        let result = format_bang_output(&output);
        assert!(result.contains("hello"));
        // 0 exit → no "exit code" line
        assert!(!result.contains("exit code"));
    }

    #[test]
    fn format_bang_output_success_with_stderr() {
        let output = shell_output("echo ok && echo warning >&2");
        let result = format_bang_output(&output);
        assert!(result.contains("ok"));
        assert!(result.contains("[stderr]"));
        assert!(result.contains("warning"));
        assert!(!result.contains("exit code")); // 0 exit → no exit line
    }

    #[test]
    fn format_bang_output_failure_shows_exit_code() {
        // `false` exits with code 1, no stdout.
        let output = shell_output("false");
        let result = format_bang_output(&output);
        assert!(result.contains("[exit code 1]"));
    }

    #[test]
    fn format_bang_output_success_no_output() {
        // `true` exits 0 with no stdout or stderr.
        let output = shell_output("true");
        let result = format_bang_output(&output);
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn format_bang_output_failure_with_stderr() {
        // Command that fails and writes to stderr.
        let output = shell_output("echo error msg >&2 && false");
        let result = format_bang_output(&output);
        assert!(result.contains("[stderr]"));
        assert!(result.contains("error msg"));
        assert!(result.contains("[exit code 1]"));
    }
}
