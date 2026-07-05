# Comet Design Handoff

- Change: fix-subagent-focus-nav
- Phase: design
- Mode: compact
- Context hash: 7df3ace3a63a925f9119ecd4a9b999d75ec736d42f3d719ee705f75fd1198d4f

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/fix-subagent-focus-nav/proposal.md

- Source: openspec/changes/fix-subagent-focus-nav/proposal.md
- Lines: 1-51
- SHA256: c4cc7e970bdc3b7be6966aa701b65ea592a4e6e2684ec4a8d8a2dc8be8daf598

```md
## Why

Subagent focus view 当前用 Tab 在 timeline 与 selector 两个焦点区之间切换，且默认焦点在 timeline——用户按 ↑↓ 滚动的是 timeline 而非选择器，导致"选不到 main / 看不到 main"的体感。同时选择器高度固定为 6 行、无滚动跟随，subagent 较多时选中标记 `▶` 会移出可见区，且进入 focus view 时光标 `▶`（停在 main）与当前 `●`（被打开的 subagent）错位。此外两个内容问题：(a) `task` 工具每次会创建一个永不更新、无实质信息的 "task" 包装根节点，污染 selector 与 status bar 的 active 计数；(b) 完成的 subagent 在 selector 中长期堆积，无清理机制。本次重做 focus view 键位与选择器交互，并在源头移除包装节点、为 selector 增加完成态灰显与延迟移除。

## What Changes

### 键位与选择器交互（subagent-focus-view）

- **BREAKING**: 移除 `FocusArea`/`active_area` 双焦点机制，不再用 Tab 在 timeline 与 selector 之间切换。
- **BREAKING**: focus view 内 ↑↓ 改为导航选择器（main + subagents，wrap），不再滚动 timeline；timeline 滚动改为仅鼠标滚轮（移除 ↑↓/PageUp/PageDn 的 timeline 滚动）。
- 进入 focus view 时光标 `▶` 与当前 `●` 对齐——`FocusViewState::build` 把 `selector_index` 初始化为当前 `node_id` 在 `node_list` 中的索引（+1 main 偏移），而非硬编码 0。
- 选择器高度从 `Length(6)` 调整为 `Length(8)`，并增加"选中项始终可见"的滚动跟随逻辑。
- 修复 `build_selector_lines` 的 `take(available)` 越界。
- `'t'` 折叠工具调用的快捷键保留，但不再依赖 `active_area`（始终可用）。
- 视觉：选择器边框高亮（交互区），timeline 边框暗淡（只读）。
- `Esc` 退出、`Enter` 切换/返回 main 的行为保持不变；Tab 改为 no-op。

### 完成态灰显 + 延迟移除（subagent-focus-view）

- selector 中 Completed/Failed/Cancelled 的 subagent 灰显（label 变灰，保留状态图标）。
- App 维护 `HashMap<node_id, Instant>` 跟踪完成时刻；`Tick` 事件过滤 selector 中超过 N 秒（默认 10s）的完成项，使其不再出现在 selector（仍保留在 tree，当前正在查看的 node 不被过滤）。
- 导航 wrap 长度基于过滤后的可见列表，避免跳到不可见项。

### 移除 "task" 包装节点（agent-runtime → 影响 subagent-status-display / subagent-focus-view）

- `task` 工具不再创建 `root_node_id` 包装节点；直接让 subagent 节点为 root（`parent_id: None`），progress callback 更新该节点。
- 修复 status bar `active_count()` / `active_node_ids()` 因包装节点永久 "Running" 导致的计数虚高。
- selector 不再出现 "task:" 空条目。

## Capabilities

### New Capabilities
<!-- 无新增 capability -->

### Modified Capabilities
- `subagent-focus-view`: 导航与选择器交互变更（移除 Tab 双焦点、↑↓ 导航选择器、timeline 鼠标滚轮、滚动跟随、光标对齐、高度调整）；selector 完成态灰显与延迟移除；selector 不再显示 "task" 包装节点。

> 注：`subagent-status-display` 的 active 计数准确性是包装节点移除的实现层修复——现有 spec 已要求计数正确，无需 delta，仅需修正实现。

## Impact

- **代码**：
  - `src/tui/components/subagent_focus_view.rs`（渲染布局、选择器滚动、光标起点、完成态灰显、`FocusArea` 移除）。
  - `src/tui/app/event.rs`（focus view 键位重映射、完成时间跟踪、`Tick` 过滤）。
  - `src/tui/app/mod.rs`（App 新增完成时间 `HashMap` 字段）。
  - `src/tools/meta/task.rs`（移除包装根节点创建，subagent 直接为 root）。
- **传参**：`src/tui/app/render.rs` 仅传参，无逻辑变更。
- **测试**：`subagent_focus_view.rs` 现有单测断言需更新；新增滚动跟随/光标对齐/完成态过滤测试。`task.rs` 单测若断言包装节点需更新。
- **spec**：`openspec/specs/subagent-focus-view/spec.md` 与 `openspec/specs/subagent-status-display/spec.md` 需 delta。
- **依赖/API**：无外部 API、无依赖变更。
- **回归风险**：focus view 键位 breaking；包装节点移除改变 tree 结构（subagent 直接为 root），需确认 HTML 报告/transcript 不依赖包装节点（已核实：HTML 报告用 transcript store，不依赖内存 tree；transcript label 独立）。主聊天窗口键位不受影响。
```

## openspec/changes/fix-subagent-focus-nav/design.md

- Source: openspec/changes/fix-subagent-focus-nav/design.md
- Lines: 1-134
- SHA256: cc6496611b009c11b7cfca5eb4457234a9918e05527d9a4acd8f869d200b78c4

[TRUNCATED]

```md
## Context

`subagent-focus-view` 是 TUI 中全屏查看单个 subagent 执行时间线的视图。当前实现把交互分成两个焦点区（`FocusArea::Timeline` / `FocusArea::Selector`），默认焦点在 Timeline，Tab 切换。这带来三个问题：

1. **键位错配**：用户按 ↑↓ 期望切换 agent，实际滚动的是 timeline——必须先 Tab 才能操作选择器。
2. **选择器无滚动**：`build_selector_lines` 中 `scroll = 0` 写死，subagent 多时 `▶` 移出可见区；且 `take(available)` 在已 push "main" 后仍 take `available` 个，越界裁掉最后一项。
3. **光标错位**：`FocusViewState::build` 把 `selector_index` 硬设为 0（main），但 `node_id` 是被打开的 subagent，导致 `▶`（main）与 `●`（subagent）进入时不一致。

此外两个内容问题：

4. **"task" 包装节点**：`task` 工具（`src/tools/meta/task.rs`）每次调用创建 1 个包装根节点（`label: "task: <description>"`，`parent_id: None`，状态 Running）+ 1 个 subagent 子节点（1:1，无分组作用）。包装节点创建后**永不更新**（`make_progress_callback` 只更新子节点），永远卡在 "Running"、无 events/messages。它被 `node_list()`/`active_count()`/`active_node_ids()` 当作真实节点，导致 selector 出现空条目、status bar active 计数虚高。
5. **完成态堆积**：完成的 subagent 在 selector 中长期堆积，无清理机制，干扰寻找 active 项。

涉及文件：`src/tui/components/subagent_focus_view.rs`、`src/tui/components/subagent_tree.rs`、`src/tui/components/subagent_status_bar.rs`、`src/tui/app/event.rs`、`src/tui/app/mod.rs`、`src/tools/meta/task.rs`。

## Goals / Non-Goals

**Goals:**
- ↑↓ 直接导航选择器（main + subagents），无需 Tab。
- 进入 focus view 时光标 `▶` 与当前 `●` 对齐。
- 选择器选中项始终可见（滚动跟随），subagent 多时不再裁掉。
- timeline 滚动交给鼠标滚轮，键位简化。
- 修掉 `take(available)` 越界。
- selector 中完成态灰显，并在延迟后自动移除（declutter）。
- 源头移除 "task" 包装节点，修正 active 计数与 selector 噪声。
- TUI 层过滤 "delegate" 分组节点（1:N，不可源头移除），修正计数虚高与 selector 噪声。

**Non-Goals:**
- 不改 focus view 入口（状态栏 Enter 打开）。
- 不改四区布局（header / timeline / selector / help）。
- 不改 `node_list` DFS 顺序或嵌套 subagent 的 tree 结构。
- 不动主聊天窗口键位与滚动。
- 不重做 timeline 内容渲染（事件类型视觉区分保持）。
- 不改 HTML 报告 / transcript 存储（已确认不依赖内存 tree 包装节点）。

## Decisions

### D1: 移除 `FocusArea`/`active_area`，选择器为唯一交互区

删除 `FocusArea` enum 与 `FocusViewState::active_area` 字段。选择器是 focus view 内唯一可键盘交互的区域；timeline 变为只读视图（鼠标滚轮滚动）。双焦点的 Tab 切换是当前混乱根因；保留枚举但默认 Selector 仍留死代码（timeline ↑↓ 滚动本就要移除）。

### D2: `selector_index` 初始化对齐当前 `node_id`

`FocusViewState::build` 中，计算 `node_list` 中 `node_id` 的位置 `pos`，设 `selector_index = pos + 1`（+1 是 main 占用的 index 0）。找不到时回退 0。进入时 `▶` 与 `●` 重合，消除错位。

### D3: 选择器滚动跟随——统一列表 + 滑动窗口

`build_selector_lines` 重构为对统一列表 `["main", ...visible_node_ids]` 做 `skip(scroll_start).take(available)`，`scroll_start` 由 `selector_index` 推导：

```
if selector_index < scroll_start          { scroll_start = selector_index }
if selector_index >= scroll_start + avail { scroll_start = selector_index - avail + 1 }
scroll_start = scroll_start.min(list_len.saturating_sub(avail))
```

`scroll_start` 渲染时计算（纯函数 of `selector_index` + 列表长度 + `available`），不入状态。同时修掉 `take(available)` 越界。

### D4: 选择器高度 `Length(6)` → `Length(8)`

内层 6 行，可显示 main + 5 个 subagent，剩余靠 D3 滚动跟随。timeline 仍为 `Min(5)`。

### D5: 键位重映射

| 键 | 行为 |
|----|------|
| ↑ / ↓ | 导航选择器（wrap，含 main，基于过滤后可见列表） |
| Enter | 切换到 `▶` 所指 agent；`▶` 在 main 则退出 focus view |
| Esc | 退出 focus view（保留） |
| `t` | 折叠/展开工具调用（始终可用） |
| 鼠标滚轮 | 滚动 timeline |
| PageUp / PageDown | focus view 内不响应 |
| Tab | no-op（`_ => return` 兜底） |

### D6: 边框视觉——选择器高亮，timeline 暗淡

selector 边框恒为 `active_border`，timeline 边框恒为 `inactive_border`。help bar 文案同步更新为单焦点版本。

### D7: 源头移除 "task" 包装节点

`src/tools/meta/task.rs` 不再创建 `root_node_id` 包装节点。subagent 节点直接以 `parent_id: None` 创建，成为 tree root；`make_progress_callback` 用 `parent_id: None` 更新该节点。`SubagentTree::upsert` 现有逻辑（第一个 `parent_id: None` 设为 root）天然适配。
```

Full source: openspec/changes/fix-subagent-focus-nav/design.md

## openspec/changes/fix-subagent-focus-nav/tasks.md

- Source: openspec/changes/fix-subagent-focus-nav/tasks.md
- Lines: 1-59
- SHA256: 068052170413751ea8ba4e03a17abb5d94984ae187a3aca3470d0c8b47a0d3ed

```md
# Tasks — fix-subagent-focus-nav

实现 subagent focus view 导航与选择器交互重做、完成态灰显+延迟移除、移除 "task" 包装节点、过滤 "delegate" 分组节点。参考 `design.md`（D1–D9）与 `specs/subagent-focus-view/spec.md`（delta）。

## 1. 移除 "task" 包装节点（task.rs，D7）

- [ ] 1.1 `src/tools/meta/task.rs`：移除 `root_node_id` 包装节点创建块（背景模式 + 同步模式两处）
- [ ] 1.2 subagent 节点改为 `parent_id: None`（直接为 root）；`make_progress_callback` 调用 `parent_id` 改 `None`
- [ ] 1.3 更新 `task.rs` 中断言包装节点存在的测试（若有）
- [ ] 1.4 验证 `SubagentTree::upsert` 把首个 subagent 设为 root，`node_list()`/`active_count()` 不再含包装节点

## 2. SubagentTree 分组节点过滤 + status bar（D9）

- [ ] 2.1 `src/tui/components/subagent_tree.rs`：新增 `is_grouping_node(id) = !children.is_empty() && events.is_empty() && messages.is_empty()`
- [ ] 2.2 新增 `real_node_list()` = `node_list()` 过滤掉 `is_grouping_node` 的节点
- [ ] 2.3 `active_count()`、`count_by_status()`、`active_node_ids()`、`total_count()` 改用 `real_node_list()` 或遍历时跳过分组节点
- [ ] 2.4 `src/tui/components/subagent_status_bar.rs`：`active_node_ids` 走 `real_node_list`，修正 delegate 包装节点 stale "Running" 导致的计数虚高
- [ ] 2.5 验证 delegate（rlm）1:N 分组：包装节点排除、sub-task 保留；`task` 残留包装节点也被兜底过滤

## 3. subagent_focus_view.rs 状态与渲染重构（D1–D6, D8）

- [ ] 3.1 移除 `FocusArea` enum 与 `FocusViewState::active_area` 字段（同步 `build`/`rebuild` 与所有引用）
- [ ] 3.2 `FocusViewState::build`：`selector_index` 初始化为当前 `node_id` 在 `real_node_list` 中的索引 +1（main 偏移）；找不到回退 0
- [ ] 3.3 重构 `build_selector_lines`：统一列表 `["main", ...visible_node_ids]` + 滑动窗口 `skip(scroll_start).take(available)`，`scroll_start` 由 `selector_index` 推导（D3 算法）；`visible_node_ids` 基于 `real_node_list` 再套完成态过滤
- [ ] 3.4 选择器布局高度 `Constraint::Length(6)` → `Constraint::Length(8)`
- [ ] 3.5 边框：selector 恒为 `active_border`，timeline 恒为 `inactive_border`（移除条件分支）
- [ ] 3.6 help bar 文案更新为单焦点版本（`↑↓ navigate · Enter switch/exit · t fold · Esc back · wheel scroll timeline`）
- [ ] 3.7 完成态灰显：Completed/Failed/Cancelled 的 subagent label 渲染为灰色，保留状态图标
- [ ] 3.8 延迟移除过滤：渲染时排除 `completed_at[node]` 超过 `COMPLETED_REMOVE_DELAY_SECS`（10s）的 node；当前 `node_id` 例外

## 4. app/mod.rs + event.rs 状态与键位（D5, D8）

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

## 5. 测试更新与新增

- [ ] 5.1 更新 `test_build_from_node`：`selector_index` 期望值改为 `pos+1`，移除 `active_area` 断言
- [ ] 5.2 更新 `test_rebuild_preserves_ui_state` 等涉及 `active_area`/`selector_index` 的断言
- [ ] 5.3 新增单测：`build` 时光标对齐当前 node（多 node tree，打开非根 node 验证 `selector_index`）
- [ ] 5.4 新增单测：选择器滚动跟随（cursor 接近底部时 `scroll_start` 跟随，cursor 始终在窗口内）
- [ ] 5.5 新增单测：`build_selector_lines` 不越界（main + N subagent，available 较小时总数 ≤ available）
- [ ] 5.6 新增单测：完成态灰显与延迟移除过滤（超时 node 被排除，当前 node 例外）
- [ ] 5.7 新增单测：`is_grouping_node` 过滤（delegate 包装排除、sub-task 保留；`active_count`/`active_node_ids` 不含分组节点）
- [ ] 5.8 新增/更新 `task.rs` 测试：确认不再创建包装节点，subagent 为 root

## 6. 构建与验收

- [ ] 6.1 `cargo build` 通过
- [ ] 6.2 `cargo test` 通过（含新单测）
- [ ] 6.3 手动验收：按 `specs/subagent-focus-view/spec.md` 验收场景逐项核对（↑↓ 导航、Enter 切换/退出、鼠标滚 timeline、't' 折叠、Tab no-op、选择器滚动跟随、短终端、完成态灰显、延迟移除、task 包装节点不出现、delegate 分组节点不出现、active 计数正确）
```

## openspec/changes/fix-subagent-focus-nav/specs/subagent-focus-view/spec.md

- Source: openspec/changes/fix-subagent-focus-nav/specs/subagent-focus-view/spec.md
- Lines: 1-92
- SHA256: 59687267cbe64a500ae7d20050f971f4ff701e84b1d195d3ebe01e6ddbd55264

[TRUNCATED]

```md
## MODIFIED Requirements

### Requirement: Focus view navigation and exit
The focus view SHALL use the subagent selector as the sole keyboard-interactive area: ↑↓ SHALL navigate the selector (the "main" entry plus all visible subagents) and Enter SHALL switch the displayed subagent or exit to the main chat. The event timeline SHALL be read-only, scrollable only via mouse wheel. The focus view SHALL return to the main chat layout when the user presses Esc or selects the "main" entry and presses Enter.

#### Scenario: Arrow keys navigate the selector
- **WHEN** the focus view is open
- **THEN** ↑↓ SHALL move the selector cursor (▶) among the "main" entry and all visible subagents, wrapping at both ends
- **AND** ↑↓ SHALL NOT scroll the event timeline

#### Scenario: Enter switches subagent or exits to main
- **WHEN** the selector cursor is on a subagent and the user presses Enter
- **THEN** the focus view SHALL switch to display that subagent's event timeline
- **AND** the timeline scroll position SHALL reset to the latest event
- **WHEN** the selector cursor is on the "main" entry and the user presses Enter
- **THEN** the TUI SHALL close the focus view and restore the main chat layout

#### Scenario: Timeline scrolls only via mouse wheel
- **WHEN** the focus view is open and the event timeline exceeds the visible area
- **THEN** mouse wheel SHALL scroll the timeline (ScrollUp toward older, ScrollDown toward newer)
- **AND** PageUp/PageDown SHALL have no effect inside the focus view

#### Scenario: Fold toggle is always available
- **WHEN** the user presses `t` inside the focus view
- **THEN** the focus view SHALL toggle fold/expand of tool calls in the timeline
- **AND** this behavior SHALL NOT depend on any focus area

#### Scenario: Tab is a no-op
- **WHEN** the user presses Tab inside the focus view
- **THEN** no focus toggle SHALL occur; Tab SHALL be a no-op

#### Scenario: Exiting focus view returns to main chat
- **WHEN** the user presses Esc while in the focus view
- **THEN** the TUI SHALL close the focus view and restore the main chat + input layout
- **AND** the input box SHALL regain focus for text entry
- **AND** the subagent status bar SHALL remain visible if subagents are still running

### Requirement: Focus view subagent selector bar
The focus view SHALL include a subagent selector bar at the bottom of the screen, listing a "main" entry (for returning to the main chat) followed by all real subagents (active and completed) with their status icons and labels. Placeholder/wrapper nodes that carry no execution information SHALL NOT appear in the selector — this includes both 1:1 wrapper nodes (e.g., a "task:" entry wrapping a single subagent) and 1:N grouping nodes (e.g., a "delegate:" entry that groups several sub-tasks but has no events or messages of its own). Grouping nodes SHALL be excluded from the selector, from active/total counts, and from the active-node list used by the status bar. The selector allows direct switching between subagent views without returning to the main chat. The selector SHALL be the sole keyboard-interactive area in the focus view.

#### Scenario: Selector shows main entry plus real subagents only
- **WHEN** the focus view is open and there are 3 real subagents (1 running, 2 completed)
- **THEN** the selector SHALL display a "main" entry at index 0 followed by exactly those 3 subagents with their status icons and labels
- **AND** the selector SHALL NOT display any placeholder/wrapper node (e.g., a "task:" entry with no events)
- **AND** the selector SHALL be at least 8 rows tall (including borders) so that the "main" entry plus several subagents are visible without scrolling

#### Scenario: Grouping nodes are excluded from selector and counts
- **WHEN** a `delegate` invocation creates a 1:N grouping node (e.g., "delegate: ...") with up to 8 child sub-task nodes, and the grouping node has no events or messages of its own
- **THEN** the selector SHALL NOT display the grouping node
- **AND** the selector SHALL display the child sub-task nodes as real subagents
- **AND** the active count, total count, and active-node list used by the status bar SHALL exclude the grouping node so its stale "Running" status does not inflate counts
- **AND** the grouping node SHALL remain in the tree as a parent for its children, only filtered from display and counts

#### Scenario: Cursor aligns with current subagent on entry
- **WHEN** the user opens the focus view for subagent "explore" from the status bar
- **THEN** the selector cursor (▶) SHALL start on "explore" (the currently viewed subagent)
- **AND** the current-view marker (●) SHALL also be on "explore", so ▶ and ● are aligned on entry

#### Scenario: Selector scrolls to keep cursor visible
- **WHEN** there are more subagents than visible selector rows and the user navigates the cursor past the bottom of the visible window
- **THEN** the selector SHALL scroll so the cursor remains visible
- **AND** the "main" entry MAY scroll out of view when the cursor is far down the list

#### Scenario: Selector wraps around including main
- **WHEN** the user navigates past the last visible subagent in the selector with ↓
- **THEN** the selection SHALL wrap around to the "main" entry (index 0)
- **AND** navigating past the "main" entry with ↑ SHALL wrap to the last visible subagent

#### Scenario: Selector distinguishes cursor from current view
- **WHEN** the focus view is displaying subagent "explore" and the cursor is on a different subagent
- **THEN** the selector SHALL visually distinguish the currently viewed subagent (● marker) from the cursor position (▶ marker)

#### Scenario: Selector border indicates interactive area
- **WHEN** the focus view is open
- **THEN** the selector border SHALL be highlighted (active color) to indicate it is the interactive area
- **AND** the timeline border SHALL be dimmed to indicate it is read-only

#### Scenario: Completed subagents are dimmed
- **WHEN** a subagent in the selector has completed (Completed, Failed, or Cancelled) and is still within the removal window
- **THEN** the selector SHALL render that subagent's label in a dimmed color while keeping its status icon
```

Full source: openspec/changes/fix-subagent-focus-nav/specs/subagent-focus-view/spec.md

