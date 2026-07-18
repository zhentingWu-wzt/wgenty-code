---
change: per-turn-file-checkpoint
design-doc: docs/superpowers/specs/2026-07-18-per-turn-file-checkpoint-design.md
base-ref: e9e691b4849f566b33306db874a92f705b3f3c5b
archived-with: 2026-07-18-per-turn-file-checkpoint
---

# Implementation Plan: per-turn-file-checkpoint

Reference: [Design Doc](../../../docs/superpowers/specs/2026-07-18-per-turn-file-checkpoint-design.md) · [OpenSpec change](../../../openspec/changes/per-turn-file-checkpoint/)

## Phase 1 — CheckpointStore core (`src/tools/checkpoint_store.rs`, new)

- [x] 1.1 Define `CheckpointStore { root: PathBuf }`, `FileState { Saved, Tombstone, Skipped }`, `ManifestEntry { path, blob, state }`, `Manifest { turn_id, created_at, files }` (serde)
- [x] 1.2 `begin_turn(turn_id)`: create `checkpoints/<turn-id>/` + `blobs/` + empty manifest; idempotent
- [x] 1.3 `capture_file(turn_id, path)`: if not already in manifest → read pre-edit content; binary/oversized → `Skipped`; else write blob (content-addressed) + `Saved` entry. Same file twice → no-op after first
- [x] 1.4 `capture_files(turn_id, paths)`: iterate (apply_patch multi-file)
- [x] 1.5 `rewind(turn_id)`: load manifest; per entry: `Saved`→overwrite file from blob, `Tombstone`→recreate from blob, `Skipped`→warn+count; return summary. No git ops
- [x] 1.6 `prune(keep_n)`: list turn dirs by mtime, delete oldest beyond N
- [x] 1.7 `list() -> Vec<(turn_id, created_at, file_count)>`
- [x] 1.8 Unit tests: capture-once-per-file, tombstone rebuild, prune ordering, skipped binary, rewind restores pre-edit

## Phase 2 — Hook integration (`src/tools/executor.rs`)

- [x] 2.1 Add `checkpoint_store: Option<Arc<CheckpointStore>>` + `current_turn_id` access to executor/hook context
- [x] 2.2 In `execute_with_hooks`, before execute: if tool ∈ {file_edit,file_write,apply_patch} → extract target path(s) → `capture_file(s)`; on Err `tracing::warn!` (non-fatal)
- [x] 2.3 Path extractors: `file_edit`/`file_write` → `args["path"]`; `apply_patch` → parse `*** Begin/End Patch` file headers (reuse existing apply_patch parser if present)
- [x] 2.4 Thread `turn_id` via `ToolContext` (add field) from turn entry → hook
- [x] 2.5 Test: capture happens before execute; repeat-edit single capture

## Phase 3 — Turn entry wiring

- [x] 3.1 Daemon `chat_stream` (`handlers.rs:82`): gen `turn_id=Uuid::new_v4()`, `begin_turn`, `prune(default)`, stash turn_id for execute_tool/hook (session context or shared state)
- [x] 3.2 plan_mode skip (no edits expected)
- [x] 3.3 REPL agent loop turn entry: `begin_turn` (locate turn boundary in agent loop)
- [x] 3.4 Subagent edits share root turn_id

## Phase 4 — Rewire CheckpointManager + remove old logic (`src/tools/checkpoint.rs`, `handlers.rs`)

- [x] 4.1 `CheckpointManager::undo` → delegate to `CheckpointStore::rewind` (accept optional turn_id/checkpoint_id)
- [x] 4.2 `CheckpointManager::create` → extra snapshot on current turn (manual tool path)
- [x] 4.3 `CheckpointManager::list` → `CheckpointStore::list`
- [x] 4.4 DELETE `handlers.rs:270-279` per-tool git-stash block
- [x] 4.5 Replace `let _ =` with `if let Err(e)=...{ tracing::warn!(...) }`
- [x] 4.6 Update `checkpoint` tool description/schema (no longer git-stash)
- [x] 4.7 Regression test: `file_edit` produces no `git stash` entry

## Phase 5 — Config + cleanup

- [x] 5.1 `agent.checkpoint.keep_n` (default 10) in Settings; pass to prune
- [x] 5.2 `.wgenty-code/checkpoints/` gitignore (if not covered by `.wgenty-code/`)
- [x] 5.3 Register `CheckpointStore` in DaemonState / ToolRegistry construction

## Phase 6 — Verify

- [x] 6.1 `cargo fmt --check`
- [x] 6.2 `cargo clippy --all-targets -- -D warnings`
- [x] 6.3 `cargo test --all`
- [x] 6.4 Manual: cross-turn rewind, untracked survives, prune, daemon+REPL parity
- [x] 6.5 Perf: binary size delta ≤ 500KB, startup ≤ 5%
- [x] 6.6 CHANGELOG.md
