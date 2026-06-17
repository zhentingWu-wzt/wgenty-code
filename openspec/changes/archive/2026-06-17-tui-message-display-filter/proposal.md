## Why

当前 TUI 的 session 历史恢复和正常对话中，**系统提示词**（约 8 层组装指令）和**工具结果**（如文件内容、命令输出）会直接渲染在聊天窗口中。这导致：

1. **Session 恢复时噪声大**：恢复历史 session 后，8 层系统提示词全部以 dim 颜色展示，占据大量屏幕空间，干扰用户回顾对话内容
2. **工具结果干扰阅读**：经常性的工具结果（Read 文件内容、Bash 输出等）夹在对话中，打断用户阅读 agent 回复的连贯性
3. **信息密度低**：用户核心关注的是「与 agent 的对话」，而非系统内部指令或工具执行的原始输出

## What Changes

修改 TUI 的消息渲染逻辑（`src/tui/components/chat.rs` 和 `src/tui/app/event.rs`），实现：

- **系统提示词不展示**：`role: "system"` 的消息在渲染时完全跳过
- **工具结果默认折叠**：`role: "tool"` 的消息默认显示为一行可展开摘要（格式：`tool_name result (N lines)`），用户按 Enter 可展开查看完整内容
- **工具调用正常展示**：`role: "assistant"` 中的 `tool_use` blocks 保持当前展示行为（工具名 + 核心参数）

### 影响范围

- **正常对话渲染**：`src/tui/components/chat.rs` 中的消息渲染逻辑
- **Session 历史恢复**：`src/tui/app/event.rs` 中 `HistoryLoaded` handler 的消息转换逻辑

### 不修改

- `conversation_history` 数据结构
- Session 文件的存储格式
- Anthropic API 调用逻辑（已正确处理 system 消息）
- `/clear`、auto-compaction 行为

## Capabilities

### Modified Capabilities

- `tui-command-completion`：消息渲染模块增加 system/tool 消息的过滤和折叠能力

## Impact

- **修改文件**：
  - `src/tui/components/chat.rs`：消息渲染逻辑（system 跳过、tool 折叠）
  - `src/tui/app/event.rs`：`HistoryLoaded` handler 中 `MessageRole` 转换逻辑
- **可能新增**：
  - `src/tui/components/chat.rs` 中新增 tool-result 折叠渲染方法
- **风险**：低。纯显示层修改，不影响数据层和 API 层
