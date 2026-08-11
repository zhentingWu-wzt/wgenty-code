# Work-Graph Audit and Evaluation Design

## Goal

Make the static Work-Graph explainable and measurable before adding dynamic
construction or new sub-agent routes. Every real anchor execution and every
code-owned route must leave durable, typed evidence that survives checkpoints.

## Scope

Add an append-only `graph_audit` collection to `org_graph::WorkState` and
record it from `NodeRuntime` during a static compile → test → verify pass.
Add scenario tests that evaluate routing, anchor order, retry budget, and audit
persistence from the same external-anchor state used in production.

## Data Model

`GraphAuditEvent` is an `org_graph` type so it serializes with the existing
`WorkState` checkpoint without creating an `org_graph -> exec_session`
dependency. It contains:

- `node_id` and `attempt`: identify the node and fresh graph pass;
- `kind`: `profile_resolved`, `anchor_completed`, or `route_selected`;
- `anchor`: optional `compile`, `test`, or `verify` phase;
- `commands`: zero or more `AuditCommandRun { command, exit_code, stderr }`;
- `route`: optional selected `WorkGraphStep` as a stable snake-case value;
- `budget`: optional immutable snapshot of `{ max_iter, iter_used, token_used }`;
- RFC 3339 `timestamp`.

Only stderr is retained from command output: it contains actionable compiler
and test diagnostics while avoiding unbounded logs or accidental persistence of
normal output. Each stderr value is capped at 8,192 UTF-8-safe bytes with a
deterministic truncation marker.

`WorkState::graph_audit()` is read-only. `append_graph_audit` is coordinator
privileged and not exposed through field-permission APIs, so a model or a
sub-agent cannot manufacture acceptance evidence. `reset_for_new_node`,
`reset_for_work_graph_pass`, and `inherit_for_new_turn` retain prior audit
events; audit is historical evidence, not a node product.

## Runtime Flow

At node creation, `NodeRuntime` appends `profile_resolved` with the persisted
profile and resolved anchor names. At the start of `run_work_graph`, it derives
the current node id and attempt number from the persisted audit history.

After each command batch completes, before routing, the runtime appends one
`anchor_completed` event based on actual `CommandRun` values. It records the
structured state, checkpoints it, evaluates `next_step`, then appends a
`route_selected` event with that edge and a cloned budget snapshot, and
checkpoints again. The final VerifyGate follows the same order after its
changed-file boundary check. No lock is held during command execution.

All routes remain code-owned. Audit values describe the decision after it is
made; they never influence route selection.

## Evaluation Scenarios

Focused runtime tests use a scripted executor and assert both real command
order and persisted audit event sequences for:

1. Rust profile success: compile, test, verify, then `complete`.
2. Compile failure with budget remaining: compile then `implement` and one
   consumed iteration.
3. Test failure followed by retry: each pass starts at compile and audit
   retains both attempts.
4. Final boundary violation: compile, test, verify, then `escalate`.
5. Failed compile with exhausted retry budget: route is `escalate`.

Each test reloads the relevant checkpoint where applicable so event persistence
is proven rather than inferred from in-memory state.

## Compatibility and Non-goals

`graph_audit` uses `#[serde(default)]`; historical checkpoints deserialize
without it. This phase does not add a CLI/API reader, daemon streaming, new
sub-agent roles, or dynamic graph selection. Those consume the evidence in
later phases after the event schema and evaluation suite are stable.
