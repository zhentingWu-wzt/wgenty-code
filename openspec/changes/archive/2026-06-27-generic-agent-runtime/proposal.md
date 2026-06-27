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
