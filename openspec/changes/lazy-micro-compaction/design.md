## Context

Agent loop 每轮构建 API 请求视图（`loop_.rs:227-241`）：`system_messages` + 可选摘要 + `raw[boundary..]` 经 `micro_compact_messages` 压实。压实规则：最近 3 条工具结果逐字，其余替换为 `[Previous: used X]`（`file_read`/`read_file` 豁免）。该函数每轮无条件执行，仅作用于请求视图（history 不回写；413 应急 `fallback_micro_compact` 是唯一回写路径）。

Anthropic 路径有两个缓存断点：system prompt（`conversions.rs:103`）+ 倒数第二消息（`apply_conversation_cache_breakpoint`，`conversions.rs:153`），意图是缓存整个稳定前缀。但每轮无条件压实使「最近 3 条」窗口滑动，翻转点前的 token 每轮重新计价。

工具错误内容格式（已核实）：
- executor 成功：`{"success":true,"output_type":…,"content":…,"metadata":…}`（`executor.rs:161`）
- executor 失败：`{"success":false,"error":{"message":…,"code":…}}`（`executor.rs:168,262`）
- 参数解析失败：`{"success":false,"error":"task tool call arguments are invalid JSON…"}`（error 为字符串，`loop_.rs:551`）
- hook 拦截：纯文本 `Tool 'X' blocked by hook: …`（`executor.rs:228`）

## Goals / Non-Goals

**Goals:**

- 请求视图默认逐字；前缀在轮间稳定（缓存全命中）。
- 逼近上下文窗口时一次性压实（amortized 翻转成本，替代每轮翻转）。
- 失败工具结果压实后保留错误摘要，支持模型自纠。
- `file_read` 豁免、全量压缩行为、413 应急路径、会话持久化全部零回归。

**Non-Goals:**

- 不改 Anthropic 断点放置策略（system + 倒数第二消息已足够，前缀稳定后自然生效）。
- 不改 `micro_compact_messages` 本体（413 应急与摘要器输入仍用它）。
- 不做 OpenAI 显式缓存参数（隐式缓存随前缀稳定自动受益）。
- 不持久化 frontier 到会话文件。

## Decisions

### D1：verbatim 默认 + 水位触发

低于水位线时请求视图 = `system_messages ++ raw[boundary..]`（逐字，不压实）。水位线：`(context_window * 3 / 5).saturating_sub(max_tokens)`，即全量压缩阈值（4/5 − max_tokens，`needs_compaction` `compaction.rs:157`）的 3/4——保证廉价压实先于昂贵 LLM 摘要触发，且留足余量。估算复用 `estimate_prompt_tokens_calibrated` + `fixed_overhead_chars`（含工具定义开销），与 `needs_compaction` 同源。

### D2：sticky frontier，单调推进

`LoopTurnState` 新增 `micro_compact_frontier: usize`（raw history 绝对索引，默认 0）。请求视图：

```
view_tail = micro_compact_messages(raw[boundary..frontier]) ++ raw[frontier..]
```

触发条件（每轮构建前评估）：`estimate(当前视图) > 水位线` → 推进 `frontier` 到「覆盖最新 K=3 条工具结果」的最小索引（即第 len−K 新的 tool 消息位置）。前沿之前的消息从此永久 marker 化（file_read 除外，见 D4）。

**防振荡**：前沿只前进不后退。触发后视图立即降到水位线下；后续轮次在压实前缀上 append-only 追加，直到再次超水位再推进一次。这消除「压实/逐字来回切换导致前缀反复分歧」的病态模式——这是 naive「每轮独立判断是否压实」的致命缺陷，必须避免。

**与 boundary 的交互**：全量压缩推进 `compaction_boundary` 后，clamp `frontier = max(frontier, boundary)`（避免重复压实已摘要段，语义上无害但保持整洁）。

### D3：错误摘要 marker

压实时嗅探错误载荷（按序尝试）：

1. 内容可解析为 JSON 且 `success == false` → 取 `error.message`（对象形态）或 `error`（字符串形态）。
2. 内容以 `Tool '` 开头且含 ` blocked by hook: ` → 保留首行。
3. 其余 → 成功结果，裸 marker。

marker 格式：`[Previous: used <name>; error: <摘要>]`，摘要截断至 200 chars（char 边界安全）。成功结果保持 `[Previous: used <name>]` 不变（向后兼容）。

### D4：file_read 豁免保持

`micro_compact_messages` 的 `file_read`/`read_file` 豁免在压实段内继续生效（`compaction.rs:199`）——前沿内的读取结果永久逐字。这本身缓存友好（内容永不翻转）。

### D5：全量压缩评估不变

`needs_compaction` 在实际发送视图（前沿后）上评估——与今天「在压实视图上评估」等价，全量摘要触发时点、`split_for_compaction` / boundary 推进逻辑零变化，只是因 lazy 压实使视图更小而**更少触发**。

### D6：frontier 运行时态

不写入会话文件。恢复会话后 frontier=0（全逐字），首次超水位时重新推进——正确性无损（视图是历史函数，非状态函数），只付一次缓存重建成本。

## Risks / Trade-offs

- **[Risk] 水位线估算偏差**（校准未就绪的早期轮次用 chars/4 低估 CJK）→ Mitigation：估算器与 `needs_compaction` 同源同校准；低估只会延迟触发，最终仍由全量压缩兜底；水位 60% 留有 20% 缓冲。
- **[Risk] 压实段内错误嗅探每轮重复解析 JSON** → Mitigation：仅前沿推进时对新压实消息嗅探一次并在 marker 中固化——但视图每轮从 raw 重建，故实现在 `micro_compact_messages` 的 frontier 感知变体中顺带解析；工具结果 JSON 通常 <10KB，解析成本远低于其省下的 token 费用。可选优化：缓存嗅探结果（YAGNI，先不做）。
- **[Trade-off] 峰值请求变大**：低于水位时视图含全部历史 → 这正是缓存友好的形态；上限由水位线控制（60%），不会超过今天的全量压缩应对范围。
- **[Trade-off] frontier 不持久化**：恢复会话后可能再次压实相同消息 → 一次性成本，换零 schema 变更。

## Migration Plan

纯运行时行为变更，无 schema/存储迁移。新会话与恢复会话均安全（frontier 默认 0）。

## Open Questions

- 水位线 3/5 是否最优？→ 上线后观察全量压缩触发频率与 cache hit 计量（`usage` 里的 cached_tokens 字段已存在），必要时调 D1 常数。
- 错误嗅探是否需要覆盖更多纯文本错误形态（如 daemon 转发路径）？→ 以 executor 三种已核实格式起步，遇到再扩。
