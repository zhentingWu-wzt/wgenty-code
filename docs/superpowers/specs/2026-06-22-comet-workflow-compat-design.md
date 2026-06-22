---
comet_change: comet-workflow-compat
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-22-comet-workflow-compat
status: final
---

# Comet Workflow Compatibility — Technical Design

## 1. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      TUI App (main session)                  │
│  ┌──────────┐  ┌──────────────┐  ┌──────────────────────┐   │
│  │  Input   │  │ Agent Loop   │  │  Event Handler       │   │
│  │ submit_  │  │ run_agent_   │  │ handle AppEvent      │   │
│  │ input()  │  │ loop()       │  │                      │   │
│  │          │  │              │  │ SessionStart/End fire │   │
│  │ UserPr.  │  │ resolve_tool │  │ Stop fire            │   │
│  │ Submit   │  │ _timeout()   │  │                      │   │
│  │ fire     │  │              │  │                      │   │
│  └────┬─────┘  └──────┬───────┘  └──────────┬───────────┘   │
│       │               │                     │               │
│       │     ┌─────────┼─────────────────────┘               │
│       │     │         │                                     │
│       │     │  ┌──────▼──────────┐                          │
│       │     │  │  DaemonClient   │                          │
│       │     │  │  tool exec API  │                          │
│       │     │  └──────┬──────────┘                          │
│       │     │         │                                     │
└───────┼─────┼─────────┼─────────────────────────────────────┘
        │     │         │
        │     │  ┌──────▼──────────────────────────────┐
        │     │  │            Daemon State              │
        │     │  │                                      │
        │     │  │  ┌──────────────────────────────┐    │
        │     │  │  │       ToolExecutor            │    │
        │     │  │  │                              │    │
        │     │  │  │  1. CometGuard::check()  ◄───┼────┤── NEW
        │     │  │  │  2. PreToolUse hooks          │    │
        │     │  │  │  3. Tool execute              │    │
        │     │  │  │  4. PostToolUse hooks         │    │
        │     │  │  │  5. Notification (phase block)│    │
        │     │  │  └──────────────────────────────┘    │
        │     │  │                                      │
        │     │  │  ┌──────────────────────────────┐    │
        │     │  │  │      ToolRegistry             │    │
        │     │  │  │  git_operations (worktree)    │    │
        │     │  │  │  execute_command (timeout)    │    │
        │     │  │  │  task (comet_context)         │    │
        │     │  │  │  skill (root resolver)        │    │
        │     │  │  └──────────────────────────────┘    │
        │     │  └──────────────────────────────────────┘
        │     │
        │  ┌──▼──────────────────────────────────┐
        │  │         src/comet/ (NEW)             │
        │  │                                      │
        │  │  state.rs  — read .comet.yaml        │
        │  │  guard.rs  — phase → tool allow/deny │
        │  │  workflow.rs — active change list    │
        │  └──────────────────────────────────────┘
        │
   ┌────▼─────────────────────────────────────┐
   │        src/knowledge/                    │
   │  root_resolver.rs (NEW)                  │
   │  → project .wgnty-code/skills/          │
   │  → ~/.wgnty-code/skills/                │
   │  → ~/.claude/skills/                    │
   └──────────────────────────────────────────┘
```

## 2. Component Designs

### 2.1 SkillRootResolver (`src/knowledge/root_resolver.rs`)

New singleton providing unified skill root discovery.

```rust
pub struct SkillRootResolver;

impl SkillRootResolver {
    /// Returns ordered roots: project > user wgenty > user claude
    pub fn roots() -> Vec<ExternalSkillRoot> {
        let home = dirs::home_dir().unwrap_or_default();
        let project = std::env::current_dir().unwrap_or_default();
        vec![
            ExternalSkillRoot::new(
                project.join(".wgenty-code").join("skills"),
                ExternalSkillSource::ProjectWgentyCode { root: project.join(".wgenty-code").join("skills") },
            ),
            ExternalSkillRoot::new(
                home.join(".wgenty-code").join("skills"),
                ExternalSkillSource::UserWgentyCode { root: home.join(".wgenty-code").join("skills") },
            ),
            ExternalSkillRoot::new(
                home.join(".claude").join("skills"),
                ExternalSkillSource::UserClaude { root: home.join(".claude").join("skills") },
            ),
        ]
    }
}
```

Consumers (all replaced to call `SkillRootResolver::roots()`):
- `DaemonState::new()` at `src/daemon/state.rs:145`
- `App::new()` at `src/tui/app/mod.rs:155`
- `AppEvent::ConfigChanged` handler at `src/tui/app/event.rs:693`
- `CompletionEngine::load()` at `src/tui/completion.rs:76`
- `run_skills()` CLI handler at `src/cli/args.rs:796`

### 2.2 Hook Lifecycle Completion

Each new fire site carries a `HookContext` with `session_id`, `working_directory`, `timestamp`, and `comet_phase` (when active).

| Event | Fire Location | Trigger Condition |
|---|---|---|
| `SessionStart` | `App::new()` end | After all init, before event loop |
| `SessionEnd` | `App::run()` before daemon shutdown | On quit |
| `UserPromptSubmit` | `submit_input()` top | After built-in cmd check, before slash route |
| `Stop` | `AppEvent::TurnComplete` / `TurnAborted` handler | Turn finishes or aborts |
| `PermissionRequest` | `execute_tool_with_permission()` before `PermissionRequired` event | Tool needs user approval |
| `Notification` | `ToolExecutor::execute_with_hooks()` when comet guard blocks | Phase restriction hit |

All hook fires are async (tokio::spawn) and do not block the main loop. Timeout per hook is configurable via `HookDefinition.timeout_secs` (default 30s).

### 2.3 Comet Phase Guard (`src/comet/`)

**state.rs** — reads active change state:
```rust
pub struct CometState {
    pub change_name: String,
    pub phase: CometPhase,
    pub workflow: WorkflowType,
    pub build_mode: Option<BuildMode>,
    pub isolation: Option<IsolationType>,
}

impl CometState {
    /// Scan openspec/changes/*/.comet.yaml, return first non-archived active change.
    pub fn read(working_dir: &Path) -> Option<Self> { ... }
}

pub enum CometPhase {
    Open,
    Design,
    Build,
    Verify,
    Archive,
}
```

**guard.rs** — tool restriction matrix:

| Phase | Blocked Tools | Exceptions |
|---|---|---|
| `Open` / `Design` | file_write, file_edit, apply_patch | User approval via ask_user_question |
| `Build` | (none) | — |
| `Verify` | file_write, file_edit, apply_patch | User approval via ask_user_question |
| `Archive` | file_write, file_edit, apply_patch, execute_command(mutating) | User approval via ask_user_question |

Read-only tools (file_read, grep, glob, web_search, web_fetch, lsp, git status/log/diff) are allowed in all phases.

**Integration point** — `ToolExecutor::execute_with_hooks()`:
```rust
pub async fn execute_with_hooks(...) -> ChatMessage {
    // NEW: Comet guard check BEFORE PreToolUse hooks
    if let Some(decision) = self.comet_guard.check(tool_name, args) {
        if decision.blocked {
            return ChatMessage::tool(id, &decision.error_message);
        }
    }
    // Existing PreToolUse hook
    // Existing tool execute
    // Existing PostToolUse hook
}
```

**workflow.rs** — lists active changes:
```rust
pub fn active_changes(working_dir: &Path) -> Vec<ChangeInfo> { ... }
```

### 2.4 Worktree Operations (`git_operations` extension)

New operations in `GitOperationsTool`:

```
worktree_add:
  path: string (required) — relative path under .wgenty-code/worktrees/
  branch: string (required) — new branch name
  base_ref: string (optional, default: "origin/main")

worktree_remove:
  path: string (required)
  force: boolean (optional, default: false)
  Without force, refuses if uncommitted changes exist.

worktree_list:
  (no additional params)
```

All worktree commands execute with `current_dir` set to the git repository root (detected from `.git`).

### 2.5 Configurable Tool Timeout

New function in `src/tui/agent/core.rs`:

```rust
fn resolve_tool_timeout(tool_name: &str, args: &serde_json::Value) -> Duration {
    match tool_name {
        "task" | "delegate" => Duration::from_secs(300),
        "execute_command" | "exec_command" => {
            let user_timeout = args["timeout"].as_u64().unwrap_or(60);
            Duration::from_secs((user_timeout + 30).max(120))
        }
        _ => Duration::from_secs(120),
    }
}
```

Replaces the inline ternary at `src/tui/agent/core.rs:322`.

### 2.6 Subagent Comet Context

**TaskTool schema extension:**
```json
{
  "comet_context": {
    "type": "object",
    "properties": {
      "change": { "type": "string" },
      "task_index": { "type": "integer" }
    }
  }
}
```

When present, the subagent's system prompt is prefixed with Comet implementer instructions including the change name, task index, and TDD protocol.

**Coordinator guard** — when Comet `build_mode: subagent-driven-development` is active:
- System message reminder: "You are a Comet coordinator. Do not directly execute source file writes. Dispatch each task to a subagent via the task tool."
- CometGuard in ToolExecutor enforces this for file_write/file_edit/apply_patch (unless user approves via ask_user_question).

**Progress persistence** — coordinator (main session) writes `.comet/subagent-progress.md` after each stage:
```
## Task 3 — Implement
- Status: complete
- Subagent: subagent:implement-task-3
- Summary: ...

## Task 3 — Spec Review
- Status: pass
- Subagent: subagent:review-spec-task-3
- Issues: none

## Task 3 — Quality Review
- Status: pass
- Subagent: subagent:review-quality-task-3
- Issues: none

## Task 3 — Complete
- Commit: abc1234
```

## 3. Error Handling

| Scenario | Behavior |
|---|---|
| No active .comet.yaml | CometGuard returns `None` (allow all) |
| Multiple active changes | Log warning, apply most restrictive rules across all |
| .comet.yaml malformed | Log error, fall back to `None` (allow all) — don't crash |
| Hook execution timeout | Log warning, don't block tool execution |
| Worktree path collision | Return git error message to agent |
| Subagent timeout during review | Coordinator spawns new review subagent (max 1 retry) |

## 4. Testing Strategy

- **Unit tests**: `SkillRootResolver::roots()`, `CometState::read()`, `CometGuard::check()`, `resolve_tool_timeout()`
- **Integration tests**: Hook fire + context correctness per event type; worktree add/list/remove round-trip; execute_command with explicit timeout
- **Manual verification**: Full `/comet` flow from open through archive in Wgenty Code TUI
