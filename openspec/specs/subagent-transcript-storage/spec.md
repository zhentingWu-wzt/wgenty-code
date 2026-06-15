# subagent-transcript-storage Specification

## Purpose
TBD - created by archiving change rlm-observability-and-robustness. Update Purpose after archive.
## Requirements
### Requirement: Transcript database schema
The system SHALL maintain a SQLite database at `~/.wgenty-code/subagent_transcripts.db` with tables for transcript headers and per-round events.

#### Scenario: Database created on first use
- **WHEN** the first subagent transcript is written and the database file does not exist
- **THEN** the system SHALL create the database file with the correct schema automatically

#### Scenario: Transcript header row written on subagent completion
- **WHEN** a subagent reaches Completed, Failed, or Cancelled status
- **THEN** a row SHALL be inserted into `subagent_transcripts` with id, session_id, parent_id, label, status, system_prompt, user_prompt, started_at, finished_at, total_tokens, error_message (if any), and summary

#### Scenario: Events batch-written on subagent completion
- **WHEN** a subagent completes
- **THEN** all events (thought, action, tool_result, error) from the subagent's execution SHALL be inserted into `subagent_events` in a single transaction

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

