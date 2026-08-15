## Purpose

Single multiplexed WebSocket connection carrying all daemon push channels (heartbeat, subagent trace events, global events, per-session run events) so browser clients are not constrained by the per-origin HTTP/1.1 connection budget that exhausts under SSE fan-out.

## ADDED Requirements

### Requirement: Single-connection message envelope

The system SHALL provide a WebSocket endpoint that multiplexes all push message types over one connection. Every server-to-client message SHALL be a JSON envelope carrying a `type` field distinguishing at least: heartbeat, trace event, global event, and session event. Server-to-client envelopes for session events SHALL carry the session id and the event's sequence number.

#### Scenario: All push types on one connection

- **WHEN** a client holds one authenticated WebSocket connection while a subagent runs and a todo list changes
- **THEN** the client receives trace events, global events, and heartbeat messages as typed envelopes on that single connection, with no additional transport connections opened

#### Scenario: Sequence numbers preserved

- **WHEN** a session event envelope arrives after a reconnect
- **THEN** the envelope's sequence number allows the client to drop duplicates and resume from its last seen cursor

### Requirement: Session event subscription

The system SHALL let a client subscribe and unsubscribe to a session's run-event stream via control messages on the shared connection (`subscribe` carrying the session id and an optional `after` sequence cursor; `unsubscribe` carrying the session id). A client MAY hold multiple concurrent session subscriptions. Unsubscribing or dropping the connection SHALL stop delivery for those sessions without affecting other subscriptions or other clients.

#### Scenario: Subscribe with cursor resumes missed events

- **WHEN** a client sends `subscribe` for session S with `after` = last seen sequence N
- **THEN** the server first delivers session S events with sequence greater than N, then continues with live events

#### Scenario: Unsubscribe stops only that session

- **WHEN** a client sends `unsubscribe` for session S while holding a subscription for session T
- **THEN** session S events stop arriving while session T events continue

#### Scenario: Disconnect releases subscriptions

- **WHEN** a client disconnects without explicit unsubscribe
- **THEN** the server drops that connection's session subscriptions; global and trace delivery to other clients is unaffected

#### Scenario: Subscription table bounded

- **WHEN** a client holds subscriptions at the configured per-connection limit and sends another `subscribe`
- **THEN** the server rejects the additional subscription with an error envelope and keeps the connection and existing subscriptions intact

### Requirement: WebSocket handshake authentication

The WebSocket upgrade request SHALL be rejected with the same authentication failure behavior as other protected daemon routes when credentials are missing or invalid, and SHALL accept the existing daemon bearer token via a mechanism available to browser WebSocket APIs (query parameter, subprotocol header, or first-message authentication).

#### Scenario: Unauthenticated upgrade rejected

- **WHEN** a client attempts a WebSocket upgrade without valid credentials
- **THEN** the connection is rejected with the same auth failure semantics as other protected routes

#### Scenario: First-message authentication

- **WHEN** the chosen mechanism is first-message authentication and the first frame does not carry valid credentials within a short timeout
- **THEN** the server closes the connection without delivering any push messages

#### Scenario: Token rotation forces reconnect

- **WHEN** the daemon restarts and rotates the token while a WebSocket connection is open
- **THEN** the server closes remaining connections with close code 4001 (token invalid/rotated), and the client refreshes its token and reconnects

### Requirement: Reconnection and replay recovery

A client that reconnects after a drop SHALL be able to restore full state: resubscribe with per-session `after` cursors, request trace replay for a session since a timestamp, and rely on global-event sequence numbers to realign. The server SHALL not deliver session events predating the requested cursor.

#### Scenario: Reconnect restores subscriptions

- **WHEN** the connection drops mid-run and the client reconnects
- **THEN** the client resubscribes with its last seen cursors and receives the events missed during the gap without duplicates earlier than those cursors

### Requirement: Idle shutdown accounting

The daemon's thin-client idle shutdown timer SHALL count an authenticated WebSocket connection as an active client for as long as the connection is open, so a web client keeping only a WebSocket open prevents idle shutdown exactly as an SSE-holding client does.

#### Scenario: WebSocket keeps daemon alive

- **WHEN** a web client holds an authenticated WebSocket connection and performs no other requests
- **THEN** the daemon's idle shutdown deadline is continuously deferred

### Requirement: SSE endpoints remain available

During the WebSocket transition the existing SSE endpoints (heartbeat, trace stream, global events, per-session events) SHALL remain functional and independently usable, so non-browser or older clients are unaffected.

#### Scenario: Legacy SSE client coexists

- **WHEN** one client consumes SSE endpoints while another uses the WebSocket channel
- **THEN** both receive equivalent event streams and neither connection mode disturbs the other
