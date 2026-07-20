# Comet Design Handoff

- Change: esc-interrupt-turn
- Phase: design
- Mode: compact
- Context hash: 95b55d0bf9f37a8dc3ce081333b0a19a07cacad833c7cde790d11b141c89580d

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/esc-interrupt-turn/proposal.md

- Source: openspec/changes/esc-interrupt-turn/proposal.md
- Lines: 1-27
- SHA256: c3f906a798ff689918d88ea842f42de9829151be98e837aaaba1aa46245180a9

```md
## Why

The TUI REPL has no keyboard shortcut to interrupt a running agent turn. The only way to stop a generation/tool-execution in progress is `/clear`, which also wipes the conversation. Meanwhile ESC currently quits the entire application — a destructive, surprising action mid-turn. Users need a fast, non-destructive way to cancel the current turn (matching Claude Code's ESC-to-interrupt behavior).

## What Changes

- ESC, when an agent turn is actively running, SHALL interrupt that turn: abort the spawned agent task, return the phase to `Idle`, stop the streaming indicator, and surface a `⏹ Interrupted by user` system message. Already-generated partial content stays visible.
- ESC SHALL no longer quit the application. Quitting remains via the existing Ctrl+C double-press.
- Contextual panels that already consume ESC (focus view, completion panel, permission panel = Deny, question panel, session popup, status bar) keep their existing priority and semantics — they intercept ESC before the interrupt logic.
- The interrupt reuses the existing `cancel_current_turn()` machinery (`handle.abort()` + `TurnAborted::Interrupted`), supplementing it with `streaming_active = false` and a user-facing system message.
- `/compact` compaction turns are also interruptible by ESC (best-effort).
- **BREAKING**: ESC no longer quits the app; users who relied on ESC-to-quit must use Ctrl+C double-press instead.

## Capabilities

### New Capabilities
- `tui-turn-interruption`: ESC-key interruption of a running agent turn in the TUI REPL, including interrupt feedback and the removal of ESC-to-quit.

### Modified Capabilities
<!-- None. No existing spec covers TUI turn lifecycle or keyboard interrupt. -->

## Impact

- **Code**: `src/tui/app/event_key.rs` (new ESC-interrupt branch in `handle_key_event`, removal of ESC-quit fallback), `src/tui/app/turn.rs` (`cancel_current_turn` extended to reset streaming + emit feedback message), `src/state/agent_phase.rs` (no schema change; reuses `TurnAbortReason::Interrupted`).
- **APIs/Dependencies**: None new. Reuses `crossterm` event handling and existing `AppEvent::TurnAborted`.
- **Systems**: TUI only. No daemon, CLI query, or backend changes. Subprocesses spawned by tools are not hard-killed (best-effort task abort; known limitation).
- **UX**: ESC semantics change (interrupt-only, no quit). Existing Ctrl+C double-press quit path is unaffected.
```

## openspec/changes/esc-interrupt-turn/design.md

- Source: openspec/changes/esc-interrupt-turn/design.md
- Lines: 1-87
- SHA256: 01d8a948e15f5564c5fba20521b84931f275f63a62e93bc17dc8efd57a632e11

[TRUNCATED]

```md
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
```

Full source: openspec/changes/esc-interrupt-turn/design.md

## openspec/changes/esc-interrupt-turn/tasks.md

- Source: openspec/changes/esc-interrupt-turn/tasks.md
- Lines: 1-21
- SHA256: 6e10fc3a689fa9c7eff2ac970fec36a1df6997f693e6600a0c5697c820944685

```md
# Implementation Tasks: ESC Interrupt Running Turn

## 1. Interrupt primitive & UX (`src/tui/app/turn.rs`)

- [ ] 1.1 Add `interrupt_running_turn(&mut self)` method to `App`: commit non-empty/non-hint `streaming_content` as an `Assistant` `UIMessage`, then clear `streaming_content` and set `streaming_active = false` (mirror `StreamDone` in `event.rs:153`)
- [ ] 1.2 In `interrupt_running_turn`, finalize a running tool placeholder: set `has_running_tool = false` and, if the last committed message is a `Tool` row with `tool_running == true`, set its `tool_running = false`
- [ ] 1.3 In `interrupt_running_turn`, call `cancel_current_turn()` to abort the task, set phase `Idle`, and emit `TurnAborted::Interrupted`
- [ ] 1.4 In `interrupt_running_turn`, replicate `/clear`'s async `reset_agent_generation` (`input.rs:65-83`) to cancel daemon-side subagents and adopt a fresh generation
- [ ] 1.5 In `interrupt_running_turn`, push system message `⏹ Interrupted by user` via `push_system_message`

## 2. Key binding wiring (`src/tui/app/event_key.rs`)

- [ ] 2.1 Add ESC-interrupt branch in `handle_key_event`, placed after all contextual panel handlers (focus view, completion, permission, question, session popup, status bar) and before scroll/input handling: `if key.code == KeyCode::Esc && self.current_turn_handle.is_some() { self.interrupt_running_turn(); return; }`
- [ ] 2.2 Remove the ESC-to-quit fallback (`if !handled && key.code == KeyCode::Esc { self.should_quit = true; }`) so ESC no longer quits; quit remains Ctrl+C double-press

## 3. Verification

- [ ] 3.1 Add or update unit/integration tests covering: ESC with a live `current_turn_handle` calls the interrupt path; ESC with no live handle does not quit; permission panel still intercepts ESC before the interrupt branch
- [ ] 3.2 Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` (zero warnings)
- [ ] 3.3 Run `cargo test --all` (all tests pass)
- [ ] 3.4 Manual TUI verification: ESC interrupts a streaming turn (partial text preserved, `⏹ Interrupted by user` shown, phase returns to idle); ESC interrupts tool execution and `/compact`; idle ESC is a no-op (no quit); ESC during a permission prompt still Denies; Ctrl+C double-press still quits
```

## openspec/changes/esc-interrupt-turn/specs/tui-turn-interruption/spec.md

- Source: openspec/changes/esc-interrupt-turn/specs/tui-turn-interruption/spec.md
- Lines: 1-99
- SHA256: 322bb062347f53cb484a1be594355fb006447bd24676b50f40c1ff6c06790c5d

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: ESC interrupts a running agent turn

The TUI REPL SHALL interrupt the currently running agent turn when the user presses ESC and a turn task is live (`current_turn_handle` is present). Interrupting SHALL abort the turn's spawned task, set the agent phase to `Idle`, and enable `suppress_phase_updates` so residual phase-changing events from the aborted task do not flip the status bar back to a busy state.

#### Scenario: ESC interrupts during streaming response

- **WHEN** an agent turn is streaming an LLM response (`current_turn_handle` is present) and the user presses ESC
- **THEN** the turn task SHALL be aborted
- **AND** the agent phase SHALL become `Idle`
- **AND** the streaming indicator SHALL stop (`streaming_active = false`)

#### Scenario: ESC interrupts during tool execution

- **WHEN** an agent turn is executing a tool (`current_turn_handle` is present, a tool placeholder is running) and the user presses ESC
- **THEN** the turn task SHALL be aborted
- **AND** the running tool placeholder SHALL be finalized (`tool_running = false`) so the spinner stops
- **AND** the agent phase SHALL become `Idle`

#### Scenario: ESC interrupts during compaction

- **WHEN** a `/compact` turn is running (`current_turn_handle` is present, phase is `Compacting`) and the user presses ESC
- **THEN** the compaction turn SHALL be aborted (best-effort)
- **AND** the agent phase SHALL become `Idle`

#### Scenario: ESC while idle does nothing

- **WHEN** no turn is running (`current_turn_handle` is absent) and the user presses ESC
- **THEN** no turn SHALL be interrupted
- **AND** the application SHALL NOT quit

### Requirement: Interrupt preserves partial streamed content

When ESC interrupts a turn that has produced partial streamed text, the system SHALL commit the non-empty, non-hint `streaming_content` as an `Assistant` chat message before clearing the streaming buffer, so the user can see what was generated before the interruption.

#### Scenario: Partial response remains visible after interrupt

- **WHEN** a turn has streamed partial text into `streaming_content` and the user presses ESC
- **THEN** the partial text SHALL be committed as an `Assistant` message in the chat
- **AND** `streaming_content` SHALL be cleared and `streaming_active` SHALL be false
- **AND** the partial text SHALL remain visible in the conversation

#### Scenario: No partial content leaves no artifact

- **WHEN** a turn has no streamed text (empty `streaming_content` or only the "preparing tools..." hint) and the user presses ESC
- **THEN** no `Assistant` message SHALL be committed from the streaming buffer
- **AND** `streaming_active` SHALL be false

### Requirement: Interrupt surfaces user feedback

The system SHALL surface a `⏹ Interrupted by user` system message in the chat when a turn is interrupted via ESC, so the user has a clear visual confirmation that the interrupt was applied.

#### Scenario: System message shown on interrupt

- **WHEN** the user presses ESC and a running turn is interrupted
- **THEN** a system message with content `⏹ Interrupted by user` SHALL be appended to the committed messages

### Requirement: Interrupt cancels running subagents

When ESC interrupts a turn, the system SHALL advance the agent generation on the daemon (via `reset_agent_generation`) so that daemon-side subagent subtrees belonging to the interrupted turn are cancelled, and the next turn adopts a fresh generation.

#### Scenario: Subagents cancelled on interrupt

- **WHEN** a turn with running subagents is interrupted via ESC
- **THEN** the daemon agent generation SHALL be advanced
- **AND** the subagent tree SHALL be cleared from the UI
- **AND** the next turn SHALL use the new generation

### Requirement: ESC no longer quits the application

The TUI REPL SHALL NOT quit when ESC is pressed. The previous ESC-to-quit fallback SHALL be removed. Quitting the application SHALL remain via the existing Ctrl+C double-press within 500ms.

#### Scenario: ESC does not quit when idle

- **WHEN** no turn is running and no contextual panel is open and the user presses ESC
- **THEN** the application SHALL NOT quit
- **AND** `should_quit` SHALL remain false

#### Scenario: Ctrl+C double-press still quits
```

Full source: openspec/changes/esc-interrupt-turn/specs/tui-turn-interruption/spec.md

