## ADDED Requirements

### Requirement: File read uses reduced default character limit
The `file_read` tool SHALL default `max_chars` to 6000 (reduced from 12000).

#### Scenario: Default max_chars
- **WHEN** reading without explicit `max_chars`
- **THEN** output SHALL be capped at 6000 characters

#### Scenario: Explicit max_chars overrides default
- **WHEN** `max_chars` is explicitly provided
- **THEN** the explicit value SHALL be used

### Requirement: File read truncates long individual lines
Lines exceeding 300 characters SHALL be truncated with `…[truncated]` suffix.

#### Scenario: Long line truncation
- **WHEN** a line exceeds 300 characters
- **THEN** it SHALL be truncated to 300 chars with `…[truncated]`

#### Scenario: Per-line + max_chars truncation
- **WHEN** a file has long lines AND total content exceeds `max_chars`
- **THEN** per-line truncation SHALL apply first, then total `max_chars` cap
