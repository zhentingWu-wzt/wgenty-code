# Comet Design Handoff

- Change: rlm-observability-and-robustness
- Phase: design
- Mode: compact
- Context hash: 1120a7c5dd3b789a9fca2df662fb3f886a5b684c09b9a891c976b8a34ac48282

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/rlm-observability-and-robustness/proposal.md

- Source: openspec/changes/rlm-observability-and-robustness/proposal.md
- Lines: 1-56
- SHA256: 0c5d9b5a8347d0676c37cc0abb76f38c1edfdd1efb328b5b44d9e9b610be9cd3

```md
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
```

## openspec/changes/rlm-observability-and-robustness/design.md

- Source: openspec/changes/rlm-observability-and-robustness/design.md
- Lines: 1-199
- SHA256: d47370af3979ba63dee09b94276767c19dc4425b683c81be9e4e581634046df3

[TRUNCATED]

```md
## Context

当前 wgenty-code 的多 Agent 系统由三层组成：
1. **主 Agent Loop**（`src/tui/agent/`）：处理用户输入，调用 LLM，分发工具执行
2. **Task/Delegate 工具**（`src/tools/meta/`）：子 agent 调度入口，支持直接 subagent 和 RLM pipeline
3. **Subagent Loop**（`src/teams/subagent_loop.rs`）：隔离的 agent 循环，独立上下文

TUI 层通过 `SubagentTree`（内存）→ `SubagentPanel`（渲染）展示执行进度。`SubagentProgress` 事件通过共享 `HashMap<session_id, HashMap<node_id, progress>>` 传递，action log 截断到 50 条，text snapshot 截断到 200 字符。

现有护栏：`max_subagent_depth=3`、`max_concurrent_subagents=5`、`subagent_timeout_secs=240`、`StuckDetector`（3 次重复 → abort）。

### 约束
- 运行在终端环境（Ratatui TUI），无浏览器 DOM 能力
- LLM API 通过 HTTP SSE 调用，不改变协议层
- Subagent 结果通过 mailbox 机制 offload 大结果到磁盘
- 配置热加载（`ConfigChanged` 事件）

## Goals / Non-Goals

**Goals:**
1. TUI 输入框中实现 skills（`@name`）和 plugin 命令（`/cmd`）的自动补全
2. Subagent 执行完整时间线记录 + SQLite 持久化
3. Subagent 错误完整因果链展示 + 一键回滚重试
4. RLM pipeline 结构化归约输出（claims / diff）+ 跨层级进展跟踪 + per-subagent 预算

**Non-Goals:**
- 不改变 LLM API 协议
- 不引入 OpenTelemetry 等外部基础设施
- 不重写主 agent loop
- 不修改 daemon HTTP API 和 WASM 端
- 不改变 subagent 的最大并发数或深度限制的默认值

## Decisions

### D1: TUI 输入框补全架构

**选择**：在 `src/tui/input_reader.rs` 增加补全模式，由 `App` 状态机管理

```
┌─────────────────────────────────────────────────┐
│  Input Box                                       │
│  ▸ @skill-name █                                 │
│    ┌────────────┐                                │
│    │ brainstorming │  ← 弹出补全面板             │
│    │ comet-open    │                              │
│    │ tdd           │                              │
│    └────────────┘                                │
└─────────────────────────────────────────────────┘
```

- `@` 触发 skill 补全：读取 `~/.claude/skills/` 目录下所有 skill 名称
- `/` 触发 plugin command 补全：从 `PluginRegistry.commands` 获取已注册命令
- 补全面板复用现有 `PermissionState` 的 inline panel 模式（不弹窗）
- Tab / Shift+Tab 循环候选项，Enter 确认，Esc 取消

**备选方案**：使用 popup overlay → 拒绝，与现有 permission/question UI 风格不一致，且 popup 需要额外 z-order 管理

### D2: Subagent Transcript 持久化

**选择**：SQLite 数据库，每个 subagent 执行记录为一行

```sql
CREATE TABLE subagent_transcripts (
    id TEXT PRIMARY KEY,           -- UUID
    session_id TEXT NOT NULL,       -- 所属会话
    parent_id TEXT,                 -- 父节点 ID (NULL = root)
    label TEXT NOT NULL,            -- 人类可读标签
    status TEXT NOT NULL,           -- pending/running/completed/failed/cancelled
    system_prompt TEXT,
    user_prompt TEXT,
    started_at INTEGER NOT NULL,    -- unix ms
    finished_at INTEGER,
    total_tokens INTEGER DEFAULT 0,
    error_message TEXT,
    summary TEXT,                   -- 最终结果摘要 (truncated)
    created_at INTEGER DEFAULT (strftime('%s','now'))
);

CREATE TABLE subagent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
```

Full source: openspec/changes/rlm-observability-and-robustness/design.md

## openspec/changes/rlm-observability-and-robustness/tasks.md

- Source: openspec/changes/rlm-observability-and-robustness/tasks.md
- Lines: 1-69
- SHA256: caa903b6a506fc37529cb6cd2043e89fefba57e2606289825c34bf22cb9e1848

```md
## 1. TUI 输入框命令补全

- [ ] 1.1 在 `src/tui/` 下创建 `CompletionEngine`，启动时扫描 `~/.claude/skills/` 目录和 `PluginRegistry` 加载候选项
- [ ] 1.2 在 `src/tui/input_reader.rs` 增加补全触发逻辑：检测 `@` 和 `/` 前缀，发送 `AppEvent::CompletionTrigger` 事件
- [ ] 1.3 在 `src/tui/app/event.rs` 增加补全相关事件类型：`CompletionTrigger`、`CompletionSelect`、`CompletionDismiss`
- [ ] 1.4 在 `src/tui/components/` 创建 `completion_panel.rs`，实现内联补全面板（复用 PermissionState 的 inline panel 模式）
- [ ] 1.5 在 `src/tui/app/` 增加补全状态管理：过滤、导航（Up/Down/Tab/Enter/Esc）、选中替换输入框内容
- [ ] 1.6 在 `packages/cli/src/components/input-box.tsx` 增加 Ink CLI 侧的 `@`/`/` 补全触发支持

## 2. Subagent Transcript 持久化

- [ ] 2.1 在 `src/` 下创建 `transcript/` 模块（`mod.rs` + `store.rs`），实现 `SubagentTranscriptStore`（SQLite CRUD）
- [ ] 2.2 定义数据库 schema（`subagent_transcripts` + `subagent_events` 表），实现 auto-migration
- [ ] 2.3 在 `SubagentTranscriptStore` 实现 `list_by_session`、`get_by_id`、`search` 查询方法
- [ ] 2.4 在 `src/config/settings.rs` 增加 `max_transcript_age_days: u32` 配置字段（默认 30）
- [ ] 2.5 在 `run_subagent_loop()` 完成/失败时调用 `TranscriptStore::save()` 批量写入所有 events
- [ ] 2.6 在 `TranscriptStore::save()` 中实现保留策略：删除超过 `max_transcript_age_days` 的旧记录

## 3. Subagent 执行时间线完整记录

- [ ] 3.1 扩展 `SubagentEvent` 枚举，增加 `ToolResult` 和 `Error` 事件类型
- [ ] 3.2 修改 `run_subagent_loop()`：移除 action_log 的 50 条截断限制，保留完整事件列表直到 subagent 完成
- [ ] 3.3 修改 `run_subagent_loop()`：移除 text_snapshot 的 200 字符截断（改为完整文本存储，TUI 层截断显示）
- [ ] 3.4 修改 `SubagentProgress` 结构：增加 `progress_delta: Option<f32>` 字段

## 4. Subagent 错误可视化与恢复

- [ ] 4.1 在 `subagent_panel.rs` 增加 Failed 节点的错误详情展示（红色高亮 + 错误消息 + `[r] retry  [d] details` 提示）
- [ ] 4.2 在 `subagent_panel_state.rs` 增加 detail view 切换状态和快捷键处理（Enter → 全屏 detail，d → detail，r → retry）
- [ ] 4.3 创建 `SubagentDetailView` 组件：从 SQLite 读取 transcript 并以分页方式渲染完整事件时间线
- [ ] 4.4 在 `tool_dispatch.rs` 或 `task.rs` 实现重试逻辑：读取失败 subagent 的 prompt → 注入 `previous_attempt_error` → 重新 spawn
- [ ] 4.5 实现回滚机制：subagent 修改文件前创建 git stash，重试时 revert 到父节点状态

## 5. RLM 结构化归约

- [ ] 5.1 在 `src/tools/meta/rlm/` 下创建 `formats.rs`：定义 `StructuredClaims` 和 `UnifiedDiff` 的 Rust struct（含 serde 序列化）
- [ ] 5.2 修改 RLM planner prompt：根据任务类型（analysis/modification/mixed）在 sub-task 描述中注入输出格式指令
- [ ] 5.3 修改 `run_subagent_loop()` 的 system prompt：当父任务要求结构化输出时，追加格式规范指令
- [ ] 5.4 实现 Aggregator 结构化合并逻辑：Jaccard 相似度去重（阈值 0.8）、conflicts_with 冲突检测、同文件 diff 冲突标记
- [ ] 5.5 Aggregator 在结构化合并后，仅对无法 resolve 的冲突项 fallback 到 LLM merge

## 6. RLM 预算控制与进展跟踪

- [ ] 6.1 在 `task` 工具的 `input_schema` 增加 `token_budget: Option<u64>` 字段
- [ ] 6.2 在 `src/config/settings.rs` 增加 `default_subagent_token_budget_k: usize` 配置（默认 0 = 不限）
- [ ] 6.3 在 `run_subagent_loop()` 每轮 API 调用后累加 `cumulative_tokens`，超限立即返回 `Err("Token budget exceeded")`
- [ ] 6.4 实现 RLM pipeline 预算分配逻辑：planner 10% + sub-tasks 80% + aggregator 10%，未用完预算滚动到下阶段
- [ ] 6.5 在 `run_subagent_loop()` 实现 progress_delta 计算：每轮比较新发现数 vs 总发现数，连续 3 轮 delta < 0.05 → `StuckStatus::NoProgress` abort

## 7. TUI 集成与渲染

- [ ] 7.1 更新 `status.rs`（status bar 组件）：显示 subagent 失败计数（红色）、token 预算使用情况
- [ ] 7.2 更新 `subagent_tree.rs`：存储并暴露 progress_delta、budget、error_details 字段
- [ ] 7.3 更新 `subagent_panel.rs`：渲染 token 预算信息（"1.5k/10k tokens"）、progress_delta 低警告、完整 action timeline
- [ ] 7.4 更新 `render.rs`：集成补全面板渲染、detail view 全屏模式

## 8. 配置与 CLI 入口

- [ ] 8.1 在 `settings.rs` 的 `set()` 方法增加新配置项的 setter
- [ ] 8.2 在 Ink CLI (`packages/cli/`) 侧更新 `use-agent.ts` 的 `AgentStatus` 类型以支持新的事件状态
- [ ] 8.3 确保配置热加载（`ConfigChanged` 事件）能正确传播新字段到运行中的 agent

## 9. 验证与测试

- [ ] 9.1 运行现有测试套件（`cargo test`），确保无回归
- [ ] 9.2 手动验证：TUI 输入框 `@` 触发 skills 补全 → 选择 skill → 提交
- [ ] 9.3 手动验证：spawn subagent → 查看 subagent panel 完整时间线 → 查看 transcript detail view
- [ ] 9.4 手动验证：强制 subagent 失败（timeout/budget exceeded）→ 查看错误详情 → 重试
- [ ] 9.5 手动验证：RLM pipeline 中两个 subagent 产出冲突 claims → Aggregator 正确标记冲突
```

## openspec/changes/rlm-observability-and-robustness/specs/rlm-budget-control/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/rlm-budget-control/spec.md
- Lines: 1-53
- SHA256: 629002720970c44269937f1e9a958f6298dafb9e67216e969bacedf0617b633c

```md
# rlm-budget-control Specification

## Purpose
Enable per-subagent token budget allocation with hard cutoffs to prevent runaway cost in recursive multi-agent execution.

## ADDED Requirements

### Requirement: Token budget parameter on task tool
The `task` tool input schema SHALL include an optional `token_budget` parameter (in thousands of tokens) that limits the subagent's total token consumption.

#### Scenario: Budget specified and enforced
- **WHEN** a subagent is spawned with `token_budget = 10` (10k tokens)
- **AND** the subagent's cumulative token usage reaches 10,000 tokens
- **THEN** the subagent loop SHALL immediately stop and return an error: "Token budget exceeded (limit: 10k, used: 10k)"

#### Scenario: No budget specified
- **WHEN** a subagent is spawned without a `token_budget` parameter (or `token_budget = 0`)
- **THEN** token consumption SHALL NOT be limited by budget (other limits like max_rounds and timeout still apply)

#### Scenario: Budget check on each API round
- **WHEN** a subagent completes an API round
- **THEN** the cumulative token count SHALL be checked against the budget before executing any tool calls from that round

### Requirement: Default budget configuration
The system SHALL support a global default token budget via `default_subagent_token_budget_k` in Settings.

#### Scenario: Default budget used when not explicitly set
- **WHEN** a subagent is spawned without an explicit `token_budget`
- **AND** `settings.default_subagent_token_budget_k` is set to 50
- **THEN** the subagent SHALL be subject to a 50k token budget

#### Scenario: Explicit budget overrides default
- **WHEN** a subagent is spawned with `token_budget = 20`
- **AND** `settings.default_subagent_token_budget_k` is set to 50
- **THEN** the 20k budget SHALL be enforced, overriding the default

### Requirement: RLM pipeline budget distribution
When the RLM pipeline is used, the total budget SHALL be distributed across planner, executor, and aggregator phases.

#### Scenario: Budget distributed across pipeline phases
- **WHEN** a delegate task is called with `token_budget = 100`
- **THEN** the planner SHALL receive up to 10% (10k), sub-tasks SHALL evenly split 80% (80k / N), and the aggregator SHALL receive up to 10% (10k)

#### Scenario: Unused budget rolls forward
- **WHEN** the planner phase uses only 5k of its 10k allocation
- **THEN** the unused 5k SHALL be added to the sub-task pool

### Requirement: Budget exhaustion is reported
When a subagent is killed due to budget exhaustion, the error SHALL include actionable diagnostics.

#### Scenario: Budget exhaustion error details
- **WHEN** a subagent exceeds its token budget
- **THEN** the error message SHALL include: limit, actual usage, number of rounds completed, and the last tool being executed
```

## openspec/changes/rlm-observability-and-robustness/specs/rlm-structured-reduction/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/rlm-structured-reduction/spec.md
- Lines: 1-66
- SHA256: b7b52f621a484c4acb636030b31187eb512842976e669f365e4cfa0957f6094b

```md
# rlm-structured-reduction Specification

## Purpose
Replace natural-language subagent result aggregation with structured data formats (claims for analysis, unified diff for code changes) to enable deterministic conflict detection and merging.

## ADDED Requirements

### Requirement: Subagent output format selection
The RLM pipeline SHALL select between `structured-claims` and `unified-diff` output formats based on the sub-task type.

#### Scenario: Analysis/exploration task uses claims format
- **WHEN** a sub-task is classified as analysis or exploration (e.g., searching codebase, investigating a bug)
- **THEN** the subagent SHALL be instructed to output results in `structured-claims/1` format

#### Scenario: Code modification task uses diff format
- **WHEN** a sub-task is classified as code modification (e.g., refactoring, implementing a feature)
- **THEN** the subagent SHALL be instructed to output results in `unified-diff/1` format

#### Scenario: Mixed task uses both formats
- **WHEN** a sub-task involves both analysis and code changes
- **THEN** the subagent SHALL output claims for the analysis portion and diffs for the code changes, each in their respective sections

### Requirement: Structured claims format
Analysis subagent results SHALL conform to the `structured-claims/1` JSON schema.

#### Scenario: Valid claims output
- **WHEN** a subagent produces analysis results
- **THEN** the output SHALL be a JSON object with `format: "structured-claims/1"` and a `claims` array, where each claim has `id`, `claim`, `evidence`, `confidence` (0.0-1.0), `conflicts_with` (array of claim IDs), `actionable` (boolean), and optional `recommendation`

#### Scenario: Claims with conflict detection
- **WHEN** a subagent identifies a finding that contradicts another claim
- **THEN** the claim's `conflicts_with` array SHALL reference the conflicting claim's `id`

#### Scenario: Claims confidence is numeric
- **WHEN** a subagent is uncertain about a finding
- **THEN** the `confidence` field SHALL reflect the uncertainty as a float between 0.0 and 1.0, not as a text label

### Requirement: Unified diff format
Code modification subagent results SHALL conform to the `unified-diff/1` JSON schema.

#### Scenario: Valid diff output
- **WHEN** a subagent produces code changes
- **THEN** the output SHALL be a JSON object with `format: "unified-diff/1"` and a `changes` array, where each change has `file`, `intent`, `diff` (unified diff string), `confidence` (0.0-1.0), and `depends_on` (array of file paths)

#### Scenario: Multiple files changed
- **WHEN** a subagent modifies multiple files
- **THEN** each file's change SHALL be a separate entry in the `changes` array with its own `intent` and `diff`

### Requirement: Aggregator merges structured results
The RLM Aggregator SHALL merge structured sub-task results deterministically before falling back to LLM synthesis.

#### Scenario: Claims deduplication by text similarity
- **WHEN** two sub-tasks produce claims with Jaccard similarity > 0.8 on the `claim` text
- **THEN** the Aggregator SHALL merge them into one claim, keeping the higher confidence value and combining evidence

#### Scenario: Conflict detection from conflicts_with
- **WHEN** any claim's `conflicts_with` array references another claim by ID
- **THEN** the Aggregator SHALL mark both claims as `status: conflicted` and present them for resolution

#### Scenario: Diff conflict detection by file path
- **WHEN** two sub-tasks produce changes for the same file path
- **THEN** the Aggregator SHALL mark those changes as `status: potential_write_conflict` and include both in the final output for review

#### Scenario: Fallback to LLM aggregation
- **WHEN** sub-task results cannot be parsed as valid structured output
- **THEN** the Aggregator SHALL fall back to the existing LLM-based merge with a warning in the output metadata
```

## openspec/changes/rlm-observability-and-robustness/specs/subagent-action-visibility/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/subagent-action-visibility/spec.md
- Lines: 1-63
- SHA256: 3210f0ec56a72a769817db2aa6d81e59a30dcaf0560ac0669590e05b044becbb

```md
# subagent-action-visibility — Delta Spec

## MODIFIED Requirements

### Requirement: Subagent tool calls are visible with key parameters
The TUI SHALL display the tool calls made by each subagent, including the tool name and key parameters, so users can perceive what actions the model is taking.

#### Scenario: Subagent calls a tool with parameters
- **WHEN** a subagent calls `file_read` with `file_path = "src/auth.rs"`
- **THEN** the TUI SHALL display `file_read("src/auth.rs")` as the current action for that subagent node

#### Scenario: Subagent calls a tool with multiple parameters
- **WHEN** a subagent calls `grep` with `pattern = "fn authenticate"` and `path = "src/"`
- **THEN** the TUI SHALL display `grep("fn authenticate", src/)` extracting the most meaningful 1-2 params

#### Scenario: Subagent calls a tool with long parameters
- **WHEN** a tool parameter value exceeds 80 characters
- **THEN** the params display SHALL be truncated to ~80 chars with "…" suffix

#### Scenario: Subagent has not called any tool yet
- **WHEN** a subagent is Running but has not yet made its first tool call (still in initial API call)
- **THEN** the TUI SHALL display "thinking…" as the current action

### Requirement: Subagent action history shows complete tool call timeline
Each subagent node SHALL maintain a complete, unbounded history of tool calls (name + params), visible in the overlay panel and detail view. The action log SHALL NOT be truncated in the transcript; the TUI panel MAY truncate the display for readability.

#### Scenario: Subagent has made multiple tool calls
- **WHEN** a subagent has called `grep`, `file_read`, and `file_read` in sequence
- **THEN** the overlay panel SHALL display all tool calls beneath that node, with the ability to scroll when the list exceeds the visible area

#### Scenario: Action history is persisted, not truncated
- **WHEN** a subagent has made more than 50 tool calls
- **THEN** all tool calls SHALL be preserved in the SQLite transcript; the TUI panel SHALL support scrolling/paging to view beyond the visible window

#### Scenario: Completed subagent action history
- **WHEN** a subagent reaches Completed status
- **THEN** the complete action log SHALL be written to SQLite and remain viewable via the detail view

### Requirement: Model text is displayed alongside tool calls
The TUI SHALL display the model's text responses alongside tool calls so users can see the think→call→think→call loop. The text snapshot shows what the model is analyzing or concluding; the action log shows what tools it called.

#### Scenario: Model text followed by tool call
- **WHEN** a subagent outputs text "I need to find where authentication logic is defined" then calls `grep("fn authenticate")`
- **THEN** the TUI SHALL display the text snapshot above the current tool action, so the display reads: the model's thought → then the action it took

#### Scenario: Tool call followed by model analysis
- **WHEN** a subagent completes a `file_read` call and the model responds with "Found the auth module, it needs refactoring in 3 places"
- **THEN** the TUI SHALL update the text snapshot to show the model's analysis, with the completed tool call now in the action history

#### Scenario: Full text preserved in transcript
- **WHEN** a subagent produces a text response of any length
- **THEN** the full text SHALL be recorded in the SQLite transcript; the TUI text snapshot MAY truncate for inline display but the detail view SHALL show the complete text

### Requirement: Inline subagent card shows current action with context
The inline subagent card rendered in the chat area SHALL show the current tool call with parameters and the most recent model text, so users can see what the subagent is doing without opening the overlay panel.

#### Scenario: Inline card during active subagent
- **WHEN** a subagent is Running with text snapshot "Analyzing the auth module structure…" and current tool `file_read("src/auth.rs")`
- **THEN** the inline card SHALL display the tool call with params and a dimmed preview of the model's text

#### Scenario: Inline card when subagent has no text yet
- **WHEN** a subagent is Running but has no text snapshot yet (first round, still streaming)
- **THEN** the inline card SHALL display "thinking…" and no text preview
```

## openspec/changes/rlm-observability-and-robustness/specs/subagent-content-preview/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/subagent-content-preview/spec.md
- Lines: 1-75
- SHA256: 65854be0f8b18167362f1643ba237fa65e2c66bebb93e7ef7ea4a216509a6ddc

```md
# subagent-content-preview — Delta Spec

## MODIFIED Requirements

### Requirement: SubagentProgress records tool call action log
The `SubagentProgress` struct SHALL include an `action_log: Vec<SubagentEvent>` field that records all tool calls and thoughts made by the subagent, each containing the event type, tool name / text, and a summary of key parameters. Tool results SHALL NOT be included in the action log during execution; they SHALL be stored separately in the SQLite transcript.

#### Scenario: Tool call started
- **WHEN** a subagent begins executing a tool call
- **THEN** a `SubagentEvent` with `event_type: Action { tool_name, params_summary }` SHALL be appended to the action log in the next progress event

#### Scenario: Action log preserves complete history for transcript
- **WHEN** the action log accumulates entries during subagent execution
- **THEN** all entries SHALL be preserved in memory until the subagent completes; the log SHALL NOT be truncated before writing to SQLite

#### Scenario: Action log written to SQLite on completion
- **WHEN** a new progress event is emitted for the final subagent status (Completed, Failed, Cancelled)
- **THEN** the complete action log SHALL be persisted to the SQLite transcript via `SubagentTranscriptStore`

### Requirement: SubagentProgress captures current tool parameters
The `SubagentProgress` struct SHALL include a `current_params: Option<String>` field that describes the key parameters of the currently executing tool, so the TUI can display not just the tool name but what it's operating on.

#### Scenario: Tool with file path parameter
- **WHEN** the subagent calls `file_read` with `file_path = "src/auth.rs"`
- **THEN** `current_params` SHALL be `Some("src/auth.rs")` and `current_tool` SHALL be `Some("file_read")`

#### Scenario: Tool with no meaningful params to summarize
- **WHEN** the subagent calls a tool with no extractable params
- **THEN** `current_params` SHALL be `None` and the TUI SHALL display just the tool name

### Requirement: Subagent text snapshots are captured during execution
The subagent execution loop SHALL capture the full assistant text response after each round and include a truncated snapshot in `SubagentProgress`. The full text SHALL be stored in the SQLite transcript for later retrieval.

#### Scenario: Subagent completes first round with text output
- **WHEN** a subagent finishes its first API call and produces a text response before any tool call
- **THEN** the emitted `SubagentProgress` SHALL include `text_snapshot` containing up to the last 200 characters of that response; the full text SHALL be queued for SQLite storage

#### Scenario: Subagent produces only tool calls with no text
- **WHEN** a subagent finishes a round with only tool calls and no assistant text
- **THEN** the emitted `SubagentProgress` SHALL have `text_snapshot` as `None` or empty

#### Scenario: Text snapshot is truncated for inline display
- **WHEN** the assistant text response exceeds 200 characters
- **THEN** the `text_snapshot` SHALL be truncated to the last 200 characters; the full text SHALL be available in the SQLite transcript

#### Scenario: Completed subagent archives full transcript
- **WHEN** a subagent reaches Completed status
- **THEN** the full action log, all text responses, and metadata SHALL be persisted to SQLite; the text snapshot in memory MAY be cleared

### Requirement: SubagentProgress includes token consumption
The `SubagentProgress.metadata.token_count` field SHALL be populated with the cumulative token usage from all API calls made by the subagent, reported on completion and at periodic intervals during execution.

#### Scenario: Subagent completes with known token usage
- **WHEN** a subagent completes after 3 API rounds consuming 500 input + 300 output tokens total
- **THEN** the final `SubagentProgress` event with status `Completed` SHALL have `metadata.token_count = Some(800)`

#### Scenario: Token counts unavailable from provider
- **WHEN** the API provider response does not include token usage information
- **THEN** `metadata.token_count` SHALL remain `None`

#### Scenario: Per-round token update during execution
- **WHEN** a subagent is Running and has completed at least one API round with token usage data
- **THEN** each progress event SHALL include the cumulative `token_count` in metadata to show live token consumption

### Requirement: Daemon progress store is session-scoped
The daemon's subagent progress storage SHALL be scoped by session ID so that concurrent sessions do not cross-contaminate progress data.

#### Scenario: Two concurrent sessions with subagents
- **WHEN** session A runs 2 subagents and session B runs 1 subagent concurrently
- **THEN** polling progress for session A SHALL return only session A's 2 subagent nodes
- **THEN** polling progress for session B SHALL return only session B's 1 subagent node

#### Scenario: Session disconnection cleans up progress
- **WHEN** a session disconnects or its progress poller stops
- **THEN** the session's progress entries SHALL be removed from the daemon store within a reasonable timeout (e.g., 60 seconds of no polling)
```

## openspec/changes/rlm-observability-and-robustness/specs/subagent-status-display/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/subagent-status-display/spec.md
- Lines: 1-76
- SHA256: dd1fe446f4ae27b05ea5d59e971295b13619a1bd60ce639f5e9b9de887e49665

```md
# subagent-status-display — Delta Spec

## MODIFIED Requirements

### Requirement: Status bar shows subagent progress counters
The TUI status bar SHALL display real-time subagent progress counters when subagents are active, including the number of active, completed, and failed subagents. Failed count SHALL be visually distinct (red).

#### Scenario: Multiple subagents running
- **WHEN** 3 subagents are active and 5 have completed out of 8 total
- **THEN** the status bar SHALL display a label like "3 active · 5/8 done" instead of a static "Subagent running…"

#### Scenario: All subagents complete successfully
- **WHEN** all subagents have completed with no failures
- **THEN** the status bar SHALL display "N tasks done" where N is the total subagent count

#### Scenario: Some subagents failed
- **WHEN** 2 subagents completed and 1 failed
- **THEN** the status bar SHALL display "2 done · 1 failed" with the failure count in red

#### Scenario: No subagents active
- **WHEN** no subagents are running and none have been used in the current turn
- **THEN** the status bar SHALL NOT display any subagent counter information

### Requirement: Subagent panel shows per-node timing, token usage, and budget
The subagent overlay panel SHALL display elapsed time, token consumption, and token budget (when set) for each subagent node.

#### Scenario: Active subagent node
- **WHEN** a subagent node is in Running status with 3 rounds completed out of 20 max
- **THEN** the panel SHALL display "round 3/20 · 12.3s" next to the node

#### Scenario: Completed subagent node with token and budget data
- **WHEN** a subagent node has completed, token_count is 1500, and token_budget was 10000
- **THEN** the panel SHALL display "1.5k/10k tokens · 45.2s" next to the completed node

#### Scenario: Subagent node without token data
- **WHEN** a subagent node has completed but token_count is None
- **THEN** the panel SHALL display elapsed time but SHALL NOT display token information

#### Scenario: Subagent exceeded budget
- **WHEN** a subagent was killed due to token budget exhaustion
- **THEN** the node SHALL display status Failed with error "Budget exceeded" and SHALL show the budget limit vs actual usage

### Requirement: Subagent panel shows error details and recovery actions
Failed and Cancelled subagent nodes SHALL display error details and offer recovery actions (retry, view details).

#### Scenario: Failed node with error message
- **WHEN** a subagent node has status Failed with error "Subagent timed out after 240 seconds"
- **THEN** the panel SHALL display the error message in red beneath the node label

#### Scenario: Retry action for failed node
- **WHEN** a Failed node is selected in the subagent panel
- **THEN** the panel SHALL display a hint "[r] retry  [d] details" and pressing `r` SHALL respawn the subagent with the same prompt and context

#### Scenario: Retry includes previous error context
- **WHEN** a subagent is retried after failure
- **THEN** the respawned subagent's system prompt SHALL include a `previous_attempt_error` field describing what went wrong

#### Scenario: Rollback before retry for code-modifying subagent
- **WHEN** a failed subagent had modified files before failing
- **AND** user presses `r` to retry
- **THEN** the system SHALL git-stash or revert the partial changes before respawning

#### Scenario: Detail view for failed node
- **WHEN** user presses `d` on a Failed node
- **THEN** the full transcript detail view SHALL open, scrolled to the error event

### Requirement: Progress delta tracking
Each `SubagentProgress` event SHALL include a `progress_delta: Option<f32>` field indicating the estimated progress increment since the last update.

#### Scenario: Progress delta reported during execution
- **WHEN** a subagent completes a round and has new findings relative to previous rounds
- **THEN** `progress_delta` SHALL be > 0.0, calculated as new_findings / total_expected_findings

#### Scenario: No progress detected
- **WHEN** two consecutive rounds produce progress_delta < 0.05
- **THEN** the subagent loop SHALL emit a warning event; after three consecutive low-delta rounds, the subagent SHALL abort with `StuckStatus::NoProgress`
```

## openspec/changes/rlm-observability-and-robustness/specs/subagent-transcript-storage/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/subagent-transcript-storage/spec.md
- Lines: 1-62
- SHA256: b7191d2221d69512c5cb9e7be898469d767d1c5427efd5c0e84882335143a90b

```md
# subagent-transcript-storage Specification

## Purpose
Persist complete subagent execution records (transcripts) in SQLite for historical querying, debugging, and replay.

## ADDED Requirements

### Requirement: Transcript database schema
The system SHALL maintain a SQLite database at `~/.wgenty-code/subagent_transcripts.db` with tables for transcript headers and per-round events.

#### Scenario: Database created on first use
- **WHEN** the first subagent transcript is written and the database file does not exist
- **THEN** the system SHALL create the database file with the correct schema automatically

#### Scenario: Transcript header row written on subagent completion
- **WHEN** a subagent reaches Completed, Failed, or Cancelled status
- **THEN** a row SHALL be inserted into `subagent_transcripts` with id, session_id, parent_id, label, status, system_prompt, user_prompt, started_at, finished_at, total_tokens, error_message (if any), and summary

#### Scenario: Events batch-written on subagent completion
- **WHEN** a subagent completes
- **THEN** all events (thought, action, tool_result, error) from the subagent's execution SHALL be inserted into `subagent_events` in a single transaction

### Requirement: Transcript store API
The `SubagentTranscriptStore` SHALL provide methods for listing, retrieving, and searching transcripts.

#### Scenario: List transcripts by session
- **WHEN** `list_by_session(session_id)` is called
- **THEN** all transcripts for that session SHALL be returned ordered by started_at descending

#### Scenario: Get transcript by ID with events
- **WHEN** `get_by_id(transcript_id)` is called
- **THEN** the transcript header and all associated events SHALL be returned

#### Scenario: Search transcripts by label substring
- **WHEN** `search(query)` is called with a text query
- **THEN** all transcripts whose label contains the query (case-insensitive) SHALL be returned, limited to 100 results

### Requirement: Transcript retention policy
Transcripts older than a configurable retention period SHALL be automatically deleted.

#### Scenario: Retention period honored
- **WHEN** a new transcript is written and `max_transcript_age_days` is set to 30
- **THEN** transcripts with `started_at` older than 30 days SHALL be deleted in the same transaction

#### Scenario: Unlimited retention
- **WHEN** `max_transcript_age_days` is set to 0
- **THEN** no automatic deletion SHALL occur

### Requirement: TUI transcript detail view
The TUI SHALL provide a full-screen transcript detail view accessible from the subagent panel.

#### Scenario: Open transcript detail for a node
- **WHEN** user presses Enter on a Completed or Failed node in the subagent panel
- **THEN** a full-screen view SHALL open showing the transcript header (status, timing, token count) and the full event timeline

#### Scenario: Navigate transcript detail
- **WHEN** the transcript detail view is open
- **THEN** Up/Down keys SHALL scroll through events; PageUp/PageDown SHALL scroll by page; Escape SHALL return to the subagent panel

#### Scenario: Transcript detail for running node
- **WHEN** user presses Enter on a Running node
- **THEN** the detail view SHALL show the partial event timeline available so far, with a "streaming…" indicator
```

## openspec/changes/rlm-observability-and-robustness/specs/task-complexity-detection/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/task-complexity-detection/spec.md
- Lines: 1-52
- SHA256: 9c4d85ae45fa6b373b516dcb400294d487507f0bcb1abc23e5ed4c89934b7a39

```md
# task-complexity-detection — Delta Spec

## MODIFIED Requirements

### Requirement: Task complexity detection uses structural analysis
The `is_complex_task()` function SHALL use structural analysis of the prompt to determine complexity. The function SHALL also classify the task type (analysis, modification, or mixed) to determine the appropriate structured output format.

#### Scenario: Simple single-step task
- **WHEN** the prompt is "create a file called config.json with default settings"
- **THEN** `is_complex_task()` SHALL return `false`, routing the task to direct execution

#### Scenario: Multi-step task with numbered steps
- **WHEN** the prompt contains numbered steps (e.g., "1. Refactor the auth module\n2. Update all callers\n3. Add tests") referencing multiple files
- **THEN** `is_complex_task()` SHALL return `true`, routing to RLM pipeline

#### Scenario: Task with explicit dependencies
- **WHEN** the prompt describes tasks where one depends on another (e.g., "after X completes, do Y")
- **THEN** `is_complex_task()` SHALL return `true`, routing to RLM pipeline

#### Scenario: Long but simple prompt
- **WHEN** the prompt is >1000 characters but describes a single straightforward operation
- **THEN** `is_complex_task()` SHALL NOT automatically classify it as complex based on length alone

### Requirement: Task type classification for structured output
The complexity detection SHALL classify tasks as `analysis`, `modification`, or `mixed` to determine which structured output format the subagent should produce.

#### Scenario: Analysis task
- **WHEN** the prompt primarily requests investigation, searching, or understanding (e.g., "find where authentication logic is implemented")
- **THEN** the task type SHALL be classified as `analysis` and the subagent SHALL use `structured-claims/1` output format

#### Scenario: Modification task
- **WHEN** the prompt primarily requests code changes (e.g., "refactor the auth module to use JWT")
- **THEN** the task type SHALL be classified as `modification` and the subagent SHALL use `unified-diff/1` output format

#### Scenario: Mixed task
- **WHEN** the prompt requests both investigation and code changes
- **THEN** the task type SHALL be classified as `mixed` and the subagent SHALL produce both claims and diffs

### Requirement: Routing decisions are logged and visible
When the `task` tool routes a prompt to RLM pipeline or direct subagent execution, the routing rationale and task type SHALL be included in the tool result metadata.

#### Scenario: Task routed to RLM with type classification
- **WHEN** a task is routed to the RLM pipeline
- **THEN** the tool result metadata SHALL include `routing_reason`, `task_type`, and `output_format` fields

#### Scenario: Task executed directly
- **WHEN** a task is executed as a direct subagent
- **THEN** the tool result SHALL indicate "direct execution" with task type in metadata

#### Scenario: TUI displays routing reason with format
- **WHEN** the tool result contains routing metadata
- **THEN** the TUI SHALL render the routing reason, task type, and output format as dimmed text near the subagent card
```

## openspec/changes/rlm-observability-and-robustness/specs/tui-command-completion/spec.md

- Source: openspec/changes/rlm-observability-and-robustness/specs/tui-command-completion/spec.md
- Lines: 1-62
- SHA256: 536ae9d621464358141f2e647c31b4c9b71d499e3c76ff951165d06be34b0369

```md
# tui-command-completion Specification

## Purpose
Enable users to interactively select and invoke skills and plugin commands directly from the TUI input box, without memorizing slash-command syntax.

## ADDED Requirements

### Requirement: Skills completion triggered by @ prefix
The TUI input box SHALL trigger a skill completion panel when the user types `@` followed by zero or more characters. The completion panel SHALL list all available skills whose names contain the typed substring (case-insensitive).

#### Scenario: User types @ to see all skills
- **WHEN** user types `@` in the input box
- **THEN** an inline completion panel SHALL appear showing all available skill names sorted alphabetically

#### Scenario: User filters skills by typing partial name
- **WHEN** user types `@com` in the input box
- **THEN** the completion panel SHALL filter to show skills containing "com" (e.g., "comet", "comet-open", "comet-build")

#### Scenario: User selects a skill from the panel
- **WHEN** user navigates to a skill name with arrow keys and presses Enter
- **THEN** the input box SHALL replace `@com` with the full skill invocation syntax (e.g., `/comet-open`)

#### Scenario: User dismisses completion panel
- **WHEN** the completion panel is visible and user presses Escape
- **THEN** the panel SHALL close and the `@` prefix text SHALL remain in the input box unchanged

### Requirement: Plugin command completion triggered by / prefix
The TUI input box SHALL trigger a plugin command completion panel when the user types `/` at the beginning of input (or after whitespace). The panel SHALL list all registered plugin commands.

#### Scenario: User types / to see plugin commands
- **WHEN** user types `/` at the start of the input box
- **THEN** an inline completion panel SHALL appear showing all registered plugin command names with their descriptions

#### Scenario: User filters plugin commands by typing partial name
- **WHEN** user types `/code-` in the input box
- **THEN** the completion panel SHALL filter to show plugin commands starting with "code-" (e.g., "code-review")

#### Scenario: Plugin command with required arguments
- **WHEN** user selects a plugin command that requires arguments
- **THEN** the input box SHALL populate with the command name followed by a space, and the panel SHALL show argument hints

### Requirement: Completion panel provides keyboard navigation
The completion panel SHALL support keyboard navigation consistent with the rest of the TUI.

#### Scenario: Navigate options with arrow keys
- **WHEN** the completion panel is visible
- **THEN** Up/Down arrow keys SHALL move the selection highlight; Tab SHALL move forward, Shift+Tab SHALL move backward

#### Scenario: Cycle through options
- **WHEN** user presses Tab past the last option
- **THEN** selection SHALL wrap to the first option

### Requirement: Completion data sources
The TUI SHALL source skill names from the local skills directory and plugin commands from the PluginRegistry.

#### Scenario: Skills directory scanned at startup
- **WHEN** the TUI starts
- **THEN** all directory names under `~/.claude/skills/` SHALL be loaded as available skill names

#### Scenario: Plugin commands loaded from registry
- **WHEN** plugins are loaded
- **THEN** all registered plugin commands with their descriptions SHALL be available for completion
```

