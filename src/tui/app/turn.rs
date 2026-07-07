//! Turn lifecycle — starting, spawning, and cancelling agent turns.

use super::types::*;
use super::App;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::agent::{AgentError, AgentLoop};
use crate::tui::util::truncate_session_name;

impl App {
    /// Start the next pending turn (if any).
    pub(super) fn start_next_turn(&mut self) {
        if let Some(pending) = self.pending_inputs.pop_front() {
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
        let (planner_client, max_rounds, subagent_timeout_secs) = {
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
            )
        };
        let token_counter = self.token_counter.clone();
        let hook_manager = self.hook_manager.clone();
        let prompt_context = self.prompt_context.clone();
        self.current_turn_handle = Some(tokio::spawn(async move {
            let mut agent = AgentLoop::new(
                client,
                event_tx.clone(),
                session_id,
                history,
                sys_msgs,
                plan_mode,
                planner_client,
                max_rounds,
                token_counter,
                hook_manager,
                prompt_context,
                subagent_timeout_secs,
            );
            let result = agent.process_input(input_text).await;
            if let Err(ref e) = result {
                let reason = match e {
                    AgentError::StreamTimeout(_) => TurnAbortReason::TimedOut,
                    AgentError::MaxRoundsExceeded { .. }
                    | AgentError::TokenBudgetExhausted { .. } => TurnAbortReason::MaxRoundsExceeded,
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
        let (max_rounds, subagent_timeout_secs) = {
            let s = self.settings_lock.read().unwrap();
            (
                s.agent.max_rounds.unwrap_or(100),
                s.agent.subagent.timeout_secs,
            )
        };
        let token_counter = self.token_counter.clone();
        let hook_manager = self.hook_manager.clone();
        let prompt_context = self.prompt_context.clone();
        self.current_turn_handle = Some(tokio::spawn(async move {
            let mut agent = AgentLoop::new(
                client,
                event_tx.clone(),
                session_id,
                history,
                sys_msgs,
                false,
                None,
                max_rounds,
                token_counter,
                hook_manager,
                prompt_context,
                subagent_timeout_secs,
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
