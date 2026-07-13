---
status: approved-design
date: 2026-07-11
scope: agent-runtime
supersedes-authorization-semantics: 2026-06-13-subagent-visibility-design
---

# Strict Subagent Isolation and Structured Concurrency Design

## Context

The current subagent implementation already records `parent_id` in
`SubagentProgress` and renders a tree in the TUI, but that tree is an
observability structure rather than an authorization boundary. Several runtime
paths still treat agent identity as caller-supplied data:

- `TaskTool` reads `_session_id` and `_subagent_depth` from model-controlled tool
  input.
- Ordinary `task` subagents are registered with `parent_id: None`, while the RLM
  path sets a parent, so hierarchy semantics differ by execution path.
- `ToolRegistry::execute` receives only a tool name and JSON input. It cannot
  authenticate which agent invoked an identity-sensitive tool.
- The daemon progress endpoint returns the complete progress map for a session.
- `SubagentTree::real_node_list()` and the focus selector traverse every real
  node in the current tree.
- `background: true` uses `tokio::spawn` and returns immediately, so a parent
  turn can finish while its child continues to exist.
- The older flat `AgentsService` stores globally listable sessions without a
  parent relation.

These behaviors make a hierarchical UI possible, but they do not provide a
hierarchical security model. A malicious or confused model can forge reserved
JSON fields, and a client that receives the whole session tree can observe
agents outside its local branch.

This design introduces a trusted execution context, a central coordinator, and
structured concurrency. Every agent, including the root agent, receives the
same strict local view: itself and its direct children only.

## Relationship to Existing Designs

This design extends the observability concepts in
`2026-06-13-subagent-visibility-design.md` but replaces that document's
authorization semantics.

The earlier design remains useful for progress events, status rendering, and
historical summaries. Its full-tree traversal is no longer an allowed data
source for an agent-facing API or model context. A trusted user interface may
navigate one level at a time through scoped capabilities, but each request is
authorized independently and returns only the selected agent's local view.

The structured error and partial-result behavior documented in
`2026-07-06-subagent-structured-errors-design.md` remains valid. This design
adds cancellation, join, and visibility rules around those results; it does not
remove `SubagentError` or partial-result delivery.

## Goals

1. Give every agent a strict local view containing only itself and its direct
   children.
2. Derive agent identity, parentage, depth, and cancellation scope from trusted
   runtime state rather than model JSON.
3. Ensure parents cannot complete while scoped children are still running.
4. Recursively cancel a subtree when its parent fails or is cancelled.
5. Make creation, lookup, transcript access, result delivery, and cancellation
   use one authorization policy across TUI, daemon, and internal tools.
6. Preserve useful progress visibility without leaking ancestor, sibling,
   grandchild, or unrelated-branch information into model context.
7. Migrate incrementally without requiring every tool to change at once.

## Non-Goals

- Providing a root or administrator agent with global tree access.
- Allowing agents to address siblings, ancestors, or arbitrary descendants.
- Treating TUI filtering as a security boundary.
- Supporting detached agents through the subagent API.
- Redesigning general chat-session persistence or all background commands.
- Encrypting transcripts at rest. Authorization controls access, while storage
  encryption remains a separate concern.
- Replacing the existing structured subagent error taxonomy.

## Security Invariants

The implementation must preserve these invariants on every execution path:

1. **Trusted identity**: `session_id`, `agent_id`, `parent_id`, `depth`, and the
   cancellation token come from runtime context, never tool input.
2. **Runtime parentage**: spawning from agent `A` always creates a child whose
   `parent_id` is `A.agent_id` and whose `depth` is `A.depth + 1`.
3. **Strict local visibility**: an agent can read itself and direct children,
   and no other agents.
4. **No existence oracle**: a nonexistent target and a target outside the local
   view produce the same `NotVisible` error.
5. **Scoped termination**: a parent cannot transition to `Completed`, `Failed`,
   or `Cancelled` while any direct child is non-terminal.
6. **Recursive cancellation**: parent failure or cancellation cancels every
   live descendant before the parent reaches its terminal state.
7. **No orphan subagents**: every live subagent belongs to exactly one live
   parent scope.
8. **Summary-only upward flow**: a child's descendant identifiers, descendant
   transcripts, and tree structure are not embedded in the result returned to
   its parent.
9. **Uniform root policy**: the root agent has no visibility exception. It sees
   only itself and its direct children.

## Architecture

### Trusted execution context

Each running agent receives an immutable identity envelope created by the
runtime:

```rust
pub struct AgentExecutionContext {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub depth: usize,
    pub cancellation: CancellationToken,
}
```

`parent_id` is retained in the context for lifecycle bookkeeping and trusted
telemetry. It is not exposed through the agent's local-view API. An agent does
not gain permission to inspect its parent merely because the runtime knows the
parent identifier.

The root context is created by the session runtime with `parent_id: None` and
`depth: 0`. Child contexts can only be created by `AgentCoordinator`.

Reserved compatibility fields such as `_agent_id`, `_parent_id`, `_session_id`,
and `_subagent_depth` must be ignored or rejected at the identity boundary.
They cannot override `AgentExecutionContext`.

### AgentCoordinator

`AgentCoordinator` is the sole entry point for agent creation, local lookup,
joining, and subtree cancellation. It owns the hierarchy store, concurrency
permits, scoped task handles, and lifecycle transitions.

The minimum interface is:

```rust
impl AgentCoordinator {
    pub async fn spawn_child(
        &self,
        caller: &AgentExecutionContext,
        request: SpawnChildRequest,
    ) -> Result<ChildHandle, CoordinatorError>;

    pub async fn list_local(
        &self,
        caller: &AgentExecutionContext,
    ) -> Result<LocalAgentView, CoordinatorError>;

    pub async fn join_children(
        &self,
        caller: &AgentExecutionContext,
        policy: JoinPolicy,
    ) -> Result<Vec<ChildResult>, CoordinatorError>;

    pub async fn cancel_subtree(
        &self,
        caller: &AgentExecutionContext,
        target: AgentId,
    ) -> Result<(), CoordinatorError>;
}
```

All operations validate that the caller's `session_id` matches the target
session. Targeted read, transcript, result, and cancellation operations accept
only `caller.agent_id` or an agent whose `parent_id == caller.agent_id`.

Concurrency limits use a Tokio `Semaphore`. The permit is acquired before the
child record becomes runnable and is owned by the child scope until terminal
cleanup. This replaces `AtomicUsize` plus polling and removes the race between
checking the count and incrementing it.

### Runtime ownership

Each agent scope owns:

- Its trusted `AgentExecutionContext`.
- Direct-child handles registered by `AgentCoordinator`.
- A cancellation token derived from the parent's token.
- A finalization guard that joins or cancels direct children.
- Its own transcript and progress writer.

The model loop does not own hierarchy mutation directly. It asks the `task`
tool to spawn work, and that tool delegates to the coordinator using the trusted
tool context.

## Data Model

The canonical agent record contains internal fields that are never serialized
wholesale to an agent-facing client:

```rust
pub struct AgentRecord {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub depth: usize,
    pub generation: u64,
    pub status: AgentLifecycleStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub summary: Option<ChildSummary>,
}
```

`generation` changes when an identifier is reissued or its capability scope is
invalidated. Agent identifiers should normally remain unique and never be
reused, but generation binding prevents stale capabilities from becoming valid
if storage restoration or migration reintroduces an identifier.

The hierarchy store requires indexes for:

- `(session_id, agent_id)` unique lookup.
- `(session_id, parent_id)` direct-child listing.
- `(session_id, status)` cleanup and recovery.

The store may retain complete hierarchy data for trusted runtime bookkeeping,
but callers receive projection types such as `SelfView`, `DirectChildView`, and
`LocalAgentView`, not `AgentRecord`.

## Local View and Authorization

`LocalAgentView` contains the caller's own status plus direct-child projections:

```rust
pub struct LocalAgentView {
    pub self_view: SelfView,
    pub children: Vec<DirectChildView>,
}
```

Neither projection includes `parent_id`, descendant counts, descendant IDs, or
links to arbitrary agents. A direct child may expose a boolean such as
`has_children` only to a trusted human UI through a navigation capability; it
must not be included in model context unless the product explicitly needs that
hint. The default model-facing projection omits it.

Authorization matrix:

| Operation | Self | Direct child | Parent | Sibling | Grandchild | Other branch |
|---|---:|---:|---:|---:|---:|---:|
| Read status/summary | allow | allow | deny | deny | deny | deny |
| List children | allow for caller's own children | deny through target ID | deny | deny | deny | deny |
| Read transcript | allow | allow | deny | deny | deny | deny |
| Read final result | allow | allow | deny | deny | deny | deny |
| Cancel | allow for current scope | allow | deny | deny | deny | deny |
| Spawn child | from current scope | not on behalf of target | deny | deny | deny | deny |

"Cancel self" means requesting cancellation of the caller's own scope. The
coordinator propagates that request to descendants. An agent cannot impersonate
a direct child and spawn grandchildren on the child's behalf.

All denied target operations return:

```rust
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("agent is not visible from the current execution scope")]
    NotVisible,
}
```

The complete enum also contains the operational categories specified in the
Error Handling section.

Logs may record the internal denial reason and target ID for operators, but the
model-facing or untrusted API response must not distinguish absent, cross-session,
or outside-scope targets.

## Lifecycle and Structured Concurrency

### States

The lifecycle state machine is:

```text
Pending -> Running -> WaitingForChildren -> Finalizing -> Completed
Pending/Running/WaitingForChildren/Finalizing -> Cancelling -> Failed
                                                        \-> Cancelled
```

`Failed`, `Cancelled`, and `Completed` are terminal. A child is considered live
in every other state. `Cancelling` is the common subtree-teardown state for both
failure and explicit cancellation; the requested terminal outcome determines
whether it transitions to `Failed` or `Cancelled` after all children terminate.

`WaitingForChildren` means the agent's model loop has produced its candidate
result but one or more direct children are still live. `Finalizing` means all
direct children are terminal and the runtime is aggregating their summaries,
saving transcripts, releasing permits, and publishing the parent's terminal
record.

The transition to any terminal state is rejected unless every direct child is
terminal. This check happens in the coordinator transaction, not only in the
caller.

### Join policies

The default policy is `AllRequired`:

- Wait for every direct child.
- If any child fails or is cancelled, the parent receives that terminal result
  and decides whether its own work can still succeed.
- The parent cannot silently omit a live child.

Optional policies may be added without weakening scope ownership:

- `BestEffort`: wait for all direct children and aggregate successful and failed
  summaries.
- `FailFast`: on the first required-child failure, cancel remaining live direct
  children, wait for their cancellation, then return failure.

Every policy reaches a state where all direct children are terminal before the
parent becomes terminal.

### Why a child can currently outlive its parent

The current `background: true` path starts the child with `tokio::spawn` and
returns a task identifier immediately. The parent loop therefore receives a
successful tool result and is free to end its turn; the spawned task is owned by
the Tokio runtime, not by a parent agent scope. The current progress tree merely
observes that detached lifetime.

This design changes the meaning of `background: true` to **asynchronous within
the parent scope**. The parent may continue reasoning or start sibling work, but
its finalization guard waits for or cancels all scoped children before the parent
reaches a terminal state.

Work that must survive its initiating agent is not a subagent. It must be
submitted to a separate `BackgroundJob` subsystem with explicit ownership,
persistence, cancellation, result retrieval, and retention semantics.

### Cancellation

Cancellation proceeds top-down and completion is observed bottom-up:

1. Mark the target scope `Cancelling` so no new children can be spawned.
2. Cancel the target's derived cancellation token.
3. Recursively signal all live direct children.
4. Await child task handles and terminal persistence with a bounded shutdown
   timeout.
5. Mark children terminal, release their semaphore permits, then mark the target
   `Cancelled`.

If a child ignores cooperative cancellation, the runtime aborts its task after
the shutdown timeout, records an internal forced-abort reason, and still performs
terminal cleanup. User-facing summaries report cancellation without exposing
hidden descendants.

A parent failure follows the same subtree-cancellation procedure before the
parent is persisted as `Failed`.

### Crash recovery

On daemon restart, records left in non-terminal states cannot be resumed as live
subagents without their task handles and cancellation scopes. Recovery marks the
affected subtree as cancelled with an internal `runtime_restarted` reason. It
must not reconstruct children as detached tasks.

## Result and Information Flow

Children return a bounded `ChildResult` to their direct parent:

```rust
pub struct ChildResult {
    pub child_id: AgentId,
    pub status: ChildTerminalStatus,
    pub summary: String,
    pub error_code: Option<String>,
    pub partial_result: Option<String>,
}
```

The parent receives only results for its direct children. `summary` and
`partial_result` must not include descendant IDs, raw descendant transcripts, or
serialized tree structures. The child is responsible for synthesizing any
descendant work into its own result before returning it upward.

Large successful or partial results continue to use the existing mailbox or an
equivalent bounded delivery mechanism. Authorization to retrieve an offloaded
result is bound to the parent-child relation and expires with the result handle.

Prompts, logs, and error strings are untrusted content. The runtime must not
parse identity or authorization claims from them.

## Tool Execution Context

Identity-sensitive tools need a trusted context without forcing an immediate
rewrite of every `Tool` implementation. Introduce:

```rust
pub struct ToolContext<'a> {
    pub agent: &'a AgentExecutionContext,
    pub invocation_id: ToolInvocationId,
}
```

The registry gains a contextual execution path. During migration, an adapter
calls the existing `Tool::execute(input)` for context-free tools. The `task`,
subagent trace/transcript, local-agent list/get, result retrieval, and
cancellation tools must use the contextual path before hierarchical spawning is
enabled.

The model-visible JSON schema must not advertise reserved identity fields. The
daemon and TUI must stop injecting `_session_id` into arguments once the trusted
context path is wired. `_subagent_depth` is replaced by `context.agent.depth`.

Depth enforcement occurs in `AgentCoordinator::spawn_child`. Tool filtering may
still hide `task` at the limit as a usability optimization, but it is not the
enforcement boundary.

## Daemon and Capability API

The current endpoint that returns a complete session progress map is replaced
for agent navigation by local endpoints:

```text
GET  /api/v1/agents/self
GET  /api/v1/agents/children
GET  /api/v1/agents/children/{capability}
GET  /api/v1/agents/children/{capability}/transcript
POST /api/v1/agents/children/{capability}/cancel
```

Requests authenticate an execution scope or a trusted UI session. Raw agent IDs
are not sufficient authority.

For a trusted human UI, each direct-child projection may include an opaque,
short-lived navigation capability. The capability is bound to:

- Session ID.
- Target agent ID.
- Target generation.
- Allowed operation set.
- Expiration time.
- Issuing viewer identity or trusted UI session.

Opening a child in the TUI exchanges that capability for the child's local
view. The new view contains the selected child and only its direct children. A
subsequent descent requires a capability issued by that local view.

Capabilities are bearer secrets and must not be placed in model messages,
transcripts, tool outputs, or ordinary logs. TUI state may hold them in memory.
Going back in the UI uses client-side navigation history; it does not grant the
currently selected agent visibility of its ancestor.

The existing full-session progress endpoint may temporarily remain behind a
trusted operator-only boundary during migration. It must not be callable from
model tools or treated as a normal client API, and it is removed after scoped
endpoints are adopted.

## TUI Behavior

The TUI presents a local tree rather than the whole forest:

- The initial view shows the root agent and its direct children.
- Selecting a child and pressing the existing focus action opens that child's
  local view through its navigation capability.
- The next screen shows that child and only its direct children.
- Back navigation restores the previous trusted UI screen from navigation
  history.
- Search, selection, counts, and completion summaries operate only on the
  currently loaded local view.

`SubagentTree::real_node_list()` cannot be used as an agent selector over a
session-wide store. It may remain as a rendering helper only when its input has
already been scoped by the server/runtime.

The UI may show breadcrumb labels to the human operator, but breadcrumbs and
ancestor metadata are UI-owned state and must not be copied into the focused
agent's model context.

## Storage and Transcript Access

Progress and transcript storage use the canonical `(session_id, agent_id,
parent_id)` relation. Reads are performed through repository methods that accept
an authorization scope; callers do not fetch a full map and filter it afterward.

Required repository operations include:

- Read self projection by authenticated caller ID.
- List records where `parent_id == caller.agent_id` in the same session.
- Read a transcript only after self/direct-child authorization.
- Persist terminal state and child summaries atomically with lifecycle checks.
- Enumerate descendants only for internal cancellation and recovery, never for
  an agent-facing read.

Historical local views follow the same rules. Completion does not broaden
visibility, and transcript retention does not turn old agents into globally
listable records.

## Error Handling

Coordinator errors use `thiserror` and stable internal categories. Expected
categories include:

- `NotVisible` for absent or unauthorized targets.
- `DepthLimitReached` when `spawn_child` exceeds configured depth.
- `ConcurrencyClosed` when the coordinator is shutting down.
- `ParentNotRunning` when a terminal or cancelling parent tries to spawn.
- `JoinFailed` when scoped task cleanup cannot complete normally.
- `Storage` with actionable context for persistence failures.

Agent-facing messages remain concise and do not reveal hidden topology. Runtime
and daemon layers add operational context with `anyhow::Context` at boundaries.

The existing `SubagentError` remains the child execution failure payload.
`CoordinatorError` covers orchestration and authorization failures. Converting
between them must preserve stable error codes and any permitted partial result.

## Migration Plan

### Phase 1: Trusted identity and local authorization

1. Add typed IDs, `AgentExecutionContext`, `ToolContext`, and coordinator error
   types.
2. Add a contextual tool-execution adapter without changing context-free tools.
3. Move `task` depth and session reads from JSON fields to trusted context.
4. Introduce canonical agent records and self/direct-child repository queries.
5. Add authorization contract tests before exposing new APIs.

At the end of this phase, identity-sensitive operations no longer trust model
input, even if old progress rendering remains in place.

### Phase 2: Coordinator and structured concurrency

1. Route ordinary `task` and RLM child creation through `AgentCoordinator`.
2. Replace `AtomicUsize` polling with a semaphore.
3. Add lifecycle states, child handles, join policies, and finalization guards.
4. Redefine `background: true` as scoped asynchronous execution.
5. Implement recursive cancellation, forced-abort cleanup, and restart recovery.

At the end of this phase, no subagent can outlive its parent scope.

### Phase 3: Scoped daemon APIs and TUI navigation

1. Add self, direct-child, transcript, result, and cancellation endpoints.
2. Add opaque navigation capabilities for the trusted TUI.
3. Change TUI state and focus selection to consume only local projections.
4. Remove model/tool access to the full-session progress endpoint.
5. Preserve human back navigation without adding ancestor data to model context.

At the end of this phase, clients no longer need a whole-session agent tree.

### Phase 4: Consolidation and compatibility cleanup

1. Migrate or retire flat `AgentsService` session listing in favor of coordinator
   records.
2. Remove `_session_id`, `_agent_id`, `_parent_id`, and `_subagent_depth`
   compatibility handling from identity-sensitive paths.
3. Remove the unscoped progress endpoint after all consumers migrate.
4. Update README and `WGENTY.md` claims about depth propagation, background
   execution, and hierarchy visibility.
5. Add a separate `BackgroundJob` design before supporting truly detached work.

## Testing Strategy

### Unit tests

- Context-derived parent and depth override forged JSON fields.
- Root and nested agents receive the same self/direct-child visibility policy.
- Self and direct-child reads succeed.
- Parent, sibling, grandchild, other-branch, cross-session, and nonexistent reads
  all return `NotVisible` where target addressing is involved.
- Transcript, result, cancellation, and spawn-on-behalf checks use the same
  authorization predicate.
- Semaphore permits are released on success, failure, cancellation, timeout,
  panic/abort cleanup, and persistence error paths.
- Parent completion is rejected while any direct child is non-terminal.
- `AllRequired`, `BestEffort`, and `FailFast` leave no live direct children.
- Child results exclude descendant IDs and transcript structures.
- Capabilities reject wrong session, target, generation, operation, viewer, or
  expiration.

### Integration tests

- Build a three-level tree and assert that each level sees exactly itself and
  its direct children.
- Attempt visibility escalation with forged reserved fields and raw IDs.
- Complete a parent model loop while a background child is running and verify
  the parent enters `WaitingForChildren`, not `Completed`.
- Fail and cancel a parent with running descendants and verify bottom-up terminal
  cleanup with no orphan records or leaked permits.
- Restart the daemon with non-terminal records and verify deterministic subtree
  cancellation.
- Navigate root to child to grandchild in the TUI and verify every server
  response is locally scoped while back navigation still works.
- Verify model messages for a focused child contain no ancestor, sibling, or
  other-branch identifiers or transcripts.
- Verify unauthorized and nonexistent daemon targets have indistinguishable
  external responses.

### Regression tests

- Existing progress updates, structured errors, partial results, mailbox
  delivery, and transcript persistence continue to work within local scope.
- Maximum depth and maximum concurrency settings remain enforced.
- Synchronous subagents preserve current result behavior.
- RLM and ordinary `task` paths produce identical parent/depth semantics.

### Performance checks

- Local child listing uses the parent index and does not scan a session-wide
  tree.
- Capability verification is constant time apart from storage lookup.
- The semaphore does not introduce polling wakeups.
- Release build startup, memory, and binary size remain within repository
  performance constraints.

## Acceptance Criteria

The change is complete when all of the following are true:

1. No identity-sensitive tool or daemon handler trusts model-supplied agent,
   parent, session, or depth fields.
2. Every agent-facing list/get/transcript/result/cancel operation returns only
   self or direct-child data.
3. Root agents have no global-view exception.
4. Parent, sibling, grandchild, other-branch, cross-session, and nonexistent
   targets cannot be distinguished or accessed by an agent.
5. Ordinary task and RLM creation both derive parent and depth from the caller's
   trusted context.
6. A parent cannot reach `Completed`, `Failed`, or `Cancelled` while a scoped
   child remains non-terminal.
7. Parent failure and cancellation recursively terminate the subtree and release
   all concurrency permits.
8. `background: true` children may run concurrently with their parent but never
   outlive the parent scope.
9. The TUI can navigate hierarchy one level at a time without sending ancestor
   or other-branch data into model context.
10. Full-session progress data is unavailable to model tools and ordinary
    clients.
11. Existing structured errors and partial-result delivery remain functional.
12. README and architecture documentation describe the implemented semantics
    accurately.

## Consequences and Trade-offs

- Strict local views reduce accidental context leakage and make authorization
  reasoning compositional: every agent follows the same rule.
- The coordinator becomes critical infrastructure and requires careful recovery,
  lifecycle, and storage testing.
- Structured concurrency may make a parent turn appear longer because it cannot
  finish while scoped asynchronous work is live. This is intentional; the prior
  shorter lifetime represented detached work rather than completed dependency
  handling.
- Layer-by-layer TUI navigation requires capabilities and navigation state, but
  avoids shipping the whole tree to every client.
- Separating `BackgroundJob` from subagents creates a clearer ownership model at
  the cost of a future dedicated subsystem for truly detached work.

## Documentation Corrections

During implementation, documentation must stop claiming that
`_subagent_depth` itself provides reliable depth propagation. The correct claim
is that the runtime derives depth from `AgentExecutionContext` and the
coordinator enforces `agent.subagent.max_depth`.

Documentation about background subagents must also change from "the parent turn
returns while the subagent continues" to "the parent may continue concurrently,
but cannot terminate until scoped children are terminal." Existing reports that
describe the old behavior remain historical records and should not be rewritten.
