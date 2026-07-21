# subagent-transcript-storage Specification

## Purpose
TBD - created by archiving change rlm-observability-and-robustness. Update Purpose after archive.
## Requirements
### Requirement: Transcript database schema
The system SHALL maintain a SQLite database at `~/.wgenty-code/subagent_transcripts.db` with tables for transcript headers and per-round events. The `subagent_transcripts` table SHALL additionally include columns `failure_diagnostics` (JSON text, nullable), `root_cause` (text, nullable), and `retry_history` (JSON text, nullable) to persist structured failure diagnostics.

#### Scenario: Database created on first use
- **WHEN** the first subagent transcript is written and the database file does not exist
- **THEN** the system SHALL create the database file with the correct schema (including the new diagnostics columns) automatically

#### Scenario: Transcript header row written on subagent completion
- **WHEN** a subagent reaches Completed, Failed, or Cancelled status
- **THEN** a row SHALL be inserted into `subagent_transcripts` with id, session_id, parent_id, label, status, system_prompt, user_prompt, started_at, finished_at, total_tokens, error_message (if any), summary, and--on failure--`failure_diagnostics`, `root_cause`, and `retry_history`

#### Scenario: Events batch-written on subagent completion
- **WHEN** a subagent completes
- **THEN** all events (thought, action, tool_result, error) from the subagent's execution SHALL be inserted into `subagent_events` in a single transaction

#### Scenario: Idempotent migration adds diagnostics columns to existing databases
- **WHEN** the transcript store is opened on an existing database created before the diagnostics columns existed
- **THEN** the system SHALL add `failure_diagnostics`, `root_cause`, and `retry_history` columns via `ALTER TABLE ADD COLUMN` only if they are not already present (checked via `PRAGMA table_info`), without losing existing data

#### Scenario: Old rows degrade gracefully
- **WHEN** a transcript row predates the diagnostics columns (columns NULL)
- **THEN** reads SHALL surface `root_cause` as `Unknown` and empty `retry_history`/`failure_diagnostics` rather than erroring

### Requirement: Transcript store API
The `SubagentTranscriptStore` SHALL provide methods for listing, retrieving, and searching transcripts.

#### Scenario: List transcripts by session
- **WHEN** `list_by_session(session_id)` is called
- **THEN** all transcripts for that session SHALL be returned ordered by started_at descending

#### Scenario: Get transcript by ID with events
- **WHEN** `get_by_id(transcript_id)` is called
- **THEN** the transcript header and all associated events SHALL be returned

#### Scenario: Search transcripts by label substring
- **WHEN** `search(query)` is called with a text query
- **THEN** all transcripts whose label contains the query (case-insensitive) SHALL be returned, limited to 100 results

### Requirement: Transcript retention policy
Transcripts older than a configurable retention period SHALL be automatically deleted.

#### Scenario: Retention period honored
- **WHEN** a new transcript is written and `max_transcript_age_days` is set to 30
- **THEN** transcripts with `started_at` older than 30 days SHALL be deleted in the same transaction

#### Scenario: Unlimited retention
- **WHEN** `max_transcript_age_days` is set to 0
- **THEN** no automatic deletion SHALL occur

### Requirement: TUI transcript detail view
The TUI SHALL provide a full-screen transcript detail view accessible from the subagent panel.

#### Scenario: Open transcript detail for a node
- **WHEN** user presses Enter on a Completed or Failed node in the subagent panel
- **THEN** a full-screen view SHALL open showing the transcript header (status, timing, token count) and the full event timeline

#### Scenario: Navigate transcript detail
- **WHEN** the transcript detail view is open
- **THEN** Up/Down keys SHALL scroll through events; PageUp/PageDown SHALL scroll by page; Escape SHALL return to the subagent panel

#### Scenario: Transcript detail for running node
- **WHEN** user presses Enter on a Running node
- **THEN** the detail view SHALL show the partial event timeline available so far, with a "streaming…" indicator

### Requirement: Failure diagnostics persistence
On subagent failure, the system SHALL persist the structured failure diagnostics (root cause, failed tool-call sequence, failed-round context, retry history) into the `subagent_transcripts` diagnostics columns in the same transaction that writes the header row, so that CLI and SSE replay can retrieve them without re-running the subagent.

#### Scenario: Diagnostics written with header on failure
- **WHEN** a subagent fails and its transcript header is written
- **THEN** `failure_diagnostics`, `root_cause`, and `retry_history` SHALL be populated in the same transaction

#### Scenario: Diagnostics absent on success
- **WHEN** a subagent completes successfully
- **THEN** the diagnostics columns SHALL be NULL/empty

