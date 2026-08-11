# Work-Graph Audit and Evaluation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist typed command, route, and budget evidence for every static Work-Graph pass and validate the graph with deterministic failure scenarios.

**Architecture:** `org_graph::WorkState` owns serde-compatible, append-only audit events and retains them across node/pass resets. `exec_session::NodeRuntime` projects real `CommandRun`, profile, route, and budget values into those types before checkpointing. Tests use the existing scripted command executor to prove command order, route selection, retries, and recovery.

**Tech Stack:** Rust, Tokio, Serde, chrono, tempfile, existing ExecutionSession checkpoint and Work-Graph runtime.

## Global Constraints

- Audit types in `org_graph` must not depend on `exec_session` types.
- Only runtime/coordinator code appends audit events; agents and sub-agents cannot write acceptance evidence through field-permission APIs.
- Every persisted event uses only actual executor output and code-owned route decisions.
- stderr is capped to 8,192 UTF-8-safe bytes; stdout is never persisted in graph audit events.
- `graph_audit` must retain history across new nodes, retries, and `inherit_for_new_turn`.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all` before committing.

---

### Task 1: Add typed, checkpoint-compatible graph audit state

**Files:**

- Modify: `src/org_graph/work_state.rs`
- Modify: `src/org_graph/mod.rs`
- Test: unit tests in `src/org_graph/work_state.rs`

**Interfaces:**

- `GraphAuditEvent { node_id: String, attempt: u32, kind: GraphAuditKind, anchor: Option<GraphAuditAnchor>, commands: Vec<AuditCommandRun>, route: Option<GraphAuditRoute>, profile: Option<GraphAuditProfile>, budget: Option<Budget>, timestamp: String }`.
- `GraphAuditKind::{ProfileResolved, AnchorCompleted, RouteSelected}`; `GraphAuditAnchor::{Compile, Test, Verify}`; `GraphAuditRoute::{Implement, CompileAnchor, TestAnchor, VerifyGate, Complete, Escalate}`; `GraphAuditProfile::{None, Rust}` serialize with snake-case values.
- `WorkState::graph_audit(&self) -> &[GraphAuditEvent]` is public read-only; `append_graph_audit` is crate-visible and appends exactly one event.

- [ ] **Step 1: Write failing serde and reset-retention tests**

```rust
#[test]
fn graph_audit_round_trips_and_survives_all_work_state_resets() {
    let mut state = WorkState::default();
    state.append_graph_audit(event("n1", 1, GraphAuditKind::ProfileResolved));
    state.reset_for_new_node();
    state.reset_for_work_graph_pass();
    let inherited = state.inherit_for_new_turn();
    assert_eq!(inherited.graph_audit().len(), 1);
    assert_eq!(serde_json::from_str::<WorkState>(&serde_json::to_string(&inherited).unwrap()).unwrap(), inherited);
}
```

- [ ] **Step 2: Run the focused test and observe the missing audit API**

Run: `cargo test org_graph::work_state::tests::graph_audit_round_trips_and_survives_all_work_state_resets --lib`

Expected: compilation fails because `GraphAuditEvent` and `graph_audit` are absent.

- [ ] **Step 3: Implement the types and retention semantics**

Add the serde-defaulted collection to `WorkState`, all public types to `org_graph::mod`, and the crate-visible append method. Update equality/round-trip fixtures and make every reset/inheritance constructor clone existing audit history rather than clearing it.

- [ ] **Step 4: Verify Task 1**

Run: `cargo test org_graph::work_state::tests --lib && cargo fmt -- --check`

Expected: all WorkState tests and formatting pass.

### Task 2: Project real anchors and routes into audit events

**Files:**

- Modify: `src/exec_session/node_runtime.rs`
- Test: unit tests in `src/exec_session/node_runtime.rs`

**Interfaces:**

- `NodeRuntime` maps `VerificationProfile`, `CommandRun`, `WorkGraphStep`, and a cloned `Budget` into the new `org_graph` audit types without leaking execution-layer types.
- A fresh node appends one profile event after its resolved contract persists.
- Each pass appends an anchor event followed by a route event after compile, test, and final verification; route events carry the budget snapshot.

- [ ] **Step 1: Write a failing success-path audit sequence test**

```rust
#[tokio::test]
async fn rust_work_graph_persists_real_anchor_and_route_audit_sequence() {
    let setup = TestSetup::with_scripted_exit_codes([0, 0, 0]);
    setup.write_cargo_manifest();
    setup.begin_turn();
    setup.runtime.begin_node("goal".into(), vec![], vec![]).await.unwrap();
    assert!(matches!(setup.runtime.verify_current_node().await.unwrap(), NodeVerificationOutcome::WorkGraph(_)));
    let audit = setup.coord.read().unwrap().work_state().graph_audit();
    assert_eq!(audit.iter().map(|event| &event.kind).collect::<Vec<_>>(), [
        &GraphAuditKind::ProfileResolved, &GraphAuditKind::AnchorCompleted,
        &GraphAuditKind::RouteSelected, &GraphAuditKind::AnchorCompleted,
        &GraphAuditKind::RouteSelected, &GraphAuditKind::AnchorCompleted,
        &GraphAuditKind::RouteSelected,
    ]);
    assert_eq!(audit.last().unwrap().route, Some(GraphAuditRoute::Complete));
}
```

- [ ] **Step 2: Run the focused test and observe absent events**

Run: `cargo test exec_session::node_runtime::tests::rust_work_graph_persists_real_anchor_and_route_audit_sequence --lib`

Expected: fails because no graph audit events are emitted.

- [ ] **Step 3: Implement non-forgeable runtime emission**

After resolved-node persistence, append/checkpoint `ProfileResolved`. For every actual command batch, copy command, exit code, and truncated stderr into `AuditCommandRun`, append/checkpoint `AnchorCompleted`, then append/checkpoint `RouteSelected` only after `next_step` returns. Use explicit conversion helpers for profile/step and a UTF-8-safe truncation helper. Clone all data under a short lock and never retain locks across `.await`.

- [ ] **Step 4: Add truncation and checkpoint recovery coverage**

Add a test with an oversized multi-byte stderr proving the stored string is valid UTF-8 and bounded. Add a checkpoint reload test that reads emitted events after a fresh `SessionCoordinator` load, then assert node id, attempt, command exit code, and route remain intact.

- [ ] **Step 5: Verify Task 2**

Run: `cargo test exec_session::node_runtime::tests --lib && cargo clippy --all-targets -- -D warnings`

Expected: runtime audit tests and clippy pass.

### Task 3: Add route-and-budget evaluation scenarios

**Files:**

- Modify: `src/exec_session/node_runtime.rs`
- Test: unit tests in `src/exec_session/node_runtime.rs`
- Modify: `docs/superpowers/plans/2026-08-12-work-graph-audit-evaluation.md`

**Interfaces:**

- Scripted executor support returns configured command results in call order and records every invocation.
- Each scenario asserts real command calls plus audit route and budget snapshots, never model text.

- [x] **Step 1: Write failing evaluation tests**

Add separate tests with these exact assertions:

```rust
// compile failure: calls ["cargo check"], final route Implement, iter_used == 1
// test failure then retry: first pass [check, test], second pass [check, test, clippy],
//     attempts [1, 2], final route Complete, iter_used == 1
// boundary violation: [check, test, clippy], final route Escalate
// exhausted compile budget: [check], final route Escalate, iter_used == max_iter
```

- [x] **Step 2: Run the evaluation tests and observe missing audit assertions**

Run: `cargo test exec_session::node_runtime::tests::audit_ --lib`

Expected: fails until the runtime exposes all required events and route snapshots.

- [x] **Step 3: Implement only test-support and audit corrections needed by scenarios**

Extend the existing mock executor with a queue of exit codes/stderr strings. Do not introduce a dynamic router, LLM decision, or new sub-agent. Make the tests assert profile commands and existing `WorkGraphStep` outcomes exactly.

- [x] **Step 4: Run final validation and commit**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all`

Expected: all commands exit 0.

Run `git add src/org_graph/work_state.rs src/org_graph/mod.rs src/exec_session/node_runtime.rs docs/superpowers/plans/2026-08-12-work-graph-audit-evaluation.md` followed by `git commit -m "feat(graph): audit static work graph routing"`.
