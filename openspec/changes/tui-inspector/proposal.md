# Proposal: TUI Request Context Inspector

## Summary

在 TUI 中新增一个**请求上下文透视面板（Inspector）**，让用户能实时查看每次请求发给 LLM 的完整上下文：system prompt 分层来源、召回记忆、hook 注入、完整 messages 数组和 token 统计。

## Motivation

当前 wgenty-code 的 TUI 有丰富的功能面板（diff、subagent、session、memory、plan），但**缺少对"agent 看到了什么"的可见性**。开发者需要理解：

- agent 为什么做出某个决策——它看到了哪些系统指令？
- 本轮召回了哪些记忆——TF-IDF 召回了什么？全局记忆注入了哪些？
- System prompt 来自哪里——哪几层是内置的？哪几层来自用户配置/AGENTS.md/WGENTY.md 文件？
- 完整的 API messages 是什么样的——system + history + user 的顺序和内容。

现有的 `prompt.debug_dump_reminder` 只能 dump `<system-reminder>` 到文件，不包含 system cascade 且非 TUI 内交互。

## Scope

### In Scope

1. **删除遗留 `task_panel`**：被 `subagent_tree` + `status_bar` + `plan_panel` 取代，释放右侧 30% Split Pane 位置给 Inspector
2. **侧边栏 Inspector 面板**：`F2` 切换右侧 ~35% 分屏面板
3. **System Prompt 分层视图 Tab**：10 层 system prompt 树形展示，每层标注来源（内置/配置路径/文件路径），可展开查看内容
4. **召回记忆视图 Tab**：每轮的 project/global 记忆列表，标注 scope、importance、TF-IDF score
5. **完整 Messages 视图 Tab**：API messages 数组完整展示，JSON 折行渲染
6. **Hook 注入视图 Tab**：`<system-reminder>` hook 注入结果（model-only / transcript）
7. **Token 统计 Tab**：各层消息的字符数 + 估算 token 数
8. **历史轮次快照**：App state 维护最近 N 轮的 `TurnContext`，在 Inspector 内 ↑↓ 切换

### Out of Scope

- Diff 高亮渲染（已实现 `diff.rs`）
- 子代理面板（已实现 `subagent_tree` + `focus_view` + `status_bar`）
- 会话列表/切换（已实现 `/session` slash command）
- Web 可视化（Phase 2 `web-ops-console`）
- 快捷键体系全面重构
- Config 热修改

## Architecture Impact

| 模块 | 变更 |
|------|------|
| `src/tui/components/task_panel.rs` | **删除**：遗留组件，被 subagent/plan 组件取代 |
| `src/tui/app/mod.rs` | 移除 `task_panel` 字段；增加 `turn_contexts: Vec<TurnContext>` ring buffer |
| `src/prompts/mod.rs` | `AssembledInstructions` 增加 `LayerMeta` 结构，记录每层来源 |
| `src/tui/app/turn.rs` | 每轮完成后抓取 `TurnContext` 快照 |
| `src/tui/components/inspector.rs` | **新组件**：5 tab 渲染 + 历史切换 |
| `src/tui/app/render.rs` | 移除 task_panel 渲染逻辑 + Inspector 渲染调度 |
| `src/tui/app/event.rs` | 移除 Ctrl+T toggle；增加 F2 toggle |
