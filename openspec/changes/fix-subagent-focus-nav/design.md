## Context

`subagent-focus-view` 是 TUI 中全屏查看单个 subagent 执行时间线的视图。当前实现把交互分成两个焦点区（`FocusArea::Timeline` / `FocusArea::Selector`），默认焦点在 Timeline，Tab 切换。这带来三个问题：

1. **键位错配**：用户按 ↑↓ 期望切换 agent，实际滚动的是 timeline——必须先 Tab 才能操作选择器。
2. **选择器无滚动**：`build_selector_lines` 中 `scroll = 0` 写死，subagent 多时 `▶` 移出可见区；且 `take(available)` 在已 push "main" 后仍 take `available` 个，越界裁掉最后一项。
3. **光标错位**：`FocusViewState::build` 把 `selector_index` 硬设为 0（main），但 `node_id` 是被打开的 subagent，导致 `▶`（main）与 `●`（subagent）进入时不一致。

此外两个内容问题：

4. **"task" 包装节点**：`task` 工具（`src/tools/meta/task.rs`）每次调用创建 1 个包装根节点（`label: "task: <description>"`，`parent_id: None`，状态 Running）+ 1 个 subagent 子节点（1:1，无分组作用）。包装节点创建后**永不更新**（`make_progress_callback` 只更新子节点），永远卡在 "Running"、无 events/messages。它被 `node_list()`/`active_count()`/`active_node_ids()` 当作真实节点，导致 selector 出现空条目、status bar active 计数虚高。
5. **完成态堆积**：完成的 subagent 在 selector 中长期堆积，无清理机制，干扰寻找 active 项。

涉及文件：`src/tui/components/subagent_focus_view.rs`、`src/tui/components/subagent_tree.rs`、`src/tui/components/subagent_status_bar.rs`、`src/tui/components/input.rs`、`src/tui/app/event.rs`、`src/tui/app/mod.rs`、`src/tools/meta/task.rs`。

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
- 状态栏 ↑↓ 自动激活并导航（移除 Tab 焦点切换），降低进入 focus view 的键位摩擦。
- 主聊天滚动简化为 PageUp/PageDn + 鼠标滚轮，↑↓ 让位给状态栏导航。
- `InputBox` 抽出 `update_style()`，修复频繁重渲染下输入框闪烁/消失的视觉故障。

**Non-Goals:**
- 不改 focus view 入口（状态栏 Enter 打开）。
- 不改四区布局（header / timeline / selector / help）。
- 不改 `node_list` DFS 顺序或嵌套 subagent 的 tree 结构。
- 不改主聊天窗口的文本输入与提交行为（仅简化滚动/导航键位）。
- 不重做 timeline 内容渲染（事件类型视觉区分保持）。
- 不改 HTML 报告 / transcript 存储（已确认不依赖内存 tree 包装节点）。
- 不把 `REMOVE_DELAY` 暴露为设置项（YAGNI）。

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

**理由**：包装节点 1:1 包裹 subagent、永不更新、不被 HTML 报告/transcript 依赖，是无信息的噪声节点。源头移除一举修正：selector 噪声条目、`active_count()` 虚高、`active_node_ids()` 误含。比 TUI 层启发式过滤（脆弱、不修计数）更彻底。

**已核实**：HTML 报告（`subagent_trace.rs`）不读内存 tree；transcript label `"task: <description>"`（`task/transcript.rs`）是持久化记录的独立 label，与内存节点无关，不受影响。

**嵌套 subagent**：`task` 工具对超过 `max_depth` 的子任务过滤掉 `task` 工具本身，且 `upsert` 只在 `root_id` 为 None 时设 root。移除包装后，首个 subagent 成为 root；后续 `task` 调用创建的新 root 因 `root_id` 已设而不可见——这是与现状一致的预存行为，本次不引入回归。

### D8: 完成态灰显 + 延迟移除（TUI 侧跟踪）

**状态**：App 新增 `completed_at: HashMap<String, std::time::Instant>`，记录每个 subagent node 完成时刻。

**写入时机**：`AppEvent::SubagentUpdate(progress)` 中，若 `progress.status` 是 Completed/Failed/Cancelled 且该 node 之前非完成态，写入 `completed_at`。`subagent_tree.clear()`（新 turn）时清空。

**可见列表计算**：focus view selector 渲染与导航基于"可见列表"——main + 所有 subagent，但**排除**满足 `completed_at[node]` 存在且 `elapsed > REMOVE_DELAY`（默认 10s）的 node；**当前 `node_id`（正在查看的）例外**，始终保留，避免用户正在看的 agent 消失。

**灰显**：在可见列表中、已完成但未到移除时间的 node，label 渲染为灰色（保留状态图标 ✓/✗）。

**导航**：↑↓ wrap 长度 = 可见列表长度 + 1（main），避免跳到已移除项。

**配置**：`REMOVE_DELAY` 暂用常量 10s（`const COMPLETED_REMOVE_DELAY_SECS: u64 = 10;`），不暴露为设置项（YAGNI）。

**理由**：TUI 侧跟踪避免改 `SubagentProgress` 数据模型；`Instant` 与 App 现有 `turn_started_at`/`last_ctrl_c` 一致。当前 node 例外保证用户不会因延迟移除丢失正在查看的 timeline。

### D9: TUI 层过滤 "delegate" 分组节点

`delegate`/rlm 工具（`src/tools/meta/rlm/mod.rs`）是 1:N 分组（pipeline 最多 8 个 sub-task 挂同一 root 包装节点），**不能**像 D7 源头移除——否则 sub-task 失去父节点成为孤儿。故在 `SubagentTree` 层加分组节点过滤。

**判据**：`is_grouping_node(id) = !children.is_empty() && events.is_empty() && messages.is_empty()`（有子节点 + 无执行信息）。Pending leaf（刚启动无 events）无子节点不会被误过滤；真实 subagent 即便 events 暂空也是 leaf，保留。

**应用**：新增 `real_node_list()` = `node_list()` 过滤 `is_grouping_node`；`active_count()`/`count_by_status()`/`active_node_ids()`/`total_count()` 全部改用 `real_node_list()`。D8 的 `visible_node_ids` 基于 `real_node_list()` 再套完成态过滤。`subagent_status_bar.rs` 的 `active_node_ids` 同步走 `real_node_list`，修正 delegate 包装节点 stale "Running" 的计数虚高。

**与 D7 的关系**：`task` 包装节点也满足 `is_grouping_node` 判据（1 子节点 + 无 events），D9 过滤器兜底 D7 源头移除后任何残留。D7 优先源头移除（彻底、修计数语义），D9 作为 TUI 层统一过滤层覆盖 delegate 与未来同类分组节点。

### D10: 状态栏 ↑↓ 自动激活，移除 Tab 焦点切换

`event.rs` 主聊天键位分支中，当 `subagent_focus.is_none()` 且有活跃 subagent 时：移除 `Tab` 切换 `subagent_status_bar_focused` 的逻辑；改为按下 ↑↓ 即自动置 `subagent_status_bar_focused = true` 并导航。Esc 取消焦点（已有）。Enter 打开 focus view 行为不变。

**理由**：原 Tab 切换增加键位摩擦——用户需先 Tab 进入状态栏再 ↑↓。↑↓ 自动激活符合"按方向键即导航"的直觉，与 focus view 内 ↑↓ 导航选择器（D5）语义一致。状态栏可见时 ↑↓ 即导航，不可见时不响应（让位给 D11 的聊天滚动）。

### D11: 主聊天 ↑↓ 单行滚动移除，改 PageUp/PageDn + 鼠标滚轮

`event.rs` 移除主聊天的 `KeyCode::Up`/`KeyCode::Down` 单行滚动分支（原 `scroll_offset.saturating_add/sub(1)`）。聊天滚动保留 `PageUp`/`PageDown`（整页 ±10）与鼠标滚轮（`MouseScrolled`）。↑↓ 在有活跃 subagent 时由 D10 接管导航状态栏；无活跃 subagent 时 ↑↓ 不响应（避免与 D10 状态栏导航语义冲突，且 PageUp/PageDn + 鼠标滚轮已覆盖滚动需求）。

**理由**：↑↓ 单行滚动与 D10 状态栏导航键位冲突；保留单行滚动会让"↑↓ 到底滚聊天还是导航状态栏"取决于焦点状态，反直觉。移除后聊天滚动用 PageUp/PageDn（整页更高效）+ 鼠标滚轮，↑↓ 专用于 subagent 导航。

**回归风险**：习惯 ↑↓ 滚动聊天的用户需改用 PageUp/PageDn 或鼠标滚轮。help bar / 文档应提示。

### D12: `InputBox` 抽出 `update_style()`，`render()` 纯净化

`src/tui/components/input.rs`：将 slash-command 文本样式逻辑（`@`/`/` 前缀着色）从 `render()` 抽出为 `pub fn update_style(&mut self)`。`render()` 只负责边框 + 委托 `textarea` 渲染。`event.rs` 在任何文本变更后调用 `update_style()`：按键输入（`textarea.input`）、补全插入（`@`/`/` completion `insert_str`）、粘贴（`insert_char` 循环）、`take_text`（提交后清空）、Shift+Enter 换行。

**理由**：原 `render()` 每次都重设样式，但 subagent 频繁触发重渲染（Tick 100ms + SubagentUpdate）时，`render()` 与 `textarea` 内部状态竞态导致输入框闪烁/消失。抽出 `update_style()` 在变更点显式同步样式，`render()` 纯净化只读 textarea 当前状态，消除竞态。

## Risks / Trade-offs

- **[Breaking: 老用户依赖 Tab]** → 迁移：help bar 文案明确新键位；Tab 保留为 no-op。spec delta 同步。
- **[短终端 selector 压缩]** → D4 增高到 8 + D3 滚动跟随保证可见性；极端短终端不专门处理，但 `available > 0` 时 main 至少 1 行可见。
- **[timeline 无键盘滚动可访问性]** → 用户明确选择鼠标滚轮；`MouseScrolled` 已实现且不依赖 `active_area`。
- **[包装节点移除改变 tree 结构]** → 已核实 HTML 报告/transcript 不依赖；嵌套 subagent 行为与现状一致。回滚：`git revert` 单次提交。
- **[完成延迟移除误删正在查看的 node]** → D8 当前 node 例外，正在查看的 agent 不被过滤。
- **[完成时间跨 focus view 会话]** → `completed_at` 在 App 层（非 FocusViewState），退出再进入 focus view 时延迟移除仍生效，符合预期。
- **[现有单测断言旧值]** → `test_build_from_node` 等需更新 `selector_index` 期望值并移除 `active_area` 断言；`task.rs` 若有断言包装节点的测试需更新。新增滚动跟随/对齐/完成态过滤/分组节点过滤测试。
- **[Breaking: 状态栏 Tab 移除（D10）]** → 迁移：↑↓ 自动激活更直观；help bar / 状态栏提示文案说明。spec delta 同步。
- **[Breaking: 主聊天 ↑↓ 单行滚动移除（D11）]** → 迁移：PageUp/PageDn + 鼠标滚轮覆盖滚动；help bar 提示。习惯 ↑↓ 滚动的用户需适应。
- **[D11 无活跃 subagent 时 ↑↓ 不响应]** → 有意为之：避免 ↑↓ 在"滚聊天/导航状态栏"间二义。PageUp/PageDn 始终可滚。
- **[D12 update_style 漏调点]** → 漏调会导致样式未同步（输入框显示旧样式），不崩溃。code review 核对所有文本变更点已调 `update_style`。

## Migration Plan

1. `task.rs`：移除 `root_node_id` 包装节点创建块，subagent 节点 `parent_id: None`，callback 用 `parent_id: None`。
2. `subagent_tree.rs`：新增 `is_grouping_node` + `real_node_list`；`active_count`/`count_by_status`/`active_node_ids`/`total_count` 排除分组节点。
3. `subagent_status_bar.rs`：`active_node_ids` 走 `real_node_list`。
4. `input.rs`：抽出 `update_style()`，`render()` 纯净化（D12）。
5. `subagent_focus_view.rs`：移除 `FocusArea`；重构 `build_selector_lines`（统一列表 + 滚动跟随 + 完成态灰显 + 延迟移除过滤，基于 `real_node_list`）；调高度；改 `build` 初始化 `selector_index`；边框与 help 文案。
6. `app/mod.rs`：App 加 `completed_at` 字段。
7. `event.rs`：删 `active_area` 分支，键位按 D5 重映射；状态栏 ↑↓ 自动激活、移除 Tab（D10）；主聊天移除 ↑↓ 单行滚动（D11）；`update_style` 调用点（D12）；`SubagentUpdate` 写 `completed_at`；`Submit`/`clear` 清空 `completed_at`。
8. 更新现有单测，新增滚动跟随/对齐/完成态过滤/分组节点过滤/状态栏自动激活测试。
9. `cargo build` + `cargo test` 验证。
10. 手动验收（按 tasks.md 验收场景）。

回滚：单次提交，`git revert`；无数据/配置迁移。
