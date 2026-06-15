## Why

当前多 Agent（RLM）系统存在四个工程短板：subagent 执行过程对用户不可见（只能看到树状进度条 + 200 字符截断）、错误发生时缺乏完整因果链和恢复机制、递归调用缺少跨层级进展跟踪和 per-subagent 预算控制、以及多条 subagent 结果聚合缺乏结构化冲突检测。同时，用户无法在 TUI 输入框中直接操作 skills/plugin 命令，必须记住 `/` 前缀语法。这些问题导致调试困难、成本不可控、用户体验割裂。

## What Changes

### TUI 交互增强
- TUI 输入框集成 skills 和 plugin 命令的自动补全、内联提示和交互式选择
- 支持 `@skill-name` 语法触发 skill 补全，`/plugin-command` 语法触发插件命令补全

### Subagent 执行透明化
- 扩展 `SubagentProgress` 事件模型，记录完整 thinking/action 时间线（不再截断到 50 条 / 200 字符）
- 新增 subagent transcript 持久化（SQLite），支持历史查询和回放
- TUI subagent panel 增强：显示每节点的完整推理步骤、工具调用参数和中间结果

### Subagent 错误可视化与恢复
- 错误发生时展示完整因果链：哪个节点 → 什么操作 → 为什么失败
- 支持从 subagent panel 一键回滚到父节点状态并重试
- Failed/Cancelled 节点可展开查看错误详情和 metadata（token 消耗、耗时）

### RLM 递归防护强化
- **跨层级进展跟踪**：每层递归携带进展增量指标，连续两轮增量 < 阈值 → 强制终止
- **Per-subagent token 预算**：父 agent 调用 subagent 时声明 token 上限，超支立即 kill
- **结构化归约输出**：subagent 代码变更输出 unified diff + 变更意图；分析结论输出 structured claims `{claim, evidence, confidence, conflicts_with}`；Aggregator 按 claim 去重、按 confidence 排序、检测 conflicts_with 冲突

## Capabilities

### New Capabilities
- `tui-command-completion`: TUI 输入框中 skills（`@name`）和 plugin 命令（`/cmd`）的自动补全和交互式选择
- `subagent-transcript-storage`: Subagent 完整执行记录的 SQLite 持久化存储，支持历史查询和回放
- `rlm-structured-reduction`: RLM pipeline 的结构化归约输出（unified diff / structured claims），Aggregator 按结构化数据合并而非自然语言理解
- `rlm-budget-control`: Per-subagent token 预算分配与硬性熔断，父 agent 调用时声明上限

### Modified Capabilities
- `subagent-action-visibility`: 要求不再截断 action log（50 条）和 text snapshot（200 字符），改为完整时间线记录
- `subagent-content-preview`: 要求接入 SQLite transcript 存储，支持通过 subagent panel 查看历史子任务完整内容
- `subagent-status-display`: 要求展示错误详情（完整因果链）、支持一键回滚/重试操作
- `task-complexity-detection`: 要求复杂任务检测结果影响结构化归约格式选择（分析型 → claims，修改型 → diff）

## Impact

### 涉及模块
- `src/tui/` — 输入框补全、subagent panel 增强
- `src/tools/meta/task.rs` — subagent 调度（预算传递、结构化输出要求）
- `src/tools/meta/rlm/` — RLM pipeline（结构化归约、进展跟踪）
- `src/teams/subagent_loop.rs` — 子 agent 循环（transcript 记录、预算检查）
- `src/agent/progress.rs` — 进度事件模型扩展
- `src/plugins/commands.rs` — 命令注册与补全数据源
- `packages/cli/src/components/input-box.tsx` — Ink CLI 输入框补全
- `src/config/settings.rs` — 新增配置字段（transcript 开关、预算默认值）

### 不涉及
- `src/web/` — daemon HTTP API
- `src/wasm/` — 浏览器端
- `src/api/` — LLM API 协议层
- 外部基础设施依赖（无需 OpenTelemetry collector 等）
