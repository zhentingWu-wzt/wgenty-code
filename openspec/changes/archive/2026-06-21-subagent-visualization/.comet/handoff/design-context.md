# Comet Design Handoff

- Change: subagent-visualization
- Phase: design
- Mode: compact
- Context hash: 3085e9768b4d7928ce9d1bc5d4919afcefb25408f377af46491c05ad8e4a43e3

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/subagent-visualization/proposal.md

- Source: openspec/changes/subagent-visualization/proposal.md
- Lines: 1-24
- SHA256: 87f1be65291a6fa2e3b78d917dc043b7887677ff5bb795948febab11a962eec1

```md
## Why

Subagent 执行追踪的 HTML 报告功能在实现过程中因崩溃中断（`lenient_json.rs` 字节索引 panic），遗留 `nodes_to_json()` 和 `build_html_report()` 两个核心函数未实现导致编译失败。同时，Task Panel（Ctrl+T）目前将所有任务（subagent 与普通任务）混为一谈，仅展示统一的待办列表，无法让用户快速感知 subagent 的执行状态、token 消耗和耗时。需补齐 HTML 报告并增强 Task Panel 对 subagent 的区分展示能力。

## What Changes

- **完成 HTML Report**: 实现 `nodes_to_json()` 将 TraceNode 树序列化为 JSON，实现 `build_html_report()` 生成自包含 HTML 页面（内联 CSS/JS），包含可折叠调用树、事件时间线、健康仪表盘
- **修复字节索引 panic**: `subagent_trace.rs:131`（工具参数截断）和 `subagent_trace.rs:254`（错误消息截断）存在与 `lenient_json.rs:217` 相同的多字节字符切片 panic 风险
- **Task Panel 增强**: 扩展 `TodoItem` 携带 subagent 元数据（subagent_type、token_usage、rounds、duration），Task Panel 渲染时对 subagent 任务展示专用图标、token/轮次/耗时信息

## Capabilities

### New Capabilities
- `subagent-trace-html-report`: 自包含 HTML 报告，展示 subagent 调用树、事件时间线和健康仪表盘，无需外部依赖

### Modified Capabilities
- `subagent-status-display`: Task Panel 增加 subagent 任务的区分展示，包含 token 消耗、轮次、耗时等关键指标

## Impact

- `src/teams/subagent_trace.rs` — 新增 `nodes_to_json()`、`build_html_report()`，修复字节索引截断
- `src/tui/components/task_panel.rs` — 增强渲染逻辑区分 subagent 任务
- `src/tui/client.rs` — `TodoItem` 扩展 subagent 元数据字段
- `src/tasks/` — 任务创建时携带 subagent 标识
```

## openspec/changes/subagent-visualization/design.md

- Source: openspec/changes/subagent-visualization/design.md
- Lines: 1-49
- SHA256: 872cfbd35c72926884fbd3251b55b9097da9528b042efbcb20cb293cce541fb4

```md
## Context

Subagent 追踪基础设施（`SubagentTraceReporter`）已实现 ASCII 调用树、Chrome Trace Event 导出、错误时间线三种输出格式。HTML 报告作为第四种格式，代码骨架已就位，仅缺两个序列化/渲染函数。Task Panel 当前只展示泛化 `TodoItem` 列表，不区分 subagent 任务。

## Goals / Non-Goals

**Goals:**
- 实现 `nodes_to_json()`: 将 `Vec<TraceNode>` 递归序列化为 `serde_json::Value`，保留树结构和所有字段
- 实现 `build_html_report()`: 生成自包含 HTML 页面，内联 CSS（Catppuccin 主题）和 JS（无外部依赖），支持调用树展开/折叠、健康仪表盘、错误列表
- 修复 2 处 `&s[..N]` 字节索引 panic
- Task Panel 对 subagent 任务展示专用图标（🤖）、状态、token、轮次、耗时

**Non-Goals:**
- 不改变 ASCII/Chrome Trace 输出格式
- 不新增 `TraceNode` 数据模型字段
- 不改变 Subagent Monitor Panel（Ctrl+Shift+T）——该面板已足够丰富
- Task Panel 只增强展示，不改变任务创建/管理流程

## Decisions

### 1. HTML 报告：内联自包含

**选择**: 单文件 HTML，CSS/JS 全部内联，无 CDN 依赖。

**理由**: Daemon 环境可能无网络；内联保证离线可用。CSS 使用 Catppuccin Mocha 色系与 TUI 主题一致。JS 仅约 150 行实现树展开/折叠、Tab 切换（调用树/时间线/健康）。

**替代方案**: 使用模板引擎（如 Tera）→ 引入额外依赖，过度设计。使用 CDN 图表库（如 ECharts）→ 需要网络。

### 2. JSON 嵌入策略

**选择**: `nodes_to_json()` 返回 `serde_json::Value`，在 HTML 中通过 `<script>const DATA = {...};</script>` 内嵌。

**理由**: 简单直接，编译期类型安全，无需序列化+反序列化往返。

### 3. 字节索引修复

**选择**: 复用 `lenient_json.rs` 中已新增的 `floor_char_boundary`/`ceil_char_boundary` 模式，但原地实现（避免跨模块依赖 util 函数）。使用 `s.char_indices()` 或 `s.is_char_boundary()` 循环查找边界。

### 4. Task Panel TodoItem 扩展

**选择**: 在 `TodoItem` 中增加可选字段 `subagent: Option<SubagentTodoMeta>`，包含 `subagent_type`、`token_usage`、`rounds`、`duration_ms`。Panel 渲染时根据此字段切换展示。

**理由**: 向后兼容——普通任务的 `subagent` 字段为 `None`，渲染逻辑不变。Daemon 端只需在任务来源为 subagent 时填充该字段。

## Risks / Trade-offs

- [大量 subagent 节点时 HTML 体积较大] → 限制展开深度默认 3 层，折叠深层节点
- [Task Panel 宽度有限（30%）] → subagent 信息精简为单行: `🤖 agent_name · 3r · 2.1s · 5k tokens`
- [TodoItem 新增字段需同步 daemon API] → 使用 `Option` + `#[serde(default)]` 保证向后兼容
```

## openspec/changes/subagent-visualization/tasks.md

- Source: openspec/changes/subagent-visualization/tasks.md
- Lines: 1-21
- SHA256: 8713190065c5deba83b36981c444880f53d173d3e9df492a38b677224d96b835

```md
## 1. 修复字节索引 panic

- [ ] 1.1 修复 `subagent_trace.rs:131` 工具参数截断 `&s[..60]`，使用 char-boundary 安全切分
- [ ] 1.2 修复 `subagent_trace.rs:254` 错误消息截断 `&err[..100]`，使用 char-boundary 安全切分

## 2. HTML Report 核心实现

- [ ] 2.1 实现 `nodes_to_json()` — 递归序列化 `Vec<TraceNode>` 为 `serde_json::Value`
- [ ] 2.2 实现 `build_html_report()` — 生成自包含 HTML（内联 CSS/JS），包含可折叠调用树、Tab 切换、健康仪表盘、错误时间线
- [ ] 2.3 验证 `SubagentTraceReporter::render_html_report()` 编译通过并正确组装数据

## 3. Task Panel 增强

- [ ] 3.1 扩展 `TodoItem`（`src/tui/client.rs`），增加 `subagent: Option<SubagentTodoMeta>` 字段（含 subagent_type、token_usage、rounds、duration_ms）
- [ ] 3.2 增强 `task_panel.rs` 渲染：对 subagent 任务展示 🤖 图标 + token/轮次/耗时信息
- [ ] 3.3 在 `src/tasks/` 任务创建处填充 subagent 元数据（标记任务来源为 subagent 时）

## 4. 编译验证

- [ ] 4.1 `cargo check` 全项目编译通过
- [ ] 4.2 运行相关单元测试确认无回归
```

## openspec/changes/subagent-visualization/specs/subagent-status-display/spec.md

- Source: openspec/changes/subagent-visualization/specs/subagent-status-display/spec.md
- Lines: 1-18
- SHA256: bc12fe44c077041a5ef807230c57bcdb89cfdeaa87c405791fecd29271eaa323

```md
# subagent-status-display Delta Specification

## ADDED Requirements

### Requirement: Task Panel shows subagent-specific metadata
The Task Panel (Ctrl+T) SHALL distinguish subagent tasks from regular tasks by displaying a subagent icon (🤖), subagent type, token usage, round count, and elapsed duration when the task originates from a subagent.

#### Scenario: Subagent task visible in Task Panel
- **WHEN** a subagent task is created with subagent metadata (type="explore", tokens=2500, rounds=3, duration_ms=12300)
- **THEN** the Task Panel SHALL display "🤖 explore · 3r · 12.3s · 2.5k tokens" with the subagent icon in a distinct color

#### Scenario: Regular task in Task Panel
- **WHEN** a regular task (non-subagent) is displayed in the Task Panel
- **THEN** the Task Panel SHALL display it with the existing task icon and label format, unchanged from current behavior

#### Scenario: Backward compatibility
- **WHEN** the daemon sends TodoItem data without the `subagent` field
- **THEN** the Task Panel SHALL render the item as a regular task without errors
```

## openspec/changes/subagent-visualization/specs/subagent-trace-html-report/spec.md

- Source: openspec/changes/subagent-visualization/specs/subagent-trace-html-report/spec.md
- Lines: 1-56
- SHA256: 00359e53749f3d0924f5ce3a1f5f90a3306e32a2a858411cdbf419928ee5d55a

```md
# subagent-trace-html-report Specification

## Purpose
Define the self-contained HTML report for subagent execution trace visualization.

## ADDED Requirements

### Requirement: HTML report is self-contained
The system SHALL generate a single HTML file with all CSS and JavaScript inlined, requiring no external network dependencies or CDN resources.

#### Scenario: Offline viewing
- **WHEN** the HTML report is opened in a browser without network access
- **THEN** all styling, interactivity, and data SHALL render correctly

### Requirement: Collapsible call tree
The HTML report SHALL render the subagent call tree with expand/collapse functionality, showing each node's status icon, label, duration, token usage, and round count.

#### Scenario: Default collapsed view
- **WHEN** the HTML report is first opened
- **THEN** root-level nodes SHALL be visible and child nodes beyond depth 3 SHALL be collapsed by default

#### Scenario: Expand node
- **WHEN** user clicks a collapsed node's expand icon
- **THEN** its direct children SHALL become visible with a smooth transition

### Requirement: Tab navigation
The HTML report SHALL provide tab navigation between three views: Call Tree, Health Dashboard, and Error Timeline.

#### Scenario: Switch tabs
- **WHEN** user clicks a tab header
- **THEN** the corresponding content panel SHALL be displayed and other panels SHALL be hidden

### Requirement: Health dashboard
The health dashboard SHALL display overall subagent health metrics including success rate, health score, total runs, average rounds/tokens/duration, and failure mode breakdown with severity indicators.

#### Scenario: Healthy status
- **WHEN** overall success rate is above 90%
- **THEN** the health dashboard SHALL display a green "Healthy" indicator

#### Scenario: Critical status
- **WHEN** overall success rate is below 50%
- **THEN** the health dashboard SHALL display a red "Critical" indicator with failure mode recommendations

### Requirement: JSON-safe TraceNode serialization
`nodes_to_json()` SHALL serialize `Vec<TraceNode>` into `serde_json::Value`, preserving tree structure, all node fields, and child event arrays, without panicking on any valid UTF-8 content.

#### Scenario: Multi-byte UTF-8 in node content
- **WHEN** a TraceNode label or event data contains multi-byte UTF-8 characters (e.g., box-drawing, CJK, emoji)
- **THEN** `nodes_to_json()` SHALL produce valid JSON without panicking

### Requirement: String truncation is char-boundary safe
All string truncation operations in the trace module SHALL ensure slice boundaries fall on valid UTF-8 character boundaries, preventing panics with multi-byte characters.

#### Scenario: Multi-byte character at truncation boundary
- **WHEN** a string containing multi-byte UTF-8 character '─' (3 bytes) at the truncation cutoff position
- **THEN** the truncation SHALL adjust to the nearest valid char boundary instead of panicking
```

