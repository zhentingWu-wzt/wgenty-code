## ADDED Requirements

### Requirement: Failure diagnostics surfaced in trace rendering
The trace rendering (`call_tree`, `error_timeline`, `chrome_trace`, `html`) SHALL surface the structured failure diagnostics when a subagent failed: the `FailureRootCause` category (and guardian reason when applicable), the complete failed tool-call sequence with per-step elapsed time, the truncated failed-round context (assistant text + final tool output), and the retry history.

#### Scenario: Call tree shows failed sequence and root cause
- **WHEN** a failed subagent trace is rendered with `call_tree`
- **THEN** the output SHALL include the root-cause category and the ordered failed tool-call sequence with per-step durations

#### Scenario: Error timeline groups by root cause
- **WHEN** a failed subagent trace is rendered with `error_timeline`
- **THEN** the breakdown SHALL group failures by `FailureRootCause` category and include retry-history entries

#### Scenario: HTML report includes diagnostics section
- **WHEN** a failed subagent trace is rendered with `html`
- **THEN** the report SHALL include a failure-diagnostics section with root cause, failed sequence, failed-round context, and retry history

### Requirement: Raw diagnostics output
The trace rendering SHALL support a raw mode that prints the stored failure diagnostics JSON (root cause, failed sequence, failed-round context, retry history) without rendering, for piping to external tools.

#### Scenario: Raw mode emits diagnostics JSON
- **WHEN** a failed subagent trace is rendered with raw mode
- **THEN** the stored diagnostics SHALL be printed as pretty JSON to stdout
