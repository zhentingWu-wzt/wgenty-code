# Implementation Tasks: ESC Interrupt Running Turn

## 1. Interrupt primitive & UX (`src/tui/app/turn.rs`)

- [x] 1.1 Add `interrupt_running_turn(&mut self)` method to `App`: commit non-empty/non-hint `streaming_content` as an `Assistant` `UIMessage`, then clear `streaming_content` and set `streaming_active = false` (mirror `StreamDone` in `event.rs:153`)
- [x] 1.2 In `interrupt_running_turn`, finalize a running tool placeholder: set `has_running_tool = false` and, if the last committed message is a `Tool` row with `tool_running == true`, set its `tool_running = false`
- [x] 1.3 In `interrupt_running_turn`, call `cancel_current_turn()` to abort the task, set phase `Idle`, and emit `TurnAborted::Interrupted`
- [x] 1.4 In `interrupt_running_turn`, replicate `/clear`'s async `reset_agent_generation` (`input.rs:65-83`) to cancel daemon-side subagents and adopt a fresh generation
- [x] 1.5 In `interrupt_running_turn`, push system message `⏹ Interrupted by user` via `push_system_message`

## 2. Key binding wiring (`src/tui/app/event_key.rs`)

- [x] 2.1 Add ESC-interrupt branch in `handle_key_event`, placed after all contextual panel handlers (focus view, completion, permission, question, session popup, status bar) and before scroll/input handling: `if key.code == KeyCode::Esc && self.current_turn_handle.is_some() { self.interrupt_running_turn(); return; }`
- [x] 2.2 Remove the ESC-to-quit fallback (`if !handled && key.code == KeyCode::Esc { self.should_quit = true; }`) so ESC no longer quits; quit remains Ctrl+C double-press

## 3. Verification

- [x] 3.1 Add or update unit/integration tests covering: ESC with a live `current_turn_handle` calls the interrupt path; ESC with no live handle does not quit; permission panel still intercepts ESC before the interrupt branch
- [x] 3.2 Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` (zero warnings)
- [x] 3.3 Run `cargo test --all` (all tests pass)
- [x] 3.4 Manual TUI verification: ESC interrupts a streaming turn (partial text preserved, `⏹ Interrupted by user` shown, phase returns to idle); ESC interrupts tool execution and `/compact`; idle ESC is a no-op (no quit); ESC during a permission prompt still Denies; Ctrl+C double-press still quits
