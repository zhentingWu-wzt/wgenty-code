# Design: 新 turn 清树抹掉 background subagent

## 方案

`TurnStarted` 改为「空闲才清」——仅当无活动 subagent 时清树 + 重置；有活动（background）subagent 时保留全部状态。

### 1. `SubagentTree::clear_if_idle()`

`src/tui/components/subagent_tree.rs` 新增：

```rust
/// Clear the tree only when no subagents are still active (Running/Pending).
/// Background subagents (task tool background mode) outlive the main turn and
/// must remain visible/selectable across turn boundaries. Returns true if the
/// tree was cleared.
pub fn clear_if_idle(&mut self) -> bool {
    if self.active_count() == 0 {
        self.clear();
        true
    } else {
        false
    }
}
```

`active_count()` 已存在（Running + Pending 计数，走 `real_node_list`，排除 grouping 节点）。

### 2. `TurnStarted` 用 `clear_if_idle`

`src/tui/app/event.rs`：

```rust
AppEvent::TurnStarted { .. } => {
    // Fresh turn. Only clear the previous turn's subagent state when no
    // subagents are still active — background subagents (task background
    // mode) outlive the main turn and must stay visible/selectable.
    if self.subagent_tree.clear_if_idle() {
        self.completed_at.clear();
        self.subagent_focus = None;
        self.subagent_status_bar_selected = 0;
    }
    self.turn_started_at = Some(std::time::Instant::now());
}
```

`clear_if_idle()` 返回 true（已清）→ 重置 `completed_at`/`subagent_focus`/`selected`。返回 false（有 background subagent，保留）→ 跳过重置，保留用户当前 focus/selection。

### 3. `TurnAborted` 不变

保持 `subagent_tree.clear()` 全清。理由：`cancel_current_turn`（/clear）与 turn 失败时，前台 subagent 可能卡在 Running（不发 Cancelled 更新），全清才能移除僵尸；background subagent 会在下次 `SubagentUpdate::upsert` 重新加回（/clear 瞬态闪烁可接受，非本 bug）。

## 时机正确性

| 场景 | 旧行为（TurnStarted 无条件清） | 新行为（clear_if_idle） |
|---|---|---|
| 前台 subagent 运行中提交 | 入队，不开新 turn（TurnStarted 不触发） | 同（不入队 TurnStarted） |
| 后台 subagent 运行中提交 | **TurnStarted 清树，抹掉 background subagent（bug）** | clear_if_idle 返回 false，保留 background subagent + 状态栏 |
| 无 subagent 时提交 | TurnStarted 清空树（空操作）| clear_if_idle 返回 true，清树 + 重置 |
| 上一 turn 全完成后再提交 | TurnStarted 清掉已完成 subagent | clear_if_idle 返回 true（无 active），清树 + 重置 |
| /clear | TurnAborted 全清 | 不变（全清） |

## delta spec

`subagent-status-display` 主 spec（上次归档合并的 ADDED requirement "Subagent tree lifecycle across submitted prompts"）的 scenario "New turn start clears the tree" 需 MODIFIED：改为「新 turn 开始时，仅当无活动 subagent 才清树；有 background subagent 时保留」。

## 验证策略

- **单元测试**（`subagent_tree.rs`）：
  - `clear_if_idle` 有 Running subagent → 返回 false，树保留。
  - `clear_if_idle` 只有 Completed subagent → 返回 true，树清空。
  - `clear_if_idle` 空树 → 返回 true。
- **编译 + 既有测试**：`cargo build` + `cargo test --lib` 不回归。
- **手动验收**（verify）：后台 subagent 运行中提交新提示词 → 状态栏不消失，仍能 Enter 进 focus view。
