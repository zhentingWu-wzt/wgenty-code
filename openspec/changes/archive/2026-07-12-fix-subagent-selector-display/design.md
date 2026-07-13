# Design: 修复 subagent 无法显示在 selector 区域

## 问题

`task` 工具为 fire-and-forget：`execute_with_context` 通过 `tokio::spawn` 在 daemon 侧异步启动子代理，立即返回 `{"status":"running"}`（`src/tools/meta/task.rs`）。TUI 的进度显示依赖 poller 每 500ms 调用 `get_root_agent_view` 并发送 `AppEvent::AgentLocalView`，驱动 `subagent_tree.replace_local` 更新 selector / status bar。

未提交的改动将 poller 管理从 `drop(poll_handle)`（detach）改为 `poll_handle.abort()`。由于 `task` 立即返回，`abort()` 在 poller 首次 500ms 轮询前将其终止，导致无任何 `AgentLocalView` 事件到达 TUI--子代理完全无法显示。

`abort()` 的初衷是防止上一批次的 stale poller 经 `replace_local` 覆盖当前 tree，但该问题已由 `AgentLocalView.generation` + handler 的 generation 校验解决（`/clear` 后 stale 视图被丢弃）。

## 方案

**Self-terminating poller**：poller 在每次发送视图后检测 `view.children` 非空且全部 `is_terminal()`，满足则 `break` 自行退出。工具返回时不再 `abort()`，改为 detach（`drop(poll_handle)`），让 poller 继续跟踪 fire-and-forget 子代理直至其完成。

### 修改点

1. `src/tui/agent/tool_dispatch.rs` `execute_tool_static`（task 并行路径）：
   - poller 循环增加 all-terminal self-termination。
   - 移除末尾 `abort()`，改为 `drop(poll_handle)` + 注释。

2. `src/tui/agent/core.rs` 顺序路径（delegate）：
   - poller 循环增加相同 self-termination。
   - success 路径与 timeout 路径的 `abort()` 均改为 `drop(poll_handle)`。

### 为什么不保留 abort

- `task` 立即返回 → abort 杀死 poller 早于首次轮询 → 子代理永不显示。
- `delegate` 阻塞 → abort 在工具返回后丢失最终 terminal 视图 → 已完成子代理卡在 Running。
- generation 校验已处理 `/clear` 后的 stale 覆盖；self-termination 将同 generation 跨 turn 的 stale 窗口缩到最小（子代理完成后即停）。

## 权衡

- **未采用 merge 代替 replace**：`replace_local` 全量重建 tree，理论上存在跨 turn stale 视图短暂覆盖的窗口，但 self-termination 使 poller 在子代理完成后立即停止，窗口极小且下一次轮询（500ms）即恢复。改成 merge 语义是更大重构，超出 hotfix 范围。
- **未新增 mock 化 poller 测试**：self-termination 条件为纯表达式 `!children.is_empty() && all(is_terminal)`，`is_terminal` / tree / coordinator 已有单测覆盖；poller 深度嵌入需 DaemonClient 的集成路径，mock 化成本超出 light-verify hotfix。

## 不变项

- `task` fire-and-forget 语义不变。
- `AgentLocalView.generation` 机制不变。
- `replace_local` tree 重建逻辑不变。
- daemon 侧 coordinator / view 构建不变。
