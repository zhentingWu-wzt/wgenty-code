---
change: subagent-visualization
design-doc: docs/superpowers/specs/2026-06-21-subagent-visualization-design.md
base-ref: 3b60351444a8d4a0704fb9adab58a084c02aec9b
---

# Subagent Visualization — Implementation Plan

## 概述

四组任务逐步实现：修复字节 panic → HTML Report → Task Panel 增强 → 编译验证

## Group 1: 修复字节索引 panic

### 1.1 定义 safe_truncate 辅助函数 + 修复工具参数截断
**文件**: `src/teams/subagent_trace.rs` 第 130 行
**改为**: 安全 char-boundary 截断

### 1.2 修复错误消息截断 (line 254) + error_timeline (line 383)
**改为**: 使用 safe_truncate

## Group 2: HTML Report 核心实现

### 2.1 实现 nodes_to_json() — 递归序列化 Vec<TraceNode> → serde_json::Value
### 2.2 实现 build_html_report() — 生成自包含 HTML（内联CSS/JS，3-tab）
### 2.3 验证 render_html_report() 编译通过

## Group 3: Task Panel 增强

### 3.1 tasks::todo_write — 新增 SubagentTodoMeta + TodoItem.subagent 字段
### 3.2 daemon::models — TodoItemResponse 添加 subagent
### 3.3 daemon::handlers — get_todos 传递 subagent
### 3.4 tui::client — TodoItem 添加 subagent 字段
### 3.5 task_panel.rs — subagent 任务渲染增强（🤖 + 统计信息）
### 3.6 tools::meta::task — ToolOutput.metadata 返回 subagent 统计数据

## Group 4: 编译验证

### 4.1 cargo check 全项目编译
### 4.2 cargo test 相关测试

## 文件修改清单

| 文件 | 修改类型 |
|------|----------|
| src/teams/subagent_trace.rs | 修改 + 新增（safe_truncate, nodes_to_json, build_html_report, tests） |
| src/tasks/todo_write.rs | 修改（SubagentTodoMeta, TodoItem 扩展） |
| src/tasks/mod.rs | 修改（导出 SubagentTodoMeta） |
| src/daemon/models.rs | 修改（TodoItemResponse 扩展） |
| src/daemon/handlers.rs | 修改（get_todos 传递 subagent） |
| src/tui/client.rs | 修改（SubagentTodoMeta + TodoItem 扩展） |
| src/tui/components/task_panel.rs | 修改（subagent 渲染增强） |
| src/tools/meta/task.rs | 修改（metadata 传递 subagent 统计） |
