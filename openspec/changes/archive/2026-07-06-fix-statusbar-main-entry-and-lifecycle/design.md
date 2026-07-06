# Design: 状态栏 main 占位 + submit 清树生命周期

## 方案

两个问题，三处源码改动 + 一个 delta spec。

### 修复 1：状态栏 selector 增加 "main" 占位

#### 1a. render（`subagent_status_bar.rs`）

`render` 在活动 subagent 列表前插入 "main" 条目（index 0）。统一列表 = `["main", ...active]`，长度 N+1（N = active 数）。

- "main" 行：`▶ ` 选中标记 + `"main"` label（选中时高亮色，否则暗灰）。
- subagent 行：保持现有图标 + label + detail，但 `is_selected` 判定改为 `abs_index == selected`（abs_index = i+1）。
- `selected_index` 仍是 `usize`，但现在索引统一列表（0 = main, 1..N = subagent）。
- wrap / 选中标记用 `selected_index % (N+1)`。

#### 1b. 高度（`render.rs`）

`status_bar_layout_height(active_count)` 从 `active_count.min(5) + 1`（subagents + border）改为 `active_count.min(5) + 2`（main + subagents + border）。

| active | 旧高度 | 新高度 | 可见内容 |
|---|---|---|---|
| 1 | 2 | 3 | main + 1 subagent |
| 3 | 4 | 5 | main + 3 subagents |
| 5 | 6 | 7 | main + 5 subagents（全显） |
| 6 | 6 | 7 | main + 5 subagents（第 6 裁切） |

封顶：visible subagents ≤ 5（保持 spec "capped at 5 lines" 对 subagent 行数的解读），main 始终显示。

#### 1c. 导航 + Enter（`event.rs` 状态栏处理块，306-355）

```rust
let active = active_node_ids(&self.subagent_tree);
if !active.is_empty() {
    let len = active.len() + 1; // +1 for "main"
    if key.code == KeyCode::Up || key.code == KeyCode::Down {
        self.subagent_status_bar_focused = true;
    }
    if self.subagent_status_bar_focused {
        match key.code {
            KeyCode::Up => {
                self.subagent_status_bar_selected = wrap_prev(self.subagent_status_bar_selected, len);
                return;
            }
            KeyCode::Down => {
                self.subagent_status_bar_selected = wrap_next(self.subagent_status_bar_selected, len);
                return;
            }
            KeyCode::Enter => {
                if self.subagent_status_bar_selected == 0 {
                    // "main" selected — dismiss status bar focus (consistent
                    // with focus view's "main" exit semantics)
                    self.subagent_status_bar_focused = false;
                } else if let Some(node_id) = active.get(self.subagent_status_bar_selected - 1) {
                    if let Some(state) = FocusViewState::build(node_id, &self.subagent_tree) {
                        self.subagent_focus = Some(state);
                    }
                }
                return;
            }
            // Esc / Tab / other: unchanged
            ...
        }
    }
}
```

关键变化：
- wrap 长度 `active.len() + 1`。
- `Enter` 分支：`selected == 0`（main）→ 取消焦点；`selected >= 1` → `active.get(selected - 1)` 取 subagent（与 focus view selector 的 +1 偏移一致）。
- `subagent_status_bar_selected` 初始 0 = main（新 turn 重置后默认选中 main，非 subagent）。

#### 1d. `subagent_status_bar_selected` 越界保护

`render` 用 `selected_index % (N+1)` 防越界；`active.get(selected - 1)` 在 `selected == 0` 时短路。selected 不会超过 `N`（wrap 限制），但跨 turn 的 N 变化时，`% (N+1)` 保证安全。

### 修复 2：清树生命周期

`event.rs`：

- **`Submit` 处理器**：移除 `subagent_tree.clear()` / `completed_at.clear()` / `subagent_focus = None` / `subagent_status_bar_selected = 0`。只调 `submit_input(text)`。
- **`TurnStarted` 处理器**：加入清树 + 重置（新 turn 刷新）：
  ```rust
  AppEvent::TurnStarted { .. } => {
      self.subagent_tree.clear();
      self.completed_at.clear();
      self.subagent_focus = None;
      self.subagent_status_bar_selected = 0;
      self.turn_started_at = Some(std::time::Instant::now());
  }
  ```
- **`TurnAborted` 处理器**：加入清树 + 重置（覆盖 /clear 的 `cancel_current_turn` → `TurnAborted`，以及 turn 失败）：
  ```rust
  AppEvent::TurnAborted { ref reason } => {
      // ... existing Stop hook + last_abort_reason ...
      self.subagent_tree.clear();
      self.completed_at.clear();
      self.subagent_focus = None;
      self.subagent_status_bar_selected = 0;
      self.turn_started_at = None;
  }
  ```

#### 时机正确性

| 场景 | 旧行为 | 新行为 |
|---|---|---|
| 提交提示词，无 turn 运行 | Submit 清树 → start_next_turn | submit_input → start_next_turn → TurnStarted 清树 |
| 提交提示词，turn 运行中 | **Submit 清树（bug）** → 入队 | 入队，**树保留**；turn 完成后 TurnComplete 快照 → start_next_turn → TurnStarted 清树 |
| /clear | Submit 清树 → cancel_current_turn | cancel_current_turn → TurnAborted 清树 |
| turn 失败 | Submit 清树（下次提交时） | TurnAborted 清树（即时） |
| turn 正常完成 | TurnComplete 快照（树保留）→ 下次 Submit 清 | TurnComplete 快照 → start_next_turn → TurnStarted 清 |

`TurnComplete` 的 `subagent_history.insert(snapshot)` 在 `start_next_turn` 之前，快照不受 TurnStarted 清树影响。

### delta spec（`subagent-status-display`）

MODIFIED Requirements：
- "Status bar shows each active subagent" → 增加 "main" 占位在最前。
- "Arrow keys auto-activate and navigate" → wrap 长度含 main（N+1）。
- "Status bar Enter triggers focus view" → Enter on main 取消焦点（新增子场景）。

focus view selector 已有 "main"，`subagent-focus-view` 无需 delta。

### 不做的事

- **不动 focus view selector**：已有 "main" + 滑动跟随，无需改。
- **不动 `cancel_current_turn` 发 Cancelled 更新**：TurnAborted 清树已覆盖 /clear；让 cancel 传播 Cancelled 是 agent runtime 改动，超出范围。
- **不改 `subagent_history` 快照逻辑**：TurnComplete 快照时机不变。

## 验证策略

- **单元测试**：
  - `render.rs::status_bar_layout_height` 更新断言（+1 for main）：`1→3`、`3→5`、`5→7`、`6→7`、`0→0`。
  - `subagent_status_bar.rs` 新增 render 测试：验证 "main" 行存在、选中态、subagent 行偏移。
  - `event.rs` 状态栏导航：可用纯函数抽取 wrap + selected 逻辑测试（若不易测，靠 render 测试 + 手动验收）。
- **编译 + 既有测试**：`cargo build` + `cargo test --lib` 不回归。
- **手动验收**（verify 阶段）：触发 subagent 后状态栏显示 main + subagent；↑↓ 在 main/subagent 间 wrap；Enter on main 取消焦点；Enter on subagent 开 focus view；运行中提交新提示词状态栏不消失。
- **delta spec → full verification**。
