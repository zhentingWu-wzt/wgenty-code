# Design: prompt-cache-economy

## Context

z.ai GLM 走隐式自动前缀缓存：命中部分折扣计费，命中数在 `usage.prompt_tokens_details.cached_tokens`。请求结构为 `[system × N][历史 append-only][tools][最新 user]`——前缀稳定性决定跨请求命中。当前 daemon 每轮 `run_session_turn`（src/daemon/run_loop.rs:2011）召回记忆写入 `PromptContext.memories`，`assemble_instructions`（src/prompts/mod.rs:522）据此生成 system 级 Layer 5b；TUI 侧存在同一模式（run_loop 注释标注 "mirrors TUI turn.rs:229"）。user-message reminder 通道（`build_user_turn_reminder`，目前 hook-only）已存在，产物 `ReminderOutput{to_model, to_transcript}` 前置到最后一条 user 消息。API 边界已有消息清洗先例 `sanitize_tool_call_args_for_replay`（src/api/mod.rs:52）。Anthropic 原生路径已在 system 与末位 tool 设 `cache_control` 断点，不受本变更影响。

## Goals / Non-Goals

**Goals:**
- 跨 turn 前缀稳定：system 级联不再含逐轮变化的记忆内容
- 缓存命中可观测：cached_tokens 贯通到 web Inspector
- OpenAI 兼容请求不再回传历史 `reasoning_content`（DeepSeek 除外）

**Non-Goals:**
- 不改 Anthropic 路径的 cache_control 断点策略
- 不实现 z.ai 显式缓存 API / 缓存预热
- 不改变磁盘会话文件格式（reasoning 仍持久化供回放；compactor 对 reasoning 的使用不变）
- 不调整 TF-IDF 召回算法本身

## Decisions

- **D1 记忆注入位置**：从 system Layer 5b 改为 `build_user_turn_reminder` 内新增 memories section（`<relevant_memories>` 块原样迁移），随 hook reminder 前置到最后一条 user 消息。理由：user 消息位于请求尾部，逐轮变化的内容放在尾部不破坏前缀；该通道已具备 to_model/to_transcript 双通道与调试 dump 设施。备选「保留 system 但固定内容」被否——记忆召回天然逐轮变化。Layer 5b/5c 处理：5b（project 召回）整体移除；5c（global 固定集合，软上限 50）内容跨 turn 稳定，保留在 system 不动。
- **D2 daemon/TUI 同步**：TUI turn 注入点与 daemon 同步迁移，防止两端 prompt 结构漂移（现有测试断言两端口径一致的先例沿用）。
- **D3 cached_tokens 贯通**：OpenAI usage 结构体新增 `prompt_tokens_details.cached_tokens`（serde default 缺省）；Anthropic 转换层把 `cache_read_input_tokens` 映射到同一字段；token_counter 增加 `last_cached_tokens`；TurnContext usage JSON 增加 `cached_tokens`；web `TurnContextUsage` 类型同步，Inspector TokensTab 显示 `cached / prompt (xx%)`。字段缺失（旧网关）时显示为 null，不做猜测。
- **D4 reasoning 剥离挂点**：`src/api/mod.rs` 新增 `strip_reasoning_content_for_replay(messages, provider)`，在 OpenAI 兼容请求组装处调用；provider 判定复用 `resolve_provider` 结果——DeepSeek 保留，其余（含 GLM/OpenAI/自建网关）剥离。备选「入库时剥离」被否——会破坏本地回放与 compactor 上下文；备选「ChatMessage 序列化层全局剥」被否——DeepSeek 合法需要。

## Risks / Trade-offs

- 记忆从请求头部移到尾部，模型对记忆的注意力位置变化——业界实践（尾部 reminder）已是主流，风险低；通过现有 TUI/web 双端行为一致性验证。
- reminder 变大（记忆多时）使每轮 user 消息膨胀——既有 `to_model` 通道本就承载 hook 注入，且 TF-IDF 召回有 top_n 上限，可接受。
- cached_tokens 字段各家网关命名不一——以 OpenAI `prompt_tokens_details.cached_tokens` 与 Anthropic `cache_read_input_tokens` 两个已知形态为准，其余静默为 null。
- 剥离 reasoning 后若某网关实际要求 echo-back（未知网关），表现为模型行为轻微退化而非报错——通过 provider 白名单（默认剥、DeepSeek 留）+ 可配置开关兜底（settings 加 `strip_reasoning_replay`，默认 auto）。
