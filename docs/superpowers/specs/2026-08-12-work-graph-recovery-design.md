# Work-Graph Restart Recovery Design

## Goal

Allow a daemon or headless runtime to resume an existing ExecutionSession after
a process restart without losing its current node, checkpointed `WorkState`,
anchor evidence, budget, or graph audit. A RootCause child that disappeared in
the restart is never trusted after recovery; the runtime deterministically
dispatches a fresh child from the restored external-anchor evidence.

## Scope

This design completes the checkpoint boundary required by the fixed diagnostic
Work-Graph. It does not introduce model-selected routes, arbitrary roles, or a
dynamic Work-Graph. It makes those later steps safe by ensuring the static
graph's shared State and its code-owned route can survive a runtime restart.

## Current Gap

`SessionState` and its per-turn `WorkState` checkpoint already persist below
`.wgenty-code/snapshots/<session-id>/`. However,
`ExecutionSessionRuntimeStore::entry_for` currently calls
`SessionCoordinator::new` every time its in-memory map is empty. That replaces
an existing `session.json` with a fresh empty session, so restart loses the
node cursor, failure anchors, graph audit, and the RootCause route.

The in-memory `pending_root_cause` record cannot be restored as-is: its child
identity belonged to a process that no longer exists. Treating that child as
live would violate the external-anchor and authenticated-handoff invariants.

## Decision

Use **open-or-create session recovery plus safe RootCause re-dispatch**.

1. `SessionCoordinator::open_or_create` resolves the session directory. If
   `session.json` exists, it loads it and restores `WorkState` from the active
   turn's checkpoint. A missing legacy work-state sidecar restores as default,
   preserving compatibility. If no session exists, it creates one exactly as
   today.
2. `ExecutionSessionRuntimeStore::entry_for` uses this operation, so every
   trusted session id maps to the same persisted graph state across process
   lifetimes.
3. On a recovered current node, the store derives the next static step solely
   from restored State. `RootCause` means the previous child was lost. The
   store creates an unbound, recovered dispatch reservation from the persisted
   node id and audit attempt; it never restores a child id.
4. A code-owned recovery entry delegates only that recovered RootCause edge to
   the existing `RegistryRootCauseDispatcher`. The dispatcher reserves a new
   coordinator child and TaskTool binds the new child id before spawning it.
   The normal root-cause gate stays closed throughout.
5. If a recovered session is already `Complete` or `Escalate`, no child is
   dispatched. A terminal session remains terminal and is not silently made
   writable.

The recovery entry must not re-run compile, test, or verify anchors. Those
observations are already persisted external evidence for the current attempt;
re-running them would create a different attempt and invalidate the report
binding the RootCause child receives.

## Interfaces

`SessionCoordinator` gains an open-or-create constructor that returns the same
public coordinator abstraction used today. Its failure modes include corrupt
`session.json`, a missing current-turn record, and an unreadable WorkState
sidecar; errors carry session-directory context and do not overwrite the
existing session.

`ExecutionSessionRuntimeStore` exposes a recovery query that returns one of:

- `NoRecovery`: a new or terminal session, or a state that is not on a
  RootCause edge;
- `RootCause(RootCauseDispatchRequest)`: a recovered retryable failure whose
  anchored state requires a new diagnostic child; or
- `Blocked`: an invalid restored state that must be escalated, not dispatched.

`VerifyNodeTool` and the daemon startup/session attachment path use a shared
code-owned dispatcher helper for both a newly-selected and a recovered
RootCause edge. The helper accepts only a trusted `ToolContext`; model JSON
cannot ask to recover a different session, node, attempt, or child role.

## State and Lifecycle Invariants

- The persisted node id and the latest `GraphAuditRoute::RootCause` attempt
  are the recovery source of truth. A new child must bind to both before it can
  submit a report.
- A recovered reservation always starts with `child_id = None`. Old IDs are
  deliberately discarded.
- While a recovered reservation is unbound or bound, the registry denies all
  non-read-only tools except `submit_specialist_report`; this applies to root
  and every child context. The dispatcher crosses that guard only through its
  private code-owned TaskTool call.
- A recovered report follows the same node/attempt/child-id checks and writes
  the same `Implement` audit event as an uninterrupted run.
- Corrupt or structurally inconsistent session data is fail-closed: preserve
  files, return a structured recovery error, and never replace the snapshot
  with a new session.

## Alternatives Rejected

1. **Always create a new session:** simple but destroys checkpoint semantics
   and makes graphs unauditable across a daemon restart.
2. **Restore the old child id:** unsafe because the old child is not live and
   could allow a stale report to release `Implement`.
3. **Re-run verification to rediscover the route:** changes the attempt and
   races a recovered diagnostic child. Recovery must use the persisted anchor
   evidence.
4. **Immediately escalate every recovered RootCause:** safe but needlessly
   removes autonomous recovery, contrary to the long-term multi-agent goal.

## Tests and Evidence

The implementation must add tests proving:

1. a store recreated over the same project/session restores the current node,
   WorkState, audit route, and budget without overwriting `session.json`;
2. a persisted RootCause route produces a fresh unbound reservation with the
   original node and attempt, never the old child id;
3. recovery dispatch reaches TaskTool, binds before spawn, and a new child
   report releases only `Implement`;
4. no recovery path runs an anchor command or increments the graph attempt;
5. corrupt `session.json` fails with context and remains unchanged; and
6. terminal `Complete` and `Escalate` checkpoints do not dispatch a child.

The final gate remains `cargo fmt -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --all`, and
`git diff --check`.
