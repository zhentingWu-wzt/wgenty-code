# Verification Report: per-turn-file-checkpoint

- **Date**: 2026-07-18
- **Change**: `per-turn-file-checkpoint`
- **verify_mode**: full
- **Commit**: `def0d5f`
- **base-ref**: `e9e691b`

## Summary

| Dimension    | Status |
|--------------|--------|
| Completeness | 35/35 tasks complete; 7/7 requirements present |
| Correctness  | All requirements covered by implementation + tests; no CRITICAL gaps |
| Coherence    | Follows Design Doc intent; 3 documented implementation divergences (WARNING) |
| Build/Test   | `cargo build` OK; `cargo test --lib` 944 passed; `cargo test --all` 142 passed; fmt/clippy clean |

**Final assessment**: No CRITICAL issues. Ready for branch handling / archive (with noted WARNs).

## Completeness

- tasks.md: 35/35 `[x]`
- Superpowers plan: all items `[x]`
- Delta capability: `per-turn-checkpoint` (1)
- Changed files vs base-ref: 25 files (+1273 / -445)

### Requirements → evidence

| Requirement | Evidence |
|-------------|----------|
| Per-turn file snapshot | `CheckpointStore` + `maybe_capture_pre_edit` in `src/tools/mod.rs`; tests `pre_edit_capture_runs_before_file_write_and_is_idempotent` |
| Non-destructive rewind | `CheckpointStore::rewind`; tests `rewind_preserves_unrelated_untracked_file`, `undo_restores_captured_file_and_preserves_unrelated` |
| No bash tracking | capture only for `file_edit`/`file_write`/`apply_patch` |
| keep-N prune | `agent.checkpoint.keep_n` default 10; `begin_turn`/`try_capture_file` call `prune`; test `prune_keeps_newest_n` |
| Visible failures | `tracing::warn!` on capture/begin_turn/undo; capture never aborts tool |
| No per-tool git-stash | `handlers.rs` per-tool stash block removed; `CheckpointManager` is file-snapshot facade |
| Daemon + REPL consistency | daemon `execute_tool` + headless `RegistryToolPort` + subagent `GuardingToolPort` all pass `checkpoint_store` |

### Scenario coverage

| Scenario | Covered? |
|----------|----------|
| One turn, multiple edits same file → one capture | Yes — unit test |
| Snapshot per-turn not per-tool | Yes — no per-tool stash; capture keyed by turn_id |
| Rewind preserves untracked | Yes — unit test |
| Rewind across multi-turn | Yes — rewind by turn_id; manager undo targets turn |
| Tombstone restores deleted file | Functionally yes (existing file delete → Saved+blob restore); see WARN-2 |
| Bash-created file not in snapshot | Yes — tool filter |
| Prune on create keep-N=10 | Yes — unit test |
| Capture failure non-fatal | Yes — trait swallows + warn |
| No per-tool stash on file_edit | Yes — code path removed (no dedicated regression assert on `git stash list`) |
| REPL turn also snapshots | Yes — headless begin_turn + capture on execute |

## Correctness notes

- Capture is in `ToolRegistry::execute_with_context` (not only `execute_with_hooks`), so subagent path that bypasses hooks still captures. Better than design sketch; intentional.
- TUI mints `turn_id` per user turn (`src/tui/app/turn.rs`) and passes it on `execute_tool`; daemon `begin_turn` runs there. Headless mints one turn_id for the run.
- Delete via apply_patch on an existing file: pre-edit capture sees the file → `Saved`+blob; rewind restores content (matches scenario outcome).

## Coherence / Design adherence

Followed:
1. File snapshots not git stash
2. Per-turn granularity
3. Pre-edit intercept for file tools
4. Non-destructive rewind
5. keep-N prune (configurable)
6. turn-id = uuid
7. Subagent folds into root turn
8. Manual checkpoint tool retained
9. No bash tracking

## Issues

### CRITICAL
None.

### WARNING

1. **WARN-1 — turn mint site vs Design Doc**  
   Design/OpenSpec design say daemon `chat_stream` generates `turn_id` + `begin_turn`. Implementation: frontends mint `turn_id` (TUI/headless); daemon `begin_turn` on `execute_tool` when `body.turn_id` is present. `chat_stream` itself does not mint or begin.  
   **Impact**: Clients that call `execute_tool` without `turn_id` get no capture. Main TUI/headless paths always supply it.  
   **Recommendation**: Accept as better fit for stateless daemon (design Context already notes frontend orchestration), and optionally amend design.md with an Implementation Divergence note; or later mint in `chat_stream` and return turn_id to clients.

2. **WARN-2 — delete recorded as Saved not Tombstone**  
   Design: apply_patch delete → tombstone. Code: existing file at capture → `Saved`+blob; rewind still restores file. New-file create → `Tombstone` without blob → rewind deletes.  
   **Impact**: Spec scenario outcome holds; state naming differs.  
   **Recommendation**: Accept, or rename/document; optional test named after the delete scenario.

3. **WARN-3 — capture hook location**  
   Design: `execute_with_hooks`. Code: `ToolRegistry::execute_with_context` so GuardingToolPort/subagent also capture.  
   **Impact**: Broader/correct coverage.  
   **Recommendation**: Document as intentional divergence in design.md.

### SUGGESTION

1. **SUG-1** — `FileState::Tombstone` branch with `blob: Some` is currently unreachable (capture always sets tombstone with `blob: None`). Simplify or use for delete-with-content if WARN-2 is changed.
2. **SUG-2** — Add explicit regression test: `file_edit` leaves `git stash list` unchanged.
3. **SUG-3** — Manifest RMW is unlocked; parallel tool calls in one turn could race. Low likelihood today; consider a per-turn mutex if parallel edits land.

## Security

- No hardcoded secrets.
- No new unsafe.
- Rewind does not run destructive git ops.

## Build / quality gates

- `cargo fmt --check` — pass
- `cargo clippy --all-targets -- -D warnings` — pass
- `cargo test --lib` — 944 passed
- `cargo test --all` — 142 passed, 4 ignored
- CHANGELOG.md updated under Unreleased
- `.gitignore` includes `.wgenty-code/checkpoints/`

## Decision log (verify)

- Accept WARN-1/2/3 as non-blocking divergences (functional requirements met).
- Branch handling: pending user choice.
