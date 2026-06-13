# Comet Design Handoff

- Change: reduce-tool-output-verbosity
- Phase: design
- Mode: compact
- Context hash: 2faea4421893a21aea0497edc0f226d834aa025ec51c52c7a1b065ef99ee6c64

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/reduce-tool-output-verbosity/proposal.md

- Source: openspec/changes/reduce-tool-output-verbosity/proposal.md
- Lines: 1-28
- SHA256: 936e99414093d19f4e3c08ff13b44eae1079713e361e8ac0538aa54fb3b206e6

```md
## Why

Searched (grep) and Read (file_read) tools produce excessively verbose output in the conversation. Grep results include full matching lines regardless of length, and Read dumps up to 12000 characters of file content. This floods the chat context, making it difficult to scan results and wasting token budget.

## What Changes

### Searched (grep) Output Compactness
- **`files_with_matches` mode**: When enabled, only show file paths with match counts (e.g., `src/auth.rs (3 matches)`) instead of every matching line.
- **Line content truncation**: Match lines longer than 200 characters are truncated with `…[truncated]`.

### Read (file_read) Output Compactness
- **Reduced default `max_chars`**: Lower from 12000 to 6000.
- **Per-line truncation**: Lines exceeding 300 characters are truncated.

## Capabilities

### New Capabilities
- `search-output-compactness`: Grep tool supports `files_with_matches` mode and long-line truncation.
- `read-output-compactness`: File read tool uses smaller default character limit and per-line truncation.

### Modified Capabilities
<!-- None -->

## Impact

- `src/tools/search/grep.rs`: Add `files_with_matches` mode, line truncation at 200 chars
- `src/tools/search/search.rs`: Add `files_with_matches` input parameter schema
- `src/tools/filesystem/file_read.rs`: Lower default max_chars (12000→6000), add per-line truncation at 300 chars
```

## openspec/changes/reduce-tool-output-verbosity/design.md

- Source: openspec/changes/reduce-tool-output-verbosity/design.md
- Lines: 1-40
- SHA256: 583a0ada61808f1a9e90818bb381fba27c1895e2eb5f45bcc7367c83ace7b652

```md
## Context

Searched (grep) and Read (file_read) are among the most frequently used tools. Their output goes into conversation context, consuming token budget and screen space. Neither truncates long individual lines; grep has no compact mode; Read defaults to 12000 chars.

## Goals / Non-Goals

**Goals:**
- Add `files_with_matches` boolean param to grep/search for compact per-file summaries
- Truncate grep match lines >200 chars
- Reduce Read default `max_chars` 12000→6000
- Truncate Read lines >300 chars

**Non-Goals:**
- Changing `ToolOutput` struct or `Tool` trait
- New tools, endpoints, or streaming mechanisms
- Affecting other tools

## Decisions

### Decision 1: `files_with_matches` as boolean flag

Default `false` (backward-compatible). When true, output `"path (N matches)"` instead of `"path:line: content"`.

**Rationale**: Simplest extension, no new types needed, user opts in explicitly.

### Decision 2: Line truncation thresholds

200 chars for grep, 300 chars for Read. Both use `…[truncated]` suffix.

**Rationale**: 200 chars sufficient to identify a grep match; Read lines (code) need more room as they're the primary output.

### Decision 3: Default max_chars 6000

**Rationale**: ~half screen of content — enough context, still compact. Users can pass `max_chars` explicitly for full reads.

## Risks / Trade-offs

- Truncation may hide context → `…[truncated]` marker signals re-read with range
- `files_with_matches` may miss match details → default `false`, user opts in
- Default max_chars change may surprise → user always controls via explicit `max_chars`
```

## openspec/changes/reduce-tool-output-verbosity/tasks.md

- Source: openspec/changes/reduce-tool-output-verbosity/tasks.md
- Lines: 1-23
- SHA256: 7ebb40f803fd09acad58329a7eddc440f88e196f2f9c8c68fbc9d75cc16df272

```md
## 1. Search Tool — files_with_matches Mode

- [ ] 1.1 Add `files_with_matches` parameter extraction and branch logic in `GrepTool::execute()` in `src/tools/search/grep.rs`
- [ ] 1.2 In files_with_matches mode: count matches per file, output `"path (N matches)"` format
- [ ] 1.3 Add `files_with_matches` to `GrepTool::input_schema()` in `src/tools/search/grep.rs`
- [ ] 1.4 Add `files_with_matches` to `SearchTool::input_schema()` in `src/tools/search/search.rs`

## 2. Search Tool — Line Truncation

- [ ] 2.1 Add line truncation (>200 chars → `…[truncated]`) in grep's non-files_with_matches path in `src/tools/search/grep.rs`
- [ ] 2.2 Ensure truncation does NOT apply in files_with_matches mode (only file paths shown)

## 3. Read Tool — Default max_chars & Line Truncation

- [ ] 3.1 Change default `max_chars` from 12000 to 6000 in `src/tools/filesystem/file_read.rs`
- [ ] 3.2 Add per-line truncation (>300 chars → `…[truncated]`) before `max_chars` total cap in `src/tools/filesystem/file_read.rs`

## 4. Testing & Verification

- [ ] 4.1 Run `cargo test --lib` — all existing tests pass
- [ ] 4.2 Run `cargo build` — compiles without errors
- [ ] 4.3 Run `cargo clippy --all-targets -- -D warnings` — no new warnings introduced
- [ ] 4.4 Manual verification: trigger grep with `files_with_matches: true`, verify compact output
```

## openspec/changes/reduce-tool-output-verbosity/specs/read-output-compactness/spec.md

- Source: openspec/changes/reduce-tool-output-verbosity/specs/read-output-compactness/spec.md
- Lines: 1-23
- SHA256: 598b10e771955871d214b77d8d3ad5ebe1d45c85812ab7549b58beda5caea8a6

```md
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
```

## openspec/changes/reduce-tool-output-verbosity/specs/search-output-compactness/spec.md

- Source: openspec/changes/reduce-tool-output-verbosity/specs/search-output-compactness/spec.md
- Lines: 1-24
- SHA256: 45344822199eb73e4bb5e9169f2cbff465bbc90677667d3b71f23d10dbaef496

```md
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
```

