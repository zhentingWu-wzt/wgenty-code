# Brainstorm Summary

- Change: subagent-visualization
- Date: 2026-06-21

## 确认的技术方案

方案 A — 单文件内联 HTML + Option 扩展 TodoItem：

1. **nodes_to_json()**: 递归将 `Vec<TraceNode>` 序列化为 `serde_json::Value`，保留完整树结构和所有字段
2. **build_html_report()**: 生成自包含 HTML，Catppuccin Mocha 主题，内联 CSS (~200行) + JS (~150行)，3-tab 布局（Call Tree / Health Dashboard / Error Timeline），数据通过 `<script>const DATA={...};</script>` 内嵌
3. **截断修复**: 使用 `is_char_boundary()` 循环调整到合法 UTF-8 边界
4. **TodoItem 扩展**: `subagent: Option<SubagentTodoMeta>` 字段，包含 subagent_type/token_usage/rounds/duration_ms，`#[serde(default)]` 保证向后兼容
5. **Task Panel 渲染**: subagent 任务展示 🤖 图标 + 精简单行元数据

## 关键取舍与风险

- HTML 体积 (100节点~200KB) — 可接受，默认折叠深层节点
- Task Panel 宽度 30% — 信息精简为单行
- 向后兼容 — Option + serde(default) 保证旧 daemon 不传字段时不报错

## 测试策略

- nodes_to_json 单元测试：空树/嵌套树/多字节字符
- build_html_report 验证输出 HTML 结构完整性
- Task Panel subagent vs 普通任务渲染验证
- cargo check 全项目编译

## Spec Patch

无
