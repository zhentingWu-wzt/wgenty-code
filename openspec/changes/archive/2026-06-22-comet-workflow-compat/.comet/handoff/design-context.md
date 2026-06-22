# Comet Design Handoff

- Change: comet-workflow-compat
- Phase: design
- Mode: compact
- Context hash: 4a30b136516518b99b1f931f00bc24e02c721fa1690529135027f2d0c7113914

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/comet-workflow-compat/proposal.md

- Source: openspec/changes/comet-workflow-compat/proposal.md
- Lines: 1-84
- SHA256: 4459ef177cc27d5bf30eeed2c0554141d4bb041e5764324b94ad8cb9bb518ff1

[TRUNCATED]

```md
## Why

Wgenty Code 已具备 slash command、external skill、subagent/RLM、hooks、OpenSpec artifacts 等基础设施，上一篇深度分析确认它已覆盖 Comet 工作流的 70%–80%。但在运行严格 `/comet` 流程时仍有几个硬缺口：skill 运行时路径不完备、hook 生命周期未全量触发、Comet phase guard 不是 runtime 原生强约束、worktree 隔离缺一等能力、长命令（如 verify）可能被外层 120s timeout 截断、subagent-driven-development 没有 Comet 专属编排/审查/恢复闭环。

本 change 的目标是补齐这些关键闭环，使 Wgenty Code 能从"依赖 agent 自觉的半自动 Comet"升级到"被 runtime 硬约束保护且可恢复的严格 Comet 工作流"。这与项目"Claude Code ecosystem compat"的长期方向一致。

## What Changes

### External skill / slash command 路径兼容
- 运行时 external skill registry 扩展覆盖 `~/.claude/skills/`（当前只认 `~/.wgenty-code/skills/` 和项目 `.wgenty-code/skills/`）
- TUI completion、daemon registry、skill tool registry 使用同一套 root resolution
- 启动时显示实际发现的所有 skill roots 和可用 skills 数量

### Hook 生命周期全量触发
- 补上 `SessionStart` / `SessionEnd` / `UserPromptSubmit` / `Stop` / `PermissionRequest` / `Notification` 的实际 fire 点
- PreToolUse hook 支持按 phase 分类拦截（open/design 阶段禁止写源码等）
- hook context 携带 phase 信息，使 Comet guard hook 能基于 `.comet.yaml` 做决策

### Comet phase guard 硬约束
- 新增 `src/comet/` 模块：读取 `.comet.yaml`、active change 发现、按 phase 返回允许/禁止的工具操作
- 每个 tool execute 前通过 PreToolUse hook 或内置 comet guard 检查是否超出当前 phase 允许范围
- agent loop 每轮开始前自动检测 active `.comet.yaml` 并注入 phase context 到系统消息（用于提示模型）

### Worktree 一等隔离
- `git_operations` 工具扩展 `worktree_add` / `worktree_remove` / `worktree_list` 操作
- 或新增独立 `enter_worktree` / `exit_worktree` 工具
- 与 Comet build 阶段 `isolation: worktree` 对接，确保 session cwd 切换到 worktree 目录
- 退出时支持 keep / remove（含 discard_changes 检查）

### 长命令 timeout 解除
- `execute_command` 的外层 timeout 从硬编码 120s 改为读取 tool args 中的 `timeout` 字段
- 或将测试/长验证命令引入 background tool / task tool 路径，避免被主 loop 截断
- 确保 `/comet-verify` 场景下 `cargo test --all` 不会被截断

### Subagent-driven-development Comet 专属编排
- 协调者 mode 下主会话不直接执行 task
- 每个 implementer 自动加载 TDD skill
- 双审查闭环：spec compliance reviewer + code quality reviewer
- 审查不通过自动 spawn fix agent（最多 N 轮）
- tasks.md 定向勾选 + 立即 git commit
- 断点恢复通过 `.comet/subagent-progress.md`

## Capabilities

### New Capabilities
- `comet-skill-path-compat`: external skill registry 扩展，统一 runtime / TUI / daemon 的 skill root discovery，覆盖 `~/.claude/skills`
- `hook-lifecycle-complete`: 补充 hook 事件的实际 fire 点，使 Comet guard 能作为硬约束运行
- `comet-phase-guard`: runtime 阶段守卫，基于 `.comet.yaml` 的 phase 字段限制工具操作
- `worktree-isolation-tool`: git worktree 操作的一等工具支持，含 enter / exit / list
- `long-command-timeout-config`: execute_command 等工具的外层 timeout 可配置，不硬截断长验证
- `comet-subagent-orchestrator`: Comet 专属的 implementer→reviewer×2→fixer→commit 编排调度与恢复

### Modified Capabilities
（本次不修改已有 spec；external skill 和 hook 的扩展不会改变已有 capability 的 spec 级行为。）

## Impact

### 影响的主要代码区域
- `src/knowledge/external_registry.rs` — 扩展 root discovery
- `src/tui/completion.rs` — 统一 root resolution
- `src/daemon/state.rs` — 统一 root resolution + comet module 初始化
- `src/hooks/mod.rs` — 补 hook fire site
- `src/tools/executor.rs` — comet guard 检查
- `src/tools/execution/git_operations.rs` — worktree 操作
- `src/tools/execution/execute_command.rs` — timeout 配置化
- `src/tui/agent/core.rs` — 外层 timeout 解除 + comet phase context 注入
- `src/tui/app/input.rs` — UserPromptSubmit hook fire
- `src/tui/app/mod.rs` — SessionStart/SessionEnd hook fire
- `src/comet/` — 新模块（state / guard / workflow）
- `src/teams/` — subagent orchestrator 扩展

### 依赖
- external skill registry 根目录配置（已有）
- openspec CLI（外部依赖，不改变）
- Comet scripts（外部依赖，不改变）
- git CLI（已有依赖）
- hook manager（已有依赖）

### 非目标
- 不重写 OpenSpec CLI
```

Full source: openspec/changes/comet-workflow-compat/proposal.md

## openspec/changes/comet-workflow-compat/design.md

- Source: openspec/changes/comet-workflow-compat/design.md
- Lines: 1-191
- SHA256: 53a712065bc3077e0c61014efe89b73c3d873fb4cc26731853b7326272bedd56

[TRUNCATED]

```md
## Context

Wgenty Code 是一个 Rust 重写的 coding agent CLI/TUI，已实现了 70%–80% 的 Comet/OpenSpec+Superpowers 工作流基础设施。当前状态：

- **已有**: external skill registry、`skill` tool、slash command 路由、`ask_user_question` 阻塞确认、plan mode/`update_plan`、subagent/RLM/TaskTool、hook 框架（8 种事件类型）、git operations（branch/checkout/commit）、sandbox/guardian 权限、session 持久化
- **已有但不完整**: hook 生命周期只实际 fire 了 PreToolUse/PostToolUse；skill root discovery 在 runtime 和 daemon 侧不覆盖 `~/.claude/skills`；worktree 没有一等工具；长命令被外层 120s 硬截断
- **缺失**: Comet phase 原生状态感知、subagent-driven-development Comet 专属编排

本 design 不重写 OpenSpec CLI 或 Comet scripts，也不将 Comet 内建为 Rust 原生状态机。它补齐 runtime 约束层，让 Comet 的 shell 脚本 + skill 指令能通过 hooks 和工具能力得到硬约束保护。

## Goals / Non-Goals

**Goals:**
1. External skill runtime 能发现并加载 `~/.claude/skills/` 下的 Comet skills
2. Hook 8 种事件类型全部实际 fire，使 Comet phase guard hook 可全生命周期拦截
3. 在 tool executor 中引入 Comet phase guard，按 `.comet.yaml` 的 `phase` 限制工具操作
4. Git tool 新增 worktree 一等操作
5. `execute_command` 的外层 timeout 由 tool args 控制，不再硬截断
6. Subagent 编排支持 Comet 专属 implementer→双 review→fix→commit 闭环

**Non-Goals:**
- 不重写或内建 OpenSpec CLI
- 不重写 Comet scripts（comet-state / comet-guard / comet-handoff / comet-archive）
- 不改变模型 provider
- 不新增 UI 组件（hook/comet guard 复用现有 TUI 组件）
- 不实现完整的 phase 自推进状态机（仍由 skill 指令 + scripts 驱动）
- 不修改已有 spec 的 requirement 级行为

## Decisions

### D1: Skill root discovery 统一到 `SkillRootResolver`

**选择**: 新增 `SkillRootResolver` 单例，TUI app、daemon state、completion engine 和 CLI skills list 都通过它获取统一的 root 列表。

**Root 列表（按优先级）**:
1. 项目 `.wgenty-code/skills/`
2. 用户 `~/.wgenty-code/skills/`
3. 用户 `~/.claude/skills/`（**新增**）

**替代方案**: 
- 在各处分别硬编码 root 列表 → 拒绝，会导致不同路径下 `/comet` 发现行为不一致
- 通过 settings.json 配置额外 root → 保留为未来增强，但默认应覆盖 `~/.claude/skills`

**对应文件**:
- 新增 `src/knowledge/root_resolver.rs`
- 修改 `src/tui/app/mod.rs:155`、`src/daemon/state.rs:145`、`src/tui/completion.rs:76`、`src/cli/args.rs:796`

### D2: Hook firing 补充策略

**选择**: 在关键生命周期节点添加 hook fire，hook context 中携带 `comet_phase` 等信息。

| Hook Event | Fire 位置 | 说明 |
|---|---|---|
| `SessionStart` | `App::new()` 结束时 | session 创建后立即 fire |
| `SessionEnd` | `App::run()` 退出前 | daemon shutdown 前 fire |
| `UserPromptSubmit` | `submit_input()` 入口，slash command 路由前 | 携带 raw input text |
| `Stop` | agent loop 正常完成（TurnComplete）/ 被打断（TurnAborted）时 | 携带 turn result |
| `PermissionRequest` | `execute_tool_with_permission()` 发送 PermissionRequired event 前 | 携带 tool name + reason |
| `Notification` | comet guard 检查时、phase transition 时 | 用于 comet guard → agent 通知 |
| `PreToolUse` | 已有 | 扩展 comet phase context |
| `PostToolUse` | 已有 | 不变 |

**替代方案**:
- 只补 UserPromptSubmit + Stop → 不够，SessionStart/End 对 comet context recovery 很重要

### D3: Comet phase guard 集成方式

**选择**: 在 `ToolExecutor::execute_with_hooks()` 中，PreToolUse hook 之前插入 `CometGuard::check()`。

```rust
// 伪代码
let phase = CometState::read(working_dir)?.phase;
let allowed = CometGuard::check(phase, tool_name, args);
if !allowed {
    return ChatMessage::tool(id, "BLOCKED: phase restriction");
}
// ... 原有 PreToolUse hook + tool execute
```

**Phase 限制矩阵**（存储在 `src/comet/guard.rs`）:
```

Full source: openspec/changes/comet-workflow-compat/design.md

## openspec/changes/comet-workflow-compat/tasks.md

- Source: openspec/changes/comet-workflow-compat/tasks.md
- Lines: 1-51
- SHA256: a44efb61007f435ef335469fdca0b6cba92dbd7d1ad5183672b4e6c2135f158e

```md
## 1. Skill path compat — Unified root resolver

- [ ] 1.1 Add `src/knowledge/root_resolver.rs`: `SkillRootResolver` struct with `roots() -> Vec<ExternalSkillRoot>` that returns project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`, user `~/.claude/skills/` in priority order
- [ ] 1.2 Wire `SkillRootResolver` into `DaemonState::new()` at `src/daemon/state.rs:145`, replacing the inline `vec!` of roots
- [ ] 1.3 Wire `SkillRootResolver` into `App::new()` at `src/tui/app/mod.rs:155`, replacing the inline `vec!` of roots
- [ ] 1.4 Wire `SkillRootResolver` into `AppEvent::ConfigChanged` at `src/tui/app/event.rs:693`, replacing the inline `vec!` of roots
- [ ] 1.5 Wire `SkillRootResolver` into `CompletionEngine::load()` at `src/tui/completion.rs:76`, adding `~/.claude/skills/` to the scan roots
- [ ] 1.6 Wire `SkillRootResolver` into `run_skills()` at `src/cli/args.rs:796`, for CLI `skills list` consistency
- [ ] 1.7 Add startup trace log: count of discovered skills and roots scanned (in `App::new()`)

## 2. Hook lifecycle — Complete all 8 event fire sites

- [ ] 2.1 Fire `SessionStart` hook at end of `App::new()` in `src/tui/app/mod.rs`, after all initialization is done
- [ ] 2.2 Fire `SessionEnd` hook before daemon shutdown in `App::run()` at `src/tui/app/mod.rs` (or in the daemon shutdown path)
- [ ] 2.3 Fire `UserPromptSubmit` hook at start of `submit_input()` in `src/tui/app/input.rs`, after built-in command handling but before slash command routing, carrying raw input text as `tool_input`
- [ ] 2.4 Fire `Stop` hook on `AppEvent::TurnComplete` and `AppEvent::TurnAborted` in the event handler at `src/tui/app/event.rs`, carrying turn finish/abort reason
- [ ] 2.5 Fire `PermissionRequest` hook in `execute_tool_with_permission()` at `src/tui/agent/tool_dispatch.rs:122`, before sending the `PermissionRequired` event
- [ ] 2.6 Add `Notification` hook fire in comet guard module (see task 3.4) with subtype `comet_phase_block`
- [ ] 2.7 Ensure hook context JSON carries `session_id`, `working_directory`, `timestamp`, and `comet_phase` (when active) in all relevant hook firings

## 3. Comet phase guard — New `src/comet/` module

- [ ] 3.1 Create `src/comet/mod.rs`: re-exports `state`, `guard`, `workflow`
- [ ] 3.2 Create `src/comet/state.rs`: `CometState` struct with `.read(working_dir) -> Option<CometState>` that scans `openspec/changes/*/.comet.yaml` and returns the first non-archived active change's `phase`, `workflow`, `build_mode`, `isolation`
- [ ] 3.3 Create `src/comet/guard.rs`: `CometGuard::check(phase, tool_name, args) -> CometGuardDecision` implementing the phase tool restriction matrix (open/design: no source writes; verify: limited writes; archive: no writes)
- [ ] 3.4 Integrate `CometGuard::check()` into `ToolExecutor::execute_with_hooks()` at `src/tools/executor.rs:128`, BEFORE PreToolUse hook firing. On block, fire `Notification` hook with subtype `comet_phase_block`
- [ ] 3.5 Add `CometState` reading and phase context injection into agent system messages during prompt assembly in `src/tui/app/mod.rs` or `src/prompts/mod.rs`
- [ ] 3.6 Create `src/comet/workflow.rs`: `active_changes() -> Vec<ChangeInfo>` wrapping `openspec changes/*/.comet.yaml` directory scan

## 4. Worktree isolation — Git operations extension

- [ ] 4.1 Add `worktree_add` operation to `GitOperationsTool::execute()` at `src/tools/execution/git_operations.rs:106`, accepting `path`, `branch`, optional `base_ref` (default `origin/main`). Execute `git worktree add -b <branch> <path> <base_ref>`
- [ ] 4.2 Add `worktree_remove` operation to `GitOperationsTool::execute()`, accepting `path`, optional `force` (boolean). Execute `git worktree remove [--force] <path>`. Without `force`, refuse if worktree has uncommitted changes
- [ ] 4.3 Add `worktree_list` operation to `GitOperationsTool::execute()`. Execute `git worktree list`
- [ ] 4.4 Update `input_schema()` to include `worktree_add`, `worktree_remove`, `worktree_list` in the `operation` enum and document the new parameters (`base_ref`, `force`)
- [ ] 4.5 Ensure all worktree operations run with `current_dir` set to the repository root (use existing `repo_path` logic or resolve to git root from `path`)

## 5. Long command timeout — Configurable per-tool timeout

- [ ] 5.1 Create `resolve_tool_timeout(tool_name: &str, args: &serde_json::Value) -> Duration` in `src/tui/agent/core.rs` with logic: task/delegate → 300s, execute_command/exec_command → max(args.timeout + 30, 120), other → 120
- [ ] 5.2 Replace hardcoded inline ternary at `src/tui/agent/core.rs:322` with call to `resolve_tool_timeout`
- [ ] 5.3 Update `execute_command` `input_schema()` at `src/tools/execution/execute_command.rs:54` to clarify the `timeout` field description: "Timeout in seconds (optional, default: 60, max enforced by agent loop with 30s buffer)"
- [ ] 5.4 Add unit test for `resolve_tool_timeout` covering: execute_command with timeout=600 → 630s, execute_command without timeout → 120s, task → 300s, file_read → 120s

## 6. Subagent orchestrator — Comet context and review flow

- [ ] 6.1 Add optional `comet_context` parameter to `TaskTool` input schema at `src/tools/meta/task.rs:211`, accepting `{ "change": "<name>", "task_index": <n> }`
- [ ] 6.2 When `comet_context` is present, prepend Comet implementer system prompt prefix to the subagent's system prompt, including TDD instructions and change/task context
- [ ] 6.3 Add comet guard check in agent loop at `src/tui/agent/core.rs`: when comet `build_mode` is `subagent-driven-development`, inject a system reminder instructing the coordinator NOT to directly execute source-file writes, only to dispatch subagents
- [ ] 6.4 Ensure `.comet/subagent-progress.md` can be written by coordinator via existing `file_write` tool (no new tool needed — verify path resolves correctly within `openspec/changes/<name>/.comet/`)
- [ ] 6.5 Add Comet subagent dispatch protocol documentation as a section in `src/comet/` or as inline comments documenting the implementer→review×2→fix→commit flow
```

## openspec/changes/comet-workflow-compat/specs/comet-phase-guard/spec.md

- Source: openspec/changes/comet-workflow-compat/specs/comet-phase-guard/spec.md
- Lines: 1-62
- SHA256: 586f5c9b2ddefba338cef4fe34fe8ed2180181467a7797a668f4eb63eeb24d04

```md
## ADDED Requirements

### Requirement: Comet state reader detects active change and phase
The system SHALL read `openspec/changes/<name>/.comet.yaml` for the active change (if any) and extract the `phase` field. If no active change exists, the phase SHALL be `null` (unrestricted).

#### Scenario: Active change in open phase detected
- **WHEN** `openspec/changes/comet-workflow-compat/.comet.yaml` contains `phase: open`
- **THEN** `CometState::read()` SHALL return `phase: open` and `workflow: full`

#### Scenario: No active change, no restrictions
- **WHEN** no `openspec/changes/<name>/.comet.yaml` exists with a non-archived phase
- **THEN** `CometState::read()` SHALL return `phase: null`
- **AND** the comet guard SHALL allow all tool operations

#### Scenario: Multiple active changes
- **WHEN** multiple active changes exist (each with `.comet.yaml`)
- **THEN** comet state SHALL log a warning and use the first one found
- **AND** comet guard SHALL apply the most restrictive phase rules across all active changes

### Requirement: Phase guard blocks tools outside allowed set
The system SHALL, before executing any tool, check the current Comet phase (if active) against a tool allow/deny matrix and block tools not permitted in the current phase.

#### Scenario: file_write blocked in open phase
- **WHEN** comet phase is `open`
- **AND** agent calls `file_write` or `file_edit` targeting a source file
- **THEN** the tool SHALL be blocked with error message indicating phase restriction
- **AND** a `Notification` hook SHALL fire with subtype `comet_phase_block`

#### Scenario: file_read allowed in all phases
- **WHEN** comet phase is `open`, `design`, `build`, `verify`, or `archive`
- **AND** agent calls `file_read`
- **THEN** the tool SHALL execute normally without phase-related blocking

#### Scenario: git commit allowed in build phase
- **WHEN** comet phase is `build`
- **AND** agent calls `git_operations` with `operation: commit`
- **THEN** the tool SHALL execute normally

### Requirement: Phase guard bypass on explicit user approval
The system SHALL allow bypassing the phase guard when the user explicitly approves a blocked tool operation through the permission panel.

#### Scenario: User approves write in open phase
- **WHEN** comet phase is `open` and agent attempts `file_write`
- **AND** the phase guard blocks it and presents the block reason to the user
- **AND** user selects "Allow once" or "Always allow"
- **THEN** the tool SHALL execute

### Requirement: Comet guard integrates at ToolExecutor level
The system SHALL integrate the comet phase guard into `ToolExecutor::execute_with_hooks()`, running the guard check BEFORE `PreToolUse` hooks.

#### Scenario: Guard blocks before PreToolUse hooks
- **WHEN** a tool would be blocked by both comet guard and PreToolUse hook
- **THEN** the comet guard check SHALL run first and block the tool
- **AND** PreToolUse hooks SHALL NOT fire for the blocked tool

### Requirement: Phase context injected into agent system messages
The system SHALL, when an active Comet change exists, append phase context to the agent's system message indicating the current phase, workflow type, and restrictions.

#### Scenario: Agent receives phase context in build phase
- **WHEN** an active change is in `phase: build` with `build_mode: subagent-driven-development`
- **THEN** the system message SHALL include text indicating the current phase and mode
- **AND** the text SHALL reference the relevant Comet skill instructions
```

## openspec/changes/comet-workflow-compat/specs/comet-skill-path-compat/spec.md

- Source: openspec/changes/comet-workflow-compat/specs/comet-skill-path-compat/spec.md
- Lines: 1-35
- SHA256: d6c8e59158a768c2ca254dd52bdf3db1f4ea1dcbfeef033fc68007b7a44713b8

```md
## ADDED Requirements

### Requirement: Runtime external skill registry discovers all Comet-compatible roots
The system SHALL discover external skills from a unified set of root directories: project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`, and user `~/.claude/skills/`. Discovery SHALL be consistent across TUI app startup, daemon tool registry wiring, CLI `skills list`, and TUI completion engine.

#### Scenario: Comet skills installed only in `~/.claude/skills` are discoverable
- **WHEN** Comet skills (comet, comet-open, comet-design, comet-build, comet-verify, comet-archive, comet-hotfix, comet-tweak) are installed in `~/.claude/skills/` but not in `~/.wgenty-code/skills/`
- **THEN** the external skill registry SHALL resolve `/comet` to the Comet skill
- **AND** the `skill` tool SHALL list comet as an available external skill
- **AND** the TUI slash command completion SHALL suggest `/comet` when user types `/com`

#### Scenario: Same skill exists in both roots, wgenty-code wins
- **WHEN** `comet` skill exists in both `~/.wgenty-code/skills/comet/` and `~/.claude/skills/comet/`
- **THEN** the registry SHALL resolve to the `~/.wgenty-code/skills/` version
- **AND** the shadowed `~/.claude/skills/` version SHALL be recorded in the shadowed definitions list

#### Scenario: Skill root does not exist on disk
- **WHEN** `~/.claude/skills/` does not exist on disk
- **THEN** the discovery SHALL silently skip that root without error
- **AND** other roots SHALL still be scanned normally

### Requirement: Unified skill root resolution accessible to all consumers
The system SHALL provide a single `SkillRootResolver` that returns the ordered list of skill roots: project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`, user `~/.claude/skills/`.

#### Scenario: All consumers see the same root list
- **WHEN** TUI app, daemon state, completion engine, and CLI skills list each request skill roots
- **THEN** all four consumers SHALL receive roots in the same order and count
- **AND** no consumer SHALL hardcode its own root list

### Requirement: Startup logging of discovered skills
The system SHALL log, at session startup, the total number of external skills discovered and the root directories scanned.

#### Scenario: Session starts with skills in multiple roots
- **WHEN** a new TUI session starts and skills exist in `~/.wgenty-code/skills/` and `~/.claude/skills/`
- **THEN** a trace-level log SHALL report the count of skills discovered and which roots were scanned
```

## openspec/changes/comet-workflow-compat/specs/comet-subagent-orchestrator/spec.md

- Source: openspec/changes/comet-workflow-compat/specs/comet-subagent-orchestrator/spec.md
- Lines: 1-54
- SHA256: 2e2d3f649878a416b590f63087da579b008bf3ab3b279eda4150471e324d73a2

```md
## ADDED Requirements

### Requirement: Subagent dispatch carries Comet context
When the coordinator (main session) spawns a subagent via the `task` tool and a Comet change is active with `build_mode: subagent-driven-development`, the task tool SHALL accept an optional `comet_context` field containing the change name and current task index from `tasks.md`.

#### Scenario: Implementer subagent receives Comet context
- **WHEN** coordinator calls `task` with `comet_context: { "change": "comet-workflow-compat", "task_index": 3 }`
- **THEN** the subagent's system prompt SHALL include a Comet implementer prefix with TDD instructions
- **AND** the subagent SHALL know it is working on task 3 of the specified change

#### Scenario: Subagent without comet_context works as before
- **WHEN** coordinator calls `task` without `comet_context`
- **THEN** the subagent SHALL behave exactly as current non-Comet task subagents

### Requirement: Coordinator enforces review-before-commit
When operating in Comet subagent-driven-development mode, the coordinator (main session agent) SHALL NOT mark a task as completed or commit until BOTH a spec compliance review and a code quality review have passed for the current task's changes.

#### Scenario: Both reviews pass, task is committed
- **WHEN** implementer subagent completes task 3
- **AND** spec compliance reviewer subagent returns `{ "pass": true }`
- **AND** code quality reviewer subagent returns `{ "pass": true }`
- **THEN** coordinator SHALL call `git_operations commit` with task-specific message
- **AND** coordinator SHALL update `tasks.md` to check off task 3

#### Scenario: One review fails, fix cycle starts
- **WHEN** implementer subagent completes task 3
- **AND** spec compliance reviewer returns `{ "pass": false, "issues": [...] }`
- **THEN** coordinator SHALL spawn a fix subagent with the review issues
- **AND** after fix completes, reviews SHALL re-run (up to 3 fix cycles)
- **AND** if 3 cycles pass without both reviews passing, coordinator SHALL report failure and pause for user decision

### Requirement: Subagent progress is persisted to .comet/subagent-progress.md
The coordinator SHALL write structured progress to `openspec/changes/<name>/.comet/subagent-progress.md` after each subagent stage completes (implement / review / fix / commit).

#### Scenario: Progress file updated after each stage
- **WHEN** implementer subagent completes
- **THEN** `.comet/subagent-progress.md` SHALL be appended with the implementer result
- **AND** after reviewer completes, it SHALL be appended again
- **AND** the file SHALL use a format compatible with Comet context recovery

#### Scenario: Progress file enables recovery after interruption
- **WHEN** the session is interrupted mid-task
- **AND** `.comet/subagent-progress.md` exists with the last completed stage
- **THEN** on resume, coordinator SHALL read the progress file
- **AND** coordinator SHALL resume from the next incomplete stage (not restart the task)

### Requirement: Main session does not directly execute build tasks
When a Comet change is in `build_mode: subagent-driven-development`, the main session (coordinator) SHALL NOT directly execute pending tasks from `tasks.md`. All task execution SHALL be delegated to subagents via the `task` tool.

#### Scenario: Coordinator refuses to directly implement a task
- **WHEN** Comet mode is `subagent-driven-development`
- **AND** agent attempts to `file_write` or `file_edit` source code for a pending task directly
- **THEN** the comet guard SHALL flag this as a mode violation
- **AND** a warning SHALL be added to the conversation context reminding the coordinator to use subagents
```

## openspec/changes/comet-workflow-compat/specs/hook-lifecycle-complete/spec.md

- Source: openspec/changes/comet-workflow-compat/specs/hook-lifecycle-complete/spec.md
- Lines: 1-61
- SHA256: 8429f880f03f8b8151d623a161e7b0162c4166f4a54f8783af7b83643940c2a6

```md
## ADDED Requirements

### Requirement: SessionStart hook fires on session creation
The system SHALL fire the `SessionStart` hook event immediately after a new TUI app session is fully initialized. The hook context SHALL carry the session ID and working directory.

#### Scenario: SessionStart hook blocks on startup error
- **WHEN** a `SessionStart` hook is configured that returns `{ "continue_execution": false }`
- **THEN** the hook outcome SHALL be logged as blocked
- **AND** the session SHALL still proceed (SessionStart hook does not prevent session creation)

#### Scenario: SessionStart context contains session info
- **WHEN** a `SessionStart` hook fires
- **THEN** the hook context JSON SHALL include `session_id`, `working_directory`, and `timestamp`

### Requirement: SessionEnd hook fires on session exit
The system SHALL fire the `SessionEnd` hook event before the daemon is shut down and the terminal is restored. The hook context SHALL carry the session ID.

#### Scenario: SessionEnd fires before daemon shutdown
- **WHEN** the user quits the TUI (double Ctrl+C or equivalent)
- **THEN** the `SessionEnd` hook SHALL fire before the daemon shutdown signal is sent
- **AND** hook execution SHALL complete (or timeout) before the process exits

### Requirement: UserPromptSubmit hook fires on every input submission
The system SHALL fire the `UserPromptSubmit` hook event when the user submits input in the TUI, AFTER built-in slash commands are handled but BEFORE the input is routed as a slash command or queued for the agent.

#### Scenario: Comet guard hook reads input before agent
- **WHEN** user submits `/comet add login feature` in the TUI input
- **THEN** the `UserPromptSubmit` hook SHALL fire with `tool_input` containing the raw input text
- **AND** hook execution SHALL complete before the input reaches `route_slash_command`

#### Scenario: UserPromptSubmit does not fire on built-in commands
- **WHEN** user submits `/clear` or `/help` (built-in commands)
- **THEN** the `UserPromptSubmit` hook SHALL NOT fire (built-ins are handled before hook)

### Requirement: Stop hook fires on turn completion or abort
The system SHALL fire the `Stop` hook event when an agent turn completes normally (`TurnComplete`) or is aborted (`TurnAborted`).

#### Scenario: Stop hook fires on normal turn completion
- **WHEN** an agent turn finishes without error and `TurnComplete` event is emitted
- **THEN** the `Stop` hook SHALL fire with the turn finish reason "stop"

#### Scenario: Stop hook fires on turn abort
- **WHEN** an agent turn is cancelled or exceeds max rounds and `TurnAborted` event is emitted
- **THEN** the `Stop` hook SHALL fire with the abort reason in the hook context

### Requirement: PermissionRequest hook fires before user permission prompt
The system SHALL fire the `PermissionRequest` hook event when a tool execution requires user permission (via `PermissionRequired` event), BEFORE the permission prompt is shown to the user.

#### Scenario: Hook can auto-deny permission without showing prompt
- **WHEN** a `PermissionRequest` hook is configured that returns `{ "continue_execution": false, "reason": "auto-denied by policy" }`
- **THEN** the permission prompt SHALL NOT be shown to the user
- **AND** the tool execution SHALL be denied with the hook's reason

### Requirement: Notification hook fires on comet guard phase events
The system SHALL fire the `Notification` hook event when comet phase guard detects a phase-related event (e.g., tool blocked by phase restriction, phase transition detected).

#### Scenario: Notification on phase-restricted tool attempt
- **WHEN** the agent attempts `file_write` during open/design phase
- **AND** comet phase guard blocks it
- **THEN** a `Notification` hook SHALL fire with subtype `comet_phase_block`
- **AND** the notification context SHALL include the blocked tool name, current phase, and reason
```

## openspec/changes/comet-workflow-compat/specs/long-command-timeout-config/spec.md

- Source: openspec/changes/comet-workflow-compat/specs/long-command-timeout-config/spec.md
- Lines: 1-37
- SHA256: 19a30ce30a180cc88bfd8149eb0621412c1c7ab69c9ae472fa98a97836f4e992

```md
## ADDED Requirements

### Requirement: execute_command respects user-specified timeout
The `execute_command` tool SHALL respect the `timeout` field in its input schema. The outer agent loop SHALL NOT impose a hard 120s timeout that overrides a user-specified longer timeout.

#### Scenario: execute_command with 600s timeout runs to completion
- **WHEN** agent calls `execute_command` with `{ "command": "cargo test --all", "timeout": 600 }`
- **THEN** the outer agent loop timeout SHALL be at least `max(args.timeout + 30, 120)` = 630s
- **AND** the command SHALL be allowed to run for up to 600 seconds without timeout

#### Scenario: execute_command without explicit timeout uses default
- **WHEN** agent calls `execute_command` with `{ "command": "echo hello" }` (no timeout field)
- **THEN** the outer agent loop timeout SHALL default to 120s
- **AND** the sandbox/execution layer SHALL use its own default (60s)

### Requirement: Task and delegate tools retain independent timeout
The `task` and `delegate` tools SHALL continue to have their own timeout (300s). This timeout SHALL NOT be affected by the execute_command timeout logic.

#### Scenario: task tool has 300s timeout
- **WHEN** agent calls the `task` tool to spawn a subagent
- **THEN** the outer agent loop timeout SHALL be 300s
- **AND** this SHALL NOT be overridden by any execute_command timeout configuration

### Requirement: Timeout resolution is centralized
The system SHALL provide a single function `resolve_tool_timeout(tool_name, args)` used by the agent loop to determine per-tool timeout. This SHALL replace the current inline ternary expression.

#### Scenario: Centralized timeout logic used
- **WHEN** any tool is executed in the agent loop
- **THEN** `resolve_tool_timeout` SHALL be called to determine the timeout
- **AND** the timeout logic SHALL NOT be duplicated inline

### Requirement: Timeout documentation in input schema
The `execute_command` tool's `input_schema` SHALL document the `timeout` field clearly, stating it is in seconds and the default is 60s.

#### Scenario: Schema describes timeout field
- **WHEN** the `execute_command` tool definition is retrieved
- **THEN** the `timeout` field description SHALL state "Timeout in seconds (optional, default: 60)"
```

## openspec/changes/comet-workflow-compat/specs/worktree-isolation-tool/spec.md

- Source: openspec/changes/comet-workflow-compat/specs/worktree-isolation-tool/spec.md
- Lines: 1-44
- SHA256: 13cee44fef9ed52bcebc3b6399db0479b5051ad9ab9181c53b3f27112ab1d5ec

```md
## ADDED Requirements

### Requirement: Git operations supports worktree_add
The `git_operations` tool SHALL support `operation: "worktree_add"` to create a new git worktree. Required parameters: `path` (relative or absolute path under `.claude/worktrees/` or `.wgenty-code/worktrees/`), `branch` (new branch name). Optional: `base_ref` (default: `origin/main`).

#### Scenario: Create worktree with new branch from origin/main
- **WHEN** agent calls `git_operations` with `operation: "worktree_add"`, `path: ".claude/worktrees/feature-x"`, `branch: "feature/20260622/feature-x"`, `base_ref: "origin/main"`
- **THEN** `git worktree add -b feature/20260622/feature-x .claude/worktrees/feature-x origin/main` SHALL be executed
- **AND** the result SHALL include the new worktree path
- **AND** the output SHALL be the git command stdout

#### Scenario: Create worktree fails when branch already exists
- **WHEN** `base_ref` branch already has a worktree at the given path or the branch name is taken
- **THEN** the tool SHALL return a `non_zero_exit` error with the git error message

### Requirement: Git operations supports worktree_remove
The `git_operations` tool SHALL support `operation: "worktree_remove"` to remove a git worktree. Required parameter: `path`. Optional: `force` (boolean, default: `false`).

#### Scenario: Remove worktree with force when clean
- **WHEN** agent calls `git_operations` with `operation: "worktree_remove"`, `path: ".claude/worktrees/feature-x"`, `force: true`
- **THEN** `git worktree remove --force .claude/worktrees/feature-x` SHALL be executed
- **AND** the worktree directory and its branch SHALL be removed

#### Scenario: Remove with uncommitted changes without force fails
- **WHEN** the worktree has uncommitted changes or commits not on the original branch
- **AND** `force` is `false`
- **THEN** the tool SHALL return a failure indicating uncommitted changes exist
- **AND** the error SHALL suggest using `force: true` to discard changes

### Requirement: Git operations supports worktree_list
The `git_operations` tool SHALL support `operation: "worktree_list"` to list all git worktrees for the repository.

#### Scenario: List all worktrees
- **WHEN** agent calls `git_operations` with `operation: "worktree_list"`
- **THEN** `git worktree list` SHALL be executed
- **AND** the output SHALL contain one line per worktree with path, HEAD, and branch info

### Requirement: Worktree operations run in the repository root
All worktree operations SHALL execute with `current_dir` set to the repository root (where `.git` lives), regardless of the `path` argument in the tool call.

#### Scenario: Worktree command runs from repo root
- **WHEN** agent's working directory is a subdirectory
- **AND** agent calls `git_operations` with `operation: "worktree_add"`
- **THEN** the git command SHALL execute with `current_dir` set to the repository root
```

