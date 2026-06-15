# subagent-content-preview Specification

## Purpose
TBD - created by archiving change improve-subagent-progress-visibility. Update Purpose after archive.
## Requirements
### Requirement: SubagentProgress records tool call action log
The `SubagentProgress` struct SHALL include an `action_log: Vec<SubagentEvent>` field that records all tool calls and thoughts made by the subagent, each containing the event type, tool name / text, and a summary of key parameters. Tool results SHALL NOT be included in the action log during execution; they SHALL be stored separately in the SQLite transcript.

#### Scenario: Tool call started
- **WHEN** a subagent begins executing a tool call
- **THEN** a `SubagentEvent` with `event_type: Action { tool_name, params_summary }` SHALL be appended to the action log in the next progress event

#### Scenario: Action log preserves complete history for transcript
- **WHEN** the action log accumulates entries during subagent execution
- **THEN** all entries SHALL be preserved in memory until the subagent completes; the log SHALL NOT be truncated before writing to SQLite

#### Scenario: Action log written to SQLite on completion
- **WHEN** a new progress event is emitted for the final subagent status (Completed, Failed, Cancelled)
- **THEN** the complete action log SHALL be persisted to the SQLite transcript via `SubagentTranscriptStore`

### Requirement: SubagentProgress captures current tool parameters
The `SubagentProgress` struct SHALL include a `current_params: Option<String>` field that describes the key parameters of the currently executing tool, so the TUI can display not just the tool name but what it's operating on.

#### Scenario: Tool with file path parameter
- **WHEN** the subagent calls `file_read` with `file_path = "src/auth.rs"`
- **THEN** `current_params` SHALL be `Some("src/auth.rs")` and `current_tool` SHALL be `Some("file_read")`

#### Scenario: Tool with no meaningful params to summarize
- **WHEN** the subagent calls a tool with no extractable params
- **THEN** `current_params` SHALL be `None` and the TUI SHALL display just the tool name

### Requirement: Subagent text snapshots are captured during execution
The subagent execution loop SHALL capture the full assistant text response after each round and include a truncated snapshot in `SubagentProgress`. The full text SHALL be stored in the SQLite transcript for later retrieval.

#### Scenario: Subagent completes first round with text output
- **WHEN** a subagent finishes its first API call and produces a text response before any tool call
- **THEN** the emitted `SubagentProgress` SHALL include `text_snapshot` containing up to the last 200 characters of that response; the full text SHALL be queued for SQLite storage

#### Scenario: Subagent produces only tool calls with no text
- **WHEN** a subagent finishes a round with only tool calls and no assistant text
- **THEN** the emitted `SubagentProgress` SHALL have `text_snapshot` as `None` or empty

#### Scenario: Text snapshot is truncated for inline display
- **WHEN** the assistant text response exceeds 200 characters
- **THEN** the `text_snapshot` SHALL be truncated to the last 200 characters; the full text SHALL be available in the SQLite transcript

#### Scenario: Completed subagent archives full transcript
- **WHEN** a subagent reaches Completed status
- **THEN** the full action log, all text responses, and metadata SHALL be persisted to SQLite; the text snapshot in memory MAY be cleared

### Requirement: SubagentProgress includes token consumption
The `SubagentProgress.metadata.token_count` field SHALL be populated with the cumulative token usage from all API calls made by the subagent, reported on completion and at periodic intervals during execution.

#### Scenario: Subagent completes with known token usage
- **WHEN** a subagent completes after 3 API rounds consuming 500 input + 300 output tokens total
- **THEN** the final `SubagentProgress` event with status `Completed` SHALL have `metadata.token_count = Some(800)`

#### Scenario: Token counts unavailable from provider
- **WHEN** the API provider response does not include token usage information
- **THEN** `metadata.token_count` SHALL remain `None`

#### Scenario: Per-round token update during execution
- **WHEN** a subagent is Running and has completed at least one API round with token usage data
- **THEN** each progress event SHALL include the cumulative `token_count` in metadata to show live token consumption

### Requirement: Daemon progress store is session-scoped
The daemon's subagent progress storage SHALL be scoped by session ID so that concurrent sessions do not cross-contaminate progress data.

#### Scenario: Two concurrent sessions with subagents
- **WHEN** session A runs 2 subagents and session B runs 1 subagent concurrently
- **THEN** polling progress for session A SHALL return only session A's 2 subagent nodes
- **THEN** polling progress for session B SHALL return only session B's 1 subagent node

#### Scenario: Session disconnection cleans up progress
- **WHEN** a session disconnects or its progress poller stops
- **THEN** the session's progress entries SHALL be removed from the daemon store within a reasonable timeout (e.g., 60 seconds of no polling)

