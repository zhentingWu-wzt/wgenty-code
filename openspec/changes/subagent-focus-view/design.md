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
