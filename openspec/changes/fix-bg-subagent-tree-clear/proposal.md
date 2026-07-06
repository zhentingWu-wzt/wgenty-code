# Fix: 新 turn 清树抹掉 background subagent

## Why

`fix-statusbar-main-entry-and-lifecycle` 把清树从 `Submit` 移到 `TurnStarted`，修了**前台** subagent 场景（主 turn 运行中提交提示词 → 入队 → 树保留）。但**后台** subagent 场景仍有 bug：

`task` 工具的 background 模式（`src/tools/meta/task.rs:356-491`）通过 `tokio::spawn` 启动 subagent 并**立即返回**，subagent 独立运行，主 turn 不阻塞。主 turn 完成后 `current_turn_handle = None`。此时用户提交新提示词 → `submit_input` 见 `current_turn_handle.is_none()` → `start_next_turn` → `TurnStarted` → `subagent_tree.clear()`（`event.rs:753`）把仍在运行的 background subagent 从树里抹掉 → 状态栏消失 → 进不去 focus view。

用户报告：「子代理正在跑的时候，再提交提示词，subagent 的 selector 区域会消失，再也进不到 subagent 窗口了」。

## Root Cause

`TurnStarted` 的 `subagent_tree.clear()` 是**无条件**的。它假设所有 subagent 都属于上一 turn（前台，已完成）。但 background subagent 跨 turn 存活、仍 Running，被一并清掉。

- 前台 subagent：主 turn 阻塞等待 → 提交时入队、不开新 turn → 树保留（已修）。
- 后台 subagent：主 turn 立即返回并完成 → 提交时开新 turn → `TurnStarted` 清树 → background subagent 被抹（未修）。

证据：二进制已含上次修复（mtime 16:27 > 修复 commit 14:17），bug 仍在 → 不是 stale binary。全仓库仅 `event.rs:753`（TurnStarted）和 `:823`（TurnAborted）两处 `subagent_tree.clear()`，无其他清树路径。SubagentError 并行改动不触及 tree/status bar。

## Fix Goals

`TurnStarted` 清树前判断：若仍有活动（Running|Pending）subagent，**不清**（保留 background subagent 及相关 UI 状态）；仅当无活动 subagent 时才清树 + 重置 focus/selected/completed_at。

`TurnAborted`（/clear、turn 失败）保持全清——abort 时前台 subagent 可能卡在 Running（`cancel_current_turn` 不发 Cancelled 更新），全清才能移除僵尸节点；background subagent 会在下次 progress 更新时被 `SubagentUpdate::upsert` 重新加回（/clear 的瞬态闪烁可接受，且非本 bug）。

## Impact

- **代码**：
  - `src/tui/components/subagent_tree.rs`（新增 `clear_if_idle()` 方法 + 测试）。
  - `src/tui/app/event.rs`（`TurnStarted` 用 `clear_if_idle()`，仅在清树时重置 `completed_at`/`subagent_focus`/`subagent_status_bar_selected`）。
- **spec**：`subagent-status-display` 主 spec 已有 "Subagent tree lifecycle across submitted prompts" requirement（上次归档合并的 ADDED）。其中 "New turn start clears the tree" scenario 需微调为「仅在无活动 subagent 时清树，保留 background subagent」。创建 delta spec（MODIFIED）。
- **依赖/API**：无。
- **风险**：保留 background subagent 时，上一 turn 的已完成 subagent 也会保留在树里（不清），但它们被状态栏的 active 过滤 + focus view 的 completed-after-delay 过滤隐藏，UI 不受影响；树会累积，属轻微内存代价。
