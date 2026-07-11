# Design: 直接执行 `!` 前缀命令

## 实现说明

在 TUI 输入提交入口 `App::submit_input`（`src/tui/app/input.rs`）最前面新增一个 `!` 前缀分支，所有现有 `/` 斜杠命令与普通消息逻辑保持不变。

### 判定规则

```text
let trimmed = text.trim();
if let Some(rest) = trimmed.strip_prefix('!') {
    // rest 为命令体
}
```

- 仅当首字符为 `!` 时触发。`!` 后可有可无空格（`!ls` 与 `! ls` 等价），实现时对 `rest` 做 `trim_start`。
- 纯 `!`（无命令体）视为无效输入，提示用户，不执行。
- 该判定置于所有 `/` 斜杠命令判定之前，确保 `!` 永远不会被 `/` 路由或当作普通消息。

### 执行模型

采用 fire-and-forget 的后台 `tokio::spawn`，与现有 `submit_input` 内的 spawn 模式（如 `/clear`、`/continue`、workflow 路由）一致：

1. 调用方持有 `self.event_tx` 的克隆（参考 event.rs 中 `tokio::spawn` 闭包的写法）。
2. 在 spawn 的任务里用 `tokio::process::Command::new("sh").arg("-c").arg(command)` 执行，带超时（默认 120s，与 `execute_command` 工具的最低超时一致）。
3. 工作目录取 `std::env::current_dir()`，与 TUI 其余命令执行一致。
4. 捕获 stdout / stderr / exit code，格式化为单条系统消息，通过 `event_tx.send(AppEvent::BackgroundTaskResult(...))` 回传。
5. UI 侧已有 `AppEvent::BackgroundTaskResult` 处理（event.rs:1057），会将其作为 `MessageRole::System` 推入 `committed_messages`，无需新增事件类型或渲染分支。

### 为什么用 `BackgroundTaskResult` 而非新事件

`BackgroundTaskResult(String)` 已是「后台任务结果以系统消息呈现」的通用通道（当前用于 subagent/background 通知）。`!` 命令结果在语义上完全吻合，复用它可避免新增 enum 变体、render 分支与 util.rs 的 `None` 分类项。

### 结果消息格式

```text
$ <command>
(stdout 内容，若有)
(exit code N，仅非零时显示)
(stderr 内容，若有)
```

- 始终包含执行的命令行（前缀 `$ `），便于回溯。
- stdout/stderr 合并展示但分别标注；为空则省略该段。
- 退出码为 0 时不显示 exit 行；非零时显示 `exit code N`。
- 超时：显示 `Command timed out (120s)`。
- 执行失败（spawn 失败）：显示错误信息。

### 输入框样式

`src/tui/components/input.rs` 的 `update_style` 当前仅对 `/` 做强调色高亮。扩展为：首字符为 `/` **或** `!` 时，前缀段使用 `ACCENT` 色、命令体使用白色。复用现有 boundary 检测逻辑（`last_boundary`），将判定从 `starts_with('/')` 扩展到 `starts_with('/') || starts_with('!')`，space 边界语义不变。

### /help 文档

`default_builtin_commands`（completion.rs）描述的是 `/` 命令。`!` 不是斜杠命令，不加入该列表；改为在 `/help` 输出末尾追加一行说明 `! <command> - Run a shell command directly and show its output`，使该能力可被发现。

### 边界与安全

- 与 Claude Code bash mode 一致：`!` 命令直接执行，**不**经过 permission/sandbox/hook 链路，**不**进入 conversation history。这是有意的「快速本地执行」语义。
- 不支持多行 `!` 命令：仅取第一行首字符判定，命令体为整段 `rest`（`sh -c` 天然支持 `&&` / `|` 等）。
- 不进入 `pending_inputs`、不触发 `start_next_turn`，因此不会与正在运行的 agent turn 冲突。

## 涉及文件

1. `src/tui/app/input.rs` - 新增 `!` 分支与执行逻辑。
2. `src/tui/components/input.rs` - `update_style` 扩展 `!` 高亮。
3. `src/tui/completion.rs` 或 `input.rs` 的 `/help` 分支 - 文档说明。

无需 delta spec：本变更不改变任何已有 spec 的验收场景，仅新增输入行为。
