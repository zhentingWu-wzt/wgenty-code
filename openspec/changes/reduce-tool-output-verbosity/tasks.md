## 1. Search Tool — files_with_matches Mode

- [x] 1.1 Add `files_with_matches` parameter extraction and branch logic in `GrepTool::execute()` in `src/tools/search/grep.rs`
- [x] 1.2 In files_with_matches mode: count matches per file, output `"path (N matches)"` format
- [x] 1.3 Add `files_with_matches` to `GrepTool::input_schema()` in `src/tools/search/grep.rs`
- [x] 1.4 Add `files_with_matches` to `SearchTool::input_schema()` in `src/tools/search/search.rs`

## 2. Search Tool — Line Truncation

- [x] 2.1 Add line truncation (>200 chars → `…[truncated]`) in grep's non-files_with_matches path in `src/tools/search/grep.rs`
- [x] 2.2 Ensure truncation does NOT apply in files_with_matches mode (only file paths shown)

## 3. Read Tool — Default max_chars & Line Truncation

- [x] 3.1 Change default `max_chars` from 12000 to 6000 in `src/tools/filesystem/file_read.rs`
- [x] 3.2 Add per-line truncation (>300 chars → `…[truncated]`) before `max_chars` total cap in `src/tools/filesystem/file_read.rs`

## 4. Testing & Verification

- [x] 4.1 Run `cargo test --lib` — all existing tests pass
- [x] 4.2 Run `cargo build` — compiles without errors
- [x] 4.3 Run `cargo clippy --all-targets -- -D warnings` — no new warnings introduced
- [ ] 4.4 Manual verification: trigger grep with `files_with_matches: true`, verify compact output
