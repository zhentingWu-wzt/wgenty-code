## Why

目前 TUI 中的文件变更展示仅显示裸 `+`/`-` 行，缺少上下文、hunk 头、词级高亮和统计信息。对标 `git diff --unified` 和 Claude Code 的 diff 展示，用户需要更丰富、更可读的变更视图来快速理解代码修改。

## What Changes

- **重写 diff 渲染核心**：基于 `similar` crate 的 `grouped_ops()` 生成标准 unified diff 格式
- **Hunk 头**：`@@ -start,count +start,count @@` 精确标注变更位置
- **上下文行**：变更前后各保留 3 行不变上下文（可配置）
- **词级 Diff 高亮**：行内变更词用更亮的颜色标记（绿色/红色的深色和浅色变体）
- **统计摘要**：文件头显示 `▸ path/to/file  +N -M`
- **双模式渲染**：独立视图（带行号 gutter）和聊天内嵌（紧凑模式）
- **截断保护**：独立视图 50 行上限，内联 25 行上限
- **注册 diff 模块**：在 `components/mod.rs` 中声明并公开 API

## Capabilities

### New Capabilities
- `rich-diff-display`: 以 unified diff 格式渲染文件变更，包含 hunk 头、上下文行、词级高亮和统计信息

### Modified Capabilities
<!-- 无现有 capability 被修改，这是全新功能 -->

## Impact

| 影响范围 | 详情 |
|---------|------|
| 修改文件 | `src/tui/components/diff.rs` (重写), `src/tui/components/chat.rs` (适配新 API), `src/tui/components/mod.rs` (注册模块) |
| 依赖 | `similar` 2.5（已引入）— 无新增依赖 |
| API | 公开 `render()` 和 `diff_to_lines()` 两个函数 |
| 测试 | 8 个单元测试覆盖：空 diff、简单变更、纯增/删、词级 diff、多 hunk、渲染输出、hunk 格式 |
