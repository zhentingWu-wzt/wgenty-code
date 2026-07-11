//! Input submission — slash commands and normal user input.

use super::types::*;
use super::{App, PendingInput};
use crate::api::ChatMessage;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason};

impl App {
    fn push_system_message(&mut self, content: impl Into<String>) {
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
            self.cancel_current_turn();
            // Reset phase immediately and suppress stale events from the
            // just-aborted turn so the status bar shows "Ready" instead of
            // lingering on "Thinking". Cleared when a new turn starts.
            self.phase = AgentPhase::Idle;
            self.suppress_phase_updates = true;
            let history = self.conversation_history.clone();
            let sys_msgs = self.assembled_system_messages.clone();
            tokio::spawn(async move {
                let mut h = history.lock().await;
                *h = sys_msgs;
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
    /// `sh -c` in the current working directory with a 120s timeout. The
    /// result is delivered back to the UI as a `BackgroundTaskResult` system
    /// message (the existing channel for background-task notifications), so no
    /// new event variant or render branch is needed.
    fn run_bang_command(&mut self, command: String) {
        // Immediate feedback so the user sees the command was accepted. The
        // command line is shown here once; the result message below carries
        // only the output (stdout/stderr/exit) to avoid duplication.
        self.push_system_message(format!("$ {}", command));

        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let cwd = std::env::current_dir().unwrap_or_default();
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(BANG_COMMAND_TIMEOUT_SECS),
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(&cwd)
                    .output(),
            )
            .await;

            let message = match result {
                Ok(Ok(output)) => format_bang_output(&output),
                Ok(Err(e)) => format!("Failed to execute command: {}", e),
                Err(_) => format!(
                    "Command timed out ({}s)",
                    BANG_COMMAND_TIMEOUT_SECS
                ),
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
