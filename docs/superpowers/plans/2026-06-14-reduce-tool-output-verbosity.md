---
change: reduce-tool-output-verbosity
design-doc: docs/superpowers/specs/2026-06-14-reduce-tool-output-verbosity-design.md
base-ref: cd36fd2d7f664fea53cc4f82ca48d222ff4a4ca3
---

# Implementation Plan: Reduce Tool Output Verbosity

## Summary

3 files, 11 tasks. Search tool compactness + Read tool output reduction. Search changes already staged.

## Task Order

### Phase 1: Search Tool (tasks 1.1–2.2) — already staged
1. `src/tools/search/grep.rs`: `files_with_matches` mode + line truncation 200 chars
2. `src/tools/search/search.rs`: `files_with_matches` schema parameter

### Phase 2: Read Tool (tasks 3.1–3.2)
3. `src/tools/filesystem/file_read.rs`: default max_chars 12000→6000 + per-line truncation 300 chars

### Phase 3: Verify (tasks 4.1–4.4)
4. `cargo test --lib`, `cargo build`, `cargo clippy`, manual check

## Key Implementation Notes

- `files_with_matches`: count per file, output `"path (N matches)"`, cap at `max_results` file entries
- Line truncation: `chars().take(N)` + `"…[truncated]"` suffix
- Read truncation order: per-line (300) → join → max_chars cap (6000)
- All params backward-compatible (defaults preserve existing behavior)
