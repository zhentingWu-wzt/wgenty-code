---
archived-with: 2026-08-08-inspector-perspective
status: final
---
# Design Doc: Inspector Perspective

> 技术设计文档 for `inspector-perspective` change。

## 概述

Inspector 透视面板：展示 5 类 turn 上下文数据（system prompt layers、召回记忆、新消息、hook reminder、token usage），通过新的 TurnContext SSE 事件广播。

## 设计决策

### 1. 回忆功能缺口补齐

daemon `run_session_turn` 之前完全不调用 memory recall（TUI 走 AgentLoop 才有）。现在用 `state.memory_manager` 调 `MemoryContextInjector::recall`，和 TUI 对齐。recall 返回类型扩展为 `(text, Vec<RecalledMemory>)`——文本注入 prompt，结构化元数据给 inspector。

### 2. TurnContext 一次性广播

5 类数据随 `SessionEventKind::TurnContext` 在 turn 结束后一次性广播，而非每类独立事件。payload 紧凑：layers 只带 label/source/char_count（content 截断），messages 截断 500 字符，memories 截断 200 字符 preview。

### 3. Hook reminder deferred

daemon 的 HookManager（runtime/hooks）处理工具生命周期 hook（PreToolUse/PostToolUse），不处理 prompt reminder。TUI 使用 plugins/hooks.rs 的 InjectedFragment 收集——daemon 侧需要更深的集成。Inspector 面板的 Hooks tab 显示"数据不可用"提示。

### 4. Token counter 注入

创建 run-scoped `TokenCounter` 注入 `LoopHooks.token_counter`，turn 结束后读取 per-turn input/output tokens。零配置，自动采集。

## 影响范围

- daemon: run_loop.rs（recall + token_counter + TurnContext 广播）、inject.rs（recall 返回类型扩展）
- TUI: turn.rs（适配 recall 新签名）、server_side.rs（TurnContext match 分支）
- 前端: InspectorPanel（新建）、sessionRunner（处理事件）、sessionStore（存储）、RightRail（入口）
