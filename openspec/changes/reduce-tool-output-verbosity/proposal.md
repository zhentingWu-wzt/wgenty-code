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
