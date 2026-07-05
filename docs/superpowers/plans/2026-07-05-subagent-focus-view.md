---
change: subagent-focus-view
design-doc: docs/superpowers/specs/2026-07-05-subagent-focus-view-design.md
base-ref: 1a7825b080586921f998a3f033437d5ec53eedd6
---

# Implementation Plan: Subagent Focus View

## 概述

将子代理执行可视化从"内联卡片 + Ctrl+Shift+T 监控面板"重构为"状态条 + 全屏焦点视图"模式。主聊天窗口保持干净，子代理进度通过输入框上方状态条展示，按 Enter 进入全屏焦点视图查看完整事件时间线。

## 现有代码关键结构

- `SubagentProgress` (`src/agent/progress.rs`): node_id, parent_id, label, status, round, max_rounds, current_tool, current_params, events, elapsed_ms, cumulative_tokens, token_budget_k, error_details
- `SubagentEvent` / `SubagentEventType`: Thought{text}, Action{tool_name, params_summary}, ToolResult{tool_name, success, summary}, Error{message, error_type}, Completion{status, summary}
- `SubagentStatus`: Pending, Running, Completed, Failed, Cancelled
- `SubagentTree` (`src/tui/components/subagent_tree.rs`): root_id, nodes: HashMap<String, SubagentNode>; 有 active_count(), count_by_status() 等
- `SubagentNode`: progress: SubagentProgress, children: Vec<String>
- `DetailViewState` (`src/tui/components/subagent_panel_state.rs`): transcript_id, scroll_offset, events, loading, status, total_elapsed_ms, cumulative_tokens, token_budget_k, error_message, round, max_rounds
- `SubagentPanelState`: selected_index, expanded_nodes, scroll_offset, detail_view: Option<DetailViewState>; 有 node_list(tree) 静态方法、build_detail_view(tree)
- `DetailView::render(f, area, &DetailViewState)` (`src/tui/components/detail_view.rs`): 渲染 header + 事件时间线
- App 字段: subagent_tree, subagent_history, subagent_panel_visible, subagent_panel_state
- render.rs 布局: chat | [panel] | status | pending | input; subagent_panel overlay; detail_view 全屏

## 重构策略

采用**先加后删**策略：先创建新组件并接入，确认编译通过后再删除旧组件。每个 Task 产出可编译、可提交的状态。

---

## Task 1: SubagentTree::node_list() 方法

**文件:** `src/tui/components/subagent_tree.rs`

**目标:** 将 `SubagentPanelState::node_list()` 的深度优先遍历逻辑移至 `SubagentTree` 作为实例方法。

**实现:**

```rust
impl SubagentTree {
    /// Depth-first flattened list of all node IDs in the tree.
    pub fn node_list(&self) -> Vec<String> {
        let mut list = Vec::new();
        fn walk(tree: &SubagentTree, node_id: &str, list: &mut Vec<String>) {
            list.push(node_id.to_string());
            if let Some(node) = tree.nodes.get(node_id) {
                for child in &node.children {
                    walk(tree, child, list);
                }
            }
        }
        if let Some(ref root) = self.root_id {
            walk(self, root, &mut list);
        }
        list
    }
}
```

**测试:** 在 `subagent_tree.rs` 的 `#[cfg(test)]` 模块中添加：
- `test_node_list_flat` — 单根无子节点
- `test_node_list_tree` — 根 + 2 子 + 1 孙，验证 DFS 顺序
- `test_node_list_empty` — 空树返回空 Vec

**验收:** `cargo test subagent_tree` 通过。

---

## Task 2: FocusViewState 与 FocusArea 类型

**文件:** `src/tui/components/subagent_focus_view.rs` (新建)

**目标:** 创建从 `DetailViewState` 演进的状态类型，支持焦点视图所需的全部数据。

**实现:**

```rust
use crate::agent::progress::{SubagentEvent, SubagentStatus};
use crate::tui::components::subagent_tree::SubagentTree;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusArea {
    Timeline,
    Selector,
}

#[derive(Debug, Clone)]
pub struct FocusViewState {
    pub node_id: String,
    pub label: String,
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
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub active_area: FocusArea,
    pub selector_index: usize,
}
```

**方法:**

```rust
impl FocusViewState {
    /// Build a FocusViewState from a node in the tree.
    pub fn build(node_id: &str, tree: &SubagentTree) -> Option<Self> {
        let node = tree.nodes.get(node_id)?;
        let p = &node.progress;
        Some(Self {
            node_id: node_id.to_string(),
            label: p.label.clone(),
            events: p.events.clone(),
            status: p.status.clone(),
            elapsed_ms: p.elapsed_ms,
            cumulative_tokens: p.cumulative_tokens,
            token_budget_k: p.token_budget_k,
            round: p.round,
            max_rounds: p.max_rounds,
            error_message: p.error_details.as_ref().map(|e| e.message.clone()),
            current_tool: p.current_tool.clone(),
            current_params: p.current_params.clone(),
            scroll_offset: 0,
            auto_scroll: true,
            active_area: FocusArea::Timeline,
            selector_index: 0,
        })
    }

    /// Rebuild cached data from tree, preserving UI state.
    pub fn rebuild(&mut self, tree: &SubagentTree) {
        if let Some(node) = tree.nodes.get(&self.node_id) {
            let p = &node.progress;
            self.events = p.events.clone();
            self.status = p.status.clone();
            self.elapsed_ms = p.elapsed_ms;
            self.cumulative_tokens = p.cumulative_tokens;
            self.current_tool = p.current_tool.clone();
            self.current_params = p.current_params.clone();
            self.error_message = p.error_details.as_ref().map(|e| e.message.clone());
        }
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }
}
```

**测试:**
- `test_build_from_node` — 从有事件的节点构建，验证字段映射
- `test_rebuild_preserves_ui_state` — rebuild 后 scroll_offset 和 active_area 不变
- `test_rebuild_auto_scroll_resets` — auto_scroll=true 时 scroll_offset 归零

**验收:** `cargo test subagent_focus_view` 通过（需要先在 mod.rs 注册模块）。

---

## Task 3: FocusView 渲染组件

**文件:** `src/tui/components/subagent_focus_view.rs` (续)

**目标:** 从 `DetailView::render` 演进，添加 label header、底部选择栏、Tab 区域指示。

**实现要点:**

```rust
pub struct FocusView;

impl FocusView {
    pub fn render(
        f: &mut Frame,
        area: Rect,
        state: &FocusViewState,
        tree: &SubagentTree,
    ) {
        // 布局: header | timeline | selector | help
        // header: label · status · time · rounds · tokens
        // timeline: 复用 DetailView 的事件渲染逻辑 (颜色/图标/换行)
        // selector: 所有子代理列表，高亮当前选中
        // help: Esc return · Tab switch · ↑↓ scroll
    }
}
```

- 复用 `DetailView` 的事件渲染逻辑（Thought 💬 / Action 🛠 / ToolResult ✅❌ / Error 颜色方案）
- header 新增 label 字段
- selector 区域: `tree.node_list()` 获取全部节点，显示状态图标 + label，selector_index 高亮
- active_area 用边框颜色区分 Timeline（高亮）vs Selector（高亮）

**验收:** 编译通过，渲染逻辑通过手动验证。

---

## Task 4: SubagentStatusBar 渲染组件

**文件:** `src/tui/components/subagent_status_bar.rs` (新建)

**目标:** 输入框上方紧凑状态条，展示活跃子代理列表。

**实现:**

```rust
use crate::agent::progress::SubagentStatus;
use crate::tui::components::subagent_tree::SubagentTree;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;

pub fn render(
    f: &mut Frame,
    area: Rect,
    tree: &SubagentTree,
    selected_index: usize,
) {
    // 收集活跃子代理 (Running + Pending)
    let active: Vec<_> = tree.nodes.values()
        .filter(|n| matches!(n.progress.status,
            SubagentStatus::Running | SubagentStatus::Pending))
        .collect();

    // 每行: 状态图标 + label + 当前工具 (或 "thinking…")
    // 选中项高亮 (selected_index % active.len())
}
```

**验收:** 编译通过，状态条在有活跃子代理时显示。

---

## Task 5: App struct 与模块注册

**文件:** `src/tui/app/mod.rs`, `src/tui/components/mod.rs`

**目标:** 添加新字段，注册新模块。

**mod.rs 变更:**
```rust
// 新增
pub mod subagent_focus_view;
pub mod subagent_status_bar;
// 保留旧模块（Task 9 删除）
```

**App struct 变更:**
```rust
// 新增字段
pub subagent_focus: Option<FocusViewState>,
pub subagent_status_bar_selected: usize,
```

**验收:** `cargo check` 通过。

---

## Task 6: render.rs 布局变更

**文件:** `src/tui/app/render.rs`

**目标:** 添加状态条区域 + 焦点视图全屏分支。

**实现要点:**

1. 在 `render()` 开头添加焦点视图全屏分支:
```rust
if let Some(ref focus) = self.subagent_focus {
    components::subagent_focus_view::FocusView::render(
        f, f.area(), focus, &self.subagent_tree,
    );
    return;
}
```

2. 在布局 constraints 中添加状态条区域（status 与 pending 之间）:
```rust
let status_bar_height = self.subagent_tree.active_count().min(5) as u16;
let has_status_bar = status_bar_height > 0;
// constraints 增加 Constraint::Length(status_bar_height)
```

3. 渲染状态条:
```rust
if has_status_bar {
    components::subagent_status_bar::render(
        f, layout[status_bar_idx],
        &self.subagent_tree, self.subagent_status_bar_selected,
    );
}
```

**验收:** `cargo check` 通过。

---

## Task 7: event.rs — 状态条键盘路由

**文件:** `src/tui/app/event.rs`

**目标:** 状态条可见时拦截 ↑↓ + Enter，移除 Ctrl+Shift+T。

**实现要点:**

```rust
// 状态条可见且焦点视图未激活时:
let active_count = self.subagent_tree.active_count();
if active_count > 0 && self.subagent_focus.is_none() {
    match key_event.code {
        KeyCode::Up => {
            // wrap-around 导航
            self.subagent_status_bar_selected =
                (self.subagent_status_bar_selected + active_count - 1) % active_count;
            return; // 不传递到输入框
        }
        KeyCode::Down => {
            self.subagent_status_bar_selected =
                (self.subagent_status_bar_selected + 1) % active_count;
            return;
        }
        KeyCode::Enter => {
            // 构建 FocusViewState
            let active: Vec<_> = /* 收集活跃节点 ID */;
            if let Some(node_id) = active.get(self.subagent_status_bar_selected) {
                if let Some(state) = FocusViewState::build(node_id, &self.subagent_tree) {
                    self.subagent_focus = Some(state);
                }
            }
            return;
        }
        _ => {} // 其他键继续到输入框
    }
}
```

- 移除 Ctrl+Shift+T → `subagent_panel_visible = !subagent_panel_visible` 的代码

**验收:** `cargo check` 通过。

---

## Task 8: event.rs — 焦点视图键盘路由 + SubagentUpdate 重建

**文件:** `src/tui/app/event.rs`

**目标:** 焦点视图激活时的键盘处理 + 实时刷新。

**实现要点:**

```rust
if let Some(ref mut focus) = self.subagent_focus {
    match key_event.code {
        KeyCode::Esc => { self.subagent_focus = None; return; }
        KeyCode::Tab => {
            focus.active_area = match focus.active_area {
                FocusArea::Timeline => FocusArea::Selector,
                FocusArea::Selector => FocusArea::Timeline,
            };
            return;
        }
        KeyCode::Up | KeyCode::Down if focus.active_area == FocusArea::Timeline => {
            // 滚动事件时间线, auto_scroll = false
            focus.auto_scroll = false;
            // scroll_offset += / -=
            return;
        }
        KeyCode::Up | KeyCode::Down if focus.active_area == FocusArea::Selector => {
            // 导航选择栏, wrap-around
            let list = self.subagent_tree.node_list();
            // selector_index += / -=
            return;
        }
        KeyCode::Enter if focus.active_area == FocusArea::Selector => {
            // 切换到选中的子代理
            let list = self.subagent_tree.node_list();
            if let Some(node_id) = list.get(focus.selector_index) {
                if let Some(new_state) = FocusViewState::build(node_id, &self.subagent_tree) {
                    *focus = new_state;
                }
            }
            return;
        }
        _ => return; // 焦点视图激活时吞掉所有其他键
    }
}
```

**SubagentUpdate 处理:**
```rust
AppEvent::SubagentUpdate => {
    // ... 现有的 tree.upsert 逻辑 ...
    if let Some(ref mut focus) = self.subagent_focus {
        focus.rebuild(&self.subagent_tree);
    }
}
```

**验收:** `cargo check` 通过。

---

## Task 9: 移除旧组件

**文件:** `src/tui/components/chat.rs`, `src/tui/components/mod.rs`, `src/tui/app/mod.rs`, `src/tui/app/render.rs`, `src/tui/components/status.rs`

**目标:** 删除内联卡片、监控面板、旧 DetailView。

**操作:**

1. **chat.rs**: 移除 `render_subagent_card()` 和 `render_tree_nodes()` 函数；移除 `render()` 中 `tool_name == "task" || "delegate"` 分支的内联卡片调用
2. **render.rs**: 移除 `subagent_panel_visible` overlay 渲染块；移除 `detail_view` 全屏渲染块（已被 FocusView 替代）
3. **app/mod.rs**: 移除 `subagent_panel_visible: bool` 和 `subagent_panel_state: SubagentPanelState` 字段；移除相关 import
4. **components/mod.rs**: 移除 `pub mod subagent_panel;` 和 `pub mod subagent_panel_state;` 和 `pub mod detail_view;`
5. **status.rs**: 简化子代理状态展示（移除详细计数或仅保留总计数）
6. **删除文件**: `src/tui/components/subagent_panel.rs`, `src/tui/components/subagent_panel_state.rs`, `src/tui/components/detail_view.rs`
7. **event.rs**: 移除所有引用 `subagent_panel_state`、`subagent_panel_visible`、`detail_view` 的代码

**验收:** `cargo check` 通过，无残留引用。

---

## Task 10: 验证

**命令:**
```bash
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo fmt -- --check
```

**验收:** 三项全部通过，零 warning。
