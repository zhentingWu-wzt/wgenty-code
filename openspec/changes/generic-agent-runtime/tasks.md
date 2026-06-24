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

- [ ] 12.1 Write integration test: `/comet <description>` with no active change triggers discovery script, matches routing rule for 0 changes, calls InteractionService for clarification question, does NOT create artifacts before user confirms
- [ ] 12.2 Write integration test: `/comet <description>` with one active change matches routing rule for 1 change, presents continue-or-new question, user selecting "continue" resumes at current state
- [ ] 12.3 Write integration test: file_write in open state blocked by RuleBasedGuard from workflow.yaml tool_guards, error message matches configured message
- [ ] 12.4 Write integration test: `comet_slash_agent_prompt()` internal wrapper text does NOT appear in user-visible chat; only friendly status messages appear
- [ ] 12.5 Write integration test: new workflow (e.g., `hello-workflow`) can be added by creating `.wgenty-code/skills/hello/workflow.yaml` + `SKILL.md` with zero Rust changes, `/hello` command is routed correctly
- [ ] 12.6 Manual validation: full Comet flow from `/comet <description>` through open → design → build → verify → archive works identically to before the change; internal prompts hidden; decision points block; guard rules enforced
