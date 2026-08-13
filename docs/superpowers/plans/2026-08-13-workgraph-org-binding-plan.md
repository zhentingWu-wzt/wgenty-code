# Work-Graph Org-Graph Binding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bind selected Work-Graph roles to `NodeRegistry` contracts and persist dispatched child-agent role identities.

**Architecture:** Keep the current code-owned templates and routing. Add a registry-backed validation/binding layer to `WorkGraphPlan`, then add durable audit metadata for RootCause child instances. Legacy checkpoints remain compatible through serde defaults.

**Tech Stack:** Rust, serde, anyhow, existing `NodeRegistry`, `WorkState`, `ExecutionSessionRuntimeStore`.

## Global Constraints

- Do not add new node types.
- Preserve external-anchor and checkpoint recovery behavior.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all`.
- Preserve the pre-existing progress ledger modification.

### Task 1: Registry-backed Work-Graph role binding

**Files:**
- Modify: `src/org_graph/work_graph_plan.rs`
- Modify: `src/exec_session/node_runtime.rs`
- Test: `src/org_graph/work_graph_plan.rs`

**Interfaces:**
- Add `WorkGraphPlan::bind_registry(&self, registry: &NodeRegistry) -> Result<BoundWorkGraphPlan>`.
- `BoundWorkGraphPlan` contains the selected plan and role contract names.
- `begin_node_with_work_graph` validates the selected plan against the built-in registry before checkpoint persistence.

- [ ] Write tests for successful binding and missing-role rejection.
- [ ] Run `cargo test org_graph::work_graph_plan::tests --lib` and observe the new API test fail before implementation.
- [ ] Implement binding with code-owned role lookup and actionable errors.
- [ ] Run focused tests, format, and Clippy.
- [ ] Commit: `feat(graph): bind work graph roles to org contracts`.

### Task 2: Persist RootCause child-agent role binding

**Files:**
- Modify: `src/org_graph/work_state.rs`
- Modify: `src/exec_session/runtime_store.rs`
- Test: `src/exec_session/runtime_store.rs`

**Interfaces:**
- Extend `GraphAuditEvent` with optional `role` and `child_agent_id` fields using serde defaults.
- When RootCause dispatch binds a child, append a durable audit event with role `RootCause` and that child ID.

- [ ] Write a test that binds a RootCause child and finds its role/child ID in persisted audit state.
- [ ] Run the focused runtime-store test and observe failure.
- [ ] Implement the event projection without exposing model-controlled identity fields.
- [ ] Run focused tests, format, and Clippy.
- [ ] Commit: `feat(graph): audit child agent role bindings`.

### Task 3: Full verification and handoff

**Files:**
- Modify: `docs/superpowers/specs/2026-08-13-workgraph-org-binding-design.md` only if verification notes need correction.

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo clippy --all-targets -- -D warnings`.
- [ ] Run `cargo test --all` with loopback permission if required.
- [ ] Run `git diff --check` and confirm only the pre-existing progress ledger is dirty.
- [ ] Commit any required verification-only fixes with a Conventional Commit.
