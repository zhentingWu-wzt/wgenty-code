//! Turn lifecycle — starting, spawning, and cancelling agent turns.

use super::types::*;
use super::App;
use crate::api::ChatMessage;
use crate::config::resolve_context_window;
use crate::context::inject::MemoryContextInjector;
use crate::context::TurnRecord;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::agent::{AgentError, AgentLoop};
use crate::tui::components::turn_picker::TurnPickerState;
use crate::tui::components::undo_scope_picker::UndoScopePickerState;
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
            if pending.hidden {
                self.spawn_hidden_agent_turn(pending.agent_input);
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

    /// Start a synthetic turn without rendering it or using its input as the
    /// session title. Background results use this path.
    fn spawn_hidden_agent_turn(&mut self, input_text: String) {
        self.spawn_agent_turn_inner(input_text, true, false);
    }

    /// Spawn an agent turn with `input_text` as the initial user message.
    /// When `hide_input` is true, the input is not displayed as a user message
    /// in the chat (used for internal prompts like /init).
    /// Server-side turn: POST /run and let the SSE reader render events.
    /// The daemon owns the loop (LLM + tools + history); TUI only observes.
    pub(super) fn start_server_side_run(&mut self, input_text: String, hide_input: bool) {
        if !hide_input {
            // Push user message optimistically for rendering (daemon owns history).
            self.committed_messages.push(UIMessage {
                role: MessageRole::User,
                content: input_text.clone(),
                tool_name: None,
                tool_args: None,
                content_collapsed: false,
                tool_collapsed: true,
                tool_running: false,
                diff_data: None,
                tool_metadata: None,
            });
        }
        let client = self.daemon_client.clone();
        let sid = self.session_id.clone();
        let tx = self.event_tx.clone();
        let plan_mode = self.mode == AgentMode::PlanMode;
        let name = self.session_name.clone();
        let history = self.conversation_history.clone();
        let ui_messages: Vec<_> = self
            .committed_messages
            .iter()
            .map(UIMessage::to_session_ui_message)
            .collect();
        // (Re)subscribe the session-event SSE reader for the current session
        // id. `ready` fires once the subscription is established so the run
        // below doesn't miss its live-only events (mirrors the web client's
        // subscribe-then-run ordering).
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        self.respawn_session_event_reader(Some(ready_tx));
        self.respawn_trace_event_reader();
        self.server_side_turn_active = true;
        tokio::spawn(async move {
            // The daemon 404s unknown session ids on both POST /run and
            // GET /events. The TUI's session id is generated locally (not
            // POSTed at startup to avoid flooding the session panel), so
            // upsert it first via PUT — this creates the session record.
            let h = { history.lock().await.clone() };
            if let Err(e) = client.save_session(&sid, &name, &h, &ui_messages).await {
                tracing::warn!(
                    session_id = %sid,
                    error = %e,
                    "pre-run session upsert failed; server-side run may 404"
                );
            }
            // Wait for the reader to connect so the run's events aren't
            // missed. Non-fatal: if it times out, the run proceeds anyway and
            // the persisted history is the catch-up path.
            if tokio::time::timeout(std::time::Duration::from_secs(10), ready_rx)
                .await
                .is_err()
            {
                tracing::warn!(
                    session_id = %sid,
                    "session event reader not connected before run; some early events may be missed"
                );
            }
            match client.run_session(&sid, &input_text, plan_mode).await {
                Ok(_run_id) => {
                    // The session-event SSE reader (respawned above) delivers
                    // events into the UI as the daemon-owned run progresses.
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::StreamError(format!(
                        "server-side run failed: {e}"
                    )));
                    let _ = tx.send(AppEvent::TurnComplete);
                }
            }
        });
    }

    pub(super) fn spawn_agent_turn(&mut self, input_text: String, hide_input: bool) {
        self.spawn_agent_turn_inner(input_text, hide_input, true);
    }

    fn spawn_agent_turn_inner(
        &mut self,
        input_text: String,
        hide_input: bool,
        name_hidden_session: bool,
    ) {
        if hide_input && name_hidden_session {
            // Auto-name session from a short label instead of the full prompt
            if self.session_name == "New Session" {
                self.session_name = "Init Project".to_string();
            }
        } else if !hide_input && self.session_name == "New Session" {
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
        // checkpoint_turn_id == turn_id: the per-turn checkpoint dir is keyed
        // by turn_id (daemon `begin_turn`). file_count is filled lazily from
        // the checkpoint manifest when the /undo picker opens.
        self.record_turn_start(turn_id.to_string(), &input_text, turn_id.to_string());
        // Server-side mode: POST /run; the SSE reader renders events.
        if self.server_side_loop {
            self.start_server_side_run(input_text, hide_input);
            return;
        }
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
        let (recall_top_n, recall_threshold) = {
            let s = self.settings_lock.read().expect("lock poisoned: settings");
            (
                s.storage.memory.recall_top_n,
                // Read the effective-importance floor from settings so the TUI
                // path matches the headless path (which already reads this key).
                // Previously hardcoded to 0.5, silently ignoring user config.
                s.storage.memory.recall_min_effective_importance as f64,
            )
        };

        // Capture pre-turn layer metadata for the turn-context inspector.
        let pre_turn_layers = self.assembled_instructions.layers.clone();
        let turn_idx = self.turn_count;

        // Clone conversation_history Arc a second time so we can read it
        // after the first clone is moved into AgentLoop::new.
        let history_for_snapshot = self.conversation_history.clone();

        self.current_turn_handle = Some(tokio::spawn(async move {
            // Reward the memories injected last turn: the user continuing the
            // conversation is an implicit "those were useful" signal. Must run
            // before this turn's recall, which overwrites last_injected_ids.
            if let Err(e) = memory_manager.reinforce_last_injected().await {
                tracing::warn!(error = %e, "failed to reinforce last injected memories");
            }
            // Per-turn recall: use MemoryContextInjector for keyword extraction
            // and TF-IDF search over cross-session memories.
            let recall_result = MemoryContextInjector::recall(
                &input_agent,
                &memory_manager,
                recall_top_n,
                recall_threshold,
                None,
            )
            .await;
            let recalled_text = &recall_result.text;

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

    /// Whether either TUI-owned or daemon-owned work is currently running.
    pub(super) fn has_running_turn(&self) -> bool {
        self.current_turn_handle.is_some() || self.server_side_turn_active
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
    /// turn (used by code-rollback); it equals the turn_id because the daemon
    /// keys per-turn checkpoint dirs by turn_id (`begin_turn`). `file_count`
    /// starts at 0 and is filled lazily from the checkpoint manifest by
    /// `apply_file_counts` when the `/undo` picker opens.
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
            committed_messages_end_idx: 0,
            // Filled lazily via RefreshUndoFileCounts -> apply_file_counts.
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
                last.committed_messages_end_idx = self.committed_messages.len();
            }
        }
    }

    /// Roll back to `turn_id`, keeping that turn and everything before it.
    /// `scope` selects code / chat / both.
    ///
    /// Code rollback rewinds every turn *after* the target (oldest-first) via
    /// the daemon `CheckpointStore`, restoring the working tree to its
    /// end-of-target-turn state. The TUI does not hold `CheckpointStore`
    /// directly, so this goes over HTTP (`POST /api/v1/tools/undo-turn`).
    /// `Both` runs code before chat so the later-turn ids are collected before
    /// the chat branch truncates `turn_records`.
    pub(super) async fn undo_to_turn(&mut self, turn_id: &str, scope: UndoScope) -> UndoReport {
        let mut report = UndoReport::default();
        let Some(idx) = self.turn_records.iter().position(|r| r.turn_id == turn_id) else {
            return report; // turn not found, no-op
        };
        let target = self.turn_records[idx].clone();

        if matches!(scope, UndoScope::Code | UndoScope::Both) {
            let later_turn_ids: Vec<String> = self.turn_records[idx + 1..]
                .iter()
                .map(|t| t.turn_id.clone())
                .collect();
            if later_turn_ids.is_empty() {
                // Target is the latest turn: nothing after it to rewind.
                report.code_skipped = true;
            } else {
                match self.daemon_client.undo_turn_range(&later_turn_ids).await {
                    Ok(result) => {
                        report.files_restored = result.restored;
                        // No later turn had a non-empty checkpoint (e.g. all
                        // were pure-chat turns) -> nothing was actually rolled
                        // back.
                        if result.rewound_turns.is_empty() {
                            report.code_skipped = true;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "code rollback (undo-turn) failed");
                        report.code_skipped = true;
                    }
                }
            }
        }
        if matches!(scope, UndoScope::Chat | UndoScope::Both) {
            {
                let mut hist = self.conversation_history.lock().await;
                let prev = hist.len();
                hist.truncate(target.message_end_idx);
                report.messages_truncated = prev.saturating_sub(hist.len());
            }
            let prev_ui = self.committed_messages.len();
            self.committed_messages
                .truncate(target.committed_messages_end_idx);
            report.ui_messages_truncated = prev_ui.saturating_sub(self.committed_messages.len());
            let prev_turns = self.turn_records.len();
            self.turn_records.truncate(idx + 1);
            report.turns_removed = prev_turns.saturating_sub(self.turn_records.len());
            // Keep `turn_count` in sync with the (now-truncated) turn_records
            // and clear `current_turn_id` if it pointed at a turn that was just
            // rolled back. A stale `current_turn_id`/`turn_count` would desync
            // the UI (e.g. the status bar's turn counter) from the real history.
            self.turn_count = self.turn_records.len();
            if let Some(tid) = &self.current_turn_id {
                if !self.turn_records.iter().any(|t| t.turn_id == tid.0) {
                    self.current_turn_id = None;
                }
            }
        }
        report
    }

    // ── /undo interactive rollback flow (Task 8) ──────────────────────

    /// Open the `/undo` turn-picker popup, pre-filtering `turn_records` to
    /// only undo-able turns (those ending at or after the compaction
    /// boundary).  When nothing is undo-able a "No turns to undo" system
    /// message is pushed instead and no picker is opened.
    pub(super) fn open_undo_picker(&mut self) {
        self.undo_picker_open = true;
        let turns = filter_undo_turns(&self.turn_records, self.compaction_boundary);
        if turns.is_empty() {
            self.push_system_message("No turns to undo.");
            self.undo_picker_open = false;
            return;
        }
        self.turn_picker = Some(TurnPickerState::new(turns));
        // Asynchronously refresh file_count from the daemon checkpoint store so
        // the turn-picker can show "✓N" per turn. The picker renders fine with
        // file_count = 0 until the refresh lands.
        let _ = self.event_tx.send(AppEvent::RefreshUndoFileCounts);
    }

    // ── /model switch flow ────────────────────────────────────────────

    /// Open the `/model` picker. Asynchronously fetches the switchable profile
    /// list from the daemon; the picker opens when `ModelsReady` lands. A
    /// placeholder message is pushed immediately so the user sees feedback.
    pub(super) fn open_model_picker(&mut self) {
        self.push_system_message("Loading model profiles…");
        let _ = self.event_tx.send(AppEvent::RefreshModels);
    }

    /// Switch directly to a profile by name (`/model <name>`), bypassing the
    /// picker. Dispatched to the async loop since the daemon call is async.
    pub(super) fn switch_model_direct(&mut self, profile: &str) {
        let profile = profile.to_string();
        self.push_system_message(format!("Switching to model '{profile}'…"));
        let _ = self
            .event_tx
            .send(AppEvent::ModelSwitchRequested { profile });
    }

    /// Confirm the current `/model` picker selection: close the popup and
    /// dispatch the switch to the async loop.
    pub(super) fn confirm_model_selection(&mut self) {
        let profile = self
            .model_picker
            .as_ref()
            .and_then(|p| p.selected_option())
            .map(|o| o.key.clone());
        self.model_picker = None;
        if let Some(profile) = profile {
            let _ = self
                .event_tx
                .send(AppEvent::ModelSwitchRequested { profile });
        }
    }

    /// Rebuild the turn-picker popup from the current `turn_records`,
    /// preserving the previously selected turn (via `pending_undo_turn_id`)
    /// when it is still present.  Used to fall back from the scope picker on
    /// Esc without losing the user's selection.
    pub(super) fn rebuild_turn_picker(&mut self) {
        let mut picker = TurnPickerState::new(filter_undo_turns(
            &self.turn_records,
            self.compaction_boundary,
        ));
        if let Some(ref id) = self.pending_undo_turn_id {
            if let Some(i) = picker.turns.iter().position(|t| &t.turn_id == id) {
                picker.selected = i;
            }
        }
        self.turn_picker = Some(picker);
    }

    /// Fill `turn_records[*].file_count` from daemon checkpoint infos (matched
    /// by turn id), then rebuild the turn picker if it is open so the "✓N"
    /// markers appear. Called from the `UndoFileCountsReady` event handler.
    pub(super) fn apply_file_counts(&mut self, infos: &[crate::tui::client::CheckpointInfo]) {
        for rec in &mut self.turn_records {
            if let Some(info) = infos.iter().find(|i| i.turn_id == rec.turn_id) {
                rec.file_count = info.file_count;
            }
        }
        if self.undo_picker_open && self.turn_picker.is_some() {
            self.rebuild_turn_picker();
        }
    }

    /// Confirm the turn selected in the turn picker and advance to the
    /// scope picker.  Captures the selected `turn_id` into
    /// `pending_undo_turn_id`, opens the scope picker, and closes the turn
    /// picker.  No-op (closes the turn picker) if no turn is selected.
    pub(super) fn confirm_turn_selection(&mut self) {
        let selected = self
            .turn_picker
            .as_ref()
            .and_then(|p| p.selected_turn_id().map(String::from));
        self.turn_picker = None;
        let Some(turn_id) = selected else {
            return;
        };
        self.pending_undo_turn_id = Some(turn_id);
        self.scope_picker = Some(UndoScopePickerState::new());
    }
}

/// Filter `turn_records` down to those eligible for `/undo`: a turn is
/// undo-able only if it ends at or after the compaction boundary
/// (`message_end_idx >= boundary`).  Turns whose messages have already been
/// summarized away by compaction cannot be cleanly rolled back and are
/// excluded.  A pure function so the filtering logic is unit-testable
/// independently of `App`.
pub(super) fn filter_undo_turns(turns: &[TurnRecord], boundary: usize) -> Vec<TurnRecord> {
    turns
        .iter()
        .filter(|t| t.message_end_idx >= boundary)
        .cloned()
        .collect()
}

/// Scope of an `/undo` rollback operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoScope {
    Code,
    Chat,
    Both,
}

/// Result of an `/undo` rollback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct UndoReport {
    pub messages_truncated: usize,
    pub ui_messages_truncated: usize,
    pub turns_removed: usize,
    /// Files restored by code-rollback (0 when `code_skipped`).
    pub files_restored: usize,
    pub code_skipped: bool,
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
    use crate::tools::execution::BackgroundResult;
    use crate::tui::app::PendingInput;
    use crate::tui::client::{CheckpointInfo, DaemonClient};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    fn build_app() -> App {
        let client = DaemonClient::new("http://localhost:0".to_string());
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        App::new(client, "test-interrupt".to_string(), settings)
    }

    fn background_result() -> BackgroundResult {
        BackgroundResult {
            task_id: "bg_a".to_string(),
            session_id: Some("test-interrupt".to_string()),
            result_type: "command".to_string(),
            command: "true".to_string(),
            stdout: "done".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
            sandbox_bypassed: false,
            permission_mode: None,
            sandbox_level: None,
        }
    }

    #[test]
    fn background_result_pending_input_is_hidden_and_serialized() {
        let pending = PendingInput::background_result(background_result());

        assert!(pending.hidden);
        assert!(pending.display_text.is_empty());
        assert_eq!(
            pending.agent_input,
            r#"{"task_id":"bg_a","session_id":"test-interrupt","result_type":"command","command":"true","stdout":"done","stderr":"","exit_code":0,"success":true,"sandbox_bypassed":false,"permission_mode":null,"sandbox_level":null}"#
        );
    }

    #[tokio::test]
    async fn completed_background_result_starts_one_hidden_turn_when_idle() {
        let mut app = build_app();

        app.handle_event(AppEvent::BackgroundTaskCompleted(background_result()))
            .await;

        assert!(
            app.current_turn_handle.is_some(),
            "idle app starts one turn"
        );
        assert!(app.pending_inputs.is_empty(), "result is not left queued");
        assert_eq!(
            app.session_name, "New Session",
            "hidden result does not name the session"
        );
        assert!(app.committed_messages.iter().any(|message| {
            message.role == MessageRole::System
                && message.content == "[Background task bg_a completed: SUCCESS]"
        }));
        assert!(!app
            .committed_messages
            .iter()
            .any(|message| { message.role == MessageRole::User && message.content.is_empty() }));

        app.current_turn_handle
            .take()
            .expect("idle result starts a turn")
            .abort();
    }

    #[tokio::test]
    async fn completed_background_result_is_displayed_without_queuing_while_running() {
        let mut app = build_app();
        app.current_turn_handle = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));

        app.handle_event(AppEvent::BackgroundTaskCompleted(background_result()))
            .await;

        assert!(
            app.current_turn_handle.is_some(),
            "existing turn keeps running"
        );
        assert!(
            app.pending_inputs.is_empty(),
            "running turn does not queue a continuation"
        );
        assert!(app.committed_messages.iter().any(|message| {
            message.role == MessageRole::System
                && message.content == "[Background task bg_a completed: SUCCESS]"
        }));

        app.current_turn_handle
            .take()
            .expect("running turn remains active")
            .abort();
    }

    #[tokio::test]
    async fn server_side_background_result_marks_the_continuation_as_running() {
        let mut app = build_app();
        app.server_side_loop = true;

        app.handle_event(AppEvent::BackgroundTaskCompleted(background_result()))
            .await;

        assert!(
            app.server_side_turn_active,
            "server-side continuation records an active daemon turn"
        );
        assert!(app.current_turn_handle.is_none());

        if let Some(handle) = app.session_event_reader.take() {
            handle.abort();
        }
        if let Some(handle) = app.trace_event_reader.take() {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn stale_background_result_is_dropped_after_session_switch() {
        let mut app = build_app();
        app.session_id = "session-b".to_string();

        app.handle_event(AppEvent::BackgroundTaskCompleted(background_result()))
            .await;

        assert!(app.committed_messages.is_empty());
        assert!(app.pending_inputs.is_empty());
        assert!(app.current_turn_handle.is_none());
    }

    #[tokio::test]
    async fn session_switch_restarts_the_session_scoped_global_reader() {
        let mut app = build_app();
        assert!(app.global_event_reader.is_none());

        app.handle_event(AppEvent::SessionSwitched {
            id: "session-b".to_string(),
            name: "Session B".to_string(),
        })
        .await;

        assert!(
            app.global_event_reader.is_some(),
            "session switch installs a reader filtered to the new session"
        );

        if let Some(handle) = app.global_event_reader.take() {
            handle.abort();
        }
        if let Some(handle) = app.session_event_reader.take() {
            handle.abort();
        }
        if let Some(handle) = app.trace_event_reader.take() {
            handle.abort();
        }
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

    // ── /undo flow integration (Task 8) ───────────────────────────────

    /// Helper: build a finalized `TurnRecord` with a given `message_end_idx`.
    fn make_turn_with_end(id: &str, message_end_idx: usize) -> TurnRecord {
        TurnRecord {
            turn_id: id.to_string(),
            created_at: "2025-06-01T14:30:00Z".to_string(),
            user_summary: format!("Turn {}", id),
            checkpoint_turn_id: String::new(),
            message_end_idx,
            file_count: 0,
            committed_messages_end_idx: 0,
        }
    }

    #[test]
    fn filter_undo_turns_keeps_turns_at_or_after_boundary() {
        let turns = vec![
            make_turn_with_end("t1", 2),
            make_turn_with_end("t2", 5),
            make_turn_with_end("t3", 8),
        ];
        // boundary=5: keep t2 (5>=5) and t3 (8>=5); drop t1 (2<5, compacted).
        let filtered = filter_undo_turns(&turns, 5);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].turn_id, "t2");
        assert_eq!(filtered[1].turn_id, "t3");
    }

    #[test]
    fn filter_undo_turns_boundary_zero_keeps_all() {
        let turns = vec![make_turn_with_end("t1", 0), make_turn_with_end("t2", 4)];
        let filtered = filter_undo_turns(&turns, 0);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_undo_turns_empty_when_all_before_boundary() {
        let turns = vec![make_turn_with_end("t1", 2), make_turn_with_end("t2", 3)];
        let filtered = filter_undo_turns(&turns, 10);
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn open_undo_picker_populates_turn_picker_filtered_by_boundary() {
        let mut app = build_app();
        app.turn_records = vec![
            make_turn_with_end("t1", 2),
            make_turn_with_end("t2", 5),
            make_turn_with_end("t3", 8),
        ];
        app.compaction_boundary = 5;
        app.open_undo_picker();
        let picker = app.turn_picker.expect("turn_picker should be open");
        assert_eq!(picker.turns.len(), 2, "only turns at/after boundary");
        assert_eq!(picker.turns[0].turn_id, "t2");
        assert_eq!(picker.turns[1].turn_id, "t3");
        assert!(picker.open);
        assert!(app.undo_picker_open);
    }

    #[tokio::test]
    async fn open_undo_picker_empty_pushes_no_turns_message() {
        let mut app = build_app();
        app.turn_records = vec![make_turn_with_end("t1", 2)];
        app.compaction_boundary = 10;
        app.open_undo_picker();
        assert!(app.turn_picker.is_none(), "no picker when nothing to undo");
        assert!(
            app.committed_messages
                .iter()
                .any(|m| m.content.contains("No turns to undo")),
            "should push a no-turns message"
        );
        assert!(!app.undo_picker_open);
    }

    #[tokio::test]
    async fn confirm_turn_selection_switches_to_scope_picker() {
        let mut app = build_app();
        app.turn_records = vec![make_turn_with_end("t1", 2), make_turn_with_end("t2", 5)];
        app.compaction_boundary = 0;
        app.open_undo_picker();
        // Move selection to the second turn (index 1).
        app.turn_picker.as_mut().unwrap().down();
        app.confirm_turn_selection();
        assert!(app.turn_picker.is_none(), "turn picker closed");
        let scope = app.scope_picker.expect("scope picker open");
        assert!(scope.open);
        assert_eq!(
            app.pending_undo_turn_id.as_deref(),
            Some("t2"),
            "pending turn id captured"
        );
    }

    #[tokio::test]
    async fn undo_requested_event_runs_undo_and_pushes_report() {
        let mut app = build_app();
        // Two finalized turns; t1 ends at idx 2, t2 ends at idx 4.
        app.turn_records = vec![make_turn_with_end("t1", 2), make_turn_with_end("t2", 4)];
        app.turn_records[0].committed_messages_end_idx = 2;
        app.turn_records[1].committed_messages_end_idx = 4;
        app.compaction_boundary = 0;
        // Populate conversation history with 4 messages.
        {
            let mut h = app.conversation_history.lock().await;
            for i in 0..4 {
                h.push(ChatMessage::user(format!("msg {}", i)));
            }
        }
        // Populate committed (UI) messages.
        for i in 0..4 {
            app.committed_messages.push(UIMessage {
                role: MessageRole::User,
                content: format!("ui {}", i),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_running: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
        }
        app.pending_undo_turn_id = Some("t1".to_string());

        app.handle_event(AppEvent::UndoRequested {
            turn_id: "t1".to_string(),
            scope: UndoScope::Chat,
        })
        .await;

        // The UndoReport should be echoed as a system message.
        assert!(
            app.committed_messages
                .iter()
                .any(|m| m.content.contains("messages_truncated")),
            "undo report pushed as system message"
        );
        // History truncated to t1's message_end_idx (2).
        let h = app.conversation_history.lock().await;
        assert_eq!(h.len(), 2);
        // turn_records truncated to just t1.
        assert_eq!(app.turn_records.len(), 1);
        assert_eq!(app.turn_records[0].turn_id, "t1");
    }

    // ── Task 9: post-undo state adjustment ─────────────────────────────

    /// Build an app with two finalized turns (t1, t2) and matching
    /// conversation/UI history, ready for `undo_to_turn` state-sync tests.
    async fn build_app_with_two_turns() -> App {
        let mut app = build_app();
        app.turn_records = vec![make_turn_with_end("t1", 2), make_turn_with_end("t2", 4)];
        app.turn_records[0].committed_messages_end_idx = 2;
        app.turn_records[1].committed_messages_end_idx = 4;
        app.compaction_boundary = 0;
        app.turn_count = 2;
        {
            let mut h = app.conversation_history.lock().await;
            for i in 0..4 {
                h.push(ChatMessage::user(format!("msg {}", i)));
            }
        }
        for i in 0..4 {
            app.committed_messages.push(UIMessage {
                role: MessageRole::User,
                content: format!("ui {}", i),
                tool_name: None,
                tool_args: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_running: false,
                diff_data: None,
                tool_metadata: None,
            });
        }
        app
    }

    #[tokio::test]
    async fn undo_to_turn_syncs_turn_count_with_turn_records() {
        let mut app = build_app_with_two_turns().await;
        assert_eq!(app.turn_count, 2, "precondition: two turns recorded");

        // Roll back to t1, dropping t2 from turn_records.
        app.undo_to_turn("t1", UndoScope::Chat).await;

        assert_eq!(
            app.turn_count,
            app.turn_records.len(),
            "turn_count must mirror turn_records.len() after undo"
        );
        assert_eq!(app.turn_count, 1, "one turn remains");
    }

    #[tokio::test]
    async fn undo_to_turn_clears_current_turn_id_when_pointing_at_removed_turn() {
        let mut app = build_app_with_two_turns().await;
        // current_turn_id points at t2, which gets removed by rolling back to t1.
        app.current_turn_id = Some(TurnId("t2".to_string()));

        app.undo_to_turn("t1", UndoScope::Chat).await;

        assert!(
            app.current_turn_id.is_none(),
            "current_turn_id must be cleared when its turn was rolled back"
        );
    }

    #[tokio::test]
    async fn undo_to_turn_preserves_current_turn_id_when_pointing_at_kept_turn() {
        let mut app = build_app_with_two_turns().await;
        // current_turn_id points at t1, the rollback target which is kept.
        app.current_turn_id = Some(TurnId("t1".to_string()));

        app.undo_to_turn("t1", UndoScope::Chat).await;

        assert_eq!(
            app.current_turn_id.as_ref().map(|t| t.0.as_str()),
            Some("t1"),
            "current_turn_id must survive when its turn is retained"
        );
    }

    // ── Task 4: code rollback wiring (UndoScope::Code) ───────────────────

    #[tokio::test]
    async fn undo_to_turn_code_only_does_not_truncate_chat() {
        let mut app = build_app_with_two_turns().await;
        // Code-only scope must not touch chat history / turn records, even
        // though the code-rollback HTTP call is attempted (and fails: the
        // test daemon at localhost:0 is unreachable).
        let report = app.undo_to_turn("t1", UndoScope::Code).await;
        assert_eq!(app.turn_records.len(), 2, "Code scope keeps all turns");
        let h = app.conversation_history.lock().await;
        assert_eq!(h.len(), 4, "Code scope keeps all messages");
        assert!(report.code_skipped, "unreachable daemon -> code_skipped");
        assert_eq!(report.files_restored, 0);
    }

    #[tokio::test]
    async fn undo_to_turn_code_scope_last_turn_is_skipped() {
        let mut app = build_app_with_two_turns().await;
        // Rolling back to the last turn (t2): no later turns to rewind.
        let report = app.undo_to_turn("t2", UndoScope::Code).await;
        assert!(report.code_skipped, "no later turns -> code_skipped");
        assert_eq!(report.files_restored, 0);
    }

    #[tokio::test]
    async fn undo_to_turn_both_scope_runs_code_then_chat() {
        let mut app = build_app_with_two_turns().await;
        app.current_turn_id = Some(TurnId("t2".to_string()));

        let report = app.undo_to_turn("t1", UndoScope::Both).await;

        // Code attempted first (daemon unreachable -> skipped), then chat
        // truncated. The code branch must read later turn ids *before* the
        // chat branch truncates turn_records.
        assert!(report.code_skipped, "code attempted but daemon unreachable");
        assert_eq!(report.turns_removed, 1, "t2 removed by chat rollback");
        assert_eq!(app.turn_records.len(), 1, "only t1 remains");
        assert_eq!(app.turn_records[0].turn_id, "t1");
        assert!(app.current_turn_id.is_none(), "t2 cleared");
    }

    // ── Task 4: file_count refresh from checkpoint manifest ─────────────

    #[tokio::test]
    async fn apply_file_counts_updates_turn_records_and_rebuilds_picker() {
        let mut app = build_app_with_two_turns().await;
        assert_eq!(app.turn_records[0].file_count, 0);
        assert_eq!(app.turn_records[1].file_count, 0);
        app.open_undo_picker();
        assert!(app.turn_picker.is_some());

        let infos = vec![
            CheckpointInfo {
                turn_id: "t1".to_string(),
                created_at: String::new(),
                file_count: 3,
            },
            CheckpointInfo {
                turn_id: "t2".to_string(),
                created_at: String::new(),
                file_count: 1,
            },
        ];
        app.apply_file_counts(&infos);

        assert_eq!(app.turn_records[0].file_count, 3);
        assert_eq!(app.turn_records[1].file_count, 1);
        let picker = app.turn_picker.as_ref().expect("picker still open");
        assert_eq!(picker.turns[0].file_count, 3);
        assert_eq!(picker.turns[1].file_count, 1);
    }

    #[tokio::test]
    async fn apply_file_counts_ignores_unknown_turn_ids() {
        let mut app = build_app_with_two_turns().await;
        let infos = vec![CheckpointInfo {
            turn_id: "unknown".to_string(),
            created_at: String::new(),
            file_count: 9,
        }];
        app.apply_file_counts(&infos);
        assert_eq!(app.turn_records[0].file_count, 0);
        assert_eq!(app.turn_records[1].file_count, 0);
    }

    #[tokio::test]
    async fn undo_file_counts_ready_event_fills_counts() {
        let mut app = build_app_with_two_turns().await;
        let infos = vec![
            CheckpointInfo {
                turn_id: "t1".to_string(),
                created_at: String::new(),
                file_count: 5,
            },
            CheckpointInfo {
                turn_id: "t2".to_string(),
                created_at: String::new(),
                file_count: 2,
            },
        ];
        app.handle_event(AppEvent::UndoFileCountsReady(infos)).await;
        assert_eq!(app.turn_records[0].file_count, 5);
        assert_eq!(app.turn_records[1].file_count, 2);
    }
}
