# Unified Subagent Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace synchronous/background subagent modes with one structured-concurrency lifecycle, including parent completion barriers, persistent-root result delivery, synthetic continuation turns, and capability-scoped TUI navigation.

**Architecture:** `AgentCoordinator` remains the lifecycle authority while a new focused `agent::task_group` module owns group/generation/delivery state. Every `task` call spawns immediately and returns an acknowledgement; non-root loops perform a coordinator-managed child-result synthesis round before finalizing, while the persistent main agent atomically claims ready result groups through a daemon API and either merges them into a user turn or starts an internal continuation turn. The TUI continues to render scoped local views and uses issued capabilities to descend one level at a time.

**Tech Stack:** Rust, Tokio, Axum, serde, thiserror/anyhow, Ratatui, existing `AgentCoordinator`, existing TUI event loop, Cargo test/fmt/clippy.

---

## File Structure

- Create `src/agent/task_group.rs`: task-group IDs, generation-bound group state, terminal result batches, and atomic delivery claims.
- Modify `src/agent/mod.rs`: export task-group public types.
- Modify `src/agent/identity.rs`: carry the originating root turn through trusted tool context without accepting it from model JSON.
- Modify `src/agent/coordinator.rs`: connect child reservation/finish/finalization/cancellation to task groups and expose synthesis/delivery methods.
- Modify `src/agent/store.rs`: preserve task-group generation on coordinator-created records where needed by result authorization.
- Modify `src/tools/meta/task.rs`: remove runtime mode selection, always spawn/register a child task, and return a structured acknowledgement.
- Modify `src/tools/meta/task/tests.rs`: schema, acknowledgement, identity, and immediate-return tests.
- Modify `src/teams/subagent_loop.rs`: perform the mandatory post-child synthesis round before returning a non-root final result.
- Modify `src/daemon/models.rs`, `src/daemon/handlers.rs`, `src/daemon/routes.rs`, `src/daemon/state.rs`: expose atomic task-group delivery and generation reset APIs.
- Create `src/tui/app/continuation.rs`: claim ready groups, choose user-turn versus synthetic consumption, and start internal continuation turns.
- Modify `src/tui/app/mod.rs`, `src/tui/app/types.rs`, `src/tui/app/turn.rs`, `src/tui/app/input.rs`, `src/tui/app/event.rs`: integrate continuation scheduling and `/clear` generation reset.
- Modify `src/tui/agent/mod.rs` and remove subagent delivery from `src/tui/agent/compaction.rs`: inject structured task-group results rather than background-manager user messages.
- Modify `src/tui/client.rs`: daemon task-group and scoped navigation client methods.
- Modify `src/tui/app/event_key.rs`, `src/tui/components/subagent_tree.rs`, and `src/tui/components/subagent_focus_view.rs`: capability-driven descent/back navigation.
- Modify `src/prompts/base.md`, `src/prompts/init_instructions.md`, `CHANGELOG.md`: document the unified behavior and remove `background` guidance.
- Modify `tests/strict_subagent_isolation.rs` and create `tests/unified_subagent_lifecycle.rs`: boundary and end-to-end lifecycle contracts.

### Task 1: Add Generation-Bound Task Group State

**Files:**
- Create: `src/agent/task_group.rs`
- Modify: `src/agent/mod.rs`
- Test: `src/agent/task_group.rs`

- [ ] **Step 1: Write failing unit tests for grouping and exactly-once delivery**

```rust
#[tokio::test]
async fn group_becomes_ready_only_after_every_child_is_terminal() {
    let store = TaskGroupStore::default();
    let group = store
        .create_for_root_turn(
            SessionId::new("s"),
            AgentId::new("root"),
            "turn-1",
            3,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
    store.add_child(&group, AgentId::new("a")).await.unwrap();
    store.add_child(&group, AgentId::new("b")).await.unwrap();
    store.record_result(&group, result("a", ChildTerminalStatus::Completed)).await.unwrap();
    assert!(store.claim_ready(&SessionId::new("s"), 3).await.unwrap().is_none());
    store.record_result(&group, result("b", ChildTerminalStatus::Failed)).await.unwrap();
    assert_eq!(store.claim_ready(&SessionId::new("s"), 3).await.unwrap().unwrap().results.len(), 2);
}

#[tokio::test]
async fn ready_group_can_be_claimed_exactly_once() {
    let store = ready_store().await;
    assert!(store.claim_ready(&SessionId::new("s"), 0).await.unwrap().is_some());
    assert!(store.claim_ready(&SessionId::new("s"), 0).await.unwrap().is_none());
}

#[tokio::test]
async fn stale_generation_is_not_deliverable() {
    let store = ready_store_for_generation(4).await;
    store.advance_generation(&SessionId::new("s")).await;
    assert!(store.claim_ready(&SessionId::new("s"), 4).await.unwrap().is_none());
}

#[tokio::test]
async fn deadline_marks_unfinished_children_as_timeout_failures() {
    let store = store_with_expired_group_and_live_child().await;
    store.expire_due_groups(tokio::time::Instant::now()).await.unwrap();
    let delivery = store.claim_ready(&SessionId::new("s"), 0).await.unwrap().unwrap();
    assert_eq!(delivery.results[0].status, ChildTerminalStatus::Failed);
    assert_eq!(delivery.results[0].error_code.as_deref(), Some("timeout"));
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run: `cargo test agent::task_group::tests --lib`

Expected: FAIL because `agent::task_group` and its types do not exist.

- [ ] **Step 3: Implement the task-group domain types and store**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskGroupId(String);

impl TaskGroupId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGroupDelivery {
    pub group_id: TaskGroupId,
    pub generation: u64,
    pub results: Vec<ChildResult>,
}

#[derive(Default)]
pub struct TaskGroupStore {
    inner: RwLock<TaskGroupState>,
}

#[derive(Default)]
struct TaskGroupState {
    generations: HashMap<SessionId, u64>,
    groups: HashMap<TaskGroupId, GroupRecord>,
}

struct GroupRecord {
    id: TaskGroupId,
    session_id: SessionId,
    owner_id: AgentId,
    origin_turn_id: Option<String>,
    generation: u64,
    deadline_at: tokio::time::Instant,
    child_ids: HashSet<AgentId>,
    results: HashMap<AgentId, ChildResult>,
    claimed: bool,
    cancelled: bool,
}

impl TaskGroupStore {
    pub async fn claim_ready(
        &self,
        session: &SessionId,
        generation: u64,
    ) -> Result<Option<TaskGroupDelivery>, TaskGroupError> {
        let mut state = self.inner.write().await;
        let Some(group) = state.groups.values_mut().find(|group| {
            group.session_id == *session
                && group.generation == generation
                && !group.claimed
                && !group.cancelled
                && !group.child_ids.is_empty()
                && group.child_ids.len() == group.results.len()
        }) else {
            return Ok(None);
        };
        group.claimed = true;
        Ok(Some(TaskGroupDelivery {
            group_id: group.id.clone(),
            generation,
            results: group.results.values().cloned().collect(),
        }))
    }
}
```

Define `TaskGroupError` with `thiserror`, deterministic result ordering by child ID, `create_for_root_turn`, `create_for_parent`, `add_child`, `record_result`, `expire_due_groups`, `current_generation`, `advance_generation`, and `cancel_generation`. `expire_due_groups` records unfinished children as `Failed` with error code `timeout` before making the group deliverable. Keep this module independent of TUI and daemon types.

- [ ] **Step 4: Export the types and rerun tests**

Add `pub mod task_group;` and exports for `TaskGroupDelivery`, `TaskGroupId`, `TaskGroupStore`, and `TaskGroupError` in `src/agent/mod.rs`.

Run: `cargo test agent::task_group::tests --lib`

Expected: PASS.

- [ ] **Step 5: Commit the task-group primitive**

```bash
git add src/agent/task_group.rs src/agent/mod.rs
git commit -m "feat(agent): add generation-bound task groups"
```

### Task 2: Make the Coordinator Own Group Membership and Result Delivery

**Files:**
- Modify: `src/agent/coordinator.rs`
- Modify: `src/agent/store.rs`
- Modify: `src/agent/identity.rs`
- Test: `src/agent/coordinator.rs`

- [ ] **Step 1: Write failing coordinator tests**

```rust
#[tokio::test]
async fn reserved_root_children_join_the_current_turn_group() {
    let coordinator = AgentCoordinator::new(4, 3);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let group = coordinator.create_root_task_group(&root, "turn-1").await.unwrap();
    let child = coordinator.reserve_child_in_group(&root, SpawnChildRequest::new("work"), group.clone()).await.unwrap();
    coordinator.finish_child(&child.context, ChildTerminal::completed("done")).await.unwrap();
    let delivery = coordinator.claim_ready_root_group(&root.session_id, 0).await.unwrap().unwrap();
    assert_eq!(delivery.group_id, group);
    assert_eq!(delivery.results[0].summary, "done");
}

#[tokio::test]
async fn collect_children_for_synthesis_waits_without_finalizing_parent() {
    let coordinator = AgentCoordinator::new(4, 3);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let parent = coordinator.reserve_child(&root, SpawnChildRequest::new("parent")).await.unwrap();
    let child = coordinator.reserve_child(&parent.context, SpawnChildRequest::new("child")).await.unwrap();
    coordinator.finish_child(&child.context, ChildTerminal::completed("evidence")).await.unwrap();
    let results = coordinator.collect_children_for_synthesis(&parent.context).await.unwrap();
    assert_eq!(results[0].summary, "evidence");
    assert_eq!(coordinator.status(&parent.context).await.unwrap(), AgentLifecycleStatus::Running);
}

#[tokio::test]
async fn persistent_root_cannot_enter_a_terminal_state() {
    let coordinator = AgentCoordinator::new(4, 3);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let error = coordinator
        .finalize_scope(&root, ParentOutcome::Completed("done".into()), JoinPolicy::BestEffort)
        .await
        .unwrap_err();
    assert!(matches!(error, CoordinatorError::RootHasNoTerminalState));
    assert_eq!(coordinator.status(&root).await.unwrap(), AgentLifecycleStatus::Running);
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run: `cargo test agent::coordinator::tests --lib`

Expected: FAIL because group-aware reservation, root claiming, and synthesis collection are missing.

- [ ] **Step 3: Add coordinator fields and group-aware methods**

```rust
pub struct AgentCoordinator {
    // existing fields...
    task_groups: Arc<TaskGroupStore>,
    child_groups: Arc<RwLock<HashMap<(SessionId, AgentId), TaskGroupId>>>,
}

pub struct ToolContext<'a> {
    pub agent: &'a AgentExecutionContext,
    pub invocation_id: ToolInvocationId,
    pub origin_turn_id: Option<&'a str>,
}

pub async fn reserve_child_in_group(
    &self,
    caller: &AgentExecutionContext,
    request: SpawnChildRequest,
    group_id: TaskGroupId,
) -> Result<ChildReservation, CoordinatorError> {
    let reservation = self.reserve_child(caller, request).await?;
    self.task_groups.add_child(&group_id, reservation.context.agent_id.clone()).await?;
    self.child_groups.write().await.insert(
        (reservation.context.session_id.clone(), reservation.context.agent_id.clone()),
        group_id,
    );
    Ok(reservation)
}
```

Update `finish_child` so it creates the sanitized `ChildResult`, persists terminal state, releases the permit, and records the result in the mapped direct-parent group before notifying waiters. A root-turn group contains only root's direct children; a non-root group contains only that parent's direct children, so descendant results are synthesized one level at a time and never leak to the root. Map task-group errors through a new actionable `CoordinatorError::TaskGroup(String)`.

- [ ] **Step 4: Add the non-terminal synthesis barrier**

```rust
pub async fn collect_children_for_synthesis(
    &self,
    caller: &AgentExecutionContext,
) -> Result<Vec<ChildResult>, CoordinatorError> {
    self.transition(caller, AgentLifecycleStatus::WaitingForChildren).await?;
    let results = self.join_children(caller, JoinPolicy::BestEffort).await?;
    self.transition(caller, AgentLifecycleStatus::Running).await?;
    Ok(results)
}

pub async fn begin_finalizing(
    &self,
    caller: &AgentExecutionContext,
) -> Result<(), CoordinatorError> {
    if !self.live_direct_children(caller).await?.is_empty() {
        return Err(CoordinatorError::ChildrenStillRunning);
    }
    self.transition(caller, AgentLifecycleStatus::Finalizing).await
}
```

Do not use `finalize_scope` for the root. Preserve it for cancellation/backward compatibility until all call sites migrate, then make root finalization return `RootHasNoTerminalState`.

- [ ] **Step 5: Run coordinator and strict-isolation tests**

Run: `cargo test agent::coordinator::tests --lib && cargo test --test strict_subagent_isolation`

Expected: PASS.

- [ ] **Step 6: Commit coordinator ownership**

```bash
git add src/agent/coordinator.rs src/agent/store.rs src/agent/identity.rs
git commit -m "feat(agent): coordinate task group delivery"
```

### Task 3: Unify `task` as an Always-Asynchronous Tool

**Files:**
- Modify: `src/tools/meta/task.rs`
- Modify: `src/tools/meta/task/tests.rs`
- Modify: `src/tools/meta/rlm/pipeline.rs`
- Modify: `src/daemon/models.rs`
- Modify: `src/daemon/handlers.rs`
- Modify: `src/tui/client.rs`
- Modify: `src/tui/agent/core.rs`
- Modify: `src/tui/agent/tool_dispatch.rs`
- Modify: `src/tui/app/turn.rs`
- Modify: `src/teams/subagent_loop.rs`
- Modify: `src/tools/executor.rs`
- Modify: `src/tools/mod.rs`
- Modify: `tests/comet_integration_test.rs`
- Test: `src/tools/meta/task/tests.rs`

- [ ] **Step 1: Write failing schema and acknowledgement tests**

```rust
#[test]
fn task_schema_has_no_background_switch() {
    let schema = test_task_tool().input_schema();
    assert!(schema["properties"].get("background").is_none());
}

#[tokio::test]
async fn task_returns_before_child_finishes() {
    let fixture = blocking_task_fixture().await;
    let output = fixture.tool.execute_with_context(&fixture.context, task_input()).await.unwrap();
    assert_eq!(output.metadata["status"], "running");
    assert!(output.metadata["child_id"].as_str().is_some());
    assert!(output.metadata["task_group_id"].as_str().is_some());
    assert!(!fixture.child_finished.load(Ordering::SeqCst));
}
```

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cargo test tools::meta::task::tests --lib`

Expected: FAIL because the schema still exposes `background` and the synchronous branch still exists.

- [ ] **Step 3: Replace both branches with one spawn path**

```rust
let group_id = if context.agent.parent_id.is_none() {
    let turn_id = context.origin_turn_id.ok_or_else(|| ToolError {
        message: "root task invocation is missing its trusted turn id".to_string(),
        code: Some("missing_turn_context".to_string()),
    })?;
    self.coordinator.current_or_create_root_group(context.agent, turn_id).await?
} else {
    self.coordinator.current_or_create_parent_group(context.agent).await?
};
let reservation = self.coordinator
    .reserve_child_in_group(context.agent, SpawnChildRequest::new(description), group_id.clone())
    .await?;
let child_context = reservation.context.clone();
let child_id = child_context.agent_id.clone();

let completion_coordinator = self.coordinator.clone();
let completion_context = child_context.clone();
let handle = tokio::spawn(async move {
    let terminal = match run_subagent_loop(/* existing arguments plus coordinator */).await {
        Ok(summary) => ChildTerminal::Completed { summary },
        Err(error) => ChildTerminal::Failed {
            code: error.code().to_string(),
            partial_result: error.partial_result.clone(),
        },
    };
    if let Err(error) = completion_coordinator
        .finish_child(&completion_context, terminal.clone())
        .await
    {
        tracing::error!(child_id = %completion_context.agent_id, error = %error, "failed to persist child terminal state");
    }
    terminal
});
self.coordinator.register_task(&child_context, handle).await?;
```

The coordinator's registered handle must be the only child owner. The registered future persists its own terminal through the coordinator so root children become deliverable even when no parent is synchronously joining them; later joins take the stored-terminal fast path and must not call `finish_child` twice. Use `child_context.agent_id` as the progress-store key in both task and RLM paths; do not generate progress UUIDs.

Add `turn_id: Option<String>` to `ExecuteToolRequest`. Pass the current trusted `TurnId` from `App::spawn_agent_turn` into `AgentLoop`, then through both sequential and parallel tool dispatch into `DaemonClient::execute_tool`. The daemon copies this transport field into `ToolContext::origin_turn_id`; it must continue ignoring model-supplied `_turn_id` or identity fields inside tool arguments. Nested subagent tool calls use `current_or_create_parent_group` and do not inherit the root's direct-child group. Update every `ToolContext` struct literal in `src/daemon/handlers.rs`, `src/teams/subagent_loop.rs`, `src/tools/executor.rs`, `src/tools/mod.rs`, `src/tools/meta/task/tests.rs`, and `tests/comet_integration_test.rs`; non-root/direct test contexts set `origin_turn_id: None` unless the test specifically exercises root-turn grouping.

- [ ] **Step 4: Return one structured acknowledgement**

```rust
let metadata = HashMap::from([
    ("child_id".to_string(), serde_json::json!(child_id.as_str())),
    ("task_group_id".to_string(), serde_json::json!(group_id.as_str())),
    ("status".to_string(), serde_json::json!("running")),
]);
Ok(ToolOutput {
    output_type: "json".to_string(),
    content: serde_json::json!({
        "child_id": child_id.as_str(),
        "task_group_id": group_id.as_str(),
        "status": "running"
    }).to_string(),
    metadata,
})
```

- [ ] **Step 5: Run task, RLM, and isolation tests**

Run: `cargo test tools::meta::task --lib && cargo test tools::meta::rlm --lib && cargo test tui::agent --lib && cargo test --test strict_subagent_isolation`

Expected: PASS.

- [ ] **Step 6: Commit the unified tool path**

```bash
git add src/tools/meta/task.rs src/tools/meta/task/tests.rs src/tools/meta/rlm/pipeline.rs src/daemon/models.rs src/daemon/handlers.rs src/tui/client.rs src/tui/agent/core.rs src/tui/agent/tool_dispatch.rs src/tui/app/turn.rs src/teams/subagent_loop.rs src/tools/executor.rs src/tools/mod.rs tests/comet_integration_test.rs
git commit -m "refactor(task): use one asynchronous subagent path"
```

### Task 4: Resume Non-Root Parents for Child-Result Synthesis

**Files:**
- Modify: `src/teams/subagent_loop.rs`
- Test: `src/teams/subagent_loop.rs`
- Test: `tests/unified_subagent_lifecycle.rs`

- [ ] **Step 1: Write a failing loop test for mandatory synthesis**

```rust
#[tokio::test]
async fn candidate_final_with_children_triggers_one_synthesis_round() {
    let fixture = scripted_loop(vec![
        assistant_final("draft without child evidence"),
        assistant_final("final with child evidence"),
    ]).with_completed_child("child evidence");
    let result = fixture.run().await.unwrap();
    assert_eq!(result, "final with child evidence");
    assert_eq!(fixture.api_call_count(), 2);
    assert!(fixture.second_request_contains("child evidence"));
}
```

- [ ] **Step 2: Run the focused test and verify failure**

Run: `cargo test teams::subagent_loop::tests::candidate_final_with_children_triggers_one_synthesis_round --lib`

Expected: FAIL because the loop returns the first non-tool assistant response immediately.

- [ ] **Step 3: Add coordinator-aware final candidate handling**

Change `run_subagent_loop` to receive `Arc<AgentCoordinator>`. At the current final-return branch, use:

```rust
let candidate = choice.message.content.unwrap_or_default();
let child_results = coordinator.collect_children_for_synthesis(context).await
    .map_err(coordinator_error)?;
if child_results.is_empty() {
    coordinator.begin_finalizing(context).await.map_err(coordinator_error)?;
    return Ok(candidate);
}
messages.push(ChatMessage::assistant(candidate));
messages.push(ChatMessage::system(format_child_result_batch(&child_results)));
continue;
```

Add an explicit conversion helper rather than relying on a nonexistent blanket `From` implementation:

```rust
fn coordinator_error(error: CoordinatorError) -> SubagentError {
    SubagentError {
        message: format!("subagent lifecycle coordination failed: {error}"),
        error_type: ErrorType::Unknown,
        partial_result: None,
    }
}
```

Use `.map_err(coordinator_error)?` for coordinator calls.

Track `children_synthesized` IDs so already-consumed terminal children are not injected on every later candidate response. After the first batch, new children spawned during synthesis require another barrier before final return.

- [ ] **Step 4: Add bounded structured formatting**

```rust
fn format_child_result_batch(results: &[ChildResult]) -> String {
    let body = serde_json::to_string(results).unwrap_or_else(|error| {
        format!(r#"{{"error":"serialize_child_results","message":{}}}"#, serde_json::json!(error.to_string()))
    });
    format!("<child-results>\n{}\n</child-results>", body)
}
```

Return serialization errors with actionable context; do not use `unwrap()` in production code.

- [ ] **Step 5: Run subagent-loop and lifecycle tests**

Run: `cargo test teams::subagent_loop --lib && cargo test --test unified_subagent_lifecycle non_root`

Expected: PASS.

- [ ] **Step 6: Commit the synthesis barrier**

```bash
git add src/teams/subagent_loop.rs tests/unified_subagent_lifecycle.rs
git commit -m "feat(agent): synthesize child results before finalizing"
```

### Task 5: Expose Atomic Root-Group Delivery Through the Daemon

**Files:**
- Modify: `src/daemon/models.rs`
- Modify: `src/daemon/handlers.rs`
- Modify: `src/daemon/routes.rs`
- Modify: `src/daemon/state.rs`
- Modify: `src/tui/client.rs`
- Test: `tests/strict_subagent_isolation.rs`

- [ ] **Step 1: Write failing API tests for claim and generation reset**

```rust
#[tokio::test]
async fn root_delivery_claim_is_atomic_and_session_scoped() {
    let app = daemon_fixture_with_ready_group("session-a").await;
    let first = claim_group(&app, "session-a", 0).await;
    let second = claim_group(&app, "session-a", 0).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::NO_CONTENT);
    assert_eq!(claim_group(&app, "session-b", 0).await.status(), StatusCode::NO_CONTENT);
}
```

- [ ] **Step 2: Run the API test and verify failure**

Run: `cargo test --test strict_subagent_isolation root_delivery_claim_is_atomic_and_session_scoped`

Expected: FAIL because no delivery endpoint exists.

- [ ] **Step 3: Add request/response models and routes**

```rust
#[derive(Debug, Deserialize)]
pub struct ClaimTaskGroupRequest {
    pub session_id: String,
    pub generation: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskGroupDeliveryResponse {
    pub group_id: String,
    pub generation: u64,
    pub results: Vec<ChildResult>,
}
```

Add protected routes:

```rust
.route("/api/v1/agents/task-groups/claim", post(handlers::claim_task_group))
.route("/api/v1/agents/generation/reset", post(handlers::reset_agent_generation))
```

The claim handler returns `200` with a delivery or `204` when none is ready. The reset handler advances the generation and calls coordinator subtree cancellation for the old root generation before returning the new generation.

- [ ] **Step 4: Add typed client methods**

```rust
pub async fn claim_task_group(
    &self,
    session_id: &str,
    generation: u64,
) -> anyhow::Result<Option<TaskGroupDeliveryResponse>>;

pub async fn reset_agent_generation(&self, session_id: &str) -> anyhow::Result<u64>;
```

Use `.context("claim ready task group")` and `.context("reset agent generation")` at decode/request boundaries.

- [ ] **Step 5: Run daemon and client tests**

Run: `cargo test --test strict_subagent_isolation && cargo test tui::client::tests --lib`

Expected: PASS.

- [ ] **Step 6: Commit the delivery API**

```bash
git add src/daemon/models.rs src/daemon/handlers.rs src/daemon/routes.rs src/daemon/state.rs src/tui/client.rs tests/strict_subagent_isolation.rs
git commit -m "feat(daemon): expose atomic subagent result delivery"
```

### Task 6: Schedule Synthetic Continuation Turns in the TUI

**Files:**
- Create: `src/tui/app/continuation.rs`
- Modify: `src/tui/app/mod.rs`
- Modify: `src/tui/app/types.rs`
- Modify: `src/tui/app/turn.rs`
- Modify: `src/tui/app/event.rs`
- Modify: `src/tui/agent/mod.rs`
- Modify: `src/tui/agent/compaction.rs`
- Test: `src/tui/app/continuation.rs`
- Test: `tests/unified_subagent_lifecycle.rs`

- [ ] **Step 1: Write failing scheduler tests for idle and racing delivery**

```rust
#[tokio::test]
async fn idle_app_starts_hidden_continuation_for_claimed_group() {
    let mut app = app_with_claimed_delivery(delivery("g1"));
    app.poll_ready_task_groups().await;
    assert!(matches!(app.pending_inputs.front().unwrap().kind, TurnInputKind::Continuation(_)));
    assert_eq!(app.visible_user_message_count(), 0);
}

#[tokio::test]
async fn queued_user_turn_wins_and_consumes_delivery_once() {
    let mut app = app_with_user_input_and_delivery(delivery("g1"));
    app.poll_ready_task_groups().await;
    assert_eq!(app.pending_inputs.len(), 1);
    assert!(app.pending_inputs[0].continuation.is_some());
    assert_eq!(app.synthetic_turn_count(), 0);
}
```

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cargo test tui::app::continuation::tests --lib`

Expected: FAIL because continuation scheduling and typed pending input do not exist.

- [ ] **Step 3: Introduce typed turn input without fabricating a user message**

```rust
pub enum TurnInputKind {
    User(String),
    Continuation(TaskGroupDeliveryResponse),
}

pub struct PendingInput {
    pub display_text: String,
    pub agent_input: String,
    pub kind: TurnInputKind,
    pub continuation: Option<TaskGroupDeliveryResponse>,
}
```

Refactor `spawn_agent_turn` into `spawn_agent_turn(TurnInputKind, bool)`. For a continuation, append a structured system message to history and call a new `AgentLoop::process_continuation(delivery)` method; do not append a `ChatMessage::user` or visible user row.

- [ ] **Step 4: Poll and atomically claim only when consumption is possible**

```rust
pub(super) async fn poll_ready_task_groups(&mut self) {
    if self.current_turn_handle.is_some() {
        return;
    }
    let Some(delivery) = self.daemon_client
        .claim_task_group(&self.session_id, self.agent_generation)
        .await
        .ok()
        .flatten() else { return; };
    if let Some(next) = self.pending_inputs.front_mut() {
        next.continuation = Some(delivery);
    } else {
        self.pending_inputs.push_back(PendingInput::continuation(delivery));
    }
    self.start_next_turn();
}
```

Throttle the existing 100ms `Tick` path to a 500ms claim interval so it does not generate excessive HTTP traffic. Atomicity remains daemon-owned.

- [ ] **Step 5: Remove subagent delivery from the command background manager**

Delete the `result_type == "subagent"` branch in `inject_background_results`; retain this endpoint only for command-background execution. Remove `BackgroundTaskResult` usage for subagent completion. Structured deliveries enter the model through continuation input and the visible result through the resulting assistant response.

- [ ] **Step 6: Run TUI scheduler and end-to-end tests**

Run: `cargo test tui::app::continuation --lib && cargo test --test unified_subagent_lifecycle main_root`

Expected: PASS, including no-user-input automatic follow-up and user-input race cases.

- [ ] **Step 7: Commit continuation scheduling**

```bash
git add src/tui/app/continuation.rs src/tui/app/mod.rs src/tui/app/types.rs src/tui/app/turn.rs src/tui/app/event.rs src/tui/agent/mod.rs src/tui/agent/compaction.rs tests/unified_subagent_lifecycle.rs
git commit -m "feat(tui): synthesize completed subagent groups"
```

### Task 7: Reset Generations and Cancel Old Trees on Clear/Shutdown

**Files:**
- Modify: `src/tui/app/input.rs`
- Modify: `src/tui/app/turn.rs`
- Modify: `src/tui/app/mod.rs`
- Modify: `src/agent/coordinator.rs`
- Modify: `src/daemon/handlers.rs`
- Modify: `src/daemon/routes.rs`
- Modify: `src/tui/client.rs`
- Test: `tests/unified_subagent_lifecycle.rs`

- [ ] **Step 1: Write failing reset and late-delivery tests**

```rust
#[tokio::test]
async fn clear_cancels_old_generation_and_rejects_late_results() {
    let fixture = running_root_child_fixture().await;
    fixture.app.submit_input("/clear".into());
    fixture.complete_old_child("late").await;
    assert!(fixture.client.claim_task_group("s", fixture.old_generation).await.unwrap().is_none());
    assert_eq!(fixture.app.agent_generation, fixture.old_generation + 1);
}
```

- [ ] **Step 2: Run the focused test and verify failure**

Run: `cargo test --test unified_subagent_lifecycle clear_cancels_old_generation_and_rejects_late_results`

Expected: FAIL because `/clear` only clears UI/history and aborts the current turn.

- [ ] **Step 3: Reset generation before clearing local state**

On `/clear`, spawn an async reset request and deliver `AppEvent::AgentGenerationReset { generation }`. Until it completes, set `suppress_phase_updates` and do not start queued work. Handle reset failure by showing an actionable system message and retaining the old generation rather than pretending cancellation succeeded.

```rust
AppEvent::AgentGenerationReset { generation } => {
    self.agent_generation = generation;
    self.subagent_tree.clear();
    self.completed_at.clear();
    self.agent_navigation = AgentNavigationState::default();
}
```

- [ ] **Step 4: Route application shutdown through coordinator cancellation**

Add `POST /api/v1/agents/session/cancel` in `src/daemon/routes.rs`, implement `cancel_agent_session` in `src/daemon/handlers.rs`, and add `DaemonClient::cancel_agent_session`. The handler resolves the trusted root, calls coordinator root-subtree cancellation, waits for `shutdown_timeout`, persists `Cancelled` descendants, and releases every permit. Assert no synthetic continuation is scheduled for the cancelled generation.

- [ ] **Step 5: Run lifecycle cancellation tests**

Run: `cargo test --test unified_subagent_lifecycle clear shutdown && cargo test agent::coordinator::tests --lib`

Expected: PASS.

- [ ] **Step 6: Commit generation reset behavior**

```bash
git add src/tui/app/input.rs src/tui/app/turn.rs src/tui/app/mod.rs src/agent/coordinator.rs src/daemon/handlers.rs src/daemon/routes.rs src/tui/client.rs tests/unified_subagent_lifecycle.rs
git commit -m "fix(agent): cancel obsolete subagent generations"
```

### Task 8: Wire Capability-Scoped Selector Navigation

**Files:**
- Modify: `src/tui/app/event_key.rs`
- Modify: `src/tui/app/event.rs`
- Modify: `src/tui/app/types.rs`
- Modify: `src/tui/client.rs`
- Modify: `src/tui/components/subagent_tree.rs`
- Modify: `src/tui/components/subagent_focus_view.rs`
- Test: `src/tui/components/subagent_focus_view.rs`
- Test: `tests/strict_subagent_isolation.rs`

- [ ] **Step 1: Write failing descent/back-stack tests**

```rust
#[tokio::test]
async fn enter_on_child_uses_capability_and_replaces_local_view() {
    let mut app = app_with_root_view(root_with_child("child", "cap-1"));
    app.open_focus_on("child");
    app.handle_key_event(key(KeyCode::Enter));
    assert_eq!(app.agent_navigation.back_stack.len(), 1);
    assert_eq!(app.subagent_tree.local_view.as_ref().unwrap().self_view.agent_id, "child");
}

#[tokio::test]
async fn back_navigation_restores_previous_scoped_view() {
    let mut app = descended_app();
    app.navigate_back();
    assert_eq!(app.subagent_tree.local_view.as_ref().unwrap().self_view.agent_id, "root");
}
```

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cargo test tui::components::subagent_focus_view --lib`

Expected: FAIL because selector Enter only rebuilds from the current in-memory tree.

- [ ] **Step 3: Preserve child capabilities in selector nodes**

Add `navigation_capability: Option<String>` to `SubagentNode`. In `replace_local`, set it from each `DirectChildResponse`; the self node has `None`. Do not expose parent IDs or descendant lists.

- [ ] **Step 4: Add async navigation events**

```rust
AppEvent::NavigateAgent { capability } => { /* spawn client.navigate_agent_view */ }
AppEvent::AgentViewNavigated(view) => {
    if let Some(current) = self.agent_navigation.current.take() {
        self.agent_navigation.back_stack.push(current);
    }
    self.agent_navigation.current = Some(AgentViewFrame::from_view((*view).clone()));
    self.subagent_tree.replace_local(*view);
}
AppEvent::NavigateAgentBack => self.restore_previous_agent_view(),
```

Use Enter on a selected direct child to dispatch `NavigateAgent`; use Backspace for one-level back navigation while focus view is open. A failed, stale, or unauthorized capability leaves the current view intact and adds a concise system error.

- [ ] **Step 5: Verify strict isolation remains intact**

Run: `cargo test --test strict_subagent_isolation && cargo test tui::components::subagent_focus_view --lib`

Expected: PASS; root cannot see grandchildren until it navigates using a valid direct-child capability.

- [ ] **Step 6: Commit scoped navigation**

```bash
git add src/tui/app/event_key.rs src/tui/app/event.rs src/tui/app/types.rs src/tui/client.rs src/tui/components/subagent_tree.rs src/tui/components/subagent_focus_view.rs tests/strict_subagent_isolation.rs
git commit -m "feat(tui): navigate scoped subagent views"
```

### Task 9: Remove Legacy Mode Guidance and Complete Verification

**Files:**
- Modify: `src/prompts/base.md`
- Modify: `src/prompts/init_instructions.md`
- Modify: `CHANGELOG.md`
- Modify: `src/tools/meta/task/tests.rs`
- Test: `tests/unified_subagent_lifecycle.rs`

- [ ] **Step 1: Add a repository contract test for removed mode selection**

```rust
#[test]
fn bundled_prompts_do_not_instruct_models_to_select_background_mode() {
    for path in ["src/prompts/base.md", "src/prompts/init_instructions.md"] {
        let text = std::fs::read_to_string(path).expect("read bundled prompt");
        assert!(!text.contains("background: true"), "legacy instruction in {path}");
        assert!(!text.contains("background: false"), "legacy instruction in {path}");
    }
}
```

- [ ] **Step 2: Run the contract test and verify failure**

Run: `cargo test --test unified_subagent_lifecycle bundled_prompts_do_not_instruct_models_to_select_background_mode`

Expected: FAIL because bundled prompts still describe the old switch.

- [ ] **Step 3: Update prompts and compatibility behavior**

Describe `task` as concurrent structured delegation: it returns immediately, parents synthesize children before terminal completion, and main-agent results may arrive through automatic continuation. During the compatibility window, ignore an incoming `background` property and include `"ignored_arguments":["background"]` in acknowledgement metadata; do not branch on its value.

- [ ] **Step 4: Update changelog and architecture documentation**

Add an Unreleased entry covering the removed runtime mode split, persistent-root continuation, generation-safe cancellation, and scoped selector navigation. If WGENTY.md documents task execution modes, update it in the same commit.

- [ ] **Step 5: Run targeted and full verification**

Run:

```bash
cargo test --test unified_subagent_lifecycle
cargo test --test strict_subagent_isolation
cargo test --all
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```

Expected: every command exits 0 with no warnings or formatting differences.

- [ ] **Step 6: Check release-build performance constraints**

Run:

```bash
cargo build --release
time ./target/release/wgenty_code --version
/usr/bin/time -l ./target/release/wgenty_code --version 2> /tmp/wgenty-code-memory.txt
ls -lh ./target/release/wgenty_code
```

Expected: startup regression is no more than 5%, the `maximum resident set size` reported in `/tmp/wgenty-code-memory.txt` increases by no more than 2%, and binary size increases by no more than 500KB versus measurements captured from the pre-implementation commit with the same commands. Record both baselines and final values in the PR description; do not claim compliance without both measurements.

- [ ] **Step 7: Commit documentation and compatibility cleanup**

```bash
git add src/prompts/base.md src/prompts/init_instructions.md src/tools/meta/task/tests.rs CHANGELOG.md WGENTY.md tests/unified_subagent_lifecycle.rs
git commit -m "docs(agent): document structured subagent concurrency"
```

Only include `WGENTY.md` in `git add` when it actually required changes.

## Final Review Checklist

- [ ] Every `task` invocation uses the same immediate-return execution path.
- [ ] Every non-root parent gets a post-child reasoning round before terminal state.
- [ ] The main agent completes turns without entering a terminal agent state.
- [ ] Ready root groups are consumed exactly once by a user turn or synthetic continuation.
- [ ] `/clear` and shutdown prevent obsolete result injection and release permits.
- [ ] Progress and lifecycle joins use coordinator-issued agent IDs.
- [ ] Selector navigation remains capability-scoped and never flattens hidden descendants.
- [ ] Command-background execution remains functional and separate from subagent delivery.
- [ ] Changelog, bundled prompts, and any architecture docs agree with runtime behavior.
- [ ] Full tests, formatting, clippy, and proportional performance verification pass.
