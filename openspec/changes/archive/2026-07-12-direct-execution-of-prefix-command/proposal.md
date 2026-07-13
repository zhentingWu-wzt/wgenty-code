# Proposal: 直接执行 `!` 前缀命令

## 动机

Wgenty Code 的 TUI 输入框目前只支持两类输入：`/` 斜杠命令（内置 + workflow 路由）和普通消息（交给 agent 处理）。用户没有便捷的方式在对话中直接运行一条本地 shell 命令并立刻看到输出——必须等 agent 走一轮 tool-call 流程（`execute_command` / `exec_command`），既慢又会消耗 token，也无法脱离 agent turn 独立执行。

Claude Code CLI 提供了 `!` 前缀的 bash mode：在输入框中输入 `! <command>` 即在本地直接执行该命令并把输出展示在对话流中。本变更将该能力引入 Wgenty Code。

## 目标

- 在 TUI 输入框中，当用户提交的文本以 `!` 开头时，剥离 `!` 后的剩余内容作为 shell 命令直接在本地执行（经 `sh -c`）。
- 执行不经过 agent turn、不消耗 LLM token、不进入 conversation history。
- 输出（stdout/stderr/exit code）以系统消息形式展示在对话流中。
- 与现有斜杠命令路由互不干扰：`!` 处理在 `/` 之前判定，`/` 命令行为不变。

## 范围

**包含**
- `submit_input` 中新增 `!` 前缀检测与命令执行分支。
- 后台异步执行命令，结果通过已有的 `AppEvent` 通道回传 UI。
- 结果系统消息格式化（命令、退出码、stdout、stderr）。
- 输入框样式：`!` 开头时给予与 `/` 一致的强调色提示。
- `/help` 文档中补充 `!` 说明。

**不包含**
- 命令审批 / 沙箱隔离 / 危险命令拦截（与 Claude Code bash mode 行为一致：直接执行）。
- 命令历史补全（`!` 不进入 `/` 的补全引擎）。
- headless / voice / daemon 非 TUI 输入路径。
- 可交互、长驻会话式命令（这是 `exec_command` 工具的职责）。

## 非目标 / 不改变

- 不新增 capability：复用现有 `tokio::process::Command` 与 `AppEvent` 通道。
- 不改变架构与接口：仅在 `submit_input` 入口新增一个前置分支，并复用 `push_system_message`。
- 不影响 agent turn、permission、sandbox、hook 链路。
