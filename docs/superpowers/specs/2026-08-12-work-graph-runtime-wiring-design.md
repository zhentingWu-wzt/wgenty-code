# Production Work-Graph Runtime Wiring Design

## Goal

Make the already implemented static Work-Graph reachable in normal daemon and
headless execution without sharing state between user sessions. Until this is
true, additional specialist roles and dynamic construction are only dormant
contracts rather than a usable Coding-Agent capability.

## Current Gap

`NodeRuntime`, `VerifyGate`, and the node tools are currently built in unit
tests, but normal `DaemonState` constructs an `AgentCoordinator` and global
`ToolRegistry` without an `ExecutionSession::SessionCoordinator`. The headless
runtime does the same. A global `BeginNodeTool` cannot safely own one session
coordinator because the registry serves multiple trusted `ToolContext` session
ids.

## Design

Introduce `ExecutionSessionRuntimeStore` in `exec_session`. It owns immutable
runtime dependencies (project root, checkpoint store, retry limit) and maps a
trusted `SessionId` to one `Arc<NodeRuntime>`. On first access it creates a
`SessionCoordinator` with `SessionSource::AgentSelf`, a ProcessCommandExecutor,
and the default hooks. Concurrent callers use a single map mutex so the same
session id receives exactly one runtime.

The store is never indexed by model JSON. Context-aware tool adapters resolve
only `context.agent.session_id`; root and child agents in the same trusted
agent session intentionally share the same runtime, while different sessions
receive distinct checkpoint/session records.

`BeginNodeTool`, `VerifyNodeTool`, and `RollbackNodeTool` become store-backed
adapters. Their context-free `execute` path fails closed, preventing callers
from accidentally creating an unscoped graph. Their contextual path resolves
the session runtime and retains the existing input/output schema.

Before `begin_node` creates the first node, the adapter ensures the runtime has
an active turn. This is idempotent for a running graph session: it starts a
turn only when none exists. The normal agent-loop end-of-turn hook remains a
future improvement; this scope guarantees that every reachable graph node has
the active turn/checkpoint that `NodeRuntime` already requires.

`ToolRegistry::register_exec_session_tools` accepts an
`ExecutionSessionRuntimeStore` instead of a per-session coordinator and
registers the four adapters (legacy `verify_and_complete` is retained as a
store-backed adapter too). Daemon startup creates one shared store from its
project-root checkpoint store and registers it within the global registry.
Headless startup creates and registers the same type before constructing its
context-aware `RegistryToolPort`.

## Invariants

- A tool can never route graph work by an untrusted session identifier.
- One trusted agent session maps to exactly one runtime and persisted
  `.wgenty-code/snapshots/<session-id>/session.json` record.
- Two agent sessions never share current node, WorkState, audit events, or
  checkpoints.
- The first `begin_node` owns an active graph turn; later begin/verify/rollback
  calls reuse it.
- Compile/test/verify commands remain selected by persisted verification
  profiles and evaluated through actual external anchors.
- The registry does not retain an agent coordinator; specialist handoff wiring
  follows only after this session-runtime boundary is available.

## Evaluation

1. A contextual `begin_node` followed by `verify_node` runs the Rust profile
   commands in order and persists the graph audit under the caller session id.
2. Context-free calls fail with `missing_tool_context` and create no snapshot.
3. Two trusted roots with different session ids receive different runtime
   pointers and independent persisted node/audit state.
4. Two concurrent requests using the same session id get one runtime and do
   not race turn creation.
5. Daemon construction registers all node tools, and a daemon handler test can
   exercise begin/verify using its trusted root context.
6. Headless construction exposes the same node tools through
   `RegistryToolPort`.

## Non-Goals

This change does not automatically decide when to start a graph, run
specialists, replace normal task-group delivery, or create dynamic work graphs.
It is a precondition for all of those steps because it gives each active agent
session a durable, isolated graph state owner.
