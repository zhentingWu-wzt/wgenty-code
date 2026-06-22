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
- 不改变模型 provider 行为
- 不做无关 UI 重构
- 不实现 OpenSpec 文件格式修改
- 不在本 change 中将 Comet 内建为 Rust 原生实现（仍然通过外部 scripts + skill 指令）
