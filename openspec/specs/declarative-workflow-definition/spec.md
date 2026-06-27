# declarative-workflow-definition Specification

## Purpose
TBD - created by archiving change generic-agent-runtime. Update Purpose after archive.
## Requirements
### Requirement: Workflow YAML defines entry commands

The system SHALL accept a `workflow.yaml` with an `entry_commands` field listing slash commands that trigger this workflow. The `CommandRouter` SHALL match user input to these commands.

#### Scenario: Entry command triggers workflow

- **WHEN** a workflow YAML defines `entry_commands: [comet, comet-open]`
- **AND** the user types `/comet fix the bug`
- **THEN** the `CommandRouter` SHALL route the invocation to this workflow

#### Scenario: Unknown command does not match any workflow

- **WHEN** the user types `/unknown-command`
- **AND** no workflow defines `unknown-command` in its `entry_commands`
- **THEN** the `CommandRouter` SHALL return an unknown command error with suggestions

### Requirement: Workflow YAML defines states with on_enter actions

The system SHALL accept a `states` map where each key is a state identifier (string) and each value defines `skill` (skill to inject), `on_enter` (actions to run on entering the state), and optional `on_exit` actions.

#### Scenario: State definition binds a skill

- **WHEN** a state defines `skill: comet-open`
- **THEN** upon entering that state, the `SkillManager` SHALL load and inject the `comet-open` skill into the model context

#### Scenario: State definition runs on_enter context injection

- **WHEN** a state defines `on_enter: [context: [phase-instruction]]`
- **THEN** the `ContextAssembler` SHALL inject the context layer with id `phase-instruction` into the model's hidden context

### Requirement: Workflow YAML defines transitions with typed guards

The system SHALL accept a `transitions` list where each transition has `from`, `to`, and a `guards` list. Each guard has a `type` field selecting a registered guard implementation.

#### Scenario: Transition with file_exists guard

- **WHEN** a transition guard has `type: file_exists` with `files: [proposal.md, design.md]` and `relative_to: ${CHANGE_DIR}`
- **THEN** the guard SHALL check that all listed files exist at the resolved paths
- **AND** return `true` only if all files exist

#### Scenario: Transition with user_confirm guard

- **WHEN** a transition guard has `type: user_confirm` with `message` and `options`
- **THEN** the guard SHALL call `InteractionService::confirm()` with the configured message
- **AND** return `true` only if the user selects a non-abort option

#### Scenario: Transition with script guard

- **WHEN** a transition guard has `type: script` with `run: "${COMET_GUARD} ${CHANGE} open --apply"`
- **THEN** the guard SHALL execute the script via `ScriptRunner`
- **AND** return `true` only if the script exits with code 0

#### Scenario: Transition with file_check guard

- **WHEN** a transition guard has `type: file_check` with `file: "tasks.md"` and `check: all_tasks_checked`
- **THEN** the guard SHALL read the file and verify all task checkboxes are marked complete

### Requirement: Workflow YAML defines routing rules

The system SHALL accept a `routing` list where each rule has a `when` condition (matching discovery output) and a `then` action list. The first matching rule SHALL be executed.

#### Scenario: Routing matches no changes

- **WHEN** a routing rule has `when: { discovery: { changes: { count: 0 } } }`
- **AND** discovery output matches this condition
- **THEN** the `then` actions SHALL be executed (e.g., show an interaction question, transition to a state)

#### Scenario: Routing matches with interaction

- **WHEN** a routing rule's `then` includes `interaction` with `question` and `options`
- **THEN** the `InteractionService` SHALL present the question to the user
- **AND** the selected option's `action` SHALL determine the next step

### Requirement: Workflow YAML defines tool guard rules

The system SHALL accept a `tool_guards` list where each entry defines a guard rule for the `GuardPipeline`. Supported types include `deny_in_states` (block tools in specific states).

#### Scenario: deny_in_states blocks mutating tools

- **WHEN** `tool_guards` defines `{ type: deny_in_states, states: [open, design], tools: [file_write, file_edit] }`
- **AND** the current state is `open`
- **AND** the agent attempts `file_write`
- **THEN** the `RuleBasedGuard` SHALL return `Deny` with the configured message

#### Scenario: deny_in_states with unless_command_matches

- **WHEN** `tool_guards` defines `{ type: deny_in_states, states: [open], tools: [exec_command], unless_command_matches: [git status, ls] }`
- **AND** the agent runs `git status`
- **THEN** the guard SHALL return `Allow` because the command matches the exception pattern

### Requirement: Workflow YAML defines context layers

The system SHALL accept a `context` list where each layer has an `id`, `priority` (ordering), `visibility` (`internal` or `visible`), and a `source` (template string, file path, or conditional).

#### Scenario: Internal template context hidden from user

- **WHEN** a context layer defines `{ visibility: internal, template: "当前处于 {{ state }} 阶段" }`
- **THEN** the rendered content SHALL be injected into model context
- **AND** SHALL NOT appear in user-visible chat

### Requirement: Workflow YAML defines discovery

The system SHALL accept a `discovery` section with a `script` field that runs to discover the current workflow state.

#### Scenario: Discovery script output used for routing

- **WHEN** `discovery.script: "openspec list --json"` is defined
- **THEN** the `DiscoveryEngine` SHALL run this script at the start of workflow execution
- **AND** the parsed JSON output SHALL be available for routing rule `when` conditions

### Requirement: Workflow YAML validates at load time

The system SHALL validate the `workflow.yaml` structure at load time and reject invalid definitions with specific error messages.

#### Scenario: Undeclared state in transition fails validation

- **WHEN** a transition references `from: foo` but `foo` is not in the `states` map
- **THEN** validation SHALL fail with `"transition from 'foo' references undeclared state"`

#### Scenario: Unknown guard type fails validation

- **WHEN** a guard has `type: nonexistent_guard`
- **AND** no guard implementation is registered under that name
- **THEN** validation SHALL fail with `"unknown guard type 'nonexistent_guard'"`

