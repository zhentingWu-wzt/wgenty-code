<!-- Tasks placeholder — to be filled during the build phase (writing-plans) -->

## 1. TUI Layout & State Changes

- [x] 1.1 Add `subagent_focus_node_id: Option<String>` and `subagent_status_bar_selected: usize` to App struct (`src/tui/app/mod.rs`)
- [x] 1.2 Update main render layout to include subagent status bar area (`src/tui/app/render.rs`)
- [x] 1.3 Add focus view full-screen render branch (`src/tui/app/render.rs`)

## 2. Remove Inline Subagent Card

- [x] 2.1 Remove `render_subagent_card` call from `render()` in `src/tui/components/chat.rs`
- [x] 2.2 Clean up or repurpose `render_subagent_card` and `render_tree_nodes` functions

## 3. Subagent Status Bar Component

- [x] 3.1 Create `src/tui/components/subagent_status_bar.rs` — compact status bar with active subagent list
- [x] 3.2 Add keyboard navigation (↑↓ select, Enter to focus view) in `src/tui/app/event.rs`
- [x] 3.3 Register module in `src/tui/components/mod.rs`

## 4. Subagent Focus View Component

- [x] 4.1 Create `src/tui/components/subagent_focus_view.rs` — full-screen event timeline view
- [x] 4.2 Add keyboard handling (Esc return, ↑↓ scroll) in `src/tui/app/event.rs`
- [x] 4.3 Implement real-time polling for running subagents in focus view

## 5. Tests & Validation

- [x] 5.1 Unit tests for status bar state and selection logic
- [x] 5.2 Unit tests for focus view rendering
- [x] 5.3 `cargo clippy --all-targets -- -D warnings` passes
- [x] 5.4 `cargo test --all` passes
