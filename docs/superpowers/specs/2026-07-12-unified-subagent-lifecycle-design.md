# Unified Subagent Lifecycle Design

## Summary

Wgenty Code will use one structured-concurrency model for all subagents. The
`task` tool will always launch a child asynchronously and return immediately;
the current `background` switch and its synchronous execution branch will be
removed.

Non-root agents may continue independent work after spawning children, but
they cannot enter a terminal state until every child has reached a terminal
state. Once the children finish, their structured results are injected and the
parent receives another reasoning round to synthesize them before finalizing.

The main agent is a persistent session root. It ends turns, not its own agent
lifecycle, and therefore never enters `Completed` or `Failed`. Its children may
outlive the turn that created them. When their results become ready and no user
turn is available to consume them, the system starts a synthetic continuation
turn so results are never stranded in a mailbox.

## Goals

- Give every subagent one execution model instead of separate synchronous and
  background paths.
- Preserve useful concurrency: a parent can perform work that does not depend
  on its children while those children run.
- Enforce structured lifetimes: a non-root parent cannot terminate before its
  descendants are resolved.
- Let the persistent main agent complete turns while children continue across
  turn boundaries.
- Deliver and synthesize child results even when the user sends no further
  message.
- Preserve strict local visibility: an agent or trusted UI view sees only self
  and direct children unless it uses an issued navigation capability.
- Make the coordinator the sole source of truth for identity, lifecycle,
  task-group membership, and completion.

## Non-goals

- Detached jobs that survive their session or have no owning agent.
- Restoring a globally flattened subagent tree in the TUI.
- Requiring the model to call an explicit `await_task` tool correctly.
- Treating one child failure as an automatic failure of its parent.
- Redesigning unrelated command-background execution.

## Lifecycle Model

### Persistent main agent

The main agent is the root execution context for a session. Its lifecycle is
persistent and does not use terminal child statuses.

The main agent may be operationally `Idle`, `Running`, or `Cancelling`, while
each conversational turn has its own lifecycle:

```text
Idle -> Running -> TurnComplete -> Idle
```

A turn may complete while root children remain active. This does not orphan
the children because the persistent root still owns them.

The main agent only ceases to exist when its session is closed. Session close,
`/clear`, or another explicit reset initiates structured cancellation of the
applicable descendants.

### Non-root agents

Non-root agents use a terminal lifecycle:

```text
Pending
  -> Running
  -> WaitingForChildren
  -> Finalizing
  -> Completed | Failed | Cancelled
```

`WaitingForChildren` is entered when an agent attempts to finish while it
still owns live children. It is a lifecycle barrier, not a blocking `task`
call. The agent may perform independent work before reaching this barrier.

After all children become terminal, the coordinator injects their structured
results and resumes the parent for at least one reasoning round. The parent
must be allowed to revise or replace any previously drafted answer before it
enters `Finalizing`.

An agent must not cache a final answer, wait for children, and then emit the
cached answer unchanged; that would prevent child results from participating
in synthesis.

## Unified `task` Semantics

The `background` input property will be removed. Every `task` invocation uses
the following flow:

1. The coordinator validates the trusted caller context and reserves a child.
2. The coordinator derives the child's session, parent, depth, identity, and
   cancellation scope.
3. The child begins executing concurrently.
4. `task` immediately returns a structured acknowledgement containing at
   least `child_id`, `task_group_id`, and the initial lifecycle status.
5. The parent continues reasoning or executing other work.
6. On child termination, the coordinator records a structured result and
   publishes a completion event to the owning parent/task group.
7. The result is injected before the parent's next relevant reasoning round.

There is no separate synchronous code path. Waiting is enforced when a
non-root parent attempts to terminate, not when it invokes `task`.

## Task Groups and Result Delivery

Children created by one main-agent turn belong to an originating task group.
Task groups provide a stable unit for batching, result delivery, cancellation,
and UI aggregation.

A child result contains a bounded, structured projection:

- child ID;
- terminal status;
- summary or result handle;
- optional error code;
- optional bounded partial result.

Raw descendant identifiers, unrestricted transcripts, and arbitrary internal
state are not propagated upward.

### Non-root delivery

When a non-root parent reaches its completion barrier, the coordinator waits
for every live child in scope. After all children are terminal, the complete
result batch is injected and the parent resumes reasoning. Successful,
failed, and cancelled results are all included so the parent can make an
informed synthesis or explain degraded output.

### Main-agent delivery

The main agent does not wait before completing its current turn. Results are
handled in one of two ways:

1. If a user turn begins before the task group is ready, completed results are
   merged into the next applicable reasoning context. Results already consumed
   by that turn must not trigger a second continuation.
2. If the task group reaches a deliverable terminal state while no user turn
   is consuming it, the coordinator starts a synthetic continuation turn.

The synthetic continuation contains structured child-result events, not a
fabricated user message. It lets the main agent synthesize the batch and send
a follow-up response even if the user never sends another message.

Task-group delivery is idempotent. A race between user input and a completion
event must result in exactly one consumer winning the delivery claim.

### Batching policy

The default batching boundary is the originating main-agent turn. The system
waits until all children in that group are terminal before starting a synthetic
continuation, preventing one unsolicited response per child.

If a group contains work that intentionally remains live until timeout or
cancellation, its configured deadline establishes the latest aggregation
point. At that point unfinished work is represented by a failed result with a
`timeout` error code.

## Failure and Cancellation

Child failure does not automatically fail its parent. The parent receives the
failure alongside successful and partial results, then decides whether it can
continue, retry within policy, degrade its answer, or fail for its own reason.

The terminal child outcomes are:

- `Completed`;
- `Failed`, including timeout through a stable `timeout` error code;
- `Cancelled`.

Using a timeout error code instead of an additional lifecycle status keeps the
terminal state machine small while preserving an actionable cause.

Every task group is bound to a session generation. `/clear` increments the
generation, cancels descendants owned by the old generation, and prevents late
results from being injected into the new conversation context.

Session shutdown uses cooperative cancellation first. After a bounded grace
period, the coordinator aborts unresponsive tasks, records their terminal
outcome, and releases their concurrency permits. Permits are held until a
child's terminal state has been persisted or forced cleanup has completed.

Synthetic continuations are suppressed for cancelled or obsolete task groups.

## Coordinator Responsibilities

The coordinator is the canonical owner of:

- agent identity and trusted parentage;
- agent lifecycle state;
- concurrency permits;
- task-group membership and generation;
- child terminal results;
- result-delivery claims;
- completion and cancellation events.

Progress storage is a presentation projection only. It must use the
coordinator-issued `agent_id` and must not create a second UUID namespace.
Messages, snapshots, token counts, and other display information may be
cross-populated by that canonical ID.

The parent/child completion barrier belongs in the coordinator rather than in
the `task` tool or prompt instructions. Prompt text may explain the behavior,
but correctness must not depend on model compliance.

## TUI and Scoped Navigation

The root status bar shows the main agent's direct children, including children
that remain active across turn boundaries. A completed child may remain visible
for a short configured delay so the user can inspect it.

The focus-view selector remains scoped:

- it shows the currently viewed agent and that agent's direct children;
- it never flattens arbitrary descendants into the root view;
- selecting a child uses the daemon-issued `navigation_capability` to fetch the
  child's local view;
- a bounded back stack returns to prior trusted views;
- arbitrary agent IDs cannot be used to bypass capability checks.

This makes a background child spawned by a subagent discoverable by navigating
into its direct parent without weakening strict subagent isolation.

The UI may show task-group aggregation such as `2 running / 1 completed / 1
failed`. Synthetic continuation activity should be distinguishable from user
input but rendered as a normal main-agent follow-up response when complete.

## Data Flow

```text
main turn
  -> task acknowledgement
  -> coordinator child scope
  -> child runs concurrently
  -> main turn may complete
  -> child terminal result
  -> task-group delivery claim
       -> active user turn: inject once
       -> no active user turn: synthetic continuation
  -> main-agent synthesis response
```

For a non-root parent:

```text
parent running
  -> task acknowledgement
  -> parent performs independent work
  -> parent requests completion
  -> live children exist
  -> WaitingForChildren
  -> all child results injected
  -> parent reasoning resumes
  -> Finalizing
  -> terminal outcome
```

## Migration

1. Introduce coordinator-owned task groups, generations, result batches, and
   idempotent delivery claims.
2. Add synthetic continuation scheduling for the persistent main agent.
3. Enforce the non-root completion barrier and mandatory post-child synthesis
   round.
4. Unify progress records on coordinator `agent_id`.
5. Wire TUI capability navigation and its back stack.
6. Change `task` to return an asynchronous acknowledgement in all cases.
7. Remove the `background` schema property and delete the synchronous execution
   branch.
8. Update prompts, documentation, changelog, and compatibility handling for
   callers that still send `background`.

During a short compatibility window, an incoming `background` property may be
accepted and ignored with diagnostic metadata. It must not select a different
execution path. The property should then be removed once bundled prompts and
known clients have migrated.

## Testing Strategy

### Coordinator tests

- `task` reserves a child and returns without awaiting child completion.
- Trusted parentage, depth, session, and IDs are coordinator-derived.
- A non-root parent cannot become terminal while any child is live.
- A waiting parent resumes after all children terminate.
- The resumed parent receives one complete structured result batch.
- Failed, cancelled, and timed-out children release concurrency permits.
- `/clear` prevents old-generation delivery.
- Concurrent delivery attempts have exactly one winner.

### Agent-loop tests

- A parent continues independent work after spawning a child.
- Post-child synthesis occurs before non-root finalization.
- A main turn completes while root children remain active.
- A user turn consumes ready results without a duplicate synthetic turn.
- With no user input, group completion starts one synthetic continuation.
- Multiple children in one group produce one aggregated follow-up.

### TUI and daemon tests

- Root views contain only root and direct children.
- Capability navigation reveals the selected child's direct children.
- Invalid, stale, sibling, and cross-session navigation is denied uniformly.
- Active root children remain visible across turn completion and the next turn.
- Completed nodes follow the configured retention delay.
- Selector content uses coordinator IDs and receives the matching messages and
  snapshots.

### End-to-end tests

- A main agent spawns multiple children, replies before they finish, and later
  sends one automatic synthesized follow-up.
- A subagent spawns children, performs independent work, waits at completion,
  and returns a synthesis that includes every child outcome.
- User input racing with group completion consumes results exactly once.
- `/clear` during execution cancels the old tree and produces no late follow-up.
- Session shutdown cleans up cooperative and unresponsive children.

## Acceptance Criteria

- `task` exposes one asynchronous subagent behavior and no runtime mode switch.
- No non-root agent reaches a terminal state with live descendants.
- The main agent remains persistent and may finish turns with active children.
- Every terminal task group is either consumed by a user turn, delivered by one
  synthetic continuation, or explicitly cancelled as obsolete.
- The TUI can navigate the strict local-view hierarchy without receiving a
  flattened global agent tree.
- Coordinator identity is the only identity used to join lifecycle and progress
  data.
- Cancellation, timeout, and delivery races are covered by deterministic tests.
