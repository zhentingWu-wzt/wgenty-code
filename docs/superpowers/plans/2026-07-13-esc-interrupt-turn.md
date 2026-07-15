---
change: esc-interrupt-turn
design-doc: docs/superpowers/specs/2026-07-13-esc-interrupt-turn-design.md
base-ref: 9349d77e760b553403b78ee57c1fede46721c924
archived-with: 2026-07-14-esc-interrupt-turn
---

# ESC Interrupt Running Turn Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Make ESC interrupt a running agent turn in the TUI REPL (abort the turn, preserve partial output, cancel subagents, surface feedback) and stop ESC from quitting the app.

**Architecture:** Add an `interrupt_running_turn()` wrapper around the existing `cancel_current_turn()` abort primitive; it commits partial streamed text, finalizes a running tool placeholder, cancels daemon-side subagents via `reset_agent_generation`, and pushes an "Interrupted by user" system message. A new ESC branch in `handle_key_event` (gated on `current_turn_handle.is_some()`) routes ESC to it; the old ESC-to-quit fallback is removed.

**Tech Stack:** Rust 2021, tokio, crossterm 0.28, ratatui. TDD with `#[tokio::test]` using `App::new(DaemonClient::new("http://localhost:0"), ...)`.

## Global Constraints

- `cargo fmt` enforced (CI); `cargo clippy --all-targets -- -D warnings` zero warnings (CI).
- Error handling: no bare `unwrap()`; fire-and-forget daemon calls use `let _ =` on send and `tracing::warn!` on error (mirroring `/clear`).
- User-facing string `⏹ Interrupted by user` is hardcoded, consistent with existing system messages (e.g. "Plan mode enabled").
- Only `src/tui/` is touched; `cancel_current_turn()` stays unchanged (still used by `/clear`).
- Conventional Commits, English, scope `tui`.

archived-with: 2026-07-14-esc-interrupt-turn
---

## File Structure

- **Modify** `src/tui/app/turn.rs` — add `interrupt_running_turn(&mut self)` in the existing `impl App` block (after `cancel_current_turn`, ~line 342); add a `#[cfg(test)] mod tests` block at file end.
- **Modify** `src/tui/app/event_key.rs` — insert ESC-interrupt branch in `handle_key_event` (after the subagent-status-bar block, before the `PageUp/PageDown` scroll block ~line 373); delete the ESC-to-quit fallback (~lines 451-453); add a `#[cfg(test)] mod tests` block at file end.
- **No change** to `src/state/agent_phase.rs` (reuses `TurnAbortReason::Interrupted`).

archived-with: 2026-07-14-esc-interrupt-turn
---

### Task 1: `interrupt_running_turn()` method

**Files:**
- Modify: `src/tui/app/turn.rs` (add method after `cancel_current_turn`, line ~342; add test module at end)
- Test: `src/tui/app/turn.rs` (in-module `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `cancel_current_turn()` (`turn.rs:328`), `push_system_message()` (`input.rs:9`, module-private, accessible in `tui::app`), `AppEvent::AgentGenerationReset`, `DaemonClient::reset_agent_generation`, fields `streaming_content`/`streaming_active`/`has_running_tool`/`current_turn_handle`/`committed_messages`/`daemon_client`/`session_id`/`event_tx`.
- Produces: `pub(super) fn interrupt_running_turn(&mut self)` — called by Task 2's ESC branch.

- [x] **Step 1: Write the failing tests**

Append to `src/tui/app/turn.rs`:

```rust
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
}
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib interrupt_running_turn 2>&1 | tail -20`
Expected: COMPILE ERROR — `no method named interrupt_running_turn found` (method not yet defined).

- [x] **Step 3: Write minimal implementation**

Add this method to the `impl App` block in `src/tui/app/turn.rs`, immediately after `cancel_current_turn` (before `pending_count`):

```rust
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
    }
```

- [x] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib interrupt_running_turn 2>&1 | tail -20`
Expected: PASS — both `interrupt_running_turn_commits_partial_and_resets_state` and `interrupt_running_turn_skips_preparing_hint` pass.

- [x] **Step 5: Commit**

```bash
git add src/tui/app/turn.rs docs/superpowers/specs/2026-07-13-esc-interrupt-turn-design.md docs/superpowers/plans/2026-07-13-esc-interrupt-turn.md
git commit -m "feat(tui): add interrupt_running_turn for ESC turn interruption

- Commit partial streamed content as Assistant message before abort
- Finalize running tool placeholder and stop spinner
- Reuse cancel_current_turn for abort + cancel subagents via reset_agent_generation
- Surface 'Interrupted by user' system message"
```

archived-with: 2026-07-14-esc-interrupt-turn
---

### Task 2: ESC key binding wiring + remove ESC-to-quit

**Files:**
- Modify: `src/tui/app/event_key.rs` (insert ESC-interrupt branch ~line 373; remove ESC-quit fallback ~lines 451-453; add test module at end)
- Test: `src/tui/app/event_key.rs` (in-module `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `interrupt_running_turn()` (Task 1), fields `current_turn_handle`/`should_quit`.
- Produces: ESC interrupts a running turn; idle ESC is a no-op (no quit).

- [x] **Step 1: Write the failing tests**

Append to `src/tui/app/event_key.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::watcher::SettingsHandle;
    use crate::config::Settings;
    use crate::tui::client::DaemonClient;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    fn build_app() -> App {
        let client = DaemonClient::new("http://localhost:0".to_string());
        let settings: SettingsHandle = Arc::new(RwLock::new(Settings::default()));
        App::new(client, "test-esc".to_string(), settings)
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())
    }

    #[tokio::test]
    async fn esc_interrupts_running_turn() {
        let mut app = build_app();
        app.current_turn_handle = Some(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
        app.handle_key_event(esc());
        assert!(
            app.current_turn_handle.is_none(),
            "ESC should interrupt the running turn"
        );
        assert!(
            app.committed_messages
                .iter()
                .any(|m| m.content.contains("Interrupted by user")),
            "ESC interrupt should surface feedback"
        );
        assert!(!app.should_quit, "ESC must not quit during a running turn");
    }

    #[tokio::test]
    async fn esc_idle_does_not_quit() {
        let mut app = build_app();
        // No turn running (current_turn_handle is None).
        app.handle_key_event(esc());
        assert!(!app.should_quit, "ESC must not quit when idle");
    }
}
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib esc_interrupts_running_turn esc_idle_does_not_quit 2>&1 | tail -25`
Expected: `esc_interrupts_running_turn` FAILS (`current_turn_handle` still `Some` — ESC currently quits via the fallback or does nothing; either way the handle is not cleared and no message is added). `esc_idle_does_not_quit` FAILS (`should_quit` becomes `true` via the existing ESC-to-quit fallback).

- [x] **Step 3: Write minimal implementation**

(a) Insert the ESC-interrupt branch in `handle_key_event`. Place it immediately after the subagent-status-bar block closes (after the `}` that ends the `if self.subagent_focus.is_none() { ... }` status-bar block, just before the `// Scroll handling: PageUp/PageDown only.` comment, ~line 373):

```rust
        // ESC: interrupt a running turn. Placed after all contextual panels
        // (focus view, completion, permission = Deny, question, session popup,
        // status bar) have had their chance to consume ESC. ESC no longer
        // quits; quitting is via Ctrl+C double-press.
        if key.code == KeyCode::Esc && self.current_turn_handle.is_some() {
            self.interrupt_running_turn();
            return;
        }
```

(b) Remove the ESC-to-quit fallback. Delete these lines (near the end of `handle_key_event`, ~lines 451-453):

```rust
        if !handled && key.code == KeyCode::Esc {
            self.should_quit = true;
        }
```

Leave the preceding `let handled = self.input_box.textarea.input(key);` and `self.input_box.update_style();` lines in place.

- [x] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib esc_interrupts_running_turn esc_idle_does_not_quit 2>&1 | tail -25`
Expected: PASS — both tests pass.

Also run the full turn-interrupt tests to confirm no regression: `cargo test --lib interrupt_running_turn 2>&1 | tail -10` → PASS.

- [x] **Step 5: Commit**

```bash
git add src/tui/app/event_key.rs
git commit -m "feat(tui): wire ESC to interrupt running turn and drop ESC-to-quit

- ESC with a live current_turn_handle calls interrupt_running_turn
- Remove ESC-to-quit fallback; quit remains Ctrl+C double-press
- Contextual panels retain ESC priority (permission = Deny, popups dismiss)"
```

archived-with: 2026-07-14-esc-interrupt-turn
---

### Task 3: Lint, format, and full test verification

**Files:** none (verification only; apply fixes if flags surface)

- [x] **Step 1: Format check**

Run: `cargo fmt`
Expected: formats the new code; no errors.

- [x] **Step 2: Clippy zero-warning**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | tail -20`
Expected: zero warnings. If clippy flags the new code, fix and re-run (do not silence with `#[allow]`).

- [x] **Step 3: Full test suite**

Run: `cargo test --all 2>&1 | tail -30`
Expected: all tests pass, including the new `interrupt_running_turn` and ESC tests and the existing `auto_dream_service_is_initialized_on_app_creation`.

- [x] **Step 4: Commit if any formatting/fixups**

```bash
git add -A
git commit -m "style(tui): apply fmt/clippy fixes for ESC interrupt" || echo "nothing to commit"
```

- [x] **Step 5: Manual TUI verification (optional but recommended)**

Run: `cargo run -- repl`
Verify: send a prompt; while streaming, press ESC → turn stops, `⏹ Interrupted by user` shown, partial text visible, status bar returns to idle. Press ESC while idle → app does not quit. Ctrl+C twice → app quits.
