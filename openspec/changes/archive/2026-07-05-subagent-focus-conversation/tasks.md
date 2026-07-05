## 1. Data Flow — SubagentProgress.messages

- [x] 1.1 Add `messages: Vec<ChatMessage>` field to `SubagentProgress` (`src/agent/progress.rs`) with serde serialization
- [x] 1.2 Subagent loop (`src/teams/subagent_loop.rs`): snapshot `messages` at each emit point and set `progress.messages = messages.clone()`
- [x] 1.3 Verify daemon serialization round-trip: subagent loop → daemon store → TUI poll carries `messages`

## 2. Focus View — ChatMessage→UIMessage Conversion

- [x] 2.1 Add a conversion function `chat_message_to_ui_lines` in `chat.rs` or `subagent_focus_view.rs` that renders a `ChatMessage` as `Vec<ratatui::text::Line>`, reusing existing `message_to_lines` logic (User→"› You" block, Assistant→text block, Tool→collapsible block with verb+args+result+diff)
- [x] 2.2 Unit tests for the conversion: each ChatMessage role produces expected line groups

## 3. Focus View — Render

- [x] 3.1 `FocusViewState` (`subagent_focus_view.rs`): add `messages: Vec<ChatMessage>`, `collapsed_tool_indices: HashSet<String>` (keyed by tool_call_id to survive message insertions)
- [x] 3.2 `FocusViewState::build` and `::rebuild`: populate `messages` from `node.progress.messages`; rebuild refreshes messages and preserves collapse state
- [x] 3.3 `FocusView::render`: replace the timeline area with a conversation view — iterate `messages`, convert each to lines via step 2.1, apply collapse state for tool messages, scroll with existing scroll model. Keep the header and selector bars.
- [x] 3.4 Add keyboard handling: ↑↓ scroll conversation; `t` toggle expand selected tool (or Ctrl+O like main chat)

## 4. Event Routing

- [x] 4.1 `SubagentUpdate` handler (`event.rs`): call `focus.rebuild()` to refresh messages (existing logic, verify it works for messages too)
- [x] 4.2 Ensure auto_scroll pins to latest conversation message on entry/rebuild

## 5. Tests & Validation

- [x] 5.1 Unit tests for ChatMessage→UIMessage conversion
- [x] 5.2 Unit tests for FocusViewState messages rebuild (preserves collapse state)
- [x] 5.3 `cargo clippy --all-targets -- -D warnings` passes
- [x] 5.4 `cargo test --all` passes
- [x] 5.5 `cargo fmt --check` passes
