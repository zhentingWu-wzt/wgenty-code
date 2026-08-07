# Design: gui-advanced-panels

## Context

TUI 侧已有 subagent 状态展示相关能力（多个 subagent-* specs），tui-inspector change（进行中）在建设 TurnContext 采集与 5 个透视 tab（system prompt 10 层来源、召回记忆、messages JSON、hook 注入、token 统计）。daemon 提供 subagent 进度、tasks/todos 等端点。foundation 提供面板挂载点。

## Goals / Non-Goals

**Goals:**
- subagent 层级/状态/工具进度的树形可视化
- todos 面板实时同步
- 透视面板对齐 tui-inspector 的五类数据展示

**Non-Goals:**
- 核心对话、会话管理、配置界面（其他 change）
- TurnContext 采集机制本身（复用 tui-inspector 成果；若其未落地则本 change 降级为展示 daemon 已暴露的数据）
- 透视数据的编辑/写回

## Decisions

1. **数据复用优先**：subagent 与 todos 数据走 daemon 现有 API；透视数据优先复用 tui-inspector 的采集层，GUI 只做展示层，不重复采集。
2. **树形与分栏布局**：subagent tree 用可展开树组件，透视面板用 tab + 分栏，发挥 GUI 相对于 TUI 的布局优势。
3. **实时更新采用轮询/事件推送中的低成本方案**：build 阶段按 daemon 实际支持（SSE 事件或轮询）确定，不为面板单独扩展服务端推送通道。

## Risks / Trade-offs

- [tui-inspector 未完成导致透视数据源缺失] → 依赖检查前置；缺失时降级范围并在 tasks 中标注
- [subagent 高频状态刷新导致界面抖动] → 状态合并与节流刷新
- [透视面板展示完整 messages 可能包含敏感内容] → 默认折叠详情，展示前提示数据敏感性
