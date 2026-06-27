# external-skill-runtime Delta Spec

## MODIFIED Requirements

### Requirement: Slash command skill routing

The system SHALL replace `comet_slash_agent_prompt()` and `route_slash_command()` with the generic `CommandRouter` that matches slash commands to workflow `entry_commands` in `workflow.yaml`. The hardcoded `"This is a native Comet dispatch wrapper..."` prompt SHALL be removed.

#### Scenario: User invokes comet slash command via CommandRouter

- **WHEN** the user enters `/comet fix the bug`
- **AND** a workflow YAML defines `entry_commands: [comet]`
- **THEN** `CommandRouter` SHALL route the invocation to the `WorkflowEngine` for that workflow
- **AND** the `WorkflowEngine` SHALL run discovery, evaluate routing rules, and inject context
- **AND** the user SHALL NOT see `"This is a native Comet dispatch wrapper..."` in their chat

#### Scenario: CommandRouter handles unknown slash commands

- **WHEN** the user enters a slash command with no matching built-in or workflow `entry_commands`
- **THEN** `CommandRouter` SHALL return an unknown command result
- **AND** the TUI SHALL display suggestions based on Levenshtein distance to known commands

#### Scenario: Internal workflow context hidden from user

- **WHEN** the `WorkflowEngine` injects context layers with `visibility: internal`
- **THEN** those context layers SHALL be sent to the model as hidden system instructions
- **AND** the user SHALL only see friendly status messages (e.g., "Starting Comet workflow...")

### Requirement: Available skills prompt listing

The system SHALL replace the hardcoded available-skills listing with `SkillManager::list_available()` output, which SHALL include skills discovered from all configured roots plus workflows from `workflow.yaml` `entry_commands`.

#### Scenario: Available listing contains both skills and workflow entry commands

- **WHEN** `SkillManager::list_available()` is called
- **THEN** the output SHALL include skills discovered from SKILL.md files
- **AND** the output SHALL include workflow entry commands from `workflow.yaml` files
- **AND** each entry SHALL include name and description

### Requirement: Nested Skill runtime action

The system SHALL replace the hardcoded `skill`/`Skill` tool with a generic `SkillTool` that delegates to `SkillManager::load()` and `SkillManager::inject()`. The tool SHALL be workflow-agnostic.

#### Scenario: Model loads a skill by name

- **WHEN** the model calls the `Skill` tool with `name: comet-open`
- **THEN** `SkillManager::load("comet-open")` SHALL return the full SKILL.md body
- **AND** the body SHALL be injected as a `tool_result` into the conversation
