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

### 主聊天与状态栏键位简化 + 输入框样式修复（subagent-status-display）

- **BREAKING**: 状态栏移除 Tab 焦点切换；改为 ↑↓ 自动激活状态栏焦点并导航（无需先 Tab），Esc 取消焦点。Enter 打开 focus view 行为不变。
- **BREAKING**: 主聊天窗口移除 ↑↓ 单行滚动；聊天滚动改为 PageUp/PageDn（整页）+ 鼠标滚轮。↑↓ 在有活跃 subagent 时用于状态栏导航，无活跃 subagent 时不响应（避免与状态栏导航语义冲突）。
- `InputBox` 抽出 `update_style()` 方法，在任何文本变更（按键输入、补全插入、粘贴、`take_text`、Shift+Enter）后调用，使 `render()` 保持纯净，修复 subagent 频繁重渲染时输入框闪烁/消失的视觉故障。

## Capabilities

### New Capabilities
<!-- 无新增 capability -->

### Modified Capabilities
- `subagent-focus-view`: 导航与选择器交互变更（移除 Tab 双焦点、↑↓ 导航选择器、timeline 鼠标滚轮、滚动跟随、光标对齐、高度调整）；selector 完成态灰显与延迟移除；selector 不再显示 "task" 包装节点。
- `subagent-status-display`: 状态栏焦点模型变更（移除 Tab 切换，↑↓ 自动激活并导航，Esc 取消焦点）；active 计数排除分组节点（delegate 包装）。

> 注：active 计数排除分组节点是对现有 spec「计数正确」要求的实现层修正；状态栏焦点模型变更是行为变更，需 delta。主聊天 ↑↓ 滚动移除与输入框 `update_style` 重构无对应 capability spec，作为实现层变更记录在 design 与 tasks 中。

## Impact

- **代码**：
  - `src/tui/components/subagent_focus_view.rs`（渲染布局、选择器滚动、光标起点、完成态灰显、`FocusArea` 移除）。
  - `src/tui/components/subagent_tree.rs`（`is_grouping_node` + `real_node_list`，count 方法过滤分组节点）。
  - `src/tui/components/subagent_status_bar.rs`（`active_node_ids` 走 `real_node_list`）。
  - `src/tui/components/input.rs`（抽出 `update_style()`，`render()` 纯净化）。
  - `src/tui/app/event.rs`（focus view 键位重映射、状态栏 ↑↓ 自动激活/移除 Tab、主聊天 ↑↓ 滚动移除、完成时间跟踪、`Tick` 过滤、`update_style` 调用点）。
  - `src/tui/app/mod.rs`（App 新增完成时间 `HashMap` 字段）。
  - `src/tools/meta/task.rs`（移除包装根节点创建，subagent 直接为 root）。
- **传参**：`src/tui/app/render.rs` 仅传参，无逻辑变更。
- **测试**：`subagent_focus_view.rs` 现有单测断言需更新；新增滚动跟随/光标对齐/完成态过滤/分组节点过滤测试。`task.rs` 单测若断言包装节点需更新。
- **spec**：`openspec/specs/subagent-focus-view/spec.md` 与 `openspec/specs/subagent-status-display/spec.md` 需 delta。
- **依赖/API**：无外部 API、无依赖变更。
- **回归风险**：focus view 键位 breaking；状态栏 Tab→↑↓ 自动激活 breaking；主聊天 ↑↓ 单行滚动移除 breaking（用户需改用 PageUp/PageDn 或鼠标滚轮）；包装节点移除改变 tree 结构（subagent 直接为 root），需确认 HTML 报告/transcript 不依赖包装节点（已核实：HTML 报告用 transcript store，不依赖内存 tree；transcript label 独立）。
