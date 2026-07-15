---
comet_change: esc-interrupt-turn
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-14-esc-interrupt-turn
status: final
---

# Design Doc: ESC Interrupt Running Turn

- **Change**: `esc-interrupt-turn`
- **Capability**: `tui-turn-interruption` (new)
- **Date**: 2026-07-13
- **Status**: Approved (brainstorming confirmed - Approach A + hardcoded i18n)

> This document is the deep-design output of the design phase. It does not repeat the `proposal.md`; see `openspec/changes/esc-interrupt-turn/proposal.md` for why/what/impact and `specs/tui-turn-interruption/spec.md` for formal requirements.

## 1. Problem & Existing Infrastructure

The TUI REPL spawns each agent turn as a tokio task stored in `App::current_turn_handle`. There is **no keyboard shortcut** to stop a running turn; the only cancellation path is `/clear` (which wipes the conversation). ESC currently quits the whole app (`event_key.rs`).

Cancellation machinery already exists and is reused, not reinvented:

- `cancel_current_turn()` (`turn.rs:328`): aborts the `JoinHandle`, sets `phase = Idle`, `suppress_phase_updates = true`, emits `AppEvent::TurnAborted { reason: Interrupted }`. Today only `/clear` calls it.
- `TurnAbortReason::Interrupted` (`agent_phase.rs:80`).
- `TurnAborted` handler (`event.rs:423`): clears the subagent tree, records `last_abort_reason`.
- `/clear` cancels daemon-side subagents via async `reset_agent_generation` (`input.rs:65-83`).

The gap is the **key binding and interrupt-specific UX** (preserve partial output, surface feedback, cancel subagents), not the abort primitive.

## 2. Approaches Considered

| Approach | Mechanism | Verdict |
|----------|-----------|---------|
| **A (selected)** | `interrupt_running_turn()` wraps `cancel_current_turn()`: commit partial streaming, finalize tool placeholder, `reset_agent_generation`, push feedback message; ESC branch gated on `current_turn_handle.is_some()`; remove ESC-quit | Reuses proven abort mechanism; complete UX; change scoped to `src/tui/` |
| B | ESC calls `cancel_current_turn()` only | Partial content vanishes (`streaming_active=false` hides it), subagents keep running, no feedback - UX clearly worse. Rejected. |
| C | Cooperative `CancellationToken` threaded through `AgentLoop` + all tool paths | More elegant but touches `src/tui/agent/` and `src/agent/` - disproportionate scope, inconsistent with existing abort pattern. Rejected. |

## 3. Selected Design (Approach A)

### 3.1 New `interrupt_running_turn()` (`src/tui/app/turn.rs`)

`cancel_current_turn()` stays a pure low-level abort (used by `/clear`). The new wrapper adds interrupt UX so `/clear`'s clean-slate semantics are untouched:

1. **Commit partial streaming** - mirror `StreamDone` (`event.rs:153`): if `streaming_content` is non-empty and not the `⏳` "preparing tools..." hint, push it as an `Assistant` `UIMessage`; then `streaming_content.clear()` + `streaming_active = false`.
2. **Finalize tool placeholder** - `has_running_tool = false`; if the last committed message is a `Tool` row with `tool_running == true`, set `tool_running = false` (stop spinner).
3. **Cancel root task** - call `cancel_current_turn()` (abort + `Idle` + `suppress_phase_updates = true` + `TurnAborted::Interrupted`).
4. **Cancel subagents** - replicate `/clear`'s async `reset_agent_generation` so daemon-side subagent subtrees are cancelled and the next turn adopts a fresh generation.
5. **Feedback** - push system message `⏹ Interrupted by user` via the existing `push_system_message` helper (hardcoded string, consistent with surrounding system messages like "Plan mode enabled").

### 3.2 ESC branch (`src/tui/app/event_key.rs`)

Insert the interrupt check **after all contextual ESC consumers** (which `return` early) and **before scroll/input handling**:

```
focus view -> BackTab -> Ctrl+P -> completion -> permission -> question
-> session popup -> status bar            // all early-return on ESC
-> [NEW] if Esc && current_turn_handle.is_some() { interrupt_running_turn(); return; }
-> PageUp/Down -> Ctrl+L -> Enter -> Ctrl+J -> @/-trigger -> textarea.input
-> [REMOVED] if !handled && Esc { should_quit = true }
```

Because permission/completion/focus/session/status-bar intercept ESC first, ESC during `AwaitingPermission` remains **Deny** and ESC with a popup dismisses the popup - both preserved.

### 3.3 Remove ESC-to-quit

Delete the `if !handled && key.code == KeyCode::Esc { self.should_quit = true; }` fallback. Idle ESC becomes a no-op. Quitting remains Ctrl+C double-press (`event.rs:777`, `CtrlCPressed`). No other quit path depends on ESC.

### 3.4 Interrupt signal

Gate on `current_turn_handle.is_some()` (not `phase.is_busy()`): the `JoinHandle` is the authoritative live-turn signal, set on spawn and cleared on `TurnComplete`, avoiding phase-stale races.

## 4. Risks & Mitigations

1. **Stale events re-activating streaming** (low): `handle.abort()` drops in-flight futures (reqwest stream, tool await), so no new content events are produced post-abort. Already-queued events process first (FIFO) and only enrich the partial content we then commit. `suppress_phase_updates` prevents phase flip-back. Same proven mechanism as `/clear`. *Mitigation not adopted now*: gating `ContentDelta`/`StreamDone`/`ToolResult` on `!suppress_phase_updates` would change existing behavior and expand scope - retained as a future observation item.
2. **Tool subprocess not hard-killed** (accepted limitation): `exec_command` subprocesses are independent `tokio::spawn`s; abort does not kill them. Best-effort; recorded as a non-goal.
3. **Double/rapid ESC**: first ESC aborts and clears `current_turn_handle`; subsequent ESC hits the idle branch (no-op). No quit risk.

## 5. Testing Strategy

`App::new(DaemonClient::new("http://localhost:0"), session, settings)` constructs an `App` in tests without a real daemon (`mod.rs:754`).

- **`interrupt_running_turn()` unit test** (`turn.rs` test mod): spawn a dummy long task as `current_turn_handle`, populate `streaming_content`, call the method; assert partial committed as `Assistant` message, `streaming_active == false`, `has_running_tool == false`, system message present, `current_turn_handle == None`, `phase == Idle`, `suppress_phase_updates == true`.
- **ESC branch routing test** (`event_key.rs` test mod): construct `App` + dummy handle, dispatch `KeyCode::Esc`; assert interrupt path ran (handle cleared, message present).
- **Idle ESC no-quit test**: no handle, dispatch `Esc`; assert `should_quit == false`.
- **Manual TUI verification**: ESC interrupts streaming (partial preserved, `⏹ Interrupted by user` shown, phase idle); interrupts tool execution and `/compact`; idle ESC no-op; ESC during permission still Denies; Ctrl+C double-press still quits.

## 6. Files Touched

| File | Change |
|------|--------|
| `src/tui/app/turn.rs` | Add `interrupt_running_turn()`; `cancel_current_turn()` unchanged. |
| `src/tui/app/event_key.rs` | Add ESC-interrupt branch; remove ESC-quit fallback. |
| `src/state/agent_phase.rs` | No change (reuses `TurnAbortReason::Interrupted`). |

No new dependencies, no daemon/API changes, no new `AppEvent` variants.

## 7. Open Questions

None - all resolved during requirement clarification and brainstorming.
