# Proposal: 修复 subagent 无法显示在 selector 区域

## 动机

`task` 工具已改为 fire-and-forget 模式：调用时通过 `tokio::spawn` 在 daemon 侧异步启动子代理，并立即返回 `{"status":"running"}` 结构化确认（`src/tools/meta/task.rs`）。TUI 侧的进度依赖一个轮询任务（poller），每 500ms 调用 `get_root_agent_view` 获取子代理视图并通过 `AppEvent::AgentLocalView` 回传，驱动 `subagent_tree.replace_local` 更新 selector。

近期未提交的改动将 poller 的生命周期管理从 `drop(poll_handle)`（detach，让 poller 自行运行至超时）改为 `poll_handle.abort()`（工具返回时立即终止）。由于 `task` 工具立即返回，`abort()` 在 poller 首次 500ms 轮询之前就将其杀死，导致没有任何 `AgentLocalView` 事件到达 TUI——子代理完全无法出现在 selector / status bar 中。

## 根因

- `src/tui/agent/tool_dispatch.rs` `execute_tool_static`（task 并行路径）：`client.execute_tool("task")` 立即返回后 `poll_handle.abort()` 终止 poller。
- `src/tui/agent/core.rs` 顺序路径（delegate）：工具返回后同样 `abort()`，可能丢失最终 terminal 视图，使已完成的子代理卡在 Running。
- `abort()` 的初衷是防止上一批次的 stale poller 通过 `replace_local` 覆盖当前 tree，但该问题已由 `AgentLocalView` 携带的 `generation` 字段 + handler 的 generation 校验解决（`/clear` 后 stale 视图被丢弃）。

## 目标

- 恢复 fire-and-forget `task` 子代理在 selector / status bar 的实时显示。
- poller 在所有被轮询子代理达到 terminal 状态后自行退出（self-terminate），避免 1800s 长驻与跨 turn stale 覆盖。
- 保留 generation 校验（`/clear` 安全）。

## 范围

**包含**
- `src/tui/agent/tool_dispatch.rs`：poller 循环增加 all-terminal self-termination；移除 `abort()`，改为 detach。
- `src/tui/agent/core.rs`：顺序路径 poller 同样增加 self-termination；移除 success / timeout 路径的 `abort()`。

**不包含**
- 不改变 `task` 工具的 fire-and-forget 语义。
- 不改变 daemon 侧 coordinator / view 构建。
- 不引入新的 capability 或架构变更。

## 非目标 / 不改变

- 不改变 `AgentLocalView` 的 generation 机制。
- 不改变 `replace_local` 的 tree 重建逻辑。
- 不影响 `delegate` 的阻塞语义。
