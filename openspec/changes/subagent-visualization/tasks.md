## 1. 修复字节索引 panic

- [x] 1.1 修复 `subagent_trace.rs:131` 工具参数截断 `&s[..60]`，使用 char-boundary 安全切分
- [x] 1.2 修复 `subagent_trace.rs:254` 错误消息截断 `&err[..100]`，使用 char-boundary 安全切分

## 2. HTML Report 核心实现

- [x] 2.1 实现 `nodes_to_json()` — 递归序列化 `Vec<TraceNode>` 为 `serde_json::Value`
- [x] 2.2 实现 `build_html_report()` — 生成自包含 HTML（内联 CSS/JS），包含可折叠调用树、Tab 切换、健康仪表盘、错误时间线
- [x] 2.3 验证 `SubagentTraceReporter::render_html_report()` 编译通过并正确组装数据

## 3. Task Panel 增强

- [x] 3.1 扩展 `TodoItem`（`src/tui/client.rs`），增加 `subagent: Option<SubagentTodoMeta>` 字段（含 subagent_type、token_usage、rounds、duration_ms）
- [x] 3.2 增强 `task_panel.rs` 渲染：对 subagent 任务展示 🤖 图标 + token/轮次/耗时信息
- [x] 3.3 在 `src/tasks/` 任务创建处填充 subagent 元数据（标记任务来源为 subagent 时）

## 4. 编译验证

- [x] 4.1 `cargo check` 全项目编译通过
- [x] 4.2 运行相关单元测试确认无回归
