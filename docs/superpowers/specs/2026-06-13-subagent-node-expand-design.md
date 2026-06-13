# Subagent Node Expand — Interactive Full History

**Date:** 2026-06-13
**Status:** Design approved, pending implementation

## Problem

当前 `SubagentPanel` 是纯展示覆盖层：
- 每个节点只显示最近 **3** 条工具调用
- 历史模型文本（think→call→think→call 链路中的 "think"）只保留最新一条
- 面板无任何交互——用户无法看到子代理的完整执行轨迹

用户需要能够在面板中导航、展开节点，查看完整的工具调用 + 模型文本历史。

## Design

### 数据模型：`SubagentAction` → `SubagentEvent`

当前 `action_log` 只存工具调用（`SubagentAction`），文本快照 `text_snapshot` 独立存储且每次覆盖。为保留完整 think→call 链路，将事件统一为枚举：

```rust
/// 子代理执行过程中的一个事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEvent {
    pub event_type: SubagentEventType,
    /// 事件发生时的已用时间（ms，从 subagent 启动算起）
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubagentEventType {
    /// 模型输出的文本（分析、计划、结论）
    Thought { text: String },           // 截断到 200 chars
    /// 模型调用了一个工具
    Action {
        tool_name: String,              // e.g., "file_read"
        params_summary: String,         // e.g., "src/auth.rs"
    },
}
```

- `action_log: Vec<SubagentEvent>` 上限 **50 条**（约 10KB JSON per node）
- 按时间正序排列（最早→最新），emit 时追加到末尾
- `text_snapshot` 字段保留 → 始终为最新一条 `Thought` 的 text

### 渲染：收起 vs 展开

**收起状态**（紧凑，当前行为不变）：
```
📁 task: 重构 auth — round 3/10 · 12.3s
   💬 "需要在 3 个地方修改..."
   ▸ file_read("src/auth.rs")
   ▸ grep("fn authenticate")
   ▸ file_read("src/main.rs")
```

**展开状态**（按 Enter 切换）：
```
▶ 📁 task: 重构 auth — round 3/10 · 12.3s         ← 选中 + 展开
   │
   💭 "需要先找到认证逻辑的入口点"          0.8s
   │
   ▸ grep("fn authenticate")              1.2s
   │
   💭 "找到了 3 处定义，主要在 auth.rs"    2.5s
   │
   ▸ file_read("src/auth.rs")             3.1s
   │
   💭 "这个模块需要重构，拆分职责..."      5.8s
   │
   ▸ file_read("src/auth/tests.rs")       6.2s
   │
   💭 "测试覆盖了主要路径，可以放心改"     7.9s
   ▼
```

### 交互

| 按键 | 行为 |
|------|------|
| `j` / `↓` | 移动到下一个节点 |
| `k` / `↑` | 移动到上一个节点 |
| `Enter` | 切换当前节点的展开/折叠 |
| `Esc` / `Ctrl+Shift+T` | 关闭面板 |
| `g` | 跳到第一个节点 |
| `G` | 跳到最后一个节点 |

- 选中节点以高亮边框或背景色区分
- 面板底部：操作提示栏 `[↑↓ navigate · Enter expand · Esc close]`
- 面板内部滚动由展开内容自动撑开，ratatui 的 `Paragraph` 配合固定视口高度处理

### 状态管理

`App` 新增 `SubagentPanelState`：

```rust
struct SubagentPanelState {
    selected_index: usize,           // 当前选中的节点索引（扁平化顺序）
    expanded_nodes: HashSet<String>, // 已展开的 node_id 集合
    scroll_offset: u16,              // 面板内容垂直滚动偏移
}

impl SubagentPanelState {
    fn node_list(tree: &SubagentTree) -> Vec<String>; // 深度优先扁平化
    fn move_up(&mut self, tree: &SubagentTree);
    fn move_down(&mut self, tree: &SubagentTree);
    fn toggle_expand(&mut self);
    fn is_expanded(&self, node_id: &str) -> bool;
}
```

`AppEvent` 新增键盘事件分发：
```rust
AppEvent::SubagentPanelKey(KeyEvent),     // 面板可见时的按键
```

事件处理流程：
```
KeyEvent → SubagentPanelKey
  → subagent_panel_state.move_up/down/toggle
  → 重新渲染面板
```

### 变更范围

| 文件 | 变更 |
|------|------|
| `agent/progress.rs` | `SubagentAction` → `SubagentEvent` enum + `action_log` 上限 50 |
| `teams/subagent_loop.rs` | 每轮 API 响应后追加 `Thought` 事件 + 工具调用时追加 `Action` 事件 |
| `tui/components/subagent_panel.rs` | 重写：有状态组件，渲染选中 + 展开逻辑 |
| `tui/app/mod.rs` | 新增 `SubagentPanelState` 字段 |
| `tui/app/types.rs` | 新增 `SubagentPanelKey` 事件 |
| `tui/app/event.rs` | 键盘事件分发 → subagent panel |

### Non-Goals

- 不支持鼠标点击选择节点
- 不支持在展开视图中过滤/搜索
- 不改变树结构的层级展示
- 展开状态不跨 turn 持久化（面板关闭或新 turn 开始即重置）

## Self-Review

- [x] 无 TBD/TODO
- [x] 数据模型与渲染逻辑一致（`SubagentEvent` 同时服务存储和展示）
- [x] 范围聚焦：仅交互式展开，不涉及树结构改动
- [x] 无歧义：按键绑定、展开行为、上限值均已明确
