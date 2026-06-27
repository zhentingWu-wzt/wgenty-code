# external-skill-runtime Specification

## Purpose
TBD - created by archiving change external-skill-runtime. Update Purpose after archive.
## Requirements
### Requirement: External skill discovery
The system SHALL discover Claude Code-style external skills from configured wgenty-code project, wgenty-code user, plugin/cache, and configured extra skill roots without requiring skills to be compiled into wgenty-code.

#### Scenario: Discover project-local skill
- **WHEN** a repository contains `.wgenty-code/skills/comet/SKILL.md`
- **THEN** the external skill registry includes a `comet` skill sourced from that project-local path

#### Scenario: Discover user skill
- **WHEN** the user home contains `.wgenty-code/skills/superpowers/brainstorming/SKILL.md`
- **THEN** the external skill registry includes a `superpowers:brainstorming` skill sourced from the user path

#### Scenario: Discover plugin cache skill
- **WHEN** an enabled plugin cache contains a `skills/<name>/SKILL.md` entry
- **THEN** the external skill registry includes that skill with plugin/cache source metadata

#### Scenario: Discover configured extra root skill
- **WHEN** settings include an extra skill root containing `skills/custom-flow/SKILL.md`
- **THEN** the external skill registry includes `custom-flow` with configured-root source metadata

#### Scenario: Portable namespace directory maps to canonical name
- **WHEN** a skill exists at `.wgenty-code/skills/superpowers/brainstorming/SKILL.md` without a frontmatter name
- **THEN** the runtime uses `superpowers:brainstorming` as the canonical skill name

### Requirement: Skill metadata parsing
The system SHALL parse external skill frontmatter and body into a runtime definition while preserving the markdown instructions verbatim for model consumption.

#### Scenario: Frontmatter defines name and description
- **WHEN** a `SKILL.md` file contains YAML frontmatter with `name` and `description`
- **THEN** the runtime definition uses those fields for canonical name and available-skill listing
- **AND** the full markdown body remains available for on-demand loading

#### Scenario: Missing name falls back to directory
- **WHEN** a `SKILL.md` file omits `name` in frontmatter
- **THEN** the runtime uses the skill directory name as the canonical skill name

### Requirement: Deterministic skill conflict resolution
The system SHALL resolve duplicate external skill names using a deterministic source priority and retain shadowed definitions for diagnostics.

#### Scenario: Project skill overrides user skill
- **WHEN** both project and user roots define a skill named `comet`
- **THEN** resolving `comet` returns the project-local definition
- **AND** diagnostics can report that the user-level definition was shadowed

#### Scenario: Conflict source is explainable
- **WHEN** debug or verbose skill listing is requested
- **THEN** the system reports the selected source path and any shadowed source paths for duplicate skill names

### Requirement: Available skills prompt listing

The system SHALL replace the hardcoded available-skills listing with `SkillManager::list_available()` output, which SHALL include skills discovered from all configured roots plus workflows from `workflow.yaml` `entry_commands`.

#### Scenario: Available listing contains both skills and workflow entry commands

- **WHEN** `SkillManager::list_available()` is called
- **THEN** the output SHALL include skills discovered from SKILL.md files
- **AND** the output SHALL include workflow entry commands from `workflow.yaml` files
- **AND** each entry SHALL include name and description

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

### Requirement: Nested Skill runtime action

The system SHALL replace the hardcoded `skill`/`Skill` tool with a generic `SkillTool` that delegates to `SkillManager::load()` and `SkillManager::inject()`. The tool SHALL be workflow-agnostic.

#### Scenario: Model loads a skill by name

- **WHEN** the model calls the `Skill` tool with `name: comet-open`
- **THEN** `SkillManager::load("comet-open")` SHALL return the full SKILL.md body
- **AND** the body SHALL be injected as a `tool_result` into the conversation

### Requirement: Loaded skill context tracking
The system SHALL track which external skills are loaded during a session or turn so subsequent policy hooks and diagnostics can inspect the active instruction context.

#### Scenario: Loaded skills are recorded
- **WHEN** a slash command loads `comet` and a nested Skill action loads `comet-open`
- **THEN** runtime state records both loaded skill names and their source paths

#### Scenario: Duplicate load is idempotent
- **WHEN** the same skill is loaded more than once in a turn
- **THEN** the runtime avoids duplicating identical full instructions in model context
- **AND** records the repeated load as an invocation event

### Requirement: Policy hook extension points
The system SHALL expose policy hook interfaces for skill lifecycle and guarded execution events without requiring Comet-specific enforcement in the first version.

#### Scenario: Hook observes skill load
- **WHEN** an external skill is resolved and loaded
- **THEN** registered policy hooks receive the skill name, arguments, source metadata, and current loaded-skill context

#### Scenario: Default policy permits execution
- **WHEN** no custom policy denies a skill load, nested skill call, or observed guarded event
- **THEN** the runtime allows the operation to proceed
- **AND** emits structured diagnostics for later inspection

#### Scenario: Future policy can deny operation
- **WHEN** a custom policy hook returns a denial for a nested skill call or guarded event
- **THEN** the runtime stops that operation and returns an actionable denial message to the model/user

