//! Keyboard event handling for the TUI application.

use super::types::*;
use super::App;
use crate::tui::components::subagent_focus_view::{visible_node_ids, FocusViewState};
use crate::tui::components::subagent_status_bar::active_node_ids;
use crate::tui::traits::Component;
use crate::tui::util::{wrap_next, wrap_prev};
use crossterm::event::{KeyCode, KeyModifiers};

impl App {
    /// Handle a keyboard event and update focused TUI state.
    pub(super) fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        // Full-screen subagent focus view: while open it swallows all keys
        // (Esc exits; ↑↓ navigate the selector; Enter switches to the
        // selected subagent or exits on "main"; 't' folds tool calls;
        // mouse wheel scrolls the read-only timeline).
        let mut exit_focus = false;
        if let Some(ref mut focus) = self.subagent_focus {
            match key.code {
                KeyCode::Esc => exit_focus = true,
                // ↑↓ navigate the selector (main + visible subagents), wrap.
                // The selector is the sole keyboard-interactive area; the
                // timeline is read-only (mouse-wheel scroll only).
                KeyCode::Up => {
                    let now = std::time::Instant::now();
                    let visible = visible_node_ids(
                        &self.subagent_tree,
                        &self.completed_at,
                        now,
                        &focus.node_id,
                    );
                    let len = visible.len() + 1; // +1 for "main"
                    focus.selector_index = wrap_prev(focus.selector_index, len);
                    return;
                }
                KeyCode::Down => {
                    let now = std::time::Instant::now();
                    let visible = visible_node_ids(
                        &self.subagent_tree,
                        &self.completed_at,
                        now,
                        &focus.node_id,
                    );
                    let len = visible.len() + 1;
                    focus.selector_index = wrap_next(focus.selector_index, len);
                    return;
                }
                KeyCode::Enter => {
                    if focus.selector_index == 0 {
                        // "main" selected → exit focus view
                        exit_focus = true;
                    } else {
                        let now = std::time::Instant::now();
                        let visible = visible_node_ids(
                            &self.subagent_tree,
                            &self.completed_at,
                            now,
                            &focus.node_id,
                        );
                        let new_state = visible.get(focus.selector_index - 1);
                        if let Some(id) = new_state {
                            // Switch the timeline to the selected node in-memory.
                            // We intentionally do NOT descend via NavigateAgent:
                            // descending switches `agent_navigation` to the child
                            // scope, which makes the background progress poller's
                            // root-scope `AgentLocalView` updates fail the
                            // `matches_current_scope` check (event.rs) and get
                            // skipped, freezing the focus view. The root tree
                            // always carries every direct child's live progress,
                            // so an in-memory switch keeps the timeline live.
                            if let Some(state) = FocusViewState::build(id, &self.subagent_tree) {
                                *focus = state;
                            }
                        }
                        return;
                    }
                }
                // Toggle fold: if any tools are expanded, collapse all;
                // otherwise expand all. Uses the conversion shared with
                // build_conversation_lines to find tool_call_ids.
                KeyCode::Char('t') => {
                    focus.toggle_fold_all();
                    return;
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // pass through to global Ctrl+P handler
                }
                KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // pass through to global Ctrl+L handler
                }
                KeyCode::Backspace => {
                    // One-level back navigation while the focus view is open.
                    let _ = self.event_tx.send(AppEvent::NavigateAgentBack);
                    return;
                }
                _ => return,
            }
        }
        if exit_focus {
            // Restore the root view by popping all navigation frames.
            // This ensures the status bar selector shows the root's
            // direct children (main + subagents) after exiting the
            // focus view, not the navigated subagent's scoped view.
            // The first frame in the back_stack is the root (pushed
            // first when descending); restoring it returns the tree
            // to the top-level local view.
            if let Some(root) = self.agent_navigation.back_stack.first().cloned() {
                self.agent_navigation.current = Some(root.clone());
                self.subagent_tree.replace_local(root.view);
            }
            self.agent_navigation.back_stack.clear();
            self.subagent_focus = None;
            self.inspector.visible = self.inspector.was_visible_before_focus;
            return;
        }
        // /model picker popup: ↑↓ navigate, Enter confirms the selection
        // (dispatches ModelSwitchRequested), Esc cancels.
        if self.model_picker.is_some() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(ref mut p) = self.model_picker {
                        p.up();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(ref mut p) = self.model_picker {
                        p.down();
                    }
                }
                KeyCode::Enter => self.confirm_model_selection(),
                KeyCode::Esc => self.model_picker = None,
                _ => {}
            }
            return;
        }
        // /undo turn-picker popup: ↑↓ navigate, Enter confirms the selection
        // (advances to the scope picker), Esc cancels the whole flow.
        if self.turn_picker.is_some() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(ref mut p) = self.turn_picker {
                        p.up();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(ref mut p) = self.turn_picker {
                        p.down();
                    }
                }
                KeyCode::Enter => self.confirm_turn_selection(),
                KeyCode::Esc => {
                    self.turn_picker = None;
                    self.undo_picker_open = false;
                    self.pending_undo_turn_id = None;
                }
                _ => {}
            }
            return;
        }
        // /undo scope-picker popup: ↑↓ navigate, Enter runs the rollback
        // (dispatched to the async event loop since `undo_to_turn` is async),
        // Esc falls back to the turn picker (selection preserved).
        if self.scope_picker.is_some() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(ref mut p) = self.scope_picker {
                        p.up();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(ref mut p) = self.scope_picker {
                        p.down();
                    }
                }
                KeyCode::Enter => {
                    let scope = self.scope_picker.as_ref().map(|p| p.selected());
                    let turn_id = self.pending_undo_turn_id.take();
                    // Close pickers immediately for responsiveness; the actual
                    // rollback runs in the async `UndoRequested` handler.
                    self.scope_picker = None;
                    self.undo_picker_open = false;
                    if let (Some(scope), Some(turn_id)) = (scope, turn_id) {
                        let _ = self
                            .event_tx
                            .send(AppEvent::UndoRequested { turn_id, scope });
                    }
                }
                KeyCode::Esc => {
                    self.scope_picker = None;
                    self.rebuild_turn_picker();
                }
                _ => {}
            }
            return;
        }
        // Permission panel handling (inline, not popup)
        // Shift+Tab: cycle agent mode (but not when completion panel or inspector is active)
        if key.code == KeyCode::BackTab
            && !self
                .completion_state
                .as_ref()
                .map(|s| s.visible)
                .unwrap_or(false)
            && !self.inspector.visible
        {
            self.mode = self.mode.next();
            self.sync_permission_mode_to_daemon();
            self.apply_mode_to_prompt_permissions();
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
            self.sync_permission_mode_to_daemon();
            self.apply_mode_to_prompt_permissions();
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
                                // Preserve the trigger prefix (@ skills vs / commands).
                                let insert = crate::tui::components::input::completion_insert_text(
                                    state.prefix,
                                    &m.text,
                                );
                                self.input_box.textarea.insert_str(&insert);
                                self.input_box.update_style();
                            }
                        }
                    }
                    // state was already taken (set to None by take())
                    return;
                }
                _ => {}
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
            // When a delete is pending, intercept all keys for the
            // confirm / cancel flow.
            if self.session_state.pending_delete {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        if let Some(id) = self.session_state.confirm_delete() {
                            let _ = self.event_tx.send(AppEvent::DeleteSession(id));
                        }
                    }
                    // Any other key cancels the pending delete.
                    _ => {
                        self.session_state.cancel_delete();
                    }
                }
                return;
            }
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
                        let name = session.name.clone();
                        if self.has_running_turn() || !self.pending_inputs.is_empty() {
                            self.push_system_message(
                                "Finish or cancel the active turn before switching sessions.",
                            );
                            return;
                        }
                        self.session_state.dismiss();
                        if id == self.session_id {
                            return;
                        }
                        // Record the latest requested switch target so stale
                        // completions from earlier switches are dropped when
                        // multiple switches are in flight simultaneously.
                        self.session_state.pending_switch = Some(id.clone());
                        // Automatic daemon continuation is invisible to local
                        // busy gates. Keep observing the old session until its
                        // run claim is released after the final save.
                        self.mark_daemon_owned_session_history();
                        let client = self.daemon_client.clone();
                        let tx = self.event_tx.clone();
                        let from_id = self.session_id.clone();
                        tokio::spawn(async move {
                            match client
                                .cancel_run_and_wait_for_release(
                                    &from_id,
                                    std::time::Duration::from_secs(3),
                                )
                                .await
                            {
                                Ok(()) => {
                                    let _ =
                                        tx.send(AppEvent::SessionSwitched { from_id, id, name });
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        session_id = %from_id,
                                        error = %error,
                                        "session switch cancellation failed"
                                    );
                                    let _ = tx.send(AppEvent::SystemNotice(format!(
                                        "⚠ session switch cancelled; old session remains active: {error}"
                                    )));
                                }
                            }
                        });
                    }
                }
                KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                    self.session_state.request_delete();
                }
                KeyCode::Esc => {
                    self.session_state.dismiss();
                }
                _ => {}
            }
            return;
        }
        // Memory browser popup
        if self.memory_state.visible {
            // When a delete is pending, intercept all keys for the
            // confirm / cancel flow.
            if self.memory_state.pending_delete {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        if let Some((origin, id)) = self.memory_state.confirm_delete() {
                            let _ = self.event_tx.send(AppEvent::DeleteMemory(origin, id));
                        }
                    }
                    // Any other key cancels the pending delete.
                    _ => {
                        self.memory_state.cancel_delete();
                    }
                }
                return;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.memory_state.move_up();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.memory_state.move_down();
                }
                KeyCode::Tab => {
                    self.memory_state.cycle_filter();
                }
                KeyCode::Enter => {
                    self.memory_state.toggle_detail();
                }
                KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                    self.memory_state.request_delete();
                }
                // 👍 reinforce selected memory (user positive feedback).
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    if let Some(item) = self.memory_state.selected_item() {
                        let origin = item.origin;
                        let id = item.entry.id.clone();
                        let _ = self.event_tx.send(AppEvent::ReinforceMemory(origin, id));
                    }
                }
                // 👎 penalize selected memory (user negative feedback).
                KeyCode::Char('-') | KeyCode::Char('_') => {
                    if let Some(item) = self.memory_state.selected_item() {
                        let origin = item.origin;
                        let id = item.entry.id.clone();
                        let _ = self.event_tx.send(AppEvent::PenalizeMemory(origin, id));
                    }
                }
                KeyCode::Esc => {
                    if self.memory_state.detail_mode {
                        self.memory_state.detail_mode = false;
                    } else {
                        self.memory_state.dismiss();
                    }
                }
                _ => {}
            }
            return;
        }
        // Subagent status bar: ↑↓ auto-focus and navigate, Enter opens
        // the focus view (or dismisses focus on "main"), Esc unfocuses.
        // No Tab — auto-focus on arrow keys.
        if self.subagent_focus.is_none() {
            let active = active_node_ids(&self.subagent_tree);
            if !active.is_empty() {
                // Unified list: ["main", ...active]. wrap len = N+1.
                let len = active.len() + 1;
                // Auto-activate on ↑↓ (skip when inspector is visible:
                // let inspector consume the arrows for turn navigation)
                if (key.code == KeyCode::Up || key.code == KeyCode::Down) && !self.inspector.visible
                {
                    self.subagent_status_bar_focused = true;
                }
                if self.subagent_status_bar_focused {
                    match key.code {
                        KeyCode::Up => {
                            self.subagent_status_bar_selected =
                                wrap_prev(self.subagent_status_bar_selected, len);
                            return;
                        }
                        KeyCode::Down => {
                            self.subagent_status_bar_selected =
                                wrap_next(self.subagent_status_bar_selected, len);
                            return;
                        }
                        KeyCode::Enter => {
                            // Use the modded index (matches render's
                            // `selected % (N+1)`) so Enter stays
                            // consistent with the displayed selection
                            // even if the active set shrank between
                            // navigation and Enter (e.g. a subagent
                            // completed mid-interaction).
                            let cur = self.subagent_status_bar_selected % len;
                            if cur == 0 {
                                // "main" selected — dismiss status bar
                                // focus (consistent with focus view's
                                // "main" exit semantics).
                                self.subagent_status_bar_focused = false;
                            } else if let Some(node_id) = active.get(cur - 1) {
                                if let Some(state) =
                                    FocusViewState::build(node_id, &self.subagent_tree)
                                {
                                    self.inspector.was_visible_before_focus =
                                        self.inspector.visible;
                                    self.inspector.visible = false;
                                    self.subagent_focus = Some(state);
                                }
                                // Intentionally do NOT descend via NavigateAgent
                                // here. Descending switches `agent_navigation`
                                // to the child scope, which makes the background
                                // progress poller's root-scope `AgentLocalView`
                                // updates fail the `matches_current_scope` check
                                // (event.rs) and get skipped -- freezing the focus
                                // view's elapsed_ms/rounds/timeline. Keeping the
                                // root scope lets `replace_local` + `focus.rebuild`
                                // stay live.
                            }
                            return;
                        }
                        KeyCode::Esc => {
                            self.subagent_status_bar_focused = false;
                            return;
                        }
                        KeyCode::Tab => {
                            // Tab has no effect on status bar focus (per
                            // spec): it neither toggles into nor out of
                            // the status bar. Consume without state change.
                            return;
                        }
                        _ => {
                            // Any other key disengages focus and passes
                            // through to the input box
                            self.subagent_status_bar_focused = false;
                        }
                    }
                }
            }
        }
        // ESC interrupts a running turn instead of quitting.
        if key.code == KeyCode::Esc && self.current_turn_handle.is_some() {
            self.interrupt_running_turn();
            return;
        }
        // Scroll handling: PageUp/PageDown only. ↑↓ reserved for
        // status bar navigation. Scroll by mouse wheel instead.
        match key.code {
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
        // F2: toggle inspector panel
        if key.code == KeyCode::F(2) {
            self.inspector.visible = !self.inspector.visible;
            if self.inspector.visible {
                // Sync to latest context when opening
                self.inspector.selected_turn = self.turn_contexts.len().saturating_sub(1);
                self.inspector.sync(&self.turn_contexts);
            }
            return;
        }
        // Inspector key handling (when visible and no subagent focus).
        // Must come BEFORE PageUp/PageDown/Enter/BackTab global handlers
        // so the inspector can intercept these keys for its own navigation.
        if self.inspector.visible
            && self.subagent_focus.is_none()
            && self.inspector.handle_key(&key)
        {
            return; // Inspector consumed the key
        }
        // Ctrl+L: clear screen
        if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.committed_messages.clear();
            self.streaming_content.clear();
            self.scroll_offset = 0;
            self.user_scrolled = false;
            self.sandbox_bypassed_session = false;
            return;
        }
        // Handle Enter/Shift+Enter/Ctrl+J BEFORE tui-textarea consumes
        // them. tui-textarea's default binding inserts a newline on
        // Enter, so we intercept:
        //   - Shift+Enter -> newline. Needs the kitty keyboard protocol
        //     enabled in args.rs; terminals without it report a bare
        //     Enter with no SHIFT bit, so Shift+Enter acts as submit.
        //   - Ctrl+J -> newline. Universal fallback that works in ANY
        //     terminal, including ones without kitty support (macOS
        //     Terminal.app). In raw mode crossterm decodes Ctrl+J's
        //     0x0A byte as Char('j')+CONTROL (crossterm only maps \n
        //     to Enter outside raw mode), so it never reaches this
        //     Enter branch and is safe to claim here.
        //   - unmodified Enter -> submit.
        // Memory / session panels are slash-only (`/memory`, `/session`).
        if key.code == KeyCode::Enter {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.input_box.textarea.insert_char('\n');
                self.input_box.update_style();
            } else if !self.input_box.is_empty() {
                let text = self.input_box.take_text();
                let _ = self.event_tx.send(AppEvent::Submit(text));
            }
            return;
        }
        if key.code == KeyCode::Char('j') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.input_box.textarea.insert_char('\n');
            self.input_box.update_style();
            return;
        }
        // Detect @ and / completion triggers BEFORE feeding to textarea
        if let KeyCode::Char(c) = key.code {
            let is_completion_char = (c == '@' && !key.modifiers.contains(KeyModifiers::CONTROL))
                || (c == '/' && key.modifiers.is_empty());
            if is_completion_char {
                let text = self.input_box.textarea.lines().join("\n");
                let should_trigger = text.is_empty() || text.ends_with(' ') || text.ends_with('\n');
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
        self.input_box.textarea.input(key);
        self.input_box.update_style();
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

    /// Fire-and-forget: push the current agent mode to the daemon so subagents
    /// inherit root permission mode and shell tools use the correct sandbox
    /// EffectiveMode (Plan stays Plan).
    pub(super) fn sync_permission_mode_to_daemon(&self) {
        let client = self.daemon_client.clone();
        let session_id = self.session_id.clone();
        let mode = self.mode.to_root_permission_mode();
        let effective_mode = self.mode.to_effective_mode();
        tokio::spawn(async move {
            if let Err(e) = client
                .set_permission_mode(&session_id, mode, effective_mode)
                .await
            {
                tracing::warn!(error = ?e, "failed to sync permission mode to daemon");
            }
        });
    }

    /// Keep system-prompt permissions layer in sync with Shift+Tab / Plan toggle.
    ///
    /// Updates `prompt_context` + `assembled_instructions` only. The agent
    /// loop prepends `assembled_instructions.system_messages` each API round, so history
    /// stays dialogue-only and must not be rewritten with system layers.
    pub(super) fn apply_mode_to_prompt_permissions(&mut self) {
        let sandbox = self.mode.prompt_sandbox_mode().to_string();
        let approval = self.mode.prompt_approval_policy().to_string();
        if self.prompt_context.sandbox_mode.as_deref() == Some(sandbox.as_str())
            && self.prompt_context.approval_policy.as_deref() == Some(approval.as_str())
        {
            return;
        }
        let mut new_ctx = (*self.prompt_context).clone();
        new_ctx.sandbox_mode = Some(sandbox);
        new_ctx.approval_policy = Some(approval);
        let new_ctx = std::sync::Arc::new(new_ctx);
        let settings = self
            .settings_lock
            .read()
            .expect("lock poisoned: settings")
            .clone();
        let assembled = crate::prompts::assemble_instructions(&settings, &new_ctx);
        self.prompt_context = new_ctx;
        self.assembled_instructions = assembled;
        tracing::info!(
            mode = ?self.mode,
            sandbox = self.prompt_context.sandbox_mode.as_deref().unwrap_or("?"),
            approval = self.prompt_context.approval_policy.as_deref().unwrap_or("?"),
            "agent mode changed; system prompt permissions re-assembled"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::watcher::SettingsHandle;
    use crate::config::Settings;
    use crate::tools::execution::BackgroundResult;
    use crate::tui::client::DaemonClient;
    use crate::tui::client::SessionInfo;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::Router;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    fn build_app() -> App {
        let client = DaemonClient::new("http://localhost:0".to_string());
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        App::new(client, "test-esc".to_string(), settings)
    }

    #[tokio::test]
    async fn esc_interrupts_running_turn() {
        let mut app = build_app();
        app.current_turn_handle = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));

        app.handle_key_event(KeyCode::Esc.into());

        assert!(
            app.current_turn_handle.is_none(),
            "ESC should interrupt the running turn"
        );
        assert!(
            !app.should_quit,
            "ESC should not quit when a turn is running"
        );
    }

    #[tokio::test]
    async fn esc_idle_does_not_quit() {
        let mut app = build_app();
        assert!(
            app.current_turn_handle.is_none(),
            "app should be idle initially"
        );

        app.handle_key_event(KeyCode::Esc.into());

        assert!(
            !app.should_quit,
            "ESC should not quit when idle (fallback removed)"
        );
    }

    #[tokio::test]
    async fn loading_selected_session_restarts_session_scoped_readers() {
        async fn no_active_run() -> StatusCode {
            StatusCode::NOT_FOUND
        }

        let router = Router::new().route("/api/v1/sessions/:id/cancel", post(no_active_run));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind idle session-switch server");
        let address = listener.local_addr().expect("read idle switch address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve idle session-switch server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "test-esc".to_string(),
            settings,
        );
        app.session_state.show(vec![SessionInfo {
            id: "loaded-session".to_string(),
            name: "Loaded Session".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 1,
            summary: None,
        }]);

        app.handle_key_event(KeyCode::Enter.into());

        tokio::time::timeout(Duration::from_secs(1), async {
            while app.session_id != "loaded-session" {
                let event = app
                    .event_rx
                    .recv()
                    .await
                    .expect("idle session switch event channel remains open");
                app.handle_event(event).await;
            }
        })
        .await
        .expect("idle session switch completes transactionally");

        assert_eq!(app.session_id, "loaded-session");
        assert!(app.global_event_reader.is_some());

        if let Some(handle) = app.global_event_reader.take() {
            handle.abort();
        }
        if let Some(handle) = app.session_event_reader.take() {
            handle.abort();
        }
        if let Some(handle) = app.trace_event_reader.take() {
            handle.abort();
        }
        server.abort();
    }

    #[tokio::test]
    async fn switching_sessions_requests_cancellation_for_invisible_daemon_work() {
        async fn capture_cancel(State(count): State<Arc<AtomicUsize>>) -> StatusCode {
            count.fetch_add(1, Ordering::SeqCst);
            StatusCode::NOT_FOUND
        }

        let cancellations = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/api/v1/sessions/:id/cancel", post(capture_cancel))
            .with_state(cancellations.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind session-switch cancellation server");
        let address = listener
            .local_addr()
            .expect("read session-switch server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve session-switch cancellation server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "old-session".to_string(),
            settings,
        );
        app.session_state.show(vec![SessionInfo {
            id: "loaded-session".to_string(),
            name: "Loaded Session".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 1,
            summary: None,
        }]);

        app.handle_key_event(KeyCode::Enter.into());

        tokio::time::timeout(Duration::from_secs(1), async {
            while cancellations.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session switch must cancel the old daemon session");
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn session_switch_waits_for_old_daemon_run_to_release_before_adopting_target() {
        #[derive(Clone)]
        struct CancelState {
            attempts: Arc<AtomicUsize>,
            released: Arc<AtomicBool>,
        }

        async fn cancel_until_released(State(state): State<CancelState>) -> StatusCode {
            state.attempts.fetch_add(1, Ordering::SeqCst);
            if state.released.load(Ordering::SeqCst) {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::NO_CONTENT
            }
        }

        let state = CancelState {
            attempts: Arc::new(AtomicUsize::new(0)),
            released: Arc::new(AtomicBool::new(false)),
        };
        let router = Router::new()
            .route("/api/v1/sessions/:id/cancel", post(cancel_until_released))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind transactional session-switch server");
        let address = listener
            .local_addr()
            .expect("read transactional session-switch address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve transactional session-switch server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "old-session".to_string(),
            settings,
        );
        app.session_state.show(vec![SessionInfo {
            id: "loaded-session".to_string(),
            name: "Loaded Session".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 1,
            summary: None,
        }]);

        app.handle_key_event(KeyCode::Enter.into());
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.attempts.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session switch requests cancellation");
        assert_eq!(
            app.session_id, "old-session",
            "the target cannot be adopted while the old daemon claim is active"
        );

        state.released.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(1), async {
            while app.session_id != "loaded-session" {
                let event = app
                    .event_rx
                    .recv()
                    .await
                    .expect("session switch event channel remains open");
                app.handle_event(event).await;
            }
        })
        .await
        .expect("session switch adopts the target after final release");

        server.abort();
    }

    #[tokio::test]
    async fn session_switch_failure_keeps_the_old_session_active() {
        async fn reject_cancel() -> StatusCode {
            StatusCode::INTERNAL_SERVER_ERROR
        }

        let router = Router::new().route("/api/v1/sessions/:id/cancel", post(reject_cancel));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind rejected session-switch server");
        let address = listener
            .local_addr()
            .expect("read rejected session-switch address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve rejected session-switch server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "old-session".to_string(),
            settings,
        );
        app.session_state.show(vec![SessionInfo {
            id: "loaded-session".to_string(),
            name: "Loaded Session".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 1,
            summary: None,
        }]);

        app.handle_key_event(KeyCode::Enter.into());
        tokio::time::timeout(Duration::from_secs(1), async {
            while !app.committed_messages.iter().any(|message| {
                message.role == MessageRole::System && message.content.contains("session switch")
            }) {
                let event = app
                    .event_rx
                    .recv()
                    .await
                    .expect("session switch failure event channel remains open");
                app.handle_event(event).await;
            }
        })
        .await
        .expect("session switch failure is reported");

        assert_eq!(app.session_id, "old-session");
        server.abort();
    }

    #[tokio::test]
    async fn latest_session_switch_request_wins_when_older_completion_arrives_first() {
        #[derive(Clone)]
        struct OrderedCancelState {
            attempts: Arc<AtomicUsize>,
            release_second: Arc<tokio::sync::Notify>,
        }

        async fn ordered_cancel(State(state): State<OrderedCancelState>) -> StatusCode {
            let attempt = state.attempts.fetch_add(1, Ordering::SeqCst);
            if attempt > 0 {
                state.release_second.notified().await;
            }
            StatusCode::NOT_FOUND
        }

        let state = OrderedCancelState {
            attempts: Arc::new(AtomicUsize::new(0)),
            release_second: Arc::new(tokio::sync::Notify::new()),
        };
        let router = Router::new()
            .route("/api/v1/sessions/:id/cancel", post(ordered_cancel))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ordered session-switch server");
        let address = listener
            .local_addr()
            .expect("read ordered session-switch address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve ordered session-switch server");
        });
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        let mut app = App::new(
            DaemonClient::new(format!("http://{address}")),
            "session-a".to_string(),
            settings,
        );

        app.session_state.show(vec![SessionInfo {
            id: "session-b".to_string(),
            name: "Session B".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 1,
            summary: None,
        }]);
        app.handle_key_event(KeyCode::Enter.into());
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.attempts.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first switch cancellation completes");

        app.session_state.show(vec![SessionInfo {
            id: "session-c".to_string(),
            name: "Session C".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 1,
            summary: None,
        }]);
        app.handle_key_event(KeyCode::Enter.into());
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.attempts.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second switch cancellation is pending");

        loop {
            let event = app
                .event_rx
                .recv()
                .await
                .expect("older switch completion is queued");
            let is_stale_switch =
                matches!(&event, AppEvent::SessionSwitched { id, .. } if id == "session-b");
            app.handle_event(event).await;
            if is_stale_switch {
                break;
            }
        }
        assert_eq!(
            app.session_id, "session-a",
            "completion for A→B must not win after A→C becomes the latest request"
        );

        state.release_second.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while app.session_id != "session-c" {
                let event = app
                    .event_rx
                    .recv()
                    .await
                    .expect("latest switch event channel remains open");
                app.handle_event(event).await;
            }
        })
        .await
        .expect("latest A→C request adopts C");
        server.abort();
    }

    #[tokio::test]
    async fn session_picker_refuses_switch_while_server_run_is_active_after_background_notice() {
        let mut app = build_app();
        app.server_side_loop = true;
        app.server_side_turn_active = true;
        app.handle_event(AppEvent::BackgroundTaskCompleted(BackgroundResult {
            task_id: "session-a-result".to_string(),
            session_id: Some("test-esc".to_string()),
            result_type: "command".to_string(),
            command: "true".to_string(),
            stdout: "done".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
            sandbox_bypassed: false,
            permission_mode: None,
            sandbox_level: None,
        }))
        .await;
        app.session_state.show(vec![SessionInfo {
            id: "session-b".to_string(),
            name: "Session B".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            message_count: 0,
            summary: None,
        }]);

        app.handle_key_event(KeyCode::Enter.into());

        assert_eq!(app.session_id, "test-esc");
        assert!(app.session_state.visible);
        assert!(app.pending_inputs.is_empty());
        assert!(app.committed_messages.iter().any(|message| {
            message
                .content
                .contains("Background task session-a-result completed")
        }));
    }

    #[tokio::test]
    async fn mode_cycle_rebuilds_prompt_permissions() {
        let mut app = build_app();
        // Cycle: Normal → Plan → AcceptEdits → Yolo → Normal
        assert_eq!(app.mode, AgentMode::Normal);
        assert_eq!(
            app.prompt_context.sandbox_mode.as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            app.prompt_context.approval_policy.as_deref(),
            Some("on-request")
        );

        // → Plan: read-only / on-request
        app.handle_key_event(crossterm::event::KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        ));
        assert_eq!(app.mode, AgentMode::PlanMode);
        assert_eq!(
            app.prompt_context.sandbox_mode.as_deref(),
            Some("read-only")
        );
        assert_eq!(
            app.prompt_context.approval_policy.as_deref(),
            Some("on-request")
        );

        // → AcceptEdits: workspace-write / on-request
        app.handle_key_event(crossterm::event::KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        ));
        assert_eq!(app.mode, AgentMode::AcceptEdits);
        assert_eq!(
            app.prompt_context.sandbox_mode.as_deref(),
            Some("workspace-write")
        );

        // → Yolo: disabled + never
        app.handle_key_event(crossterm::event::KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        ));
        assert_eq!(app.mode, AgentMode::Yolo);
        assert_eq!(app.prompt_context.sandbox_mode.as_deref(), Some("disabled"));
        assert_eq!(app.prompt_context.approval_policy.as_deref(), Some("never"));
        let yolo_perm = app
            .assembled_instructions
            .system_messages
            .iter()
            .find_map(|m| {
                m.content
                    .as_deref()
                    .filter(|c| c.contains("<permissions_instructions>"))
            })
            .expect("Yolo should inject permissions layer");
        assert!(yolo_perm.contains("disabled"), "{yolo_perm}");
        assert!(yolo_perm.contains("never"), "{yolo_perm}");

        // → Normal again
        app.handle_key_event(crossterm::event::KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        ));
        assert_eq!(app.mode, AgentMode::Normal);
        assert_eq!(
            app.prompt_context.sandbox_mode.as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            app.prompt_context.approval_policy.as_deref(),
            Some("on-request")
        );
    }

    #[tokio::test]
    async fn plan_toggle_sets_read_only_sandbox_prompt() {
        let mut app = build_app();
        app.handle_key_event(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(app.mode, AgentMode::PlanMode);
        assert_eq!(
            app.prompt_context.sandbox_mode.as_deref(),
            Some("read-only")
        );
        assert_eq!(
            app.prompt_context.approval_policy.as_deref(),
            Some("on-request")
        );
        let perm = app
            .assembled_instructions
            .system_messages
            .iter()
            .find_map(|m| {
                m.content
                    .as_deref()
                    .filter(|c| c.contains("<permissions_instructions>"))
            })
            .expect("Plan should inject permissions layer");
        assert!(perm.contains("read-only"), "{perm}");
        assert!(
            perm.contains("across the disk"),
            "Plan read-only copy should describe full-disk read: {perm}"
        );
    }
}
