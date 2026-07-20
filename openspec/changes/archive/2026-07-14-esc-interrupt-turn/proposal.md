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
