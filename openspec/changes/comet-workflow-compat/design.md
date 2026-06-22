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

| Phase | 允许 | 禁止 |
|---|---|---|
| `open` | file_read, grep, glob, web_search, web_fetch, ask_user_question, skill, Bash(只读), git(status/log/diff) | file_write, file_edit, apply_patch, execute_command(写) |
| `design` | open 的允许 + skill, brainstorming 相关 | 同 open（禁止写源码） |
| `build` | 全部工具 | 无 |
| `verify` | file_read, execute_command(test), git(log/diff/status), 无写文件 | file_write, file_edit, apply_patch（除非修复验证失败） |
| `archive` | file_read, execute_command(comet-archive), git | file_write, file_edit, apply_patch |

**Comet 配置文件位置**: `src/comet/` 下新建模块：
- `src/comet/mod.rs` — re-export
- `src/comet/state.rs` — 读 `.comet.yaml`
- `src/comet/guard.rs` — phase 工具限制矩阵
- `src/comet/workflow.rs` — active change discovery

**替代方案**:
- 完全通过 hook 脚本实现 → 可行但每轮都要 fork 进程读 `.comet.yaml`，性能差且易出错
- 在 agent loop 中内建 → 耦合太重，不如在 ToolExecutor 层统一

### D4: Worktree 工具设计

**选择**: 扩展现有 `git_operations` tool，新增 `worktree_add`、`worktree_remove`、`worktree_list` 操作。

**Schema 扩展**:
```json
{
  "operation": "worktree_add",
  "path": ".claude/worktrees/feature-x",
  "branch": "feature/20260622/feature-x",
  "base_ref": "origin/main"
}
```

```json
{
  "operation": "worktree_remove",
  "path": ".claude/worktrees/feature-x",
  "force": true
}
```

```json
{
  "operation": "worktree_list"
}
```

**替代方案**:
- 新增独立 `worktree` tool → 语义上更清晰，但会增加 tool 数量。git_operations 已涵盖 git 子命令，worktree 是 git 子命令，放一起更自然。

### D5: 长命令 timeout 策略

**选择**: 两层改动：

1. `execute_command` 的 `input_schema` 中 `timeout` 字段已是 schema 的一部分（当前默认 60s），确保这个值被外层 `agent/core.rs` 的硬编码 120s 尊重。
2. 修改 `agent/core.rs:322` 的外层 timeout 逻辑：当 tool name 是 `execute_command` 时，从 args 中读取 `timeout` 字段 + 30s buffer，而不是硬编码 120s。

```rust
// 当前:
let tool_timeout = if tc.function.name == "task" {
    Duration::from_secs(300)
} else {
    Duration::from_secs(120)  // 硬编码
};

// 改后:
let tool_timeout = resolve_tool_timeout(&tc.function.name, &args);
```

`resolve_tool_timeout` 逻辑：
- `task` / `delegate` → 300s
- `execute_command` / `exec_command` → max(args.timeout + 30, 120)
- 其他 → 120s

**替代方案**:
- 完全移除硬编码 timeout → 有风险，某些工具可能永久挂起
- 通过 settings.json 全局配置 → 太重，不同命令需要不同 timeout

### D6: Comet subagent orchestrator

**选择**: 不新增独立 Rust 服务，而是通过以下方式支持：

1. 主会话通过 system prompt / comet skill 指令获知 `build_mode: subagent-driven-development`
2. Task tool 的 subagent dispatch 在创建 implementer subagent 时自动附加 Comet 专属 system prompt 前缀（含 TDD、reviewer 协议）
3. Subagent 完成后，coordinator（主会话）spawn spec compliance reviewer 和 code quality reviewer 作为并行 subagent
4. 审查不通过时 spawn fix agent（最多 3 轮），通过后 coordinator 调用 `git_operations` commit + `TodoWrite` 勾选
5. 进度写入 `.comet/subagent-progress.md`（由 coordinator 通过 file_write 写入）

**Rust 层改动**（轻量）:
- Task tool 支持 `comet_context` 可选参数，携带 change name + task index，自动附加 Comet system prompt
- `.comet/subagent-progress.md` 的读写通过现有 file_read/file_write

**替代方案**:
- 内建完整的 Comet orchestrator service → 本次不做，过于重量且耦合 Comet 协议版本

## Risks / Trade-offs

| Risk | Mitigation |
|---|---|
| `~/.claude/skills` 扫描可能覆盖 Claude Code 非 Comet skills，产生 shadow 冲突 | 按 ExternalSkillSource 优先级排序，`~/.wgenty-code/skills` 优先级高于 `~/.claude/skills` |
| Hook fire 增加可能拖慢每轮/每次输入的响应 | 所有 hook 异步执行，不阻塞主 loop；timeout 独立配置 |
| Comet phase guard 误拦截合法操作 | phase 限制矩阵保守设计：不确定时 allow + warn log |
| Worktree 操作错误可能损坏 git 仓库 | 操作前检查 git status clean；remove 前二次确认 |
| 长 timeout 允许后 仍可能被 sandbox/系统限制截断 | 不做此层兜底，sandbox timeout 由 sandbox profile 独立处理 |

## Open Questions

1. **skill root 优先级**: 当 `~/.wgenty-code/skills/comet` 和 `~/.claude/skills/comet` 同时存在时，选哪个？当前设计是 wgenty-code 优先，需要确认这是否符合用户预期。
2. **verify phase 修复例外**: verify 阶段 guard 应该完全禁止源码写入，还是允许"修复验证发现的问题"？当前设计是"禁止除非用户明确 approve"。
3. **worktree path convention**: 是否与 Claude Code 的 `.claude/worktrees/` 保持一致？还是用 `.wgenty-code/worktrees/`？
4. **subagent progress 文件同步**: `.comet/subagent-progress.md` 由主会话写，subagent 只读。并发安全性依赖 Rust 文件锁，需确认是否需要更严格的序列化。
