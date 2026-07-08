## Why

当前 TUI 输入框下方的模式标签栏（`NORMAL`/`PLAN`/`ACCEPT EDIT`/`YOLO`）只显示当前模式，用户无法直观感知当前对话已消耗了多少上下文窗口。随着对话增长，用户可能在不知情的情况下接近上下文上限，导致模型"遗忘"早期内容或触发自动压缩。在模式标签旁添加一个上下文占比指示器，让用户实时了解上下文使用情况。

## What Changes

- **TokenCounter 扩展**：新增 `last_prompt_tokens` 原子计数器，记录最近一次 API 响应中的 `prompt_tokens`（即当前上下文窗口中实际发送给模型的 token 总量，含系统提示词、对话历史、工具定义等）
- **API 用量记录**：在 `AgentLoop::run_agent_loop` 的 token accounting 处，将 `usage.prompt_tokens` 存入 `last_prompt_tokens`（保留现有 `used`/`turn_input`/`turn_output` 不变）
- **上下文窗口配置**：在 `Settings` 中新增 `models.context_window` 字段（默认 200000），作为占比计算的分母
- **进度条组件**：新增 `src/tui/components/context_bar.rs`，渲染 Unicode 进度条 + 百分比，颜色随阈值变化（绿 <50%，黄 50-80%，红 >80%）
- **模式标签栏扩展**：`render_mode_label` 在模式标签右侧渲染上下文进度条，窄终端（宽度 <40）时自动隐藏

## Capabilities

### New Capabilities

- `context-usage-indicator`: 在 TUI 输入框模式标签栏右侧显示当前上下文窗口占比，以进度条+百分比形式呈现，颜色随使用率阈值变化

### Modified Capabilities

<!-- 无现有 spec 受影响 -->

## Impact

| 文件 | 变更 |
|------|------|
| `src/api/token_counter.rs` | 新增 `last_prompt_tokens` 字段及 `set_prompt_tokens`/`last_prompt_tokens` 方法 |
| `src/tui/agent/core.rs` | token accounting 处增加 `usage.prompt_tokens` → `token_counter.set_prompt_tokens()` |
| `src/config/mod.rs` | `Settings`/`ModelConfig` 新增 `context_window` 字段（默认 200000） |
| `src/tui/components/context_bar.rs` | **新增**：进度条渲染组件，含颜色阈值逻辑 |
| `src/tui/components/mod.rs` | 注册 `context_bar` 模块 |
| `src/tui/app/render.rs` | `render_mode_label` 扩展为在模式标签右侧渲染上下文进度条 |
| `src/tui/app/mod.rs` | App 传入 `token_counter` 和 context window 配置给渲染 |
