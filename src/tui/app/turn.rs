//! Turn lifecycle — starting, spawning, and cancelling agent turns.

use super::types::*;
use super::App;
use crate::api::ChatMessage;
use crate::config::resolve_context_window;
use crate::context::inject::MemoryContextInjector;
use crate::context::TurnRecord;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::agent::{AgentError, AgentLoop};
use crate::tui::util::truncate_session_name;

impl App {
    /// Start the next pending turn (if any).
    pub(super) fn start_next_turn(&mut self) {
        if let Some(pending) = self.pending_inputs.pop_front() {
            if pending.is_continuation() {
                // Synthetic continuation: inject the delivered child results
                // as a `user` message with no visible user row.
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
        // Record this turn's metadata for the /undo interactive rollback flow.
        // checkpoint_turn_id defaults to turn_id until Task 4 wires up the
        // checkpoint manifest.
        self.record_turn_start(turn_id.to_string(), &input_text, turn_id.to_string());
        let history = self.conversation_history.clone();
        let client = self.daemon_client.clone();
        let event_tx = self.event_tx.clone();
        let session_id = self.session_id.clone();
        let sys_msgs = self.assembled_instructions.system_messages.clone();
        let plan_mode = self.mode == AgentMode::PlanMode;
        // Read agent config from settings
        let (
            planner_client,
            max_rounds,
            subagent_timeout_secs,
            context_window,
            max_tokens,
            debug_dump_reminder,
        ) = {
            let s = self.settings_lock.read().expect("lock poisoned: settings");
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
                resolve_context_window(&s.models.main, s.models.context_window),
                s.models.transport.max_tokens,
                crate::prompts::reminder_dump_enabled(s.prompt.debug_dump_reminder),
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
            let s = self.settings_lock.read().expect("lock poisoned: settings");
            s.storage.memory.recall_top_n
        };

        // Capture pre-turn layer metadata for the turn-context inspector.
        let pre_turn_layers = self.assembled_instructions.layers.clone();
        let turn_idx = self.turn_count;

        // Clone conversation_history Arc a second time so we can read it
        // after the first clone is moved into AgentLoop::new.
        let history_for_snapshot = self.conversation_history.clone();

        self.current_turn_handle = Some(tokio::spawn(async move {
            // Per-turn recall: use MemoryContextInjector for keyword extraction
            // and TF-IDF search over cross-session memories.
            let recalled_text = MemoryContextInjector::recall(
                &input_agent,
                &memory_manager,
                recall_top_n,
                // Use the importance threshold from settings for filtering.
                0.5,
                None,
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
                debug_dump_reminder,
            );
            let result = agent.process_input(input_agent).await;
            // Build TurnContext snapshot for the inspector.
            {
                let sys_msgs_vec: Vec<ChatMessage> = pre_turn_layers
                    .iter()
                    .map(|l| ChatMessage::system(l.content.clone()))
                    .collect();
                let history_msgs: Vec<ChatMessage> = {
                    let lock = history_for_snapshot.lock().await;
                    lock.clone()
                };
                let full_messages: Vec<ChatMessage> =
                    sys_msgs_vec.into_iter().chain(history_msgs).collect();
                let turn_ctx = TurnContext {
                    turn_index: turn_idx,
                    layers: pre_turn_layers.clone(),
                    memories: vec![],
                    reminder: None,
                    full_messages,
                };
                let _ = event_tx.send(AppEvent::TurnContextCaptured(turn_ctx));
            }
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
    /// structured `user` message inside `process_continuation`.
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
        let sys_msgs = self.assembled_instructions.system_messages.clone();
        let plan_mode = self.mode == AgentMode::PlanMode;
        let (
            planner_client,
            max_rounds,
            subagent_timeout_secs,
            context_window,
            max_tokens,
            debug_dump_reminder,
        ) = {
            let s = self.settings_lock.read().expect("lock poisoned: settings");
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
                resolve_context_window(&s.models.main, s.models.context_window),
                s.models.transport.max_tokens,
                crate::prompts::reminder_dump_enabled(s.prompt.debug_dump_reminder),
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
                debug_dump_reminder,
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
        let sys_msgs = self.assembled_instructions.system_messages.clone();
        let (max_rounds, subagent_timeout_secs, context_window, max_tokens) = {
            let s = self.settings_lock.read().expect("lock poisoned: settings");
            (
                s.agent.max_rounds.unwrap_or(100),
                s.agent.subagent.timeout_secs,
                resolve_context_window(&s.models.main, s.models.context_window),
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
                false,
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

    /// Interrupt the running turn from a user keypress (ESC).
    ///
    /// Finalizes visible streaming/tool state, aborts the turn task and any
    /// daemon-side subagents, then surfaces an "Interrupted by user" system
    /// message. Unlike `/clear` (which wipes the conversation), already-
    /// generated partial output is preserved. `/clear` still calls
    /// `cancel_current_turn` directly, so its clean-slate semantics are
    /// unaffected.
    pub(super) fn interrupt_running_turn(&mut self) {
        // Commit partial streamed content as an Assistant message so it stays
        // visible after streaming is turned off (the chat only renders
        // streaming_content while streaming_active is true). Mirrors StreamDone.
        let content = std::mem::take(&mut self.streaming_content);
        let is_hint = content.starts_with('\u{23F3}');
        if !content.is_empty() && !is_hint {
            self.committed_messages.push(UIMessage {
                role: MessageRole::Assistant,
                content,
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: true,
                tool_running: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
        }
        self.streaming_active = false;
        // Stop the tool spinner and finalize a running tool placeholder so it
        // does not show as perpetually running after the abort.
        self.has_running_tool = false;
        if let Some(last) = self.committed_messages.last_mut() {
            if last.role == MessageRole::Tool && last.tool_running {
                last.tool_running = false;
                last.tool_collapsed = true;
            }
        }
        // Abort the turn task (phase -> Idle, suppress stale phase updates,
        // emit TurnAborted::Interrupted).
        self.cancel_current_turn();
        // Cancel daemon-side subagents belonging to this turn by advancing the
        // agent generation, mirroring /clear. The next turn adopts the fresh
        // generation returned asynchronously.
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
                        "reset_agent_generation failed during interrupt; retaining old generation"
                    );
                    let _ = event_tx.send(AppEvent::AgentGenerationReset {
                        generation: u64::MAX,
                    });
                }
            }
        });
        // User-facing feedback.
        self.push_system_message("\u{23F9} Interrupted by user");
        // Persist interrupted UI + history. Save path sanitizes unpaired
        // tool_calls under the session lock (abort may skip TurnComplete).
        // Exit flush is a second safety net if the user quits immediately.
        self.spawn_save_session();
    }

    /// Number of inputs waiting in the queue (excluding the running one).
    pub(super) fn pending_count(&self) -> usize {
        self.pending_inputs.len()
    }

    /// Record the start of a new turn by pushing a [`TurnRecord`] onto
    /// `turn_records`.  Called from `spawn_agent_turn` right after the
    /// `turn_id` is created and before `tokio::spawn`.
    ///
    /// `checkpoint_turn_id` identifies the pre-edit file checkpoint for this
    /// turn (used by code-rollback).  Until Task 4 wires up the checkpoint
    /// manifest it defaults to the turn_id itself.
    pub(super) fn record_turn_start(
        &mut self,
        turn_id: String,
        input: &str,
        checkpoint_turn_id: String,
    ) {
        let record = TurnRecord {
            turn_id: turn_id.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            user_summary: first_sentence(input),
            checkpoint_turn_id,
            message_end_idx: 0,
            // TODO Task 4: from checkpoint manifest
            file_count: 0,
        };
        self.turn_records.push(record);
    }

    /// Finalize the most recent `TurnRecord` by setting its `message_end_idx`.
    /// Called from the `AppEvent::TurnComplete` handler with
    /// `conversation_history.len()`.  No-op if `turn_records` is empty or the
    /// last entry already has a non-zero `message_end_idx` (guards against a
    /// duplicate `TurnComplete` event clobbering the real value).
    pub(super) fn finalize_turn_end(&mut self, message_end_idx: usize) {
        if let Some(last) = self.turn_records.last_mut() {
            if last.message_end_idx == 0 {
                last.message_end_idx = message_end_idx;
            }
        }
    }
}

/// Extract the first sentence of `input` for display in the turn-picker
/// popup.  A sentence ends at `.`, `!`, or `?` followed by whitespace (or
/// end of string).  If no boundary is found the whole trimmed input is
/// returned, capped at 100 characters with an ellipsis.
fn first_sentence(input: &str) -> String {
    let trimmed = input.trim();
    for (i, ch) in trimmed.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let after = &trimmed[i + ch.len_utf8()..];
            if after.is_empty() || after.starts_with(char::is_whitespace) {
                return trimmed[..i + ch.len_utf8()].to_string();
            }
        }
        if ch == '\n' {
            return trimmed[..i].trim_end().to_string();
        }
    }
    // No sentence boundary found: cap at 100 chars to keep the popup readable.
    let capped: String = trimmed.chars().take(100).collect();
    if capped.chars().count() < trimmed.chars().count() {
        format!("{capped}\u{2026}")
    } else {
        capped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::watcher::SettingsHandle;
    use crate::config::Settings;
    use crate::tui::client::DaemonClient;
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    fn build_app() -> App {
        let client = DaemonClient::new("http://localhost:0".to_string());
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        App::new(client, "test-interrupt".to_string(), settings)
    }

    #[tokio::test]
    async fn interrupt_running_turn_commits_partial_and_resets_state() {
        let mut app = build_app();
        app.streaming_content = "partial response".to_string();
        app.streaming_active = true;
        app.current_turn_handle = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));

        app.interrupt_running_turn();

        assert!(!app.streaming_active, "streaming should be inactive");
        assert!(app.streaming_content.is_empty(), "streaming buffer cleared");
        assert!(app.current_turn_handle.is_none(), "turn handle cleared");
        assert_eq!(app.phase, AgentPhase::Idle);
        assert!(app.suppress_phase_updates, "phase updates suppressed");
        assert!(
            app.committed_messages
                .iter()
                .any(|m| m.role == MessageRole::Assistant && m.content == "partial response"),
            "partial content committed as Assistant message"
        );
        assert!(
            app.committed_messages
                .iter()
                .any(|m| m.content.contains("Interrupted by user")),
            "interrupt feedback message present"
        );
    }

    #[tokio::test]
    async fn interrupt_running_turn_skips_preparing_hint() {
        let mut app = build_app();
        app.streaming_content = "\u{23F3} preparing tools...".to_string();
        app.streaming_active = true;
        app.current_turn_handle = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));

        app.interrupt_running_turn();

        assert!(
            !app.committed_messages.iter().any(|m| {
                m.role == MessageRole::Assistant && m.content.contains("preparing tools")
            }),
            "preparing-tools hint should not be committed as Assistant content"
        );
        assert!(!app.streaming_active);
    }

    // ── TurnRecord tracking (Task 2: /undo interactive rollback) ─────────

    #[tokio::test]
    async fn record_turn_start_pushes_correct_turn_record() {
        let mut app = build_app();
        assert!(app.turn_records.is_empty(), "starts empty");

        app.record_turn_start(
            "turn-abc".to_string(),
            "Hello world. This is a longer second sentence.",
            "turn-abc".to_string(),
        );

        assert_eq!(app.turn_records.len(), 1, "one record pushed");
        let rec = &app.turn_records[0];
        assert_eq!(rec.turn_id, "turn-abc");
        assert!(!rec.created_at.is_empty(), "created_at populated");
        assert_eq!(
            rec.user_summary, "Hello world.",
            "user_summary is the first sentence"
        );
        assert_eq!(rec.checkpoint_turn_id, "turn-abc");
        assert_eq!(rec.message_end_idx, 0, "message_end_idx starts at 0");
        assert_eq!(rec.file_count, 0, "file_count starts at 0");
    }

    #[tokio::test]
    async fn record_turn_start_handles_input_without_sentence_boundary() {
        let mut app = build_app();
        app.record_turn_start(
            "turn-1".to_string(),
            "just some words with no period",
            "turn-1".to_string(),
        );
        let rec = &app.turn_records[0];
        assert_eq!(
            rec.user_summary, "just some words with no period",
            "no boundary → whole input (capped)"
        );
    }

    #[tokio::test]
    async fn record_turn_start_accumulates_multiple_turns() {
        let mut app = build_app();
        app.record_turn_start("t1".to_string(), "First turn.", "t1".to_string());
        app.record_turn_start("t2".to_string(), "Second turn.", "t2".to_string());
        assert_eq!(app.turn_records.len(), 2);
        assert_eq!(app.turn_records[0].turn_id, "t1");
        assert_eq!(app.turn_records[1].turn_id, "t2");
    }

    #[tokio::test]
    async fn finalize_turn_end_sets_message_end_idx_on_last_record() {
        let mut app = build_app();
        app.record_turn_start("t1".to_string(), "Hello.", "t1".to_string());
        assert_eq!(app.turn_records[0].message_end_idx, 0);

        app.finalize_turn_end(42);

        assert_eq!(
            app.turn_records[0].message_end_idx, 42,
            "message_end_idx updated"
        );
    }

    #[tokio::test]
    async fn finalize_turn_end_does_not_overwrite_nonzero_end_idx() {
        let mut app = build_app();
        app.record_turn_start("t1".to_string(), "Hello.", "t1".to_string());
        app.finalize_turn_end(10);
        // A second TurnComplete should not clobber the already-set value.
        app.finalize_turn_end(99);
        assert_eq!(app.turn_records[0].message_end_idx, 10);
    }

    #[tokio::test]
    async fn finalize_turn_end_no_panic_on_empty_records() {
        let mut app = build_app();
        // Should be a no-op, not a panic.
        app.finalize_turn_end(5);
        assert!(app.turn_records.is_empty());
    }
}
