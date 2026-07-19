# Task 6 Brief: verify_fail hook + unverified 兜底

## Goal
verify 失败触发 `hooks.verify_fail()` (AutoRetry/Escalate/Abort/WarnAndContinue)；agent 完成但没调 verify_and_complete 时，agent loop 兜底标 Unverified。

## Key Decisions

### D1: VerifyFailAction 加 WarnAndContinue 变体
- Plan 6.4 要求 WarnAndContinue -> Completed(警告)
- Task 2 实现了 AutoRetry{remaining}/Escalate/Abort，缺 WarnAndContinue
- **决定**: 加 `WarnAndContinue` 变体（additive，不破坏 Task 2 测试）
- 语义映射: plan 的 `Block` -> `Abort`/`Escalate`，`AutoRetry{max}` -> `AutoRetry{remaining}`

### D2: hook 调用点在 verify_and_complete 失败分支
- 计算 fail_reason + append verify_log 后，调 `hooks.verify_fail(ctx)`
- `attempt` = verify_log 中的 attempt_num（1-based）
- 按 action 转换 status + 设置 verify_log final_status

### D3: status 转换矩阵
| Action | session.status | verify_log.final_status |
|---|---|---|
| AutoRetry{remaining>0} | InProgress (不变) | None (留 open) |
| AutoRetry{remaining:0} | Failed (defensive) | Failed |
| Escalate | Failed | Failed |
| Abort | Failed | Failed |
| WarnAndContinue | Completed | Completed |
| (success) | Completed | Completed |

### D4: verify_log final_status 重构
- `append_verify_log` 不再设 final_status（只 append attempt entry）
- 新增 `set_verify_log_final_status(dir, status)` 独立函数
- 调用方按 action 显式设置 -> 职责清晰

### D5: VerifyResult 加 action 字段
- `action: Option<VerifyFailAction>` — None=success, Some=failure+runtime 决策
- agent 看到 action 知道：AutoRetry 可重试 / Escalate 已升级 / WarnAndContinue 已放行

### D6: unverified 兜底方法
- `VerifyGate::mark_unverified_if_incomplete() -> Result<UnverifiedOutcome>`
- InProgress -> Unverified + final_status=Unverified
- 已终态(Completed/Failed/Unverified) -> no-op
- agent loop (Task 7) 在 session 结束信号时调用

### D7: 工作区保留不变式
- 失败不回退：failed verify 后工作区文件保留（spec §3.3 核心原则）
- 测试显式验证：write file -> verify fail -> file still on disk

## Tests (6.1-6.5)
- 6.1: NoHooks(默认 AutoRetry max:2) attempt=1 -> InProgress + 工作区保留
- 6.2: 连续失败 attempt=3 -> Escalate -> Failed + final_status=Failed
- 6.3: custom hook Abort -> Failed(立即, attempt 1)
- 6.4: custom hook WarnAndContinue -> Completed(final_status=Completed, attempt result=CommandFailed)
- 6.5: mark_unverified_if_incomplete: InProgress->Unverified; 已 Completed->no-op

## Files
- Modify: `src/exec_session/hooks.rs` (加 WarnAndContinue)
- Modify: `src/exec_session/verify_gate.rs` (hook 调用 + 兜底 + 测试)
