# subagent-status-display Delta Specification

## ADDED Requirements

### Requirement: Task Panel shows subagent-specific metadata
The Task Panel (Ctrl+T) SHALL distinguish subagent tasks from regular tasks by displaying a subagent icon (🤖), subagent type, token usage, round count, and elapsed duration when the task originates from a subagent.

#### Scenario: Subagent task visible in Task Panel
- **WHEN** a subagent task is created with subagent metadata (type="explore", tokens=2500, rounds=3, duration_ms=12300)
- **THEN** the Task Panel SHALL display "🤖 explore · 3r · 12.3s · 2.5k tokens" with the subagent icon in a distinct color

#### Scenario: Regular task in Task Panel
- **WHEN** a regular task (non-subagent) is displayed in the Task Panel
- **THEN** the Task Panel SHALL display it with the existing task icon and label format, unchanged from current behavior

#### Scenario: Backward compatibility
- **WHEN** the daemon sends TodoItem data without the `subagent` field
- **THEN** the Task Panel SHALL render the item as a regular task without errors
