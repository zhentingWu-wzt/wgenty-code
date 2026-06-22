# Brainstorm Summary

- Change: comet-workflow-compat
- Date: 2026-06-22

## 确认的技术方案

### 1. Comet phase guard 集成方式：ToolExecutor 内建
- 在 `ToolExecutor::execute_with_hooks()` 中直接调用 `CometGuard::check()`，读 `.comet.yaml` 的 `phase` 做硬拦截
- phase 限制矩阵写在 `src/comet/guard.rs`
- PreToolUse hook 仍然可以 fire 附加检查（如额外 hook 脚本），但不替代硬 guard

### 2. Verify 阶段修复例外：禁止但可 approve
- verify 阶段默认禁止 file_write/file_edit/apply_patch
- agent 可通过 ask_user_question 描述发现的问题，用户 approve 后逐一放行写入
- 不放行时，问题必须回到 build 阶段修复

### 3. Worktree 路径约定：`.wgenty-code/worktrees/`
- 使用 Wgenty Code 自己的命名空间
- 不与 Claude Code `.claude/worktrees/` 混用
- `git_operations` 的 worktree_add 默认 `path` 参数相对于 repo root

### 4. Subagent progress 并发：主会话独占写
- 只有 coordinator（主会话 agent）通过 file_write 写入 `.comet/subagent-progress.md`
- subagent 不直接写 progress 文件
- coordinator 串行处理每个 task 的 implement → review ×2 → fix → commit，自然避免并发写

### 5. Skill root 优先级（继承 design.md D1）
- 项目 `.wgenty-code/skills/` > 用户 `~/.wgenty-code/skills/` > 用户 `~/.claude/skills/`
- 统一通过 `SkillRootResolver` 获取

### 6. 长命令 timeout（继承 design.md D5）
- `resolve_tool_timeout` 函数：task/delegate → 300s，execute_command → max(args.timeout + 30, 120)，其他 → 120
- 替代 agent/core.rs 的硬编码 120s

## 关键取舍与风险

- **Guard 内建 vs 纯 hook**: 选择内建，牺牲"脚本可热更新 phase 矩阵"的灵活性，换取性能和可靠性
- **Verify 修复通道**: 通过 ask_user_question 放行，由用户判断而非 agent 自动决定，引入人机交互延迟但保持阶段隔离
- **Worktree 路径独立**: 不与 Claude Code 共用 `.claude/worktrees/`，避免两个工具的工作区冲突，但意味着两边 worktree 不互通
- **主会话串行写 progress**: 简化并发模型，但长链 task 时 coordinator 可能成为瓶颈

## 测试策略

- **Guard 单元测试**: `CometState::read()` 解析 `.comet.yaml`；`CometGuard::check()` 按 phase 矩阵返回正确决定
- **Hook 集成测试**: 每个新增 hook fire site 验证 hook 实际触发且 context 正确
- **Worktree 集成测试**: git worktree add/remove/list 完整流程
- **Timeout 单元测试**: `resolve_tool_timeout` 覆盖所有 tool name + timeout 组合
- **Subagent orchestrator**: 端到端测试 implement→review→fix→commit 流程

## Spec Patch

无（delta spec 在 open 阶段已完整创建，未发现缺少验收场景或歧义描述）
