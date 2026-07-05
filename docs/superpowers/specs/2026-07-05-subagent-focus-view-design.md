---
comet_change: subagent-focus-view
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-05-subagent-focus-view
status: final
---

# Design Doc: Subagent Focus View

## Context

### Problem

子代理执行时，主聊天窗口被内联的树形卡片（`render_subagent_card`）占据大量空间，将子代理执行细节与主 agent 对话流混在一起。每次调用 `task` / `delegate` 工具后，聊天区插入 5-10 行树形卡片，打断阅读节奏。现有的 Ctrl+Shift+T 监控面板是居中弹窗覆盖层，需主动唤起、遮挡全部内容，无法在"看主 agent"和"看子代理"之间快速切换。

### Current Data Flow

```
subagent_loop.rs → emit(SubagentProgress) → daemon shared store
    → TUI poll_subagent_progress() every 500ms
    → AppEvent::SubagentUpdate → subagent_tree.upsert()
    → render: inline card (chat.rs) + status bar + monitor panel (Ctrl+Shift+T)
```

### Existing Code Structure (verified)

- `src/tui/components/detail_view.rs` — `DetailView` struct with `render(f, area, &DetailViewState)`，已验证的事件渲染逻辑（颜色/图标/换行）
- `src/tui/components/subagent_panel_state.rs` — `DetailViewState`（transcript_id, scroll_offset, events, loading, status, total_elapsed_ms, cumulative_tokens, token_budget_k, error_message, round, max_rounds）和 `SubagentPanelState`
- `src/tui/components/subagent_panel.rs` — Ctrl+Shift+T 监控面板（`render(f, area, &SubagentTree, &SubagentPanelState, is_executing)`）
- `src/tui/components/subagent_tree.rs` — `SubagentTree`（upsert, active_count, count_by_status, total_tokens 等）和 `SubagentNode`
- `src/tui/app/mod.rs` — `subagent_panel_visible: bool` + `subagent_panel_state: SubagentPanelState` 字段
- `src/tui/components/chat.rs` — `render_subagent_card` 和 `render_tree_nodes` 函数

### Goal

焦点分离的交互模式：主聊天窗口保持干净，子代理执行过程在需要时才进入查看，且能一键返回。

## Technical Design

### 决策 1: 移除监控面板，焦点视图接管

删除 Ctrl+Shift+T 监控面板（`subagent_panel` 组件）和 `subagent_panel_visible` 状态。焦点视图 + 底部选择栏完全替代。状态条 ↑↓+Enter 是唯一入口。

### 决策 2: 演进 DetailView → FocusView

将 `detail_view.rs` 重命名为 `subagent_focus_view.rs`，在现有代码基础上扩展：header 添加 label、底部子代理选择栏、Tab 切换、实时刷新。`DetailViewState` 重命名为 `FocusViewState` 并扩展。保留已验证的事件渲染逻辑（颜色/图标/换行）。

### 决策 3: 删除 Ctrl+Shift+T 快捷键

移除 Ctrl+Shift+T 快捷键和 `subagent_panel_visible` 状态。焦点视图仅通过状态条 ↑↓+Enter 进入。子代理完成后状态条消失，无法再查看详情（结果已在聊天区工具结果中）。

### 决策 4: 状态管理 — Cached + rebuild

`FocusViewState` 存储所有数据（事件、状态、tokens 等，如现有 `DetailViewState`）。每次 `SubagentUpdate` 轮询时，如果焦点节点有更新，重建缓存数据但保留 UI 状态（scroll_offset、active_area、selector_index）。使用 `auto_scroll` 标志跟踪用户是否跟随最新事件。

### 1. 状态管理（App struct 变更）

**移除字段:**
- `subagent_panel_visible: bool`
- `subagent_panel_state: SubagentPanelState`

**新增字段:**
- `subagent_focus: Option<FocusViewState>` — 焦点视图状态（None = 未激活）
- `subagent_status_bar_selected: usize` — 状态条选中索引

**FocusViewState 结构（从 DetailViewState 演进）:**

```rust
pub struct FocusViewState {
    // Identity
    pub node_id: String,
    pub label: String,
    // Cached data (rebuilt on SubagentUpdate)
    pub events: Vec<SubagentEvent>,
    pub status: SubagentStatus,
    pub elapsed_ms: u64,
    pub cumulative_tokens: u64,
    pub token_budget_k: Option<u64>,
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
    pub error_message: Option<String>,
    pub current_tool: Option<String>,
    pub current_params: Option<String>,
    // UI state (preserved across rebuilds)
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub active_area: FocusArea, // Timeline | Selector
    pub selector_index: usize,
}

pub enum FocusArea {
    Timeline,
    Selector,
}
```

相比现有 `DetailViewState` 的变化：移除 `transcript_id`/`loading`（改用 `node_id`），新增 `label`/`current_tool`/`current_params`/`auto_scroll`/`active_area`/`selector_index`。

### 2. 布局变更（render.rs）

当前布局: `chat | [panel] | status | pending | input`

新布局: `chat | [panel] | status | [subagent_status_bar] | pending | input`

- 状态条区域仅在 `subagent_tree.active_count() > 0` 时分配空间
- 高度 = `min(active_count, 5)` 行
- 当 `subagent_focus.is_some()` 时，替换整个布局为焦点视图全屏渲染

### 3. 状态条组件（subagent_status_bar.rs — 新文件）

- 展示活跃子代理（Running + Pending），每行：状态图标 + label + 当前工具（或 "thinking…"）
- 选中项高亮
- ↑↓ wrap-around 导航
- Enter 触发焦点视图

### 4. 焦点视图组件（subagent_focus_view.rs — 从 detail_view.rs 演进）

```
┌─────────────────────────────────────────┐
│ Header: label · status · time · rounds · tokens │
├─────────────────────────────────────────┤
│                                         │
│ Event Timeline (full, no truncation)    │
│ 💬 Thought / 🛠 Action / ✅❌ ToolResult │
│                                         │
├─────────────────────────────────────────┤
│ Selector: [explore] [plan] [general-p]  │
├─────────────────────────────────────────┤
│ Help: Esc return · Tab switch · ↑↓ scroll │
└─────────────────────────────────────────┘
```

- 复用 DetailView 的事件渲染逻辑（颜色/图标/换行）
- 新增 header 中的 label
- 新增底部选择栏（所有子代理，含已完成）
- Tab 切换 Timeline / Selector 焦点
- 选择栏 ↑↓ + Enter 切换子代理

### 5. 事件处理（event.rs）

**状态条键盘路由（当状态条可见且焦点视图未激活）:**
- ↑↓ → 导航状态条选择
- Enter → 进入焦点视图（构建 FocusViewState）
- 其他键 → 正常输入框处理

**焦点视图键盘路由（当 subagent_focus.is_some()）:**
- Esc → 关闭焦点视图
- Tab → 切换 Timeline / Selector
- Timeline 模式: ↑↓/PageUp/PageDown 滚动, g/G 跳转
- Selector 模式: ↑↓ 选择子代理, Enter 切换
- 鼠标滚动 → Timeline 滚动

**SubagentUpdate 处理:**
- 如果焦点视图激活且焦点节点有更新 → 重建缓存数据，保留 UI 状态
- `auto_scroll` 为 true 时自动滚动到最新事件

### 6. 文件变更清单

| 文件 | 操作 |
|------|------|
| `src/tui/components/detail_view.rs` | 重命名为 `subagent_focus_view.rs`，扩展为焦点视图 |
| `src/tui/components/subagent_status_bar.rs` | **新建** |
| `src/tui/components/subagent_panel.rs` | **删除** |
| `src/tui/components/subagent_panel_state.rs` | **删除**（`node_list()` 移至 `SubagentTree`） |
| `src/tui/components/chat.rs` | 移除 `render_subagent_card` 和 `render_tree_nodes` |
| `src/tui/components/status.rs` | 简化子代理状态展示 |
| `src/tui/components/mod.rs` | 注册新模块，移除旧模块 |
| `src/tui/app/mod.rs` | 移除旧字段，新增 `subagent_focus` 和 `subagent_status_bar_selected` |
| `src/tui/app/render.rs` | 新增状态条区域 + 焦点视图全屏分支 |
| `src/tui/app/event.rs` | 新增状态条 + 焦点视图键盘路由，移除监控面板路由 |
| `src/tui/components/subagent_tree.rs` | 新增 `node_list()` 方法 |

## 关键取舍与风险

- **↑↓ 冲突**: 状态条可见时 ↑↓ 被拦截用于导航，无法在输入框中移动光标。可接受（子代理运行时通常不需要多行编辑）
- **子代理完成后无法查看**: 状态条隐藏后无入口查看已完成子代理详情。结果在聊天区工具结果中
- **Cached rebuild 开销**: 每次 SubagentUpdate 克隆事件列表。事件量通常不大（<100 events），性能可接受

## 测试策略

- **单元测试**: `FocusViewState` 重建逻辑（保留 UI 状态、auto_scroll 行为）、状态条选择 wrap-around、`SubagentTree.node_list()` 正确性
- **集成测试**: 状态条显示/隐藏条件、焦点视图进入/退出流程
- **CI 验证**: `cargo clippy --all-targets -- -D warnings` + `cargo test --all`

## Spec Patch

无需 Spec Patch。经评估，现有三个 delta spec 已完整覆盖设计方案：
- `subagent-focus-view/spec.md` — 焦点视图全屏、事件时间线、选择栏、Tab 切换、Esc 返回 ✓
- `subagent-action-visibility/spec.md` — 移除内联卡片、状态条接管 ✓
- `subagent-status-display/spec.md` — 状态条显示/隐藏、主状态行摘要 ✓

监控面板移除是设计决策（非能力变更），不需要在 capability spec 中记录。
