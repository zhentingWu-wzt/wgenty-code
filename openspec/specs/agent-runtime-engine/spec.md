# agent-runtime-engine Specification

## Purpose
TBD - created by archiving change generic-agent-runtime. Update Purpose after archive.
## Requirements
### Requirement: Workflow engine interprets declarative YAML definitions

The system SHALL provide a `WorkflowEngine` that loads, validates, and executes a workflow defined by a `workflow.yaml` file. The engine SHALL NOT contain any domain-specific knowledge about the workflow it interprets.

#### Scenario: Engine loads a valid workflow YAML

- **WHEN** a `workflow.yaml` file exists at `<skill_dir>/workflow.yaml` with valid structure
- **THEN** `WorkflowEngine::load()` SHALL return a `WorkflowDefinition` with all states, transitions, guards, context layers, routing rules, and tool guard rules parsed
- **AND** the engine SHALL validate that all transition `from`/`to` values reference declared states

#### Scenario: Engine rejects invalid YAML at load time

- **WHEN** a `workflow.yaml` contains a transition referencing an undeclared state
- **THEN** `WorkflowEngine::load()` SHALL return a validation error identifying the invalid reference

#### Scenario: Engine caches parsed definition for session

- **WHEN** a workflow is loaded successfully
- **THEN** the parsed `WorkflowDefinition` SHALL be cached in memory
- **AND** subsequent operations SHALL use the cached definition until the YAML file's mtime changes

### Requirement: String-keyed state machine with transition guards

The system SHALL provide a `StateMachine` that manages states as string identifiers, tracks the current state, evaluates transition guards, and executes state transitions. States and their semantics are defined entirely by the workflow YAML.

#### Scenario: State machine starts at initial state

- **WHEN** a workflow is loaded and its state source returns a current state value
- **THEN** `StateMachine::current()` SHALL return that state identifier as a string

#### Scenario: Valid transition changes state

- **WHEN** a transition is requested from state A to state B
- **AND** all transition guards pass (return `true`)
- **THEN** `StateMachine::current()` SHALL return state B
- **AND** all `on_exit` actions for state A SHALL execute
- **AND** all `on_enter` actions for state B SHALL execute

#### Scenario: Guard blocks transition

- **WHEN** a transition is requested but one or more guards return `false`
- **THEN** the transition SHALL be blocked
- **AND** the current state SHALL remain unchanged
- **AND** the guard failure reason SHALL be returned to the caller

#### Scenario: Transition guard of type `user_confirm` pauses for interaction

- **WHEN** a transition guard has `type: user_confirm`
- **THEN** the guard SHALL call `InteractionService::confirm()` with the configured message and options
- **AND** the transition SHALL only proceed if the user selects the confirm option

### Requirement: Context assembler with priority layering and visibility control

The system SHALL provide a `ContextAssembler` that merges `ContextLayer` definitions (from workflow YAML) into ordered, separated output streams based on `priority` and `visibility`.

#### Scenario: Internal layer not visible to user

- **WHEN** a `ContextLayer` has `visibility: internal`
- **THEN** the assembled output SHALL include that layer's content in the model's system instructions
- **AND** the content SHALL NOT appear in user-visible chat messages

#### Scenario: Visible layer appears to user

- **WHEN** a `ContextLayer` has `visibility: visible`
- **THEN** the assembled output SHALL include that layer's content in the user-visible chat

#### Scenario: Layers ordered by priority

- **WHEN** multiple `ContextLayer` definitions exist with different `priority` values
- **THEN** the assembled context SHALL order layers from lowest priority to highest priority
- **AND** higher priority content SHALL appear later in the context window (closer to the current turn)

#### Scenario: Template layer renders with state variables

- **WHEN** a `ContextLayer` has `source: template` with `{{ state }}` placeholder
- **THEN** the assembler SHALL render the template with the current workflow state
- **AND** the rendered content SHALL replace the placeholder with the actual state value

### Requirement: Guard pipeline with deny-first semantics

The system SHALL provide a `GuardPipeline` that executes an ordered list of tool guards. The first guard returning `Deny` SHALL short-circuit the pipeline. Guards SHALL NOT modify tool arguments.

#### Scenario: First guard deny short-circuits pipeline

- **WHEN** `GuardPipeline` contains [GuardA, GuardB] and GuardA returns `Deny`
- **THEN** GuardB SHALL NOT be evaluated
- **AND** the pipeline SHALL return GuardA's `GuardDecision`

#### Scenario: All guards allow, pipeline passes

- **WHEN** all guards in the pipeline return `Allow`
- **THEN** the pipeline SHALL return `GuardDecision::allow()`

#### Scenario: deny_in_states guard blocks matching tool

- **WHEN** a `RuleBasedGuard` is configured with `deny_in_states: { states: [open, design], tools: [file_write] }`
- **AND** the current state is `open`
- **AND** the tool being checked is `file_write`
- **THEN** the guard SHALL return `Deny` with the configured `message`

#### Scenario: deny_in_states guard allows unmatched state

- **WHEN** a `RuleBasedGuard` is configured with `deny_in_states: { states: [open, design] }`
- **AND** the current state is `build`
- **THEN** the guard SHALL return `Allow`

### Requirement: Skill manager with progressive disclosure

The system SHALL provide a `SkillManager` that loads skills in three tiers: (1) frontmatter discovery at startup, (2) full body on demand, (3) reference files on explicit request.

#### Scenario: Tier 1 discovers skills without loading bodies

- **WHEN** `SkillManager::discover()` scans configured skill roots
- **THEN** each skill's name and description (from SKILL.md frontmatter) SHALL be available
- **AND** no skill body content SHALL be loaded into memory

#### Scenario: Tier 2 loads body on demand

- **WHEN** a skill is requested by name via `SkillManager::load(name)`
- **THEN** the full SKILL.md body content SHALL be loaded and cached
- **AND** the body SHALL be available for context injection

#### Scenario: Tier 3 loads reference files on explicit request

- **WHEN** a loaded skill's body references a file in its `reference/` directory
- **AND** `SkillManager::load_reference(skill, ref_name)` is called
- **THEN** the reference file content SHALL be loaded and returned

### Requirement: Interaction service as platform-agnostic trait

The system SHALL define `InteractionService` as a trait with `ask()` and `confirm()` methods. Platform implementations (TUI, CLI, daemon, headless) SHALL provide concrete behavior.

#### Scenario: TUI implementation pauses agent loop

- **WHEN** `InteractionService::ask()` is called in TUI mode
- **THEN** a structured question SHALL be displayed in the TUI
- **AND** the agent loop SHALL pause until the user responds

#### Scenario: CLI implementation presents terminal choices

- **WHEN** `InteractionService::confirm()` is called in CLI mode
- **THEN** terminal choices SHALL be presented
- **AND** stdin input SHALL be awaited for the user's selection

#### Scenario: Headless mode fails explicitly

- **WHEN** `InteractionService::ask()` is called in headless/non-interactive mode without a pre-configured policy
- **THEN** the call SHALL return an error indicating interaction is not available
- **AND** the workflow SHALL NOT proceed with a default choice

### Requirement: Event bus for runtime observations

The system SHALL provide an `EventBus` that emits structured events for tool calls, state transitions, guard decisions, and user interactions. Consumers (hooks, logging, UI) SHALL subscribe to event types.

#### Scenario: Tool call event emitted on execution

- **WHEN** a tool is executed through the `GuardPipeline`
- **THEN** a `RuntimeEvent::ToolCall` event SHALL be emitted with tool name, arguments (sanitized), and guard decision

#### Scenario: State transition event emitted

- **WHEN** a state transition completes successfully
- **THEN** a `RuntimeEvent::StateTransition` event SHALL be emitted with `from`, `to`, and `trigger`

### Requirement: State source abstraction for external state

The system SHALL define `StateSource` as a trait that decouples the runtime from any specific state storage mechanism. Implementations include script-based, file-based, and composite sources.

#### Scenario: Script state source invokes shell command

- **WHEN** a `ScriptStateSource` is configured with `read_script: "openspec list --json"`
- **AND** `StateSource::read()` is called
- **THEN** the script SHALL be executed
- **AND** stdout SHALL be parsed as the return value
- **AND** non-zero exit code SHALL return an error

#### Scenario: File state source reads YAML

- **WHEN** a `FileStateSource` is configured with `path: ".comet.yaml"`
- **AND** the file exists and contains valid YAML
- **THEN** `StateSource::read()` SHALL return the parsed YAML as a `Value`

### Requirement: Script runner with output protocol

The system SHALL provide a `ScriptRunner` that executes shell scripts, captures stdout/stderr and exit code, and optionally parses structured JSON output from stdout.

#### Scenario: Script succeeds with exit code 0

- **WHEN** `ScriptRunner::run(script_path)` executes a script that exits with code 0
- **THEN** the result SHALL indicate success
- **AND** stdout SHALL be captured

#### Scenario: Script fails with non-zero exit code

- **WHEN** `ScriptRunner::run(script_path)` executes a script that exits with code non-zero
- **THEN** the result SHALL indicate failure
- **AND** stderr SHALL be captured as the error message

#### Scenario: Script outputs JSON on stdout

- **WHEN** a script outputs a JSON line on stdout prefixed with `JSON:`
- **THEN** `ScriptRunner` SHALL parse that line as structured data
- **AND** the parsed JSON SHALL be available in the result's `data` field

