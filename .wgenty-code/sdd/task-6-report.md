# Task 6 Report: verify_fail hook + unverified 兜底

## Summary
实现 verify 失败时的 `SessionHooks::verify_fail` 调用（AutoRetry/Escalate/Abort/WarnAndContinue 四种决策）+ agent loop 兜底标 Unverified 的 `mark_unverified_if_incomplete` 方法。失败不回退（工作区保留）的核心不变式由测试显式验证。

## Changes

### `src/exec_session/hooks.rs`
- `VerifyFailAction` 新增 `WarnAndContinue` 变体（plan 6.4 要求）：hook 可决定"接受失败，标 Completed"。用于 flaky test / 预存在失败等可接受场景。
- 更新枚举 doc 注释，说明四种 action 语义。

### `src/exec_session/verify_gate.rs`

**VerifyGate 结构体**
- 新增 `hooks: Arc<dyn SessionHooks>` 字段
- `new(coordinator, executor, hooks)` 改为 3 参数
- 新增 `new_with_default_hooks(coordinator, executor)` 便捷构造（用 `NoHooks`，默认 AutoRetry{max:2}）

**verify_and_complete 失败路径**
- 失败时构建 `VerifyFailContext { session_id, turn_id, attempt, failure }`，`attempt` = verify_log 中的 attempt_num（1-based）
- 调 `hooks.verify_fail(&ctx)` -> `VerifyFailAction`
- 按 action 转换 status + 设置 verify_log final_status：

| Action | session.status | verify_log.final_status |
|---|---|---|
| AutoRetry{remaining>0} | InProgress（不变） | None（留 open） |
| AutoRetry{remaining:0} | Failed（defensive） | Failed |
| Escalate | Failed | Failed |
| Abort | Failed | Failed |
| WarnAndContinue | Completed | Completed |

- `VerifyResult` 新增 `action: Option<VerifyFailAction>` 字段，agent 可见 runtime 决策

**verify_log 重构**
- `append_verify_log` 不再设 final_status（只 append attempt entry）
- 新增 `set_verify_log_final_status(dir, status)` 独立函数，由调用方按 action 显式调用
- 职责分离：append 只记录 attempt，final_status 由 hook 决策驱动

**unverified 兜底**
- 新增 `VerifyGate::mark_unverified_if_incomplete() -> Result<UnverifiedOutcome>`
- InProgress -> Unverified + final_status=Unverified
- 已终态(Completed/Failed/Unverified) -> no-op（返回 `AlreadyTerminal(status)`）
- agent loop (Task 7) 在 session 结束信号时调用
- 新增 `UnverifiedOutcome { MarkedUnverified, AlreadyTerminal(SessionStatus) }` 枚举

**format_verify_result**
- 失败输出新增 Action 行：retry allowed (N remaining) / escalated / aborted / accepted with warning

## Tests (6 new, 61 total)
- `task6_default_hook_attempt1_keeps_in_progress_and_preserves_workspace` (6.1): NoHooks attempt=1 -> AutoRetry{remaining:2}, status InProgress, **工作区文件保留**（显式验证不变式）
- `task6_consecutive_failures_exhaust_budget_marks_failed` (6.2): attempt 1/2 -> AutoRetry, attempt 3 -> Escalate -> Failed + final_status=Failed
- `task6_custom_hook_abort_marks_failed_immediately` (6.3): custom hook Abort -> Failed（attempt 1 立即）
- `task6_custom_hook_warn_and_continue_marks_completed` (6.4): custom hook WarnAndContinue -> Completed，verify_log attempt result=CommandFailed 但 final_status=Completed（可审计）
- `task6_mark_unverified_when_in_progress` (6.5a): InProgress -> Unverified, final_status=Unverified, 无 attempts
- `task6_mark_unverified_noop_when_already_terminal` (6.5b): 已 Completed -> no-op, status 不变

## Verification
- `cargo test exec_session` -> **61 passed** (55 旧 + 6 新), 0 failed
- `cargo test checkpoint` -> 21 passed（不受影响）
- `cargo test undo` -> 3 passed（不受影响）
- `cargo clippy --all-targets -- -D warnings` -> 零 warning
- `cargo fmt --check` -> clean
- 解耦不变式：`grep -rn "comet" src/exec_session/` 仅 2 处，均在 doc 注释/文档举例（spec §2.4 允许："除注释/文档举例"）

## Design Decisions

### D1: WarnAndContinue 加入 VerifyFailAction
Task 2 实现了 AutoRetry/Escalate/Abort，plan 6.4 要求 WarnAndContinue。**决定加**：additive 不破坏 Task 2 测试，有真实场景（flaky test），补全 hook 决策空间。语义："hook 权威接受失败，标 Completed"，verify_log 保留 attempt 的 CommandFailed result + final_status=Completed 做审计。

### D2: final_status 从 append_verify_log 移出
原来 append_verify_log 在 success 时设 final_status=Completed。Task 6 有 4 种失败 outcome，final_status 由 hook 决定。**移出到独立 `set_verify_log_final_status`**，append 只管记录 attempt，职责清晰。

### D3: AutoRetry{remaining:0} defensive -> Failed
well-behaved hook 在预算耗尽时返回 Escalate，但 remaining=0 理论可能。**defensive 转 Failed**，避免 session 卡在 InProgress 永远。

### D4: mark_unverified 不动工作区
Unverified 语义 = "工作可能 OK 但未证明"，不回退（用户可见标记后自行决定）。与 verify 失败一致：runtime 不抹工作。

## Spec Compliance
- §3.3 "gate 失败 ≠ 自动回退"：✓ 工作区保留（6.1 显式测试）
- §3.3 "触发 verify_fail hook（默认 AutoRetry{max:2}）"：✓ NoHooks 默认实现
- §3.3 "连续失败超 AutoRetry.max -> session.status = failed"：✓ 6.2 测试
- §3.3 "兜底(unverified)：agent 完成但没调 verify_and_complete -> 标记 unverified"：✓ 6.5 测试
- §3.3 "verify_log 记录每次 attempt + final_status"：✓ 重构后 final_status 按 hook 决策设置
- §2.4 解耦不变式：✓ 无 "comet" 代码引用（仅 doc 举例）

## Remaining
- Task 7: agent loop 集成（turn 边界挂 coordinator + 注册 verify_and_complete 工具 + session 结束调 mark_unverified_if_incomplete）
- Task 8: 端到端测试 + 解耦不变式验证
