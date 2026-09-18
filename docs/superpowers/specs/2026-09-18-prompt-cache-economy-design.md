---
comet_change: prompt-cache-economy
role: technical-design
canonical_spec: openspec
---

# Technical Design: prompt-cache-economy

## Context

主模型走 z.ai GLM-5.3（OpenAI 兼容端点，隐式自动前缀缓存，命中数在 `usage.prompt_tokens_details.cached_tokens`）。请求结构 `[system × N][历史 append-only][tools][最新 user]` 的前缀稳定性决定跨请求命中。三处与缓存经济性冲突的现状：

1. daemon（`run_session_turn`）与 TUI（`turn.rs`）每轮把 TF-IDF 记忆召回写入 `PromptContext.memories`，`assemble_instructions`（`src/prompts/mod.rs:522`）据此生成 system 级 Layer 5b `<relevant_memories>`——位于请求最前缀且逐轮变化，跨 turn 缓存失效。
2. `cached_tokens` 未被解析（全库无该字段），命中率不可观测。
3. 历史 assistant 消息的 `reasoning_content` 随 `ChatMessage` serde 回传 API（`types.rs:63`），z.ai 无需 echo-back，重复计费。

关键结构事实（探索确认）：TUI（`turn.rs:272-286`）与 daemon（`run_loop.rs:2019-2026`）都已把召回结果填入 `ctx.memories`，且都已把 `prompt_context` 传给 `build_user_turn_reminder`（`tui/agent/mod.rs:327`、`run_loop.rs:2066`）——注入位置改造可完全收敛在 `prompts/mod.rs`。API 边界已有消息清洗先例 `sanitize_tool_call_args_for_replay`（`api/mod.rs:52`）。Anthropic 路径已有 `cache_control` 断点，不受影响。

## Goals / Non-Goals

**Goals:**
- system 级联跨 turn 逐字节稳定（记忆移至 user 尾部 reminder），解锁隐式前缀缓存
- `cached_tokens` 从响应解析贯通到 web Inspector 展示
- OpenAI 兼容请求不再回传历史 `reasoning_content`（DeepSeek 除外，用户确认默认剥离）

**Non-Goals:**
- 不改 Anthropic 路径 cache_control 策略；不实现 z.ai 显式缓存 API
- 不改磁盘会话格式（reasoning 仍持久化，回放/compactor 行为不变）
- 不调整召回算法与 Layer 5c（global memories 固定集合，跨 turn 稳定，留在 system）

## Decisions

### D1 记忆注入位置：prompts/mod.rs 单点改造（方案 A）

- `assemble_instructions`：删除 Layer 5b 块；Layer 5c 保留。
- `build_user_turn_reminder`：新增 memories section，section 顺序 hooks → memories（hook 为用户显式配置，优先级高）；沿用 `ReminderOutput{to_model, to_transcript}` 双通道与 dump 设施；"hook-only" 文案更新。
- 调用点（daemon/TUI）零改动——双方已按正确顺序填充 `ctx.memories` 并调用 builder。

备选：B（改 builder 签名显式传 memories）需改两侧调用点，无收益；C（只改 daemon）造成双端口径漂移。均否。

### D2 cached_tokens 贯通

- OpenAI `Usage` 增加 `prompt_tokens_details.cached_tokens`（`#[serde(default)]`，缺省 None）。
- Anthropic 响应转换层将 `cache_read_input_tokens` 映射到统一 `Usage.cached_tokens`。
- `TokenCounter` 增加 `last_cached_tokens`；TurnContext usage JSON 增加 `cached_tokens`。
- web：`TurnContextUsage.cached_tokens?: number | null`；Inspector TokensTab 显示 `Cached 12,000 / 15,000 (80%)`，null 显示 "—"。

备选「仅 OpenAI」被否：Anthropic 映射是同一管道的顺带工作。

### D3 reasoning 剥离：API 边界 + provider 白名单（默认剥离，用户确认）

- `src/api/mod.rs` 新增 `strip_reasoning_content_for_replay(messages, provider)`，挂在 `sanitize_tool_call_args_for_replay` 同一调用序列（OpenAI 兼容请求组装处）。
- 规则：DeepSeek provider 保留（官方要求 echo-back）；其余 OpenAI 兼容（含 GLM/OpenAI/自建网关）剥离。
- 兜底：settings `models.transport.strip_reasoning_content`（`Option<bool>`，缺省 None = 按 provider 规则；Some(false) 强制保留）。
- 仅请求边界：磁盘会话、本地回放、compactor 对 reasoning 的读取均不变。

备选「入库时剥离」破坏回放与 compaction 上下文；「serde 全局剥」误伤 DeepSeek。均否。

## Data Flow

```
① recall → ctx.memories ──→ build_user_turn_reminder ──→ [system × N(稳定)][…history…][user + <system-reminder>hooks+memories]
        └──────────────╢ assemble_instructions 不再产出 Layer 5b（Layer 5c 留守）
② usage.cached_tokens ─→ TokenCounter.last_cached_tokens ─→ TurnContext JSON ─→ Inspector TokensTab
③ 出站请求 ─→ strip_reasoning_content_for_replay ─→ 序列化（无 reasoning_content，DeepSeek 除外）
```

## Risks / Trade-offs

- 记忆注意力位置从头部移到尾部：业界尾部 reminder 为主流实践，风险低；双端行为一致可验证。
- reminder 增大：召回有 top_n 上限，可接受。
- 未知网关要求 echo-back：行为退化（非报错）——settings 开关兜底，用户已确认默认剥离。
- cached_tokens 网关命名差异：仅认 OpenAI/Anthropic 已知形态，其余静默 null（展示 "—"）。

## Testing

- ① `prompts` 单测：连续两 turn 不同召回下 system 消息序列逐字节一致；无召回且无 hook 时 reminder 为 None；memories section 顺序在 hooks 之后；dump 文案。
- ② `api` usage 解析单测（OpenAI 有值/缺字段、Anthropic 映射）；web vitest：Inspector 有值/null 两态。
- ③ 序列化断言：GLM(OpenAI 兼容) 请求 JSON 无 `reasoning_content`；DeepSeek 保留；`strip_reasoning_content=false` 覆盖生效。
- 全量门：`cargo test --lib`、`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、web `typecheck` + `npm test`。
