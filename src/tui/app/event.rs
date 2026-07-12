//! Event handling for the TUI application.

use super::types::*;
use super::App;
use crate::prompts::{self, PromptContext};
use crate::tui::util::{
    agent_phase_from_event, compute_collapse_state, extract_diff_data, extract_tool_metadata,
    format_tool_result, tool_label,
};
use std::collections::HashMap;

impl App {
    pub(super) async fn handle_event(&mut self, event: AppEvent) {
        // Derive phase from event (pure function); fall back to current.
        //
        // When `suppress_phase_updates` is set (after /clear or cancel),
        // ignore stale phase-changing events from a just-aborted turn so
        // the status bar doesn't flip back to "Thinking". The flag is
        // cleared when a new turn starts (spawn_agent_turn).
        if let Some(next_phase) = agent_phase_from_event(&event) {
            if self.suppress_phase_updates {
                tracing::debug!(
                    prev = ?self.phase,
                    skipped = ?next_phase,
                    "Suppressing stale phase update after turn cancellation"
                );
            } else {
                if self.phase != next_phase {
                    tracing::info!(
                        prev = ?self.phase,
                        next = ?next_phase,
                        "Agent phase transition"
                    );
                }
                self.phase = next_phase;
            }
        }
        match event {
            AppEvent::KeyEvent(key) => self.handle_key_event(*key),
            AppEvent::Paste(text) => {
                for c in text.chars() {
                    self.input_box.textarea.insert_char(c);
                }
                self.input_box.update_style();
            }
            AppEvent::MouseScrolled(delta) => {
                // Focus view timeline: scroll_offset is lines-from-bottom
                // (0 = newest). ScrollUp → older (add); ScrollDown → newer
                // (sub), re-engaging auto_scroll at the bottom. Mouse wheel
                // is the only way to scroll the timeline (read-only area).
                if let Some(ref mut focus) = self.subagent_focus {
                    if delta > 0 {
                        focus.scroll_offset = focus.scroll_offset.saturating_add(delta as usize);
                        focus.auto_scroll = false;
                    } else {
                        focus.scroll_offset = focus.scroll_offset.saturating_sub((-delta) as usize);
                        if focus.scroll_offset == 0 {
                            focus.auto_scroll = true;
                        }
                    }
                    return;
                }
                if self.session_state.visible {
                    // Session list: keyboard navigation only, ignore mouse scroll
                    return;
                }
                // Main chat scroll: 0 = bottom (newest), larger = further up (older)
                if delta > 0 {
                    // ScrollUp: see OLDER content → further from bottom
                    self.scroll_offset = self.scroll_offset.saturating_add(delta as u16);
                } else {
                    // ScrollDown: see NEWER content → closer to bottom
                    self.scroll_offset = self.scroll_offset.saturating_sub((-delta) as u16);
                    // If scrolled back to bottom, resume auto-scroll for future content
                    if self.scroll_offset == 0 {
                        self.user_scrolled = false;
                        return;
                    }
                }
                self.user_scrolled = true;
            }
            AppEvent::ToggleCollapseAll => {
                // While the subagent focus view is open, Ctrl+E toggles fold of
                // all tool calls in the focus timeline (same as `t`) and must
                // not touch the main chat's collapse state.
                if let Some(ref mut focus) = self.subagent_focus {
                    focus.toggle_fold_all();
                } else {
                    // Real toggle: if anything is currently expanded, collapse everything;
                    // otherwise expand everything. We only flip the field that's relevant
                    // for each message's role to avoid no-op toggles on user/system rows.
                    let any_expanded = self.committed_messages.iter().any(|m| match m.role {
                        MessageRole::Assistant => !m.content_collapsed,
                        MessageRole::Tool => !m.tool_collapsed,
                        _ => false,
                    });
                    let collapse = any_expanded;
                    for m in &mut self.committed_messages {
                        match m.role {
                            MessageRole::Assistant => m.content_collapsed = collapse,
                            MessageRole::Tool => m.tool_collapsed = collapse,
                            _ => {}
                        }
                    }
                }
            }
            AppEvent::ToggleCollapseLatest => {
                // While the subagent focus view is open, Ctrl+O toggles fold of
                // the last tool call in the focus timeline and must not touch
                // the main chat's collapse state.
                if let Some(ref mut focus) = self.subagent_focus {
                    focus.toggle_fold_latest();
                } else {
                    // Real toggle for the last message: flip only the field that
                    // controls visibility for its role.
                    if let Some(last) = self.committed_messages.last_mut() {
                        match last.role {
                            MessageRole::Assistant => {
                                last.content_collapsed = !last.content_collapsed;
                            }
                            MessageRole::Tool => {
                                last.tool_collapsed = !last.tool_collapsed;
                            }
                            _ => {}
                        }
                    }
                }
            }
            AppEvent::Submit(text) => {
                // NOTE: do NOT clear the subagent tree here. A prompt submitted
                // while a turn is still running is only queued (see submit_input),
                // and clearing the tree would hide the running subagents and
                // block entering the focus view. The tree is cleared at the
                // start of the next turn (TurnStarted) and on abort
                // (TurnAborted, covering /clear and turn failures).
                self.submit_input(text);
            }
            AppEvent::PreparingTools if self.streaming_content.is_empty() => {
                // Show "preparing..." hint in streaming area immediately
                self.streaming_content = "\u{23F3} preparing tools...".to_string();
                self.streaming_active = true;
            }
            AppEvent::PreparingTools => {}
            AppEvent::ContentDelta(text) => {
                self.streaming_content.push_str(&text);
                self.streaming_active = true;
                // Auto-scroll: keep at bottom when streaming and user hasn't manually scrolled
                if !self.user_scrolled {
                    self.scroll_offset = 0;
                }
            }
            AppEvent::StreamDone { .. } => {
                // Commit real streamed content (skip the "preparing tools..." hint)
                let content = std::mem::take(&mut self.streaming_content);
                let is_hint = content.starts_with('\u{23F3}');
                if !content.is_empty() && !is_hint {
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::Assistant,
                        content,
                        tool_name: None,
                        tool_args: None,
                        content_collapsed: false,
                        tool_collapsed: true,
                        tool_running: false,
                        diff_data: None,
                        tool_metadata: None,
                    });
                }
                self.streaming_active = false;
                // Only reset scroll position if user was at auto-scroll bottom.
                // If the user scrolled up to read older content mid-stream,
                // preserve their position across stream completion.
                if !self.user_scrolled {
                    self.scroll_offset = 0;
                }
            }
            AppEvent::ToolStart { name, args } => {
                // Clear any "preparing tools..." hint
                self.streaming_content.clear();
                self.streaming_active = false;
                self.has_running_tool = true;
                self.last_tool_name = Some(tool_label(&name, &args));
                self.committed_messages.push(UIMessage {
                    role: MessageRole::Tool,
                    content: String::new(),
                    tool_name: Some(name.clone()),
                    tool_args: Some(args.clone()),
                    content_collapsed: false,
                    tool_collapsed: true,
                    tool_running: true,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            AppEvent::ToolResult {
                name,
                args,
                content,
            } => {
                let diff_data = extract_diff_data(&name, &args, &content);
                let tool_metadata = extract_tool_metadata(&content);
                // Tool completed, clear running state for spinner
                self.has_running_tool = false;
                // Replace the placeholder ToolStart message with the result
                if let Some(last) = self.committed_messages.last_mut() {
                    if last.role == MessageRole::Tool
                        && last.tool_running
                        && last.tool_name.as_deref() == Some(&name)
                    {
                        // task/delegate tools: keep the entry compact — the
                        // subagent's full output lives in the focus view now.
                        last.content = if name == "task" || name == "delegate" {
                            "→ focus view for details".to_string()
                        } else {
                            format_tool_result(&name, &args, &content)
                        };
                        last.tool_collapsed = true;
                        last.tool_running = false;
                        last.diff_data = diff_data;
                        last.tool_metadata = tool_metadata;
                    } else {
                        let formatted = format_tool_result(&name, &args, &content);
                        self.committed_messages.push(UIMessage {
                            role: MessageRole::Tool,
                            content: formatted,
                            tool_name: Some(name),
                            tool_args: Some(args),
                            content_collapsed: false,
                            tool_collapsed: true,
                            tool_running: false,
                            diff_data,
                            tool_metadata,
                        });
                    }
                }
            }
            AppEvent::Connecting { .. } => {
                // Phase transition handled by agent_phase_from_event;
                // status bar renders the phase label ("connecting...").
            }
            AppEvent::StreamError(msg) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("⚠ {}", msg),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: true,
                    tool_running: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
                self.streaming_active = false;
            }
            AppEvent::ContextCompacted { summary_chars } => {
                // Compaction succeeded — surface it so the user can see the
                // context window was compressed (and how much survived as a
                // summary). Collapsed by default to avoid cluttering the chat.
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!(
                        "Conversation compacted — earlier history replaced with a {}-char summary; recent results kept inline.",
                        summary_chars
                    ),
                    tool_name: None,
                    content_collapsed: true,
                    tool_collapsed: true,
                    tool_running: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            AppEvent::Tick if self.has_running_tool => {
                self.spinner_frame = self.spinner_frame.wrapping_add(1);
            }
            AppEvent::Tick => {
                // Throttle the task-group claim poll to 500ms so idle polling
                // does not generate excessive HTTP traffic. Only poll when no
                // turn is running.
                let should_poll = self.current_turn_handle.is_none()
                    && self
                        .last_claim_attempt
                        .map(|t| t.elapsed() >= std::time::Duration::from_millis(500))
                        .unwrap_or(true);
                if should_poll {
                    self.last_claim_attempt = Some(std::time::Instant::now());
                    self.poll_ready_task_groups().await;
                }
            }
            AppEvent::ConfigChanged(new_settings) => {
                // Rebuild system messages from new settings
                let prompt_ctx = PromptContext::new()
                    .with_cwd(
                        std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."))
                            .display()
                            .to_string(),
                    )
                    .with_shell(std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string()))
                    .with_sandbox("workspace-write")
                    .with_approval("never")
                    .with_collaboration(
                        new_settings
                            .prompt
                            .collaboration_mode
                            .clone()
                            .unwrap_or_default(),
                    );
                let project_root =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let wgenty_sections = crate::utils::project::read_wgenty_md_sections(&project_root);
                let agents_sections = crate::utils::project::read_agents_md_sections(&project_root);

                // Load skills inventory (including external skills)
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let skills_dirs = vec![home.join(".wgenty-code").join("skills")];
                let skill_loader =
                    crate::knowledge::loader::SkillLoader::load_from_dirs(&skills_dirs);
                let mut skill_inventory: Vec<crate::prompts::SkillEntry> = Vec::new();
                for name in skill_loader.skill_names() {
                    if !crate::knowledge::should_expose_skill_by_default(&name) {
                        continue;
                    }
                    if let Some(skill) = skill_loader.load_skill(&name) {
                        skill_inventory.push(crate::prompts::SkillEntry {
                            name,
                            description: skill.description.clone(),
                        });
                    }
                }

                // Merge external skills
                let external_registry_roots =
                    crate::knowledge::SkillRootResolver::roots_with(&home, &project_root);
                if let Ok(external_registry) =
                    crate::knowledge::ExternalSkillRegistry::discover(external_registry_roots)
                {
                    for skill_def in external_registry.list() {
                        if !crate::knowledge::should_expose_skill_by_default(
                            &skill_def.canonical_name,
                        ) {
                            continue;
                        }
                        if !skill_inventory
                            .iter()
                            .any(|s| s.name == skill_def.canonical_name)
                        {
                            skill_inventory.push(crate::prompts::SkillEntry {
                                name: skill_def.canonical_name.clone(),
                                description: skill_def.description.clone(),
                            });
                        }
                    }
                }

                let prompt_ctx = prompt_ctx
                    .with_skills(skill_inventory)
                    .with_wgenty_md(wgenty_sections)
                    .with_agents_md(agents_sections);
                let assembled = prompts::assemble_instructions(&new_settings, &prompt_ctx);
                self.assembled_system_messages = assembled.system_messages;
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: "Settings reloaded".to_string(),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: true,
                    tool_running: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            AppEvent::TurnStarted { .. } => {
                // Fresh turn. Only clear the previous turn's subagent state
                // when no subagents are still active — background subagents
                // (task tool `background` mode) outlive the main turn and must
                // stay visible/selectable. Clearing them would hide the status
                // bar and block entering their focus view.
                if self.subagent_tree.clear_if_idle() {
                    self.completed_at.clear();
                    self.subagent_focus = None;
                    self.subagent_status_bar_selected = 0;
                }
                self.turn_started_at = Some(std::time::Instant::now());
            }
            AppEvent::TurnComplete => {
                // Fire Stop hook asynchronously
                {
                    let hm = self.hook_manager.clone();
                    let sid = self.session_id.clone();
                    tokio::spawn(async move {
                        let cwd = std::env::current_dir().unwrap_or_default();
                        let ctx = crate::runtime::hooks::HookContext {
                            event: "Stop".to_string(),
                            tool_name: None,
                            tool_input: None,
                            tool_result: Some("stop".to_string()),
                            session_id: Some(sid),
                            working_directory: cwd.to_string_lossy().to_string(),
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            comet_phase: None,
                            workflow_state: None,
                            variables: Default::default(),
                        };
                        hm.fire(&crate::runtime::hooks::HookEvent::Stop, &ctx, None, None)
                            .await;
                    });
                }
                let snapshot = self.subagent_tree.clone();
                let key = format!("turn_{}", chrono::Utc::now().timestamp_millis());
                self.subagent_history.insert(key, snapshot);
                self.turn_count += 1;
                self.current_turn_handle = None;
                self.last_abort_reason = None; // normal completion clears
                self.turn_started_at = None;
                if !self.pending_inputs.is_empty() {
                    self.start_next_turn();
                }
            }
            AppEvent::TurnAborted { ref reason } => {
                // Fire Stop hook asynchronously
                {
                    let hm = self.hook_manager.clone();
                    let sid = self.session_id.clone();
                    let reason_str = format!("aborted: {:?}", reason);
                    tokio::spawn(async move {
                        let cwd = std::env::current_dir().unwrap_or_default();
                        let ctx = crate::runtime::hooks::HookContext {
                            event: "Stop".to_string(),
                            tool_name: None,
                            tool_input: None,
                            tool_result: Some(reason_str),
                            session_id: Some(sid),
                            working_directory: cwd.to_string_lossy().to_string(),
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            comet_phase: None,
                            workflow_state: None,
                            variables: Default::default(),
                        };
                        hm.fire(&crate::runtime::hooks::HookEvent::Stop, &ctx, None, None)
                            .await;
                    });
                }
                self.last_abort_reason = Some(reason.clone());
                // Aborted turn (e.g. /clear via cancel_current_turn, or a turn
                // failure): clear the subagent tree so stale subagents don't
                // linger in the status bar. cancel_current_turn does not emit
                // Cancelled updates for running subagents, so without this the
                // bar would keep showing them as Running.
                self.subagent_tree.clear();
                self.completed_at.clear();
                self.subagent_focus = None;
                self.subagent_status_bar_selected = 0;
                self.turn_started_at = None;
            }
            AppEvent::PermissionRequired {
                reason,
                rule,
                responder,
            } => {
                tracing::info!("🔐 App: showing permission panel for '{}'", rule);
                // Yolo mode: auto-approve all permissions
                if self.mode == AgentMode::Yolo {
                    let _ = responder.0.unwrap().send(PermissionResponse::AllowOnce);
                    return;
                }
                // AcceptEdits mode: auto-approve file-edit permissions
                if self.mode == AgentMode::AcceptEdits
                    && (rule == "apply_patch" || rule == "file_edit" || rule == "file_write")
                {
                    let _ = responder.0.unwrap().send(PermissionResponse::AllowOnce);
                    return;
                }
                self.permission_state.show(reason, rule, responder);
            }
            AppEvent::QuestionAsked {
                question,
                options,
                multi_select,
                responder,
            } => {
                self.question_state
                    .show(question, options, multi_select, responder);
            }
            AppEvent::ToggleSessions => {
                if self.session_state.visible {
                    self.session_state.dismiss();
                } else {
                    // Show the panel immediately with an empty list so the user
                    // sees feedback right away; the actual list is loaded async.
                    self.session_state.show(Vec::new());
                    let client = self.daemon_client.clone();
                    let tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        match client.list_sessions().await {
                            Ok(sessions) => {
                                let _ = tx.send(AppEvent::SessionListLoaded(sessions));
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to list sessions");
                                // Still update with empty list so the panel is visible
                                let _ = tx.send(AppEvent::SessionListLoaded(Vec::new()));
                            }
                        }
                    });
                }
            }
            AppEvent::SessionListLoaded(sessions) => {
                self.session_state.show(sessions);
            }
            AppEvent::DeleteSession(id) => {
                let client = self.daemon_client.clone();
                let tx = self.event_tx.clone();
                tokio::spawn(async move {
                    let _ = client.delete_session(&id).await;
                    // Refresh session list after deletion
                    if let Ok(sessions) = client.list_sessions().await {
                        let _ = tx.send(AppEvent::SessionListLoaded(sessions));
                    }
                });
            }
            AppEvent::HistoryLoaded(messages) => {
                // First pass: build tool_use_map from assistant messages' tool_calls
                // Maps tool_call id -> (tool_name, tool_args)
                let mut tool_use_map: HashMap<String, (String, serde_json::Value)> = HashMap::new();
                for msg in &messages {
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            let args = serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::Value::Null);
                            tool_use_map.insert(tc.id.clone(), (tc.function.name.clone(), args));
                        }
                    }
                }

                // Second pass: convert ChatMessage to UIMessage, filtering system messages
                self.committed_messages.clear();
                for msg in &messages {
                    match msg.role.as_str() {
                        "system" => continue,
                        "tool" => {
                            let (tool_name, tool_args) = msg
                                .tool_call_id
                                .as_ref()
                                .and_then(|id| tool_use_map.get(id))
                                .map(|(n, a)| (Some(n.clone()), Some(a.clone())))
                                .unwrap_or_else(|| {
                                    // Fallback: use the call_id as the display name
                                    (msg.tool_call_id.clone(), None)
                                });
                            let role = MessageRole::Tool;
                            let content = msg.content.clone().unwrap_or_default();
                            let (content_collapsed, tool_collapsed) =
                                compute_collapse_state(&role, &content);
                            self.committed_messages.push(UIMessage {
                                role,
                                content,
                                tool_name,
                                tool_args,
                                content_collapsed,
                                tool_collapsed,
                                tool_running: false,
                                diff_data: None,
                                tool_metadata: None,
                            });
                        }
                        "user" | "assistant" => {
                            let role = if msg.role == "user" {
                                MessageRole::User
                            } else {
                                MessageRole::Assistant
                            };
                            let content = msg.content.clone().unwrap_or_default();
                            let (content_collapsed, tool_collapsed) =
                                compute_collapse_state(&role, &content);
                            self.committed_messages.push(UIMessage {
                                role,
                                content,
                                tool_name: None,
                                tool_args: None,
                                content_collapsed,
                                tool_collapsed,
                                tool_running: false,
                                diff_data: None,
                                tool_metadata: None,
                            });
                        }
                        _ => continue,
                    }
                }
                self.scroll_offset = 0;
                self.user_scrolled = false;
            }
            AppEvent::UndoResult(output) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::Tool,
                    content: output.clone(),
                    tool_name: Some("undo".to_string()),
                    tool_args: Some(serde_json::json!({})),
                    tool_metadata: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    tool_running: false,
                    diff_data: extract_diff_data("undo", &serde_json::json!({}), &output),
                });
            }
            AppEvent::AgentLocalView(view) => {
                // Replace the tree with the scoped local view from the daemon.
                // Completion-time tracking and focus updates are computed from
                // the current response only.
                self.subagent_tree.replace_local(*view);
                if let Some(ref mut focus) = self.subagent_focus {
                    focus.rebuild(&self.subagent_tree);
                }
            }
            AppEvent::BackgroundTaskResult(notification) => {
                // Push a system-level notification message to the chat so the
                // user sees subagent/background-task results without waiting for
                // the LLM to mention them in its response.
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: notification,
                    tool_name: None,
                    tool_args: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    tool_running: false,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            AppEvent::AgentGenerationReset { generation } => {
                if generation == u64::MAX {
                    // Reset failed on the daemon: surface an actionable
                    // system message and retain the old generation rather
                    // than pretending cancellation succeeded.
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::System,
                        content: "Failed to reset subagent generation; \
                                  obsolete subagents may still be running."
                            .to_string(),
                        tool_name: None,
                        tool_args: None,
                        content_collapsed: false,
                        tool_collapsed: false,
                        tool_running: false,
                        diff_data: None,
                        tool_metadata: None,
                    });
                    self.suppress_phase_updates = false;
                    return;
                }
                self.agent_generation = generation;
                self.subagent_tree.clear();
                self.completed_at.clear();
                self.agent_navigation = crate::tui::app::types::AgentNavigationState::default();
                // Obsolete generation's deliveries are now rejected by the
                // daemon; resume normal phase updates.
                self.suppress_phase_updates = false;
            }
            AppEvent::SaveSession => {
                let id = self.session_id.clone();
                let name = self.session_name.clone();
                let client = self.daemon_client.clone();
                let history = self.conversation_history.clone();
                tokio::spawn(async move {
                    let h = history.lock().await.clone();
                    let _ = client.save_session(&id, &name, &h).await;
                });
            }
            AppEvent::CtrlCPressed => {
                let now = std::time::Instant::now();
                if let Some(last) = self.last_ctrl_c {
                    if last.elapsed().as_millis() < 500 {
                        self.shutdown_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        self.should_quit = true;
                        return;
                    }
                }
                self.last_ctrl_c = Some(now);
            }
            AppEvent::PlanUpdate(value) => {
                self.plan_panel_state.apply_update_value(&value);
            }
            AppEvent::ToggleTaskPanel => {
                self.task_panel.toggle();
                // Fetch todos from daemon if opening
                if self.task_panel.visible {
                    let client = self.daemon_client.clone();
                    let tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(todos) = client.get_todos().await {
                            let _ = tx.send(AppEvent::TodosUpdated(todos.items));
                        }
                    });
                }
            }
            AppEvent::TodosUpdated(items) => {
                self.task_panel.update(items);
            }
            _ => {}
        }
    }
}
