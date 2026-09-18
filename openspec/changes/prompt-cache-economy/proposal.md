# Proposal: prompt-cache-economy

## Why

主模型走 z.ai GLM-5.3（OpenAI 兼容端点，隐式自动前缀缓存）。当前实现存在三个成本/可观测性问题：①daemon 每轮把 TF-IDF 记忆召回结果注入 system 级 Layer 5b `<relevant_memories>`（`run_session_turn` → `assemble_instructions`），它位于请求最前缀且逐轮变化，导致跨 turn 前缀缓存基本失效；②z.ai 返回的 `usage.prompt_tokens_details.cached_tokens` 未被解析，命中率不可观测；③历史 assistant 消息的 `reasoning_content` 随 `ChatMessage` serde 原样回传 API（thinking 模型每轮数千 token 被重复计费），而 z.ai 并不需要 echo-back（仅 DeepSeek 需要）。

## What Changes

- 记忆召回改走 user-message `<system-reminder>` 尾部通道（`build_user_turn_reminder` 扩展 memories section），不再写 system Layer 5b；system 级联跨 turn 逐字节稳定，解锁隐式前缀缓存。daemon 与 TUI 同步修改（两者共享同一条注入路径的模式）。
- 解析 OpenAI 兼容响应 usage 中的 `prompt_tokens_details.cached_tokens`（Anthropic 路径映射 `cache_read_input_tokens`），贯通 token_counter → TurnContext usage JSON → web `TurnContextUsage` 类型 → Inspector Tokens 面板展示 cached/prompt 占比。
- OpenAI 兼容路径在 API 请求边界剥离历史消息中的 `reasoning_content`（挂点仿照现有 `sanitize_tool_call_args_for_replay`）；DeepSeek provider 保留 echo-back 行为；磁盘会话持久化不受影响（仍保存 reasoning 供回放）。

## Capabilities

### New Capabilities
- `prompt-cache-economy`: 提示词缓存经济性——记忆注入的缓存友好位置、缓存命中可观测、跨请求 reasoning 载荷剥离

### Modified Capabilities

<!-- 无：不改变既有 spec 级行为；Layer 5b 移除属于新 capability 的需求范畴 -->

## Impact

- 代码：`src/prompts/mod.rs`、`src/daemon/run_loop.rs`、TUI turn 注入点、`src/api/types.rs`（usage 解析）、token_counter、`src/api/mod.rs`（边界剥离）、`web/src/state/sessionStore.ts`、`web/src/features/panels/InspectorPanel.tsx`
- 不影响：Anthropic 原生路径行为、磁盘会话格式、权限/沙箱
- 风险：记忆从 system 移到 user reminder 后模型可见性位置变化（尾部 vs 头部）——通过 TUI/web 双端一致的 reminder 通道与现有 hook 注入同模式，风险低
