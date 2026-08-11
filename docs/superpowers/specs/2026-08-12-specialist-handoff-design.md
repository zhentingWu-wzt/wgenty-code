# Specialist Sub-Agent Handoff Design

## Goal

Give the static Work-Graph a real, typed data plane for specialist sub-agents
before a static template or a future dynamic selector is allowed to use them.
The first usable specialists are `explore`, `root-cause`, and `plan`; their
findings are persisted in `WorkState`, not delivered as an unstructured
conversation between agents.

## Context and Decision

The repository already has an Org-Graph registry and a coordinator capable of
running typed child agents, while `ExecutionSession` owns the checkpointed
`WorkState` and static external-anchor loop. These two paths are presently
disconnected: a child emits free text into a task-group result, and the
Work-Graph cannot consume it as governed state.

This change connects only the handoff boundary. It deliberately does not make
an LLM choose graph edges and does not yet auto-spawn a specialist from a
Work-Graph. The following change will install the fixed diagnostic template on
top of this boundary; a later policy-constrained selector may choose among
templates.

## Alternatives Considered

1. Store each specialist's final text in a generic string field. This is the
   smallest implementation but recreates state drift: downstream nodes would
   parse arbitrary prose and cannot reliably distinguish evidence from advice.
2. Allow parent and child agents to exchange a task-group result, then let the
   parent summarize it into `WorkState`. This keeps the existing mechanism but
   permits a model to silently omit or rewrite evidence.
3. **Recommended: typed, child-authenticated report submission.** A specialist
   calls a context-aware tool which verifies its trusted `NodeType`, validates a
   report schema, persists it through the existing checkpoint, and records the
   normal `WorkState` field audit. A later graph runner reads the typed report
   directly. The normal task tool remains backward compatible outside a graph.

## Data Model

`WorkState` receives a serde-defaulted `specialist_reports: Vec<SpecialistReport>`
field and a `WorkField::SpecialistReports` permission entry. A report contains:

- `producer`: the trusted emitting `NodeType`, persisted independently of input;
- `kind`: `exploration`, `root_cause`, or `implementation_plan`;
- `summary`: a bounded statement of the conclusion;
- `evidence`: a non-empty list of `{ path, detail }` observations;
- `suspected_files`: deduplicated source paths; and
- `recommended_actions`: ordered, non-empty next actions.

The state API accepts `producer` as a separate trusted parameter and rejects a
report whose declared producer differs from it. The new `RootCause` node type
is a leaf, read-only specialist. Existing `Explore` and `Plan` contracts are
also leaf reporters. `GeneralPurpose`, `Verification`, and guide nodes cannot
write specialist reports. `RootCause` can read requirement, generated diff,
compile/test results, and prior reports so a retry can diagnose real anchor
failure rather than inventing a cause.

Reports are task products, so `reset_for_new_node` and
`reset_for_work_graph_pass` clear them. `inherit_for_new_turn` retains them,
matching requirement inheritance and preserving checkpoint resume semantics.

## Trusted Submission Tool

`submit_specialist_report` is a mutating, context-aware daemon built-in tool.
It is registered only where the ExecutionSession runtime store and daemon
`AgentCoordinator` are both available. It receives the actual `ToolContext`
and never accepts `node_type`, session id, node id, or turn id from model
input.

The tool obtains the caller type from `AgentCoordinator`, rejects root,
terminal, and unauthorized callers, requires an active turn and persisted
current node, parses the typed report, persists it through `WorkState`, then
checkpoints. A persisted report therefore has a durable node/turn scope and an
ordinary field-level audit record. Failed parsing or unauthorized writing
changes neither state nor its checkpoint.

The associated specialist prompts instruct the agent to submit exactly one
report before its final response. The tool is a handoff endpoint, not a
verification endpoint: it cannot write anchors, routing events, budgets, or
acceptance status.

## Static and Dynamic Graph Boundaries

The immediate static diagnostic template will use a root-cause report as an
input product before an implementer runs, then retain the existing code-owned
compile → test → verify anchors. It will invoke only a predeclared leaf
specialist and will consume the persisted report rather than its final text.
The template itself, specialist budget, edge conditions, and retry limit remain
code-owned.

Dynamic construction is explicitly deferred until static templates demonstrate
that report production, checkpoint recovery, external anchors, and global
budgets work together. The dynamic selector will choose from registered
templates and roles; it will not create arbitrary node types or bypass the
handoff tool.

## Compatibility and Safety

- Old checkpoints deserialize because `specialist_reports` has `#[serde(default)]`.
- Existing `task` calls retain their normal task-group result behavior; the new
  endpoint only establishes the safe state path for graph-specific callers.
- The tool's `is_read_only()` is `false`; all specialist source inspection is
  still constrained through existing contract tool filtering.
- Submission authorization depends on trusted runtime context, never a JSON
  `subagent_type` string or a prompt claim.
- No report can cause a route change by itself. Static edge code may later
  inspect validated state fields, and must still defer acceptance to anchors.

## Evaluation

Unit and integration coverage will prove:

1. each report kind serde-round-trips and legacy checkpoints deserialize;
2. only authorised specialist node types can append reports;
3. producer spoofing, invalid evidence, duplicate files, and empty actions are
   rejected without mutation;
4. pass reset clears reports while new-turn inheritance and checkpoint restore
   retain them;
5. context-aware tool calls accept a trusted root-cause child and reject a root
   or verification caller; and
6. the Org-Graph render and task dispatcher expose `root-cause` as a stable,
   leaf, read-only node contract.
