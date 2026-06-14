## 1. 核心 Diff 引擎

- [x] 1.1 使用 `TextDiff::grouped_ops(3)` 生成 unified diff hunks
- [x] 1.2 计算 hunk 头的 old/new 起止行号和范围
- [x] 1.3 实现上下文行收集（前后各 3 行）
- [x] 1.4 实现词级 diff（`TextDiff::from_words()` 对配对 delete/insert 行）

## 2. 渲染输出

- [x] 2.1 渲染统计摘要行（`▸ path/file  +N -M`）
- [x] 2.2 渲染 hunk 头行（`@@ -start,count +start,count @@`）
- [x] 2.3 渲染带行号 gutter 的独立视图
- [x] 2.4 渲染无 gutter 的紧凑/内联视图
- [x] 2.5 词级 diff 分段渲染（changed/unchanged 不同颜色）
- [x] 2.6 行截断保护（独立 50 行 / 内联 25 行）

## 3. 模块集成

- [x] 3.1 在 `components/mod.rs` 注册 `pub mod diff`
- [x] 3.2 将 `chat.rs` 中的内联 `diff_to_lines()` 替换为 `diff::diff_to_lines()`
- [x] 3.3 公开 `render()` 和 `diff_to_lines()` API

## 4. 测试

- [x] 4.1 空 diff 测试（无变更场景）
- [x] 4.2 简单变更测试（1 增 1 删）
- [x] 4.3 纯新增/纯删除测试
- [x] 4.4 词级 diff segments 测试
- [x] 4.5 多 hunk 测试
- [x] 4.6 渲染输出完整性测试
- [x] 4.7 Hunk 头格式测试
- [x] 4.8 全项目测试套件验证（155 个测试通过）
