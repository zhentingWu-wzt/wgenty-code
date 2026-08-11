# Specialist Sub-Agent Handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a checkpointed, typed handoff path through which authorised specialist sub-agents publish evidence for later static Work-Graph nodes.

**Architecture:** `org_graph::WorkState` owns validated specialist reports and field-level permissions. `AgentCoordinator` exposes the trusted caller node type to a context-aware `submit_specialist_report` tool, which persists a report only for an active ExecutionSession node and turn. `RootCause` becomes a registered read-only leaf contract that the normal task dispatcher can request without trusting model-provided role metadata.

**Tech Stack:** Rust, Tokio, Serde, async-trait, existing `ToolContext`, `AgentCoordinator`, `SessionCoordinator`, and checkpoint store.

## Global Constraints

- Specialist evidence belongs in `WorkState`; graph-specific handoff must not use parent/child free-text output as its data source.
- The model must never supply an authoritative node type, session id, node id, or turn id.
- Reports cannot write external-anchor results, route/audit records, budgets, or verification status.
- RootCause, Explore, and Plan are leaf, non-mutating contracts; their tool filters must remove task/delegate and filesystem writers.
- Preserve serde compatibility for historical WorkState checkpoints with `#[serde(default)]`.
- All public types and methods require Rustdoc.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all` before committing.

---

### Task 1: Add typed specialist reports and WorkState authorization

**Files:**

- Modify: `src/org_graph/work_state.rs`
- Modify: `src/org_graph/mod.rs`
- Test: unit tests in `src/org_graph/work_state.rs`

**Interfaces:**

- `SpecialistReportKind::{Exploration, RootCause, ImplementationPlan}` serializes as snake case.
- `SpecialistEvidence { path: String, detail: String }` and `SpecialistReport { producer: NodeType, kind: SpecialistReportKind, summary: String, evidence: Vec<SpecialistEvidence>, suspected_files: Vec<String>, recommended_actions: Vec<String> }`.
- `WorkState::specialist_reports(&self) -> &[SpecialistReport]` is read-only.
- `WorkState::set_specialist_report(&mut self, producer: NodeType, report: SpecialistReport) -> Result<(), CoordinatorError>` validates the producer, non-empty evidence/actions, and duplicate `suspected_files` before replacing a same-producer/kind report or appending a new one.
- `WorkField::SpecialistReports` participates in `NodeType::field_perms` and normal `step_log` auditing.

- [x] **Step 1: Write failing report validation, permissions, and lifecycle tests**

```rust
#[test]
fn root_cause_report_is_checkpoint_compatible_and_inherited() {
    let mut state = WorkState::default();
    state.set_specialist_report(NodeType::RootCause, root_cause_report()).unwrap();
    state.reset_for_work_graph_pass();
    assert!(state.specialist_reports().is_empty());

    state.set_specialist_report(NodeType::RootCause, root_cause_report()).unwrap();
    let inherited = state.inherit_for_new_turn();
    assert_eq!(inherited.specialist_reports(), state.specialist_reports());
    assert_eq!(serde_json::from_str::<WorkState>(&serde_json::to_string(&inherited).unwrap()).unwrap(), inherited);
}

#[test]
fn specialist_report_rejects_spoofed_producer_and_invalid_evidence_without_mutation() {
    let mut state = WorkState::default();
    let mut report = root_cause_report();
    report.producer = NodeType::Explore;
    assert!(state.set_specialist_report(NodeType::RootCause, report).is_err());
    assert!(state.specialist_reports().is_empty());
}
```

- [x] **Step 2: Run the focused tests and observe the absent API**

Run: `cargo test org_graph::work_state::tests::root_cause_report --lib`

Expected: compile failure because specialist report types and APIs do not exist.

- [x] **Step 3: Implement the minimal typed state API**

Add the new report types, serde-defaulted collection, getter, `WorkField`, permission checks, and a private validation helper. Preserve reports through `inherit_for_new_turn`, but clear them when a new node or graph pass begins. Export all public report types from `org_graph::mod`.

- [x] **Step 4: Verify Task 1**

Run: `cargo test org_graph::work_state::tests --lib && cargo fmt -- --check`

Expected: every WorkState test passes and formatting is clean.

- [x] **Step 5: Commit Task 1**

```bash
git add src/org_graph/work_state.rs src/org_graph/mod.rs
git commit -m "feat(org-graph): add specialist report state"
```

### Task 2: Register the root-cause leaf contract

**Files:**

- Modify: `src/org_graph/contract.rs`
- Modify: `src/org_graph/registry.rs`
- Modify: `src/org_graph/work_state.rs`
- Modify: `src/tools/meta/task.rs`
- Test: unit tests in `src/org_graph/registry.rs`, `src/org_graph/work_state.rs`, and `src/tools/meta/task/tests.rs`

**Interfaces:**

- `NodeType::RootCause` maps from the only trusted dispatcher spellings `root-cause` and `root_cause`.
- `NodeRegistry::builtin` always includes a `root-cause` contract with structured-json input, report output, no child spawning, no filesystem mutation, and no special tool whitelist.
- RootCause field permissions read requirement, generated diff, compile/test results, verify result, and specialist reports; it writes only specialist reports.

- [x] **Step 1: Write failing root-cause contract and dispatch tests**

```rust
#[test]
fn root_cause_is_a_read_only_leaf_reporter() {
    let contract = registry(true).get(&NodeType::RootCause).unwrap();
    assert!(!contract.permissions.can_spawn);
    assert!(!contract.permissions.can_mutate_fs);
    assert_eq!(contract.input_type, IoShape::StructuredJson);
    assert_eq!(contract.output_type, IoShape::Report);
}

#[test]
fn parse_node_type_accepts_root_cause_only_through_trusted_mapping() {
    assert_eq!(parse_node_type("root-cause"), NodeType::RootCause);
    assert_eq!(parse_node_type("unknown-role"), NodeType::GeneralPurpose);
}
```

- [x] **Step 2: Run the focused tests and observe RootCause is absent**

Run: `cargo test root_cause --lib`

Expected: compile failure or failing registry/dispatcher assertions because RootCause is unregistered.

- [x] **Step 3: Implement the root-cause contract**

Extend the enum, canonical registry order, and task dispatcher mapping. Use a prompt requiring a single evidence-based `submit_specialist_report` call before the final text. Keep ordinary task completion behavior unchanged; this task only defines a role and its capability/field contract.

- [x] **Step 4: Verify Task 2**

Run: `cargo test org_graph::registry::tests --lib && cargo test tools::meta::task::tests --lib && cargo clippy --all-targets -- -D warnings`

Expected: registry, dispatcher, and lint checks pass.

- [x] **Step 5: Commit Task 2**

```bash
git add src/org_graph/contract.rs src/org_graph/registry.rs src/org_graph/work_state.rs src/tools/meta/task.rs src/tools/meta/task/tests.rs
git commit -m "feat(org-graph): register root cause specialist"
```

### Task 3: Authenticate report submission through tool context

**Files:**

- Create: `src/exec_session/specialist_report_tool.rs`
- Modify: `src/agent/coordinator.rs`
- Modify: `src/agent/mod.rs`
- Modify: `src/exec_session/mod.rs`
- Modify: `src/tools/mod.rs`
- Test: unit tests in `src/exec_session/specialist_report_tool.rs`, `src/agent/coordinator.rs`, and `src/tools/meta/task/tests.rs`

**Interfaces:**

- `AgentCoordinator::node_type_for(&self, context: &AgentExecutionContext) -> Result<NodeType, CoordinatorError>` returns the recorded child type and rejects a root or an unknown/stale child context.
- `SubmitSpecialistReportTool::new(session: Arc<RwLock<SessionCoordinator>>, coordinator: Arc<AgentCoordinator>)` implements `Tool::execute_with_context`.
- Its JSON input has exactly one `report` object matching `SpecialistReport`; it never accepts identity or session fields.
- Success persists one report then checkpoints and returns `{ "status": "recorded" }`; invalid context/input returns a structured ToolError without state mutation.

- [x] **Step 1: Write failing trusted-context tests**

```rust
#[tokio::test]
async fn root_cause_child_can_submit_and_checkpoint_a_report() {
    let setup = ToolSetup::with_root_cause_child().await;
    setup.tool.execute_with_context(json!({ "report": root_cause_json() }), &setup.child_context).await.unwrap();
    assert_eq!(setup.session.read().unwrap().work_state().specialist_reports().len(), 1);
    assert!(setup.reload_reports_from_checkpoint().await);
}

#[tokio::test]
async fn root_and_verification_contexts_cannot_submit_specialist_reports() {
    let setup = ToolSetup::with_root_cause_child().await;
    assert!(setup.tool.execute_with_context(json!({ "report": root_cause_json() }), &setup.root_context).await.is_err());
    assert!(setup.session.read().unwrap().work_state().specialist_reports().is_empty());
}
```

- [x] **Step 2: Run the focused tests and observe the missing tool**

Run: `cargo test exec_session::specialist_report_tool::tests --lib`

Expected: compilation fails because the module and context lookup API are absent.

- [x] **Step 3: Implement minimal authenticated persistence**

Add a coordinator lookup that verifies the caller scope is live and has a recorded child node type. Implement JSON parsing and `execute_with_context`; require an active turn/current persisted node, call `set_specialist_report`, and checkpoint only after success. `execute` without context must fail closed. Add the tool to `register_exec_session_tools` with the same session coordinator and a passed-in agent coordinator; update callers/tests to provide that dependency.

- [x] **Step 4: Verify Task 3**

Run: `cargo test exec_session::specialist_report_tool::tests --lib && cargo test agent::coordinator::tests --lib && cargo fmt -- --check`

Expected: authenticated report handoff and existing coordinator behavior pass.

- [x] **Step 5: Commit Task 3**

```bash
git add src/exec_session/specialist_report_tool.rs src/exec_session/mod.rs src/agent/coordinator.rs src/agent/mod.rs src/tools/mod.rs
git commit -m "feat(graph): add authenticated specialist handoff"
```

### Task 4: End-to-end contract verification and documentation

**Files:**

- Modify: `WGENTY.md`
- Modify: `docs/superpowers/specs/2026-08-12-specialist-handoff-design.md`
- Modify: `docs/superpowers/plans/2026-08-12-specialist-handoff.md`
- Test: existing unit/integration suites

**Interfaces:**

- `wgenty-code org-graph contracts --format json` includes the root-cause contract.
- `WGENTY.md` explains that graph-specialist evidence is submitted through the typed handoff and does not determine verification or routing.

- [x] **Step 1: Write the failing end-to-end contract assertion**

```rust
#[test]
fn root_cause_contract_is_rendered_as_a_leaf_reporter() {
    let rendered = render_contracts(&NodeRegistry::builtin(&SubagentLimits::default()), Format::Json);
    assert!(rendered.contains("root-cause"));
    assert!(rendered.contains("root_cause"));
}
```

- [x] **Step 2: Run it before implementation wiring is complete**

Run: `cargo test org_graph::render::tests::root_cause_contract_is_rendered_as_a_leaf_reporter --lib`

Expected: fail until Task 2's canonical registry/render behavior is complete.

- [x] **Step 3: Add documentation and test the real registry/tool boundary**

Document the role, handoff behavior, trusted context, and continued authority of external anchors. Add a final integration-style test that creates a typed root-cause child, submits a report, restores `WorkState` from its checkpoint, and confirms the report cannot mark a node complete.

- [x] **Step 4: Run the complete quality gate**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all`

Expected: all commands exit 0; report the exact passed/ignored counts from fresh output.

- [x] **Step 5: Commit Task 4**

```bash
git add WGENTY.md docs/superpowers/specs/2026-08-12-specialist-handoff-design.md docs/superpowers/plans/2026-08-12-specialist-handoff.md src
git commit -m "docs(graph): document specialist handoff"
```
