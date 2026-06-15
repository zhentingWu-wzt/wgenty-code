# subagent-status-display Specification

## Purpose
TBD - created by archiving change improve-subagent-progress-visibility. Update Purpose after archive.
## Requirements
### Requirement: Status bar shows subagent progress counters
The TUI status bar SHALL display real-time subagent progress counters when subagents are active, including the number of active, completed, and failed subagents. Failed count SHALL be visually distinct (red).

#### Scenario: Multiple subagents running
- **WHEN** 3 subagents are active and 5 have completed out of 8 total
- **THEN** the status bar SHALL display a label like "3 active · 5/8 done" instead of a static "Subagent running…"

#### Scenario: All subagents complete successfully
- **WHEN** all subagents have completed with no failures
- **THEN** the status bar SHALL display "N tasks done" where N is the total subagent count

#### Scenario: Some subagents failed
- **WHEN** 2 subagents completed and 1 failed
- **THEN** the status bar SHALL display "2 done · 1 failed" with the failure count in red

#### Scenario: No subagents active
- **WHEN** no subagents are running and none have been used in the current turn
- **THEN** the status bar SHALL NOT display any subagent counter information

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

