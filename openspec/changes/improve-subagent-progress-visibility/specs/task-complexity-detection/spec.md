## ADDED Requirements

### Requirement: Task complexity detection uses structural analysis
The `is_complex_task()` function SHALL use structural analysis of the prompt to determine complexity, replacing the current keyword-counting heuristic. The function SHALL NOT use common English words as complexity signals.

#### Scenario: Simple single-step task
- **WHEN** the prompt is "create a file called config.json with default settings"
- **THEN** `is_complex_task()` SHALL return `false`, routing the task to direct execution without RLM pipeline

#### Scenario: Multi-step task with numbered steps
- **WHEN** the prompt contains numbered steps (e.g., "1. Refactor the auth module\n2. Update all callers\n3. Add tests") referencing multiple files
- **THEN** `is_complex_task()` SHALL return `true`, routing to RLM pipeline

#### Scenario: Task with explicit dependencies
- **WHEN** the prompt describes tasks where one depends on another (e.g., "after X completes, do Y")
- **THEN** `is_complex_task()` SHALL return `true`, routing to RLM pipeline

#### Scenario: Long but simple prompt
- **WHEN** the prompt is >1000 characters but describes a single straightforward operation (e.g., a detailed specification for a single function)
- **THEN** `is_complex_task()` SHALL NOT automatically classify it as complex based on length alone

### Requirement: Routing decisions are logged and visible
When the `task` tool routes a prompt to RLM pipeline or direct subagent execution, the routing rationale SHALL be included in the tool result metadata and rendered in the TUI chat area.

#### Scenario: Task routed to RLM
- **WHEN** a task is routed to the RLM pipeline
- **THEN** the tool result metadata SHALL include a `routing_reason` field explaining why (e.g., "multi-step: 5 numbered steps, 3 file references")

#### Scenario: Task executed directly
- **WHEN** a task is executed as a direct subagent (not RLM)
- **THEN** the tool result SHALL indicate "direct execution" in a dimmed line beneath the subagent card

#### Scenario: TUI displays routing reason
- **WHEN** the tool result contains a `routing_reason`
- **THEN** the TUI SHALL render it as dimmed text near the subagent card or tool message
