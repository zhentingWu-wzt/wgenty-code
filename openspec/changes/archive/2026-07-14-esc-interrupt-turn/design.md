# Design: ESC Interrupt Running Turn

## Context

The TUI REPL spawns each agent turn as a tokio task stored in `App::current_turn_handle`. A turn streams an LLM response, executes tools, and may spawn subagents. Today there is **no keyboard shortcut** to stop a running turn; the only cancellation path is `/clear` (which wipes the conversation). ESC currently quits the whole app (`event_key.rs` `if !handled && Esc { should_quit = true }`).

Cancellation machinery already exists:
- `cancel_current_turn()` (`turn.rs:328`) - aborts the JoinHandle, sets `phase = Idle`, `suppress_phase_updates = true`, emits `AppEvent::TurnAborted { reason: Interrupted }`.
- `TurnAbortReason::Interrupted` (`agent_phase.rs:80`).
- The `TurnAborted` handler (`event.rs:423`) clears the subagent tree and records `last_abort_reason`.

The gap is purely the **key binding and interrupt-specific UX** (preserve partial output, surface feedback, cancel subagents), not the abort primitive.

## Goals & Non-Goals

**Goals**
- ESC interrupts a running turn (streaming / thinking / executing tool / connecting / preparing tools / compacting).
- Partial streamed text stays visible; a `⏹ Interrupted by user` system message is shown.
- ESC no longer quits; quit stays Ctrl+C double-press.
- Running subagents are cancelled too (no wasted daemon-side work).

**Non-Goals**
- Hard-killing tool subprocesses (best-effort task abort only).
- Changing contextual panels' ESC semantics (focus view, completion, permission=Deny, question, session popup, status bar all keep priority).
- daemon / CLI query modes.

## Design Decisions

### D1: Interrupt signal = `current_turn_handle.is_some()`

Use the presence of the turn `JoinHandle` rather than `phase.is_busy()` as the "a turn is running" predicate. `current_turn_handle` is set on spawn and cleared on `TurnComplete`, making it the most accurate live-turn signal and avoiding phase-stale races (phase is derived from events and can lag).

### D2: New `interrupt_running_turn()` wrapper, keep `cancel_current_turn()` low-level

`cancel_current_turn()` stays a pure abort (used by `/clear`). A new `interrupt_running_turn()` wraps it with interrupt-specific UX:
1. **Finalize streaming** - mirror `StreamDone` (`event.rs:153`): commit non-empty, non-hint `streaming_content` as an `Assistant` `UIMessage`, then `streaming_content.clear()` + `streaming_active = false`.
2. **Finalize tool placeholder** - `has_running_tool = false`; if the last committed message is a running `Tool` placeholder, set `tool_running = false` (stop spinner).
3. **Cancel root task** - call `cancel_current_turn()` (abort + `Idle` + `suppress_phase_updates = true` + `TurnAborted::Interrupted`).
4. **Cancel subagents** - replicate `/clear`'s async `reset_agent_generation` (`input.rs:65-83`) so daemon-side subagent subtrees are cancelled and the next turn adopts a fresh generation.
5. **Feedback** - push system message `⏹ Interrupted by user` via the existing `push_system_message` helper.

Rationale for not folding this into `cancel_current_turn()`: `/clear` clears messages/streaming *before* calling cancel and runs its *own* generation reset, so adding a message + reset inside cancel would corrupt `/clear`'s clean-slate semantics.

### D3: ESC branch placement in `handle_key_event`

Insert the interrupt check **after all contextual ESC consumers** (which `return` early) and **before scroll/input handling**:

```
focus view -> BackTab -> Ctrl+P -> completion -> permission -> question
-> session popup -> status bar  // all early-return on ESC
-> [NEW] if Esc && current_turn_handle.is_some() { interrupt_running_turn(); return; }
-> PageUp/Down -> Ctrl+L -> Enter -> Ctrl+J -> @/-trigger -> textarea.input
-> [REMOVED] if !handled && Esc { should_quit = true }
```

Because permission/question/completion/focus/session/status-bar all intercept ESC first, ESC during `AwaitingPermission` remains **Deny** (not whole-turn interrupt) and ESC with a popup open dismisses the popup - both preserved.

### D4: Remove ESC-to-quit

Delete the `if !handled && key.code == KeyCode::Esc { self.should_quit = true; }` fallback. When idle and the textarea does not consume ESC, ESC becomes a no-op. Quitting remains Ctrl+C double-press (`event.rs:777`, `CtrlCPressed`). No other quit path depends on ESC.

### D5: Stale-event safety

`handle.abort()` drops the in-flight futures (reqwest stream, tool await), so **no new content events are produced after abort**. Events already enqueued before the `KeyEvent` are processed first (FIFO unbounded channel) and only enrich the partial content we then commit - harmless. `suppress_phase_updates = true` prevents any residual phase-changing event from flipping the status bar back to "Thinking". This is the same proven mechanism `/clear` relies on.

## Edge Cases

- **Interrupt during streaming**: partial text committed as Assistant message; spinner/indicator stops.
- **Interrupt during tool execution**: tool placeholder finalized (`tool_running = false`); subprocess may linger (known limitation).
- **Interrupt during `AwaitingPermission`**: ESC = Deny (permission panel intercepts); the turn is not aborted. Intentional - the user can then re-evaluate.
- **Interrupt during `/compact`**: `Compacting` phase has a live `current_turn_handle`; ESC aborts it (best-effort).
- **Idle ESC**: no-op (no turn, no quit).
- **Double-ESC / rapid presses**: first ESC aborts and clears `current_turn_handle`; subsequent ESC hits the idle branch (no-op). No quit risk.

## Components Touched

| File | Change |
|------|--------|
| `src/tui/app/turn.rs` | Add `interrupt_running_turn()`; `cancel_current_turn()` unchanged. |
| `src/tui/app/event_key.rs` | Add ESC-interrupt branch (D3); remove ESC-quit fallback (D4). |
| `src/state/agent_phase.rs` | No change (reuses `TurnAbortReason::Interrupted`). |

No new dependencies, no daemon/API changes, no new `AppEvent` variants (reuses `TurnAborted` + `AgentGenerationReset`).

## Open Questions

None - all resolved during requirement clarification.
