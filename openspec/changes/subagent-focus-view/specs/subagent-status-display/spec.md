# subagent-status-display Specification

## ADDED Requirements

### Requirement: Subagent status bar below input area
The TUI SHALL display a compact status bar between the main status line and the input box when subagents are active. The status bar SHALL list each active subagent with its status icon, label, and current tool call (when available), and SHALL support keyboard navigation to select a subagent for focus view entry.

#### Scenario: Status bar appears when subagents start
- **WHEN** the first subagent begins execution (status transitions to Running)
- **THEN** a status bar SHALL appear between the main status line and the input box, occupying the minimum height needed to display all active subagents (capped at 5 lines)

#### Scenario: Status bar shows each active subagent
- **WHEN** 3 subagents are Running with labels "explore", "plan", "general-purpose" and current tools `grep("fn auth")`, `file_read("src/mod.rs")`, and none respectively
- **THEN** the status bar SHALL display three lines, each showing: status icon + label + current tool (or "thinking…" if no tool yet)

#### Scenario: Status bar supports selection navigation
- **WHEN** the status bar is visible and the user presses ↑ or ↓
- **THEN** the selected subagent SHALL change, with the currently selected entry highlighted in a distinct color
- **AND** the selection SHALL wrap around from first to last and vice versa

#### Scenario: Status bar hides when no subagents active
- **WHEN** all subagents have completed or failed and no new subagents are running
- **THEN** the status bar SHALL disappear and the input box SHALL reclaim the space

#### Scenario: Status bar Enter triggers focus view
- **WHEN** the user presses Enter while a subagent is selected in the status bar
- **THEN** the TUI SHALL enter the full-screen focus view for that subagent

#### Scenario: Status bar does not interfere with text input
- **WHEN** the status bar is visible and the user types characters
- **THEN** the characters SHALL go to the input box as normal, unless the user has explicitly navigated to the status bar with ↑↓
- **AND** pressing any non-navigation key SHALL return focus to the input box

## MODIFIED Requirements

### Requirement: Status bar shows subagent progress counters
The TUI main status line SHALL display a compact subagent progress summary when subagents are active. The detailed per-subagent information (current tool, label, timing) SHALL be shown in the subagent status bar below the input area, not in the main status line.

#### Scenario: Multiple subagents running
- **WHEN** 3 subagents are active and 5 have completed out of 8 total
- **THEN** the main status line SHALL display a compact summary like "Subagent 3 active · 5/8 done"
- **AND** the subagent status bar below the input SHALL list the 3 active subagents with their labels and current tools

#### Scenario: All subagents complete successfully
- **WHEN** all subagents have completed with no failures
- **THEN** the main status line SHALL display "N tasks done" where N is the total subagent count
- **AND** the subagent status bar SHALL be hidden

#### Scenario: Some subagents failed
- **WHEN** 2 subagents completed and 1 failed
- **THEN** the main status line SHALL display "2 done · 1 failed" with the failure count in red

#### Scenario: No subagents active
- **WHEN** no subagents are running and none have been used in the current turn
- **THEN** the main status line SHALL NOT display any subagent counter information
- **AND** the subagent status bar SHALL NOT be visible
