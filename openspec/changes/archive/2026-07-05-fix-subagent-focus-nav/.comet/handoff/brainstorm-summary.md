# Brainstorm Summary

- Change: fix-subagent-focus-nav
- Date: 2026-07-05

## 确认的技术方案

### 方向 1：focus view 键位与选择器交互（D1–D6）

- D1：移除 `FocusArea`/`active_area`，选择器为唯一交互区，timeline 只读。
- D2：`FocusViewState::build` 把 `selector_index` 初始化为当前 `node_id` 在 `node_list` 的索引 +1（main 偏移），找不到回退 0——进入时 ▶ 与 ● 对齐。
- D3：`build_selector_lines` 重构为统一列表 `["main", ...visible_node_ids]` + 滑动窗口 `skip(scroll_start).take(available)`；`scroll_start` 渲染时由 `selector_index` 推导（纯函数），不入状态；顺带修 `take(available)` 越界。
- D4：选择器高度 `Length(6)` → `Length(8)`。
- D5：键位 — ↑↓ 导航选择器（wrap，基于可见列表）；Enter 切换/退出；Esc 退出；'t' 折叠（始终可用）；鼠标滚轮滚 timeline；PageUp/PageDn 不响应；Tab no-op。
- D6：边框 — selector 恒亮，timeline 恒暗；help bar 文案同步。

### 方向 2：完成态灰显 + 延迟移除（D8）

- App 新增 `completed_at: HashMap<String, std::time::Instant>`。
- `SubagentUpdate`：状态转为 Completed/Failed/Cancelled 时写入 `completed_at`（仅 transition 时刻）。
- `Submit`（`subagent_tree.clear()`）清空 `completed_at`。
- 可见列表排除 `completed_at[node]` 超过 `COMPLETED_REMOVE_DELAY_SECS`（常量 10s）的 node；当前 `node_id` 例外。
- 完成但未超时的 node 灰显（label 灰色，保留状态图标）。
- 导航 wrap 长度基于可见列表 +1（main）。
- 渲染循环每个 Tick（100ms）触发 `terminal.draw`，过滤在渲染时计算 → 延迟移除 ~100ms 精度生效。

### 方向 3：移除包装节点（D7 task + D9 delegate）

- D7（task）：`task.rs` 源头不创建 `root_node_id` 包装节点；subagent 直接 `parent_id: None` 为 root；callback `parent_id: None`。task 是 1:1，移除安全。已核实 HTML 报告/transcript 不依赖内存 tree 包装节点。
- D9（delegate）：`delegate`（rlm）是 1:N 分组（最多 8 sub-task 挂 root），不能源头移除。在 `SubagentTree` 层加 `is_grouping_node(id) = !children.is_empty() && events.is_empty() && messages.is_empty()`，从 `real_node_list()`、`active_count()`、`count_by_status()`、`active_node_ids()`、`total_count()` 排除分组节点。修正 delegate 包装节点 stale "Running" 导致的计数虚高与 selector 噪声。
- 可见列表 = `real_node_list()` 再套完成态过滤。

## 关键取舍与风险

- **Breaking 键位**：移除 Tab、timeline 键盘滚动 → help bar 文案 + Tab no-op 平滑迁移。
- **短终端**：selector Length(8) + 滚动跟随；极端短不专门处理，但 `available>0` 时 main 至少 1 行可见。
- **delegate 过滤判据**：`is_grouping_node` 用"有子节点 + 无 events/messages"。Pending leaf（刚启动无 events）不会被误过滤（无子节点）。稳健。
- **嵌套 subagent**：`task` 工具对超 max_depth 过滤掉 `task` 本身；`upsert` 只在 root_id 为 None 时设 root。移除 task 包装后首个 subagent 为 root，后续 task 调用的 root 不可见——与现状一致，不引入回归。
- **完成时间跨会话**：`completed_at` 在 App 层，退出再进入 focus view 仍生效；新 turn 清空。
- **当前 node 例外**：延迟移除不删正在查看的 node，避免 timeline 消失。
- **计数语义变更**：`active_count()` 等排除分组节点是 bug 修正（分组节点本不该计入），改语义是正确的。

## 测试策略

- 更新 `test_build_from_node` 等：`selector_index` 期望 `pos+1`，移除 `active_area` 断言。
- 新增：光标对齐（多 node 打开非根）、滚动跟随（cursor 在窗口内）、`build_selector_lines` 不越界、完成态灰显+延迟移除过滤（超时排除、当前 node 例外）、`is_grouping_node` 过滤（delegate 包装排除、sub-task 保留）、`active_count` 不含分组节点。
- `task.rs` 测试：确认不再创建包装节点，subagent 为 root。
- 手动验收按 spec 验收场景。

## Spec Patch

- `specs/subagent-focus-view/spec.md` 已含包装节点不显示、完成态灰显、延迟移除、跨会话持久、新 turn 重置场景。**待补**：明确"分组节点"定义场景（delegate 包装节点也不显示）——将在 Design Doc 确认后回写 delta spec 补充 scenario。
- `subagent-status-display`：active 计数排除分组节点是实现修复，现有 spec 已要求计数正确，无需 delta。

## 涉及文件

- `src/tui/components/subagent_focus_view.rs`（渲染、选择器、灰显、`FocusArea` 移除）
- `src/tui/components/subagent_tree.rs`（新增 `is_grouping_node`、`real_node_list`，count 方法过滤）
- `src/tui/components/subagent_status_bar.rs`（`active_node_ids` 用 `real_node_list` 或过滤）
- `src/tui/app/event.rs`（键位、`completed_at` 写入/清空、可见列表导航）
- `src/tui/app/mod.rs`（App `completed_at` 字段）
- `src/tools/meta/task.rs`（移除包装节点）
- 测试：上述各文件 + `task.rs`
