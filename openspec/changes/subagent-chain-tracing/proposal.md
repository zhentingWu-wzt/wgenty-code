## Why

Subagent 链路追踪的基础设施（`SubagentProgress` 事件流、`SubagentHealthAnalyzer` 成功率统计、`SubagentTranscriptStore` 持久化、`SubagentTraceTool` 渲染）已存在，但存在三个缺口：

1. **无独立 CLI 查询入口**：现有 `subagent_trace` 工具只能在会话内由 agent 调用且依赖 `session_id`，无法脱离会话查看历史 subagent 运行与成功率。
2. **trace 无法实时流式输出**：只能事后在会话内查询，外部工具（dashboard / Perfetto）无法实时订阅。
3. **失败原因粒度不足**：`ErrorInfo` 仅保留最后一个工具与布尔 `retryable`，无法还原"怎么走到失败"的完整工具调用序列、根因分类、失败轮次上下文与重试历史。

本次补齐这三个缺口，使 subagent 执行可被完整还原、实时观测与离线复盘。

## What Changes

- **失败诊断数据模型**：扩展失败捕获，记录完整失败工具调用序列（tool name + 关键参数 + 每步耗时）、失败根因分类标记（token 超预算 / guardian 拒绝 / sandbox 失败 / API 错误 / 工具 panic / 超时 / 用户取消 / 未知）、失败轮次完整上下文（assistant 文本 + 工具原始输出，截断存储）、重试历史（每次重试错误 + 重试策略 + 最终是否成功）。
- **实时流式 trace 输出**：新增本地 JSONL trace 文件 sink（`<project>/.wgenty-code/traces/<session_id>.jsonl`）与 daemon SSE endpoint（`GET /api/v1/subagents/trace/stream`），外部工具可实时订阅。
- **CLI 历史查询**：新增 `wgenty-code subagent` CLI 子命令（`list` / `trace` / `health`），脱离会话查看历史运行、单次完整 trace、成功率与失败模式统计。
- **存储与渲染适配**：`SubagentTranscriptStore` 持久化新增失败诊断字段；`SubagentTraceTool` 渲染（call_tree / error_timeline / chrome_trace / html）展示新失败详情。

## Capabilities

### New Capabilities

- `subagent-failure-diagnostics`: 失败根因分类、完整失败工具调用序列（含每步耗时）、失败轮次完整上下文（截断）、重试历史的捕获与数据模型契约。
- `subagent-trace-streaming`: subagent 执行 trace 实时流式输出到本地 JSONL 文件 sink 与 daemon SSE endpoint，供外部实时订阅。
- `subagent-cli-tracing`: 独立 CLI 子命令 `wgenty-code subagent list|trace|health`，离线查询历史 subagent 运行、单次 trace、成功率与失败模式。

### Modified Capabilities

- `subagent-transcript-storage`: 持久化 schema 扩展以存储新增失败诊断字段（完整工具调用序列、根因分类、失败轮次上下文、重试历史），并支持 CLI 历史查询与流式 sink 读取。
- `subagent-trace-html-report`: trace 渲染（call_tree / error_timeline / chrome_trace / html）展示新增失败诊断详情。

## Impact

- **代码**：`src/agent/progress.rs`（`SubagentProgress` / `ErrorInfo` 扩展）、`src/teams/subagent_loop.rs`（失败捕获 + 重试历史）、`src/teams/subagent_health.rs`（根因分类统计）、`src/teams/subagent_trace.rs`（渲染新字段）、transcript store（schema 迁移）、`src/cli/`（新增 `Subagent` 子命令）、`src/daemon/`（新增 SSE endpoint）、新增 trace 文件 sink 模块。
- **配置**：新增 `subagent.trace.sink`（`file` | `daemon` | `both` | `off`，默认 `file`）、`subagent.trace.dir`；保留窗口复用 `storage.transcript.max_age_days`。
- **依赖**：daemon feature 已有 axum / tower，SSE 复用现有基础设施；无新外部依赖。
- **存储**：transcript schema 变更需向后兼容迁移（旧记录缺字段降级为 `unknown` / 空）。
- **安全**：trace 文件含工具参数，需遵循现有 guardian 脱敏策略；daemon endpoint 复用现有鉴权。
