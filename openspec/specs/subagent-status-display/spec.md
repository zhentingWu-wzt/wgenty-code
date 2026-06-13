# subagent-status-display Specification

## Purpose
TBD - created by archiving change improve-subagent-progress-visibility. Update Purpose after archive.
## Requirements
### Requirement: Status bar shows subagent progress counters
The TUI status bar SHALL display real-time subagent progress counters when subagents are active, replacing the static "Subagent running…" label. The display SHALL include the number of active, completed, and failed subagents.

#### Scenario: Multiple subagents running
- **WHEN** 3 subagents are active and 5 have completed out of 8 total
- **THEN** the status bar SHALL display a label like "3 active · 5/8 done" instead of a static "Subagent running…"

#### Scenario: All subagents complete successfully
- **WHEN** all subagents have completed with no failures
- **THEN** the status bar SHALL display "N tasks done" where N is the total subagent count

#### Scenario: Some subagents failed
- **WHEN** 2 subagents completed and 1 failed
- **THEN** the status bar SHALL display "2 done · 1 failed" with the failure count visually distinct (e.g., red)

#### Scenario: No subagents active
- **WHEN** no subagents are running and none have been used in the current turn
- **THEN** the status bar SHALL NOT display any subagent counter information

### Requirement: Subagent panel shows per-node timing and token usage
The subagent overlay panel SHALL display elapsed time for each subagent node and SHALL display token consumption when available.

#### Scenario: Active subagent node
- **WHEN** a subagent node is in Running status with 3 rounds completed out of 10 max
- **THEN** the panel SHALL display "round 3/10 · 12.3s" next to the node

#### Scenario: Completed subagent node with token data
- **WHEN** a subagent node has completed and token_count is populated with 1500 tokens
- **THEN** the panel SHALL display "1.5k tokens" next to the completed node

#### Scenario: Subagent node without token data
- **WHEN** a subagent node has completed but token_count is None
- **THEN** the panel SHALL display elapsed time but SHALL NOT display token information

