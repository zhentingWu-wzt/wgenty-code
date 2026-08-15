## MODIFIED Requirements

### Requirement: Daemon SSE trace streaming endpoint

When the `daemon` feature is enabled and the daemon is running, the system SHALL expose `GET /api/v1/subagents/trace/stream` returning a Server-Sent Events stream of subagent trace events, protected by the existing daemon auth middleware. The endpoint SHALL accept optional `session_id` (filter to one session; omitted = global live stream) and `since` (skip events at or before this Unix-ms timestamp) parameters. Slow SSE subscribers observe event drops per the bounded broadcast channel; file persistence is unaffected.

The same trace events SHALL additionally be deliverable over the WebSocket push channel as trace envelopes: a global live stream by default, with session filtering applied by the client. SSE and WebSocket consumers MUST observe equivalent event streams with identical redaction.

#### Scenario: Authenticated live subscription

- **WHEN** an authenticated client connects to the SSE endpoint
- **THEN** subsequent subagent trace events SHALL be pushed to the client in real time as SSE `data:` frames

#### Scenario: Unauthenticated request rejected

- **WHEN** a client connects without valid auth credentials
- **THEN** the endpoint SHALL reject the request with the same auth failure behavior as other protected daemon routes

#### Scenario: Session-filtered stream

- **WHEN** a client connects with `?session_id=<id>`
- **THEN** only trace events for that session SHALL be pushed

#### Scenario: WebSocket trace envelopes equivalent to SSE

- **WHEN** one client consumes trace events over the WebSocket push channel while another consumes the SSE endpoint
- **THEN** both observe the same events with the same redaction applied
