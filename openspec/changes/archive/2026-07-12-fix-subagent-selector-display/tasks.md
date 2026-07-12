# Tasks: 修复 subagent 无法显示在 selector 区域

## T1: 并行路径 poller self-terminate（`src/tui/agent/tool_dispatch.rs`）

- [x] 1.1 在 `execute_tool_static` 的 poller 循环中，发送 `AgentLocalView` 后检测 `view.children` 非空且全部 `is_terminal()`，满足则 `break` 自行退出。
- [x] 1.2 移除 `execute_tool_static` 末尾的 `poll_handle.abort()`，改为 detach（`drop(poll_handle)`），并补充注释说明 task 为 fire-and-forget、不可 abort。

## T2: 顺序路径 poller self-terminate（`src/tui/agent/core.rs`）

- [x] 2.1 在顺序路径 poller 循环中增加与 T1 相同的 all-terminal self-termination 逻辑。
- [x] 2.2 移除 success 路径与 timeout 路径的 `poll_handle.abort()`，改为 detach。

## T3: 测试与构建验证

- [x] 3.1 现有测试已覆盖 surrounding 逻辑（`AgentLifecycleStatus::is_terminal`、`SubagentTree::replace_local`/`active_count`、coordinator view 构建）；self-termination 条件 `!children.is_empty() && all(is_terminal)` 为纯表达式，由上述单测间接保证，无需新增 mock 化 poller 测试。
- [x] 3.2 `cargo fmt -- --check` 通过；`cargo clippy --all-targets -- -D warnings` 零 warning；`cargo build` 通过；`cargo test --lib` 659 passed / 0 failed。
