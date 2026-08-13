# Static Work-Graph v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Make the execution-session verification path a fixed, externally anchored compile → test → verify work graph with deterministic retry and escalation routing.

**Architecture:** Keep WorkState as the task-scoped persistence boundary and VerifyGate as the command executor and boundary checker. Add typed compile/test anchor outcomes and a pure next_step router in the execution-session layer; NodeRuntime owns state writes and invokes it after anchored command execution. No new LLM agent type or runtime graph generation is introduced.

**Tech Stack:** Rust, Tokio, Serde, existing exec_session, org_graph, CheckpointStore, Cargo tests.

## Global Constraints

- Follow Rust naming conventions and run cargo fmt before verification.
- Use anyhow::Context on fallible application-layer operations; do not introduce unwrap() in production code.
- Keep org_graph independent of exec_session; project execution results into org_graph types at the boundary.
- Route only from typed WorkState fields and coordinator-owned retry state; never parse an agent's text response.
- Preserve checkpoint persistence and existing rollback semantics.

---

### Task 1: Model anchored results and deterministic routes

**Files:**

- Modify: src/org_graph/work_state.rs:20-370
- Modify: src/org_graph/mod.rs:8-17
- Create: src/exec_session/work_graph.rs
- Modify: src/exec_session/mod.rs
- Test: unit tests in src/org_graph/work_state.rs and src/exec_session/work_graph.rs

**Interfaces:**

- Produces CompileResult and TestResult containing ok, command, exit_code, and stderr; TestResult additionally contains failed_cases.
- Produces WorkGraphStep::{Implement, CompileAnchor, TestAnchor, VerifyGate, Complete, Escalate}.
- Produces next_step(state: &WorkState) -> Result<WorkGraphStep, CoordinatorError>.

- [ ] **Step 1: Write the failing WorkState permission test**

```rust
#[test]
fn verification_can_write_compile_and_test_anchor_results() {
    let mut state = WorkState::default();
    state.set_compile_result(NodeType::Verification, compile_failure()).unwrap();
    state.set_test_result(NodeType::Verification, test_failure()).unwrap();
    assert!(!state.compile_result(NodeType::Verification).unwrap().unwrap().ok);
}
```

- [ ] **Step 2: Verify the test is red**

Run: cargo test org_graph::work_state::tests::verification_can_write_compile_and_test_anchor_results --lib

Expected: FAIL with node type not permitted to write compile_result.

- [ ] **Step 3: Implement the smallest state schema and permission changes**

Allow only NodeType::Verification to write compile/test fields, add execution provenance fields, and keep recording writes in step_log. Do not grant GeneralPurpose write access to anchor outputs.

- [ ] **Step 4: Write failing route tests**

```rust
#[test]
fn boundary_violation_escalates_without_retry() {
    assert_eq!(next_step(&state_with_boundary_violation()).unwrap(), WorkGraphStep::Escalate);
}

#[test]
fn failed_compile_routes_to_implement_when_budget_remains() {
    assert_eq!(next_step(&state_with_compile_failure()).unwrap(), WorkGraphStep::Implement);
}
```

- [ ] **Step 5: Verify the router tests are red**

Run: cargo test exec_session::work_graph::tests --lib

Expected: FAIL with unresolved next_step / WorkGraphStep symbols.

- [ ] **Step 6: Implement the pure router**

Use this precedence: missing compile result → CompileAnchor; failed compile → Implement when budget remains, otherwise Escalate; missing test result after compile success → TestAnchor; failed test → retry or escalate; missing verify result after both anchors pass → VerifyGate; successful verify → Complete; boundary violation → Escalate.

- [ ] **Step 7: Verify and commit**

Run: cargo test org_graph::work_state::tests --lib && cargo test exec_session::work_graph::tests --lib && cargo fmt -- --check

```bash
git add src/org_graph/work_state.rs src/org_graph/mod.rs src/exec_session/work_graph.rs src/exec_session/mod.rs
git commit -m "feat(graph): add anchored static work graph routing"
```

### Task 2: Execute compile and test anchors from NodeRuntime

**Files:**

- Modify: src/exec_session/node_runtime.rs:44-285
- Modify: src/exec_session/verify_gate.rs:40-245
- Test: unit tests in src/exec_session/node_runtime.rs
- Test: tests/integration/exec_session_node_lifecycle.rs

**Interfaces:**

- Consumes WorkGraphStep, next_step, VerifyGate::verify_and_complete, and CommandRun.
- Produces a NodeRuntime entry point that runs compile commands, test commands, and final verification in fixed order, persists each result before routing, and never holds the coordinator lock across command execution.

- [ ] **Step 1: Write the failing compile-short-circuit test**

```rust
#[tokio::test]
async fn compile_failure_is_persisted_and_does_not_run_test_anchor() {
    let runtime = runtime_with_scripted_commands([compile_failure_run()]);
    let outcome = runtime.run_work_graph().await.unwrap();
    assert_eq!(outcome.next_step, WorkGraphStep::Implement);
    assert_eq!(scripted_executor_call_count(), 1);
    assert!(!persisted_compile_result().ok);
}
```

- [ ] **Step 2: Verify the test is red**

Run: cargo test exec_session::node_runtime::tests::compile_failure_is_persisted_and_does_not_run_test_anchor --lib

Expected: FAIL with no method named run_work_graph.

- [ ] **Step 3: Implement anchored execution**

Add typed node inputs that distinguish compile, test, and final verify command lists. Run commands through CommandExecutor, project output into WorkState, checkpoint after every state write, and call next_step after each anchor. Do not hold a coordinator lock over await.

- [ ] **Step 4: Write and run failing tests for the remaining exits**

```rust
#[tokio::test]
async fn test_failure_with_exhausted_budget_escalates() {
    let setup = TestSetup::with_results([pass_run("cargo check"), fail_run("cargo test")]);
    setup.begin_turn();
    setup.runtime.begin_node_with_work_graph("goal".into(), compile_commands(), test_commands(), verify_commands(), vec![]).await.unwrap();
    assert_eq!(setup.runtime.run_work_graph().await.unwrap().next_step, WorkGraphStep::Escalate);
}

#[tokio::test]
async fn verified_anchors_complete_the_graph() {
    let setup = TestSetup::with_results([pass_run("cargo check"), pass_run("cargo test"), pass_run("cargo test --doc")]);
    setup.begin_turn();
    setup.runtime.begin_node_with_work_graph("goal".into(), compile_commands(), test_commands(), verify_commands(), vec![]).await.unwrap();
    assert_eq!(setup.runtime.run_work_graph().await.unwrap().next_step, WorkGraphStep::Complete);
}

#[tokio::test]
async fn boundary_violation_escalates_even_with_budget() {
    let setup = TestSetup::with_results([pass_run("cargo check"), pass_run("cargo test"), pass_run("cargo test --doc")]);
    setup.begin_turn();
    setup.create_out_of_scope_file("unexpected.rs");
    setup.runtime.begin_node_with_work_graph("goal".into(), compile_commands(), test_commands(), verify_commands(), vec!["expected.rs".into()]).await.unwrap();
    assert_eq!(setup.runtime.run_work_graph().await.unwrap().next_step, WorkGraphStep::Escalate);
}
```

Run: cargo test exec_session::node_runtime::tests --lib

Expected before transitions exist: the new route assertions fail.

- [ ] **Step 5: Implement minimal transitions and verify green**

Run: cargo test exec_session::node_runtime::tests --lib && cargo test --test exec_session_node_lifecycle && cargo fmt -- --check

Expected: all tests pass and formatting check exits 0.

- [ ] **Step 6: Commit**

```bash
git add src/exec_session/node_runtime.rs src/exec_session/verify_gate.rs tests/integration/exec_session_node_lifecycle.rs
git commit -m "feat(graph): execute compile and test anchors"
```

### Task 3: Persist the real workspace diff and enforce the verification veto boundary

**Files:**

- Modify: src/exec_session/coordinator.rs:386-450
- Modify: src/exec_session/node_runtime.rs
- Modify: src/org_graph/registry.rs:112-139
- Test: unit tests in src/exec_session/coordinator.rs and src/org_graph/registry.rs
- Test: tests/integration/exec_session_e2e.rs

**Interfaces:**

- Produces a coordinator method that derives GeneratedDiff { summary, files } from workspace/checkpoint evidence rather than model text.
- Enforces that NodeType::Verification cannot mutate the filesystem or spawn subagents.

- [ ] **Step 1: Write the failing veto-gate permission test**

```rust
#[test]
fn verification_is_a_non_mutating_leaf() {
    let contract = NodeRegistry::builtin(&SubagentLimits::default())
        .get(&NodeType::Verification).unwrap();
    assert!(!contract.permissions.can_spawn);
    assert!(!contract.permissions.can_mutate_fs);
}
```

- [ ] **Step 2: Verify the test is red**

Run: cargo test org_graph::registry::tests::verification_is_a_non_mutating_leaf --lib

Expected: FAIL on can_mutate_fs.

- [ ] **Step 3: Implement least-privilege verification and diff capture**

Set verification non-mutating. Add a coordinator-owned diff capture method using the project root and checkpoint/changed-files evidence; write it through the authorised production path and checkpoint the resulting GeneratedDiff. Preserve error context for git or filesystem failures.

- [ ] **Step 4: Write the failing restore test**

```rust
#[tokio::test]
async fn restored_work_state_contains_anchored_results_and_workspace_diff() {
    // Run a graph turn, restore the checkpoint into a new coordinator,
    // then assert compile_result, test_result, verify_result, and generated_diff.
}
```

- [ ] **Step 5: Implement restore and capture wiring, then verify green**

Run: cargo test --test exec_session_e2e restored_work_state_contains_anchored_results_and_workspace_diff

Expected before diff capture: FAIL because the actual diff is absent; expected after: PASS.

- [ ] **Step 6: Run full verification and commit**

Run: cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all

```bash
git add src/exec_session/coordinator.rs src/exec_session/node_runtime.rs src/org_graph/registry.rs tests/integration/exec_session_e2e.rs
git commit -m "feat(graph): enforce anchored verification veto gate"
```
