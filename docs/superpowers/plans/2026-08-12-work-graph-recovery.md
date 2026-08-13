# Work-Graph Restart Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resume a checkpointed ExecutionSession after process restart and safely re-dispatch a lost RootCause specialist without re-running anchors.

**Architecture:** Add `SessionCoordinator::open_or_create` to load an existing session and active-turn WorkState without overwriting its snapshot. Extend `ExecutionSessionRuntimeStore` with a recovered RootCause reservation, then let the daemon's existing trusted dispatcher launch a fresh diagnostic child through the same pre-spawn binding path as a new failure.

**Tech Stack:** Rust, `anyhow`, `serde`, Tokio, existing CheckpointStore, AgentCoordinator, ToolRegistry, cargo test/clippy/fmt.

## Global Constraints

- Never overwrite an existing `session.json` while attempting recovery.
- A recovered RootCause child id is always new; persisted child identities are never reused.
- Recovery routes only from persisted typed State and graph audit, never model text.
- Recovery must not execute compile/test/verify anchors or increment their attempt.
- Corrupt session state fails closed with contextual errors and preserves files.
- RootCause remains read-only except its authenticated `submit_specialist_report` sink.
- Preserve cross-platform Rust behavior and pass `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`, and `git diff --check`.

---

### Task 1: Open an existing SessionCoordinator safely

**Files:**
- Modify: `src/exec_session/coordinator.rs:34-62`
- Test: `src/exec_session/coordinator.rs` test module

**Interfaces:**
- Consumes: `SessionState::load`, `CheckpointStore::restore_work_state`.
- Produces: `SessionCoordinator::open_or_create(session_id, source, project_root, checkpoint_store) -> anyhow::Result<Self>`.

- [ ] **Step 1: Write the failing recovery tests**

```rust
#[test]
fn open_or_create_restores_existing_session_and_active_work_state() {
    let dir = tempdir().unwrap();
    let store = Arc::new(CheckpointStore::new(dir.path()));
    let mut original = SessionCoordinator::new(
        "resume".into(), SessionSource::AgentSelf, dir.path(), store.clone(),
    ).unwrap();
    original.begin_turn().unwrap();
    original.work_state_mut().set_requirement(Some("keep me".into()));
    original.capture_current_work_state().unwrap();
    let restored = SessionCoordinator::open_or_create(
        "resume".into(), SessionSource::AgentSelf, dir.path(), store,
    ).unwrap();
    assert_eq!(restored.current_turn_id(), Some("turn-0"));
    assert_eq!(restored.work_state().requirement(), Some("keep me"));
}

#[test]
fn open_or_create_corrupt_session_does_not_replace_snapshot() {
    // Write malformed session.json, call open_or_create, assert error and exact bytes unchanged.
}
```

- [ ] **Step 2: Run the new tests and verify RED**

Run: `cargo test exec_session::coordinator::tests::open_or_create --lib -- --nocapture`

Expected: compilation failure because `SessionCoordinator::open_or_create` does not exist.

- [ ] **Step 3: Implement open-or-create loading**

```rust
pub fn open_or_create(/* same arguments as new */) -> Result<Self> {
    let session_dir = project_root.join(".wgenty-code").join("snapshots").join(&session_id);
    if session_dir.join("session.json").exists() {
        let session = SessionState::load(&session_dir)
            .with_context(|| format!("load existing session: {}", session_dir.display()))?;
        if session.session_id != session_id { anyhow::bail!("..."); }
        let mut coordinator = Self { session, session_dir, checkpoint_store, project_root: project_root.to_path_buf(), work_state: WorkState::default() };
        if let Some(turn_id) = coordinator.current_turn_id().map(str::to_owned) {
            coordinator.restore_work_state_for_turn(&turn_id)?;
        }
        return Ok(coordinator);
    }
    Self::new(session_id, source, project_root, checkpoint_store)
}
```

Validate that an existing state with `current_turn = Some(...)` references a real turn before attempting WorkState restore; return a contextual error otherwise.

- [ ] **Step 4: Run the focused recovery tests and existing coordinator suite**

Run: `cargo test exec_session::coordinator::tests --lib -- --nocapture`

Expected: all coordinator tests pass, including both new recovery cases.

- [ ] **Step 5: Commit**

```bash
git add src/exec_session/coordinator.rs
git commit -m "feat(session): restore checkpointed coordinators"
```

### Task 2: Restore runtime entries and derive recovered RootCause work

**Files:**
- Modify: `src/exec_session/runtime_store.rs:73-434`
- Test: `src/exec_session/runtime_store.rs` test module

**Interfaces:**
- Consumes: `SessionCoordinator::open_or_create`, `next_step`, latest RootCause audit route.
- Produces: `ExecutionSessionRuntimeStore::prepare_recovered_root_cause_dispatch(&SessionId) -> Result<Option<RootCauseDispatchRequest>>`.

- [ ] **Step 1: Write failing store-recreation tests**

```rust
#[tokio::test]
async fn recreated_store_restores_root_cause_route_without_old_child() {
    let dir = TempDir::new().unwrap();
    let session = SessionId::new("recover-route");
    let first = test_store(&dir);
    first.ensure_turn(&session).unwrap();
    first.runtime_for(&session).unwrap().begin_node("diagnose".into(), vec![], vec![]).await.unwrap();
    first.seed_root_cause_route_for_test(&session);
    drop(first);

    let recovered = test_store(&dir);
    let request = recovered.prepare_recovered_root_cause_dispatch(&session).unwrap().unwrap();
    assert!(request.prompt.contains("seeded compile failure"));
    assert!(recovered.root_cause_pending(&session).unwrap());
    assert!(matches!(recovered.prepare_root_cause_dispatch(&session), Err(_)));
}
```

Also assert restored current node id, retry budget, and RootCause audit attempt match the first store; assert no `GraphAuditAnchor` event is added.

- [ ] **Step 2: Run the new test and verify RED**

Run: `cargo test exec_session::runtime_store::tests::recreated_store_restores_root_cause_route_without_old_child --lib -- --nocapture`

Expected: FAIL because entry creation calls `SessionCoordinator::new` and overwrites the snapshot.

- [ ] **Step 3: Implement recovery-aware entry creation and reservation**

```rust
fn entry_for(&self, session_id: &SessionId) -> Result<RuntimeEntry> {
    // Replace SessionCoordinator::new with SessionCoordinator::open_or_create.
}

pub fn prepare_recovered_root_cause_dispatch(
    &self, session_id: &SessionId,
) -> Result<Option<RootCauseDispatchRequest>> {
    // If an in-memory reservation exists, return None.
    // Read restored State; return None unless next_step is RootCause.
    // Locate the persisted latest RootCause route for current node, assemble the
    // existing anchored prompt, and store PendingRootCause { child_id: None }.
}
```

Factor prompt construction into one private helper used by both new and recovered reservations. Never deserialize or retain a previous child id.

- [ ] **Step 4: Run focused runtime-store tests**

Run: `cargo test exec_session::runtime_store::tests --lib -- --nocapture`

Expected: recreation, unchanged-session, terminal, and ordinary per-session isolation tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/exec_session/runtime_store.rs src/exec_session/coordinator.rs
git commit -m "feat(graph): recover root-cause routes"
```

### Task 3: Dispatch recovered RootCause through the trusted daemon route

**Files:**
- Modify: `src/exec_session/node_tools.rs:202-356`
- Modify: `src/tools/mod.rs:226-487`
- Modify: `src/daemon/state.rs:305-321`
- Test: `src/tools/mod.rs` test module

**Interfaces:**
- Consumes: `ExecutionSessionRuntimeStore::prepare_recovered_root_cause_dispatch`, `RootCauseDispatcher::dispatch`.
- Produces: one shared private helper that dispatches both new and recovered RootCause reservations and writes `root_cause_child_id` metadata.

- [ ] **Step 1: Write a failing dispatcher recovery test**

```rust
#[tokio::test]
async fn recovered_root_cause_dispatch_uses_new_bound_child_without_running_anchors() {
    // Persist a RootCause route in a first store, drop it, recreate the store.
    // Use a task probe that records the contextual call and returns a new id
    // with root_cause_route_bound=true.
    // Call the code-owned recovery dispatcher and assert the new id is used,
    // the route stays at the original attempt, and no anchor audit count changes.
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test tools::external_tool_tests::recovered_root_cause_dispatch --lib -- --nocapture`

Expected: FAIL because there is no recovered dispatch entry.

- [ ] **Step 3: Implement the code-owned recovery dispatcher entry**

```rust
async fn dispatch_root_cause(
    store: &ExecutionSessionRuntimeStore,
    dispatcher: &dyn RootCauseDispatcher,
    context: &ToolContext<'_>,
    request: RootCauseDispatchRequest,
    output: &mut ToolOutput,
) -> Result<(), ToolError> { /* existing pre-spawn binding acknowledgement path */ }
```

Invoke this helper from `verify_node` for new failures and from the daemon's
trusted session attachment/recovery path for restored `RootCause` State. The
recovery call obtains its request only through `prepare_recovered_root_cause_dispatch`; it must not invoke `verify_current_node`.

- [ ] **Step 4: Run focused dispatcher and specialist tests**

Run: `cargo test root_cause --lib -- --nocapture`

Expected: existing new-route dispatch and report tests remain green; the new recovered-dispatch test proves pre-spawn binding and unchanged attempt.

- [ ] **Step 5: Commit**

```bash
git add src/exec_session/node_tools.rs src/tools/mod.rs src/daemon/state.rs
git commit -m "feat(daemon): redispatch recovered diagnostics"
```

### Task 4: Document the recovery lifecycle and run the full verification gate

**Files:**
- Modify: `WGENTY.md:120-145`
- Modify: `docs/superpowers/specs/2026-08-12-specialist-handoff-design.md:80-108`
- Test: all test targets

**Interfaces:**
- Consumes: completed persisted-session and recovered-dispatch APIs.
- Produces: operator-facing explanation that restarts preserve State, discard old child identity, and safely re-dispatch only the static RootCause edge.

- [ ] **Step 1: Update docs with observable restart behavior**

Document the persisted session location, recovery conditions, no-anchor-rerun invariant, terminal behavior, and fresh-child binding requirement. Do not claim dynamic graph selection is implemented.

- [ ] **Step 2: Run format and lint**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`

Expected: zero formatting diffs and zero warnings.

- [ ] **Step 3: Run all tests and diff check**

Run: `cargo test --all && git diff --check`

Expected: every library/integration test passes and the worktree diff is whitespace-clean.

- [ ] **Step 4: Commit**

```bash
git add WGENTY.md docs/superpowers/specs/2026-08-12-specialist-handoff-design.md
git commit -m "docs(graph): document restart recovery"
```

## Plan Self-Review

- Spec coverage: Task 1 preserves sessions and WorkState; Task 2 restores typed route state; Task 3 re-dispatches only the lost RootCause child without anchors; Task 4 documents and verifies the behavior.
- Placeholder scan: no unresolved implementation references or deferred behavior remain in task steps.
- Type consistency: `open_or_create` is consumed only by `entry_for`; recovery produces the existing `RootCauseDispatchRequest`; dispatch continues through the existing `RootCauseDispatcher` and TaskTool pre-spawn binding acknowledgement.
