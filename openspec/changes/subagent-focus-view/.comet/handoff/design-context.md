# Comet Design Handoff

- Change: subagent-focus-view
- Phase: design
- Mode: compact
- Context hash: da124a38eedb5e21455cf3c58f9f7cef6d3fb6ce203837a931180a1d416a4dcf

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/subagent-focus-view/proposal.md

- Source: openspec/changes/subagent-focus-view/proposal.md
- Lines: 1-55
- SHA256: a42580a9657d9f7eeb079eec5252e72ec25eb50eac37c6bfda95d54ae61aaf5f

```md
## Why

子代理执行时，主聊天窗口被内联的树形卡片（`render_subagent_card`）占据大量空间，将子代理的执行细节（状态图标、当前工具、文本快照、Action 日志）与主 agent 的对话流混在一起。用户无法在一个干净的上下文中阅读主 agent 的回复——每次调用 `task` / `delegate` 工具后，聊天区就插入一个 5-10 行的树形卡片，打断阅读节奏。

同时，现有的 Ctrl+Shift+T 监控面板是一个居中弹窗覆盖层，需要主动唤起、遮挡全部内容、且无法在"看主 agent"和"看子代理"之间快速切换。

用户需要的是一种**焦点分离**的交互模式：主聊天窗口保持干净，子代理执行过程在需要时才进入查看，且能一键返回。

## What Changes

### 移除内联子代理卡片
- 主聊天窗口不再渲染 `render_subagent_card`，`task` / `delegate` 工具消息后仅保留工具结果摘要，不再插入树形结构
- 聊天区恢复为纯粹的"用户 ↔ 主 agent"对话流

### 输入框下方子代理状态条
- 当有子代理正在执行时，在输入框上方（status bar 与 input 之间）显示一行紧凑的状态条
- 状态条展示当前活跃的子代理列表，每个条目显示：状态图标 + label + 当前工具（如 `🔄 explore — file_read("src/auth.rs")`）
- 支持方向键 ↑↓ 在条目间选择（选中项高亮）
- 无子代理执行时状态条隐藏，不占用任何空间

### 子代理焦点视图（Enter 进入）
- 在状态条选中某个子代理后按 Enter，进入该子代理的**全屏焦点视图**
- 焦点视图内容：
  - 顶部：子代理 label + 状态 + 耗时 + 轮次 + token 消耗
  - 主体：完整事件时间线（Thought → Action → ToolResult → Error → Completion），按时间顺序排列，不截断
  - 底部：操作提示（Esc 返回 / ↑↓ 滚动）
- 焦点视图占满整个终端窗口，替换主聊天+输入布局
- 实时更新：子代理仍在执行时，焦点视图持续轮询并刷新事件流

### 返回主 agent 窗口
- 在焦点视图中按 Esc 返回主聊天窗口
- 返回后主聊天窗口恢复原状，输入框焦点回到文本输入
- 状态条仍显示子代理执行进度（如果仍在运行）

## Capabilities

### New Capabilities

- `subagent-focus-view`: 全屏子代理执行视图，展示选中子代理的完整事件时间线（Thought/Action/ToolResult/Error/Completion），支持实时刷新和 Esc 返回。通过输入框下方状态条的 ↑↓ 选择 + Enter 进入。

### Modified Capabilities

- `subagent-inline-display`: 移除主聊天窗口中的内联子代理树形卡片（`render_subagent_card`），聊天区不再展示子代理执行细节。子代理进度信息改为通过输入框下方状态条和焦点视图展示。

## Impact

- **`src/tui/components/chat.rs`**: 移除 `render_subagent_card` 调用（`render()` 函数中 `tool_name == "task" || "delegate"` 分支）；移除 `render_subagent_card` 和 `render_tree_nodes` 函数（或保留供焦点视图复用）
- **`src/tui/app/render.rs`**: 主布局新增子代理状态条区域（位于 status 与 input 之间，仅在有活跃子代理时分配空间）；新增焦点视图全屏渲染分支（当 `subagent_focus_view` 激活时替换整个布局）
- **`src/tui/app/mod.rs`**: 新增 `subagent_focus_node_id: Option<String>` 字段追踪焦点视图选中的子代理；新增 `subagent_status_bar_selected: usize` 字段追踪状态条选中索引
- **`src/tui/app/event.rs`**: 新增状态条键盘路由（↑↓ 选择子代理、Enter 进入焦点视图）；新增焦点视图键盘路由（Esc 返回、↑↓ 滚动事件时间线）
- **`src/tui/components/subagent_focus_view.rs`** (新文件): 全屏焦点视图组件，渲染选中子代理的完整事件时间线、状态摘要、操作提示
- **`src/tui/components/subagent_status_bar.rs`** (新文件): 输入框上方紧凑状态条组件，展示活跃子代理列表并支持选择高亮
- **`src/tui/components/mod.rs`**: 注册新组件模块
- **`src/tui/components/status.rs`**: 简化子代理状态展示（状态条接管详情展示，status bar 仅保留总计数或移除子代理计数）
- **`src/tui/components/subagent_panel.rs`**: 现有 Ctrl+Shift+T 监控面板保留或合并到焦点视图中（待设计阶段确定）
```

## openspec/changes/subagent-focus-view/design.md

- Source: openspec/changes/subagent-focus-view/design.md
- Lines: 1-19
- SHA256: 50d8f43d3d9f7468d97d801ce94c7aff732bb8b8d755893d14d80deacecf5cb6

```md
<!-- Design Doc placeholder — to be filled during the design phase (brainstorming → design doc) -->
## Context

待设计阶段填充。

### Current Data Flow

```
subagent_loop.rs → emit(SubagentProgress) → daemon shared store
    → TUI poll_subagent_progress() every 500ms
    → AppEvent::SubagentUpdate → subagent_tree.upsert()
    → render: inline card (chat.rs) + status bar + monitor panel (Ctrl+Shift+T)
```

### Proposed Changes

1. 移除内联子代理卡片（`render_subagent_card`）
2. 新增输入框下方子代理状态条
3. 新增全屏子代理焦点视图（Enter 进入，Esc 返回）
```

## openspec/changes/subagent-focus-view/tasks.md

- Source: openspec/changes/subagent-focus-view/tasks.md
- Lines: 1-31
- SHA256: e02b3e4e364703454838978af8676c6c968b52ccbdd1e84fb3aac142afe20721

```md
<!-- Tasks placeholder — to be filled during the build phase (writing-plans) -->

## 1. TUI Layout & State Changes

- [ ] 1.1 Add `subagent_focus_node_id: Option<String>` and `subagent_status_bar_selected: usize` to App struct (`src/tui/app/mod.rs`)
- [ ] 1.2 Update main render layout to include subagent status bar area (`src/tui/app/render.rs`)
- [ ] 1.3 Add focus view full-screen render branch (`src/tui/app/render.rs`)

## 2. Remove Inline Subagent Card

- [ ] 2.1 Remove `render_subagent_card` call from `render()` in `src/tui/components/chat.rs`
- [ ] 2.2 Clean up or repurpose `render_subagent_card` and `render_tree_nodes` functions

## 3. Subagent Status Bar Component

- [ ] 3.1 Create `src/tui/components/subagent_status_bar.rs` — compact status bar with active subagent list
- [ ] 3.2 Add keyboard navigation (↑↓ select, Enter to focus view) in `src/tui/app/event.rs`
- [ ] 3.3 Register module in `src/tui/components/mod.rs`

## 4. Subagent Focus View Component

- [ ] 4.1 Create `src/tui/components/subagent_focus_view.rs` — full-screen event timeline view
- [ ] 4.2 Add keyboard handling (Esc return, ↑↓ scroll) in `src/tui/app/event.rs`
- [ ] 4.3 Implement real-time polling for running subagents in focus view

## 5. Tests & Validation

- [ ] 5.1 Unit tests for status bar state and selection logic
- [ ] 5.2 Unit tests for focus view rendering
- [ ] 5.3 `cargo clippy --all-targets -- -D warnings` passes
- [ ] 5.4 `cargo test --all` passes
```

## openspec/changes/subagent-focus-view/specs/subagent-action-visibility/spec.md

- Source: openspec/changes/subagent-focus-view/specs/subagent-action-visibility/spec.md
- Lines: 1-19
- SHA256: 53da1abf4330144d1c530241f7e101ae199a7257f46c44af884ef380d0c93fe5

```md
# subagent-action-visibility Specification

## MODIFIED Requirements

### Requirement: Inline subagent card shows current action with context
The inline subagent card SHALL NOT be rendered in the main chat area. Instead, the current tool call with parameters and the most recent model text SHALL be displayed in the subagent status bar (below the input area) and the full execution timeline SHALL be available in the focus view.

#### Scenario: Chat area remains clean during subagent execution
- **WHEN** a subagent is Running with text snapshot "Analyzing the auth module structure…" and current tool `file_read("src/auth.rs")`
- **THEN** the main chat area SHALL NOT display any inline subagent card or tree structure
- **AND** the subagent status bar SHALL display the current tool and a compact label for the subagent

#### Scenario: No inline card when subagent has no text yet
- **WHEN** a subagent is Running but has no text snapshot yet (first round, still streaming)
- **THEN** the main chat area SHALL NOT display any inline subagent card
- **AND** the subagent status bar SHALL display the subagent label with a "thinking…" indicator

## REMOVED Requirements
<!-- The "Inline subagent card shows current action with context" requirement is modified, not removed. The scenarios above replace the old inline card scenarios. -->
```

## openspec/changes/subagent-focus-view/specs/subagent-focus-view/spec.md

- Source: openspec/changes/subagent-focus-view/specs/subagent-focus-view/spec.md
- Lines: 1-87
- SHA256: 418716a6e710270bd39407c5c6f5c66d6bc940e23ba7f9e861413902e270965b

[TRUNCATED]

```md
# subagent-focus-view Specification

## ADDED Requirements

### Requirement: Full-screen subagent focus view
The TUI SHALL provide a full-screen focus view that replaces the main chat layout when a user selects a subagent from the status bar and presses Enter. The focus view SHALL display the complete execution timeline of the selected subagent, including all events (Thought, Action, ToolResult, Error, Completion) without truncation.

#### Scenario: Entering focus view from status bar
- **WHEN** the subagent status bar is visible with at least one active subagent
- **AND** the user navigates with ↑↓ to select a subagent and presses Enter
- **THEN** the TUI SHALL replace the main chat + input layout with a full-screen focus view for the selected subagent

#### Scenario: Focus view shows complete event timeline
- **WHEN** a subagent has produced 5 Thought events, 3 Action events, and 2 ToolResult events
- **THEN** the focus view SHALL display all events in chronological order, each with its type icon, elapsed timestamp, and full content (no truncation)

#### Scenario: Focus view real-time updates
- **WHEN** the selected subagent is still Running while the focus view is open
- **THEN** the focus view SHALL continue polling for progress updates and append new events to the timeline as they arrive

#### Scenario: Focus view header shows summary metadata
- **WHEN** the focus view is open for a subagent
- **THEN** the top of the focus view SHALL display the subagent label, status icon, elapsed time, round progress (when available), and cumulative token count

### Requirement: Focus view navigation and exit
The focus view SHALL support keyboard navigation for scrolling the event timeline and shall return to the main chat layout when the user presses Esc. The focus view SHALL also provide a subagent selector bar at the bottom for direct switching between subagent focus views without returning to the main chat.

#### Scenario: Scrolling the event timeline
- **WHEN** the focus view is open and the event timeline exceeds the visible area
- **AND** the focus is on the timeline area (default)
- **THEN** ↑↓ keys SHALL scroll the timeline one line at a time, and PageUp/PageDown SHALL scroll by 10 lines

#### Scenario: Tab toggles between timeline and subagent selector
- **WHEN** the user presses Tab while in the focus view
- **THEN** the focus SHALL toggle between the event timeline area and the subagent selector bar
- **AND** the currently focused area SHALL be visually indicated (e.g., highlight border)

#### Scenario: Switching to another subagent from within focus view
- **WHEN** the user presses Tab to focus the subagent selector bar
- **AND** navigates with ↑↓ to select another subagent and presses Enter
- **THEN** the focus view SHALL switch to display the selected subagent's event timeline
- **AND** the timeline scroll position SHALL reset to the latest event

#### Scenario: Exiting focus view returns to main chat
- **WHEN** the user presses Esc while in the focus view
- **THEN** the TUI SHALL close the focus view and restore the main chat + input layout
- **AND** the input box SHALL regain focus for text entry
- **AND** the subagent status bar SHALL remain visible if subagents are still running

### Requirement: Focus view subagent selector bar
The focus view SHALL include a subagent selector bar at the bottom of the screen, listing all subagents (active and completed) with their status icons and labels. The selector bar allows direct switching between subagent views without returning to the main chat.

#### Scenario: Selector bar shows all subagents
- **WHEN** the focus view is open and there are 3 subagents (1 running, 2 completed)
- **THEN** the selector bar SHALL display all 3 subagents with their status icons and labels, with the currently viewed subagent highlighted

#### Scenario: Selector bar wraps around
- **WHEN** the user navigates past the last subagent in the selector bar with ↓
- **THEN** the selection SHALL wrap around to the first subagent
- **AND** navigating past the first subagent with ↑ SHALL wrap to the last

#### Scenario: Selector bar indicates current view
- **WHEN** the focus view is displaying subagent "explore"
- **THEN** the selector bar SHALL visually distinguish "explore" as the currently viewed subagent (e.g., reverse video or arrow marker)

### Requirement: Focus view event type visual distinction
Each event type in the focus view timeline SHALL be visually distinguishable by color and icon, so users can quickly scan the execution flow.

#### Scenario: Thought event display
- **WHEN** a Thought event is displayed in the timeline
- **THEN** it SHALL be rendered with a 💬 icon and a muted color, with the full model text wrapped to the terminal width

#### Scenario: Action event display
- **WHEN** an Action event (tool call) is displayed
- **THEN** it SHALL be rendered with a 🛠 icon and a blue accent color, showing `tool_name("params_summary")`

#### Scenario: ToolResult event display
- **WHEN** a ToolResult event is displayed
- **THEN** it SHALL be rendered with a ✅ or ❌ icon based on success, with a green or red accent color, showing the result summary

```

Full source: openspec/changes/subagent-focus-view/specs/subagent-focus-view/spec.md

## openspec/changes/subagent-focus-view/specs/subagent-status-display/spec.md

- Source: openspec/changes/subagent-focus-view/specs/subagent-status-display/spec.md
- Lines: 1-56
- SHA256: 259a2933b5cd68eab86aa93c791cb9b26bcd9abb60e84119e6274f022907a171

```md
# subagent-status-display Specification

## ADDED Requirements

### Requirement: Subagent status bar below input area
The TUI SHALL display a compact status bar between the main status line and the input box when subagents are active. The status bar SHALL list each active subagent with its status icon, label, and current tool call (when available), and SHALL support keyboard navigation to select a subagent for focus view entry.

#### Scenario: Status bar appears when subagents start
- **WHEN** the first subagent begins execution (status transitions to Running)
- **THEN** a status bar SHALL appear between the main status line and the input box, occupying the minimum height needed to display all active subagents (capped at 5 lines)

#### Scenario: Status bar shows each active subagent
- **WHEN** 3 subagents are Running with labels "explore", "plan", "general-purpose" and current tools `grep("fn auth")`, `file_read("src/mod.rs")`, and none respectively
- **THEN** the status bar SHALL display three lines, each showing: status icon + label + current tool (or "thinking…" if no tool yet)

#### Scenario: Status bar supports selection navigation
- **WHEN** the status bar is visible and the user presses ↑ or ↓
- **THEN** the selected subagent SHALL change, with the currently selected entry highlighted in a distinct color
- **AND** the selection SHALL wrap around from first to last and vice versa

#### Scenario: Status bar hides when no subagents active
- **WHEN** all subagents have completed or failed and no new subagents are running
- **THEN** the status bar SHALL disappear and the input box SHALL reclaim the space

#### Scenario: Status bar Enter triggers focus view
- **WHEN** the user presses Enter while a subagent is selected in the status bar
- **THEN** the TUI SHALL enter the full-screen focus view for that subagent

#### Scenario: Status bar does not interfere with text input
- **WHEN** the status bar is visible and the user types characters
- **THEN** the characters SHALL go to the input box as normal, unless the user has explicitly navigated to the status bar with ↑↓
- **AND** pressing any non-navigation key SHALL return focus to the input box

## MODIFIED Requirements

### Requirement: Status bar shows subagent progress counters
The TUI main status line SHALL display a compact subagent progress summary when subagents are active. The detailed per-subagent information (current tool, label, timing) SHALL be shown in the subagent status bar below the input area, not in the main status line.

#### Scenario: Multiple subagents running
- **WHEN** 3 subagents are active and 5 have completed out of 8 total
- **THEN** the main status line SHALL display a compact summary like "Subagent 3 active · 5/8 done"
- **AND** the subagent status bar below the input SHALL list the 3 active subagents with their labels and current tools

#### Scenario: All subagents complete successfully
- **WHEN** all subagents have completed with no failures
- **THEN** the main status line SHALL display "N tasks done" where N is the total subagent count
- **AND** the subagent status bar SHALL be hidden

#### Scenario: Some subagents failed
- **WHEN** 2 subagents completed and 1 failed
- **THEN** the main status line SHALL display "2 done · 1 failed" with the failure count in red

#### Scenario: No subagents active
- **WHEN** no subagents are running and none have been used in the current turn
- **THEN** the main status line SHALL NOT display any subagent counter information
- **AND** the subagent status bar SHALL NOT be visible
```

