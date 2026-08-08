# Design Doc: GUI Advanced Panels

> 技术设计文档 for `gui-advanced-panels` change。

## 概述

高级面板：subagent 执行进度树、todos 实时同步、透视面板（deferred）。

## 设计决策

### 1. Subagent 树：复用 trace SSE 流

daemon 已有 `GET /subagents/trace/stream` SSE 端点，推送 `TraceEvent`（含 `node_id`、`parent_id`、`label`、`status`、`current_tool`、`cumulative_tokens`）。web/ 已通过 `usePermissionTrace` 订阅该流处理 permission/question 事件。

实现：在 `usePermissionTrace` 中额外消费 `kind === "progress"` 事件，写入 `subagentTraceStore`。前端 `SubagentTreePanel` 从 store 构建 parent-children 树渲染。

### 2. Todos 实时同步：轮询

TasksPanel 原先只在挂载时拉一次。改为 `usePolling(3s)` —— 足够及时捕获 todo 状态变更，不过度请求 daemon。

### 3. Inspector 透视面板：deferred 到独立 change

排查发现 inspector 的 5 类数据在 daemon 端**全部没有 API**，且 recall/hook 两类数据在 daemon run loop 里**根本不产生**（连 TUI inspector 自己也只有 2/5 个 tab 有真实数据）。

完整实现需要深度改造 daemon agent loop（`run_loop.rs` / `prompts/mod.rs` / `agent/runtime/`），包括：
- system prompt layers 需在 run loop 采集并广播
- per-turn memory recall 需在 daemon 侧注入采集（TUI 自己也没做）
- hook reminder 输出需暴露
- token usage 需从 runtime 层广播

这些改动影响面大、风险高，拆分为独立 change（`inspector-perspective`）处理。

## 影响范围

- 新增：`subagentTraceStore.ts`、`SubagentTreePanel.tsx`
- 修改：`usePermissionTrace.ts`（消费 progress 事件）、`TasksPanel.tsx`（加轮询）、`RightRail.tsx` + `uiStore.ts`（加 subagents 入口）
- 不改：daemon 服务端、TUI
