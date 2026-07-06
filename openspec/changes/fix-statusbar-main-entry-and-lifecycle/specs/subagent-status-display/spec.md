## MODIFIED Requirements

### Requirement: Subagent status bar below input area
The TUI SHALL display a compact status bar between the main status line and the input box when subagents are active. The status bar SHALL list a "main" placeholder entry followed by each active subagent with its status icon, label, and current tool call (when available), and SHALL support keyboard navigation to select either "main" or a subagent. Pressing ↑ or ↓ while the status bar is visible SHALL auto-activate status bar focus and navigate the selection — no separate focus-toggle key (e.g., Tab) is required. Esc SHALL deactivate status bar focus and return focus to the input box. Tab SHALL have no effect on status bar focus. The "main" entry mirrors the focus view selector's "main" entry so the two selectors stay consistent.

#### Scenario: Status bar appears when subagents start
- **WHEN** the first subagent begins execution (status transitions to Running)
- **THEN** a status bar SHALL appear between the main status line and the input box, occupying the minimum height needed to display the "main" entry plus all active subagents (capped at 5 subagent lines)

#### Scenario: Status bar shows main plus each active subagent
- **WHEN** 3 subagents are Running with labels "explore", "plan", "general-purpose" and current tools `grep("fn auth")`, `file_read("src/mod.rs")`, and none respectively
- **THEN** the status bar SHALL display four lines: a "main" placeholder line followed by three subagent lines, each subagent line showing: status icon + label + current tool (or "thinking…" if no tool yet)
- **AND** the "main" line SHALL be selectable via arrow keys, distinct from the subagent entries

#### Scenario: Arrow keys auto-activate and navigate the status bar including main
- **WHEN** the status bar is visible (subagents active) and the user presses ↑ or ↓
- **THEN** the status bar SHALL auto-activate focus (without requiring a prior Tab or other focus-toggle keypress)
- **AND** the selected entry SHALL change across the unified list ["main", ...active subagents], with the currently selected entry highlighted in a distinct color
- **AND** the selection SHALL wrap around from "main" to the last subagent and vice versa (wrap length = active subagent count + 1)

#### Scenario: Esc deactivates status bar focus
- **WHEN** the status bar is focused (navigated) and the user presses Esc
- **THEN** the status bar SHALL deactivate focus and return focus to the input box
- **AND** the status bar SHALL remain visible as long as subagents are still active

#### Scenario: Tab does not toggle status bar focus
- **WHEN** the status bar is visible and the user presses Tab
- **THEN** Tab SHALL have no effect on status bar focus (it SHALL NOT toggle focus into or out of the status bar)

#### Scenario: Status bar hides when no subagents active
- **WHEN** all subagents have completed or failed and no new subagents are running
- **THEN** the status bar SHALL disappear and the input box SHALL reclaim the space

#### Scenario: Status bar Enter on a subagent triggers focus view
- **WHEN** the user presses Enter while a subagent (not "main") is selected in the status bar
- **THEN** the TUI SHALL enter the full-screen focus view for that subagent

#### Scenario: Status bar Enter on main dismisses focus
- **WHEN** the user presses Enter while the "main" entry is selected in the status bar
- **THEN** the TUI SHALL deactivate status bar focus and return focus to the input box
- **AND** the TUI SHALL remain in the main chat (no focus view is opened)
- **AND** the status bar SHALL remain visible as long as subagents are still active

#### Scenario: Status bar does not interfere with text input
- **WHEN** the status bar is visible and the user types characters (other than ↑↓/Esc/Enter)
- **THEN** the characters SHALL go to the input box as normal
- **AND** typing any non-navigation character SHALL deactivate status bar focus (if active) and return focus to the input box

### Requirement: Subagent tree lifecycle across submitted prompts
The subagent tree (and status bar) SHALL persist across prompt submissions while a turn is still running. Clearing the tree SHALL occur at the start of a new turn (TurnStarted) and on turn abort (TurnAborted, covering /clear and turn failures), NOT at prompt submission time. This ensures that submitting a new prompt while subagents are running does not hide the running subagents or block entering the focus view.

#### Scenario: Submitting a prompt while a turn is running preserves the tree
- **WHEN** a turn is running with active subagents and the user submits a new prompt
- **THEN** the new prompt SHALL be queued (pending inputs)
- **AND** the subagent tree and status bar SHALL remain visible (not cleared)
- **AND** the user SHALL still be able to enter the focus view for a running subagent

#### Scenario: New turn start clears the tree
- **WHEN** a new turn begins (TurnStarted), whether immediately on submit or after a queued prompt starts
- **THEN** the subagent tree, completion timestamps, focus view, and status bar selection SHALL be reset to a fresh state for the new turn

#### Scenario: Turn abort clears the tree
- **WHEN** a turn is aborted (TurnAborted), e.g. via /clear or a turn failure
- **THEN** the subagent tree, completion timestamps, focus view, and status bar selection SHALL be cleared so stale subagents do not linger in the status bar
