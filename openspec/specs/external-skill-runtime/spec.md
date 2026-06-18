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
The system SHALL inject a compact available-skills listing into model context so the model can choose valid external skills without loading every full skill body upfront.

#### Scenario: Available listing contains discovered skill
- **WHEN** external skills are discovered before a model turn
- **THEN** the prompt context includes each canonical skill name and description
- **AND** it instructs the model to load full instructions through the Skill runtime action when needed

#### Scenario: Full bodies are loaded on demand
- **WHEN** an external skill is present in the available listing but has not been invoked
- **THEN** the prompt context does not include that skill's full markdown body

### Requirement: Slash command skill routing
The system SHALL route slash commands that do not match built-in commands to matching external skills.

#### Scenario: User invokes comet slash command
- **WHEN** the user enters `/comet 如果把 comet 的能力融合进这个项目`
- **THEN** the runtime resolves the `comet` external skill
- **AND** the model turn receives the full `comet` skill instructions
- **AND** the raw argument text is preserved as `ARGUMENTS` context

#### Scenario: Unknown slash command reports suggestions
- **WHEN** the user enters a slash command with no built-in or external skill match
- **THEN** the system reports that the command is unknown
- **AND** it suggests similar external skill names when available

### Requirement: Nested Skill runtime action
The system SHALL provide a Claude Code-compatible `skill`/`Skill` runtime action that allows the model to load another external skill by canonical name with optional arguments.

#### Scenario: Comet loads child skill
- **WHEN** the loaded `comet` skill instructs the model to invoke `comet-open`
- **THEN** the model can call the `skill`/`Skill` runtime action with `skill = "comet-open"`
- **AND** the runtime returns the full `comet-open` instructions as a tool-style result

#### Scenario: Namespaced skill loads successfully
- **WHEN** the model calls the `skill`/`Skill` runtime action with `skill = "superpowers:brainstorming"`
- **THEN** the runtime resolves the namespaced skill exactly
- **AND** returns its full instructions without treating `:` as a path separator

#### Scenario: Nested skill depth limit is enforced
- **WHEN** nested skill loading would exceed depth 8
- **THEN** the runtime denies the load with an actionable maximum-depth error
- **AND** the runtime does not inject the requested skill body

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

