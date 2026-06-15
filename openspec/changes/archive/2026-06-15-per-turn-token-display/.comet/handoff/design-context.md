# Comet Design Handoff

- Change: per-turn-token-display
- Phase: design
- Mode: compact
- Context hash: 405ba064008ba60d0426ef3aa9e97dd4ae6c8e3f5570b857297ce489f3ec34bb

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/per-turn-token-display/proposal.md

- Source: openspec/changes/per-turn-token-display/proposal.md
- Lines: 1-32
- SHA256: 28d86a46e5d939b79d4061c7986bf0e84263521d18ebfc2189a451c872e0a32a

```md
## Why

当前状态栏的 token 展示统计的是 API 返回的 `total_tokens` 全程累积值，混入了系统提示词、工具定义、工具结果、对话历史等并非用户直接产生的 token 消耗。用户无法直观感知自己每次输入和模型每次输出的实际 token 规模。改为按 turn 分离展示用户输入 token 和模型输出 token，剔除系统/工具噪音，让用户看到真正属于自己的消耗。

## What Changes

- **TokenCounter 扩展**：新增 `turn_input` 和 `turn_output` 两个原子计数器，分别追踪当前 turn 的用户输入 token 估算值和模型输出 token（`completion_tokens`），保留原有 `used` 计数器用于预算控制不变
- **用户输入拦截**：在 `AgentLoop::process_input_inner` 中，用户消息推入对话历史前，用 `chars/4` 估算 token 数并累加到 `turn_input`
- **输出 token 累加**：在 `AgentLoop::run_agent_loop` 中，每轮 LLM 调用后将 `usage.completion_tokens` 累加到 `turn_output`（替代当前的 `usage.total_tokens`）
- **Turn 重置**：每个新 turn 开始时（`process_input` 入口），将 `turn_input` 和 `turn_output` 归零
- **状态栏显示变更**：从 `↓ Nk tokens`（单值）改为 `↑ N · ↓ Mk`（分别展示输入/输出），闲置时保留上 turn 值；token=0 时不展示对应部分
- **TypeScript Ink 前端**：暂不修改

## Capabilities

### New Capabilities

- `per-turn-token-display`: 按 turn 分离展示用户输入 token 和模型输出 token，剔除系统提示词、工具定义、工具结果、对话历史等非用户直接产生的 token

### Modified Capabilities

<!-- 无现有 spec 受影响 -->

## Impact

| 文件 | 变更 |
|------|------|
| `src/api/token_counter.rs` | 新增 `turn_input`/`turn_output` 字段及 `add_input`/`add_output`/`reset_turn` 方法 |
| `src/tui/agent/mod.rs` | `process_input` 入口重置 turn 计数器；`process_input_inner` 中估算用户输入 token |
| `src/tui/agent/core.rs` | `run_agent_loop` 中将 `usage.completion_tokens` 替换 `usage.total_tokens` 用于显示 |
| `src/tui/components/status.rs` | `render` 签名扩展为接收 `(input_tokens, output_tokens)`，`format_tokens` 改为 `↑ N · ↓ Mk` |
| `src/tui/app/render.rs` | 读取 `turn_input`/`turn_output` 传入 status render |
```

## openspec/changes/per-turn-token-display/design.md

- Source: openspec/changes/per-turn-token-display/design.md
- Lines: 1-59
- SHA256: 47545a91ef4cec553a08d62faa7603d194e88ff6d5792d4d127d45875ffcc17a

```md
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
```

## openspec/changes/per-turn-token-display/tasks.md

- Source: openspec/changes/per-turn-token-display/tasks.md
- Lines: 1-22
- SHA256: d2e0b1feb2e3168c7986eba38f31ceefc2fe7e74c334aed8ecf92125887794c6

```md
## 1. TokenCounter 扩展

- [ ] 1.1 为 `TokenCounter` 新增 `turn_input: Arc<AtomicUsize>` 和 `turn_output: Arc<AtomicUsize>` 字段
- [ ] 1.2 实现 `add_input(tokens: usize)` 方法，原子累加 turn_input
- [ ] 1.3 实现 `add_output(tokens: usize)` 方法，原子累加 turn_output
- [ ] 1.4 实现 `reset_turn()` 方法，将 turn_input 和 turn_output 归零
- [ ] 1.5 实现 `turn_input_tokens(&self) -> usize` 和 `turn_output_tokens(&self) -> usize` 读取方法

## 2. AgentLoop 集成

- [ ] 2.1 在 `AgentLoop::process_input` 入口调用 `token_counter.reset_turn()` 重置当前 turn 计数
- [ ] 2.2 在 `process_input_inner` 中，用户消息推入历史前，估算 `input.len() / 4` 并调用 `token_counter.add_input()`
- [ ] 2.3 在 `run_agent_loop` 中，将 `token_counter.add(usage.total_tokens)` 改为 `token_counter.add_output(usage.completion_tokens)`（预算控制仍用 `used` 字段，单独调用 `token_counter.add(usage.total_tokens)` 或用 `completion_tokens` 近似）

## 3. 状态栏渲染更新

- [ ] 3.1 修改 `components::status::render` 签名，接收 `(input_tokens: usize, output_tokens: usize)` 替代原有的 `tokens_used: usize`
- [ ] 3.2 修改 `format_tokens` 显示逻辑：`↑ N · ↓ Mk` 格式，token=0 时隐藏对应部分；k 单位阈值 1000

## 4. App 渲染适配

- [ ] 4.1 在 `App::render_status` 中读取 `token_counter.turn_input_tokens()` 和 `token_counter.turn_output_tokens()` 传入 status render
```

## openspec/changes/per-turn-token-display/specs/per-turn-token-display/spec.md

- Source: openspec/changes/per-turn-token-display/specs/per-turn-token-display/spec.md
- Lines: 1-52
- SHA256: 295ecc8b3b11dcaa646e34bef02b092e6211abba921f9deaa40416313c0db01b

```md
## ADDED Requirements

### Requirement: Per-turn input token tracking
The system SHALL estimate and accumulate user input tokens for each turn using `chars/4` formula, applied to the user's message before it is appended to conversation history.

#### Scenario: Single user input
- **WHEN** user submits a message of 100 characters
- **THEN** the turn's input token counter SHALL display `↑ 25` (100/4)

#### Scenario: Multi-byte characters
- **WHEN** user submits a message containing Chinese/UTF-8 characters of 200 bytes
- **THEN** the turn's input token counter SHALL divide by character count (`.len()`), not byte length

### Requirement: Per-turn output token tracking
The system SHALL accumulate model output tokens for each turn by summing `completion_tokens` from each LLM round's `Usage` within the turn.

#### Scenario: Single LLM round
- **WHEN** an LLM round returns `usage.completion_tokens = 800`
- **THEN** the turn's output token counter SHALL be incremented by 800

#### Scenario: Multiple LLM rounds with tool calls
- **WHEN** a turn involves 3 LLM rounds with completion_tokens [800, 500, 300]
- **THEN** the turn's output token counter SHALL accumulate to 1600

### Requirement: Turn reset on new input
The system SHALL reset both input and output token counters to zero at the beginning of each new user turn.

#### Scenario: Second turn starts
- **WHEN** a new user turn begins after a completed turn showing `↑ 25 · ↓ 1.6k`
- **THEN** both input and output counters SHALL reset to 0 before processing the new input

### Requirement: Status bar display format
The status bar SHALL display per-turn token counts in `↑ N · ↓ Mk` format. When a counter is 0, its section SHALL be omitted.

#### Scenario: Both input and output available
- **WHEN** turn has input=25 tokens and output=1600 tokens
- **THEN** status bar SHALL display `↑ 25 · ↓ 1.6k` in the meta section

#### Scenario: Only output available (no input estimated)
- **WHEN** turn has input=0 and output=800 tokens
- **THEN** status bar SHALL display `↓ 800 tokens` without the input section

#### Scenario: Idle state preserves last turn value
- **WHEN** a turn completes and agent enters idle state
- **THEN** the status bar SHALL continue displaying the last turn's token values

### Requirement: Budget counter isolation
The system SHALL maintain the existing `used`/`budget` fields in `TokenCounter` unchanged, independent of the new `turn_input`/`turn_output` counters.

#### Scenario: Budget enforcement unaffected
- **WHEN** turn_input + turn_output reach a different value than the budget counter's `used` field
- **THEN** budget enforcement SHALL be based on the `used` field only, not on turn counters
```

