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
