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
                            // Descend into the selected direct child via its
                            // opaque navigation capability. The daemon verifies
                            // the capability and returns the child's local view;
                            // we do not rebuild from the in-memory tree.
                            if let Some(capability) = self.subagent_tree.capability_for_child(id) {
                                let _ = self.event_tx.send(AppEvent::NavigateAgent { capability });
                            } else if let Some(state) =
                                FocusViewState::build(id, &self.subagent_tree)
                            {
                                // No capability (self node or stale view):
                                // fall back to the in-memory focus switch.
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
            return;
        }
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
                                // Both @ and / completion insert /name to input
                                let insert = format!("/{} ", m.text);
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
        // Subagent status bar: ↑↓ auto-focus and navigate, Enter opens
        // the focus view (or dismisses focus on "main"), Esc unfocuses.
        // No Tab — auto-focus on arrow keys.
        if self.subagent_focus.is_none() {
            let active = active_node_ids(&self.subagent_tree);
            if !active.is_empty() {
                // Unified list: ["main", ...active]. wrap len = N+1.
                let len = active.len() + 1;
                // Auto-activate on ↑↓
                if key.code == KeyCode::Up || key.code == KeyCode::Down {
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
                                    self.subagent_focus = Some(state);
                                }
                                // Descend into the selected subagent so the
                                // selector reflects the scoped local view
                                // (parent + this subagent's direct children)
                                // instead of all siblings from the root view.
                                // The AgentViewNavigated handler will rebuild
                                // the focus view once the daemon responds.
                                if let Some(capability) =
                                    self.subagent_tree.capability_for_child(node_id)
                                {
                                    let _ =
                                        self.event_tx.send(AppEvent::NavigateAgent { capability });
                                }
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
        // Other Enter modifier combos (Ctrl/Alt+Enter) fall through to
        // submit, matching prior behaviour.
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
        let mode = self.mode.to_root_permission_mode();
        let effective_mode = self.mode.to_effective_mode();
        tokio::spawn(async move {
            if let Err(e) = client.set_permission_mode(mode, effective_mode).await {
                tracing::warn!(error = ?e, "failed to sync permission mode to daemon");
            }
        });
    }

    /// Keep system-prompt permissions layer in sync with Shift+Tab / Plan toggle.
    ///
    /// Always updates `prompt_context` + `assembled_system_messages`. When no
    /// turn is running, also rewrites the leading `system` prefix of
    /// `conversation_history` so the next turn sees the new policy. Mid-turn
    /// history is left alone (in-flight loop already holds the old prefix).
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
        self.assembled_system_messages = assembled.system_messages.clone();
        tracing::info!(
            mode = ?self.mode,
            sandbox = self.prompt_context.sandbox_mode.as_deref().unwrap_or("?"),
            approval = self.prompt_context.approval_policy.as_deref().unwrap_or("?"),
            "agent mode changed; system prompt permissions re-assembled"
        );

        // Idle: rewrite leading system prefix so the next turn sees new policy.
        // Prefer try_lock so a subsequent Submit in the same tick cannot race a
        // spawned task still waiting on the mutex.
        if self.current_turn_handle.is_none() {
            let sys_msgs = assembled.system_messages;
            match self.conversation_history.try_lock() {
                Ok(mut h) => {
                    let rest: Vec<_> = h
                        .iter()
                        .skip_while(|m| m.role == "system")
                        .cloned()
                        .collect();
                    *h = sys_msgs;
                    h.extend(rest);
                }
                Err(_) => {
                    let history = self.conversation_history.clone();
                    tokio::spawn(async move {
                        let mut h = history.lock().await;
                        let rest: Vec<_> = h
                            .iter()
                            .skip_while(|m| m.role == "system")
                            .cloned()
                            .collect();
                        *h = sys_msgs;
                        h.extend(rest);
                    });
                }
            }
        }
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
        assert_eq!(
            app.prompt_context.approval_policy.as_deref(),
            Some("never")
        );
        let yolo_perm = app
            .assembled_system_messages
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
            .assembled_system_messages
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
