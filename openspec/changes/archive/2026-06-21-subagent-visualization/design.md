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
