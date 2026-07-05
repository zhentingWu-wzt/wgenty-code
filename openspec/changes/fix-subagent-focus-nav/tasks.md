# Tasks — fix-subagent-focus-nav

实现 subagent focus view 导航与选择器交互重做、完成态灰显+延迟移除、移除 "task" 包装节点、过滤 "delegate" 分组节点、状态栏 ↑↓ 自动激活、主聊天滚动简化、输入框样式修复。参考 `design.md`（D1–D12）与 `specs/subagent-focus-view/spec.md` + `specs/subagent-status-display/spec.md`（delta）。

> **注意**：工作区已有部分早期实现（`subagent_focus_view.rs` 的 main 条目 + `event.rs` 选择器导航/Enter 退出 + `input.rs` `update_style` 抽出 + 状态栏 ↑↓ 自动激活 + 主聊天 ↑↓ 滚动移除）。本计划在其上完成剩余任务，不重做已有进度。

## 1. 移除 "task" 包装节点（task.rs，D7）

- [x] 1.1 `src/tools/meta/task.rs`：移除 `root_node_id` 包装节点创建块（背景模式 + 同步模式两处）
- [x] 1.2 subagent 节点改为 `parent_id: None`（直接为 root）；`make_progress_callback` 调用 `parent_id` 改 `None`
- [x] 1.3 更新 `task.rs` 中断言包装节点存在的测试（若有）
- [x] 1.4 验证 `SubagentTree::upsert` 把首个 subagent 设为 root，`node_list()`/`active_count()` 不再含包装节点

## 2. SubagentTree 分组节点过滤 + status bar（D9）

- [x] 2.1 `src/tui/components/subagent_tree.rs`：新增 `is_grouping_node(id) = !children.is_empty() && events.is_empty() && messages.is_empty()`
- [x] 2.2 新增 `real_node_list()` = `node_list()` 过滤掉 `is_grouping_node` 的节点
- [x] 2.3 `active_count()`、`count_by_status()`、`active_node_ids()`、`total_count()` 改用 `real_node_list()` 或遍历时跳过分组节点
- [x] 2.4 `src/tui/components/subagent_status_bar.rs`：`active_node_ids` 走 `real_node_list`，修正 delegate 包装节点 stale "Running" 导致的计数虚高
- [x] 2.5 验证 delegate（rlm）1:N 分组：包装节点排除、sub-task 保留；`task` 残留包装节点也被兜底过滤

## 3. subagent_focus_view.rs 状态与渲染重构（D1–D6, D8）

- [ ] 3.1 移除 `FocusArea` enum 与 `FocusViewState::active_area` 字段（同步 `build`/`rebuild` 与所有引用）
- [ ] 3.2 `FocusViewState::build`：`selector_index` 初始化为当前 `node_id` 在 `real_node_list` 中的索引 +1（main 偏移）；找不到回退 0
- [ ] 3.3 重构 `build_selector_lines`：统一列表 `["main", ...visible_node_ids]` + 滑动窗口 `skip(scroll_start).take(available)`，`scroll_start` 由 `selector_index` 推导（D3 算法）；`visible_node_ids` 基于 `real_node_list` 再套完成态过滤；修掉 `take(available)` 越界
- [ ] 3.4 选择器布局高度 `Constraint::Length(6)` → `Constraint::Length(8)`
- [ ] 3.5 边框：selector 恒为 `active_border`，timeline 恒为 `inactive_border`（移除条件分支）
- [ ] 3.6 help bar 文案更新为单焦点版本（`↑↓ navigate · Enter switch/exit · t fold · Esc back · wheel scroll timeline`）
- [ ] 3.7 完成态灰显：Completed/Failed/Cancelled 的 subagent label 渲染为灰色，保留状态图标
- [ ] 3.8 延迟移除过滤：渲染时排除 `completed_at[node]` 超过 `COMPLETED_REMOVE_DELAY_SECS`（10s）的 node；当前 `node_id` 例外

## 4. app/mod.rs + event.rs 状态与 focus view 键位（D5, D8）

- [ ] 4.1 `app/mod.rs`：App 新增 `completed_at: HashMap<String, std::time::Instant>` 字段，初始化为空
- [ ] 4.2 `event.rs` `AppEvent::SubagentUpdate`：若 `progress.status` 为完成态且之前非完成态（transition 时刻），写入 `completed_at`
- [ ] 4.3 `event.rs` `AppEvent::Submit`（`subagent_tree.clear()` 处）：清空 `completed_at`
- [ ] 4.4 `event.rs` focus view 键位：移除 `FocusArea` 导入与所有 `active_area ==` 守卫
- [ ] 4.5 ↑↓ 改为导航选择器：基于可见列表 `wrap_prev`/`wrap_next`（含 main）
- [ ] 4.6 Enter：`selector_index == 0` 退出 focus view；否则 `FocusViewState::build(visible_list[idx-1])` 切换
- [ ] 4.7 `'t'` 折叠：移除 `active_area` 守卫，始终可用
- [ ] 4.8 Tab 删除显式分支（`_ => return` 兜底为 no-op）
- [ ] 4.9 鼠标滚轮：移除 `active_area == Timeline` 守卫，始终滚 timeline
- [ ] 4.10 移除 focus view 内 PageUp/PageDown 的 timeline 滚动分支

## 5. 状态栏 + 主聊天键位 + 输入框样式（D10–D12）

> 工作区已有这部分的部分实现，需核对完整性并补全。

- [ ] 5.1 `event.rs` 状态栏：核对移除 Tab 切换、↑↓ 自动激活 `subagent_status_bar_focused`（D10）；Esc 取消焦点；Enter 打开 focus view 保留
- [ ] 5.2 `event.rs` 主聊天：核对移除 ↑↓ 单行滚动分支（D11）；保留 PageUp/PageDn + 鼠标滚轮；无活跃 subagent 时 ↑↓ 不响应
- [ ] 5.3 `input.rs`：核对 `update_style()` 抽出完整（`render()` 不再设 slash-command 样式，只设边框）
- [ ] 5.4 `event.rs`：核对所有文本变更点调用 `update_style()`（按键输入 `textarea.input`、补全 `insert_str`、粘贴 `insert_char`、`take_text`、Shift+Enter）
- [ ] 5.5 help bar / 状态栏提示文案：反映 ↑↓ 自动激活、PageUp/PageDn 滚动（如适用）

## 6. 测试更新与新增

- [ ] 6.1 更新 `test_build_from_node`：`selector_index` 期望值改为 `pos+1`，移除 `active_area` 断言
- [ ] 6.2 更新 `test_rebuild_preserves_ui_state` 等涉及 `active_area`/`selector_index` 的断言
- [ ] 6.3 新增单测：`build` 时光标对齐当前 node（多 node tree，打开非根 node 验证 `selector_index`）
- [ ] 6.4 新增单测：选择器滚动跟随（cursor 接近底部时 `scroll_start` 跟随，cursor 始终在窗口内）
- [ ] 6.5 新增单测：`build_selector_lines` 不越界（main + N subagent，available 较小时总数 ≤ available）
- [ ] 6.6 新增单测：完成态灰显与延迟移除过滤（超时 node 被排除，当前 node 例外）
- [ ] 6.7 新增单测：`is_grouping_node` 过滤（delegate 包装排除、sub-task 保留；`active_count`/`active_node_ids` 不含分组节点）
- [ ] 6.8 新增/更新 `task.rs` 测试：确认不再创建包装节点，subagent 为 root
- [ ] 6.9 新增单测：状态栏 ↑↓ 自动激活焦点（无 Tab 前置），Esc 取消焦点
- [ ] 6.10 新增单测：主聊天 ↑↓ 不滚动（PageUp/PageDn 仍可滚）

## 7. 构建与验收

- [ ] 7.1 `cargo build` 通过
- [ ] 7.2 `cargo test` 通过（含新单测）
- [ ] 7.3 手动验收：按 `specs/subagent-focus-view/spec.md` + `specs/subagent-status-display/spec.md` 验收场景逐项核对（↑↓ 导航、Enter 切换/退出、鼠标滚 timeline、't' 折叠、Tab no-op、选择器滚动跟随、短终端、完成态灰显、延迟移除、task 包装节点不出现、delegate 分组节点不出现、active 计数正确、状态栏 ↑↓ 自动激活、Esc 取消焦点、主聊天 PageUp/PageDn 滚动、输入框不闪烁）
