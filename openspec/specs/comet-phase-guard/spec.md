# comet-phase-guard Specification

## Purpose
TBD - created by archiving change comet-workflow-compat. Update Purpose after archive.
## Requirements
### Requirement: Comet state reader detects active change and phase
The system SHALL read `openspec/changes/<name>/.comet.yaml` for the active change (if any) and extract the `phase` field. If no active change exists, the phase SHALL be `null` (unrestricted).

#### Scenario: Active change in open phase detected
- **WHEN** `openspec/changes/comet-workflow-compat/.comet.yaml` contains `phase: open`
- **THEN** `CometState::read()` SHALL return `phase: open` and `workflow: full`

#### Scenario: No active change, no restrictions
- **WHEN** no `openspec/changes/<name>/.comet.yaml` exists with a non-archived phase
- **THEN** `CometState::read()` SHALL return `phase: null`
- **AND** the comet guard SHALL allow all tool operations

#### Scenario: Multiple active changes
- **WHEN** multiple active changes exist (each with `.comet.yaml`)
- **THEN** comet state SHALL log a warning and use the first one found
- **AND** comet guard SHALL apply the most restrictive phase rules across all active changes

### Requirement: Phase guard blocks tools outside allowed set
The system SHALL, before executing any tool, check the current Comet phase (if active) against a tool allow/deny matrix and block tools not permitted in the current phase.

#### Scenario: file_write blocked in open phase
- **WHEN** comet phase is `open`
- **AND** agent calls `file_write` or `file_edit` targeting a source file
- **THEN** the tool SHALL be blocked with error message indicating phase restriction
- **AND** a `Notification` hook SHALL fire with subtype `comet_phase_block`

#### Scenario: file_read allowed in all phases
- **WHEN** comet phase is `open`, `design`, `build`, `verify`, or `archive`
- **AND** agent calls `file_read`
- **THEN** the tool SHALL execute normally without phase-related blocking

#### Scenario: git commit allowed in build phase
- **WHEN** comet phase is `build`
- **AND** agent calls `git_operations` with `operation: commit`
- **THEN** the tool SHALL execute normally

### Requirement: Phase guard bypass on explicit user approval
The system SHALL allow bypassing the phase guard when the user explicitly approves a blocked tool operation through the permission panel.

#### Scenario: User approves write in open phase
- **WHEN** comet phase is `open` and agent attempts `file_write`
- **AND** the phase guard blocks it and presents the block reason to the user
- **AND** user selects "Allow once" or "Always allow"
- **THEN** the tool SHALL execute

### Requirement: Comet guard integrates at ToolExecutor level
The system SHALL integrate the comet phase guard into `ToolExecutor::execute_with_hooks()`, running the guard check BEFORE `PreToolUse` hooks.

#### Scenario: Guard blocks before PreToolUse hooks
- **WHEN** a tool would be blocked by both comet guard and PreToolUse hook
- **THEN** the comet guard check SHALL run first and block the tool
- **AND** PreToolUse hooks SHALL NOT fire for the blocked tool

### Requirement: Phase context injected into agent system messages
The system SHALL, when an active Comet change exists, append phase context to the agent's system message indicating the current phase, workflow type, and restrictions.

#### Scenario: Agent receives phase context in build phase
- **WHEN** an active change is in `phase: build` with `build_mode: subagent-driven-development`
- **THEN** the system message SHALL include text indicating the current phase and mode
- **AND** the text SHALL reference the relevant Comet skill instructions

