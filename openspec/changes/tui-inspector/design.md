# Design: TUI Request Context Inspector

## Context

- TUI 已有侧边栏/分屏机制（layout system），子代理有 `focus_view` 全屏 overlay 模式
- 遗留 `task_panel`（Ctrl+T 右侧 30% 面板）已被更完善的 `subagent_tree` + `subagent_status_bar` + `plan_panel` 取代，本 change 将删除它以释放右侧 Split Pane 区域
- `App` struct 已持有 `assembled_system_messages: Vec<ChatMessage>`、`prompt_context: Arc<PromptContext>`、`conversation_history`、`memory_manager`
- 目前 `AssembledInstructions` 只包含 `system_messages: Vec<ChatMessage>`，构建时在 `assemble_instructions()` 中将 10+ 层合并为扁平列表，**层来源信息丢失**
- `build_user_turn_reminder()` 生成 `<system-reminder>` 后由 `process_input_inner()` 消耗，未持久化
- 每轮 user message 由 `build_user_turn_reminder` 返回的 `reminder` + `user_message` 组成，随后 `user_message` 追加到 `conversation_history`

## Goals / Non-Goals

**Goals**
- TUI 内 `F2` 可呼出/关闭右侧 Inspector 面板
- Inspector 内有 5 个 Tab：System Prompt 分层视图、召回记忆、完整 Messages、Hook 注入、Token 统计
- 每层 system prompt 标注清晰的来源信息
- 每轮请求的上下文数据（system messages、memories、reminder）在 App state 中快照保存，支持历史轮次 ↑↓ 浏览
- 无需外部工具或文件即可实时查看

**Non-Goals**
- 编辑/修改任何层的 prompt 内容（只读）
- 导出为文件（复用现有 Export 面板）
- 实时 token counting（字符数 × 启发式系数即可）
- 跨会话持久化上下文快照

## Decisions

### D1 — 数据模型：`LayerMeta` + `TurnContext`

新增两个核心数据结构：

```rust
/// 标注 system prompt 中每一层的来源
pub struct LayerMeta {
    pub label: String,         // e.g. "Layer 1: base_instructions"
    pub source: LayerSource,   // 来源枚举
    pub content: String,       // 该层完整文本
}

pub enum LayerSource {
    Builtin,                        // 内置模板
    ConfigSettings,                 // settings.json 配置
    ConfigFile(PathBuf),            // e.g. ~/.wgenty-code/WGENTY.md
    ProjectFile(PathBuf),           // e.g. <project>/AGENTS.md
    MemoryRecall { scope: String }, // 记忆召回（project/global）
    HookInjection,                  // hook 注入到 system message
    SkillInventory,                 // 技能清单
    Unknown,
}

/// 每轮请求的完整上下文快照
pub struct TurnContext {
    pub turn_index: usize,
    pub assembled_layers: Vec<LayerMeta>,     // 带层标注的 system messages
    pub recalled_memories: Vec<MemoryMeta>,   // 召回的 project+global 记忆
    pub system_reminder: Option<String>,      // <system-reminder>（to_model）
    pub full_messages: Vec<ChatMessage>,       // 完整 API messages 数组
    pub timestamp: Instant,
}
```

`LayerMeta` 存入 `AssembledInstructions`，既向后兼容（仍可 `iter().collect::<ChatMessage>()` 扁平化），又保留层元数据。

### D2 — Inspector 布局：侧边栏 Split Pane

```text
┌─────────────────────────┬─────────────────────┐
│                         │ Inspector        [F2]│
│                         │ ┌─┬──┬──┬──┬──┐     │
│    Chat Panel           │ │L│M │M │H │T │ ←tabs│
│                         │ ├─┴──┴──┴──┴──┘     │
│    (plan / diff         │ │                    │
│     overlays)           │ │  Layer 1: base_ins │
│                         │ │  ───────────────── │
│                         │ │  source: Builtin   │
│                         │ │  ┌─────────────────│
│                         │ │  │You are a coding │
│                         │ │  │agent...         │
│                         │ │  └─────────────────│
│                         │ │  Layer 2: permissi…│
│                         │ │  ...               │
├─────────────────────────┤ │                    │
│  > user input           │ │  Turn 3/5 ↑↓      │
└─────────────────────────┴─────────────────────┘
```

- Inspector 打开时主面板（chat）自动 resize 到 ~65% 宽度
- 关闭时恢复全宽
- Tab 快捷键：Tab / Shift+Tab 切换 tab；↑↓ 切换历史轮次
- 层列表支持 Enter 展开/折叠单层；L 展开/折叠全部

### D3 — Tab 内容规范

| Tab | 显示内容 | 渲染方式 |
|-----|---------|---------|
| **L**ayers（分层视图） | 10 层 system prompt，每层：label + source + 可展开 content | 列表 + 折叠 |
| **M**emories（召回记忆） | 记忆列表：scope 标签 + importance bar + content preview | 列表 |
| **Msgs**（完整 Messages） | JSON 数组 formatted，一行一个 role 标签 | 等宽字体渲染 |
| **H**ooks（Hook 注入） | to_model / to_transcript 原文 | 分区文本 |
| **T**okens（Token 统计） | 每层角色分布表格：layer / chars / est_tokens | 表格 |

Token 估算公式：`ceil(chars / 4.0)`（近似 cl100k_base）。

### D4 — 历史轮次快照

- `App.turn_contexts: Vec<TurnContext>` 维护最近 `TURN_CONTEXT_CAPACITY = 50` 轮
- 每轮 `spawn_agent_turn` 执行完成后，从 `run_agent_loop` 返回时抓取快照
- 快照数据来源：
  - `assembled_layers` → 改造后的 `AssembledInstructions.layers`
  - `recalled_memories` → `PromptContext.memories` + `MemoryManager` 元数据
  - `system_reminder` → `build_user_turn_reminder` 返回值，存入 App 临时字段
  - `full_messages` → `system_messages` + `conversation_history` 拼接
- 历史轮次切换：↑ 上一轮、↓ 下一轮，自动跳转到最新轮

### D5 — 数据流

```
用户输入 → process_input_inner()
  ├→ recall_memories() → PromptContext.memories ← 记忆数据源
  ├→ build_user_turn_reminder() → (reminder, transcript) ← hook 数据源
  │   ↓ 存入 App.latest_reminder
  ├→ assemble_instructions(prompt_context) → AssembledInstructions.layers ← layers 数据源
  │   ↓ 存入 App.assembled_system_messages (兼容现有逻辑)
  └→ run_agent_loop(system_messages, ...) → 完成后
      ├→ 拼接 full_messages = assembled_layers + conversation_history
      ├→ 创建 TurnContext 快照
      └→ push 到 App.turn_contexts
```

### D6 — 渲染实现

```rust
// src/tui/components/inspector.rs
pub struct InspectorComponent {
    pub visible: bool,
    pub active_tab: InspectorTab,
    pub selected_turn: usize,       // 当前查看的轮次索引
    pub expanded_layers: BTreeSet<usize>,  // 已展开的层索引
    pub scroll_offset: usize,
}

enum InspectorTab {
    Layers,
    Memories,
    Messages,
    Hooks,
    Tokens,
}
```

- 实现 `Component` trait（`render` + `handle_event`）
- 渲染公式：`render_header (tabs) → render_body (tabs_content) → render_footer (turn navigation)`
- 与现有 `Layout` 系统集成：Inspector 作为右侧可选面板，存在时调整主面板宽度

### D7 — 测试策略

- `inspector.rs`：组件级单元测试（tab 切换、展开/折叠、历史轮次导航）
- `prompts/mod.rs`：`LayerMeta` 构建测试（每层来源标注正确）
- `tui/app/turn.rs`：`TurnContext` 快照抓取测试（数据完整性）
- `cargo clippy --all-targets -- -D warnings` + `cargo fmt` 通过
- 无新增外部依赖（所有渲染基于现有 ratatui API）

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| `AssembledInstructions` 改造影响现有 `run_agent_loop` API | 保持 `system_messages` 字段不变，新增 `layers` 字段；向后兼容 |
| 内存开销（每轮快照） | Ring buffer 上限 50 轮；每轮 ~100KB 文本上限 ~5MB |
| Inspector 渲染性能（大量文本滚动） | 仅渲染可视区域；rustdoc-style viewport 裁剪 |
| 更新 `assemble_instructions` 签名时传参复杂 | 复用 `PromptContext`，不新增参数 |

## Migration Plan

- 无数据迁移
- 变更不影响 CLI / daemon / API 面
- 向后兼容：不打开 Inspector 时 TUI 行为与现在完全相同
- `AssembledInstructions` 新增 `layers` 字段，现有 `system_messages` 逻辑不变

## Open Questions

1. ~~Tab 命名缩写~~ → 确认使用 L/M/Msg/H/T
2. ~~历史轮次容量~~ → 50 轮（约 5MB 内存）
3. TUI 最小窗口宽度：Inspector 至少需 40 列，窗口 < 80 列时自动折叠不展示
