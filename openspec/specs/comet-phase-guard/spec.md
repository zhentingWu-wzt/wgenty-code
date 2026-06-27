# comet-phase-guard Specification

## Purpose
TBD - created by archiving change comet-workflow-compat. Update Purpose after archive.
## Requirements
### Requirement: Comet state reader detects active change and phase

The system SHALL delegate state discovery to the generic `StateSource` configured in the Comet `workflow.yaml`. The `ScriptStateSource` SHALL run the configured discovery script and return structured state. The `CometState` struct and `CometState::read()` function SHALL be removed from Rust code.

#### Scenario: Active change in open phase detected via script

- **WHEN** the Comet workflow YAML defines `state.read.script: "${COMET_STATE} current --json"`
- **AND** that script outputs `{"change": "fix-bug", "phase": "open"}`
- **THEN** the `WorkflowEngine` SHALL set the current state to `open`
- **AND** the change name SHALL be available as a template variable `${CHANGE}`

#### Scenario: No active change — state is null

- **WHEN** the state discovery script returns `{"change": null, "phase": null}`
- **THEN** the `WorkflowEngine` SHALL report no active workflow state
- **AND** the routing rules for `changes.count: 0` SHALL match

#### Scenario: Multiple active changes

- **WHEN** the state discovery script returns multiple active changes
- **THEN** the `WorkflowEngine` SHALL present them for user selection via the configured routing rule (`select_or_new` interaction)
- **AND** the agent SHALL NOT auto-select a change

### Requirement: Phase guard blocks tools outside allowed set

The system SHALL replace the hardcoded `CometGuard::check()` with a `RuleBasedGuard` in the `GuardPipeline`, configured via `workflow.yaml` `tool_guards`. The guard SHALL read the deny rules from YAML rather than a hardcoded match statement.

#### Scenario: file_write blocked in open state via YAML rule

- **WHEN** the Comet workflow YAML defines `tool_guards: [{ type: deny_in_states, states: [open, design, verify, archive], tools: [file_write, file_edit, apply_patch] }]`
- **AND** the current state is `open`
- **AND** agent calls `file_write`
- **THEN** the `RuleBasedGuard` SHALL return `Deny` with the configured message
- **AND** a `RuntimeEvent::GuardDecision` event SHALL be emitted

#### Scenario: file_read allowed in all states

- **WHEN** the `deny_in_states` rule does not list `file_read`
- **AND** agent calls `file_read`
- **THEN** the guard SHALL return `Allow`

#### Scenario: Read-only bash commands allowed in non-build states

- **WHEN** `tool_guards` defines `unless_command_matches: ["git status", "git log *", "ls *", "cat *", "find *", "grep *"]` for bash commands
- **AND** the current state is `open`
- **AND** agent runs `git status`
- **THEN** the guard SHALL return `Allow`

### Requirement: Phase guard bypass on explicit user approval
The system SHALL allow bypassing the phase guard when the user explicitly approves a blocked tool operation through the permission panel.

#### Scenario: User approves write in open phase
- **WHEN** comet phase is `open` and agent attempts `file_write`
- **AND** the phase guard blocks it and presents the block reason to the user
- **AND** user selects "Allow once" or "Always allow"
- **THEN** the tool SHALL execute

### Requirement: Comet guard integrates at ToolExecutor level

The system SHALL replace the hardcoded `CometGuard::check()` call in `ToolExecutor::execute_with_hooks()` with the generic `GuardPipeline::evaluate()`. The pipeline SHALL be constructed from `workflow.yaml` `tool_guards` rules at session start.

#### Scenario: Guard pipeline runs before PreToolUse hooks

- **WHEN** the `GuardPipeline` contains a `RuleBasedGuard` that would block a tool
- **THEN** the pipeline SHALL block the tool before any `PreToolUse` hooks execute
- **AND** a `Notification` hook SHALL fire with the block reason

#### Scenario: No workflow active — pipeline is empty

- **WHEN** no active workflow exists (no `workflow.yaml` with matching `entry_commands`)
- **THEN** the `GuardPipeline` SHALL be empty (all tools pass through)
- **AND** no guard-related events SHALL be emitted

### Requirement: Phase context injected into agent system messages

The system SHALL replace the hardcoded Layer 1b in `src/prompts/mod.rs` with `ContextAssembler` output from the `workflow.yaml` `context` definitions. Phase-specific text SHALL be defined as YAML templates, not Rust `phase_instruction()` match arms.

#### Scenario: Agent receives phase context via ContextAssembler

- **WHEN** the Comet workflow YAML defines a context layer with `visibility: internal` and `template` containing phase rules
- **AND** the current state is `build`
- **THEN** the `ContextAssembler` SHALL render the template with the current state
- **AND** the rendered text SHALL be injected into the model's hidden system instructions
- **AND** the text SHALL NOT appear in user-visible chat

#### Scenario: Coordinator mode context injected conditionally

- **WHEN** the Comet workflow YAML defines a context layer with `when: { build_mode: "subagent-driven-development" }`
- **AND** `build_mode` is `subagent-driven-development`
- **THEN** the coordinator reminder context SHALL be injected
- **AND** when `build_mode` is not `subagent-driven-development`, the context SHALL be skipped

