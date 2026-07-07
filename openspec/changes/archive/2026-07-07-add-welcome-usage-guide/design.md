## 方案

### 在欢迎屏幕中添加特色简介和使用指南

在 `src/tui/components/welcome.rs` 的 `render` 函数中，于 "Model: {model_name}" 行之后追加：

1. 一个空行分隔
2. Comet 工作流特色行：`Comet spec-driven workflow · open → design → build → verify → archive`
3. 一个空行分隔
4. 交互提示行：`Type your message and press Enter to start.`
5. 命令速览行：`/help · commands  ·  /plan · plan mode  ·  /clear · reset  ·  /compact · compress`

特色简介行使用 `Color::Rgb(160, 140, 200)`（淡紫色，与 logo 渐变呼应但不抢眼）；交互提示和命令速览使用 `Color::Rgb(120, 120, 140)` 暗灰色，与模型名行（`Rgb(140, 140, 160)`）接近但略暗，视觉上不喧宾夺主。

### 布局调整

当前 `Layout` 约束为 `[Constraint::Length(11), Constraint::Min(0)]`。

现有 11 行 = 6 logo + 1 空 + 1 标题 + 1 副标题 + 1 模型名 + 1 空（trailing）。

新增 5 行（2 空 + 1 特色 + 2 指南），将 `Length(11)` 改为 `Length(16)`。

### 特色简介内容选择

特色行聚焦 Comet spec-driven 工作流——这是 Wgenty Code 区别于普通编码助手的核心特色：

- **spec-driven** — 以 proposal/design/tasks 文档驱动开发，非即兴编码
- **open → design → build → verify → archive** — 五阶段 phase-gated 流程，guard 检查卡点
- **斜杠命令驱动** — `/comet`、`/comet-tweak`、`/comet-build` 等命令触发对应流程

不列入的：具体工具数量（属实现细节）、sandbox/guardian（属安全机制，非工作流特色）、MCP/plugins（属扩展生态）。

### 取舍

- **为何硬编码而非 i18n**: 欢迎屏现有所有文本（"Wgenty Code"、"高性能 AI 编码助手"、logo）均为硬编码，新增指南保持一致；后续若整体 i18n 化再统一迁移。
- **为何只列 4 个命令**: `/help`、`/plan`、`/clear`、`/compact` 覆盖最高频操作；`/continue`、`/undo`、`/init` 使用频率较低，避免指南过长。用户可通过 `/help` 查看完整列表。
- **为何不动态读取 `default_builtin_commands`**: 欢迎屏在渲染热路径上，且指南只需精选命令；动态构建会引入不必要的复杂度。
- **为何特色行用淡紫色而非暗灰色**: 特色简介是"吸引"内容（让用户了解能力），暗灰色适合"辅助"内容（操作提示）；淡紫色与 logo 渐变同色系，视觉上形成从 logo → 标题 → 特色 → 操作的层次递进。

### 验证

- `cargo build` 通过。
- `cargo clippy -- -D warnings` 零 warning。
- `cargo fmt --check` 通过。
- 手动启动 TUI 确认欢迎屏幕显示特色简介和指南，且开始对话后消失。
