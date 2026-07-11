# Strict Subagent Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce a uniform agent-local view of self plus direct children, derive hierarchy from trusted runtime context, and prevent scoped subagents from outliving their parent.

**Architecture:** Add typed agent identity, a canonical in-memory hierarchy store, and an `AgentCoordinator` that exclusively owns spawning, lifecycle transitions, joins, cancellation, and recovery. Propagate immutable `AgentExecutionContext` through tool execution and nested model loops, then replace full-session daemon/TUI tree access with capability-authorized local projections and layer-by-layer navigation.

**Tech Stack:** Rust 2021, Tokio, `tokio-util::sync::CancellationToken`, Axum, Serde, thiserror, anyhow, Ratatui, UUID, Chrono, HMAC-SHA-256, `rand`, Cargo tests.

---

## File Map

**New core files**

- `src/agent/identity.rs`: typed IDs plus trusted agent and tool execution contexts.
- `src/agent/store.rs`: canonical records, indexed local projections, uniform visibility authorization, transcript/result association, and recovery queries.
- `src/agent/coordinator.rs`: spawning, semaphore ownership, lifecycle transitions, joining, recursive cancellation, and restart recovery.
- `src/agent/capability.rs`: opaque, expiring, viewer-bound TUI navigation capabilities.
- `tests/strict_subagent_isolation.rs`: end-to-end hierarchy, structured concurrency, and information-flow contracts.

**Existing files with focused changes**

- `Cargo.toml`, `Cargo.lock`: add `tokio-util` cancellation support.
- `src/agent/mod.rs`, `src/agent/progress.rs`: export new modules and align progress status with coordinator lifecycle.
- `src/tools/mod.rs`, `src/tools/executor.rs`: contextual tool adapter and propagation through hooks.
- `src/teams/subagent_loop.rs`: execute nested tools with the child context and observe cancellation.
- `src/tools/meta/task.rs`, `src/tools/meta/task/tests.rs`: delegate all child creation and completion to the coordinator; remove model-controlled identity and polling.
- `src/tools/meta/rlm/mod.rs`, `src/tools/meta/rlm/pipeline.rs`, `src/tools/meta/run_script.rs`: use coordinator-created child contexts for every nested loop.
- `src/tools/meta/subagent_trace.rs`, `src/transcript/mod.rs`, `src/teams/subagent_mailbox.rs`: authorize transcript and large-result access through caller context.
- `src/daemon/state.rs`, `src/daemon/models.rs`, `src/daemon/handlers.rs`, `src/daemon/routes.rs`: own root contexts, expose scoped APIs, and remove ordinary access to the full progress map.
- `src/tui/client.rs`, `src/tui/agent/mod.rs`, `src/tui/agent/core.rs`, `src/tui/agent/tool_dispatch.rs`: carry the root execution scope and poll local views.
- `src/tui/app/types.rs`, `src/tui/app/mod.rs`, `src/tui/app/event.rs`, `src/tui/app/event_key.rs`: store local-view navigation history and capability-driven events.
- `src/tui/components/subagent_tree.rs`, `src/tui/components/subagent_focus_view.rs`, `src/tui/components/subagent_status_bar.rs`: render only the currently loaded local projection.
- `src/teams/subagent.rs`, `src/teams/mod.rs`, `src/services/mod.rs`: retire the flat globally listable `AgentsService` path.
- `README.md`, `WGENTY.md`: document trusted depth, strict local visibility, and scoped background semantics.
- `tests/refactor_e2e_test.rs`: update direct `run_subagent_loop` calls for trusted context.

### Task 1: Add Typed Identity and Lifecycle Contracts

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/agent/identity.rs`
- Modify: `src/agent/progress.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Add runtime security dependencies**

Add this dependency beside the existing Tokio dependency:

```toml
tokio-util = { version = "0.7", features = ["rt"] }
hmac = "0.12"
rand = "0.8"
```

Run: `cargo check`

Expected: PASS and `Cargo.lock` contains a `tokio-util` package entry.

- [ ] **Step 2: Write identity and lifecycle unit tests**

Append tests in `src/agent/identity.rs` that require typed IDs, a root context, a derived child context, terminal-state classification, and a non-forgeable depth relation:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_context_is_derived_from_parent() {
        let root = AgentExecutionContext::root(SessionId::new("session-a"));
        let child = root.child(AgentId::new("child-a"));

        assert_eq!(child.session_id, root.session_id);
        assert_eq!(child.parent_id.as_ref(), Some(&root.agent_id));
        assert_eq!(child.depth, 1);
        assert!(!child.cancellation.is_cancelled());
    }

    #[test]
    fn only_completed_failed_and_cancelled_are_terminal() {
        assert!(!AgentLifecycleStatus::Pending.is_terminal());
        assert!(!AgentLifecycleStatus::WaitingForChildren.is_terminal());
        assert!(!AgentLifecycleStatus::Finalizing.is_terminal());
        assert!(AgentLifecycleStatus::Completed.is_terminal());
        assert!(AgentLifecycleStatus::Failed.is_terminal());
        assert!(AgentLifecycleStatus::Cancelled.is_terminal());
    }
}
```

- [ ] **Step 3: Run the identity tests to verify they fail**

Run: `cargo test agent::identity::tests --lib`

Expected: FAIL because `identity` and the named types do not exist.

- [ ] **Step 4: Implement the identity contracts**

Create `src/agent/identity.rs` with these public types and exact constructors:

```rust
use serde::{Deserialize, Serialize};
use std::fmt;
use tokio_util::sync::CancellationToken;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(SessionId);
string_id!(AgentId);
string_id!(ToolInvocationId);

#[derive(Debug, Clone)]
pub struct AgentExecutionContext {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub depth: usize,
    pub cancellation: CancellationToken,
}

impl AgentExecutionContext {
    pub fn root(session_id: SessionId) -> Self {
        Self {
            agent_id: AgentId::new(uuid::Uuid::new_v4().to_string()),
            session_id,
            parent_id: None,
            depth: 0,
            cancellation: CancellationToken::new(),
        }
    }

    pub(crate) fn child(&self, agent_id: AgentId) -> Self {
        Self {
            session_id: self.session_id.clone(),
            agent_id,
            parent_id: Some(self.agent_id.clone()),
            depth: self.depth + 1,
            cancellation: self.cancellation.child_token(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentLifecycleStatus {
    Pending,
    Running,
    WaitingForChildren,
    Finalizing,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl AgentLifecycleStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

pub struct ToolContext<'a> {
    pub agent: &'a AgentExecutionContext,
    pub invocation_id: ToolInvocationId,
}
```

Export the module and types from `src/agent/mod.rs`. Extend `SubagentStatus` in `src/agent/progress.rs` with `WaitingForChildren`, `Finalizing`, and `Cancelling`, preserving Serde compatibility for all existing variants.

- [ ] **Step 5: Run formatting and unit tests**

Run: `cargo fmt && cargo test agent::identity::tests agent::progress::tests --lib`

Expected: PASS.

- [ ] **Step 6: Commit the contracts**

```bash
git add Cargo.toml Cargo.lock src/agent/identity.rs src/agent/progress.rs src/agent/mod.rs
git commit -m "feat(agent): add trusted execution identity contracts"
```

### Task 2: Build the Canonical Hierarchy Store and Strict Local Projections

**Files:**
- Create: `src/agent/store.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Write authorization matrix tests**

In `src/agent/store.rs`, add tests that create `root -> child -> grandchild`, a sibling, another branch, and another session. Use this helper and assertions:

```rust
fn record(session: &str, id: &str, parent: Option<&str>, depth: usize) -> AgentRecord {
    AgentRecord::new(
        SessionId::new(session),
        AgentId::new(id),
        parent.map(AgentId::new),
        depth,
    )
}

#[tokio::test]
async fn local_view_contains_only_self_and_direct_children() {
    let store = InMemoryAgentStore::default();
    store.insert(record("s", "root", None, 0)).await.unwrap();
    store.insert(record("s", "a", Some("root"), 1)).await.unwrap();
    store.insert(record("s", "b", Some("root"), 1)).await.unwrap();
    store.insert(record("s", "a1", Some("a"), 2)).await.unwrap();

    let view = store.local_view(&SessionId::new("s"), &AgentId::new("a")).await.unwrap();
    assert_eq!(view.self_view.agent_id.as_str(), "a");
    assert_eq!(view.children.iter().map(|c| c.agent_id.as_str()).collect::<Vec<_>>(), vec!["a1"]);
}

#[tokio::test]
async fn hidden_and_missing_targets_share_not_visible() {
    let store = seeded_store().await;
    for target in ["root", "sibling", "grandchild", "other-branch", "missing"] {
        assert_eq!(
            store.authorize_target(&SessionId::new("s"), &AgentId::new("child"), &AgentId::new(target)).await,
            Err(StoreError::NotVisible),
        );
    }
    assert_eq!(
        store.authorize_target(&SessionId::new("other"), &AgentId::new("child"), &AgentId::new("child")).await,
        Err(StoreError::NotVisible),
    );
}
```

The `seeded_store` helper must insert records whose IDs exactly match the assertion list.

- [ ] **Step 2: Run the store tests to verify they fail**

Run: `cargo test agent::store::tests --lib`

Expected: FAIL because the store and projection types do not exist.

- [ ] **Step 3: Implement records, projections, indexes, and uniform authorization**

Define these public contracts in `src/agent/store.rs`:

```rust
#[derive(Debug, Clone)]
pub struct AgentRecord {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub depth: usize,
    pub generation: u64,
    pub status: AgentLifecycleStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub summary: Option<ChildSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfView {
    pub agent_id: AgentId,
    pub status: AgentLifecycleStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectChildView {
    pub agent_id: AgentId,
    pub status: AgentLifecycleStatus,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAgentView {
    pub self_view: SelfView,
    pub children: Vec<DirectChildView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSummary {
    pub text: String,
    pub error_code: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("agent is not visible from the current execution scope")]
    NotVisible,
    #[error("agent record already exists")]
    AlreadyExists,
    #[error("agent store invariant failed: {0}")]
    Invariant(String),
}
```

Implement `InMemoryAgentStore` with an `Arc<RwLock<StoreState>>`. `StoreState` must contain a unique `(SessionId, AgentId)` record map and a `(SessionId, Option<AgentId>) -> Vec<AgentId>` child index. Provide:

```rust
pub async fn insert(&self, record: AgentRecord) -> Result<(), StoreError>;
pub async fn local_view(&self, session: &SessionId, caller: &AgentId) -> Result<LocalAgentView, StoreError>;
pub async fn authorize_target(&self, session: &SessionId, caller: &AgentId, target: &AgentId) -> Result<AgentRecord, StoreError>;
pub async fn direct_children(&self, session: &SessionId, parent: &AgentId) -> Result<Vec<AgentRecord>, StoreError>;
pub(crate) async fn descendants(&self, session: &SessionId, root: &AgentId) -> Result<Vec<AgentRecord>, StoreError>;
pub(crate) async fn update_status(&self, session: &SessionId, agent: &AgentId, status: AgentLifecycleStatus) -> Result<(), StoreError>;
```

`authorize_target` must return the same `StoreError::NotVisible` for absent, cross-session, parent, sibling, grandchild, and other-branch targets. `descendants` is crate-private and must never be used by an agent-facing read path.

- [ ] **Step 4: Run store tests and Clippy**

Run: `cargo fmt && cargo test agent::store::tests --lib && cargo clippy --lib -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Commit the local authorization boundary**

```bash
git add src/agent/store.rs src/agent/mod.rs
git commit -m "feat(agent): enforce strict local hierarchy projections"
```

### Task 3: Implement Coordinator Spawning and Concurrency Ownership

**Files:**
- Create: `src/agent/coordinator.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Write spawn, depth, and permit tests**

Add Tokio tests in `src/agent/coordinator.rs` covering these exact behaviors:

```rust
#[tokio::test]
async fn spawn_derives_parent_depth_and_session() {
    let coordinator = test_coordinator(4, 3);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let child = coordinator.reserve_child(&root, SpawnChildRequest::new("work")).await.unwrap();

    assert_eq!(child.context.session_id, root.session_id);
    assert_eq!(child.context.parent_id.as_ref(), Some(&root.agent_id));
    assert_eq!(child.context.depth, root.depth + 1);
}

#[tokio::test]
async fn depth_limit_is_enforced_by_coordinator() {
    let coordinator = test_coordinator(4, 1);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let child = coordinator.reserve_child(&root, SpawnChildRequest::new("child")).await.unwrap();
    assert!(matches!(
        coordinator.reserve_child(&child.context, SpawnChildRequest::new("too deep")).await,
        Err(CoordinatorError::DepthLimitReached { limit: 1 })
    ));
}

#[tokio::test]
async fn semaphore_permit_returns_after_terminal_cleanup() {
    let coordinator = test_coordinator(1, 3);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let child = coordinator.reserve_child(&root, SpawnChildRequest::new("first")).await.unwrap();
    coordinator.finish_child(&child.context, ChildTerminal::completed("done")).await.unwrap();
    assert!(coordinator.reserve_child(&root, SpawnChildRequest::new("second")).await.is_ok());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test agent::coordinator::tests --lib`

Expected: FAIL because the coordinator contracts do not exist.

- [ ] **Step 3: Implement coordinator errors and reservation types**

Use these exact public contracts:

```rust
#[derive(Debug, Clone)]
pub struct SpawnChildRequest {
    pub label: String,
}

impl SpawnChildRequest {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into() }
    }
}

pub struct ChildReservation {
    pub context: AgentExecutionContext,
    permit: tokio::sync::OwnedSemaphorePermit,
}

#[derive(Debug, Clone)]
pub enum ChildTerminal {
    Completed { summary: String },
    Failed { code: String, partial_result: Option<String> },
    Cancelled,
}

impl ChildTerminal {
    pub fn completed(summary: impl Into<String>) -> Self {
        Self::Completed { summary: summary.into() }
    }
}

struct ScopeState {
    status: AgentLifecycleStatus,
    cancellation: CancellationToken,
    task: Option<tokio::task::JoinHandle<ChildTerminal>>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    terminal: Option<ChildTerminal>,
}

#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("agent is not visible from the current execution scope")]
    NotVisible,
    #[error("maximum subagent depth {limit} reached")]
    DepthLimitReached { limit: usize },
    #[error("subagent concurrency is closed")]
    ConcurrencyClosed,
    #[error("parent agent is not running")]
    ParentNotRunning,
    #[error("child join failed: {0}")]
    JoinFailed(String),
    #[error("agent storage failed: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct AgentCoordinator {
    store: InMemoryAgentStore,
    permits: Arc<tokio::sync::Semaphore>,
    max_depth: usize,
    scopes: Arc<RwLock<HashMap<(SessionId, AgentId), ScopeState>>>,
}
```

Implement:

```rust
pub async fn ensure_root(&self, session_id: SessionId) -> Result<AgentExecutionContext, CoordinatorError>;
pub async fn reserve_child(&self, caller: &AgentExecutionContext, request: SpawnChildRequest) -> Result<ChildReservation, CoordinatorError>;
pub async fn finish_child(&self, child: &AgentExecutionContext, terminal: ChildTerminal) -> Result<(), CoordinatorError>;
pub async fn list_local(&self, caller: &AgentExecutionContext) -> Result<LocalAgentView, CoordinatorError>;
```

Acquire an owned semaphore permit before inserting a runnable child record. Store the permit in `ScopeState` when the reservation is activated; release it only after terminal persistence. Reject spawning from `Cancelling` or terminal parents. Do not use polling or `AtomicUsize`.

- [ ] **Step 4: Run coordinator tests**

Run: `cargo fmt && cargo test agent::coordinator::tests --lib`

Expected: PASS.

- [ ] **Step 5: Commit coordinator spawning**

```bash
git add src/agent/coordinator.rs src/agent/mod.rs
git commit -m "feat(agent): coordinate child spawning and concurrency"
```

### Task 4: Enforce Structured Finalization, Join Policies, and Recursive Cancellation

**Files:**
- Modify: `src/agent/coordinator.rs`
- Modify: `src/agent/store.rs`

- [x] **Step 1: Write terminal-state and cancellation tests**

Add tests with controlled child futures using `tokio::sync::oneshot`:

```rust
#[tokio::test]
async fn parent_waits_for_live_direct_children_before_completion() {
    let (coordinator, root, child) = running_parent_and_child().await;
    let coordinator_for_finalize = coordinator.clone();
    let root_for_finalize = root.clone();
    let finalize = tokio::spawn(async move {
        coordinator_for_finalize
            .finalize_scope(&root_for_finalize, ParentOutcome::Completed("root done".into()), JoinPolicy::AllRequired)
            .await
    });
    tokio::task::yield_now().await;
    assert_eq!(coordinator.status(&root).await.unwrap(), AgentLifecycleStatus::WaitingForChildren);

    let coordinator_for_finish = coordinator.clone();
    let child_for_finish = child.clone();
    let finish = tokio::spawn(async move {
        coordinator_for_finish.finish_child(&child_for_finish, ChildTerminal::completed("child done")).await.unwrap();
    });
    finish.await.unwrap();
    let results = finalize.await.unwrap().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(coordinator.status(&root).await.unwrap(), AgentLifecycleStatus::Completed);
}

#[tokio::test]
async fn cancelling_parent_terminates_descendants_bottom_up() {
    let fixture = three_level_running_tree().await;
    fixture.coordinator.cancel_scope(&fixture.root).await.unwrap();
    assert_eq!(fixture.coordinator.status(&fixture.grandchild).await.unwrap(), AgentLifecycleStatus::Cancelled);
    assert_eq!(fixture.coordinator.status(&fixture.child).await.unwrap(), AgentLifecycleStatus::Cancelled);
    assert_eq!(fixture.coordinator.status(&fixture.root).await.unwrap(), AgentLifecycleStatus::Cancelled);
    assert_eq!(fixture.coordinator.available_permits(), fixture.max_concurrent);
}
```

Also test `BestEffort` waits for all children and `FailFast` cancels remaining children after the first required failure.

- [x] **Step 2: Run lifecycle tests to verify they fail**

Run: `cargo test agent::coordinator::tests --lib`

Expected: FAIL because finalization, joining, and cancellation are not implemented.

- [x] **Step 3: Implement lifecycle and result contracts**

Add:

```rust
#[derive(Debug, Clone, Copy)]
pub enum JoinPolicy {
    AllRequired,
    BestEffort,
    FailFast,
}

#[derive(Debug, Clone)]
pub enum ParentOutcome {
    Completed(String),
    Failed { code: String, partial_result: Option<String> },
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildResult {
    pub child_id: AgentId,
    pub status: ChildTerminalStatus,
    pub summary: String,
    pub error_code: Option<String>,
    pub partial_result: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ChildTerminalStatus {
    Completed,
    Failed,
    Cancelled,
}
```

Implement:

```rust
pub async fn join_children(&self, caller: &AgentExecutionContext, policy: JoinPolicy) -> Result<Vec<ChildResult>, CoordinatorError>;
pub async fn finalize_scope(&self, caller: &AgentExecutionContext, outcome: ParentOutcome, policy: JoinPolicy) -> Result<Vec<ChildResult>, CoordinatorError>;
pub async fn cancel_scope(&self, caller: &AgentExecutionContext) -> Result<(), CoordinatorError>;
pub async fn cancel_subtree(&self, caller: &AgentExecutionContext, target: AgentId) -> Result<(), CoordinatorError>;
```

Cancellation order must be: set `Cancelling`, cancel the scope token, recursively signal direct children, await handles with the configured shutdown timeout, abort uncooperative tasks, persist child terminal states, release child permits, then persist the requested parent terminal state. `cancel_subtree` must authorize self or direct child through the same store predicate and map all hidden/absent cases to `NotVisible`.

- [x] **Step 4: Verify structured concurrency tests**

Run: `cargo fmt && cargo test agent::coordinator::tests --lib`

Expected: PASS, including permit-release assertions.

- [x] **Step 5: Commit lifecycle enforcement**

```bash
git add src/agent/coordinator.rs src/agent/store.rs
git commit -m "feat(agent): enforce structured subagent lifetimes"
```

### Task 5: Add Contextual Tool Execution Without Rewriting Context-Free Tools

**Files:**
- Modify: `src/tools/mod.rs`
- Modify: `src/tools/executor.rs`

- [ ] **Step 1: Write adapter and executor propagation tests**

Add a test tool that records the caller identity in `src/tools/mod.rs` tests:

```rust
struct ContextProbe;

#[async_trait]
impl Tool for ContextProbe {
    fn name(&self) -> &str { "context_probe" }
    fn description(&self) -> &str { "records trusted context" }
    fn input_schema(&self) -> serde_json::Value { serde_json::json!({"type": "object"}) }
    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        panic!("contextual path must be used")
    }
    async fn execute_with_context(&self, context: &ToolContext<'_>, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(context.agent.agent_id.to_string()))
    }
}
```

Assert `ToolRegistry::execute_with_context` returns the trusted agent ID even when input contains forged `_agent_id`, `_session_id`, and `_subagent_depth`. Add an executor test proving `execute_with_hooks` passes the same `ToolContext` through pre/post hooks.

- [ ] **Step 2: Run contextual execution tests to verify they fail**

Run: `cargo test tools::external_tool_tests tools::executor::tests --lib`

Expected: FAIL because contextual methods and `ToolOutput::text` do not exist.

- [ ] **Step 3: Implement the backward-compatible adapter**

Add to `Tool`:

```rust
async fn execute_with_context(
    &self,
    _context: &ToolContext<'_>,
    input: serde_json::Value,
) -> Result<ToolOutput, ToolError> {
    self.execute(input).await
}
```

Add `ToolOutput::text(content: impl Into<String>)`. Add:

```rust
pub async fn execute_with_context(
    &self,
    context: &ToolContext<'_>,
    name: &str,
    input: serde_json::Value,
) -> Result<ToolOutput, ToolError>;
```

Change both `ToolExecutor::execute_tool_call` and `ToolExecutor::execute_with_hooks` to require `&ToolContext<'_>` and call the contextual registry method. Keep hook `session_id` derived from `context.agent.session_id`; remove the separate optional session argument from these two methods.

- [ ] **Step 4: Fix all compiler-reported executor call sites with trusted root contexts**

At daemon/TUI roots, call `AgentCoordinator::ensure_root(SessionId::new(session_id))`; nested loops will receive child contexts in Task 6. Do not construct contexts from model JSON. Run:

`cargo check --all-targets`

Expected: PASS with every executor call supplying a `ToolContext` and no identity-sensitive fallback to `ToolRegistry::execute`.

- [ ] **Step 5: Run focused tests**

Run: `cargo fmt && cargo test tools::external_tool_tests tools::executor::tests --lib`

Expected: PASS.

- [ ] **Step 6: Commit contextual tool execution**

```bash
git add src/tools/mod.rs src/tools/executor.rs src/daemon/handlers.rs src/daemon/state.rs src/tui/agent
git commit -m "feat(tools): propagate trusted agent context"
```

### Task 6: Propagate Context and Cancellation Through Every Subagent Loop

**Files:**
- Modify: `src/teams/subagent_loop.rs`
- Modify: `src/tools/meta/rlm/pipeline.rs`
- Modify: `src/tools/meta/run_script.rs`
- Modify: `src/tools/meta/task.rs`
- Modify: `tests/refactor_e2e_test.rs`

- [ ] **Step 1: Add a nested-tool context regression test**

In `tests/refactor_e2e_test.rs`, update the fake registry with a contextual probe and assert a nested tool invocation observes the child context, not the root context or forged JSON values.

Use this final loop signature in the test:

```rust
run_subagent_loop(
    &api_client,
    &tool_registry,
    &child_context,
    system_prompt,
    user_prompt,
    &allowed_tools,
    max_rounds,
    timeout_secs,
    on_progress,
    token_budget_k,
).await
```

- [ ] **Step 2: Run the regression test to verify it fails**

Run: `cargo test --test refactor_e2e_test`

Expected: FAIL because `run_subagent_loop` does not accept execution context.

- [ ] **Step 3: Change the loop signature and tool call path**

Add `context: &AgentExecutionContext` as the third parameter to `run_subagent_loop`. For each model tool call, construct:

```rust
let tool_context = ToolContext {
    agent: context,
    invocation_id: ToolInvocationId::new(tool_call.id.clone()),
};
```

Replace `tool_registry.execute(tool_name, args)` with `tool_registry.execute_with_context(&tool_context, tool_name, args)`. Wrap the loop future in `tokio::select!` so `context.cancellation.cancelled()` returns a `SubagentError` categorized as cancelled and emits `SubagentStatus::Cancelled`.

- [ ] **Step 4: Update every direct call site**

Update calls in:

- `src/tools/meta/rlm/pipeline.rs`
- `src/tools/meta/run_script.rs`
- `src/tools/meta/task.rs`
- `tests/refactor_e2e_test.rs`

Each child loop must receive a coordinator-created child context. Temporary root-only utilities may receive a daemon/runtime-created root context, never a context synthesized from tool arguments.

- [ ] **Step 5: Verify all loop callers and cancellation tests**

Run: `cargo fmt && cargo test --test refactor_e2e_test && cargo check --all-targets`

Expected: PASS and `rg -n "run_subagent_loop\(" src tests` shows every call with a context argument.

- [ ] **Step 6: Commit loop propagation**

```bash
git add src/teams/subagent_loop.rs src/tools/meta/rlm/pipeline.rs src/tools/meta/run_script.rs src/tools/meta/task.rs tests/refactor_e2e_test.rs
git commit -m "feat(agent): propagate context through nested loops"
```

### Task 7: Migrate TaskTool to Coordinator-Owned Children

**Files:**
- Modify: `src/tools/meta/task.rs`
- Modify: `src/tools/meta/task/tests.rs`
- Modify: `src/tools/meta/mod.rs`
- Modify: `src/daemon/state.rs`

- [ ] **Step 1: Write forged-field, parentage, and depth tests**

Add tests that invoke `TaskTool::execute_with_context` with trusted child context and input containing:

```json
{
  "description": "nested work",
  "prompt": "inspect module",
  "background": false,
  "_session_id": "forged-session",
  "_agent_id": "forged-agent",
  "_parent_id": "forged-parent",
  "_subagent_depth": 0
}
```

Assert the created record uses the trusted caller session, `parent_id == caller.agent_id`, and `depth == caller.depth + 1`. Add a max-depth test proving forged `_subagent_depth` cannot bypass `DepthLimitReached`.

- [ ] **Step 2: Run TaskTool tests to verify they fail**

Run: `cargo test tools::meta::task::tests --lib`

Expected: FAIL because TaskTool still reads reserved JSON fields and owns its own concurrency counter.

- [ ] **Step 3: Replace TaskTool ownership fields**

Change the constructor to receive:

```rust
pub fn new(
    settings: Settings,
    tool_registry: Weak<ToolRegistry>,
    coordinator: Arc<AgentCoordinator>,
    transcript_store: Option<Arc<TranscriptStore>>,
    mailbox: SubagentResultMailbox,
) -> Self
```

Remove `active_count`, `BackgroundManager` subagent result delivery, and direct hierarchy mutation from `TaskTool`. Keep the command-background subsystem separate.

- [ ] **Step 4: Implement contextual execution and coordinator spawning**

Keep `execute` only as a defensive error for direct identity-sensitive invocation:

```rust
async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
    Err(ToolError {
        message: "task requires trusted agent context".to_string(),
        code: Some("missing_agent_context".to_string()),
    })
}
```

In `execute_with_context`, ignore all keys beginning with `_`, derive depth from `context.agent.depth`, and call `coordinator.reserve_child`. Register the child future with the reservation before running `run_subagent_loop`. Use coordinator status updates for progress instead of creating `parent_id: None` records.

For `background: false`, await the child and return the bounded direct-child result. For `background: true`, return a scoped acknowledgement containing only a public result handle and status; the coordinator retains the child handle, and parent finalization joins it.

- [ ] **Step 5: Update tool schema and user-visible text**

Remove any reserved identity/depth properties from the schema. Change background wording to:

```text
The subagent is running concurrently inside this agent scope. This agent cannot terminate until the child reaches a terminal state.
```

Do not claim delivery after the parent returns.

- [ ] **Step 6: Run TaskTool and coordinator tests**

Run: `cargo fmt && cargo test tools::meta::task::tests agent::coordinator::tests --lib`

Expected: PASS and `rg -n 'active_count|_subagent_depth|input\["_session_id"\]' src/tools/meta/task.rs` returns no matches.

- [ ] **Step 7: Commit TaskTool migration**

```bash
git add src/tools/meta/task.rs src/tools/meta/task/tests.rs src/tools/meta/mod.rs src/daemon/state.rs
git commit -m "refactor(tools): route task children through coordinator"
```

### Task 8: Make Background Children Scoped and Sanitize Upward Results

**Files:**
- Modify: `src/agent/coordinator.rs`
- Modify: `src/tools/meta/task.rs`
- Modify: `src/teams/subagent_mailbox.rs`
- Modify: `src/tools/meta/task/tests.rs`

- [ ] **Step 1: Write scoped-background and result-sanitization tests**

Test that a parent candidate result moves to `WaitingForChildren` while a background child is live, then reaches `Completed` only after the child completes. Add a child result containing strings shaped like descendant IDs and a serialized tree payload; assert `sanitize_child_result` returns bounded `summary`/`partial_result` and no fields for descendants, transcript, messages, events, or parent ID.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test scoped_background child_result --lib`

Expected: FAIL because scoped acknowledgement/finalization and sanitizer contracts are incomplete.

- [ ] **Step 3: Implement scoped result handles and sanitization**

Define an opaque handle whose serialized form contains only a random token:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChildResultHandle(String);

struct ChildResultGrant {
    session_id: SessionId,
    parent_id: AgentId,
    child_id: AgentId,
    generation: u64,
}
```

Store the binding internally and return the handle only to the direct parent. Implement:

```rust
pub fn sanitize_child_result(
    child_id: AgentId,
    terminal: ChildTerminalStatus,
    summary: &str,
    error_code: Option<&str>,
    partial_result: Option<&str>,
) -> ChildResult;
```

Limit summary and partial-result sizes using the existing mailbox thresholds. Persist large payloads under coordinator-owned metadata; do not expose filesystem paths as ambient authority. Retrieval must require the same parent context and handle binding.

- [ ] **Step 4: Verify background scope and mailbox regressions**

Run: `cargo fmt && cargo test scoped_background child_result subagent_mailbox --lib`

Expected: PASS, and no subagent path calls `BackgroundManager::push_subagent_result`.

- [ ] **Step 5: Commit scoped background semantics**

```bash
git add src/agent/coordinator.rs src/tools/meta/task.rs src/teams/subagent_mailbox.rs src/tools/meta/task/tests.rs
git commit -m "feat(agent): scope asynchronous child execution"
```

### Task 9: Unify RLM and Run-Script Child Semantics

**Files:**
- Modify: `src/tools/meta/rlm/mod.rs`
- Modify: `src/tools/meta/rlm/pipeline.rs`
- Modify: `src/tools/meta/run_script.rs`
- Modify: `src/tools/meta/task.rs`

- [ ] **Step 1: Write RLM hierarchy parity tests**

Add tests that run an RLM plan with two parallel subtasks and assert both records are direct children of the RLM caller, use caller depth plus one, consume coordinator permits, and are joined before aggregation completes. Add a run-script test asserting its nested loop receives the caller context rather than a synthetic depth zero context.

- [ ] **Step 2: Run focused tests to verify they fail**

Run: `cargo test tools::meta::rlm tools::meta::run_script --lib`

Expected: FAIL because RLM still creates UUID progress nodes and raw Tokio tasks outside coordinator ownership.

- [ ] **Step 3: Change RLM pipeline inputs and executor phase**

Use this signature:

```rust
pub async fn run_rlm_pipeline(
    settings: &Settings,
    tool_registry: Arc<ToolRegistry>,
    coordinator: Arc<AgentCoordinator>,
    caller: &AgentExecutionContext,
    task: &str,
    context: &str,
    token_budget_k: Option<u64>,
) -> Result<RlmResult, String>
```

For every planned subtask, reserve a coordinator child and register the loop handle. Replace direct `tokio::spawn` ownership and direct progress-map writes. Keep dependency-level scheduling, but join through coordinator handles before aggregation. Hide `task` when the child context reaches max depth; coordinator remains the enforcement boundary.

- [ ] **Step 4: Migrate delegate and run-script contextual paths**

Make identity-sensitive delegate execution reject missing `ToolContext`, pass `context.agent` into `run_rlm_pipeline`, and pass the trusted context through run-script nested tool execution. Remove any `_session_id` or `_subagent_depth` compatibility reads in these files.

- [ ] **Step 5: Verify semantic parity**

Run: `cargo fmt && cargo test tools::meta::rlm tools::meta::run_script --lib && cargo check --all-targets`

Expected: PASS and `rg -n 'parent_id:|Uuid::new_v4|tokio::spawn' src/tools/meta/rlm/pipeline.rs` has no hierarchy-ownership matches.

- [ ] **Step 6: Commit RLM migration**

```bash
git add src/tools/meta/rlm/mod.rs src/tools/meta/rlm/pipeline.rs src/tools/meta/run_script.rs src/tools/meta/task.rs
git commit -m "refactor(agent): unify recursive child coordination"
```

### Task 10: Scope Transcript and Result Access

**Files:**
- Modify: `src/tools/meta/subagent_trace.rs`
- Modify: `src/transcript/mod.rs`
- Modify: `src/agent/store.rs`
- Modify: `src/agent/coordinator.rs`
- Modify: `src/teams/subagent_mailbox.rs`

- [ ] **Step 1: Write shared authorization tests**

For self, direct child, parent, sibling, grandchild, other branch, cross-session, and missing targets, run status, transcript, result, and cancellation lookups. Assert self/direct child succeed and every denied targeted operation returns the same external code `not_visible` and message.

- [ ] **Step 2: Run authorization tests to verify they fail**

Run: `cargo test transcript_authorization result_authorization cancellation_authorization --lib`

Expected: FAIL because transcript/mailbox APIs accept raw identifiers or paths.

- [ ] **Step 3: Add authorized repository methods**

Implement coordinator methods whose first argument is always the trusted caller:

```rust
pub async fn read_transcript(&self, caller: &AgentExecutionContext, target: AgentId) -> Result<SubagentTranscript, CoordinatorError>;
pub async fn read_result(&self, caller: &AgentExecutionContext, handle: &ChildResultHandle) -> Result<ChildResult, CoordinatorError>;
pub async fn read_status(&self, caller: &AgentExecutionContext, target: AgentId) -> Result<DirectChildView, CoordinatorError>;
```

Perform authorization before storage retrieval. Map absent handles, wrong parent, wrong session, stale generation, and hidden targets to `CoordinatorError::NotVisible`. Keep detailed internal tracing fields out of returned tool content.

- [ ] **Step 4: Migrate SubagentTraceTool**

Override `execute_with_context`, reject direct `execute`, and remove globally listable transcript behavior. Its schema may accept a direct-child opaque handle or target ID, but authorization must use `ToolContext`; raw IDs confer no authority.

- [ ] **Step 5: Verify all scoped operations**

Run: `cargo fmt && cargo test transcript_authorization result_authorization cancellation_authorization --lib`

Expected: PASS with indistinguishable hidden/missing errors.

- [ ] **Step 6: Commit scoped information access**

```bash
git add src/tools/meta/subagent_trace.rs src/transcript/mod.rs src/agent/store.rs src/agent/coordinator.rs src/teams/subagent_mailbox.rs
git commit -m "feat(agent): authorize subagent results and transcripts"
```

### Task 11: Add Viewer-Bound Navigation Capabilities

**Files:**
- Create: `src/agent/capability.rs`
- Modify: `src/agent/mod.rs`
- Modify: `src/agent/store.rs`

- [ ] **Step 1: Write capability binding tests**

Create a deterministic test clock and assert verification rejects each mismatch independently: wrong session, target, generation, operation, viewer, expiration, and unknown token. In the test module, define `test_secret() -> [u8; 32]` as `[7; 32]` and `fixed_clock()` as an `Arc<TestClock>` initialized to `2026-07-11T00:00:00Z`. Assert capability debug/log formatting does not expose the bearer token.

```rust
#[test]
fn capability_is_bound_to_all_authority_dimensions() {
    let service = CapabilityService::with_clock(test_secret(), fixed_clock());
    let token = service.issue(&CapabilityGrant::navigate("viewer-a", "s", "child", 7));
    assert!(service.verify(&token, &CapabilityRequest::navigate("viewer-a", "s", "child", 7)).is_ok());
    assert_eq!(service.verify(&token, &CapabilityRequest::navigate("viewer-b", "s", "child", 7)), Err(CapabilityError::NotVisible));
    assert_eq!(service.verify(&token, &CapabilityRequest::transcript("viewer-a", "s", "child", 7)), Err(CapabilityError::NotVisible));
}
```

- [ ] **Step 2: Run capability tests to verify they fail**

Run: `cargo test agent::capability::tests --lib`

Expected: FAIL because the capability service does not exist.

- [ ] **Step 3: Implement opaque in-memory grants**

Define `CapabilityOperation::{Navigate, Transcript, Cancel}`, `ViewerId`, `NavigationCapability`, `CapabilityGrant`, and `CapabilityRequest`. Add a private `Clock` trait with `fn now(&self) -> DateTime<Utc>`, a production `SystemClock`, and a test `TestClock`. Generate 256-bit random bearer tokens with `rand::rngs::OsRng`, store only `Hmac<Sha256>(secret, token)` as the lookup key, bind all dimensions from the approved design, and expire grants after a configurable duration. Verification must return `CapabilityError::NotVisible` for every mismatch and remove expired entries.

Do not implement capabilities as serialized agent IDs. Do not derive authority solely from the daemon bearer token.

- [ ] **Step 4: Verify capability secrecy and constant lookup behavior**

Run: `cargo fmt && cargo test agent::capability::tests --lib`

Expected: PASS; serialized local projections may contain opaque capability strings, while `Debug` output for the service and grant omits raw tokens.

- [ ] **Step 5: Commit capability service**

```bash
git add src/agent/capability.rs src/agent/mod.rs src/agent/store.rs
git commit -m "feat(agent): add scoped navigation capabilities"
```

### Task 12: Replace Full-Session Daemon Access with Scoped Agent APIs

**Files:**
- Modify: `src/daemon/state.rs`
- Modify: `src/daemon/models.rs`
- Modify: `src/daemon/handlers.rs`
- Modify: `src/daemon/routes.rs`
- Create: `tests/strict_subagent_isolation.rs`

- [ ] **Step 1: Write daemon API contract tests**

Build an Axum router with a seeded three-level tree. Test:

```text
GET  /api/v1/agents/self?session_id=s
GET  /api/v1/agents/children?session_id=s
GET  /api/v1/agents/children/{capability}?session_id=s
GET  /api/v1/agents/children/{capability}/transcript?session_id=s
POST /api/v1/agents/children/{capability}/cancel?session_id=s
POST /api/v1/ui/viewers
```

Assert the root response contains root plus direct children only; navigating with a child's capability returns that child plus its direct children only. Compare status and response body for expired, wrong-viewer, hidden, and random capabilities; all must be indistinguishable.

- [ ] **Step 2: Run integration tests to verify they fail**

Run: `cargo test --test strict_subagent_isolation daemon_`

Expected: FAIL because scoped routes and response models do not exist.

- [ ] **Step 3: Add daemon models and trusted state ownership**

Add serializable `LocalAgentViewResponse`, `SelfAgentResponse`, and `DirectChildResponse { agent_id, status, summary, navigation_capability }`. Add `CreateViewerResponse { viewer_token: String }`; `POST /api/v1/ui/viewers` creates 256 random bits, stores only `Hmac<Sha256>(daemon_viewer_secret, token)` in daemon memory, and returns the bearer token once. `DaemonState` must own `Arc<AgentCoordinator>` and `Arc<CapabilityService>`, a viewer-token digest map, plus a root-context map keyed by the authenticated/current session.

Add:

```rust
pub async fn root_context(&self, session_id: &str) -> Result<AgentExecutionContext, anyhow::Error>;
```

This method calls `ensure_root`; it never accepts agent ID, parent ID, or depth from request JSON.

- [ ] **Step 4: Implement scoped handlers and routes**

The TUI sends its daemon-issued viewer token in `X-Wgenty-Viewer-Token`. Handlers hash and resolve that token to a trusted `ViewerId`, verify capabilities before target lookup, and issue fresh direct-child capabilities for each returned local view. Missing or unknown viewer tokens receive one stable unauthorized response. Add `.context("...")` at daemon boundaries and map capability/visibility failures to one stable 404 response without target details.

Disable request-URI logging for the scoped capability routes or install a redaction layer that records `/api/v1/agents/children/{capability}` literally. Add a handler test with a captured tracing subscriber and assert the raw capability is absent from logs.

Remove `_session_id` injection from both permission branches in `execute_tool`; build `ToolContext` from `DaemonState::root_context` and a fresh invocation ID.

- [ ] **Step 5: Restrict and then remove the full progress endpoint**

First ensure no normal client or model tool calls `/api/v1/subagent/progress`. Remove the route and `get_subagent_progress` handler in the same commit. If an operator diagnostic is still required by an existing test, expose it under an explicitly operator-only state method with no HTTP route.

- [ ] **Step 6: Verify daemon isolation contracts**

Run: `cargo fmt && cargo test --test strict_subagent_isolation daemon_ && cargo test daemon --lib`

Expected: PASS and `rg -n '/api/v1/subagent/progress|_session_id.*insert' src/daemon src/tui` returns no matches.

- [ ] **Step 7: Commit scoped daemon APIs**

```bash
git add src/daemon/state.rs src/daemon/models.rs src/daemon/handlers.rs src/daemon/routes.rs tests/strict_subagent_isolation.rs
git commit -m "feat(api): expose capability-scoped agent views"
```

### Task 13: Convert TUI State and Polling to a Single Local View

**Files:**
- Modify: `src/tui/client.rs`
- Modify: `src/tui/agent/mod.rs`
- Modify: `src/tui/agent/core.rs`
- Modify: `src/tui/agent/tool_dispatch.rs`
- Modify: `src/tui/app/types.rs`
- Modify: `src/tui/app/mod.rs`
- Modify: `src/tui/app/event.rs`
- Modify: `src/tui/components/subagent_tree.rs`
- Modify: `src/tui/components/subagent_status_bar.rs`

- [ ] **Step 1: Write local-view state tests**

Add tests proving that replacing the current view removes nodes from the previous layer, counts only the current self and direct children, and cannot select an ID absent from the current response.

```rust
#[test]
fn replacing_local_view_drops_previous_layer_nodes() {
    let mut tree = SubagentTree::default();
    tree.replace_local(root_view());
    assert_eq!(tree.selectable_ids(), vec![AgentId::new("root"), AgentId::new("child")]);
    tree.replace_local(child_view());
    assert_eq!(tree.selectable_ids(), vec![AgentId::new("child"), AgentId::new("grandchild")]);
    assert!(!tree.contains(&AgentId::new("root")));
}
```

- [ ] **Step 2: Run TUI component tests to verify they fail**

Run: `cargo test tui::components::subagent_tree --lib`

Expected: FAIL because `replace_local` and scoped selection do not exist.

- [ ] **Step 3: Replace progress-map client methods**

Remove `poll_subagent_progress`. Add:

```rust
pub async fn get_root_agent_view(&self, session_id: &str) -> anyhow::Result<LocalAgentViewResponse>;
pub async fn navigate_agent_view(&self, session_id: &str, capability: &str) -> anyhow::Result<LocalAgentViewResponse>;
pub async fn get_child_transcript(&self, session_id: &str, capability: &str) -> anyhow::Result<SubagentTranscript>;
pub async fn cancel_child(&self, session_id: &str, capability: &str) -> anyhow::Result<()>;
```

Add `DaemonClient::create_viewer()` and store its returned token in a private `DaemonClient` field for `X-Wgenty-Viewer-Token`; create it once during TUI startup and refresh it only after daemon restart/unauthorized response. Do not write viewer tokens or capability values to tracing output, chat messages, transcripts, or tool arguments.

- [ ] **Step 4: Change app events and tree storage**

Replace `AppEvent::SubagentUpdate(Box<SubagentProgress>)` with:

```rust
AppEvent::AgentLocalView(Box<LocalAgentViewResponse>)
```

Make `SubagentTree` store exactly one `SelfAgentResponse` and a vector of direct children. Remove session-wide upsert semantics from the agent view path. `node_list` and `real_node_list` may render the local input but must not discover descendants.

- [ ] **Step 5: Update polling and status rendering**

Both existing pollers fetch the current root/local endpoint and emit `AgentLocalView`. Status counts, completion summaries, and active indicators must be computed only from the current response. Preserve current rendering labels and status colors where possible.

- [ ] **Step 6: Verify local-only TUI state**

Run: `cargo fmt && cargo test tui::components::subagent_tree --lib && cargo check --all-targets`

Expected: PASS and `rg -n 'poll_subagent_progress|SubagentUpdate' src/tui` returns no matches.

- [ ] **Step 7: Commit TUI local state migration**

```bash
git add src/tui/client.rs src/tui/agent src/tui/app/types.rs src/tui/app/mod.rs src/tui/app/event.rs src/tui/components/subagent_tree.rs src/tui/components/subagent_status_bar.rs
git commit -m "refactor(tui): render scoped agent local views"
```

### Task 14: Add Layer-by-Layer TUI Navigation and Back History

**Files:**
- Modify: `src/tui/app/types.rs`
- Modify: `src/tui/app/mod.rs`
- Modify: `src/tui/app/event.rs`
- Modify: `src/tui/app/event_key.rs`
- Modify: `src/tui/components/subagent_focus_view.rs`
- Modify: `src/tui/components/subagent_tree.rs`
- Modify: `tests/strict_subagent_isolation.rs`

- [ ] **Step 1: Write navigation-history tests**

Test root-to-child-to-grandchild navigation using server-issued capabilities. Assert each loaded view contains exactly selected self plus direct children, back restores the previous cached trusted UI view, and no ancestor ID is added to the selected agent's `messages` or transcript.

- [ ] **Step 2: Run navigation tests to verify they fail**

Run: `cargo test --test strict_subagent_isolation tui_`

Expected: FAIL because focus selection still traverses a complete tree.

- [ ] **Step 3: Implement UI-owned navigation history**

Add:

```rust
#[derive(Clone)]
pub struct AgentViewFrame {
    pub view: LocalAgentViewResponse,
    pub selected: usize,
    pub breadcrumb_label: String,
}

pub struct AgentNavigationState {
    pub current: AgentViewFrame,
    pub back_stack: Vec<AgentViewFrame>,
}
```

The capability remains inside `DirectChildResponse` in UI memory. Opening a child calls `navigate_agent_view`, pushes the current frame, and replaces it with the response. Back pops a frame locally. Breadcrumb labels are display-only and must never be appended to model messages.

- [ ] **Step 4: Restrict focus selection to current direct children**

Replace `visible_node_ids()` and selector-position calculations based on `real_node_list()` with the current local view's stable list. A focus action on self opens its transcript locally; a focus action on a direct child uses its capability. No raw grandchild ID can be selected before navigating into its parent view.

- [ ] **Step 5: Verify navigation and information flow**

Run: `cargo fmt && cargo test --test strict_subagent_isolation tui_ && cargo test tui::components --lib`

Expected: PASS, including assertions that focused child model messages contain no ancestor, sibling, or other-branch identifiers.

- [ ] **Step 6: Commit capability navigation**

```bash
git add src/tui/app/types.rs src/tui/app/mod.rs src/tui/app/event.rs src/tui/app/event_key.rs src/tui/components/subagent_focus_view.rs src/tui/components/subagent_tree.rs tests/strict_subagent_isolation.rs
git commit -m "feat(tui): navigate agent hierarchy by capability"
```

### Task 15: Add Restart Recovery and Forced-Abort Cleanup

**Files:**
- Modify: `src/agent/coordinator.rs`
- Modify: `src/agent/store.rs`
- Modify: `src/daemon/state.rs`
- Modify: `tests/strict_subagent_isolation.rs`

- [ ] **Step 1: Write recovery and uncooperative-child tests**

Seed records in `Pending`, `Running`, `WaitingForChildren`, `Finalizing`, and `Cancelling`, simulate a daemon restart with no task handles, and assert the complete affected subtrees become `Cancelled` with internal reason `runtime_restarted`. Add a child future that ignores cancellation; after the configured shutdown timeout, assert it is aborted, terminal cleanup completes, and its permit is released.

- [ ] **Step 2: Run recovery tests to verify they fail**

Run: `cargo test --test strict_subagent_isolation recovery_`

Expected: FAIL because restart recovery and forced abort are not wired.

- [ ] **Step 3: Implement recovery queries and coordinator startup cleanup**

Add a store query by non-terminal status and a coordinator method:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub cancelled_scopes: usize,
    pub cancelled_subtrees: usize,
}

pub async fn recover_non_terminal_scopes(&self) -> Result<RecoveryReport, CoordinatorError>;
```

Group records into roots of affected subtrees, mark each subtree `Cancelling`, then persist descendants bottom-up as `Cancelled`. Do not reconstruct task handles or resume model loops. Return counts only in `RecoveryReport`; do not expose hidden IDs through daemon responses.

- [ ] **Step 4: Wire recovery into DaemonState initialization**

Run recovery after the canonical store is loaded and before routes accept requests. Add `.context("recovering non-terminal agent scopes after daemon restart")` at this application boundary.

- [ ] **Step 5: Verify recovery and permit cleanup**

Run: `cargo fmt && cargo test --test strict_subagent_isolation recovery_ && cargo test agent::coordinator::tests --lib`

Expected: PASS; no recovered record remains non-terminal and all permits are available.

- [ ] **Step 6: Commit recovery support**

```bash
git add src/agent/coordinator.rs src/agent/store.rs src/daemon/state.rs tests/strict_subagent_isolation.rs
git commit -m "feat(agent): recover interrupted subagent scopes"
```

### Task 16: Remove Flat and Reserved-Field Compatibility Paths

**Files:**
- Modify: `src/teams/subagent.rs`
- Modify: `src/teams/mod.rs`
- Modify: `src/services/mod.rs`
- Modify: `src/tools/meta/task.rs`
- Modify: `src/tools/meta/rlm/mod.rs`
- Modify: `src/daemon/handlers.rs`

- [ ] **Step 1: Add a source-level compatibility guard test**

Add an integration assertion that scans identity-sensitive source files and fails if they contain model-input reads for `_session_id`, `_agent_id`, `_parent_id`, or `_subagent_depth`, or expose a global agent listing method.

- [ ] **Step 2: Run the guard to verify it fails**

Run: `cargo test --test strict_subagent_isolation compatibility_`

Expected: FAIL while old compatibility code or flat listing remains.

- [ ] **Step 3: Retire the flat AgentsService hierarchy path**

Remove or reduce `AgentSession`/`AgentsService` so no API can globally list subagents. Migrate any still-needed service construction to `AgentCoordinator`. Preserve unrelated chat-session functionality; this change concerns agent hierarchy only.

- [ ] **Step 4: Remove reserved identity handling**

Delete reads, writes, and schema mentions for all four reserved identity fields in task, delegate, daemon, and nested loop paths. Unknown underscore-prefixed fields may remain ordinary ignored JSON, but they must never influence identity, authorization, depth, or cancellation.

- [ ] **Step 5: Verify compatibility cleanup**

Run:

```bash
rg -n '_session_id|_agent_id|_parent_id|_subagent_depth' src/tools/meta src/daemon src/teams
cargo test --test strict_subagent_isolation compatibility_
cargo check --all-targets
```

Expected: the search returns no identity-boundary matches, and both Cargo commands PASS.

- [ ] **Step 6: Commit compatibility cleanup**

```bash
git add src/teams/subagent.rs src/teams/mod.rs src/services/mod.rs src/tools/meta/task.rs src/tools/meta/rlm/mod.rs src/daemon/handlers.rs tests/strict_subagent_isolation.rs
git commit -m "refactor(agent): remove flat hierarchy compatibility"
```

### Task 17: Update Documentation and Architecture Claims

**Files:**
- Modify: `README.md`
- Modify: `WGENTY.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Locate stale claims**

Run:

```bash
rg -n '_subagent_depth|background subagent|background.*continues|full tree|subagent progress|parent_id' README.md WGENTY.md
```

Expected: output identifies descriptions of caller-provided depth, detached background children, or whole-tree visibility.

- [ ] **Step 2: Document the implemented contracts**

State explicitly:

```text
- Agent identity, parentage, and depth are derived from AgentExecutionContext.
- Every agent, including root, sees only itself and direct children.
- background=true permits concurrency inside the parent scope; it does not detach work.
- A parent cannot become terminal while a scoped child is live.
- TUI hierarchy navigation is one level at a time through short-lived capabilities.
- Truly detached work requires a separate BackgroundJob subsystem and is not provided by task/delegate.
```

Add a Conventional Changelog entry under the current unreleased section describing the security boundary and the behavior change.

- [ ] **Step 3: Verify documentation consistency**

Run the search from Step 1 again.

Expected: remaining matches describe only trusted runtime depth, scoped background semantics, or historical documents explicitly labeled as historical.

- [ ] **Step 4: Commit documentation**

```bash
git add README.md WGENTY.md CHANGELOG.md
git commit -m "docs(agent): describe strict subagent isolation"
```

### Task 18: Run Full Verification and Performance Checks

**Files:**
- Verify: all files changed by Tasks 1-17

- [ ] **Step 1: Run formatting check**

Run: `cargo fmt -- --check`

Expected: PASS with no diff.

- [ ] **Step 2: Run Clippy with warnings denied**

Run: `cargo clippy --all-targets -- -D warnings`

Expected: PASS with zero warnings.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --all`

Expected: PASS, including strict visibility, structured concurrency, daemon capability, TUI navigation, transcript/result, recovery, and compatibility tests.

- [ ] **Step 4: Run acceptance searches**

```bash
rg -n '/api/v1/subagent/progress|poll_subagent_progress|SubagentUpdate' src
rg -n 'input\["_(session_id|agent_id|parent_id|subagent_depth)"\]' src
rg -n 'AtomicUsize|POLL_INTERVAL_MS|push_subagent_result' src/tools/meta/task.rs
```

Expected: no matches.

- [ ] **Step 5: Build release and check repository performance constraints**

Run:

```bash
cargo build --release
time ./target/release/wgenty_code --version
ls -lh ./target/release/wgenty_code
```

Expected: release build succeeds; startup regression is at most 5%, base memory regression is at most 2%, and binary growth is at most 500KB compared with the pre-change baseline recorded before execution begins.

- [ ] **Step 6: Inspect the final diff for information leaks**

Run: `git diff develop...HEAD -- src/agent src/tools/meta src/daemon src/tui src/transcript tests`

Expected: no model-visible response, transcript, log message, or local projection contains capabilities, ancestor metadata, sibling/grandchild records, or full-session maps.

- [ ] **Step 7: Record final verification**

Add the exact command results and measured release deltas to the eventual PR description. Do not create a separate verification-only commit unless verification required source changes; if it did, commit those fixes with the narrowest applicable Conventional Commit type.
