# Final Fix Report: Work-Graph Audit/Evaluation Invariants

## Findings resolved

1. Complete Work-Graph passes are serialized by an async `NodeRuntime` gate.
   The gate spans command execution and all state/checkpoint transitions, while
   coordinator `RwLock` guards remain short-lived and are never held across an
   `.await`. Concurrent passes therefore produce sequential, distinct attempt
   numbers without interleaving shared `WorkState`.
2. A failed final verification command consumes one WorkState iteration before
   the verification anchor checkpoint and route selection. Exhaustion selects
   `Escalate` and aligns the session to `Failed`. Boundary-only failures retain
   immediate escalation, do not consume iteration budget, and also leave the
   session in the matching failed terminal state.
3. `SessionCoordinator::begin_turn` derives inherited WorkState once and
   persists it into the newly active checkpoint before returning. An immediate
   `restore_work_state_for_turn` now preserves inherited graph audit history.
4. `ProfileResolved` records the exact resolved compile, test, and final verify
   command vectors in typed `GraphAuditCommands`. The new optional event field
   has `#[serde(default)]`, so historical audit events/checkpoints deserialize
   with `resolved_commands: None`.

The changes preserve the `org_graph -> exec_session` dependency boundary,
code-owned/descriptive-only routing, persisted node identity requirement, and
the existing UTF-8-safe 8,192-byte stderr cap.

## Strict TDD evidence

### RED

1. `cargo test exec_session::node_runtime::tests::concurrent_work_graph_passes_are_serialized_with_distinct_attempts --lib`
   - Failed at the barrier assertion because a second pass entered command
     execution before the first pass finished.
2. `cargo test exec_session::node_runtime::tests::audit_ --lib`
   - `audit_exhausted_final_verification_records_consumed_budget_and_escalates_session`
     returned `Implement` instead of `Escalate`.
   - `audit_boundary_violation_records_escalate_route` observed session status
     `InProgress` instead of `Failed`.
3. `cargo test exec_session::coordinator::tests::begin_turn_immediate_restore_preserves_inherited_graph_audit --lib`
   - Failed with an audit length of 0 rather than 1 after immediate restore.
4. `cargo test exec_session::node_runtime::tests::rust_work_graph_persists_real_anchor_and_route_audit_sequence --lib`
   - Failed to compile because `GraphAuditCommands` and the
     `resolved_commands` event field did not exist.

### GREEN

The same regressions passed after their minimal production corrections.
Focused suites also passed:

- `cargo test exec_session::node_runtime::tests --lib`: 32 passed.
- `cargo test exec_session::coordinator::tests --lib`: 26 passed.
- `cargo test org_graph::work_state::tests --lib`: 21 passed.

## Final verification

- `cargo fmt -- --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed with zero warnings.
- `cargo test --all`: library 1,501 passed / 1 ignored; integration 183 passed /
  3 ignored; binary and doc suites passed; 0 failures.

## Follow-up: terminal verify-log consistency

A scoped re-review found that final-command budget exhaustion and boundary
escalation updated `session.status` to `Failed` without updating
`verify_log.json.final_status`. `VerifyGate` now owns a centralized terminal
transition that writes both durable projections, and the code-owned Work-Graph
`Escalate` route calls it after persisting the route audit. No status is accepted
from model output.

### RED

`cargo test exec_session::node_runtime::tests::audit_ --lib` failed in both
terminal scenarios:

- `audit_exhausted_final_verification_records_consumed_budget_and_escalates_session`
  observed `verify_log.final_status == None` instead of `Some(Failed)`.
- `audit_boundary_violation_records_escalate_route` observed the same mismatch.

The retry control
`audit_final_verification_retry_keeps_session_and_verify_log_open` passed with
`SessionStatus::InProgress` and `final_status == None` before and after the fix.

### GREEN

- `cargo test exec_session::node_runtime::tests::audit_ --lib`: 7 passed.
- `cargo test exec_session::verify_gate::tests --lib`: 18 passed.
- Final `cargo fmt -- --check` and
  `cargo clippy --all-targets -- -D warnings`: passed.
- Final `cargo test --all`: library 1,502 passed / 1 ignored; integration 183
  passed / 3 ignored; binary and doc suites passed; 0 failures.

## Concerns

No new concerns. Residual multi-file checkpoint atomicity and retention policy
remain explicitly out of scope for this fix wave and were not broadened.
