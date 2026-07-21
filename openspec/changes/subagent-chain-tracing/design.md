---
comet_change: subagent-chain-tracing
role: technical-design
canonical_spec: openspec
---

## Context

Subagent 链路追踪的基础设施已存在但分散且有缺口：

- `SubagentProgress`（`src/agent/progress.rs`）含 `action_log`/`events`/`error_details: ErrorInfo`（仅 `error_type`/`message`/`last_tool`/`last_params`/`round`/`retryable: bool`）。
- `SubagentTranscriptStore`（`src/transcript/store.rs`）SQLite 持久化两张表：`subagent_transcripts`（header）+ `subagent_events`（每事件含 `tool_name`/`tool_params`/`elapsed_ms`/`round`/`token_count`）。事件级工具调用序列**已存在**，但失败时未沉淀为结构化诊断。
- `FailureMode::classify`（`src/teams/subagent_health.rs`）已做 post-hoc 字符串匹配分类（token/timeout/api/tool/cancel 等），但缺 guardian-reject/sandbox-fail/tool-panic，且结果不落库。
- `SubagentTraceTool`（`src/tools/meta/subagent_trace.rs`）渲染 call_tree/error_timeline/chrome_trace/html，需 `session_id`，仅会话内可用。
- daemon（`src/daemon/routes.rs`）axum + `require_auth`，`chat_stream` 已是流式端点；flat `/api/v1/subagent/progress` 已退役为 capability-scoped 视图。
- CLI（`src/cli/mod.rs`）`Commands` enum 无 `Subagent` 子命令。

约束：MSRV 1.75、CI 零 warning、跨平台、`daemon` feature-gated、transcript schema 须向后兼容。

## Goals / Non-Goals

**Goals:**
- 失败时沉淀结构化诊断：完整失败工具调用序列（含每步耗时）、根因分类、失败轮次完整上下文（截断）、重试历史。
- 实时流式 trace 输出到本地 JSONL 文件 + daemon SSE endpoint。
- 独立 CLI `wgenty-code subagent list|trace|health` 离线查询历史运行、单次 trace、成功率与失败模式。

**Non-Goals:**
- 不改变现有 subagent 调度/重试策略本身（仅记录重试历史，不调整重试逻辑）。
- 不新增对外部可观测性后端（OTel/Jaeger）的导出（未来可扩展）。
- 不做跨项目聚合统计（仅项目本地 transcript）。
- 不重构 capability-scoped agent 路由。

## Decisions

### D1: 失败诊断数据模型--扩展 `ErrorInfo` 而非新表

扩展 `ErrorInfo` 增加：`root_cause: FailureRootCause`（枚举：TokenBudgetExceeded/GuardianRejected{reason}/SandboxFailed/ApiError/ToolPanic/Timeout/UserCancelled/Unknown）、`failed_tool_sequence: Vec<ToolCallStep>`（tool_name + 关键参数摘要 + elapsed_ms，从 `action_log` 失败轮次切片）、`failed_round_context: Option<FailedRoundContext>`（assistant 文本 + 末尾工具原始输出，各截断至 N 字符）、`retry_history: Vec<RetryAttempt>`（每次 attempt 的 error/root_cause/strategy/outcome）。

**为何不新建诊断表**：诊断是 transcript 的派生视图，强生命周期绑定；存为 header 的 JSON 列（`failure_diagnostics TEXT` 存 serde_json）避免多表 join，读取与 header 同步。事件级序列已存在 `subagent_events`，无需重复存储。

**替代方案**：新建 `subagent_failure_diagnostics` 表--被否决，增加 join 复杂度且 1:1 绑定 header。

### D2: 根因分类在捕获时确定，扩展 `FailureMode`

捕获路径（`subagent_loop.rs`）在产生错误时调用扩展后的分类器，写入结构化 `root_cause`。新增类别 GuardianRejected/SandboxFailed/ToolPanic。保留 `FailureMode::classify` 字符串匹配作为旧记录降级路径（`unknown`）。

**替代方案**：完全沿用 post-hoc 字符串匹配--被否决，guardian/sandbox 拒绝信息在错误消息中不稳定，捕获时有结构化信号更可靠。

### D3: 流式 sink--双通道（项目本地 JSONL 文件 + 全局 daemon SSE），异步 buffered writer 驱动

新增 `TraceSink`（`src/teams/trace_sink.rs`）：内部持 mpsc channel，spawn 独立 writer task 批量 flush，**不阻塞** `ProgressCallback`/agent loop（Q3 决策）。复用现有 `ProgressCallback`（`Arc<dyn Fn(SubagentProgress)>`），progress 更新时序列化为 JSONL 事件投递到 channel；writer task append 到**项目本地** `<project>/.wgenty-code/traces/<session_id>.jsonl`（Q2 决策，由运行中 session 写入，0600/0700 权限），并广播到 daemon SSE 订阅者。channel 满时丢最旧事件（仅影响实时订阅，持久在 db 不受影响）。

daemon 端新增 `GET /api/v1/subagents/trace/stream?session_id=&since=`（SSE，复用 `require_auth`），**全局**订阅所有 session（Q5 决策），从内存 broadcast channel 推送；冷启动历史通过全局 `transcript_store` 回放。

**为何双通道**：JSONL 文件为持久离线分析（外部 tail/Perfetto 导入），SSE 为实时订阅。文件 sink 默认开（`subagent.trace.sink=file`），SSE 随 daemon 启用。

**为何异步 buffered**：同步 append 每事件写盘会阻塞 agent loop；mpsc + writer task 解耦，失败不影响主循环。

**替代方案**：仅 daemon SSE--被否决，无 daemon 时丢失持久 trace。仅文件--被否决，无实时订阅。同步 append--被否决，阻塞主循环。

### D4: CLI 子命令复用 transcript store，输出格式化

新增 `Commands::Subagent { action: SubagentCommands }`，`SubagentCommands::{List, Trace, Health}`。`List` 调 `list_by_session`/全量（按时间倒序，表格输出）；`Trace { id }` 调 `get_by_id` + 渲染（复用 `SubagentTraceTool` 渲染逻辑，默认 call_tree）；`Health { --period }` 调 `SubagentHealthAnalyzer::compute_from_headers`。全部只读，直接读 SQLite，无需启动 agent。

**替代方案**：复用现有 `Agent` 子命令--被否决，`Agent` 是"运行 agent"，语义不同。

### D5: schema 迁移--幂等 `ALTER TABLE ADD COLUMN`（4 列）

`run_migrations` 在现有 `execute_batch` 之外增加独立迁移步骤，为 `subagent_transcripts` 加 4 列：`failure_diagnostics TEXT`、`root_cause TEXT`、`retry_history TEXT`、`project_path TEXT`（Q1 决策，写入时记录 project root，CLI v1 暂不暴露项目过滤，为未来预留）。SQLite `ALTER TABLE ADD COLUMN` 不支持 `IF NOT EXISTS`，用 `PRAGMA table_info` 检查列存在性后再执行，保证幂等。旧记录新列 NULL，读取时 `root_cause` 降级为 `Unknown`、其余为空。

**db 位置**（Q1）：transcript db 默认全局 `~/.wgenty-code/subagent_transcripts.db`（`settings.storage.transcript.db_path` 可配）；CLI `list` 全局显示 + `--session`/`--status`/`--limit` 过滤，直接读 SQLite，不启动 agent。

### D6: 配置项

`subagent.trace.sink`（`file`|`daemon`|`both`|`off`，默认 `file`）、`subagent.trace.dir`（默认 `<project>/.wgenty-code/traces`）、`subagent.trace.context_char_limit`（默认 2000，失败轮次上下文截断阈值）。保留窗口复用 `storage.transcript.max_age_days`。

## Risks / Trade-offs

- **trace 文件含工具参数泄露敏感信息** -> 复用 guardian 脱敏策略，`tool_params` 序列化前过滤已知敏感键（api_key/token/secret）；文件权限 0600。
- **SSE 长连接在高并发子代理下内存增长** -> broadcast channel 设容量上限，满则丢弃最旧事件（仅实时订阅受影响，持久记录在文件不受影响）。
- **schema 迁移在旧库执行失败** -> 迁移包在事务内，失败回滚；`PRAGMA table_info` 检查保证幂等；新增单测覆盖空库与旧库迁移。
- **`ErrorInfo` 扩展增加每轮 progress 序列化体积** -> `failed_tool_sequence`/`retry_history` 仅在终态填充，运行中不携带；JSONL 仅写增量事件。
- **根因分类误判** -> 保留 `Unknown` 兜底；分类逻辑集中可测；提供 `--raw` 输出原始 error_message 供人工核对。

## Migration Plan

1. 扩展 `ErrorInfo` + `FailureMode`（新增类别），保持 serde 向后兼容（`#[serde(default)]`）。
2. transcript store 幂等迁移加 4 列（`failure_diagnostics`/`root_cause`/`retry_history`/`project_path`）。
3. `subagent_loop.rs` 捕获路径填充新诊断字段；写入时记录 `project_path`。
4. `TraceSink`（异步 buffered writer）+ daemon SSE endpoint（feature-gated `daemon`）。
5. CLI `Subagent` 子命令（全局读取 + session 过滤）。
6. `SubagentTraceTool` 渲染适配新字段。
7. 配置项 + 文档（WGENTY.md 命令表与配置表）。

回滚：新列可空、serde `default`，旧二进制读取新库时忽略未知列；禁用 `subagent.trace.sink` 即关闭流式输出。

## Resolved Questions (brainstorming)

- **失败轮次上下文截断阈值**：默认 2000 字符，`subagent.trace.context_char_limit` 可调，char-boundary 安全截断（Q4）。
- **SSE 范围**：全局订阅所有 session，`?session_id`/`?since` 过滤，冷启动从全局 db 回放（Q5）。
- **trace JSONL 文件位置**：项目本地 `<project>/.wgenty-code/traces/<session_id>.jsonl`（Q2）。
- **流式写入策略**：异步 buffered writer task，不阻塞 agent loop（Q3）。
- **CLI 范围**：全局 db + session 过滤，预留 `project_path` 列（Q1）。
