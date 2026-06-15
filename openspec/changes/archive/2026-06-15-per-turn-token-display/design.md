## Context

当前状态栏 token 展示使用 `TokenCounter.used_tokens()` 获取全程累积的 `total_tokens` 值，该值包含了系统提示词、工具定义、工具结果、对话历史等非用户直接产生的 token。`Usage` 结构体已包含 `prompt_tokens` 和 `completion_tokens` 两个独立字段，但状态栏仅使用合并后的 `total_tokens`。

用户希望在状态栏中看到每个 turn 的"用户输入 token"和"模型输出 token"，剥离系统/工具/历史的噪音。

## Goals / Non-Goals

**Goals:**
- 状态栏展示当前 turn 的用户输入 token 估算值和模型输出 token（`completion_tokens`），格式 `↑ N · ↓ Mk`
- 每个新 turn 重置计数
- 闲置时保留上一个 turn 的值
- TokenCounter 的预算控制逻辑（`used`/`budget`）不受影响
- 子代理 token 不影响主状态栏显示

**Non-Goals:**
- 不使用外部 tokenizer（tiktoken 等），用 `chars/4` 估算用户输入
- 不修改 TypeScript Ink 前端
- 不修改 API/budget 逻辑

## Decisions

### D1: TokenCounter 扩展而非新建结构

**选择**：在现有 `TokenCounter` 上新增 `turn_input: Arc<AtomicUsize>` 和 `turn_output: Arc<AtomicUsize>` 字段。

**理由**：`TokenCounter` 已在 `App` 和 `AgentLoop` 之间通过 `Arc` 共享，扩展它避免引入新的共享通道。原有 `used`/`budget` 字段和行为保持不变。

**备选**：新建独立的 `TurnTokenTracker` 结构。增加额外的 `Arc` 传递和渲染参数，无实质收益。

### D2: 用户输入 token 使用 chars/4 估算

**选择**：在 `process_input_inner` 中，用户消息推入历史前，计算 `input.len() / 4` 作为输入 token 估算值。

**理由**：零依赖，性能无影响。对英文场景精度可接受（~25% 误差），中文场景会偏高约 2x。用户确认接受此精度。

**备选**：集成 `tiktoken-rs` 或 `tokenizers` crate。精度高但增加编译时间和依赖体积。当前不需要。

### D3: AgentLoop 直接操作 turn 计数器

**选择**：在 `AgentLoop::process_input` 入口调用 `token_counter.reset_turn()`，在 `process_input_inner` 中调用 `token_counter.add_input(input)`, 在 `run_agent_loop` 中调用 `token_counter.add_output(usage.completion_tokens)`。

**理由**：`AgentLoop` 已经是 token 累加的唯一入口点，在此处操作 turn 计数器是最小侵入的改动。

### D4: 状态栏 display 格式

**选择**：`↑ N · ↓ Mk` 格式。无 token 时不显示对应部分（如无输入时只显示 `↓ Mk`）。

**理由**：简洁、直观。`↑` 表示输入（用户→模型），`↓` 表示输出（模型→用户），符合直觉。

## Risks / Trade-offs

- **[精度偏差]** 中文输入场景 chars/4 估算偏高 ~2x → 用户已知悉，可接受；未来可替换为 tokenizer
- **[状态栏宽度]** 新增字段增加状态栏宽度 → 当前状态栏有充裕空间，且 token=0 时隐藏对应部分
- **[子代理输出]** 子代理 completion_tokens 不被主 AgentLoop 累加 → 符合"只看主对话"需求

## Open Questions

无。
