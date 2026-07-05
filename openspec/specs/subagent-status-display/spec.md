# subagent-status-display Specification

## Purpose
TBD - created by archiving change improve-subagent-progress-visibility. Update Purpose after archive.
## Requirements
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

### Requirement: Subagent panel shows per-node timing and token usage
The subagent overlay panel SHALL display elapsed time, token consumption, and token budget (when set) for each subagent node.

#### Scenario: Active subagent node
- **WHEN** a subagent node is in Running status with 3 rounds completed out of 20 max
- **THEN** the panel SHALL display "round 3/20 · 12.3s" next to the node

#### Scenario: Completed subagent node with token and budget data
- **WHEN** a subagent node has completed, token_count is 1500, and token_budget was 10000
- **THEN** the panel SHALL display "1.5k/10k tokens · 45.2s" next to the completed node

#### Scenario: Subagent node without token data
- **WHEN** a subagent node has completed but token_count is None
- **THEN** the panel SHALL display elapsed time but SHALL NOT display token information

#### Scenario: Subagent exceeded budget
- **WHEN** a subagent was killed due to token budget exhaustion
- **THEN** the node SHALL display status Failed with error "Budget exceeded" and SHALL show the budget limit vs actual usage

### Requirement: Subagent panel shows error details and recovery actions
Failed and Cancelled subagent nodes SHALL display error details and offer recovery actions (retry, view details).

#### Scenario: Failed node with error message
- **WHEN** a subagent node has status Failed with error "Subagent timed out after 240 seconds"
- **THEN** the panel SHALL display the error message in red beneath the node label

#### Scenario: Retry action for failed node
- **WHEN** a Failed node is selected in the subagent panel
- **THEN** the panel SHALL display a hint "[r] retry  [d] details" and pressing `r` SHALL respawn the subagent with the same prompt and context

#### Scenario: Retry includes previous error context
- **WHEN** a subagent is retried after failure
- **THEN** the respawned subagent's system prompt SHALL include a `previous_attempt_error` field describing what went wrong

#### Scenario: Rollback before retry for code-modifying subagent
- **WHEN** a failed subagent had modified files before failing
- **AND** user presses `r` to retry
- **THEN** the system SHALL git-stash or revert the partial changes before respawning

#### Scenario: Detail view for failed node
- **WHEN** user presses `d` on a Failed node
- **THEN** the full transcript detail view SHALL open, scrolled to the error event

### Requirement: Progress delta tracking
Each `SubagentProgress` event SHALL include a `progress_delta: Option<f32>` field indicating the estimated progress increment since the last update.

#### Scenario: Progress delta reported during execution
- **WHEN** a subagent completes a round and has new findings relative to previous rounds
- **THEN** `progress_delta` SHALL be > 0.0, calculated as new_findings / total_expected_findings

#### Scenario: No progress detected
- **WHEN** two consecutive rounds produce progress_delta < 0.05
- **THEN** the subagent loop SHALL emit a warning event; after three consecutive low-delta rounds, the subagent SHALL abort with `StuckStatus::NoProgress`

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

