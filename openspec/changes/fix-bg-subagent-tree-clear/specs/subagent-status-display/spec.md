## MODIFIED Requirements

### Requirement: Subagent tree lifecycle across submitted prompts
The subagent tree (and status bar) SHALL persist across prompt submissions while a turn is still running. Clearing the tree SHALL occur at the start of a new turn (TurnStarted) and on turn abort (TurnAborted, covering /clear and turn failures), NOT at prompt submission time. This ensures that submitting a new prompt while subagents are running does not hide the running subagents or block entering the focus view. At TurnStarted, the tree SHALL only be cleared when no subagents are still active (Running/Pending); background subagents (task tool background mode) that outlive the main turn SHALL be preserved so they remain visible and selectable across turn boundaries.

#### Scenario: Submitting a prompt while a turn is running preserves the tree
- **WHEN** a turn is running with active subagents and the user submits a new prompt
- **THEN** the new prompt SHALL be queued (pending inputs)
- **AND** the subagent tree and status bar SHALL remain visible (not cleared)
- **AND** the user SHALL still be able to enter the focus view for a running subagent

#### Scenario: Submitting a prompt while a background subagent runs preserves the tree
- **WHEN** a background subagent (task tool background mode) is running and the main turn has completed, and the user submits a new prompt
- **THEN** the new turn SHALL start (TurnStarted)
- **AND** the subagent tree SHALL NOT be cleared because an active (Running/Pending) subagent still exists
- **AND** the background subagent SHALL remain in the tree, the status bar SHALL remain visible, and the user SHALL still be able to enter its focus view
- **AND** the focus view, status bar selection, and completion timestamps SHALL be preserved (not reset)

#### Scenario: New turn start clears the tree only when no subagents are active
- **WHEN** a new turn begins (TurnStarted) and no subagents are still active (Running/Pending)
- **THEN** the subagent tree, completion timestamps, focus view, and status bar selection SHALL be reset to a fresh state for the new turn

#### Scenario: Turn abort clears the tree
- **WHEN** a turn is aborted (TurnAborted), e.g. via /clear or a turn failure
- **THEN** the subagent tree, completion timestamps, focus view, and status bar selection SHALL be cleared so stale subagents do not linger in the status bar
