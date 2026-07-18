# per-turn-checkpoint Specification

## Purpose
TBD - created by archiving change per-turn-file-checkpoint. Update Purpose after archive.
## Requirements
### Requirement: Per-turn file snapshot

The system SHALL create a file-content snapshot at the start of each agent turn, capturing the pre-edit state of files modified by file-editing tools (`file_edit`, `file_write`, `apply_patch`) during that turn. The snapshot SHALL be stored under `.wgenty-code/checkpoints/<turn-id>/` with a manifest mapping each tracked file to its saved content or tombstone.

#### Scenario: One turn, multiple edits to same file

- **WHEN** a turn edits the same file three times via `file_edit`
- **THEN** only the first edit's pre-edit content is captured once in that turn's snapshot

#### Scenario: Snapshot trigger is per-turn not per-tool

- **WHEN** a turn contains 5 `file_edit` calls and 3 `exec_command` calls
- **THEN** exactly one turn snapshot directory is created, not one per tool call

### Requirement: Non-destructive rewind

The system SHALL provide a rewind operation that restores files to a chosen turn's snapshot state by overwriting only the files tracked in that turn's manifest. The system MUST NOT execute `git reset`, `git clean`, or any operation that modifies files not tracked by the snapshot manifest.

#### Scenario: Rewind preserves unrelated untracked files

- **WHEN** a user rewinds to turn N and a manually-created untracked file exists in the working tree
- **THEN** the untracked file is not deleted

#### Scenario: Rewind across multiple turns

- **WHEN** turns 1, 2, 3 each edit files and the user rewinds to turn 2's checkpoint
- **THEN** files tracked by turn 2's snapshot are restored to the state before turn 2's edits, discarding turn 2 and 3 edits to those tracked files

#### Scenario: Tombstone restores deleted file

- **WHEN** an `apply_patch` deletes a file during a turn and the user rewinds that turn
- **THEN** the deleted file is recreated from the snapshot's pre-edit content

### Requirement: No bash command tracking

The system SHALL NOT capture file changes produced by `exec_command`/bash into snapshots, consistent with the local-undo (not version-control) scope.

#### Scenario: Bash-created file not in snapshot

- **WHEN** an `exec_command` creates a file via shell redirect
- **THEN** that file is not tracked by the turn snapshot and rewind does not restore it

### Requirement: Checkpoint retention pruning

The system SHALL retain at most N (default 10) most-recent turn snapshots and SHALL delete older snapshots when a new one is created. N MUST be configurable.

#### Scenario: Prune on create

- **WHEN** an 11th turn snapshot is created with keep-N=10
- **THEN** the oldest snapshot is deleted

### Requirement: Visible checkpoint failures

The system SHALL log a warning when a snapshot capture or rewind fails and SHALL NOT silently swallow checkpoint errors. A capture failure SHALL NOT abort the originating tool call.

#### Scenario: Capture failure is logged and non-fatal

- **WHEN** a pre-edit file read fails during capture
- **THEN** a warning is logged and the turn's tool call proceeds without aborting

### Requirement: No per-tool git-stash checkpoint

The system SHALL NOT create automatic git-stash checkpoints before individual mutating tool calls. Automatic checkpointing SHALL occur once per turn via file snapshots.

#### Scenario: No per-tool stash on file edit

- **WHEN** a `file_edit` tool call is approved and executed
- **THEN** no git stash is created for that individual tool call

### Requirement: Daemon and REPL consistency

The system SHALL apply the same per-turn file snapshot behavior in both the daemon (`chat_stream` turn entry) and the REPL agent loop, so checkpoint semantics are consistent across frontends.

#### Scenario: REPL turn also snapshots

- **WHEN** a REPL turn edits files via file-editing tools
- **THEN** a turn snapshot is created and rewind is available

