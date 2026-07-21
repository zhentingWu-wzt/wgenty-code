## ADDED Requirements

### Requirement: Local JSONL trace file sink
The system SHALL, when `subagent.trace.sink` is `file` or `both`, append each subagent progress event as a JSONL line to `<subagent.trace.dir>/<session_id>.jsonl` (default `<project>/.wgenty-code/traces/<session_id>.jsonl`). The sink SHALL be driven by the existing `ProgressCallback` and SHALL create the directory with restrictive permissions (0600 file, 0700 dir) on first write.

#### Scenario: Trace events appended as JSONL
- **WHEN** a subagent emits progress events during execution
- **THEN** each event SHALL be serialized as one JSON object per line and appended to the session's trace file

#### Scenario: Sensitive parameters redacted in trace file
- **WHEN** a progress event contains tool parameters with sensitive keys
- **THEN** those values SHALL be redacted before being written to the JSONL file

#### Scenario: Sink disabled by config
- **WHEN** `subagent.trace.sink` is `off`
- **THEN** no trace file SHALL be written and the progress callback SHALL skip the file sink

### Requirement: Daemon SSE trace streaming endpoint
When the `daemon` feature is enabled and the daemon is running, the system SHALL expose `GET /api/v1/subagents/trace/stream` returning a Server-Sent Events stream of subagent trace events, protected by the existing daemon auth middleware. The endpoint SHALL accept optional `session_id` (filter to one session) and `since` (event cursor) query parameters.

#### Scenario: Authenticated live subscription
- **WHEN** an authenticated client connects to the SSE endpoint
- **THEN** subsequent subagent trace events SHALL be pushed to the client in real time as SSE `data:` frames

#### Scenario: Unauthenticated request rejected
- **WHEN** a client connects without valid auth credentials
- **THEN** the endpoint SHALL reject the request with the same auth failure behavior as other protected daemon routes

#### Scenario: Session-filtered stream
- **WHEN** a client connects with `?session_id=<id>`
- **THEN** only trace events for that session SHALL be pushed

### Requirement: Daemon SSE cold-start replay
When a client connects to the SSE endpoint with a `since` cursor or for a known session, the system SHALL replay persisted history from the transcript store before streaming live events, so late subscribers do not lose prior events.

#### Scenario: Late subscriber receives history then live
- **WHEN** a client connects after a subagent has already emitted events
- **THEN** the endpoint SHALL first replay persisted events for the session (or since the cursor) and then continue with live events

### Requirement: Broadcast channel bounded
The in-memory broadcast channel feeding the SSE endpoint SHALL have a bounded capacity; when full, the oldest events SHALL be dropped for live subscribers. Dropped events SHALL remain available via the persisted JSONL file and transcript store, so persistence is not affected by live-subscriber backpressure.

#### Scenario: Backpressure drops oldest for live subscribers only
- **WHEN** the broadcast channel is full and a new event arrives
- **THEN** the oldest buffered event SHALL be dropped from the live channel, but the event SHALL still be persisted to the JSONL file and transcript store
