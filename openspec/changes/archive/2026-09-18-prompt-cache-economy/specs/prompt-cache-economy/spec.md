# prompt-cache-economy delta

## Purpose

让 z.ai GLM 等隐式前缀缓存网关获得跨 turn 缓存命中，并使缓存命中与跨请求 reasoning 载荷可观测、可控制。覆盖：记忆注入的缓存友好位置、cached_tokens 贯通展示、API 边界 reasoning 剥离。

## ADDED Requirements

### Requirement: 记忆召回注入于用户消息尾部
daemon 与 TUI 每轮的跨会话记忆召回结果（TF-IDF top-N）MUST 通过 user-message `<system-reminder>` 通道注入到最后一条用户消息，MUST NOT 写入 system 级消息。system 级联（含固定集合的 global memories Layer 5c）在会话配置不变的前提下跨 turn 逐字节稳定。

#### Scenario: 连续两轮 system 前缀稳定
- **WHEN** 同一会话连续完成两个 turn，且两个 turn 的记忆召回结果不同
- **THEN** 两次请求的 system 消息序列逐字节一致，记忆内容仅出现在最后一条 user 消息的 `<system-reminder>` 内

#### Scenario: 无召回时 reminder 退化
- **WHEN** 某 turn 记忆召回为空且无 hook 注入
- **THEN** 不构造 `<system-reminder>`，用户消息保持原文

### Requirement: 缓存命中 token 可观测
OpenAI 兼容响应的 `usage.prompt_tokens_details.cached_tokens`（及 Anthropic 的 `cache_read_input_tokens`）MUST 解析并贯通到 TurnContext 的 usage 数据；web Inspector Tokens 面板 MUST 展示 cached 与 prompt 的比值。字段缺失时展示为 null，不得猜测。

#### Scenario: Inspector 显示缓存占比
- **WHEN** 网关返回 `prompt_tokens_details.cached_tokens = 12000` 且 `prompt_tokens = 15000`
- **THEN** Inspector Tokens 面板显示 `12,000 / 15,000 (80%)` 一类的 cached 占比

#### Scenario: 聊天 StatusBar 显示缓存命中徽标
- **WHEN** 一个 turn 完成，TurnContext 携带 `cached_tokens` 与 `context_tokens`
- **THEN** 聊天区 StatusBar 在 context 条旁显示 cache 命中百分比（如 `cache 80%`）；字段缺失时不显示该徽标且不报错

#### Scenario: 网关不返回缓存字段
- **WHEN** 响应 usage 中不含 `prompt_tokens_details`
- **THEN** Inspector 显示 cached 为 null/“—”，且不产生错误

### Requirement: OpenAI 兼容路径剥离历史 reasoning_content
发往 OpenAI 兼容端点的请求中，历史 assistant 消息 MUST NOT 携带 `reasoning_content`，DeepSeek provider 除外（保留 echo-back）。磁盘会话持久化与本地回放不受影响。

#### Scenario: GLM 请求不含 reasoning 字段
- **WHEN** 会话历史含带 `reasoning_content` 的 assistant 消息，provider 为 z.ai GLM（openai 兼容）
- **THEN** 发出的请求 JSON 序列化后任何消息均不含 `reasoning_content` 字段

#### Scenario: DeepSeek 保留 echo-back
- **WHEN** provider 为 DeepSeek 且历史 assistant 消息含 `reasoning_content`
- **THEN** 发出的请求保留该字段
