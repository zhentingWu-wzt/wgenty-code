## ADDED Requirements

### Requirement: Grep tool supports files-with-matches compact mode
The `grep` tool SHALL support a `files_with_matches` boolean parameter that returns only file paths with per-file match counts.

#### Scenario: files_with_matches mode enabled
- **WHEN** `files_with_matches` is `true` and grep finds matches in 3 files
- **THEN** output SHALL contain `"src/auth.rs (3 matches)"` format
- **AND** individual matching line content SHALL NOT be included

#### Scenario: Default behavior (files_with_matches omitted)
- **WHEN** `files_with_matches` is omitted or `false`
- **THEN** output SHALL include full matching lines (existing behavior)

### Requirement: Grep tool truncates long matching lines
Lines exceeding 200 characters SHALL be truncated with `…[truncated]` suffix.

#### Scenario: Long line truncation
- **WHEN** a matching line exceeds 200 characters
- **THEN** the line SHALL be truncated to 200 chars with `…[truncated]` suffix

#### Scenario: Short line untouched
- **WHEN** a matching line is ≤200 characters
- **THEN** the line SHALL be displayed in full
