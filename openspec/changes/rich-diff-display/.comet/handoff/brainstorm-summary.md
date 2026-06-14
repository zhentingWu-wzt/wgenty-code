# Brainstorm Summary

- Change: rich-diff-display
- Date: 2026-06-14

## 确认的技术方案

基于 `similar` crate 的 `TextDiff::grouped_ops(3)` 生成标准 unified diff hunks，`TextDiff::from_words()` 做行内词级 diff。双渲染模式：独立视图（ratatui Paragraph + 行号 gutter）和聊天内嵌（紧凑模式，无 gutter）。配色沿用项目已有 diff 颜色常量。

## 关键取舍与风险

- **syntect 未集成**：代码内容纯色渲染，非语法高亮。syntect 已引入，路径清晰，后续可加。
- **截断保护**：独立 50 行 / 内联 25 行上限，防止大 diff 撑爆终端。
- **词级 diff 仅在配对 del/ins 行执行**：性能可接受，单次 diff 通常 <100 行。

## 测试策略

8 个单元测试覆盖：空 diff、简单变更、纯增/删、词级 segments、多 hunk、渲染输出、hunk 格式。全项目 155 测试通过，零 warning。

## Spec Patch

无
