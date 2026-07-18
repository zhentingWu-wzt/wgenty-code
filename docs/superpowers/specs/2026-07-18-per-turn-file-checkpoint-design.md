---
comet_change: per-turn-file-checkpoint
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-18-per-turn-file-checkpoint
status: final
---

# Design: Per-turn file checkpoint

- **Date**: 2026-07-18
- **Change**: `per-turn-file-checkpoint`
- **Status**: Approved
- **OpenSpec**: `openspec/changes/per-turn-file-checkpoint/`

## Overview

Replace the per-mutating-tool `git stash` checkpoint with a per-turn **file-content snapshot** system, aligned with Claude Code's checkpointing model. Checkpoints are created once per agent turn (not per tool call), stored as file snapshots (not git stashes), and support non-destructive rewind to any prior turn without touching unrelated files.

## Motivation

The current checkpoint (`handlers.rs:270-279`) fires before every `apply_patch`/`file_edit`/`file_write`/`exec_command`:

1. **Too frequent / stash explosion** - dozens of stashes per session; 11 stranded on `dev`/`main`; no cleanup.
2. **Wrong rewind semantics** - `undo` uses `git stash pop`, which conflicts on accumulated uncommitted state. Correct cross-turn rewind needs `git reset --hard` + `git clean -fd`, the latter nukes unrelated untracked files - relocating data-loss risk from create to undo.
3. **Silent failures** - `let _ =` swallows checkpoint errors; the safety net can be broken silently.

Benchmark (verified): Claude Code = per-user-prompt file snapshots (not git), does not track bash changes, 30-day cleanup. Codex = no checkpoint system (sandbox + approval + git). Neither checkpoints per tool call.

## Goals / Non-Goals

**Goals**
- Checkpoint frequency -> once per turn.
- Correct, non-destructive rewind to any turn; unrelated untracked files untouched.
- Visible failures (warn); bounded retention (keep-N prune).
- Consistent daemon + REPL behavior via a shared hook.

**Non-Goals**
- Conversation rewind (files only).
- Tracking bash/exec_command file changes.
- 30-day session-bound cleanup (this iteration: keep-N; evolve later).
- Subagent-independent checkpoints (folded into root turn).
- Replacing git as VCS.

## Architecture

```
turn entry (chat_stream / REPL agent loop)
  └─ CheckpointStore::begin_turn(turn_id)        # create checkpoints/<turn-id>/
       └─ execute_with_hooks(tool)
            ├─ if tool ∈ {file_edit,file_write,apply_patch}:
            │     CheckpointStore::capture_file(turn_id, path)   # pre-edit content -> blob + manifest
            └─ tool.execute()
  └─ rewind(turn_id): read manifest -> overwrite each tracked file from blob / rebuild tombstone
       (no git reset, no git clean)
```

`CheckpointManager` (existing) becomes a thin facade: `undo` -> `CheckpointStore::rewind`, `create` -> extra snapshot on current turn, `list` -> `CheckpointStore::list`.

## Components

### `CheckpointStore` (new: `src/tools/checkpoint_store.rs`)

```
CheckpointStore { root: PathBuf }   // root = <project>/.wgenty-code/checkpoints/
```

- `begin_turn(turn_id) -> Result<()>` - create `checkpoints/<turn-id>/` + empty `manifest.json`.
- `capture_file(turn_id, path) -> Result<()>` - if not already snapshotted this turn: read pre-edit content, write blob, append manifest entry. Binary/oversized -> mark `skipped`. Same file edited N times -> stored once (first pre-edit).
- `capture_files(turn_id, paths)` - batch (apply_patch multi-file).
- `rewind(turn_id) -> Result<String>` - per manifest entry: `saved` -> overwrite file with blob; `tombstone` -> recreate deleted file; `skipped` -> warn (not restorable). Returns summary. No reset/clean.
- `prune(keep_n)` - by mtime, keep newest N, delete excess dirs.
- `list() -> Vec<TurnInfo>`.

### Manifest schema

```json
{
  "turn_id": "<uuid>",
  "created_at": "<RFC3339>",
  "files": [
    { "path": "src/foo.rs", "blob": "blobs/<sha8>", "state": "saved" },
    { "path": "src/old.rs", "blob": "blobs/<sha8>", "state": "tombstone" },
    { "path": "assets/logo.png", "blob": null, "state": "skipped" }
  ]
}
```

### Hook integration (`execute_with_hooks`)

Before executing `file_edit`/`file_write`/`apply_patch`, extract target file path(s) from args and call `capture_file(s)`. Path extraction:
- `file_edit` / `file_write`: `args["path"]`.
- `apply_patch`: parse patch headers to enumerate affected files; deletions recorded as `tombstone` at capture time (pre-edit content saved so rewind can rebuild).

turn-id is threaded via `ToolContext` (or a session-scoped handle) from the turn entry to the hook.

### Turn entry

- **Daemon**: `chat_stream` (`handlers.rs:82`) generates `turn_id = Uuid::new_v4()`, calls `begin_turn`, stores turn_id where `execute_tool`/hooks can read it. `plan_mode` skips snapshotting.
- **REPL**: agent loop turn entry calls `begin_turn`; same hook path.
- **Subagents**: share root turn-id (no independent checkpoint).

### Removal

- Delete `handlers.rs:270-279` (per-tool git-stash block).
- `CheckpointManager::undo` no longer `git stash pop`; delegates to `CheckpointStore::rewind`.
- Replace `let _ =` with `if let Err(e) = ... { tracing::warn!(...) }`.

## Data Flow

1. User message -> `chat_stream` -> `begin_turn(uuid)` -> prune.
2. Model streams tool_call -> frontend dispatches `execute_tool` -> `execute_with_hooks`.
3. Hook: file-edit tool detected -> `capture_file` (pre-edit content to blob + manifest).
4. Tool executes (modifies file).
5. Repeat 2–4 for all tool calls in the turn.
6. Optional rewind: `undo(checkpoint_id?)` -> resolve turn -> `rewind(turn_id)` -> overwrite tracked files only.

## Error Handling

- **Capture failure** (read error, disk full): `tracing::warn!`, do not abort the tool call (spec: non-fatal). File omitted from manifest.
- **Rewind failure** (blob missing, write error): best-effort per file; continue remaining files; return summary with failures listed. Never leave a partial git state (no git ops).
- **Path extraction failure** (malformed apply_patch): warn + capture what can be parsed.
- All errors use `anyhow::Result` + `.context()` per AGENTS.md; no bare `unwrap()`.

## Testing

- **Unit** (`checkpoint_store.rs`): capture single/multi file; repeat-edit stores once; tombstone capture+rebuild; prune keeps newest N; skipped binary.
- **Hook**: args path extraction (incl. apply_patch multi-file); capture-before-execute ordering.
- **Integration**: cross-turn rewind restores pre-turn state; untracked file survives; daemon+REPL parity.
- **Regression**: no `git stash` created on `file_edit` (assert stash list unchanged).
- **CI**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`; binary size delta ≤ 500KB.

## Decisions

1. File snapshots (not git stash) - non-destructive, only touches tracked files.
2. Per-turn trigger (daemon `chat_stream` + REPL loop).
3. Hook intercepts pre-edit via `execute_with_hooks` (no per-tool modification).
4. Non-destructive rewind (overwrite tracked files only; no reset/clean).
5. keep-N prune (default 10, configurable).
6. turn-id = uuid.
7. Subagent edits fold into root turn.
8. Manual `checkpoint` tool retained (extra snapshot on current turn).
9. Bash/exec_command changes not tracked (matches Claude Code; snapshot can't reliably restore them).

## Risks / Trade-offs

- **Disk**: full-content snapshots cost more than diffs. Mitigation: only changed files, keep-N, size threshold for skip.
- **Turn boundary**: assumes one `chat_stream` call = one turn; retries may over-create. Mitigation: harmless (prune bounds).
- **Path extraction**: incomplete apply_patch parsing -> missed snapshots. Mitigation: unit tests for parser.
- **Concurrency**: uuid-isolated turn dirs; no cross-turn write conflict.
- **Binary/skipped**: rewind incomplete for those. Mitigation: manifest marks skipped; rewind reports them.
