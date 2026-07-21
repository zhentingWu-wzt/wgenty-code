# Subagent Progress Checkpoint - subagent-chain-tracing

> build_mode=executing-plans（subagent 模型不可靠，已切内联）。协调者=主会话直接实现。
> 恢复：读 tasks.md（看复选框）+ git log，从第一个未勾选 task 继续。

## ✅ Section 1 Complete: Failure Diagnostics Data Model (1.1-1.6)
- Tasks 1-6: failure_diagnostics.rs 类型 + ErrorInfo 扩展 + FailureMode classify_with_signals/to_root_cause + subagent_loop emit() 捕获 root_cause/failed_tool_sequence/failed_round_context/retry_history + redact_params。67+ tests GREEN。

## ✅ Section 2 Complete: Transcript Storage Adaptation (2.1-2.4)
- Task 7 (63d29f32) 2.1: run_migrations 幂等 ALTER TABLE ADD COLUMN x4（failure_diagnostics/root_cause/retry_history/project_path），PRAGMA table_info 守卫 + column_exists helper。3 迁移测试。
- Task 8 (5bd6b0ef) 2.2-2.4: SubagentTranscript += failure_diagnostics(Option<ErrorInfo>)+project_path；SubagentTranscriptHeader += root_cause(FailureRootCause)+project_path。save() 同事务写 4 列（失败 JSON，成功 NULL）；get_by_id/list/search round-trip + NULL->Unknown/None 降级。build_transcript 加参数，task 调用点注入 error_details + project_path。22 transcript tests GREEN，clippy/fmt 干净。

**累计: 89+ 测试 GREEN，clippy --all-targets -D warnings 零 warning，cargo fmt --check 通过。**

## Next Task

- **Task 11** = tasks.md 3.3: Add bounded broadcast channel for trace events; on full, drop oldest for live subscribers only (persistence unaffected). 需要在 `TraceSink` 内增加 `tokio::sync::broadcast::Sender<TraceEvent>`（容量上限，如 1024），`emit`/callback 同时 `try_send` mpsc（文件）和 `broadcast.send`（daemon）；`broadcast.send` 失败（无订阅者或满）忽略不阻塞。提供 `subscribe() -> broadcast::Receiver`。`mode.writes_daemon()` 时才广播。daemon SSE endpoint (3.4) 订阅此 channel。

## Current Stage: implementing (Task 10 完成，Task 11 待开始)

## Task 10 (3.2) 完成证据
- config: `src/config/agent.rs` 加 `TraceSinkMode`（lowercase serde, default File, writes_file/writes_daemon）+ `SubagentTraceConfig { sink, dir: Option<PathBuf> }`；`SubagentLimits.trace` 字段 + Default。
- `trace_sink.rs`: `TraceSink::for_mode(mode, dir, session)`（off/daemon->None; file/both->Some，dir 默认 `<project_root>/.wgenty-code/traces`）+ `compose_progress_callback(orig, sink)`（双调/单调/None）。
- `subagent_loop.rs`: `build_trace_sink(settings, session_id)` helper；在 synthesis 之前构造 sink + composite callback（修正了 settings 移动顺序）；select 后 `sink.shutdown().await`（best-effort）。
- 验证：trace_sink + subagent_loop 46 测试 GREEN（新增 for_mode/compose/mode-serde 7 测试）；clippy --all-targets -D warnings 零 warning；cargo fmt 干净；lib 编译通过。
- 前向兼容：`daemon`/`both` 的广播部分留待 3.3（writes_daemon 方法已就位）。

## Task 9 (3.1) 完成证据
- 新文件 `src/teams/trace_sink.rs`：`TraceEvent`（compact, serde）+ `TraceSink`（mpsc 1024 + spawned writer task + oneshot shutdown）。
- `TraceEvent::from_progress`：current_params 解析 JSON 后 `redact_params`，否则原样 string；error = `redact_params(to_value(ErrorInfo))` 递归脱敏 failed_tool_sequence/retry_history；status 经 serde 取 variant 名。
- writer task：`tokio::select! { rx.recv() | &mut shutdown }`，batch drain（try_recv 突发）后单次 flush；None/shutdown 均 drain 剩余再退出，零丢失。
- 权限：unix `set_permissions` 0700 dir / 0600 file；非 unix no-op（跨平台编译）。
- `TraceSink::new` 必须 tokio runtime（spawn）；`callback()` 返回 `Arc::clone`；`shutdown(mut self).await` 发信号+释放 sender+await handle；`Drop` best-effort 发信号防 task 泄漏。
- 验证：8/8 trace_sink 测试 GREEN（JSONL append、脱敏写盘、non-json params、建缺失目录、shutdown drain 10 事件、unix 权限 0600/0700）；`cargo clippy --all-targets -- -D warnings` 零 warning；`cargo fmt` 已应用；teams:: 107 测试无回归。
- 已注册 `pub mod trace_sink;`（src/teams/mod.rs）。

## Key Facts (恢复用)
- transcript db 全局 `~/.wgenty-code/subagent_transcripts.db`（settings.storage.transcript.db_path 可配）；schema 已加 4 列含 project_path（Q1）
- trace JSONL 项目本地 `<project>/.wgenty-code/traces/<session_id>.jsonl`（Q2）
- TraceSink 异步 buffered writer（mpsc + spawn task），emit 用 try_send（ProgressCallback 同步 Fn）（Q3）
- context_char_limit 默认 2000 config 可调（Q4，subagent_loop.rs 用常量 DEFAULT_CONTEXT_CHAR_LIMIT=2000，Task 6.1 接 config）
- SSE 全局订阅 + ?session_id/?since 过滤 + 冷启动从 transcript store 回放（Q5）；endpoint `GET /api/v1/subagents/trace/stream`，feature-gated daemon，复用 require_auth
- ProgressCallback = Arc<dyn Fn(SubagentProgress) + Send + Sync>（同步，不能 .await）
- 大文件编辑优先用 Python 脚本（execute_command）避免 file_edit 全文 dump；apply_patch 对 CJK/隐藏 unicode 上下文易失败
- COMET_BASH 须用 bash（/bin/sh 不支持进程替换），调用前 `COMET_BASH="$(command -v bash)"`
- 当前分支 feature/subagent-chain-tracing，base-ref 21637d09
- plan: docs/superpowers/plans/2026-07-21-subagent-chain-tracing.md；design: openspec/changes/subagent-chain-tracing/design.md

## Notes
- 剩余: Section 3(TraceSink+SSE，最大) / Section 4(CLI) / Section 5(渲染) / Section 6(配置/文档/lint/跨平台) + verify + archive
