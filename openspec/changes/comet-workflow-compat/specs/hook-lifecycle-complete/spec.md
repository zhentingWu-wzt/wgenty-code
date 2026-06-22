## ADDED Requirements

### Requirement: SessionStart hook fires on session creation
The system SHALL fire the `SessionStart` hook event immediately after a new TUI app session is fully initialized. The hook context SHALL carry the session ID and working directory.

#### Scenario: SessionStart hook blocks on startup error
- **WHEN** a `SessionStart` hook is configured that returns `{ "continue_execution": false }`
- **THEN** the hook outcome SHALL be logged as blocked
- **AND** the session SHALL still proceed (SessionStart hook does not prevent session creation)

#### Scenario: SessionStart context contains session info
- **WHEN** a `SessionStart` hook fires
- **THEN** the hook context JSON SHALL include `session_id`, `working_directory`, and `timestamp`

### Requirement: SessionEnd hook fires on session exit
The system SHALL fire the `SessionEnd` hook event before the daemon is shut down and the terminal is restored. The hook context SHALL carry the session ID.

#### Scenario: SessionEnd fires before daemon shutdown
- **WHEN** the user quits the TUI (double Ctrl+C or equivalent)
- **THEN** the `SessionEnd` hook SHALL fire before the daemon shutdown signal is sent
- **AND** hook execution SHALL complete (or timeout) before the process exits

### Requirement: UserPromptSubmit hook fires on every input submission
The system SHALL fire the `UserPromptSubmit` hook event when the user submits input in the TUI, AFTER built-in slash commands are handled but BEFORE the input is routed as a slash command or queued for the agent.

#### Scenario: Comet guard hook reads input before agent
- **WHEN** user submits `/comet add login feature` in the TUI input
- **THEN** the `UserPromptSubmit` hook SHALL fire with `tool_input` containing the raw input text
- **AND** hook execution SHALL complete before the input reaches `route_slash_command`

#### Scenario: UserPromptSubmit does not fire on built-in commands
- **WHEN** user submits `/clear` or `/help` (built-in commands)
- **THEN** the `UserPromptSubmit` hook SHALL NOT fire (built-ins are handled before hook)

### Requirement: Stop hook fires on turn completion or abort
The system SHALL fire the `Stop` hook event when an agent turn completes normally (`TurnComplete`) or is aborted (`TurnAborted`).

#### Scenario: Stop hook fires on normal turn completion
- **WHEN** an agent turn finishes without error and `TurnComplete` event is emitted
- **THEN** the `Stop` hook SHALL fire with the turn finish reason "stop"

#### Scenario: Stop hook fires on turn abort
- **WHEN** an agent turn is cancelled or exceeds max rounds and `TurnAborted` event is emitted
- **THEN** the `Stop` hook SHALL fire with the abort reason in the hook context

### Requirement: PermissionRequest hook fires before user permission prompt
The system SHALL fire the `PermissionRequest` hook event when a tool execution requires user permission (via `PermissionRequired` event), BEFORE the permission prompt is shown to the user.

#### Scenario: Hook can auto-deny permission without showing prompt
- **WHEN** a `PermissionRequest` hook is configured that returns `{ "continue_execution": false, "reason": "auto-denied by policy" }`
- **THEN** the permission prompt SHALL NOT be shown to the user
- **AND** the tool execution SHALL be denied with the hook's reason

### Requirement: Notification hook fires on comet guard phase events
The system SHALL fire the `Notification` hook event when comet phase guard detects a phase-related event (e.g., tool blocked by phase restriction, phase transition detected).

#### Scenario: Notification on phase-restricted tool attempt
- **WHEN** the agent attempts `file_write` during open/design phase
- **AND** comet phase guard blocks it
- **THEN** a `Notification` hook SHALL fire with subtype `comet_phase_block`
- **AND** the notification context SHALL include the blocked tool name, current phase, and reason
