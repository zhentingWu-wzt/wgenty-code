---
comet_change: reduce-tool-output-verbosity
role: technical-design
canonical_spec: openspec
---

# Technical Design: Reduce Tool Output Verbosity

## Overview

Reduce verbosity of Searched (grep) and Read (file_read) tool output: add compact mode for grep, lower Read's default max_chars, and truncate long individual lines in both tools.

## Implementation

### 1. Grep Tool (`src/tools/search/grep.rs`)

**`files_with_matches` mode**:
- Extract `files_with_matches: bool` param (default `false`)
- When `true`: count matches per file, output `"path (N matches)"` format
- When `false`: existing behavior unchanged
- Apply `max_results` cap at file level in this mode (count per file, cap at N files)

**Line truncation**:
- In default mode (not `files_with_matches`): truncate lines > 200 chars → `…[truncated]`
- In `files_with_matches` mode: no line truncation needed (only paths shown)
- Use `chars().take(200)` + `…[truncated]` suffix pattern

**Schema update**:
```json
"files_with_matches": {
    "type": "boolean",
    "description": "Only show file paths with match counts, not individual lines"
}
```

### 2. Search Tool (`src/tools/search/search.rs`)

Add `files_with_matches` to schema. The SearchTool delegates to the same grep logic via shared code — verify parameter passes through.

### 3. Read Tool (`src/tools/filesystem/file_read.rs`)

**Default max_chars**:
- Change: `unwrap_or(12000)` → `unwrap_or(6000)`

**Per-line truncation**:
- Before `max_chars` total cap, truncate each line > 300 chars
- Apply: `line.chars().count() > 300 → take(300) + "…[truncated]"`
- Per-line truncation applied first, then total `max_chars` truncation on the rendered result

### Truncation Order

```
file_read:
  raw lines → per-line truncation (300 chars) → join with line numbers → max_chars cap (6000)

grep:
  matching lines → per-line truncation (200 chars) → max_results cap (200 lines)
  OR: matching lines → files_with_matches count → "path (N matches)"
```

## Edge Cases

| Case | Handling |
|------|----------|
| Empty file | `total_lines = 0`, empty output, no truncation |
| Binary file | Rejected before truncation logic |
| Single very long line (1000+ chars) | Truncated to 300 + `…[truncated]` |
| File exactly at 6000 chars | No total truncation, `truncated: false` |
| `files_with_matches` + zero matches | Empty output, consistent with existing behavior |
| `max_results` in `files_with_matches` mode | Caps number of file entries, not individual lines |
| `start_line`/`end_line` with Read | Line range applied first, then truncation on the selected range |
