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
