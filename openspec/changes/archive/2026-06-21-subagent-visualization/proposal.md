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
