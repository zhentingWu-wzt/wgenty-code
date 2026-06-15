# task-complexity-detection — Delta Spec

## MODIFIED Requirements

### Requirement: Task complexity detection uses structural analysis
The `is_complex_task()` function SHALL use structural analysis of the prompt to determine complexity. The function SHALL also classify the task type (analysis, modification, or mixed) to determine the appropriate structured output format.

#### Scenario: Simple single-step task
- **WHEN** the prompt is "create a file called config.json with default settings"
- **THEN** `is_complex_task()` SHALL return `false`, routing the task to direct execution

#### Scenario: Multi-step task with numbered steps
- **WHEN** the prompt contains numbered steps (e.g., "1. Refactor the auth module\n2. Update all callers\n3. Add tests") referencing multiple files
- **THEN** `is_complex_task()` SHALL return `true`, routing to RLM pipeline

#### Scenario: Task with explicit dependencies
- **WHEN** the prompt describes tasks where one depends on another (e.g., "after X completes, do Y")
- **THEN** `is_complex_task()` SHALL return `true`, routing to RLM pipeline

#### Scenario: Long but simple prompt
- **WHEN** the prompt is >1000 characters but describes a single straightforward operation
- **THEN** `is_complex_task()` SHALL NOT automatically classify it as complex based on length alone

## ADDED Requirements

### Requirement: Task type classification for structured output
The complexity detection SHALL classify tasks as `analysis`, `modification`, or `mixed` to determine which structured output format the subagent should produce.

#### Scenario: Analysis task
- **WHEN** the prompt primarily requests investigation, searching, or understanding (e.g., "find where authentication logic is implemented")
- **THEN** the task type SHALL be classified as `analysis` and the subagent SHALL use `structured-claims/1` output format

#### Scenario: Modification task
- **WHEN** the prompt primarily requests code changes (e.g., "refactor the auth module to use JWT")
- **THEN** the task type SHALL be classified as `modification` and the subagent SHALL use `unified-diff/1` output format

#### Scenario: Mixed task
- **WHEN** the prompt requests both investigation and code changes
- **THEN** the task type SHALL be classified as `mixed` and the subagent SHALL produce both claims and diffs

### Requirement: Routing decisions are logged and visible
When the `task` tool routes a prompt to RLM pipeline or direct subagent execution, the routing rationale and task type SHALL be included in the tool result metadata.

#### Scenario: Task routed to RLM with type classification
- **WHEN** a task is routed to the RLM pipeline
- **THEN** the tool result metadata SHALL include `routing_reason`, `task_type`, and `output_format` fields

#### Scenario: Task executed directly
- **WHEN** a task is executed as a direct subagent
- **THEN** the tool result SHALL indicate "direct execution" with task type in metadata

#### Scenario: TUI displays routing reason with format
- **WHEN** the tool result contains routing metadata
- **THEN** the TUI SHALL render the routing reason, task type, and output format as dimmed text near the subagent card
