## Why

当前欢迎屏幕只展示 ASCII logo、标题和模型名，新用户进入 TUI 后看不到任何交互提示——既不知道可以输入消息开始对话，也不知道有哪些斜杠命令可用（`/help`、`/clear`、`/plan`、`/compact` 等）。用户需要主动摸索或查阅文档才能上手，增加了初次使用门槛。

## What Changes

在 `src/tui/components/welcome.rs` 的欢迎屏幕中添加项目特色简介和使用指南：

- Comet 工作流特色行：一行介绍 Comet spec-driven 开发流程（open → design → build → verify → archive）
- 交互提示：输入消息按 Enter 开始对话
- 核心命令速览：`/help`（查看全部命令）、`/plan`（规划模式）、`/clear`（清屏）、`/compact`（压缩上下文）

同时调整 `Layout` 约束以容纳新增行。

## Impact

- **Code**: `src/tui/components/welcome.rs`（新增特色简介 + 指南文本 + 调整 `Constraint::Length` 值）。
- **Docs**: 无。
- **User-visible behavior**: 首次进入 TUI 时欢迎屏幕显示项目特色简介和使用指南；开始对话后内容随欢迎屏幕一起消失（已有逻辑，无需改动）。
- **Non-goals**: 不改动 logo 渐变配色；不改动欢迎屏幕显示/隐藏逻辑；不新增 i18n 条目（欢迎屏现有文本均为硬编码，保持一致）。
