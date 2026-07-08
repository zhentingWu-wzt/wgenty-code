# Comet Design Handoff

- Change: tui-context-usage-indicator
- Phase: design
- Mode: compact
- Context hash: 6d0eebf333110a38534ab648f254064938223b58c10a9ec6b7bdacaa8001d957

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/tui-context-usage-indicator/proposal.md

- Source: openspec/changes/tui-context-usage-indicator/proposal.md
- Lines: 1-33
- SHA256: c7b36b6755c93639c207b37306b27adeaa8244a49364e1742d6a45d7a8367a8d

```md
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
```

## openspec/changes/tui-context-usage-indicator/design.md

- Source: openspec/changes/tui-context-usage-indicator/design.md
- Lines: 1-59
- SHA256: 07360d2aecf9e88a7c74e729b51d3376c943c9bf35a90104ffcd8df8c348d086

```md
## Context

当前 TUI 输入框下方的模式标签栏（`render_mode_label`）仅显示 ` NORMAL `（或 PLAN/ACCEPT EDIT/YOLO），右侧空间未利用。用户无法直观感知当前对话消耗了多少上下文窗口。

API 响应中的 `Usage` 结构体已包含 `prompt_tokens`（即发送给模型的全部 token，含系统提示词、对话历史、工具定义），但目前仅 `total_tokens` 和 `completion_tokens` 被记录，`prompt_tokens` 被丢弃。`prompt_tokens` 是最准确的当前上下文大小指标。

`TokenCounter` 已在 `App` 和 `AgentLoop` 之间通过 `Arc` 共享，是扩展 token 跟踪的天然位置。

## Goals / Non-Goals

**Goals:**
- 在模式标签栏右侧渲染上下文占比进度条 + 百分比（如 `▓▓▓░░░░░░ 32%`）
- 进度条颜色随阈值变化：绿色 <50%，黄色 50-80%，红色 >80%
- 上下文窗口上限可配置（`settings.json` 的 `models.context_window`，默认 200000）
- 数据源使用 API 报告的 `prompt_tokens`（最准确）
- 窄终端自动隐藏进度条

**Non-Goals:**
- 不修改现有的 `used`/`turn_input`/`turn_output` 计数器行为
- 不修改 TypeScript Ink 前端
- 不实现 token 精确计数（使用 API 报告值，不引入 tokenizer）
- 不实现自动压缩触发逻辑（仅展示，不干预）

## Decisions

### D1: TokenCounter 扩展

**选择**：在 `TokenCounter` 新增 `last_prompt_tokens: Arc<AtomicUsize>` 字段，记录最近一次 API 响应的 `prompt_tokens`。

**理由**：`TokenCounter` 已在 `App` 和 `AgentLoop` 间共享，扩展它避免引入新通道。`prompt_tokens` 是 API 报告的实际上下文大小，比估算更准确。

### D2: 进度条使用 Unicode 块字符

**选择**：使用 `▓`（填充）和 `░`（空）字符渲染进度条，宽度固定 8 格。

**理由**：零依赖，终端兼容性好，视觉效果清晰。固定宽度避免布局抖动。

### D3: 颜色阈值

**选择**：绿色 <50%，黄色 50-80%，红色 >80%。

**理由**：直观反映上下文健康状态，红色警示用户即将接近上限。

### D4: 可配置上下文窗口

**选择**：`Settings` 新增 `models.context_window: usize`（默认 200000）。

**理由**：不同模型上下文窗口不同，用户可按需调整。

## Risks / Trade-offs

- **[更新延迟]** `prompt_tokens` 仅在 API 调用后更新，首次调用前显示 0% -> 可接受，首次调用很快发生
- **[多轮工具调用]** 工具调用中间轮次的 `prompt_tokens` 会反映累积上下文 -> 符合预期，这是实际上下文大小
- **[窄终端]** 进度条占用额外宽度 -> 宽度 <40 时自动隐藏
- **[配置缺失]** 用户未配置 `context_window` 时使用默认 200000 -> 合理默认值

## Open Questions

无（需求已全部澄清）。
```

## openspec/changes/tui-context-usage-indicator/tasks.md

- Source: openspec/changes/tui-context-usage-indicator/tasks.md
- Lines: 1-33
- SHA256: de475355cdf6ebf35ecad85204a6d8b951926d5204f691b0d0818c21d3872ce7

```md
## 1. TokenCounter 扩展

- [ ] 1.1 为 `TokenCounter` 新增 `last_prompt_tokens: Arc<AtomicUsize>` 字段
- [ ] 1.2 实现 `set_prompt_tokens(tokens: usize)` 方法，更新 `last_prompt_tokens`
- [ ] 1.3 实现 `last_prompt_tokens(&self) -> usize` 读取方法

## 2. API 用量记录

- [ ] 2.1 在 `AgentLoop::run_agent_loop` 的 token accounting 处，将 `usage.prompt_tokens` 存入 `token_counter.set_prompt_tokens()`

## 3. 上下文窗口配置

- [ ] 3.1 在 `Settings`/`ModelConfig` 中新增 `context_window: usize` 字段，默认 200000
- [ ] 3.2 确保 `settings.json` 序列化/反序列化兼容（可选字段，缺失时用默认值）

## 4. 进度条组件

- [ ] 4.1 新建 `src/tui/components/context_bar.rs`，实现 `render(f, area, used, max)` 函数
- [ ] 4.2 渲染 Unicode 进度条（8 格 `▓`/`░`）+ 百分比文字
- [ ] 4.3 实现颜色阈值逻辑：绿 <50%，黄 50-80%，红 >80%
- [ ] 4.4 在 `src/tui/components/mod.rs` 注册 `context_bar` 模块

## 5. 模式标签栏集成

- [ ] 5.1 修改 `render_mode_label`，在模式标签右侧渲染上下文进度条
- [ ] 5.2 从 `App` 传入 `token_counter.last_prompt_tokens()` 和 `settings.models.context_window`
- [ ] 5.3 窄终端（宽度 <40）自动隐藏进度条

## 6. 测试

- [ ] 6.1 `TokenCounter` 的 `set_prompt_tokens`/`last_prompt_tokens` 单元测试
- [ ] 6.2 进度条颜色阈值边界测试（49%/50%/80%/81%）
- [ ] 6.3 `context_window` 配置默认值与自定义值测试
```

