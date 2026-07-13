//! Turn lifecycle — starting, spawning, and cancelling agent turns.

use super::types::*;
use super::App;
use crate::context::inject::MemoryContextInjector;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::agent::{AgentError, AgentLoop};
use crate::tui::util::truncate_session_name;

impl App {
    /// Start the next pending turn (if any).
    pub(super) fn start_next_turn(&mut self) {
        if let Some(pending) = self.pending_inputs.pop_front() {
            if pending.is_continuation() {
                // Synthetic continuation: inject the delivered child results
                // as a system message with no visible user row.
                let delivery = pending
                    .continuation
                    .clone()
                    .expect("continuation pending input carries a delivery");
                self.spawn_continuation_turn(delivery);
                return;
            }
            // Push user message to UI immediately
            self.committed_messages.push(UIMessage {
                role: MessageRole::User,
                content: pending.display_text.clone(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_running: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            // Auto-name the session from the first user message
            if self.session_name == "New Session" {
                let name = truncate_session_name(&pending.display_text);
                self.session_name = name;
            }
            self.spawn_agent_turn(pending.agent_input, false);
        }
    }

    /// Spawn an agent turn with `input_text` as the initial user message.
    /// When `hide_input` is true, the input is not displayed as a user message
    /// in the chat (used for internal prompts like /init).
    pub(super) fn spawn_agent_turn(&mut self, input_text: String, hide_input: bool) {
        if hide_input {
            // Auto-name session from a short label instead of the full prompt
            if self.session_name == "New Session" {
                self.session_name = "Init Project".to_string();
            }
        } else if self.session_name == "New Session" {
            let name = truncate_session_name(&input_text);
            self.session_name = name;
        }
        self.phase = AgentPhase::Thinking;
        // New turn: stop suppressing phase updates (set by /clear or cancel).
        self.suppress_phase_updates = false;
        let turn_id = TurnId::new();
        self.current_turn_id = Some(turn_id.clone());
        let _ = self.event_tx.send(AppEvent::TurnStarted {
            turn_id: turn_id.clone(),
        });
        let history = self.conversation_history.clone();
        let client = self.daemon_client.clone();
        let event_tx = self.event_tx.clone();
        let session_id = self.session_id.clone();
        let sys_msgs = self.assembled_system_messages.clone();
        let plan_mode = self.mode == AgentMode::PlanMode;
        // Read agent config from settings
        let (planner_client, max_rounds, subagent_timeout_secs, context_window, max_tokens) = {
            let s = self.settings_lock.read().unwrap();
            let planner = if let Some(ref pm) = s.models.planner {
                let mut planner_settings = s.clone();
                planner_settings.models.main.name = pm.name.clone();
                if let Some(ref url) = pm.base_url {
                    planner_settings.models.main.base_url = Some(url.clone());
                }
                if let Some(ref key) = pm.api_key {
                    planner_settings.models.main.api_key = Some(key.clone());
                }
                Some(crate::api::ApiClient::new(planner_settings))
            } else {
                None
            };
            (
                planner,
                s.agent.max_rounds.unwrap_or(100),
                s.agent.subagent.timeout_secs,
                s.models.context_window,
                s.models.transport.max_tokens,
            )
        };
        let token_counter = self.token_counter.clone();
        let hook_manager = self.hook_manager.clone();
        let prompt_context = self.prompt_context.clone();
        let memory_manager = self.memory_manager.clone();
        let agent_generation = self.agent_generation;
        let input_agent = input_text.clone();
        let turn_id_for_loop = turn_id.clone();

        // Per-turn smart memory recall — runs inside the tokio task.
        let recall_top_n = {
            let s = self.settings_lock.read().unwrap();
            s.storage.memory.recall_top_n
        };

        self.current_turn_handle = Some(tokio::spawn(async move {
            // Per-turn recall: use MemoryContextInjector for keyword extraction
            // and TF-IDF search over cross-session memories.
            let recalled_text = MemoryContextInjector::recall(
                &input_agent,
                &memory_manager,
                recall_top_n,
                // Use the importance threshold from settings for filtering.
                0.5,
            )
            .await;

            // Set memories on PromptContext (extract lines from the
            // <memory-context> block for the prompt builder).
            let prompt_context = {
                let mut ctx = (*prompt_context).clone();
                if !recalled_text.is_empty() {
                    ctx.memories = recalled_text
                        .lines()
                        .filter(|l| {
                            !l.trim().is_empty()
                                && !l.contains("<memory-context>")
                                && !l.contains("</memory-context>")
                        })
                        .map(|l| l.to_string())
                        .collect();
                }
                std::sync::Arc::new(ctx)
            };
            let mut agent = AgentLoop::new(
                client,
                event_tx.clone(),
                session_id,
                Some(turn_id_for_loop.to_string()),
                history,
                sys_msgs,
                plan_mode,
                planner_client,
                max_rounds,
                token_counter,
                hook_manager,
                prompt_context,
                subagent_timeout_secs,
                context_window,
                max_tokens,
                memory_manager,
                agent_generation,
            );
            let result = agent.process_input(input_agent).await;
            if let Err(ref e) = result {
                let reason = match e {
                    AgentError::StreamTimeout(_) => TurnAbortReason::TimedOut,
                    AgentError::MaxRoundsExceeded { .. } => TurnAbortReason::MaxRoundsExceeded,
                    AgentError::StreamError(_)
                    | AgentError::PlannerError(_)
                    | AgentError::EmptyResponse => TurnAbortReason::StreamError,
                };
                let _ = event_tx.send(AppEvent::TurnAborted { reason });
            }
            let _ = event_tx.send(AppEvent::TurnComplete);
        }));
    }

    /// Spawn a synthetic continuation turn that consumes a claimed task-group
    /// delivery. No visible user row is added; the delivery is injected as a
    /// structured system message inside `process_continuation`.
    pub(super) fn spawn_continuation_turn(
        &mut self,
        delivery: crate::tui::client::TaskGroupDeliveryResponse,
    ) {
        self.phase = AgentPhase::Thinking;
        self.suppress_phase_updates = false;
        let turn_id = TurnId::new();
        self.current_turn_id = Some(turn_id.clone());
        let _ = self.event_tx.send(AppEvent::TurnStarted {
            turn_id: turn_id.clone(),
        });
        let history = self.conversation_history.clone();
        let client = self.daemon_client.clone();
        let event_tx = self.event_tx.clone();
        let session_id = self.session_id.clone();
        let sys_msgs = self.assembled_system_messages.clone();
        let plan_mode = self.mode == AgentMode::PlanMode;
        let (planner_client, max_rounds, subagent_timeout_secs, context_window, max_tokens) = {
            let s = self.settings_lock.read().unwrap();
            let planner = if let Some(ref pm) = s.models.planner {
                let mut planner_settings = s.clone();
                planner_settings.models.main.name = pm.name.clone();
                if let Some(ref url) = pm.base_url {
                    planner_settings.models.main.base_url = Some(url.clone());
                }
                if let Some(ref key) = pm.api_key {
                    planner_settings.models.main.api_key = Some(key.clone());
                }
                Some(crate::api::ApiClient::new(planner_settings))
            } else {
                None
            };
            (
                planner,
                s.agent.max_rounds.unwrap_or(100),
                s.agent.subagent.timeout_secs,
                s.models.context_window,
                s.models.transport.max_tokens,
            )
        };
        let token_counter = self.token_counter.clone();
        let hook_manager = self.hook_manager.clone();
        let prompt_context = self.prompt_context.clone();
        let memory_manager = self.memory_manager.clone();
        let agent_generation = self.agent_generation;
        let turn_id_for_loop = turn_id.clone();

        self.current_turn_handle = Some(tokio::spawn(async move {
            let prompt_context = std::sync::Arc::new((*prompt_context).clone());
            let mut agent = AgentLoop::new(
                client,
                event_tx.clone(),
                session_id,
                Some(turn_id_for_loop.to_string()),
                history,
                sys_msgs,
                plan_mode,
                planner_client,
                max_rounds,
                token_counter,
                hook_manager,
                prompt_context,
                subagent_timeout_secs,
                context_window,
                max_tokens,
                memory_manager,
                agent_generation,
            );
            let result = agent.process_continuation(delivery).await;
            if let Err(ref e) = result {
                let reason = match e {
                    AgentError::StreamTimeout(_) => TurnAbortReason::TimedOut,
                    AgentError::MaxRoundsExceeded { .. } => TurnAbortReason::MaxRoundsExceeded,
                    AgentError::StreamError(_)
                    | AgentError::PlannerError(_)
                    | AgentError::EmptyResponse => TurnAbortReason::StreamError,
                };
                let _ = event_tx.send(AppEvent::TurnAborted { reason });
            }
            let _ = event_tx.send(AppEvent::TurnComplete);
        }));
    }

    /// Spawn a compaction-only turn (user pressed `/compact`). Archives the
    /// transcript and replaces history with a summary, without generating an
    /// LLM response. Reuses the same `AgentLoop` construction as
    /// `spawn_agent_turn` but calls `compact_only` instead of `process_input`.
    pub(super) fn spawn_compact_turn(&mut self) {
        self.phase = AgentPhase::Compacting;
        // New turn: stop suppressing phase updates (set by /clear or cancel).
        self.suppress_phase_updates = false;
        let turn_id = TurnId::new();
        self.current_turn_id = Some(turn_id.clone());
        let _ = self.event_tx.send(AppEvent::TurnStarted {
            turn_id: turn_id.clone(),
        });
        let history = self.conversation_history.clone();
        let client = self.daemon_client.clone();
        let event_tx = self.event_tx.clone();
        let session_id = self.session_id.clone();
        let sys_msgs = self.assembled_system_messages.clone();
        let (max_rounds, subagent_timeout_secs, context_window, max_tokens) = {
            let s = self.settings_lock.read().unwrap();
            (
                s.agent.max_rounds.unwrap_or(100),
                s.agent.subagent.timeout_secs,
                s.models.context_window,
                s.models.transport.max_tokens,
            )
        };
        let token_counter = self.token_counter.clone();
        let hook_manager = self.hook_manager.clone();
        let prompt_context = self.prompt_context.clone();
        // Inject startup memories into PromptContext.
        let prompt_context = {
            let startup = &self.startup_memories;
            if startup.is_empty() {
                prompt_context
            } else {
                let mut ctx = (*prompt_context).clone();
                ctx.memories = startup.clone();
                std::sync::Arc::new(ctx)
            }
        };
        let memory_manager = self.memory_manager.clone();
        let agent_generation = self.agent_generation;
        self.current_turn_handle = Some(tokio::spawn(async move {
            let mut agent = AgentLoop::new(
                client,
                event_tx.clone(),
                session_id,
                None,
                history,
                sys_msgs,
                false,
                None,
                max_rounds,
                token_counter,
                hook_manager,
                prompt_context,
                subagent_timeout_secs,
                context_window,
                max_tokens,
                memory_manager,
                agent_generation,
            );
            let _ = agent.compact_only().await;
            let _ = event_tx.send(AppEvent::TurnComplete);
        }));
    }

    /// Cancel the current turn and flush all queued input.
    pub(super) fn cancel_current_turn(&mut self) {
        self.pending_inputs.clear();
        if let Some(handle) = self.current_turn_handle.take() {
            handle.abort();
            // Set phase to Idle immediately and suppress stale phase updates
            // from the aborted task's in-flight events (e.g. StreamDone,
            // ToolResult) that would otherwise override Idle back to Thinking.
            self.phase = AgentPhase::Idle;
            self.suppress_phase_updates = true;
            let _ = self.event_tx.send(AppEvent::TurnAborted {
                reason: TurnAbortReason::Interrupted,
            });
        }
        self.current_turn_id = None;
    }

    /// Number of inputs waiting in the queue (excluding the running one).
    pub(super) fn pending_count(&self) -> usize {
        self.pending_inputs.len()
    }
}
