## 1. Data Model Changes

- [x] 1.1 Add `SubagentAction` struct (tool_name, params_summary, timestamp_ms) to `src/agent/progress.rs`
- [x] 1.2 Add `action_log: Vec<SubagentAction>` (max 10, newest-first) and `current_params: Option<String>` fields to `SubagentProgress` in `src/agent/progress.rs`
- [x] 1.3 Add `text_snapshot: Option<String>` field to `SubagentProgress` in `src/agent/progress.rs`
- [x] 1.4 Add `routing_reason: Option<String>` field to task tool's result metadata in `src/tools/meta/task.rs`

## 2. Core Subagent Loop — Populate New Fields

- [x] 2.1 Capture last assistant text response after each round in `run_subagent_loop()` and emit as `text_snapshot` in `SubagentProgress` (truncated to last 200 chars)
- [x] 2.2 When a tool call starts, extract key params summary (1-2 most meaningful values, max 80 chars) and emit progress with `current_params` updated; append `SubagentAction` to `action_log`
- [x] 2.3 Accumulate `input_tokens` + `output_tokens` from each API response in `run_subagent_loop()` and populate `metadata.token_count` on `Completed` progress events
- [x] 2.4 Clear `text_snapshot` on terminal status events (`Completed`, `Failed`, `Cancelled`); preserve `action_log`

## 3. Daemon Progress Store — Session Isolation

- [x] 3.1 Change `DaemonState.subagent_progress` from `HashMap<NodeId, SubagentProgress>` to `HashMap<SessionId, HashMap<NodeId, SubagentProgress>>` in `src/daemon/state.rs`
- [x] 3.2 Update progress write path (the callback registered by task tool) to accept and use `session_id`
- [x] 3.3 Update `/api/v1/subagent/progress` endpoint to accept `?session_id=` param and filter results in `src/daemon/handlers.rs`
- [x] 3.4 Add cleanup: remove session progress entries after 60s of no polling (ttl-based eviction)

## 4. Task Complexity Detection — Refined Routing

- [x] 4.1 Rewrite `is_complex_task()` in `src/tools/meta/task.rs` to use structural analysis (numbered steps, file references, dependency declarations) instead of keyword counting
- [x] 4.2 Remove the current keyword list (`COMPLEX_KEYWORDS`) and replace with structural patterns
- [x] 4.3 Raise the length-only threshold from 500 to 1000 chars, and only as a secondary signal
- [x] 4.4 Populate `routing_reason` in the task tool result describing why RLM or direct execution was chosen
- [x] 4.5 Add unit tests for `is_complex_task()` covering: simple prompt → false, numbered steps → true, dependency chain → true, long but simple → false

## 5. TUI Status Bar — Live Counters

- [x] 5.1 Add `active_count()`, `completed_count()`, `failed_count()` accessor methods to `SubagentTree` in `src/tui/components/subagent_tree.rs`
- [x] 5.2 Update status bar rendering in `src/tui/components/status.rs` to display "N active · X/Y done" instead of static "Subagent running…" when `subagent_tree` has nodes
- [x] 5.3 Handle zero-subagent case: show no counter when no subagents have been used in the current turn

## 6. TUI Panel — Enhanced Node Display

- [x] 6.1 Render per-node elapsed time and round progress in `src/tui/components/chat.rs` (e.g., "round 3/10 · 12.3s")
- [x] 6.2 Render token consumption for completed nodes (e.g., "1.5k tokens")
- [x] 6.3 Render `text_snapshot` preview (dimmed, truncated) prominently beneath active nodes — this is the model's "thinking"
- [x] 6.4 Render recent action log (last 3 tool calls with params) beneath the text snapshot, showing the call→think→call→think loop
- [x] 6.5 Show "thinking…" placeholder when a Running node has no text snapshot yet
- [x] 6.6 Render `current_tool` with `current_params` as the active action line (e.g., "executing: file_read(\"src/auth.rs\")")

## 7. Inline Subagent Card — Action Visibility

- [x] 7.1 Update inline subagent card in `src/tui/components/chat.rs` to show `current_tool` with `current_params`
- [x] 7.2 Show `text_snapshot` preview (dimmed, ~100 chars) in the inline card beneath the tool line
- [x] 7.3 Render routing reason from tool result metadata in the chat area (dimmed line beneath tool label)

## 8. Client & Wiring

- [x] 8.1 Update `DaemonClient::poll_subagent_progress()` in `src/tui/client.rs` to pass `session_id` query param
- [x] 8.2 Ensure the session ID is available in the TUI agent loop for progress polling (both sequential path in `src/tui/agent/core.rs` and parallel path in `src/tui/agent/tool_dispatch.rs`)

## 9. Testing & Verification

- [x] 9.1 Run `cargo test --lib` — 96 tests pass, 0 failures
- [ ] 9.2 Run `cargo clippy --all-targets -- -D warnings` — pre-existing warnings in unrelated files remain
- [x] 9.3 Run `cargo fmt` to ensure formatting
- [ ] 9.4 Manual verification: run `cargo run -- repl` and trigger a task that uses subagents, confirm:
  - Status bar shows live counters (active/done/failed)
  - Inline card shows tool calls with params + model text preview
  - Overlay panel shows action history (tool calls) + text snapshots + timing/tokens
  - Simple tasks are NOT routed to RLM pipeline
