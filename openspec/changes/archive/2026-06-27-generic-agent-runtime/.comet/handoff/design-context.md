# Comet Design Handoff

- Change: generic-agent-runtime
- Phase: design
- Mode: compact
- Context hash: 04e8148b483c8f88317eea5a114411f76a8f56f7072f715e224e842f59fc68b6

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/generic-agent-runtime/proposal.md

- Source: openspec/changes/generic-agent-runtime/proposal.md
- Lines: 1-34
- SHA256: c512338863d51c7212666628b355dc7367383edf9de7b9803166fd5b5129d538

```md
## Why

The current `/comet` workflow is implemented by hard-coding Comet-specific logic (phases, decision points, OpenSpec state discovery, guard rules) into Rust source code under `src/comet/`. Adding a new workflow (e.g., code review) requires writing Rust code. Changing Comet behavior requires recompilation. This change replaces `src/comet/` with a generic Agent Runtime engine that interprets declarative `workflow.yaml` files, making workflows configurable without Rust changes and keeping domain knowledge entirely outside compiled code.

## What Changes

- **BREAKING**: Remove `src/comet/` entirely — `CometState`, `CometGuard`, `CometPhase`, `active_changes()`, and `comet_slash_agent_prompt()` are deleted
- Add `src/runtime/` — a generic Agent Runtime engine providing: `WorkflowEngine` (YAML-driven state machine), `ContextAssembler` (priority-layered prompt assembly with visibility control), `GuardPipeline` (composable tool execution guards), `InteractionService` trait (platform-agnostic user interaction), `SkillManager` (progressive disclosure), `ScriptRunner`, `EventBus`, and `StateSource` trait
- Add `.wgenty-code/skills/comet/workflow.yaml` — complete declarative definition of the Comet workflow (states, transitions, guards, decisions, context layers, skill bindings)
- Migrate `src/hooks/` and `src/guardian/` into `src/runtime/` as generic runtime capabilities
- Modify `src/tools/executor.rs` — replace hardcoded `CometGuard::check()` call with `GuardPipeline` that reads rules from `workflow.yaml`
- Modify `src/tui/app/input.rs` — route slash commands through `CommandRouter` instead of `comet_slash_agent_prompt()`
- Modify `src/prompts/mod.rs` — replace hardcoded comet phase injection with `ContextAssembler` driven by `workflow.yaml` context layers

## Capabilities

### New Capabilities
- `agent-runtime-engine`: Generic Agent Runtime that interprets declarative workflow definitions. Provides state machine (string-keyed states + transitions + guards), context assembly (layered prompts with visibility), guard pipeline (deny-first tool interception), interaction service (platform-agnostic ask/confirm), skill management (progressive disclosure), script running, event bus, and state source abstraction. Knows NOTHING about Comet, OpenSpec, phases, brainstorming, or any specific workflow.
- `declarative-workflow-definition`: YAML schema for defining workflows. Declares entry commands, states, transitions (with guards of types: file_exists, user_confirm, script, file_check, deny_in_states), tool guard rules, context layers (with priority, visibility, template/file source), discovery scripts, routing rules, presets, and skill bindings. Comet becomes one such definition file.

### Modified Capabilities
- `comet-phase-guard`: Phase guard rules move from Rust `CometGuard::check()` into `workflow.yaml` `tool_guards.deny_in_states` rules interpreted by the generic `RuleBasedGuard`
- `comet-skill-path-compat`: Skill loading path resolution moves into generic `SkillManager` with progressive disclosure; comet-specific paths become `workflow.yaml` configuration
- `external-skill-runtime`: External skill slash command dispatch moves from `comet_slash_agent_prompt()` + hardcoded text into `CommandRouter` + `workflow.yaml` routing rules + `ContextAssembler` hidden context injection
- `hook-event-alignment`: Hooks move into `src/runtime/` as generic runtime events, maintaining existing hook event types but accessible to any workflow

## Impact

- **Rust source**: ~2000 lines removed from `src/comet/`, ~1500 lines added to `src/runtime/`, modifications to `src/tools/executor.rs`, `src/tui/app/input.rs`, `src/prompts/mod.rs`
- **Configuration**: New `.wgenty-code/skills/comet/workflow.yaml` (~200 lines of YAML)
- **Skills**: All existing Comet skill files under `~/.claude/skills/comet*/` and `.wgenty-code/skills/comet/` continue to work unchanged
- **Scripts**: Existing `comet-guard.sh`, `comet-state.sh`, `comet-handoff.sh`, `comet-archive.sh` continue to work as script-based guards and state management
- **No API changes**: Agent loop, tool system, MCP, API clients are unaffected
- **No user-facing regression**: `/comet` commands continue to work identically; internal prompts become hidden; decision points remain blocking
```

## openspec/changes/generic-agent-runtime/design.md

- Source: openspec/changes/generic-agent-runtime/design.md
- Lines: 1-100
- SHA256: 7da195edd61082ca8e9c1072630093ef781713bb30668bf9fcf4eb8146799007

[TRUNCATED]

```md
## Context

The `wgenty-code` project currently implements the Comet workflow (`/comet`) by hard-coding Comet-specific concepts — `CometPhase`, `CometState`, `CometGuard`, `active_changes()`, `comet_slash_agent_prompt()` — directly into Rust source under `src/comet/`. The system prompt assembly in `src/prompts/mod.rs` injects a hardcoded "Layer 1b: Comet Phase Awareness" based on `CometState::read()`. The tool executor in `src/tools/executor.rs` calls `CometGuard::check()` as a special case before the hook pipeline.

This architecture means:
- Adding a new workflow (e.g., code review, deployment) requires writing Rust code
- Changing Comet phase rules requires recompilation
- The generic runtime primitives (hooks, guardian, skill registry, prompt assembly) are already implemented but Comet-specific logic is tangled with them

Both Claude Code and Codex demonstrate the correct pattern: **a deterministic harness interprets declarative configuration; the harness knows nothing about specific workflows.** Claude Code's ~98.4% of code is deterministic infrastructure; only ~1.6% is AI decision logic.

## Goals / Non-Goals

**Goals:**
1. Delete `src/comet/` — zero Comet/OpenSpec knowledge in Rust code
2. Build `src/runtime/` — a generic Agent Runtime engine that interprets declarative `workflow.yaml` files
3. Define Comet entirely in `.wgenty-code/skills/comet/workflow.yaml` + existing scripts + SKILL.md
4. Migrate `src/hooks/` and `src/guardian/` into `src/runtime/` as generic capabilities
5. Replace hardcoded comet references in `src/tools/executor.rs`, `src/tui/app/input.rs`, `src/prompts/mod.rs` with generic Runtime API calls
6. Zero user-visible regression: `/comet` commands work identically; internal prompts become hidden

**Non-Goals:**
- Rewrite the agent loop (`src/agent/`)
- Rewrite the tool system (`src/tools/`)
- Support multiple concurrent workflows (single workflow per session)
- Change Comet's five-phase semantics
- Change the `openspec` CLI tool
- Rewrite TUI/CLI/Daemon (only add `InteractionService` impl for TUI)

## Decisions

### Decision 1: String-keyed state machine, not enum-based

**Choice**: StateMachine uses `String` keys for states, not Rust enums.

**Rationale**: If states are Rust enums, every new workflow requires a new enum definition (compilation). With string keys, the workflow YAML defines valid states, and the runtime validates transitions without knowing what the strings mean.

**Alternatives considered**:
- Generic `StateId` trait + per-workflow enum: Still requires Rust code per workflow. Rejected.
- Fully dynamic `serde_json::Value`: Loses type safety for transition logic. Rejected.

### Decision 2: Workflow YAML as the single source of truth

**Choice**: A single `workflow.yaml` per skill directory defines all states, transitions, guards, context layers, routing rules, and tool guard rules.

**Rationale**: Claude Code uses multiple mechanisms (CLAUDE.md, skills, hooks, slash commands) for different injection points. For a workflow that orchestrates all three injection points (`assemble`, `model`, `execute`), a unified definition avoids split-brain between e.g., hooks config and skill instructions.

### Decision 3: Guards as pluggable types in YAML, not trait impls in Rust

**Choice**: Transition guards and tool guards are declared in YAML by `type` field. The runtime ships with built-in guard types: `file_exists`, `user_confirm`, `script`, `file_check`, `deny_in_states`. Each guard type is a Rust struct implementing `TransitionGuard` or `ToolGuard` trait, but which guards apply to which workflow is purely YAML-configured.

**Rationale**: The set of guard primitives is small and stable. New workflows compose them differently rather than inventing new guard types. If a workflow truly needs a novel guard type, adding one Rust struct is acceptable — but changing *which* guards apply to *which* states must be YAML-only.

### Decision 4: Scripts as the bridge between Runtime and external tools

**Choice**: State discovery (`state.read.script`), state transitions (`transition.on_complete.script`), and guard checks (`guards[].type: script`) invoke shell scripts. The runtime captures exit code and stdout, parsing structured output (JSON) when needed.

**Rationale**: Existing Comet scripts (`comet-guard.sh`, `comet-state.sh`, `comet-handoff.sh`, `comet-archive.sh`) already encapsulate Comet logic. Making them pluggable via YAML preserves the script investment while removing Rust coupling. The runtime only needs to know: "run this script, check exit code, optionally parse JSON on stdout."

### Decision 5: Context visibility as a first-class concept

**Choice**: Every `ContextLayer` has a `visibility` field: `internal` (agent sees, user does not) or `visible` (both see). The `ContextAssembler` separates context into two streams — one sent to the model as hidden system instructions, one appended to user-visible chat.

**Rationale**: The current `comet_slash_agent_prompt()` is the most visible UX issue — users see `"This is a native Comet dispatch wrapper..."` in their chat. Making visibility a first-class concept in the runtime ensures no workflow can accidentally leak internal instructions.

### Decision 6: GuardPipeline as deny-first, composable

**Choice**: `GuardPipeline` holds an ordered list of `Box<dyn ToolGuard>`. The first guard that returns `Deny` short-circuits the pipeline. Guards cannot modify tool arguments (only allow/deny). A separate `PreToolUse` hook handles modification.

**Rationale**: Claude Code's deny-first model (`deny > ask > allow`) is proven in production. Separating deny (guard) from modify (hook) keeps each mechanism simple and auditable.

### Decision 7: InteractionService as a trait behind the Runtime

**Choice**: `InteractionService` is a trait with `ask()` and `confirm()` methods. The TUI provides an implementation that sends `AppEvent::QuestionAsked`, pauses the agent loop, and resumes on user response. CLI provides a terminal-choice implementation. Daemon provides SSE/WebSocket.

**Rationale**: Codex's App Server demonstrates that decoupling the core from the UI via an async protocol enables all platforms. Making interaction a trait rather than a model tool ensures blocking points are runtime-enforced, not model-dependent.

## Risks / Trade-offs

- **[Risk] YAML complexity**: workflow.yaml could become as complex as the Rust code it replaces. **Mitigation**: Keep the YAML schema minimal — only states, transitions, guards, context layers. Complex logic stays in scripts (shell) or skill instructions (markdown).
```

Full source: openspec/changes/generic-agent-runtime/design.md

## openspec/changes/generic-agent-runtime/tasks.md

- Source: openspec/changes/generic-agent-runtime/tasks.md
- Lines: 1-86
- SHA256: 4c7c92e24427508fa31361bd7f53ee48776d09436b979cefccb36409d214dac4

[TRUNCATED]

```md
## 1. Runtime Foundation — Generic Primitives

- [ ] 1.1 Create `src/runtime/mod.rs` with module structure, re-export `WorkflowEngine`, `ContextAssembler`, `GuardPipeline`, `StateMachine`, `SkillManager`, `InteractionService` trait, `ScriptRunner`, `EventBus`, `StateSource` trait
- [ ] 1.2 Implement `src/runtime/state_source.rs` — `StateSource` trait (`read() -> Result<Value>`, `write(Value) -> Result<()>`), `ScriptStateSource` (runs shell script, parses stdout), `FileStateSource` (reads YAML/JSON file), `CompositeStateSource` (combines multiple sources)
- [ ] 1.3 Implement `src/runtime/script.rs` — `ScriptRunner` struct (executes shell scripts, captures exit code + stdout + stderr), `ScriptResult` with `success`, `stdout`, `stderr`, `exit_code`, optional JSON-parsed `data` field
- [ ] 1.4 Implement `src/runtime/event.rs` — `RuntimeEvent` enum (`ToolCall`, `StateTransition`, `GuardDecision`, `UserInteraction`, `SkillDiscovery`), `EventBus` with `emit()` and `subscribe()`, integrate with existing `HookEvent` types

## 2. State Machine

- [ ] 2.1 Implement `src/runtime/state_machine.rs` — `StateMachine` struct (string-keyed states map, transitions list, current state), `validate()` (check all transition from/to reference declared states, all guard types are registered), `transition(from, to)` method
- [ ] 2.2 Implement `TransitionGuard` trait — `async fn check(&self, ctx: &TransitionContext) -> Result<bool>`, `TransitionContext` with `state_source`, `interaction`, `script_runner`, `variables`
- [ ] 2.3 Implement built-in transition guard types: `FileExistsGuard` (checks files), `UserConfirmGuard` (calls `InteractionService`), `ScriptGuard` (runs script), `FileCheckGuard` (checks file content like `all_tasks_checked`)
- [ ] 2.4 Write unit tests: valid transition, blocked transition, guard ordering, undeclared state validation failure

## 3. Context Assembler

- [ ] 3.1 Implement `src/runtime/context.rs` — `ContextLayer` struct (`id`, `priority: u8`, `visibility: LayerVisibility` enum `{Internal, Visible}`, `source: LayerSource` enum `{Template(String), File(PathBuf), Conditional { when, template }}`), template rendering with `{{ var }}` substitution
- [ ] 3.2 Implement `ContextAssembler` — `assemble(layers, variables) -> AssembledContext` with two streams: `internal_instructions: Vec<String>` (agent-only, hidden) and `visible_content: Vec<String>` (user-visible)
- [ ] 3.3 Write unit tests: internal visibility hidden from visible stream, priority ordering, template variable substitution, conditional layer (included when condition matches, skipped when not)

## 4. Guard Pipeline

- [ ] 4.1 Implement `src/runtime/guard.rs` — `ToolGuard` trait (`async fn check(&self, tool: &str, args: &Value, state: &StateSnapshot) -> GuardResult`), `GuardPipeline` (ordered list, deny-first short-circuit)
- [ ] 4.2 Implement `RuleBasedGuard` — reads `tool_guards` rules from `WorkflowDefinition`, evaluates `deny_in_states` (block tools in listed states), `unless_command_matches` (exception patterns for bash commands with glob matching)
- [ ] 4.3 Write unit tests: deny_in_states blocks matching tool, allows unmatched state, unless_command_matches exception, first deny short-circuits pipeline, empty pipeline allows all

## 5. Skill Manager

- [ ] 5.1 Implement `src/runtime/skill.rs` — `SkillManager` struct with three-tier progressive disclosure: Tier 1 `discover()` (scan SKILL.md frontmatter only), Tier 2 `load(name)` (full body on demand with caching), Tier 3 `load_reference(skill, ref_name)` (reference files on explicit request)
- [ ] 5.2 Integrate `SkillManager` with existing `ExternalSkillRegistry` — `SkillManager` wraps and replaces the registry's public API, existing SKILL.md parsing logic reused internally
- [ ] 5.3 Write unit tests: Tier 1 discovers without loading bodies, Tier 2 caches after load, Tier 3 loads reference file, duplicate resolution (first wins, shadow recorded)

## 6. Interaction Service

- [ ] 6.1 Define `InteractionService` trait in `src/runtime/interaction.rs` — `async fn ask(&self, question: Question) -> Result<Answer>`, `async fn confirm(&self, prompt: ConfirmPrompt) -> Result<bool>`, `Question`/`Answer`/`ConfirmPrompt` model types
- [ ] 6.2 Implement TUI interaction service — `TuiInteractionService` sends `AppEvent::QuestionAsked`, pauses agent loop via `AgentPhase`, resumes on user response; add `AgentPhase::WaitingForInteraction` variant to `src/state/agent_phase.rs`
- [ ] 6.3 Implement CLI interaction service — `CliInteractionService` presents numbered choices on terminal, reads stdin for selection; implement headless policy (`HeadlessPolicy::Deny | PreConfiguredAnswers`) for non-interactive mode
- [ ] 6.4 Write integration test: TUI interaction pauses loop, user answer resumes, workflow continues with correct transition

## 7. Command Router & Workflow Engine

- [ ] 7.1 Implement `src/runtime/command.rs` — `CommandInvocation` struct (`name`, `args`, `raw_input`, `visibility`), `CommandRouter` with `route(input) -> RouteResult`, matching against workflow `entry_commands` and built-in commands
- [ ] 7.2 Implement `src/runtime/workflow/mod.rs` — `WorkflowEngine` struct, `WorkflowDefinition` (parsed from YAML: name, entry_commands, state, discovery, routing, states, transitions, tool_guards, context), `load(path) -> Result<WorkflowDefinition>`, `handle(CommandInvocation) -> AgentInput`
- [ ] 7.3 Implement `src/runtime/workflow/definition.rs` — YAML parsing with `serde_yaml`, validation (check state references, guard types, script paths), YAML schema: `WorkflowYaml { name, entry_commands, state, discovery, routing, states, transitions, tool_guards, context, presets }`
- [ ] 7.4 Implement `src/runtime/discovery.rs` — `DiscoveryEngine` runs `discovery.script` from workflow YAML, parses JSON output, validates against `discovery.schema` if present, returns structured result for routing
- [ ] 7.5 Write unit tests: valid workflow YAML loads, invalid YAML caught at load, routing matches discovery conditions, undeclared state in transition fails validation

## 8. Migrate Hooks & Guardian into Runtime

- [ ] 8.1 Move `src/hooks/mod.rs`, `src/hooks/cc_adapter.rs` into `src/runtime/hooks/` — keep all existing types, events, `HookManager` logic unchanged; update all `crate::hooks` imports across the codebase to `crate::runtime::hooks`
- [ ] 8.2 Move `src/guardian/mod.rs` into `src/runtime/guardian.rs` — keep `RiskLevel`, `GuardianDecision`, pattern lists unchanged; update imports
- [ ] 8.3 Verify all existing hook tests pass after migration (PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification, Stop, UserPromptSubmit, PermissionRequest, CC format compatibility)

## 9. Create Comet workflow.yaml

- [ ] 9.1 Create `.wgenty-code/skills/comet/workflow.yaml` with complete declarative definition: `entry_commands: [comet, comet-open, comet-design, comet-build, comet-verify, comet-archive, comet-hotfix, comet-tweak]`, `state.read.script` pointing to `comet-state current --json`, `state.write.script` pointing to `comet-state set`
- [ ] 9.2 Define all 5 states (open, design, build, verify, archive) with skill bindings and `on_enter: [context: [phase-instruction, comet-phase-guard]]`; define conditional context for coordinator mode
- [ ] 9.3 Define all transitions with guards: open→design (file_exists + user_confirm + script), design→build (user_confirm + script), build→verify (file_check all_tasks_checked + script), verify→archive (user_confirm + script, when verify_result=pass), verify→build (user_decision, when verify_result=fail)
- [ ] 9.4 Define `tool_guards` with `deny_in_states` rules: block `file_write`, `file_edit`, `apply_patch` in `[open, design, verify, archive]`; block `exec_command`/`execute_command` in same states with `unless_command_matches` exceptions for read-only commands
- [ ] 9.5 Define `context` layers: `phase-instruction` (internal, priority 35, template), `coordinator-reminder` (internal, priority 30, conditional on build_mode), `comet-phase-guard` (internal, priority 25, file source)
- [ ] 9.6 Define `routing` rules for 0 changes (clarification interaction → transition open), 1 change (continue-or-new interaction), multiple changes (select-or-new interaction); define `discovery.script: "openspec list --json"`
- [ ] 9.7 Define `templates.phase_rules` with the 5 phase instruction texts (Chinese); define `presets.hotfix` and `presets.tweak` with `skip_states: [design]`

## 10. Replace Hardcoded Comet References

- [ ] 10.1 Modify `src/tools/executor.rs` — replace `CometGuard::check(&state.phase, tool_name, &guard_args)` call in `execute_with_hooks()` with `GuardPipeline::evaluate()` from the active `WorkflowEngine`. Remove `comet_state: Option<CometState>` field from `ToolExecutor`, replace with `workflow_engine: Option<Arc<WorkflowEngine>>`
- [ ] 10.2 Modify `src/prompts/mod.rs` — remove Layer 1b (hardcoded `CometState::read()` + `phase_instruction()` + `coordinator_reminder()` injection). Add a generic `ContextAssembler::assemble()` call that takes layers from `WorkflowEngine.context_layers()`. Remove `use crate::comet` import
- [ ] 10.3 Modify `src/tui/app/input.rs` — replace `crate::knowledge::route_slash_command()` + `crate::knowledge::comet_slash_agent_prompt()` calls with `CommandRouter::route()`. Remove system message `"🔧 External skill '/...' detected..."`. Show only friendly status: `"Starting Comet workflow..."`, `"Checking active OpenSpec changes..."`
- [ ] 10.4 Modify `src/tui/app/mod.rs` — initialize `WorkflowEngine` at app startup alongside `ExternalSkillRegistry`; inject `SkillManager` and `WorkflowEngine` into the app state for use by TUI components
- [ ] 10.5 Update `src/tui/completion.rs` — slash command completion now reads from `WorkflowEngine::entry_commands()` (all registered workflow entry commands) instead of `ExternalSkillRegistry` direct listing for slash commands

## 11. Delete src/comet/ and Clean Up

- [ ] 11.1 Delete `src/comet/` directory (state.rs, guard.rs, workflow.rs, protocol.rs, mod.rs) — verify `grep -r "comet" src/runtime/` returns zero results (except test data strings)
- [ ] 11.2 Verify `grep -r "openspec" src/runtime/` returns zero results; verify `grep -r "CometPhase\|CometState\|CometGuard\|comet_slash_agent_prompt" src/` returns zero results
- [ ] 11.3 Remove `pub mod comet` from `src/lib.rs`; clean up any remaining `use crate::comet` imports across the codebase
- [ ] 11.4 Run full test suite — all existing tests pass, no regression in hook behavior, tool execution, prompt assembly, slash command dispatch

## 12. Integration Testing & Validation

```

Full source: openspec/changes/generic-agent-runtime/tasks.md

## openspec/changes/generic-agent-runtime/specs/agent-runtime-engine/spec.md

- Source: openspec/changes/generic-agent-runtime/specs/agent-runtime-engine/spec.md
- Lines: 1-211
- SHA256: 90c09143cb9da3cf4b1c861d02d17a857144542960f3c6eb83e01fc110ed2b3f

[TRUNCATED]

```md
# agent-runtime-engine Specification

## Purpose

Define the generic Agent Runtime engine — a domain-agnostic interpreter for declarative workflow definitions. The engine provides primitives (state machine, context assembly, guard pipeline, interaction service, skill management, script execution, event bus, state source) without any knowledge of Comet, OpenSpec, phases, brainstorming, or any specific workflow.

## ADDED Requirements

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

```

Full source: openspec/changes/generic-agent-runtime/specs/agent-runtime-engine/spec.md

## openspec/changes/generic-agent-runtime/specs/comet-phase-guard/spec.md

- Source: openspec/changes/generic-agent-runtime/specs/comet-phase-guard/spec.md
- Lines: 1-86
- SHA256: 8215f30c381d14163edde17e44b39187512a4f8bc918694962c3d45a3feaa719

[TRUNCATED]

```md
# comet-phase-guard Delta Spec

## MODIFIED Requirements

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

```

Full source: openspec/changes/generic-agent-runtime/specs/comet-phase-guard/spec.md

## openspec/changes/generic-agent-runtime/specs/comet-skill-path-compat/spec.md

- Source: openspec/changes/generic-agent-runtime/specs/comet-skill-path-compat/spec.md
- Lines: 1-39
- SHA256: 4f4add1981a05d11b3a34ec200e7139a5aa1283739fc000a3d0c41075a91f010

```md
# comet-skill-path-compat Delta Spec

## MODIFIED Requirements

### Requirement: Runtime external skill registry discovers all Comet-compatible roots

The system SHALL move skill root discovery into the generic `SkillManager` in `src/runtime/`. The `SkillRootResolver` SHALL be replaced by `SkillManager::discover()` which reads root paths from runtime configuration. The Comet workflow SHALL NOT require special Rust-level skill path handling.

#### Scenario: Comet skills installed in any configured root are discoverable

- **WHEN** Comet skills are installed in any configured skill root directory
- **THEN** `SkillManager::discover()` SHALL resolve them by their canonical names
- **AND** the TUI slash command completion SHALL suggest `/comet` based on workflow `entry_commands`, not skill path

#### Scenario: Skill root configuration is YAML-driven

- **WHEN** skill roots need to change (add/remove/reorder)
- **THEN** the change SHALL be made in runtime configuration (settings or workflow YAML)
- **AND** SHALL NOT require Rust code changes

### Requirement: Unified skill root resolution accessible to all consumers

The system SHALL replace `SkillRootResolver` with `SkillManager` as the single entry point for skill discovery. All consumers SHALL use `SkillManager::discover()` and `SkillManager::resolve()`.

#### Scenario: All consumers use SkillManager

- **WHEN** TUI app, daemon state, completion engine, and CLI skills list each need skill resolution
- **THEN** all consumers SHALL call `SkillManager` methods
- **AND** no consumer SHALL directly read skill directories or parse SKILL.md files

### Requirement: Startup logging of discovered skills

The system SHALL move startup skill logging into `SkillManager::discover()` with structured log events.

#### Scenario: Session starts with skills in multiple roots

- **WHEN** `SkillManager::discover()` scans multiple configured roots
- **THEN** a `RuntimeEvent::SkillDiscovery` event SHALL be emitted with count per root
- **AND** the existing trace-level log behavior SHALL be preserved
```

## openspec/changes/generic-agent-runtime/specs/declarative-workflow-definition/spec.md

- Source: openspec/changes/generic-agent-runtime/specs/declarative-workflow-definition/spec.md
- Lines: 1-132
- SHA256: ef876c078786f923d931cd3acbedbb70f81edd621db5d8306d60268b12085333

[TRUNCATED]

```md
# declarative-workflow-definition Specification

## Purpose

Define the YAML schema for `workflow.yaml` — the declarative format that describes a complete agent workflow (states, transitions, guards, context layers, routing rules, discovery, tool guard rules, presets, and skill bindings) interpreted by the generic Agent Runtime.

## ADDED Requirements

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
```

Full source: openspec/changes/generic-agent-runtime/specs/declarative-workflow-definition/spec.md

## openspec/changes/generic-agent-runtime/specs/external-skill-runtime/spec.md

- Source: openspec/changes/generic-agent-runtime/specs/external-skill-runtime/spec.md
- Lines: 1-48
- SHA256: 6c477f2b0112a96891c7290f937a3466da90e2d3ba38b967d7ed9fbca8b33906

```md
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
```

## openspec/changes/generic-agent-runtime/specs/hook-event-alignment/spec.md

- Source: openspec/changes/generic-agent-runtime/specs/hook-event-alignment/spec.md
- Lines: 1-41
- SHA256: 6a0b987b85248f374479a343de8591e118205459eee19ce962c5690b7ca38ce3

```md
# hook-event-alignment Delta Spec

## MODIFIED Requirements

### Requirement: REQ-HEA-001 — new event types

The `HookEvent` enum SHALL be moved from `src/hooks/` to `src/runtime/event.rs` as part of the generic `EventBus`. The existing event types (PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification, Stop, UserPromptSubmit, PermissionRequest) SHALL be preserved. New event types SHALL be added for Runtime-level events.

#### Scenario: Existing hook events preserved after migration

- **WHEN** hooks are configured with any existing event type (PreToolUse, PostToolUse, etc.)
- **AND** the hooks module has been migrated to `src/runtime/`
- **THEN** all existing event types SHALL fire identically to before the migration

#### Scenario: Runtime events added alongside hook events

- **WHEN** the `EventBus` emits a `RuntimeEvent::StateTransition`
- **THEN** the event SHALL be handled by the `EventBus` subscription system
- **AND** existing hook event types SHALL NOT be affected

### Requirement: REQ-HEA-004 — CC hooks format compatibility

`HookManager::from_settings()` SHALL remain compatible with Claude Code hooks format after migration to `src/runtime/`. The `cc_adapter` module SHALL be preserved.

#### Scenario: CC nested array format still parsed after migration

- **WHEN** hooks config uses CC format `{"Stop": [[{"type": "command", "command": "..."}]]}`
- **AND** `HookManager::from_settings()` is called from `src/runtime/`
- **THEN** hooks SHALL be correctly parsed into `Vec<HookDefinition>` with matcher and type fields

### Requirement: REQ-HEA-005 — backward compatibility

All existing hook behavior SHALL be preserved after migration. The `GuardPipeline` (new) SHALL run before `PreToolUse` hooks (existing), matching the current `CometGuard`-before-`PreToolUse` ordering.

#### Scenario: Guard pipeline runs, then PreToolUse hooks

- **WHEN** a tool is about to execute
- **AND** both the `GuardPipeline` and `PreToolUse` hooks are configured
- **THEN** the `GuardPipeline` SHALL evaluate first
- **AND** if the guard allows, `PreToolUse` hooks SHALL then execute
- **AND** this ordering SHALL match the previous comet-guard-before-hooks behavior
```

