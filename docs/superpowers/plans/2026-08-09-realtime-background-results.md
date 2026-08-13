# Realtime Background Results Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver completed background-command results to the owning TUI session immediately through the daemon global SSE bus, without waiting for a new user prompt.

**Architecture:** Add the originating session ID to `BackgroundResult` when `BackgroundTool` starts a command from its trusted `ToolContext`. Keep daemon retention and `background_result` publication together, then extend the existing single TUI global-event reader to map matching results into a structured app event. The `App` displays it and schedules a hidden agent turn with the result payload. A session-scoped shared delivery ledger prevents the real-time event and retained-snapshot fallback from causing duplicate model turns.

**Tech Stack:** Rust, Tokio broadcast/mpsc/SSE, Axum, Reqwest, Ratatui application event loop, Cargo test/fmt/clippy.

## Global Constraints

- Preserve Rust naming conventions and run `cargo fmt -- --check` plus `cargo clippy --all-targets -- -D warnings`.
- Tool-layer code must not depend on `agent/`; consume the trusted session identity already exposed through `ToolContext::agent` without creating a reverse module dependency.
- A result missing `session_id` is legacy/unowned and must never be pushed to a TUI session.
- The SSE reader must enqueue `AppEvent`s only; it must not mutate `App` from the background task.
- Keep the existing `GET /api/v1/events` connection; do not add a second background-result SSE endpoint.
- Retain `GET /api/v1/background/results` for recovery, but filter its snapshot by the agent loop's session ID and the shared delivery ledger.

---

### Task 1: Bind background results to their originating session

**Files:**
- Modify: `src/tools/execution/background.rs:19-280, 414-482`
- Modify: `src/daemon/handlers.rs:3302-3400`
- Test: `src/tools/execution/background.rs:485-570`
- Test: `src/daemon/handlers.rs:3302-3400`

**Interfaces:**
- Consumes: `ToolContext::agent.session_id: SessionId` and `BackgroundManager::spawn(command, timeout, mode, workdir)`.
- Produces: `BackgroundResult { session_id: Option<String>, .. }`, deserializable by the TUI, and `BackgroundManager::spawn(..., session_id: Option<String>)`.
- Produces: `BackgroundTool::execute_with_context` passes `Some(context.agent.session_id.to_string())`; context-free execution passes `None`.

- [x] **Step 1: Write the failing tool-layer test**

Add a test using a real `ToolContext` with `SessionId::new("session-a")`, execute a short background command, drain its result, and assert `result.session_id.as_deref() == Some("session-a")`. Add a context-free execution test that asserts its result has no session ID.

```rust
let result = manager.drain_results().await.remove(0);
assert_eq!(result.session_id.as_deref(), Some("session-a"));
```

- [x] **Step 2: Run the focused test to verify it fails**

Run: `cargo test tools::execution::background --lib`

Expected: FAIL because `BackgroundResult` has no `session_id` field or the completion lacks the expected value.

- [x] **Step 3: Write the failing daemon retention/broadcast test**

Extend `sample_bg_result` to accept a session ID and assert the received `GlobalEventKind::BackgroundResult` payload has `data.result.session_id == "session-a"` after `record_background_result`.

```rust
assert_eq!(event.data["result"]["session_id"], "session-a");
```

- [x] **Step 4: Run the daemon test to verify it fails**

Run: `cargo test daemon::handlers::tests::background_results_are_retained_not_drained --lib`

Expected: FAIL because the payload does not contain the session identifier.

- [x] **Step 5: Implement the minimal propagation**

Add `Deserialize` plus `#[serde(default, skip_serializing_if = "Option::is_none")] pub session_id: Option<String>` to `BackgroundResult`; capture and clone it inside `BackgroundManager::spawn`; populate it in every command success, error, and timeout constructor. Preserve `None` for subagent results and non-context executions. Extend `BackgroundTool::run` with `session_id: Option<&str>`, passing the trusted ID only from `execute_with_context`. Update all fixtures and constructors.

```rust
self.run(
    input,
    context.effective_mode,
    context.workdir,
    Some(context.agent.session_id.as_str()),
).await
```

- [x] **Step 6: Run focused tests to verify they pass**

Run: `cargo test tools::execution::background --lib && cargo test daemon::handlers::tests::background_results_are_retained_not_drained --lib`

Expected: PASS; the completion and retained SSE payload preserve `session-a`.

- [x] **Step 7: Commit the bounded propagation change**

```bash
git add src/tools/execution/background.rs src/daemon/handlers.rs
git commit -m "feat(background): scope results to sessions"
```

### Task 2: Consume matching background-result events and schedule a hidden continuation

**Files:**
- Modify: `src/tui/app/server_side.rs:420-570`
- Modify: `src/tui/app/types.rs:410-435`
- Modify: `src/tui/app/mod.rs:35-105, 780-788`
- Modify: `src/tui/app/event.rs:300-325, 1012-1032`
- Modify: `src/tui/app/turn.rs:10-55`
- Test: `src/tui/app/server_side.rs:570-700`
- Test: `src/tui/app/turn.rs:800-940`

**Interfaces:**
- Consumes: `GlobalEventWire { seq, kind, data }` from `DaemonClient::subscribe_events()` and the active `session_id: String`.
- Produces: `spawn_global_event_reader(client, session_id, event_tx, shutdown) -> JoinHandle<()>`.
- Produces: matching `background_result` events as `AppEvent::BackgroundTaskCompleted(BackgroundResult)`; `todos_changed` behavior remains unchanged.
- Produces: `PendingInput::background_result(result)` with an empty display string and a model-facing, serialized result message.

- [x] **Step 1: Write failing mapping tests**

Extract a pure helper, `global_event_to_app_events(event: GlobalEventWire, session_id: &str) -> Vec<AppEvent>`. Test that a `background_result` with `data.result.session_id == "session-a"` produces exactly one `BackgroundTaskCompleted` containing that result; test that `session-b` and an absent session ID produce no event. Add an App-level test that an idle handler converts the structured event into one hidden pending turn, while a running turn keeps the result displayed but does not enqueue another turn.

```rust
assert!(global_event_to_app_events(event_for("session-b"), "session-a").is_empty());
assert!(matches!(global_event_to_app_events(event_for("session-a"), "session-a").as_slice(),
    [AppEvent::BackgroundTaskCompleted(result)] if result.task_id == "bg_a"));
```

- [x] **Step 2: Run mapping tests to verify they fail**

Run: `cargo test tui::app::server_side --lib`

Expected: FAIL because the mapping helper and background-result branch do not exist.

- [x] **Step 3: Implement the event mapping and reader rename**

Rename `spawn_todos_event_reader` to `spawn_global_event_reader`, accept the active session ID, and map events through the pure helper. On `background_result`, deserialize the contained `BackgroundResult` and reject missing/nonmatching IDs. Add `AppEvent::BackgroundTaskCompleted(BackgroundResult)`. Its app handler formats the existing system notification and, only when no turn is running, pushes `PendingInput::background_result(result)` then calls `start_next_turn`. Extend `PendingInput` with an explicit hidden flag so `start_next_turn` does not render or auto-name an empty user row. On `todos_changed`, preserve the existing sequence/order and `TodosSnapshot` forwarding behavior. Update the startup call in `App::run` to provide `self.session_id.clone()`.

```rust
match event.kind.as_str() {
    "background_result" => parse_background_result(event.data, session_id)
        .map(AppEvent::BackgroundTaskCompleted)
        .into_iter()
        .collect(),
    "todos_changed" => todos_event(event.data),
    _ => Vec::new(),
}
```

- [x] **Step 4: Run mapping tests to verify they pass**

Run: `cargo test tui::app::server_side --lib`

Expected: PASS; matching results enqueue one structured UI event, foreign/legacy results enqueue none, and an idle app starts exactly one hidden result turn.

- [x] **Step 5: Commit the TUI SSE consumption change**

```bash
git add src/tui/app/server_side.rs src/tui/app/types.rs src/tui/app/mod.rs src/tui/app/event.rs src/tui/app/turn.rs
git commit -m "feat(tui): continue on background result events"
```

### Task 3: Deduplicate real-time delivery and snapshot recovery

**Files:**
- Modify: `src/tui/app/mod.rs:95-180, 250-470`
- Modify: `src/tui/app/turn.rs:180-420`
- Modify: `src/tui/agent/mod.rs:40-160`
- Modify: `src/tui/agent/compaction.rs:10-49`
- Test: `src/tui/agent/compaction.rs:52-120` or the nearest existing TUI-agent test module
- Test: `src/daemon/handlers.rs:3356-3370`

**Interfaces:**
- Consumes: `AgentLoop::session_id`, `Arc<tokio::sync::Mutex<HashSet<String>>>` owned by `App`, retained `Vec<serde_json::Value>` from `DaemonClient::get_background_results()`, and `BackgroundResult.session_id`.
- Produces: `inject_background_results` only appends and emits unseen results whose session ID matches `self.session_id`; unowned, foreign, and delivered task IDs are ignored.
- Produces: all `AgentLoop::new` call sites receive the session's shared `delivered_background_task_ids` ledger.

- [x] **Step 1: Write the failing recovery filter test**

Use an agent-loop/client test fixture that returns four retained values: one unseen `bg_a` for `session-a`, one already-recorded `bg_seen` for `session-a`, one for `session-b`, and one without `session_id`. Assert that invoking background-result injection for `session-a` emits/history-injects only `bg_a` and records it in the shared ledger.

```rust
assert_eq!(notifications, vec!["[Background task bg_a completed: SUCCESS]"]);
assert!(seen.lock().await.contains("bg_a"));
```

- [x] **Step 2: Run the focused recovery test to verify it fails**

Run: `cargo test tui::agent --lib`

Expected: FAIL because the current filter selects all command results regardless of session and has no shared delivery ledger.

- [x] **Step 3: Implement the minimal filter**

Add `delivered_background_task_ids: Arc<Mutex<HashSet<String>>>` to `App`, initialize it for every session, reset it on `/clear`/session switch, and pass it into every `AgentLoop::new`. In the structured real-time event handler, insert the task ID before scheduling the hidden turn. In `inject_background_results`, require a matching session ID and atomically insert the task ID before adding a history/notification record; skip any ID already present. Preserve the current subagent exclusion and notification format.

```rust
if r["session_id"].as_str() != Some(self.session_id.as_str()) {
    return None;
}
```

- [x] **Step 4: Run focused and regression tests**

Run: `cargo test tui::agent --lib && cargo test daemon::handlers::tests::background_manager_results_flow_into_retained_queue --lib && cargo test tui::app::server_side --lib`

Expected: PASS; recovery is session-scoped, real-time events cannot be re-injected from the retained snapshot, and the manager-to-daemon-to-SSE route remains intact.

- [x] **Step 5: Run project verification**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all`

Expected: all commands exit 0.

- [x] **Step 6: Commit recovery and test changes**

```bash
git add src/tui/app/mod.rs src/tui/app/turn.rs src/tui/agent/mod.rs src/tui/agent/compaction.rs src/tui/agent src/daemon/handlers.rs
git commit -m "fix(background): deduplicate session result recovery"
```
