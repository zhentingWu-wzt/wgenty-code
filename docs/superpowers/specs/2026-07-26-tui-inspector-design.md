# Design Doc: TUI Request Context Inspector

> 2026-07-26 | Change: `tui-inspector` | Phase: design

## 1. Problem Statement

wgenty-code 的 TUI 已有完善的工具调用展示、diff 渲染、计划面板、子代理追踪等交互组件，但**缺失对 Agent 内部视角的可见性**。用户无法实时了解：

- Agent 收到了哪些系统指令（10 层 system prompt 各来自哪里）
- 本轮召回了哪些记忆（project/global，importance，TF-IDF score）
- hook 系统注入了什么内容（`<system-reminder>` 的 to_model / to_transcript）
- 完整发给 LLM 的 messages 数组是什么样子

当前唯一调试手段是 `prompt.debug_dump_reminder` 写文件——只包含 `<system-reminder>`，不含 system cascade，且需要切换工具查看。

## 2. Goals

1. TUI 内 `F2` 呼出右侧 Inspector 面板，实时展示本轮请求上下文
2. 5 个 Tab：System Prompt 分层 / 召回记忆 / 完整 Messages / Hook 注入 / Token 统计
3. 每层 system prompt 标注来源（Builtin / ConfigFile / ProjectFile / MemoryRecall...）
4. 支持浏览历史轮次上下文快照（最近 50 轮）
5. 不增加外部依赖，不写入磁盘，不影响现有 TUI 行为

## 3. Architecture

### 3.1 Data Model

```rust
/// 标注每层 system prompt 的来源
pub enum LayerSource {
    Builtin,
    ConfigSettings,
    ConfigFile(PathBuf),        // ~/.wgenty-code/WGENTY.md, rules/*.md
    ProjectFile(PathBuf),       // <project>/AGENTS.md, WGENTY.md
    MemoryRecall { scope: String },  // project | global
    SkillInventory,
    HookInjection,
    Unknown,
}

pub struct LayerMeta {
    pub label: String,          // "Layer 1: base_instructions"
    pub source: LayerSource,
    pub content: String,        // 该层完整文本
    pub char_count: usize,
}

/// 单轮请求的完整上下文快照
pub struct TurnContext {
    pub turn_index: usize,
    pub assembled_layers: Vec<LayerMeta>,
    pub recalled_memories: Vec<MemoryMeta>,
    pub system_reminder: Option<ReminderOutput>,
    pub full_messages: Vec<ChatMessage>,
    pub captured_at: Instant,
}
```

### 3.2 Data Flow — Real-time Capture

```
┌─ process_input_inner() ──────────────────────────────────────┐
│                                                               │
│  1. recall_memories() ──→ PromptContext.memories              │
│     ↓ (记忆召回完成，数据就绪)                                 │
│  2. build_user_turn_reminder() ──→ ReminderOutput             │
│     ↓ (hook 注入完成)                                         │
│  3. assemble_instructions(ctx) ──→ AssembledInstructions      │
│     ↓                           (.layers + .system_messages)  │
│     ├─ stash: App.pending_context = PartialTurnContext {      │
│     │     layers, memories, reminder                          │
│     │   }                                                     │
│     └─→ 继续发送 assembled_system_messages 给 run_agent_loop  │
│                                                               │
│  4. run_agent_loop(...) ──→ assistant response                │
│     ↓ (LLM 回复完成)                                          │
│  5. complete snapshot:                                        │
│     full_messages = assembled_layers + conversation_history   │
│     turn_contexts.push(TurnContext { ... })                   │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

**关键点**：
- 步骤 1-3 在 agent loop 启动前完成，Inspector 可在 streaming 期间查看本轮上下文（不含 assistant response）
- 步骤 5 在 agent loop 返回后完成，补充 assistant 消息到 `full_messages`
- 新增的 `PartialTurnContext` 是临时状态，不暴露给 Inspector（Inspector 只看 completed `TurnContext`）
- 过程完全基于共享的 App state，不增加 API 调用、文件 I/O

### 3.3 Component Architecture

```
src/tui/
├── components/
│   └── inspector.rs           # NEW: InspectorComponent
│       ├── InspectorComponent  # struct { visible, active_tab, selected_turn, ... }
│       ├── impl Component       # render() + handle_event()
│       ├── render_tabs_header() # L | M | Msg | H | T
│       ├── render_layers_tab()  # 分层树形视图
│       ├── render_memories_tab()# 记忆列表
│       ├── render_messages_tab()# JSON 数组等宽渲染
│       ├── render_hooks_tab()   # to_model / to_transcript
│       └── render_tokens_tab()  # 统计表格
├── app/
│   ├── mod.rs                  # App 增加字段
│   │   ├── + turn_contexts: Vec<TurnContext>  # 最近 50 轮
│   │   └── + pending_context: Option<PartialTurnContext>
│   ├── turn.rs                 # spawn_agent_turn 内捕获快照
│   ├── render.rs               # Inspector 渲染调度
│   └── layout.rs (in render.rs)# 右侧 Split Pane 布局
└── components/mod.rs           # 注册 InspectorComponent

src/prompts/
└── mod.rs                      # LayerMeta + LayerSource + AssembledInstructions 改造
```

### 3.4 Layout Integration

```
正常模式:                          Inspector 打开 (F2):
┌──────────────────────┐          ┌────────────────┬──────────┐
│                      │          │                │Inspector │
│    Chat + Tool       │          │   Chat (65%)   │  L M ... │
│                      │          │                │  ─────── │
│                      │          │                │ Layer 1  │
├──────────────────────┤          │                │ Layer 2  │
│  plan_panel / prompt │          │                │ ...      │
├──────────────────────┤          ├────────────────┤          │
│  > input             │          │  input          │ Turn 3/5 │
└──────────────────────┘          └────────────────┴──────────┘
```

- Inspector 可见时：chat 区缩减为 65%，Inspector 固定 35% 右侧
- Inspector 关闭时：chat 区恢复 100%
- 冲突处理（方案3：最低优先级）：
  - subagent_focus_view 弹出 → Inspector 自动折叠
  - plan_panel 展开 → 不影响 Inspector（plan 是水平条，不冲突）
  - 冲突面板关闭后 → Inspector 恢复（保持用户 F2 状态记忆）
- **前置清理**：删除遗留 `task_panel`（被 `subagent_tree` + `status_bar` + `plan_panel` 取代），释放右侧 Split Pane 区域

### 3.5 Interaction Matrix

| 事件 | Inspector 行为 |
|------|---------------|
| F2 | 切换 Inspector 可见/隐藏。隐藏时保留所有状态（tab + 当前轮次 + 展开状态） |
| Tab | 切换到下一个 Tab，循环 |
| Shift+Tab | 切换到上一个 Tab |
| ↑ (in Inspector) | 查看上一轮快照 |
| ↓ (in Inspector) | 查看下一轮快照 |
| Enter (Layers tab) | 折叠/展开当前层 |
| L (Layers tab) | 展开/折叠全部层 |
| J / K | 在长列表中滚动（Tab 特定） |
| PgUp / PgDn | 页面滚动 |
| 新轮完成 | **不自动跳转**——用户手动浏览历史时停留在当前轮次 |

## 4. Key Design Decisions

### D1 — `assemble_instructions` 改造：保持向后兼容

```rust
pub struct AssembledInstructions {
    /// 向后兼容：现有 consumer 仍然使用这个
    pub system_messages: Vec<ChatMessage>,
    /// 新增：Inspector 使用
    pub layers: Vec<LayerMeta>,
}
```

- `system_messages` 行为不变（扁平化所有层）
- `layers` 仅在 `assemble_instructions` 内部填充，每层调用 `build_xxx_layer()` 后立即 add layer
- 所有调用方（`process_input_inner`、`daemon`、`headless`）无需修改——它们继续使用 `system_messages`

### D2 — TurnContext 生命周期

```
spawn_agent_turn()
  ├→ process_input_inner()
  │   ├→ mem_recall + hook + assemble ──→ stash PartialTurnContext
  │   └→ run_agent_loop()
  │       └→ llm response + finalize
  └→ complete PartialTurnContext → TurnContext → push ring buffer
```

- Ring buffer 容量 `TURN_CONTEXT_CAPACITY = 50`
- 溢出：`remove(0)` 丢弃最旧
- 内存估算：50 轮 × ~100KB ≈ 5MB（在可接受范围）
- PartialTurnContext 只在 process_input_inner 和 spawn_agent_turn 之间短暂存在

### D3 — Token 估算

- 公式：`ceil(chars / 4.0)`（cl100k_base 启发式）
- 提供 `measured_tokens: Option<usize>` 字段——当 API 返回 `usage.prompt_tokens` 时填充
- Token Tab 显示：layer | chars | est_tokens | measured_tokens
- 当有 measured 值时突出显示（颜色标记）

### D4 — 渲染性能

- Inspector 的高开销场景是 **Messages Tab**——完整 messages 数组可能数千行
- 策略：仅渲染可视区域的 `Rect.height` 行（viewport clipping）
- 滚动偏移 `scroll_offset` 存储在 Inspector state 中
- 切换 Tab 时重置 scroll_offset

### D5 — 主题集成

新增颜色常量（`src/tui/theme.rs`）：
```rust
pub const INSPECTOR_BORDER: Color = Color::Rgb(100, 100, 140);
pub const TAB_ACTIVE: Color = Color::Rgb(100, 200, 255);
pub const TAB_DIM: Color = Color::Rgb(80, 80, 80);
pub const SOURCE_BADGE_BUILTIN: Color = Color::Rgb(60, 120, 200);
pub const SOURCE_BADGE_FILE: Color = Color::Rgb(200, 160, 60);
pub const SOURCE_BADGE_MEMORY: Color = Color::Rgb(100, 200, 120);
pub const IMPORTANCE_HIGH: Color = Color::Rgb(200, 80, 80);
pub const IMPORTANCE_LOW: Color = Color::Rgb(80, 80, 80);
```

## 5. Implementation Order

推荐实现顺序（对应 tasks.md 7 sections）：

0. **删除遗留 task_panel**（0.1-0.5）——清理废旧组件，释放右侧面板区域
1. **数据模型层**（1.1-1.4）——不渲染，只改数据结构
2. **轮次快照**（2.1-2.4）——数据流打通
3. **Inspector 组件**（3.1-3.9）——核心 UI，从最简单的 Tab 开始
4. **布局集成**（4.1-4.5）——接入 TUI layout
5. **主题**（5.1-5.2）
6. **测试**（6.1-6.6）

建议先实现 Layers Tab（最核心）+ 布局集成，然后逐步补完其他 Tab。

## 6. Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| `assemble_instructions` 签名字段增加导致编译错误传播 | Low | `system_messages` 不变，新增 `layers` 字段，调用方无需改 |
| Inspector 内存 5MB 在低内存环境不可接受 | Low | 可降低 CAPACITY 到 20 或按需分配 |
| Messages Tab 大数组渲染卡顿 | Medium | viewport clipping + ratatui 性能已足够；极端场景（>10000 行）加 `max_chars_per_message` 截断 |
| PartialTurnContext 生命周期管理遗漏导致内存泄漏 | Low | `Option<PartialTurnContext>` 在每次新轮开始前 `take()` 清除 |

## 7. Open Questions

1. 是否需要为 Inspector 加一个 `E` 导出键（将当前 TurnContext 导出为 JSON 文件）？→ **暂不实现**，后续可加
2. 50 轮快照上限是否合适？→ 先 50，观察使用反馈调整
