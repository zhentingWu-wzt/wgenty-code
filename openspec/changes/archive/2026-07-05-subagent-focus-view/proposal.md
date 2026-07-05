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
