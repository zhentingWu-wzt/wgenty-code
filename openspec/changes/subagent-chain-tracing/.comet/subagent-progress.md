# Subagent Progress Checkpoint - subagent-chain-tracing

> build_mode=executing-plans（subagent 模型不可靠，已切内联）。协调者=主会话直接实现。
> 恢复：读 tasks.md（看复选框）+ plan + git log，从第一个未勾选 task 继续。

## ✅ Section 1 Complete: Failure Diagnostics Data Model (tasks 1.1-1.6)

- **Task 1** (a1050015) 1.2: failure_diagnostics.rs 类型（FailureRootCause/ToolCallStep/FailedRoundContext/RetryAttempt/redact_params/truncate_char_safe）。5 tests GREEN。
- **Task 2** (6eef8d1a) 1.1: ErrorInfo 加 4 字段 + ErrorType/ErrorInfo Default + 7 处构造点 ..Default::default()。24 tests GREEN。
- **Task 3** (5db7be86) 1.3: FailureMode 加 3 变体 + FailureSignals + classify_with_signals() + to_root_cause()。18 tests GREEN。
- **Task 4** (384b949a feat + c2cb5a28 style) 1.4: emit() 捕获点填充 root_cause/failed_tool_sequence/failed_round_context（4 纯函数）。17 tests GREEN。
- **Task 5** (98ba484b) 1.5: RetrySignal + build_retry_history()（no in-loop retry -> 单次失败空 history，helper 就位待未来重试路径）。3 tests GREEN。
- **Task 6** (无新 commit) 1.6: redact_params 已存在（Task 1）+ 已应用于 ToolCallStep（Task 4）+ 已测试（redacts_nested_sensitive_keys）；trace emission 应用属 Section 3。
- 附带: 663c5544 (SessionId 修复) / 92f44f02 (预存 clippy+e2e gate) / c2cb5a28 (fmt 债)

**Section 1 累计: 67+ 测试 GREEN，clippy --all-targets -D warnings 零 warning，cargo fmt --check 通过。**

## Next Task

- **Task 7** = tasks.md 2.1: Transcript storage 幂等迁移 -- `run_migrations` (`src/transcript/store.rs`) 加 `ALTER TABLE subagent_transcripts ADD COLUMN` failure_diagnostics/root_cause/retry_history/project_path（Q1 第 4 列），用 `PRAGMA table_info` 守卫保证幂等。TDD：先写迁移测试（旧库无新列 -> 迁移后 4 列存在 + 二次 open 不报错）看 RED。

## Current Stage: implementing (Task 7 待开始)

## Key Facts (恢复用)
- transcript db 全局 `~/.wgenty-code/subagent_transcripts.db`（settings.storage.transcript.db_path 可配），CLI 全局显示 + session 过滤，schema 加 4 列含 project_path（Q1）
- trace JSONL 项目本地 `<project>/.wgenty-code/traces/<session_id>.jsonl`（Q2）
- TraceSink 异步 buffered writer（mpsc + spawn task）（Q3）
- context_char_limit 默认 2000 config 可调（Q4，当前 subagent_loop.rs 用常量 DEFAULT_CONTEXT_CHAR_LIMIT=2000，Task 6.1 接 config）
- SSE 全局订阅 + ?session_id/?since 过滤 + 冷启动回放（Q5）
- 迁移：run_migrations 是单次 execute_batch，加列需独立 ALTER TABLE ADD COLUMN + PRAGMA table_info 守卫（SQLite ADD COLUMN 不支持 IF NOT EXISTS）
- ProgressCallback 是同步 Fn，TraceSink emit 须用 try_send
- apply_patch 对含 CJK/隐藏 unicode 的上下文易失败 -> 优先 file_write 整体重写或 file_edit 单行锚点；cargo fmt 会重排长行，patch 前注意上下文已格式化
- COMET_BASH 须用 bash（/bin/sh 不支持进程替换），调用前 `COMET_BASH="$(command -v bash)"`
- 当前分支 feature/subagent-chain-tracing，base-ref 21637d09
- plan: docs/superpowers/plans/2026-07-21-subagent-chain-tracing.md
- design: openspec/changes/subagent-chain-tracing/design.md（含 Q1-Q5 决策）

## Notes
- subagent_loop 无 in-loop subagent 级重试（仅 stream 层 per-round 重试 + 父级 attempt_model_fallback re-dispatch），故 retry_history 单次失败为空（符合 spec "leave empty on no-retry"）
- 剩余: Section 2(存储迁移) / Section 3(TraceSink+SSE) / Section 4(CLI) / Section 5(渲染) / Section 6(配置/文档/lint/跨平台)
