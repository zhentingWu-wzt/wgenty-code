# server-interaction-tools Specification

## Purpose
TBD - created by archiving change ask-user-question-server. Update Purpose after archive.
## Requirements
### Requirement: Server-side loop drives interaction tools

The daemon's server-side agent loop SHALL support `ask_user_question` (and future interaction-class tools) by routing them through an `InteractionPort` instead of executing them via the normal ToolPort, so the tool blocks until a frontend supplies an answer.

#### Scenario: Agent calls ask_user_question during a server-side run

- **WHEN** the server-side loop encounters a tool call whose name is `ask_user_question`
- **THEN** the loop invokes `InteractionPort::ask_user_question(args)` (not ToolPort::execute), which blocks the turn until a client resolves the prompt
- **AND** the tool's serialized result (the user's answer) is fed back into the loop as the tool result

#### Scenario: No frontend connected

- **WHEN** `ask_user_question` is invoked and no client resolves the prompt within the bridge's timeout
- **THEN** the tool returns a denial/default (fail-closed), the loop continues, and no phantom prompt lingers (WaiterGuard cleans up)

### Requirement: Interaction prompt push via trace SSE

The daemon SHALL broadcast interaction prompts (and their resolutions) on the existing trace hub (`trace_hub`) so frontends already subscribed to `/api/v1/subagents/trace/stream` receive them without a new SSE connection or polling.

#### Scenario: Prompt pushed to subscribers

- **WHEN** an interaction prompt is registered (tool awaiting answer)
- **THEN** a `TraceEvent` with `kind: "question_pending"` and a `question` payload (request_id, question, options, multi_select) is published on the trace hub
- **AND** subscribers see it on the existing trace SSE stream

#### Scenario: Resolution pushed

- **WHEN** a prompt is resolved (answered, timed out, or the waiter dropped)
- **THEN** a `TraceEvent` with `kind: "question_resolved"` is published so frontends can dismiss the prompt

### Requirement: Interaction resolve endpoint

The daemon SHALL expose `POST /api/v1/interactions/:request_id/resolve` on the protected router to accept a client's answer and unblock the waiting tool.

#### Scenario: Resolve succeeds

- **WHEN** an authenticated client posts `{ "answer": "..." }` to a pending interaction's resolve endpoint
- **THEN** the waiting `InteractionPort::ask_user_question` future completes with the answer string, and the loop continues

#### Scenario: Resolve unknown or expired

- **WHEN** the request_id has no pending interaction (already resolved, timed out, or never existed)
- **THEN** the endpoint returns 404

