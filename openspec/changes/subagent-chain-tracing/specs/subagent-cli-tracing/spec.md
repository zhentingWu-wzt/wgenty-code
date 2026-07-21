## ADDED Requirements

### Requirement: Subagent CLI subcommand
The system SHALL provide a `wgenty-code subagent` CLI subcommand with `list`, `trace`, and `health` sub-actions that read directly from the project-local transcript store, without starting an agent loop. All sub-actions SHALL be read-only.

#### Scenario: Subcommand help
- **WHEN** `wgenty-code subagent --help` is run
- **THEN** the `list`, `trace`, and `health` sub-actions SHALL be documented with their options

### Requirement: List historical subagent runs
`wgenty-code subagent list` SHALL list historical subagent runs in reverse chronological order, showing at minimum: transcript id, label, status, root-cause (when failed), duration, and started-at timestamp. It SHALL support optional `--session <id>` (filter by session), `--status <status>` (filter by status), and `--limit <n>` (default 20).

#### Scenario: List recent runs
- **WHEN** `wgenty-code subagent list` is run
- **THEN** the most recent runs SHALL be printed as a table in reverse chronological order, capped at `--limit`

#### Scenario: Filter by status
- **WHEN** `wgenty-code subagent list --status failed` is run
- **THEN** only runs with Failed status SHALL be listed, each annotated with its root cause

### Requirement: Show single subagent trace
`wgenty-code subagent trace <id>` SHALL render the full trace of a single subagent run, reusing the existing trace rendering logic. It SHALL support `--format <call_tree|error_timeline|chrome_trace|html>` (default `call_tree`) and `--raw` (print the raw stored error message and failed-round context without rendering).

#### Scenario: Default call-tree rendering
- **WHEN** `wgenty-code subagent trace <id>` is run
- **THEN** the trace SHALL be rendered as an ASCII call tree including the failed tool-call sequence and root cause when the run failed

#### Scenario: HTML format output
- **WHEN** `wgenty-code subagent trace <id> --format html` is run
- **THEN** a self-contained HTML report SHALL be written to stdout (or `--output <file>`)

#### Scenario: Unknown id
- **WHEN** `wgenty-code subagent trace <unknown-id>` is run
- **THEN** the command SHALL exit non-zero with a clear "not found" error message

### Requirement: Health summary
`wgenty-code subagent health` SHALL print subagent health statistics computed from transcript headers: total runs, completed, failed, success rate, and failure-mode breakdown. It SHALL support `--period <1h|24h|7d|30d|all>` (default `24h`).

#### Scenario: Default 24h health
- **WHEN** `wgenty-code subagent health` is run
- **THEN** the 24-hour window statistics SHALL be printed, including success rate and failure-mode counts

#### Scenario: Failure-mode breakdown with root causes
- **WHEN** `wgenty-code subagent health --period 7d` is run and failures exist
- **THEN** the breakdown SHALL group failures by `FailureRootCause` category with counts
