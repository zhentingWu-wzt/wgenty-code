---
comet_change: generic-agent-runtime
role: technical-design
canonical_spec: openspec
---

# Generic Agent Runtime — Technical Design

## 1. Architecture Overview

**Core principle**: No new workflow abstractions (StateMachine, TransitionGuard, WorkflowEngine, GuardPipeline). Only extend the existing hooks system with 3 new primitives. Comet = YAML hook config + scripts + SKILL.md.

```
┌─────────────────────────────────────────────────────────────────┐
│                     Session Startup (once)                       │
│                                                                  │
│  workflow.yaml ──► Parse ──► Guards(Vec)  Layers(Vec)            │
│                              Routes(Vec)  StateHandle(Arc)       │
│                                                                  │
│  Engine disappears. Components receive pure data references.     │
└─────────────────────────────────────────────────────────────────┘
                              │
         ┌────────────────────┼────────────────────┐
         ▼                    ▼                     ▼
┌─────────────────┐  ┌───────────────┐  ┌──────────────────┐
│  ToolExecutor   │  │ PromptAssembler│  │   TUI Input      │
│  guards: Vec    │  │ layers: Vec   │  │  routes: Vec     │
│  state: Arc     │  │ state: Arc    │  │  state: Arc      │
└────────┬────────┘  └───────┬───────┘  └────────┬─────────┘
         │                   │                    │
         ▼                   ▼                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Extended Hooks System                          │
│                                                                  │
│  Events: PreToolUse | PostToolUse | SessionStart | SessionEnd   │
│          Notification | Stop | UserPromptSubmit                  │
│          PermissionRequest | SlashCommand  ◄── NEW               │
│                                                                  │
│  Actions: command (existing) | inject_context ◄── NEW            │
│           ask_user ◄── NEW                                       │
│                                                                  │
│  Conditions: matcher (existing) | when_state ◄── NEW             │
└─────────────────────────────────────────────────────────────────┘
```

### Data Injection Pattern

At session start, one function parses `workflow.yaml` and produces pure data. The engine struct is dropped — components hold plain `Vec`/`Arc` references with zero knowledge of "workflow" or "engine".

```rust
// Session startup — engine lives only here
fn inject_workflow(yaml_path: &Path) -> InjectedWorkflow {
    let def = parse_workflow_yaml(yaml_path)?;
    InjectedWorkflow {
        guards: def.build_guards(),          // Vec<HookDefinition>
        layers: def.build_layers(),          // Vec<ContextLayer>
        routes: def.build_routes(),          // Vec<CommandRoute>
        state: def.create_state_handle(),    // Arc<RwLock<str>>
    }
}
// Engine dropped. Components hold InjectedWorkflow fields.
```

### What Rust Does NOT Know

- Zero references to "comet", "openspec", "phase" in `src/runtime/`
- Zero workflow semantic concepts (brainstorming, design doc, guard apply)
- All domain logic lives in YAML + shell scripts + SKILL.md

## 2. Hook System Extensions

### 2.1 Current State

File: `src/hooks/mod.rs`

```rust
pub enum HookEvent {
    PreToolUse, PostToolUse, SessionStart, SessionEnd,
    Notification, Stop, UserPromptSubmit, PermissionRequest,
}

pub struct HookDefinition {
    pub command: String,
    pub timeout_secs: u64,
    pub matcher: Option<String>,       // pipe-separated tool names
    pub hook_type: Option<String>,     // "command" or "prompt"
}
```

Hooks execute shell commands. `HookManager::fire()` runs the command, passes JSON context via stdin, parses stdout for `{"continue_execution": bool, "reason": str}`.

### 2.2 Extension: SlashCommand Event

```rust
// Add to HookEvent enum
HookEvent::SlashCommand,
```

Fires when user types `/something`. The hook context carries:
- `tool_name`: the slash command (e.g., "comet")
- `tool_input`: JSON `{"command": "comet", "args": "fix the bug", "raw_input": "/comet fix the bug"}`

This enables hooks to react to workflow entry commands without hardcoded `comet_slash_agent_prompt()`.

### 2.3 Extension: inject_context Action

New hook action type — instead of running a shell command, injects text into the model context with visibility control.

```rust
pub enum HookAction {
    Command {
        command: String,
        timeout_secs: u64,
    },
    InjectContext {
        source: ContextSource,
        priority: u8,
        visibility: LayerVisibility,
    },
    AskUser {
        question: String,
        options: Vec<UserOption>,
    },
}

pub enum ContextSource {
    Template(String),        // "Current phase: {{ state }}"
    File(PathBuf),           // reads file content
    Inline(String),          // literal text
}

pub enum LayerVisibility {
    Internal,  // agent sees, user does not
    Visible,   // both see
}
```

When `InjectContext` fires, the `ContextAssembler` adds the rendered content to the appropriate stream (internal → hidden system message, visible → user chat).

### 2.4 Extension: ask_user Action

Pauses the agent loop, presents a question to the user, and resumes with the answer.

```rust
HookAction::AskUser {
    question: String,
    options: Vec<UserOption>,
}

pub struct UserOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}
```

The `InteractionService` trait handles the platform-specific presentation:
- TUI: `AppEvent::QuestionAsked` → renders inline choice widget → `AgentPhase::WaitingForInteraction`
- CLI: numbered terminal prompt → stdin read
- Headless: configured policy (Deny / PreConfiguredAnswers)

### 2.5 Extension: when_state Condition

Filters hooks by current workflow state.

```rust
pub struct HookDefinition {
    // ... existing fields
    pub when_state: Option<String>,  // "open|design" — pipe-separated
}
```

`HookManager::fire()` checks `when_state` against the shared `Arc<RwLock<str>>` before executing the hook. If the current state doesn't match, the hook is skipped. This replaces `CometGuard::check(state.phase, ...)` — the phase filtering happens at the hook level, not in a separate guard module.

### 2.6 Hook Execution Flow (after extension)

```
User types /comet fix bug
  → SlashCommand event fires
  → Hook with inject_context action injects comet skill instructions (internal)
  → Hook with ask_user action shows "No active change. Create one?" interaction
  → User selects option → state transitions via script

Agent calls Write tool during open phase
  → PreToolUse event fires
  → Hook with when_state: "open|design" + command action runs comet-guard.sh
  → Script returns {"continue_execution": false, "reason": "Write blocked in open phase"}
  → ToolExecutor blocks the call
```

## 3. ContextAssembler

### 3.1 Purpose

Replaces the hardcoded Layer 1b in `src/prompts/mod.rs` (lines 119-128) with a generic, priority-ordered, visibility-controlled context assembly.

### 3.2 Core Types

```rust
// src/runtime/context.rs

pub struct ContextLayer {
    pub id: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source: ContextSource,
    pub condition: Option<LayerCondition>,
}

pub enum LayerCondition {
    StateMatches(String),     // "build" — layer only active in this state
    VariableSet(String),      // "build_mode=subagent" — conditional on variable
}

pub struct ContextAssembler {
    layers: Vec<ContextLayer>,
    state: Arc<RwLock<str>>,
    variables: HashMap<String, String>,
}

pub struct AssembledContext {
    pub internal_instructions: Vec<String>,  // hidden from user
    pub visible_content: Vec<String>,        // user-visible
}
```

### 3.3 Assembly Algorithm

```
1. Filter layers by condition (state match, variable check)
2. Sort by priority (ascending — lower number = earlier in context)
3. For each layer, resolve source:
   - Template: render {{ state }}, {{ change }}, {{ build_mode }} etc.
   - File: read and optionally template-render
   - Inline: use verbatim
4. Route by visibility:
   - Internal → internal_instructions
   - Visible → visible_content
```

### 3.4 Integration with prompts/mod.rs

Before (hardcoded):
```rust
// Layer 1b: Comet Phase Awareness
if let Some(comet_state) = crate::comet::CometState::read(working_dir) {
    system_messages.push(ChatMessage::system(comet_state.phase_instruction()));
}
```

After (generic):
```rust
// Layer 1b: Workflow Context (from ContextAssembler)
if let Some(ref assembler) = context.context_assembler {
    let assembled = assembler.assemble();
    for instruction in &assembled.internal_instructions {
        system_messages.push(ChatMessage::system(instruction));
    }
}
```

`PromptContext` gains an optional `context_assembler: Option<Arc<ContextAssembler>>` field.

## 4. InteractionService

### 4.1 Trait Definition

```rust
// src/runtime/interaction.rs

#[async_trait]
pub trait InteractionService: Send + Sync {
    /// Present a question with options, return the selected value.
    async fn ask(&self, question: &InteractionQuestion) -> Result<UserAnswer>;

    /// Present a confirmation prompt, return true/false.
    async fn confirm(&self, prompt: &ConfirmPrompt) -> Result<bool>;
}

pub struct InteractionQuestion {
    pub id: String,
    pub message: String,
    pub options: Vec<InteractionOption>,
    pub multi_select: bool,
}

pub struct InteractionOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

pub struct UserAnswer {
    pub selected: Vec<String>,  // values of selected options
}

pub struct ConfirmPrompt {
    pub message: String,
    pub default_yes: bool,
}
```

### 4.2 TUI Implementation

```rust
// src/runtime/interaction_tui.rs

pub struct TuiInteractionService {
    event_tx: mpsc::UnboundedSender<AppEvent>,
    response_rx: Mutex<mpsc::UnboundedReceiver<UserAnswer>>,
}

impl InteractionService for TuiInteractionService {
    async fn ask(&self, question: &InteractionQuestion) -> Result<UserAnswer> {
        // 1. Send AppEvent::QuestionAsked(question, response_tx)
        // 2. AgentLoop pauses (AgentPhase::WaitingForInteraction)
        // 3. TUI renders inline choice widget
        // 4. User selects → response sent via oneshot channel
        // 5. AgentLoop resumes with answer
    }
}
```

### 4.3 CLI Implementation

```rust
// src/runtime/interaction_cli.rs
// Presents numbered choices on terminal, reads stdin.
// Tokio::task::spawn_blocking for stdin read.
```

### 4.4 Headless Implementation

```rust
// src/runtime/interaction_headless.rs
pub enum HeadlessPolicy {
    Deny,                          // All interactions fail
    PreConfigured(Vec<AnswerMap>), // Pre-configured answers keyed by question id
}
```

## 5. Module Structure

```
src/
├── runtime/
│   ├── mod.rs              # re-exports, module declarations
│   ├── hooks/
│   │   ├── mod.rs          # moved from src/hooks/mod.rs, extended
│   │   └── cc_adapter.rs   # moved from src/hooks/cc_adapter.rs
│   ├── guardian.rs         # moved from src/guardian/mod.rs
│   ├── context.rs          # NEW: ContextLayer, ContextAssembler
│   ├── interaction.rs      # NEW: InteractionService trait + types
│   ├── interaction_tui.rs  # NEW: TUI implementation
│   ├── interaction_cli.rs  # NEW: CLI implementation
│   └── interaction_headless.rs # NEW: headless implementation
├── comet/                  # DELETED entirely
├── hooks/                  # DELETED (moved to runtime/hooks/)
├── guardian/               # DELETED (moved to runtime/guardian.rs)
├── tools/
│   └── executor.rs         # MODIFIED: replace CometGuard with hooks when_state
├── prompts/
│   └── mod.rs              # MODIFIED: replace Layer 1b with ContextAssembler
├── tui/
│   └── app/
│       ├── input.rs         # MODIFIED: replace comet_slash_agent_prompt
│       └── mod.rs           # MODIFIED: initialize workflow at startup
└── knowledge/
    └── external_registry.rs # MODIFIED: remove comet_slash_agent_prompt,
                             #          route_slash_command → CommandRouter
```

## 6. Integration Points

### 6.1 ToolExecutor (src/tools/executor.rs)

**Current** (lines 152-174):
```rust
// Comet phase guard (before PreToolUse hooks)
if let Some(ref state) = self.comet_state {
    let decision = CometGuard::check(&state.phase, tool_name, &guard_args);
    if decision.blocked { /* block tool, fire notification hook */ }
}
// Then PreToolUse hooks
```

**After**: Remove the CometGuard block entirely. The `comet_state: Option<CometState>` field is replaced with `state_handle: Option<Arc<RwLock<str>>>`. PreToolUse hooks with `when_state` conditions replace the guard logic:

```rust
// PreToolUse hooks — when_state filtering handled by HookManager internally
let pre_ctx = HookManager::pre_tool_context(tool_name, &args, session_id)
    .with_state(self.state_handle.as_ref().map(|s| s.read().await.clone()));
let pre_outcomes = self.hook_manager.fire(&HookEvent::PreToolUse, &pre_ctx, None).await;
// If any hook blocked (e.g., comet-guard.sh returned continue_execution: false), block tool
```

The `HookContext` already has `comet_phase: Option<String>` — rename to `workflow_state: Option<String>` and populate from `state_handle`.

### 6.2 Prompt Assembler (src/prompts/mod.rs)

**Current** (lines 119-128):
```rust
if let Some(comet_state) = crate::comet::CometState::read(working_dir) {
    system_messages.push(ChatMessage::system(comet_state.phase_instruction()));
}
if crate::comet::CometGuard::is_coordinator_mode(working_dir) {
    system_messages.push(ChatMessage::system(crate::comet::CometGuard::coordinator_reminder()));
}
```

**After**: `PromptContext` gains `context_assembler: Option<Arc<ContextAssembler>>`. The hardcoded comet injection is replaced with:
```rust
if let Some(ref assembler) = context.context_assembler {
    let assembled = assembler.assemble();
    for instruction in &assembled.internal_instructions {
        system_messages.push(ChatMessage::system(instruction));
    }
}
```

Phase instructions, coordinator reminders, and phase guard rules are all defined as `ContextLayer` entries in `workflow.yaml` with `visibility: internal`.

### 6.3 TUI Input (src/tui/app/input.rs)

**Current** (lines 185-230):
```rust
let route = crate::knowledge::route_slash_command(&text, builtins, registry);
match route {
    crate::knowledge::SlashRoute::ExternalSkill { skill, args } => {
        let agent_input = crate::knowledge::comet_slash_agent_prompt(&skill, &args)...;
        // Shows "🔧 External skill '/...' detected..." message
    }
}
```

**After**: `route_slash_command()` is replaced with `CommandRouter`. `comet_slash_agent_prompt()` is deleted — its logic moves to a `SlashCommand` hook with `inject_context` action:

```rust
// Fire SlashCommand hooks
let slash_ctx = HookContext {
    event: "SlashCommand".to_string(),
    tool_name: Some(command.clone()),
    tool_input: Some(serde_json::json!({
        "command": command,
        "args": args,
    })),
    // ...
};
self.hook_manager.fire(&HookEvent::SlashCommand, &slash_ctx, None).await;

// Show only friendly status
self.committed_messages.push(UIMessage {
    role: MessageRole::System,
    content: format!("Starting {} workflow...", command),
    // ...
});
```

The `CommandRouter` is a thin wrapper that:
1. Checks built-in commands (unchanged)
2. Checks if any workflow.yaml defines the command in `entry_commands`
3. Returns `RouteResult::Workflow { name, args }` or `RouteResult::Unknown { suggestions }`

### 6.4 App Startup (src/tui/app/mod.rs)

At app startup, alongside `ExternalSkillRegistry` initialization:
```rust
// Discover and load workflow.yaml files
let workflow_config = WorkflowConfig::discover(&skill_roots)?;
if let Some(config) = workflow_config {
    let injected = inject_workflow(&config);
    // Inject into components
    tool_executor.set_state_handle(injected.state.clone());
    prompt_context.context_assembler = Some(Arc::new(injected.assembler));
    input_view.set_command_router(Arc::new(injected.router));
    // Register workflow hooks into HookManager
    hook_manager.register_workflow_hooks(injected.hooks);
    // Set interaction service
    app_state.interaction_service = Some(Arc::new(TuiInteractionService::new(event_tx)));
}
```

## 7. Comet workflow.yaml as Hook Configuration

The Comet workflow is defined entirely in `.wgenty-code/skills/comet/workflow.yaml`. The Rust runtime only interprets the hook primitives — it has zero knowledge of Comet semantics.

```yaml
# .wgenty-code/skills/comet/workflow.yaml
name: comet
entry_commands: [comet, comet-open, comet-design, comet-build, comet-verify, comet-archive]

state:
  initial: null
  read_script: "comet-state current --json"
  write_script: "comet-state set"

hooks:
  # ── SlashCommand: inject comet skill + discover OpenSpec state ──
  - event: SlashCommand
    matcher: "comet|comet-open|comet-design|comet-build|comet-verify|comet-archive"
    actions:
      - type: inject_context
        source: file
        path: "${SKILL_DIR}/SKILL.md"
        visibility: internal
      - type: command
        command: "openspec list --json"
        timeout_secs: 10

  # ── PreToolUse: phase guard via comet-guard.sh ──
  - event: PreToolUse
    when_state: "open|design|verify|archive"
    matcher: "Write|Edit|Bash"
    actions:
      - type: command
        command: "comet-guard check --phase ${STATE} --tool ${TOOL}"
        timeout_secs: 10

  # ── PreToolUse: phase guard exception for read-only commands ──
  - event: PreToolUse
    when_state: "open|design|verify|archive"
    matcher: "Bash"
    actions:
      - type: command
        command: "comet-guard check-readonly --command '${ARGS.command}'"
        timeout_secs: 5

  # ── UserPromptSubmit: phase guard rule injection ──
  - event: UserPromptSubmit
    when_state: "open|design|build|verify|archive"
    actions:
      - type: inject_context
        source: file
        path: "${SKILL_DIR}/references/phase-guard.md"
        priority: 25
        visibility: internal

context:
  # Phase awareness — highest priority, internal only
  - id: phase-instruction
    priority: 35
    visibility: internal
    condition:
      state_matches: "open|design|build|verify|archive"
    source:
      template: |
        Current Comet phase: {{ state }}
        Phase rules: {{ phase_rules[state] }}

  # Coordinator mode reminder (build phase only)
  - id: coordinator-reminder
    priority: 30
    visibility: internal
    condition:
      state_matches: "build"
      variable_set: "build_mode=subagent-driven-development"
    source:
      file: "${SKILL_DIR}/references/coordinator-reminder.md"

templates:
  phase_rules:
    open: |
      Phase: Open. Allowed: create proposal/design/tasks. Forbidden: source code changes.
    design: |
      Phase: Design. Allowed: brainstorming, create Design Doc. Forbidden: source code changes.
    build: |
      Phase: Build. Allowed: source code, tests, plan execution. Must confirm at decision points.
    verify: |
      Phase: Verify. Allowed: verification, branch handling. Must not skip failure handling.
    archive: |
      Phase: Archive. Allowed: archive confirmation. Forbidden: source code changes.
```

## 8. Hook System Internal Changes

### 8.1 HookDefinition Restructure

```rust
// Before (src/hooks/mod.rs)
pub struct HookDefinition {
    pub command: String,
    pub timeout_secs: u64,
    pub matcher: Option<String>,
    pub hook_type: Option<String>,
}

// After (src/runtime/hooks/mod.rs)
pub struct HookDefinition {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub when_state: Option<String>,        // NEW
    pub actions: Vec<HookAction>,          // was single command
}

pub enum HookAction {
    Command { command: String, timeout_secs: u64 },
    InjectContext { source: ContextSource, priority: u8, visibility: LayerVisibility },
    AskUser { question: String, options: Vec<UserOption>, multi_select: bool },
}
```

### 8.2 HookManager Changes

- `fire()` gains a `state: Option<&str>` parameter for `when_state` filtering
- `fire()` returns `Vec<HookOutcome>` — for `InjectContext`, the outcome contains the rendered text; for `AskUser`, the outcome contains the user's answer
- `register_workflow_hooks(hooks: Vec<HookDefinition>)` — merges workflow-defined hooks into the manager
- CC format compatibility (`cc_adapter.rs`) is preserved unchanged

### 8.3 HookContext Changes

```rust
pub struct HookContext {
    // ... existing fields
    pub comet_phase: Option<String>,  // → renamed to workflow_state: Option<String>
    pub variables: HashMap<String, String>, // NEW: template variables (state, change, etc.)
}
```

## 9. Deletion of src/comet/

### 9.1 Files Removed

| File | Replacement |
|------|-------------|
| `src/comet/mod.rs` | `src/runtime/mod.rs` |
| `src/comet/state.rs` | `workflow.yaml` state config + `Arc<RwLock<str>>` + script-based discovery |
| `src/comet/guard.rs` | PreToolUse hooks with `when_state` + `comet-guard.sh` |
| `src/comet/workflow.rs` | `workflow.yaml` routing rules + `CommandRouter` |
| `src/comet/protocol.rs` | Subagent dispatch protocol moved to Comet SKILL.md references |

### 9.2 Cleanup Verification

```bash
# Zero Comet/OpenSpec/phase references in runtime
grep -r "comet\|openspec\|phase" src/runtime/  # must return zero

# Zero CometPhase/CometState/CometGuard references in src/
grep -r "CometPhase\|CometState\|CometGuard\|comet_slash_agent_prompt" src/  # must return zero

# pub mod comet removed from lib.rs
grep "pub mod comet" src/lib.rs  # must return zero
```

## 10. CommandRouter

A thin newtype replacing `route_slash_command()` in `src/knowledge/external_registry.rs`.

```rust
// src/runtime/command.rs

pub struct CommandRouter {
    builtins: Vec<String>,
    workflow_commands: HashMap<String, String>,  // command → workflow name
}

pub enum RouteResult {
    BuiltIn,
    Workflow { name: String, command: String, args: String },
    Unknown { command: String, suggestions: Vec<String> },
    NotSlash,
}

impl CommandRouter {
    pub fn route(&self, input: &str, registry: Option<&ExternalSkillRegistry>) -> RouteResult {
        // 1. Check builtins
        // 2. Check workflow entry_commands
        // 3. Check ExternalSkillRegistry for suggestions
    }
}
```

The `comet_slash_agent_prompt()` function is deleted — its behavior is reproduced by a `SlashCommand` hook with `inject_context` action in `workflow.yaml`.

## 11. Migration Path

### Phase 1: Create runtime module (no existing code modified)
1. Create `src/runtime/mod.rs`, `context.rs`, `interaction.rs`
2. Move `src/hooks/` → `src/runtime/hooks/` (update all `crate::hooks` imports)
3. Move `src/guardian/mod.rs` → `src/runtime/guardian.rs` (update imports)
4. Extend `HookEvent` with `SlashCommand`
5. Add `HookAction` enum, `when_state` field to `HookDefinition`
6. Add `ContextAssembler`, `InteractionService` trait
7. Add `CommandRouter` in `src/runtime/command.rs`

### Phase 2: Create workflow.yaml + integration tests
1. Create `.wgenty-code/skills/comet/workflow.yaml`
2. Write integration tests validating the YAML produces identical behavior

### Phase 3: Modify consumers
1. `src/tools/executor.rs` — replace `CometGuard::check()` with hooks `when_state`
2. `src/prompts/mod.rs` — replace Layer 1b with `ContextAssembler`
3. `src/tui/app/input.rs` — replace `comet_slash_agent_prompt()` with `CommandRouter` + `SlashCommand` hooks

### Phase 4: Delete src/comet/
1. Remove `src/comet/` directory
2. Remove `pub mod comet` from `src/lib.rs`
3. Clean up remaining `use crate::comet` imports
4. Run full test suite, verify grep assertions

Each phase is independently testable and revertible.

## 12. Error Handling

| Failure Mode | Handling |
|---|---|
| `workflow.yaml` parse error | Session starts without workflow — all hooks/tools operate normally (passthrough). Log error at `warn` level |
| `workflow.yaml` invalid state reference | Validation at load time — log error, session starts without workflow |
| Hook script (comet-guard.sh) exits non-zero | Hook treated as failed, tool NOT blocked (fail-open). Log at `error` level |
| Hook script times out | Same as non-zero exit — fail-open |
| `AskUser` in headless mode with Deny policy | Return error to hook pipeline, workflow cannot proceed |
| `inject_context` file not found | Log warning, skip the layer, continue |
| State read script fails | State defaults to `null`, `when_state` conditions with specific states don't match |

Design principle: **fail-open for tool execution** (a broken guard doesn't block legitimate work), **fail-closed for user interactions** (don't auto-approve decisions).

## 13. Testing Strategy

### Unit Tests
- `ContextAssembler`: priority ordering, visibility separation, template rendering, conditional skip, file source
- `HookManager`: `when_state` filtering, `SlashCommand` firing, `inject_context` action, `ask_user` action, backward compatibility (existing event types)
- `CommandRouter`: builtin match, workflow match, unknown, NotSlash
- `InteractionService`: TUI event flow, CLI stdin mock, headless policy

### Integration Tests
- Full Comet flow: `/comet` → state discovery → routing → phase context injection → tool guard blocks Write in open → guard allows Read → user confirmation at transition
- New workflow: create `workflow.yaml` + `SKILL.md` for `/hello`, verify routing + context without Rust changes
- Hook format compatibility: CC nested arrays still parsed correctly after migration
- Internal prompts hidden: `grep` internal text NOT present in user-visible messages

### Regression Tests
- All existing hooks tests pass unchanged
- All existing slash commands (`/clear`, `/help`, `/plan`, `/continue`, `/undo`, `/init`) behave identically
- Guardian security checks still block critical/dangerous commands
- Agent loop, tool system, MCP, API clients unaffected

### Manual Validation
- Full Comet flow: `/comet <description>` → open → design → build → verify → archive
- Internal prompts not visible to user
- Decision points block until user responds
- `grep -r "comet\|openspec\|phase" src/runtime/` returns zero results

## 14. Risks and Mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| Hook script performance (shell invocation per tool call) | Medium | `when_state` filters at Rust level — scripts only run when state matches. Lightweight state check is a string comparison |
| YAML complexity equals deleted Rust code | Medium | Keep YAML minimal. Complex logic stays in scripts and SKILL.md references |
| CC hook format incompatibility | Low | `cc_adapter.rs` preserved unchanged. Tests verify CC format parsing |
| `when_state` typo causes guard not to fire | Low | Runtime warns on undeclared state values at YAML load time |
| Context visibility leak (internal shown to user) | High | `ContextAssembler` strictly separates streams. Integration test verifies internal text absent from user messages |
| Session without workflow loses all guard rules | Medium | Correct — this is the design. No active workflow = no restrictions. Guardian still provides security baseline |

## 15. Implementation Divergence (recorded during verify phase, 2026-06-26)

The open-phase delta specs describe architectural concepts (WorkflowEngine, StateMachine, TransitionGuard, GuardPipeline, RuleBasedGuard, ScriptRunner, EventBus, StateSource source abstraction, SkillManager) that were **deliberately not implemented**. The brainstorming phase confirmed a hooks-only approach: extend the existing hooks system with 3 new primitives (SlashCommand event, inject_context/ask_user actions, when_state condition) rather than introduce new workflow abstractions in Rust.

### Specific divergences

| Delta Spec | Spec Concept | Implementation Reality |
|---|---|---|
| agent-runtime-engine | WorkflowEngine parsing/validating YAML | workflow.yaml parsed with simple `parse_yaml_list()` for entry_commands only; full YAML structure consumed at design-time by human readers |
| agent-runtime-engine | StateMachine with transition guards | String-keyed state via `Arc<RwLock<str>>` + hook `when_state` filtering; state transitions driven by shell scripts |
| agent-runtime-engine | GuardPipeline + RuleBasedGuard | Tool blocking via PreToolUse hooks with `when_state`; `comet-guard.sh` script evaluates phase rules |
| agent-runtime-engine | ScriptRunner with JSON protocol | Shell execution embedded in HookManager; no standalone runner type |
| agent-runtime-engine | EventBus + RuntimeEvent types | HookEvent system extended (SlashCommand) rather than separate event bus |
| agent-runtime-engine | StateSource trait hierarchy | State discovery via shell scripts referenced in workflow.yaml; no Rust abstraction |
| comet-phase-guard | RuleBasedGuard reading YAML tool_guards | Hook `when_state` filtering + script evaluation replaces Rust-level guard rules |
| comet-skill-path-compat | SkillManager with progressive disclosure | ExternalSkillRegistry kept as-is; SkillManager not introduced |
| hook-event-alignment | GuardPipeline runs BEFORE PreToolUse hooks | PreToolUse hooks ARE the guard mechanism; no separate pipeline |

### Rationale

The hooks-only design was confirmed as the final approved approach. It satisfies all functional requirements (state-based tool blocking, context injection, user interaction, slash command routing) without introducing new type-system abstractions in Rust. The trade-off is that some workflow semantics (YAML validation, transition guard types, state discovery protocol) are checked at runtime by scripts rather than at compile-time by Rust types.

### Impact on delta specs

The delta specs remain as originally written (open phase). They describe design intent and acceptance criteria but diverge from implementation in architectural details. This divergence is **accepted** — the implementation follows the confirmed final design. Delta spec alignment is deferred to a follow-up change.
