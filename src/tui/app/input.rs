//! Input submission — slash commands and normal user input.

use super::types::*;
use super::App;
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
        // Slash commands
        if text.trim() == "/clear" {
            self.committed_messages.clear();
            self.streaming_content.clear();
            self.streaming_active = false;
            self.scroll_offset = 0;
            self.user_scrolled = false;
            self.cancel_current_turn();
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
                self.pending_inputs
                    .push_back("Continue the current task from where you left off.".to_string());
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
            self.pending_inputs
                .push_back("undo the most recent operation".to_string());
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
        if text.trim() == "/help" {
            let commands = crate::tui::completion::CompletionEngine::default_builtin_commands()
                .into_iter()
                .map(|command| format!("/{} — {}", command.name, command.description))
                .collect::<Vec<_>>()
                .join("\n");
            self.push_system_message(format!("Available commands:\n{}", commands));
            return;
        }

        // Fire UserPromptSubmit hook asynchronously (after built-in checks,
        // before slash routing and normal input processing).
        {
            let hm = self.hook_manager.clone();
            let input_text = text.clone();
            let sid = self.session_id.clone();
            tokio::spawn(async move {
                let cwd = std::env::current_dir().unwrap_or_default();
                let comet_phase = crate::comet::CometState::read(&cwd)
                    .map(|s| format!("{:?}", s.phase).to_lowercase());
                let ctx = crate::hooks::HookContext {
                    event: "UserPromptSubmit".to_string(),
                    tool_name: None,
                    tool_input: Some(serde_json::Value::String(input_text)),
                    tool_result: None,
                    session_id: Some(sid),
                    working_directory: std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    comet_phase,
                };
                hm.fire(&crate::hooks::HookEvent::UserPromptSubmit, &ctx, None)
                    .await;
            });
        }

        // Route unrecognized slash commands via route_slash_command (I1).
        // This catches external skill invocations like /comet or /verify
        // that are not handled by the built-in checks above.
        let trimmed = text.trim();
        if trimmed.starts_with('/') {
            let builtins = &["clear", "plan", "continue", "undo", "init", "help"];
            let route = crate::knowledge::route_slash_command(
                &text,
                builtins,
                self.external_skill_registry.as_ref().map(|r| r.as_ref()),
            );
            match route {
                crate::knowledge::SlashRoute::ExternalSkill { skill, args } => {
                    let agent_input = crate::knowledge::comet_slash_agent_prompt(&skill, &args)
                        .unwrap_or_else(|| text.clone());
                    let is_comet = agent_input != text;
                    let msg = format!(
                        "🔧 External skill '/{}' detected. {}",
                        if args.is_empty() {
                            skill
                        } else {
                            format!("{} {}", skill, args)
                        },
                        if is_comet {
                            "Sending OpenSpec-aware Comet workflow to agent."
                        } else {
                            "Sending to agent for invocation."
                        }
                    );
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::System,
                        content: msg,
                        tool_name: None,
                        content_collapsed: false,
                        tool_collapsed: false,
                        tool_running: false,
                        tool_args: None,
                        diff_data: None,
                        tool_metadata: None,
                    });
                    // Queue the routed input for the agent to process.
                    self.pending_inputs.push_back(agent_input);
                    if self.current_turn_handle.is_none() {
                        self.start_next_turn();
                    }
                    return;
                }
                crate::knowledge::SlashRoute::Unknown {
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
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::System,
                        content: msg,
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
                // BuiltIn — already handled above, NotSlash — can't happen here
                _ => {}
            }
        }
        if self.mode == AgentMode::PlanMode {
            self.phase = AgentPhase::Thinking;
            self.pending_inputs.push_back(text);
            self.start_next_turn();
            // PlanMode now persists across turns — the agent detects plan
            // confirmation replies and skips re-planning automatically.
            return;
        }
        self.pending_inputs.push_back(text);
        if self.current_turn_handle.is_none() {
            self.start_next_turn();
        }
    }
}
