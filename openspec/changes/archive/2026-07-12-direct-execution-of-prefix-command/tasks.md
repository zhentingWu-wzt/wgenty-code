# Tasks: 直接执行 `!` 前缀命令

## T1: `submit_input` 新增 `!` 命令执行分支

- [x] 1.1 在 `src/tui/app/input.rs` 的 `submit_input` 最前面（所有 `/` 斜杠命令判定之前）新增 `!` 前缀检测：`trimmed.strip_prefix('!')`，对剩余部分 `trim_start` 得到命令体。
- [x] 1.2 命令体为空（纯 `!`）时 `push_system_message` 提示用法后 `return`。
- [x] 1.3 非空时 `tokio::spawn` 后台任务：克隆 `event_tx`，以 `sh -c` 在 `current_dir` 执行命令，120s 超时；格式化 `$ <cmd>` / stdout / `exit code N` / stderr 为单条消息，经 `AppEvent::BackgroundTaskResult` 回传。
- [x] 1.4 spawn 前先 `push_system_message` 一条「正在执行...」的占位系统消息（含命令），让用户立即得到反馈。
- [x] 1.5 该分支执行后 `return`，不进入 `/` 路由或 `pending_inputs`。

## T2: 输入框 `!` 高亮与 `/help` 文档

- [x] 2.1 `src/tui/components/input.rs` 的 `update_style`：将 `is_slash` 判定从 `starts_with('/')` 扩展为 `starts_with('/') || starts_with('!')`，使 `!` 前缀同样获得 `ACCENT` 强调色。
- [x] 2.2 `/help`（`input.rs` 的 `/help` 分支）输出末尾追加一行：`! <command> - Run a shell command directly and show its output`。

## T3: 测试与构建验证

- [x] 3.1 为 `!` 前缀判定与命令体解析补充单元测试（纯 `!`、`!ls`、`! ls`、`!echo hi && echo bye`）。
- [x] 3.2 运行 `cargo fmt`、`cargo clippy`、`cargo build`、相关 `cargo test`。
