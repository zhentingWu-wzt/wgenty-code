## ADDED Requirements

### Requirement: execute_command respects user-specified timeout
The `execute_command` tool SHALL respect the `timeout` field in its input schema. The outer agent loop SHALL NOT impose a hard 120s timeout that overrides a user-specified longer timeout.

#### Scenario: execute_command with 600s timeout runs to completion
- **WHEN** agent calls `execute_command` with `{ "command": "cargo test --all", "timeout": 600 }`
- **THEN** the outer agent loop timeout SHALL be at least `max(args.timeout + 30, 120)` = 630s
- **AND** the command SHALL be allowed to run for up to 600 seconds without timeout

#### Scenario: execute_command without explicit timeout uses default
- **WHEN** agent calls `execute_command` with `{ "command": "echo hello" }` (no timeout field)
- **THEN** the outer agent loop timeout SHALL default to 120s
- **AND** the sandbox/execution layer SHALL use its own default (60s)

### Requirement: Task and delegate tools retain independent timeout
The `task` and `delegate` tools SHALL continue to have their own timeout (300s). This timeout SHALL NOT be affected by the execute_command timeout logic.

#### Scenario: task tool has 300s timeout
- **WHEN** agent calls the `task` tool to spawn a subagent
- **THEN** the outer agent loop timeout SHALL be 300s
- **AND** this SHALL NOT be overridden by any execute_command timeout configuration

### Requirement: Timeout resolution is centralized
The system SHALL provide a single function `resolve_tool_timeout(tool_name, args)` used by the agent loop to determine per-tool timeout. This SHALL replace the current inline ternary expression.

#### Scenario: Centralized timeout logic used
- **WHEN** any tool is executed in the agent loop
- **THEN** `resolve_tool_timeout` SHALL be called to determine the timeout
- **AND** the timeout logic SHALL NOT be duplicated inline

### Requirement: Timeout documentation in input schema
The `execute_command` tool's `input_schema` SHALL document the `timeout` field clearly, stating it is in seconds and the default is 60s.

#### Scenario: Schema describes timeout field
- **WHEN** the `execute_command` tool definition is retrieved
- **THEN** the `timeout` field description SHALL state "Timeout in seconds (optional, default: 60)"
