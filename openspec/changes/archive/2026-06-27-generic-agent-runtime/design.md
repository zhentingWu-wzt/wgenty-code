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
- **[Risk] Script reliability**: Shell scripts as the runtime-to-external bridge could fail in different environments. **Mitigation**: Scripts already work today; the runtime only needs to invoke them and check exit codes, which is already tested.
- **[Risk] Performance**: Reading and parsing YAML on every tool call or state transition could be slow. **Mitigation**: Parse once at session start, cache the `WorkflowDefinition` in memory. Re-parse only if file mtime changes.
- **[Trade-off] String-keyed state machine loses compile-time exhaustiveness checks**: A typo in a state name in workflow.yaml won't be caught until runtime. **Mitigation**: Validate workflow.yaml at load time — check that all transition `from`/`to` values reference declared states, all guard types are registered, all script paths exist.

## Migration Plan

1. **Phase 1**: Add `src/runtime/` modules (pure engine, no existing code modified)
2. **Phase 2**: Add `workflow.yaml` for Comet; write integration tests validating the YAML produces identical behavior
3. **Phase 3**: Modify `src/tools/executor.rs` to use `GuardPipeline`, reading rules from workflow.yaml
4. **Phase 4**: Modify `src/tui/app/input.rs` to use `CommandRouter` instead of `comet_slash_agent_prompt()`
5. **Phase 5**: Modify `src/prompts/mod.rs` to use `ContextAssembler` instead of hardcoded Layer 1b
6. **Phase 6**: Delete `src/comet/`; migrate `src/hooks/` and `src/guardian/` into `src/runtime/`

Each phase is independently testable. Rollback: revert to any prior phase. The `src/comet/` directory is only deleted in Phase 6 after all consumers are migrated.

## Open Questions

1. **Script output protocol**: Should scripts communicate structured results via stdout JSON, or exit code + stderr only? JSON on stdout enables richer guard failure messages but requires all scripts to conform.
2. **Multiple workflow support**: Current design assumes one active workflow per session. When multiple workflows are needed, how does `CommandRouter` resolve `/comet` vs `/review` vs custom commands? Prefix-based? Registry-based?
3. **workflow.yaml hot reload**: If the YAML changes mid-session (e.g., during Comet build phase), should the runtime re-read it or use the cached version? Re-reading could change rules mid-flow.
