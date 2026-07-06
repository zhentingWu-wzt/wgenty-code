---
comet_change: fix-subagent-focus-collapse-keys
phase: verify
verify_mode: full
date: 2026-07-06
---

# Verification Report: fix-subagent-focus-collapse-keys

## Summary

| Dimension    | Status |
|--------------|--------|
| Completeness | 10/10 tasks; 1 MODIFIED delta requirement implemented |
| Correctness  | 3/3 delta scenarios covered (2 unit tests + code review) |
| Coherence    | Implementation matches `design.md`; no Design Doc (hotfix) |

**Final Assessment**: No CRITICAL or IMPORTANT issues. Ready for archive.

## Change Scope

Hotfix: subagent focus view 里 Ctrl+O/Ctrl+E 失效。根因：input_reader 将 Ctrl+O/Ctrl+E 转成 `ToggleCollapseLatest`/`ToggleCollapseAll` 独立事件，绕过 focus view 的 KeyEvent 吞没逻辑，直接操作被遮住的主对话 `committed_messages`。修复：focus view 打开时这两个事件操作 focus view 的 tool call 折叠（复用 `collapsed_tool_ids`），不副作用主对话。

**Note**: commit `9aaba57` 同时包含 `subagent_timeout_secs` 可配置化改动（core.rs/mod.rs/tool_dispatch.rs/turn.rs），这是 fix-subagent-timeout-default 的遗漏提交（工作树遗留，被 `git add -A` 纳入）。用户确认保留合并。该改动与 hotfix 逻辑独立，不影响 focus view 折叠修复的正确性。

## Completeness

### Task Completion
`tasks.md`: 10/10 complete (`[x]`).

### Spec Coverage
- Delta spec `specs/subagent-focus-view/spec.md` MODIFIES "Focus view navigation and exit" requirement, adds 2 scenarios (Ctrl+E, Ctrl+O) and clarifies fold shortcuts operate on focus timeline while open.
- Implementation: `FocusViewState::toggle_fold_all` / `toggle_fold_latest` (`subagent_focus_view.rs:253-296`); event.rs guards `ToggleCollapseAll`/`ToggleCollapseLatest` with `subagent_focus` check (`event.rs:486-536`).

## Correctness

### Requirement Implementation Mapping
| Requirement clause | Evidence |
|---|---|
| Fold shortcuts (`t`, `Ctrl+O`, `Ctrl+E`) always available, operate on focus timeline while open | `event.rs:94` ('t' → `toggle_fold_all`), `event.rs:491` (Ctrl+E → `toggle_fold_all`), `event.rs:516` (Ctrl+O → `toggle_fold_latest`) |
| `Ctrl+E` toggles fold of all tool calls (same as `t`) | `FocusViewState::toggle_fold_all` (`subagent_focus_view.rs:258`) |
| `Ctrl+O` toggles fold of the last tool call | `FocusViewState::toggle_fold_latest` (`subagent_focus_view.rs:278`) |
| While focus view open, SHALL NOT affect main chat collapse state | `ToggleCollapseAll`/`Latest` main-chat branch only runs in `else` (i.e. `subagent_focus` is `None`) (`event.rs:494-510`, `519-534`) |

### Scenario Coverage
1. **Ctrl+E toggles fold of all tool calls in focus view** — `test_toggle_fold_all_expands_then_collapses`: empty set → expand all (2 ids), non-empty → collapse all. ✓
2. **Ctrl+O toggles fold of the last tool call in focus view** — `test_toggle_fold_latest_flips_only_last_tool_call`: flips only "2" (last), "1" never touched; flipping again collapses "2" back. ✓
3. **SHALL NOT affect main chat collapse state** — code review: focus-active branch calls `focus.toggle_fold_*` and returns without touching `committed_messages`; main-chat logic is in the `else` branch only. ✓

## Coherence

### Design Adherence — `design.md`
- `toggle_fold_all` extracted from event.rs 't' logic, operates on `collapsed_tool_ids` ✓
- `toggle_fold_latest` finds last `MessageRole::Tool` via `chat_messages_to_ui_messages`, flips tid in set ✓
- 't' key refactored to `focus.toggle_fold_all()` ✓
- `ToggleCollapseAll`/`Latest` guarded with `subagent_focus` check ✓
- Main-chat behavior preserved in `else` branch ✓

### Design Doc
Hotfix has no Design Doc (only `design.md`). Checks 3 (Design Doc adherence), 6 (delta vs Design Doc), 7 (Design Doc locatable) not applicable — consistent with hotfix preset.

### Code Pattern Consistency
- `toggle_fold_all`/`toggle_fold_latest` follow existing `FocusViewState` method style (`pub fn &mut self`).
- Reuses `chat_messages_to_ui_messages` and `collapsed_tool_ids` mechanism (in set = expanded) — consistent with 't' key.
- Unit tests follow existing test helper pattern (`make_node`, `ToolCall`/`ToolCallFunction`).

## Build & Test Evidence
- `cargo build` — pass
- `cargo clippy --lib -- -D warnings` — 0 warnings
- `cargo test --lib` — 517 passed, 0 failed (515 pre-existing + 2 new toggle_fold tests)

## Issues
- CRITICAL: none
- IMPORTANT: none
- WARNING: commit `9aaba57` bundles `subagent_timeout_secs` wiring (fix-subagent-timeout-default leftover) with the hotfix. User-confirmed to keep merged; the two changes are logically independent and the leftover is correct (517 passed). Documented in commit message.
- SUGGESTION: none

## Conclusion
Hotfix fully implemented: focus view Ctrl+O/Ctrl+E operate on timeline tool-call folds, main chat no longer silently mutated. delta spec scenarios covered by unit tests. No critical/important issues. Ready for archive.
