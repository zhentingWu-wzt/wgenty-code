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
            self.committed_messages.clear();
            self.streaming_content.clear();
            self.streaming_active = false;
            self.scroll_offset = 0;
            self.user_scrolled = false;
            self.sandbox_bypassed_session = false;
            self.cancel_current_turn();
            // Reset phase immediately and suppress stale events from the
            // just-aborted turn so the status bar shows "Ready" instead of
            // lingering on "Thinking". Cleared when a new turn starts.
            self.phase = AgentPhase::Idle;
            self.suppress_phase_updates = true;
            let history = self.conversation_history.clone();
            tokio::spawn(async move {
                let mut h = history.lock().await;
                // Dialogue-only: system layers live in assembled_system_messages
                // and are prepended each API round — never stored in history.
                h.clear();
            });
            // Clear queued inputs: a fresh generation cancels obsolete work.
            self.pending_inputs.clear();
            // Atomically advance the task generation on the daemon (which
            // cancels obsolete root-direct subtrees) and adopt the new
            // generation when it returns. Until it completes, stale
            // subagent views are suppressed and no queued work starts.
            let client = self.daemon_client.clone();
            let session_id = self.session_id.clone();
            let event_tx = self.event_tx.clone();
            tokio::spawn(async move {
                match client.reset_agent_generation(&session_id).await {
                    Ok(generation) => {
                        let _ = event_tx.send(AppEvent::AgentGenerationReset { generation });
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
                if self.current_turn_handle.is_none() {
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
        if text.trim() == "/undo" {
            self.committed_messages.push(UIMessage {
                role: MessageRole::System,
                content: "Undo requested".to_string(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_running: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            self.pending_inputs.push_back(PendingInput::new(
                "undo the most recent operation".to_string(),
            ));
            if self.current_turn_handle.is_none() {
                self.start_next_turn();
            }
            return;
        }
        if text.trim() == "/init" {
            self.push_system_message(
                "🔄 Running /init — 正在分析代码库以生成 WGENTY.md 和 AGENTS.md...",
            );
            if self.current_turn_handle.is_none() {
                let init_prompt = crate::prompts::get_init_prompt().to_string();
                self.spawn_agent_turn(init_prompt, true);
            }
            return;
        }
        if text.trim() == "/compact" {
            if self.current_turn_handle.is_some() {
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
                        if self.current_turn_handle.is_none() {
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
        if self.current_turn_handle.is_none() {
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
    fn shell_output(command: &str) -> std::process::Output {
        crate::sandbox::std_shell_command(command)
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
