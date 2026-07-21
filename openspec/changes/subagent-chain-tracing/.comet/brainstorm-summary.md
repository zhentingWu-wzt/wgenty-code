# Brainstorm Summary — subagent-chain-tracing

> Comet design 阶段增量检查点。未确认项标注 `[待确认]`，已确认项标注 `[已确认]`。
> 恢复时从本文件重建 brainstorm 上下文。

## 已确认需求（open 阶段）

1. **失败诊断数据模型**：完整失败工具调用序列（含每步耗时）、根因分类标记、失败轮次完整上下文（截断）、重试历史。
2. **实时流式 trace 输出**：本地 JSONL 文件 + daemon SSE endpoint（两者都要）。
3. **CLI 历史查询**：`wgenty-code subagent list|trace|health`。
4. 不改调度/重试策略本身；不导出 OTel/Jaeger；不重构 capability-scoped 路由。

## 已确认设计决策（design.md D1-D6，部分需修正）

- **D1** [已确认] 扩展 `ErrorInfo`（不新建表），诊断存为 header JSON 列；事件级序列复用 `subagent_events`。
- **D2** [已确认] 根因在捕获时从结构化信号确定，扩展 `FailureMode`，字符串匹配降级 `Unknown`。
- **D3** [已确认·需细化] 双通道流式（JSONL 文件 + daemon SSE），由 `ProgressCallback` 驱动，有界广播满则丢最旧。
- **D4** [已确认] CLI 子命令复用 transcript store。
- **D5** [已确认·需修正] 迁移：现有 `run_migrations` 是单次 `execute_batch`（CREATE TABLE IF NOT EXISTS），无版本系统。加列需**独立步骤** `ALTER TABLE ADD COLUMN` + `PRAGMA table_info` 守卫（SQLite ADD COLUMN 不支持 IF NOT EXISTS）。
- **D6** [已确认] 配置项 `subagent.trace.sink/dir/context_char_limit`。

## 待确认问题（brainstorming）

### Q1 [已确认] transcript db 位置与 CLI 范围
**事实**：`settings.storage.transcript.db_path` 默认 `~/.wgenty-code/subagent_transcripts.db`（**全局**），非项目本地。
**决策**：**全局 + 预留 `project_path` 列**。v1 CLI `list` 全局显示所有 subagent，用 `--session`/`--status`/`--limit` 过滤；schema 新增 `project_path TEXT` 列，写入时记录 project root（CLI 暂不暴露项目过滤，为未来预留）。迁移多加一列 `project_path`。

### Q5 [已确认] SSE 范围
**决策**：**全局**（跟随 Q1）。daemon SSE 订阅所有 session 的 trace 事件，`?session_id`/`?since` 过滤；冷启动从全局 transcript db 回放。

### Q2 [已确认] trace JSONL 文件位置
**决策**：**项目本地** `<project>/.wgenty-code/traces/<session_id>.jsonl`。由运行中 session（已知 project root）写入；CLI 不读 JSONL（读全局 db），故不冲突。

### Q3 [已确认] 流式 sink 写入策略
**决策**：**异步 buffered writer task**。`TraceSink` 持有 mpsc channel，spawn 写入任务批量 flush，不阻塞 `ProgressCallback`/agent loop；channel 满时丢最旧事件（持久在 db 不受影响）。

### Q4 [已确认] 失败轮次上下文截断阈值
**决策**：**默认 2000 字符，config 可调**（`subagent.trace.context_char_limit`）；char-boundary 安全截断。

## 下一步
全部 Q1-Q5 已确认。brainstorm-summary 定稿 -> [active compaction gate] -> 更新 Design Doc（design.md 纳入 Q1-Q5 决策）-> guard --apply -> build。
