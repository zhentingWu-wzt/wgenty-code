## ADDED Requirements

### Requirement: Failed subagent delivers structured error code and partial results
When a subagent fails (timeout, budget exhaustion, stuck detection, parse error, or max-rounds exceeded), the system SHALL return a structured `SubagentError` to the parent agent carrying (1) a categorized error type that maps to a stable error-code string and (2) the subagent's last accumulated text snapshot as a partial result. The parent agent SHALL receive both the structured error code (via `ToolError::code`) and the partial work (via `ToolError::message`, which appends the partial result through `full_message()`), so it can salvage partial work and make informed retry/continue/abort decisions rather than receiving a bare error string with no recoverable output. The failed transcript SHALL record the same `full_message()` as its result snapshot.

#### Scenario: Subagent timeout delivers structured error code and partial results
- **WHEN** a subagent exceeds `agent.subagent.timeout_secs`
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `subagent_timeout`
- **AND** the `ToolError::message` SHALL include any text the subagent accumulated before timing out, appended via `full_message()`'s "Partial results" section
- **AND** the failed transcript SHALL record the same `full_message()` as its result snapshot

#### Scenario: Budget exhaustion delivers budget_exceeded code and partial results
- **WHEN** a subagent exhausts its token budget
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `budget_exceeded`
- **AND** the `ToolError::message` SHALL include the subagent's partial work accumulated before budget exhaustion

#### Scenario: Empty partial result does not append an empty segment
- **WHEN** a subagent fails and has no accumulated text snapshot, or the snapshot is empty/whitespace-only
- **THEN** `full_message()` SHALL return only the error message
- **AND** SHALL NOT append a "Partial results (before failure)" section
