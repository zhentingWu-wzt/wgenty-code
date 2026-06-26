//! Event handling for the TUI application.

use super::types::*;
use super::App;
use crate::agent::progress::SubagentEventType;
use crate::prompts::{self, PromptContext};
use crate::tui::traits::Component;
use crate::tui::util::{
    agent_phase_from_event, compute_collapse_state, extract_diff_data, extract_tool_metadata,
    format_tool_result, tool_label,
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::collections::HashMap;

impl App {
    pub(super) async fn handle_event(&mut self, event: AppEvent) {
        // Derive phase from event (pure function); fall back to current
        if let Some(next_phase) = agent_phase_from_event(&event) {
            if self.phase != next_phase {
                tracing::info!(
                    prev = ?self.phase,
                    next = ?next_phase,
                    "Agent phase transition"
                );
            }
            self.phase = next_phase;
        }
        match event {
            AppEvent::KeyEvent(key) => {
                // Permission panel handling (inline, not popup)
                // Shift+Tab: cycle agent mode (but not when completion panel is active)
                if key.code == KeyCode::BackTab
                    && !self
                        .completion_state
                        .as_ref()
                        .map(|s| s.visible)
                        .unwrap_or(false)
                {
                    self.mode = self.mode.next();
                    return;
                }
                // Ctrl+P: toggle plan mode (restores previous mode when leaving PlanMode)
                if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
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
                        tool_collapsed: true,
                        tool_running: false,
                        tool_args: None,
                        diff_data: None,
                        tool_metadata: None,
                    });
                    return;
                }
                // Ctrl+Shift+T toggles subagent monitor panel
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                    && (key.code == KeyCode::Char('T') || key.code == KeyCode::Char('t'))
                {
                    let _ = self.event_tx.send(AppEvent::ToggleSubagentPanel);
                    return;
                }
                // If completion panel is visible, route keys to it
                if self
                    .completion_state
                    .as_ref()
                    .map(|s| s.visible)
                    .unwrap_or(false)
                {
                    match key.code {
                        KeyCode::Esc => {
                            self.completion_state = None;
                            return;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let Some(ref mut s) = self.completion_state {
                                s.move_previous();
                            }
                            return;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(ref mut s) = self.completion_state {
                                s.move_next();
                            }
                            return;
                        }
                        KeyCode::Left => {
                            if let Some(ref mut s) = self.completion_state {
                                s.move_to_previous_tab();
                            }
                            return;
                        }
                        KeyCode::Right => {
                            if let Some(ref mut s) = self.completion_state {
                                s.move_to_next_tab();
                            }
                            return;
                        }
                        KeyCode::Tab => {
                            // Cycle to next item
                            if let Some(ref mut s) = self.completion_state {
                                s.move_next();
                            }
                            return;
                        }
                        KeyCode::BackTab => {
                            // Cycle to previous item
                            if let Some(ref mut s) = self.completion_state {
                                s.move_previous();
                            }
                            return;
                        }
                        KeyCode::Enter => {
                            // Confirm selection: take ownership (one move, no deep clone)
                            if let Some(state) = self.completion_state.take() {
                                if let Some(m) = state.selected_match() {
                                    let text = self.input_box.textarea.lines().join("\n");
                                    if let Some(pos) = text.rfind(state.prefix) {
                                        let before = &text[..pos];
                                        self.input_box.textarea.select_all();
                                        self.input_box.textarea.cut();
                                        self.input_box.textarea.insert_str(before);
                                        // Both @ and / completion insert /name to input
                                        let insert = format!("/{} ", m.text);
                                        self.input_box.textarea.insert_str(&insert);
                                    }
                                }
                            }
                            // state was already taken (set to None by take())
                            return;
                        }
                        _ => {}
                    }
                }
                // If detail view is active, route keys to it (highest priority)
                if self.subagent_panel_visible {
                    if let Some(ref mut detail) = self.subagent_panel_state.detail_view {
                        match key.code {
                            KeyCode::Esc => {
                                self.subagent_panel_state.reset_detail();
                                return;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                detail.scroll_offset = detail.scroll_offset.saturating_sub(1);
                                return;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                detail.scroll_offset = detail.scroll_offset.saturating_add(1);
                                return;
                            }
                            KeyCode::PageUp => {
                                detail.scroll_offset = detail.scroll_offset.saturating_sub(10);
                                return;
                            }
                            KeyCode::PageDown => {
                                detail.scroll_offset = detail.scroll_offset.saturating_add(10);
                                return;
                            }
                            KeyCode::Char('g') => {
                                detail.scroll_offset = 0;
                                return;
                            }
                            KeyCode::Char('G') => {
                                detail.scroll_offset = detail.events.len().saturating_sub(1);
                                return;
                            }
                            KeyCode::Char('f') => {
                                // Jump to first Error event
                                if let Some(pos) = detail.events.iter().position(|e| {
                                    matches!(e.event_type, SubagentEventType::Error { .. })
                                }) {
                                    detail.scroll_offset = pos;
                                }
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                // If subagent panel is visible, route keys to it
                if self.subagent_panel_visible {
                    match key.code {
                        KeyCode::Esc => {
                            self.subagent_panel_visible = false;
                            return;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.subagent_panel_state.move_down(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.subagent_panel_state.move_up(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Enter => {
                            // Completed/Failed/Cancelled: open detail view
                            // Running/Pending: toggle expand
                            let is_terminal = self
                                .subagent_panel_state
                                .selected_node_id(&self.subagent_tree)
                                .and_then(|node_id| {
                                    self.subagent_tree.nodes.get(&node_id).map(|node| {
                                        matches!(
                                            node.progress.status,
                                            crate::agent::progress::SubagentStatus::Completed
                                                | crate::agent::progress::SubagentStatus::Failed
                                                | crate::agent::progress::SubagentStatus::Cancelled
                                        )
                                    })
                                })
                                .unwrap_or(false);
                            if is_terminal {
                                if let Some(detail) = self
                                    .subagent_panel_state
                                    .build_detail_view(&self.subagent_tree)
                                {
                                    self.subagent_panel_state.detail_view = Some(detail);
                                }
                            } else {
                                self.subagent_panel_state.toggle_expand(&self.subagent_tree);
                            }
                            return;
                        }
                        KeyCode::Char('g') => {
                            self.subagent_panel_state.move_first(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Char('G') => {
                            self.subagent_panel_state.move_last(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Char('d') => {
                            if let Some(detail) = self
                                .subagent_panel_state
                                .build_detail_view(&self.subagent_tree)
                            {
                                self.subagent_panel_state.detail_view = Some(detail);
                            }
                            return;
                        }
                        KeyCode::Char('r') => {
                            // Retry selected failed/cancelled node (defensive guard)
                            if let Some(node_id) = self
                                .subagent_panel_state
                                .selected_node_id(&self.subagent_tree)
                            {
                                let is_retryable = self
                                    .subagent_tree
                                    .nodes
                                    .get(&node_id)
                                    .map(|node| {
                                        matches!(
                                            node.progress.status,
                                            crate::agent::progress::SubagentStatus::Failed
                                                | crate::agent::progress::SubagentStatus::Cancelled
                                        )
                                    })
                                    .unwrap_or(false);
                                if is_retryable {
                                    let _ = self.event_tx.send(AppEvent::RetrySubagent(node_id));
                                }
                            }
                            return;
                        }
                        _ => {} // pass through
                    }
                }
                // Permission panel key handling — delegated to Component
                if self.permission_state.handle_key(&key) {
                    return;
                }
                // Question panel handling — delegated to Component.
                // handle_key sets just_submitted=true only for explicit confirmation
                // keys (Enter, number in single-select). Navigation keys (↑↓, j, k,
                // Space) only mutate cursor/selection and keep just_submitted=false.
                if self.question_state.handle_key(&key) {
                    if self.question_state.just_submitted {
                        if let Some(answers) = self.question_state.take_response() {
                            self.push_question_answer(&answers);
                        }
                    }
                    return;
                }
                // Session popup handling
                if self.session_state.visible {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.session_state.move_up();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.session_state.move_down();
                        }
                        KeyCode::Enter => {
                            if let Some(session) = self.session_state.selected_session() {
                                let id = session.id.clone();
                                // Adopt the loaded session's ID so subsequent saves
                                // go back to the original file, not a forked copy.
                                self.session_id = id.clone();
                                self.session_name = session.name.clone();
                                self.session_state.dismiss();
                                let client = self.daemon_client.clone();
                                let history = self.conversation_history.clone();
                                let tx = self.event_tx.clone();
                                tokio::spawn(async move {
                                    if let Ok(resp) = client.load_session(&id).await {
                                        let messages = resp.messages;
                                        {
                                            let mut h = history.lock().await;
                                            *h = messages.clone();
                                        }
                                        let _ = tx.send(AppEvent::HistoryLoaded(messages));
                                    }
                                });
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(id) = self.session_state.delete_selected() {
                                let _ = self.event_tx.send(AppEvent::DeleteSession(id));
                            }
                        }
                        KeyCode::Delete | KeyCode::Backspace => {
                            if let Some(id) = self.session_state.delete_selected() {
                                let _ = self.event_tx.send(AppEvent::DeleteSession(id));
                            }
                        }
                        KeyCode::Esc => {
                            self.session_state.dismiss();
                        }
                        _ => {}
                    }
                    return;
                }
                // Scroll handling (when no popup is active)
                // scroll_offset = lines scrolled UP from bottom:
                //   0 = at bottom (newest), higher values = further up (older)
                match key.code {
                    KeyCode::Up => {
                        // Scroll UP → see OLDER content → more lines from bottom
                        self.scroll_offset = self.scroll_offset.saturating_add(1);
                        self.user_scrolled = true;
                        return;
                    }
                    KeyCode::Down => {
                        // Scroll DOWN → see NEWER content → fewer lines from bottom
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                        // If scrolled back to bottom, resume auto-scroll for future content
                        if self.scroll_offset == 0 {
                            self.user_scrolled = false;
                        }
                        return;
                    }
                    KeyCode::PageUp => {
                        self.scroll_offset = self.scroll_offset.saturating_add(10);
                        self.user_scrolled = true;
                        return;
                    }
                    KeyCode::PageDown => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(10);
                        // If scrolled back to bottom, resume auto-scroll
                        if self.scroll_offset == 0 {
                            self.user_scrolled = false;
                        }
                        return;
                    }
                    _ => {}
                }
                // Ctrl+L: clear screen
                if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.committed_messages.clear();
                    self.streaming_content.clear();
                    self.scroll_offset = 0;
                    self.user_scrolled = false;
                    return;
                }
                // Handle Enter/Shift+Enter BEFORE tui-textarea consumes them.
                // tui-textarea's default binding inserts newline on Enter.
                if key.code == KeyCode::Enter {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        // Shift+Enter → newline
                        self.input_box.textarea.insert_char('\n');
                    } else if !self.input_box.is_empty() {
                        let text = self.input_box.take_text();
                        let _ = self.event_tx.send(AppEvent::Submit(text));
                    }
                    return;
                }
                // Detect @ and / completion triggers BEFORE feeding to textarea
                if let KeyCode::Char(c) = key.code {
                    let is_completion_char = (c == '@'
                        && !key.modifiers.contains(KeyModifiers::CONTROL))
                        || (c == '/' && key.modifiers.is_empty());
                    if is_completion_char {
                        let text = self.input_box.textarea.lines().join("\n");
                        let should_trigger =
                            text.is_empty() || text.ends_with(' ') || text.ends_with('\n');
                        if should_trigger {
                            let partial = String::new();
                            let matches = self
                                .completion_engine
                                .as_ref()
                                .map(|e| e.filter(c, &partial))
                                .unwrap_or_default();
                            self.completion_state = Some(CompletionState::new(c, partial, matches));
                        }
                    }
                }
                // Feed to tui-textarea for CJK/IME input.
                // Returns true if tui-textarea consumed the key.
                let handled = self.input_box.textarea.input(*key);
                if !handled && key.code == KeyCode::Esc {
                    self.should_quit = true;
                }
                // Update filter as user types more characters after @ or /
                if self
                    .completion_state
                    .as_ref()
                    .map(|s| s.visible)
                    .unwrap_or(false)
                {
                    let text = self.input_box.textarea.lines().join("\n");
                    if let Some(ref mut state) = self.completion_state {
                        if let Some(pos) = text.rfind(state.prefix) {
                            let after = &text[pos + 1..];
                            state.partial = after.to_string();
                            if let Some(ref engine) = self.completion_engine {
                                state.replace_matches(engine.filter(state.prefix, after));
                            }
                        } else {
                            // Prefix no longer in text (e.g. user deleted @ with backspace) → dismiss
                            self.completion_state = None;
                        }
                    }
                }
            }
            AppEvent::Paste(text) => {
                for c in text.chars() {
                    self.input_box.textarea.insert_char(c);
                }
            }
            AppEvent::MouseScrolled(delta) => {
                // When a popup or detail view is active, route scroll appropriately
                // instead of scrolling the main chat behind it.
                if self.subagent_panel_state.detail_view.is_some() {
                    // Detail view: scroll the event timeline (0=top, higher=older)
                    if let Some(ref mut detail) = self.subagent_panel_state.detail_view {
                        if delta > 0 {
                            detail.scroll_offset =
                                detail.scroll_offset.saturating_sub(delta as usize);
                        } else {
                            detail.scroll_offset =
                                detail.scroll_offset.saturating_add((-delta) as usize);
                        }
                    }
                    return;
                }
                if self.subagent_panel_visible || self.session_state.visible {
                    // Subagent panel / session list: keyboard navigation only, ignore mouse scroll
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
            AppEvent::ToggleCollapseLatest => {
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
            AppEvent::Submit(text) => {
                self.subagent_tree.clear();
                self.subagent_panel_state.reset();
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
                        last.content = format_tool_result(&name, &args, &content);
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
            AppEvent::Tick if self.has_running_tool => {
                self.spinner_frame = self.spinner_frame.wrapping_add(1);
            }
            AppEvent::Tick => {}
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
                        hm.fire(&crate::runtime::hooks::HookEvent::Stop, &ctx, None, None).await;
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
                        hm.fire(&crate::runtime::hooks::HookEvent::Stop, &ctx, None, None).await;
                    });
                }
                self.last_abort_reason = Some(reason.clone());
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
                    let client = self.daemon_client.clone();
                    let tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(sessions) = client.list_sessions().await {
                            let _ = tx.send(AppEvent::SessionListLoaded(sessions));
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
            AppEvent::SubagentUpdate(progress) => {
                self.subagent_tree.upsert(*progress);
            }
            AppEvent::ToggleSubagentPanel => {
                self.subagent_panel_visible = !self.subagent_panel_visible;
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
            AppEvent::RetrySubagent(node_id) => {
                // Close the panel and submit a retry request
                self.subagent_panel_visible = false;
                self.subagent_panel_state.reset();
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("🔄 Retrying subagent `{}`...", node_id),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: true,
                    tool_running: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            _ => {}
        }
    }

    /// Record the user's question answer as a chat message.
    fn push_question_answer(&mut self, answers: &[String]) {
        let q = &self.question_state.question;
        let a = answers.join(", ");
        self.committed_messages.push(UIMessage {
            role: MessageRole::System,
            content: format!("Q: {}\nA: {}", q, a),
            tool_name: Some("ask".to_string()),
            content_collapsed: false,
            tool_collapsed: false,
            tool_running: false,
            tool_args: None,
            diff_data: None,
            tool_metadata: None,
        });
    }

    #[allow(dead_code)]
    fn push_permission_result(&mut self, reason: &str, decision: &str) {
        self.committed_messages.push(UIMessage {
            role: MessageRole::System,
            content: format!("🔐 {} — {}", reason, decision),
            tool_name: Some("permission".to_string()),
            content_collapsed: false,
            tool_collapsed: false,
            tool_running: false,
            tool_args: None,
            diff_data: None,
            tool_metadata: None,
        });
    }
}
