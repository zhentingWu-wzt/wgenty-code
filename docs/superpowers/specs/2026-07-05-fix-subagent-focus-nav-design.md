---
comet_change: fix-subagent-focus-nav
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-05-fix-subagent-focus-nav
status: final
---

# Subagent Focus View 导航与选择器重做 — 技术设计

> 本文档是 Superpowers 侧技术设计，OpenSpec capability spec（`openspec/specs/subagent-focus-view/spec.md`）经 delta `openspec/changes/fix-subagent-focus-nav/specs/subagent-focus-view/spec.md` 更新后为规范权威。设计与决策记录在 `openspec/changes/fix-subagent-focus-nav/design.md`，本文档与之同源并补充实现细节（D1–D9）。

## 1. 概述

`subagent-focus-view` 是 TUI 中全屏查看单个 subagent 执行时间线的视图。当前实现存在七类问题：

1. **键位错配**：交互被拆成 `FocusArea::Timeline` / `FocusArea::Selector` 双焦点，默认焦点在 Timeline，Tab 切换。用户按 ↑↓ 期望切换 agent，实际滚动的是 timeline——必须先 Tab 才能操作选择器。这是"选不到/看不到 main"的体感根因。
2. **选择器无滚动**：`build_selector_lines` 中 `scroll = 0` 写死，subagent 多时选中标记 `▶` 移出可见区；且 `take(available)` 在已 push "main" 后仍 take `available` 个，越界裁掉最后一项。
3. **光标错位**：`FocusViewState::build` 把 `selector_index` 硬设为 0（main），但 `node_id` 是被打开的 subagent，导致 `▶`（main）与 `●`（subagent）进入时不一致。
4. **"task"/"delegate" 包装节点污染**：`task` 工具（1:1）与 `delegate`/rlm 工具（1:N）每次调用都创建一个包装根节点（`parent_id: None`，状态 Running），但 `make_progress_callback` 只更新子节点，包装节点**永不更新**——永远卡在 "Running"、无 events/messages。它被 `node_list()`/`active_count()`/`active_node_ids()` 当作真实节点，导致 selector 出现空条目、status bar active 计数虚高。
5. **完成态堆积**：完成的 subagent 在 selector 中长期堆积，无清理机制，干扰寻找 active 项。
6. **状态栏 Tab 摩擦**：主聊天状态下，进入状态栏导航需先按 Tab 切换焦点再 ↑↓，键位冗余；与 focus view 内 ↑↓ 直接导航选择器（D5）的直觉不一致。
7. **输入框样式竞态**：`InputBox::render()` 每次重设 slash-command 样式，subagent 频繁触发重渲染（Tick 100ms + SubagentUpdate）时与 `textarea` 内部状态竞态，导致输入框闪烁/消失。

本设计通过 12 个决策（D1–D12）系统解决以上问题。

## 2. Goals / Non-Goals

**Goals:**
- ↑↓ 直接导航选择器（main + subagents），无需 Tab。
- 进入 focus view 时光标 `▶` 与当前 `●` 对齐。
- 选择器选中项始终可见（滚动跟随），subagent 多时不再裁掉。
- timeline 滚动交给鼠标滚轮，键位简化。
- 修掉 `take(available)` 越界。
- selector 中完成态灰显，并在延迟后自动移除（declutter）。
- 源头移除 "task" 包装节点（1:1，安全）；TUI 层过滤 "delegate" 分组节点（1:N，不可源头移除）。修正 active 计数与 selector 噪声。
- 状态栏 ↑↓ 自动激活并导航（移除 Tab 焦点切换），降低进入 focus view 的键位摩擦。
- 主聊天滚动简化为 PageUp/PageDn + 鼠标滚轮，↑↓ 让位给状态栏导航。
- `InputBox` 抽出 `update_style()`，修复频繁重渲染下输入框闪烁/消失的视觉故障。

**Non-Goals:**
- 不改 focus view 入口（状态栏 Enter 打开）。
- 不改四区布局（header / timeline / selector / help）。
- 不改 `node_list` DFS 顺序或嵌套 subagent 的 tree 结构语义。
- 不改主聊天窗口的文本输入与提交行为（仅简化滚动/导航键位）。
- 不重做 timeline 内容渲染（事件类型视觉区分保持）。
- 不改 HTML 报告 / transcript 存储（已确认不依赖内存 tree 包装节点）。
- 不把 `REMOVE_DELAY` 暴露为设置项（YAGNI）。

## 3. 涉及文件

| 文件 | 变更 |
|------|------|
| `src/tui/components/subagent_focus_view.rs` | 移除 `FocusArea`；`build` 光标对齐；`build_selector_lines` 重构（统一列表 + 滚动跟随 + 完成态灰显 + 延迟移除过滤）；高度 `Length(8)`；边框视觉；help 文案 |
| `src/tui/components/subagent_tree.rs` | 新增 `is_grouping_node` + `real_node_list`；`active_count`/`count_by_status`/`active_node_ids`/`total_count` 排除分组节点 |
| `src/tui/components/subagent_status_bar.rs` | `active_node_ids` 走 `real_node_list` 或过滤分组节点 |
| `src/tui/components/input.rs` | 抽出 `update_style()`，`render()` 纯净化（D12） |
| `src/tui/app/event.rs` | 删 `active_area` 分支；键位按 D5 重映射；状态栏 ↑↓ 自动激活/移除 Tab（D10）；主聊天 ↑↓ 滚动移除（D11）；`update_style` 调用点（D12）；`SubagentUpdate` 写 `completed_at`；`Submit`/`clear` 清空 |
| `src/tui/app/mod.rs` | App 新增 `completed_at: HashMap<String, std::time::Instant>` 字段 |
| `src/tools/meta/task.rs` | 移除 `root_node_id` 包装节点创建块；subagent `parent_id: None`；callback `parent_id: None` |
| 各文件单测 | 更新断言 + 新增滚动跟随/对齐/完成态过滤/分组节点过滤测试 |

## 4. Decisions

### D1: 移除 `FocusArea`/`active_area`，选择器为唯一交互区

删除 `FocusArea` enum 与 `FocusViewState::active_area` 字段。选择器是 focus view 内唯一可键盘交互的区域；timeline 变为只读视图（鼠标滚轮滚动）。双焦点的 Tab 切换是当前混乱根因；保留枚举但默认 Selector 仍留死代码（timeline ↑↓ 滚动本就要移除），故直接删除枚举更彻底。

### D2: `selector_index` 初始化对齐当前 `node_id`

`FocusViewState::build` 中，计算 `node_list` 中 `node_id` 的位置 `pos`，设 `selector_index = pos + 1`（+1 是 main 占用的 index 0）。找不到时回退 0。进入时 `▶` 与 `●` 重合，消除错位。

```rust
let pos = tree.node_list().iter().position(|n| n == &node_id);
let selector_index = pos.map(|p| p + 1).unwrap_or(0);
```

### D3: 选择器滚动跟随——统一列表 + 滑动窗口

`build_selector_lines` 重构为对统一列表 `["main", ...visible_node_ids]` 做 `skip(scroll_start).take(available)`，`scroll_start` 由 `selector_index` 推导：

```
if selector_index < scroll_start          { scroll_start = selector_index }
if selector_index >= scroll_start + avail { scroll_start = selector_index - avail + 1 }
scroll_start = scroll_start.min(list_len.saturating_sub(avail))
```

`scroll_start` 渲染时计算（纯函数 of `selector_index` + 列表长度 + `available`），不入状态。同时修掉 `take(available)` 越界（统一列表后 `take(available)` 直接对齐可见行数，不再有"先 push main 再 take available"的 off-by-one）。

### D4: 选择器高度 `Length(6)` → `Length(8)`

内层 6 行，可显示 main + 5 个 subagent，剩余靠 D3 滚动跟随。timeline 仍为 `Min(5)`。

### D5: 键位重映射

| 键 | 行为 |
|----|------|
| ↑ / ↓ | 导航选择器（wrap，含 main，基于过滤后可见列表） |
| Enter | 切换到 `▶` 所指 agent；`▶` 在 main 则退出 focus view |
| Esc | 退出 focus view（保留） |
| `t` | 折叠/展开工具调用（始终可用，无 `active_area` 守卫） |
| 鼠标滚轮 | 滚动 timeline（始终生效，无 `active_area` 守卫） |
| PageUp / PageDown | focus view 内不响应 |
| Tab | no-op（`_ => return` 兜底） |

### D6: 边框视觉——选择器高亮，timeline 暗淡

selector 边框恒为 `active_border`，timeline 边框恒为 `inactive_border`（移除条件分支）。help bar 文案同步更新为单焦点版本：`↑↓ navigate · Enter switch/exit · t fold · Esc back · wheel scroll timeline`。

### D7: 源头移除 "task" 包装节点

`src/tools/meta/task.rs` 不再创建 `root_node_id` 包装节点（背景模式 + 同步模式两处创建块均移除）。subagent 节点直接以 `parent_id: None` 创建，成为 tree root；`make_progress_callback` 用 `parent_id: None` 更新该节点。`SubagentTree::upsert` 现有逻辑（第一个 `parent_id: None` 设为 root）天然适配。

**理由**：`task` 是 1:1 包裹（一个 wrapper 对一个 subagent），包装节点永不更新、不被 HTML 报告/transcript 依赖，是无信息的噪声节点。源头移除一举修正：selector 噪声条目、`active_count()` 虚高、`active_node_ids()` 误含。比 TUI 层启发式过滤（脆弱、不修计数）更彻底。

**已核实**：HTML 报告（`subagent_trace.rs`）不读内存 tree；transcript label `"task: <description>"`（`task/transcript.rs`）是持久化记录的独立 label，与内存节点无关，不受影响。

**嵌套 subagent**：`task` 工具对超过 `max_depth` 的子任务过滤掉 `task` 工具本身，且 `upsert` 只在 `root_id` 为 None 时设 root。移除包装后，首个 subagent 成为 root；后续 `task` 调用创建的新 root 因 `root_id` 已设而不可见——这是与现状一致的预存行为，本次不引入回归。

### D8: 完成态灰显 + 延迟移除（TUI 侧跟踪）

**状态**：App 新增 `completed_at: HashMap<String, std::time::Instant>`，记录每个 subagent node 完成时刻。

**写入时机**：`AppEvent::SubagentUpdate(progress)` 中，若 `progress.status` 是 Completed/Failed/Cancelled 且该 node 之前非完成态（transition 时刻），写入 `completed_at`。`subagent_tree.clear()`（新 turn `Submit`）时清空。

**可见列表计算**：focus view selector 渲染与导航基于"可见列表"——main + 所有 subagent，但**排除**满足 `completed_at[node]` 存在且 `elapsed > REMOVE_DELAY`（默认 10s）的 node；**当前 `node_id`（正在查看的）例外**，始终保留，避免用户正在看的 agent 消失。

```rust
const COMPLETED_REMOVE_DELAY_SECS: u64 = 10;

fn visible_node_ids(tree, completed_at, now, current_node_id) -> Vec<String> {
    tree.real_node_list().into_iter()
        .filter(|id| {
            if *id == current_node_id { return true; }       // 当前查看的例外
            match completed_at.get(id) {
                Some(t) => now.duration_since(*t).as_secs() < COMPLETED_REMOVE_DELAY_SECS,
                None => true,
            }
        })
        .collect()
}
```

**灰显**：在可见列表中、已完成但未到移除时间的 node，label 渲染为灰色（保留状态图标 ✓/✗）。

**导航**：↑↓ wrap 长度 = 可见列表长度 + 1（main），避免跳到已移除项。

**配置**：`REMOVE_DELAY` 暂用常量 10s，不暴露为设置项（YAGNI）。

**理由**：TUI 侧跟踪避免改 `SubagentProgress` 数据模型；`Instant` 与 App 现有 `turn_started_at`/`last_ctrl_c` 一致。当前 node 例外保证用户不会因延迟移除丢失正在查看的 timeline。渲染循环每个 Tick（100ms）触发 `terminal.draw`，过滤在渲染时计算 → 延迟移除 ~100ms 精度生效，无需 Tick 主动清理。

### D9: TUI 层过滤 "delegate" 分组节点

`delegate`/rlm 工具（`src/tools/meta/rlm/mod.rs`）是 1:N 分组（pipeline 最多 8 个 sub-task 挂在同一 root 包装节点下），**不能**像 D7 那样源头移除——否则 sub-task 会失去父节点成为孤儿，破坏 tree 结构。故在 `SubagentTree` 层加分组节点过滤。

**判据**：`is_grouping_node(id) = !children.is_empty() && events.is_empty() && messages.is_empty()`
- 有子节点（分组特征）+ 无 events/messages（无执行信息）= 分组节点。
- Pending leaf（刚启动无 events 的真实 subagent）无子节点，不会被误过滤。
- 真实 subagent 即便 events 暂空，只要它是 leaf（无 children）就保留。

**应用**：新增 `real_node_list()` = `node_list()` 过滤掉 `is_grouping_node`。`active_count()`、`count_by_status()`、`active_node_ids()`、`total_count()` 全部改用 `real_node_list()` 或在遍历时跳过分组节点。

**可见列表**：D8 的 `visible_node_ids` 基于 `real_node_list()` 再套完成态过滤——分组节点先被 D9 排除，再由 D8 排除超时完成项。

**`task` 包装节点也满足 `is_grouping_node` 判据**（1 子节点 + 无 events），所以 D9 的过滤器同时兜底 D7 源头移除后任何残留的 task 包装节点。D7 仍优先源头移除（彻底、修计数语义），D9 作为 TUI 层统一过滤层覆盖 delegate 与任何未来出现的同类分组节点。

**`subagent_status_bar.rs`**：`active_node_ids` 改走 `real_node_list`，修正 delegate 包装节点 stale "Running" 导致的 status bar active 计数虚高。

### D10: 状态栏 ↑↓ 自动激活，移除 Tab 焦点切换

`event.rs` 主聊天键位分支中，当 `subagent_focus.is_none()` 且有活跃 subagent 时：移除 `Tab` 切换 `subagent_status_bar_focused` 的逻辑；改为按下 ↑↓ 即自动置 `subagent_status_bar_focused = true` 并导航。Esc 取消焦点（已有）。Enter 打开 focus view 行为不变。

**理由**：原 Tab 切换增加键位摩擦——用户需先 Tab 进入状态栏再 ↑↓。↑↓ 自动激活符合"按方向键即导航"的直觉，与 focus view 内 ↑↓ 导航选择器（D5）语义一致。

### D11: 主聊天 ↑↓ 单行滚动移除，改 PageUp/PageDn + 鼠标滚轮

`event.rs` 移除主聊天的 `KeyCode::Up`/`KeyCode::Down` 单行滚动分支。聊天滚动保留 `PageUp`/`PageDown`（整页 ±10）与鼠标滚轮。↑↓ 在有活跃 subagent 时由 D10 接管导航状态栏；无活跃 subagent 时 ↑↓ 不响应（避免与 D10 状态栏导航语义冲突，且 PageUp/PageDn + 鼠标滚轮已覆盖滚动需求）。

**理由**：↑↓ 单行滚动与 D10 状态栏导航键位冲突；保留单行滚动会让"↑↓ 到底滚聊天还是导航状态栏"取决于焦点状态，反直觉。移除后聊天滚动用 PageUp/PageDn（整页更高效）+ 鼠标滚轮，↑↓ 专用于 subagent 导航。

### D12: `InputBox` 抽出 `update_style()`，`render()` 纯净化

`src/tui/components/input.rs`：将 slash-command 文本样式逻辑（`@`/`/` 前缀着色）从 `render()` 抽出为 `pub fn update_style(&mut self)`。`render()` 只负责边框 + 委托 `textarea` 渲染。`event.rs` 在任何文本变更后调用 `update_style()`：按键输入、补全插入、粘贴、`take_text`、Shift+Enter。

**理由**：原 `render()` 每次都重设样式，subagent 频繁触发重渲染（Tick 100ms + SubagentUpdate）时与 `textarea` 内部状态竞态导致输入框闪烁/消失。抽出 `update_style()` 在变更点显式同步样式，`render()` 纯净化只读 textarea 当前状态，消除竞态。

## 5. Risks / Trade-offs

- **[Breaking: 老用户依赖 Tab]** → 迁移：help bar 文案明确新键位；Tab 保留为 no-op。spec delta 同步。
- **[短终端 selector 压缩]** → D4 增高到 8 + D3 滚动跟随保证可见性；极端短终端不专门处理，但 `available > 0` 时 main 至少 1 行可见。
- **[timeline 无键盘滚动可访问性]** → 用户明确选择鼠标滚轮；`MouseScrolled` 已实现且不依赖 `active_area`。
- **[包装节点移除改变 tree 结构（D7）]** → 已核实 HTML 报告/transcript 不依赖；嵌套 subagent 行为与现状一致。回滚：`git revert` 单次提交。
- **[delegate 分组节点过滤判据误杀（D9）]** → `is_grouping_node` 要求"有子节点 + 无 events/messages"。Pending leaf 无子节点不会被误过滤；真实 subagent 即便 events 暂空也是 leaf。判据稳健。
- **[完成延迟移除误删正在查看的 node（D8）]** → 当前 node 例外，正在查看的 agent 不被过滤。
- **[完成时间跨 focus view 会话]** → `completed_at` 在 App 层（非 FocusViewState），退出再进入 focus view 时延迟移除仍生效，符合预期；新 turn `Submit` 清空。
- **[计数语义变更]** → `active_count()` 等排除分组节点是 bug 修正（分组节点本不该计入），改语义是正确的。
- **[现有单测断言旧值]** → `test_build_from_node` 等需更新 `selector_index` 期望值并移除 `active_area` 断言；`task.rs` 若有断言包装节点的测试需更新。新增滚动跟随/对齐/完成态过滤/分组节点过滤测试。

## 6. Migration Plan

1. `task.rs`：移除 `root_node_id` 包装节点创建块（背景 + 同步两处），subagent 节点 `parent_id: None`，callback 用 `parent_id: None`。
2. `subagent_tree.rs`：新增 `is_grouping_node` + `real_node_list`；`active_count`/`count_by_status`/`active_node_ids`/`total_count` 排除分组节点。
3. `subagent_status_bar.rs`：`active_node_ids` 走 `real_node_list`。
4. `subagent_focus_view.rs`：移除 `FocusArea`；重构 `build_selector_lines`（统一列表 + 滚动跟随 + 完成态灰显 + 延迟移除过滤，基于 `real_node_list`）；调高度 `Length(8)`；改 `build` 初始化 `selector_index`；边框与 help 文案。
5. `app/mod.rs`：App 加 `completed_at` 字段。
6. `event.rs`：删 `active_area` 分支，键位按 D5 重映射；`SubagentUpdate` 写 `completed_at`（transition 时刻）；`Submit`/`clear` 清空 `completed_at`。
7. 更新现有单测，新增滚动跟随/对齐/完成态过滤/分组节点过滤测试。
8. `cargo build` + `cargo test` 验证。
9. 手动验收（按 `specs/subagent-focus-view/spec.md` 验收场景逐项核对）。

回滚：单次提交，`git revert`；无数据/配置迁移。

## 7. Test Strategy

**更新现有单测：**
- `test_build_from_node` 等：`selector_index` 期望值改为 `pos+1`，移除 `active_area` 断言。
- `test_rebuild_preserves_ui_state` 等涉及 `active_area`/`selector_index` 的断言同步更新。
- `task.rs` 测试：确认不再创建包装节点，subagent 为 root（`parent_id: None`）。

**新增单测：**
- 光标对齐：多 node tree，打开非根 node 验证 `selector_index == pos+1`。
- 滚动跟随：cursor 接近底部时 `scroll_start` 跟随，cursor 始终在窗口内。
- `build_selector_lines` 不越界：main + N subagent，available 较小时总数 ≤ available。
- 完成态灰显与延迟移除过滤：超时 node 被排除，当前 node 例外；未超时完成 node 保留并灰显。
- `is_grouping_node` 过滤：delegate 包装节点排除，sub-task 保留；`active_count`/`active_node_ids` 不含分组节点。
- 嵌套 task 后 tree root 行为：首个 subagent 为 root，后续 task root 不可见（与现状一致）。

**手动验收**：按 `specs/subagent-focus-view/spec.md` 验收场景逐项核对（↑↓ 导航、Enter 切换/退出、鼠标滚 timeline、't' 折叠、Tab no-op、选择器滚动跟随、短终端、完成态灰显、延迟移除、包装节点不出现、active 计数正确、delegate 分组节点不出现）。
