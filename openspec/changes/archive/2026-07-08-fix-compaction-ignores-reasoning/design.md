## Context

`AgentLoop::needs_compaction`（`src/tui/agent/compaction.rs:148`）以 `total_chars / 4 > MAX_ESTIMATED_TOKENS` 判定是否触发压缩，其中 `total_chars` 仅累加每条 message 的 `content` 字段长度。`MAX_ESTIMATED_TOKENS = 800_000`（`src/tui/agent/mod.rs:23`）。

日志实测（`~/.wgenty-code/logs/wgenty-code.log`，request id `0217834853226618de218602406b4cab85da53782ff803ce2af6d`）首次失败请求的字段拆解：

| 字段 | 字符数 | 占比 |
|---|---|---|
| `content` | 148,580 | 32% |
| `reasoning_content` | 201,063 | 44% |
| `tool_calls.arguments` | 79,865 | 18% |
| `tools` 定义 | 27,843 | 6% |

`needs_compaction` 只看 `content`（32%），完全忽略 thinking 模型的 `reasoning_content`（最大头）。三次失败请求体 563K / 587K / 610K 字符递增，均在 `content ≈ 148K`（全量 message 字段 ~429K 字符）时即超 ≈128K 真实 tokens 窗口被拒。`remove-token-budget-limits` 移除预算硬截断后，压缩是唯一防线，但其估算与阈值双重失效。

## Goals / Non-Goals

**Goals:**

- 让 `needs_compaction` 反映真实请求大小（计入 `reasoning_content` + `tool_calls.arguments`）。
- 把阈值降到真实窗口附近，使压缩在溢出前触发。
- 最小改动，不引入新接口/依赖。

**Non-Goals:**

- 不改用真实 `prompt_tokens` 驱动压缩（方案 C，需线程 `last_usage` + 配置窗口，改动更大，留待后续）。
- 不计入工具定义大小（~28K，占比小且每次请求固定；message 字段已覆盖 94% 的变动成本）。
- 不改 `chars / 4` 估算系数（保持最小改动；阈值已留余量吸收误差）。
- 不改 compaction 的摘要逻辑、tail 保留逻辑、micro_compact。

## Decisions

### D1: 计入 content + reasoning_content + tool_calls.arguments

**Choice:** `needs_compaction` 的 `total_chars` 累加三部分：`content`、`reasoning_content`、每个 `tool_calls` 的 `function.arguments`。均为 `ChatMessage` 已有字段，无需改函数签名或传 tools。

**Rationale:** 这三部分覆盖请求中 message 侧 ~94% 的变动字符（剩 ~6% 是固定 tools 定义）。`reasoning_content` 是 thinking 模型最大头（44%），不计入则压缩对思考模型形同虚设。

**Alternatives considered:**

- 方案 A（仅降阈值不改统计）：需降到 ~28K 才能赶在 148K content 失败点前触发，且对 `reasoning` 占比变化脆弱。
- 方案 C（用真实 `prompt_tokens`）：最准最稳健，但需把 `last_usage` 线程进 `AgentLoop` + 新增窗口配置项，改动超出 hotfix 范围。

### D2: MAX_ESTIMATED_TOKENS 800_000 -> 80_000

**Choice:** 阈值降到 80K。按 `/4` 估算在 ~320K 全量字符触发；按实测首次失败点（content+reasoning+tc = 429K 字符 ≈128K 真实 tokens，即 ~3.35 字符/token），80K 阈值触发于 ~320K 字符 ≈95K 真实 tokens，留 ~33K 余量。

**Rationale:** 800K 远超任何模型窗口，等同关闭压缩。80K 在 128K 窗口下留足余量，且 `chars/4` 对混合内容略有低估（实测低估 ~1.19x），余量吸收误差。

**Alternatives considered:**

- 50K：更保守，压缩更频繁，余量过大。
- 不改阈值只改统计：800K 仍永不触发，无效。

## Risks

- 80K 阈值偏保守 -> 长会话压缩更频繁。可接受：优于溢出崩溃。
- `chars/4` 对纯中文略低估真实 tokens -> 已通过 ~33K 余量吸收；若后续切更大窗口模型可上调。
- 阈值仍硬编码 -> 未来可改为 settings 可配；本次不做（保持 hotfix 范围）。
