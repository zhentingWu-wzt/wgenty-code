## MODIFIED Requirements

### Requirement: Subagent status bar below input area
The TUI SHALL display a compact status bar between the main status line and the input box when subagents are active. The status bar SHALL list each active subagent with its status icon, label, and current tool call (when available), and SHALL support keyboard navigation to select a subagent for focus view entry. Pressing ↑ or ↓ while the status bar is visible SHALL auto-activate status bar focus and navigate the selection — no separate focus-toggle key (e.g., Tab) is required. Esc SHALL deactivate status bar focus and return focus to the input box. Tab SHALL have no effect on status bar focus.

#### Scenario: Status bar appears when subagents start
- **WHEN** the first subagent begins execution (status transitions to Running)
- **THEN** a status bar SHALL appear between the main status line and the input box, occupying the minimum height needed to display all active subagents (capped at 5 lines)

#### Scenario: Status bar shows each active subagent
- **WHEN** 3 subagents are Running with labels "explore", "plan", "general-purpose" and current tools `grep("fn auth")`, `file_read("src/mod.rs")`, and none respectively
- **THEN** the status bar SHALL display three lines, each showing: status icon + label + current tool (or "thinking…" if no tool yet)

#### Scenario: Arrow keys auto-activate and navigate the status bar
- **WHEN** the status bar is visible (subagents active) and the user presses ↑ or ↓
- **THEN** the status bar SHALL auto-activate focus (without requiring a prior Tab or other focus-toggle keypress)
- **AND** the selected subagent SHALL change, with the currently selected entry highlighted in a distinct color
- **AND** the selection SHALL wrap around from first to last and vice versa

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

#### Scenario: Status bar Enter triggers focus view
- **WHEN** the user presses Enter while a subagent is selected in the status bar
- **THEN** the TUI SHALL enter the full-screen focus view for that subagent

#### Scenario: Status bar does not interfere with text input
- **WHEN** the status bar is visible and the user types characters (other than ↑↓/Esc/Enter)
- **THEN** the characters SHALL go to the input box as normal
- **AND** typing any non-navigation character SHALL deactivate status bar focus (if active) and return focus to the input box
