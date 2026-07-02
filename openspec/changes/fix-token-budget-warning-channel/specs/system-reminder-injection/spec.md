## MODIFIED Requirements

### Requirement: Token budget calculation and one-time warning

The system SHALL compute the approximate token cost of the entire reminder block (all sections + preambles + attribution headers) once per session and emit a single dev-log warning when the total exceeds the configured threshold. The warning is operator-facing only (`tracing::warn!`); the system SHALL NOT emit any user-visible warning to the TUI status area, the chat log, or any other interactive surface.

#### Scenario: Reminder block under threshold
- **WHEN** the total reminder content is below the configured token threshold
- **THEN** no warning SHALL be emitted (neither dev-log nor user-visible)

#### Scenario: Reminder block exceeds threshold on first turn
- **WHEN** the reminder block exceeds the configured token threshold on the first user turn
- **THEN** the system SHALL emit exactly one `tracing::warn!` dev-log entry indicating the estimated token count
- **AND** the system SHALL NOT inject any `System` message into `committed_messages` or any other user-visible chat surface
- **AND** the request SHALL still proceed (warning is informational, not blocking)

#### Scenario: Threshold-exceeding block on subsequent turns
- **WHEN** the reminder block exceeds the threshold on the second or later turn in the same session, and the dev-log warning already fired
- **THEN** no additional warning SHALL be emitted in that session

#### Scenario: Threshold computed across all sources
- **WHEN** the budget calculation runs
- **THEN** it SHALL sum the byte/token cost of all four source layers plus preamble overhead
- **AND** the calculation SHALL NOT skip any included section

#### Scenario: No user-visible surface for the budget warning
- **WHEN** the reminder block exceeds the threshold at session startup
- **THEN** no `System`/`Assistant`/user-visible message SHALL be added to `committed_messages`
- **AND** the TUI welcome banner SHALL still render (the banner is not suppressed by the budget calculation)
