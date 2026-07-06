# Brainstorm Summary

- Change: fix-subagent-timeout-default
- Date: 2026-07-06

## 背景（追溯式设计）

本次 design 从 hotfix 升级而来。提交 fa9d8b8 捆绑了两组改动：
- **Set A**（已文档化）：subagent 默认超时 240→1800 + 移除主循环 `AGENT_LOOP_TIMEOUT`。proposal/design/tasks 已覆盖，实现完成。
- **Set B**（未文档化，触发升级）：`SubagentError` 结构化错误 + `partial_result` 交付，跨 `subagent_loop.rs`/`task.rs`/`refactor_e2e_test.rs`。已实现、已提交、build/clippy/test 通过。

Design Doc 聚焦 Set B，Set A 作为已文档化上下文引用。

## 已确认决策

1. **Design 范围**：Document as-built only — Design Doc 记录当前 SubagentError 实现的设计决策与权衡作为最终设计，不提出实现改动。
2. **Delta spec**：为 `subagent-result-delivery` 补 delta spec 验收场景（失败侧结构化错误码 + partial_result 交付）。
3. **技术方案**：方案 1（as-built 结构化 SubagentError + partial_result）确认为最终设计。

## 确认的技术方案

### 方案选型
采用方案 1（as-built）：`SubagentError{message, error_type, partial_result}`，`run_subagent_loop` 返回 `Result<String, SubagentError>`，`full_message()`/`code()`/`Display`/`From<String>`。对比方案 2（裸字符串，无部分结果挽救）与方案 3（仅枚举无 partial_result，丢失核心价值），方案 1 直接解决原 bug"跑着跑着没结果"。

### 架构与数据流
- `SubagentError`（`src/teams/subagent_loop.rs`）：`{ message: String, error_type: ErrorType, partial_result: Option<String> }`，`#[derive(Debug, Clone)]`。
- `ErrorType`（`src/agent/progress.rs:50`，复用既有）：`Timeout` / `BudgetExceeded{limit_k,used}` / `Stuck{reason}` / `ToolError{tool,message}` / `ParseError{message}` / `Unknown`。
- `run_subagent_loop` 返回 `Result<String, SubagentError>`；`partial_result` 取自 `text_snapshot: Mutex<Option<String>>`（每轮 `:430` 刷新）。
- 消费方 `TaskTool`（`src/tools/meta/task.rs`）：`Err(e)` → 保存失败 transcript（snapshot=`e.full_message()`）+ 返回 `ToolError{ message: e.full_message(), code: e.code() }`。RLM 路径 `.map_err(SubagentError::from)`。

### 错误分类法（as-built 特性，如实记录不改动）
- 实际构造：`BudgetExceeded`/`Stuck`（卡死检测）/`Stuck`（max-rounds，语义偏移）/`ParseError`/`Timeout`/`Unknown`。
- `code()` 映射：`subagent_timeout`/`budget_exceeded`/`subagent_stuck`/`subagent_tool_error`（loop 中未构造，dead in feature）/`subagent_parse_error`/`subagent_error`。
- `From<String>`：RLM 错误降级为 `Unknown`，丢失类型信息。

### partial_result 交付
- 内联：`full_message()` 拼入 partial_result → `ToolError.message` + 失败 transcript snapshot。
- 不走 mailbox/disk-persistence（仅成功大结果走该路径）。大 partial_result 直接进父上下文——as-built 已知取舍。

## 关键取舍与风险

- **取舍：内联 partial_result 而非 mailbox**：简单、直接解决"没结果"；代价是大 partial 膨胀父上下文。as-built 接受。
- **风险 1：`From<String>` 丢类型**：RLM pipeline 错误降级为 `Unknown`/`subagent_error`，父 agent 无法据 code 区分 RLM 失败原因。as-built 接受。
- **风险 2：`ToolError` variant dead in feature**：`subagent_tool_error` code 永不发出。as-built 接受。
- **风险 3：max-rounds 归为 `Stuck`**：语义偏移，但 `code()` 仍发出 `subagent_stuck`，不影响父 agent 决策。as-built 接受。
- **风险 4：`text_snapshot.lock().unwrap()`**：mutex 中毒时会 panic。as-built 接受（loop 内无其他持锁点中毒）。

## 测试策略

- 既有覆盖（已通过）：85 个 subagent lib 测试、`refactor_e2e_test.rs` 适配 `e.message`、`cargo test --no-run` 全编译、`cargo clippy --lib -- -D warnings` 零 warning。
- delta spec 新增场景的可断言性由 `code()` 与 `full_message()` 行为保证；as-built only 不新增测试，Design Doc 记录场景作为未来验证锚点。

## Spec Patch

回写 `openspec/changes/fix-subagent-timeout-default/specs/subagent-result-delivery/spec.md`（delta spec，仅 `## ADDED Requirements`）：

- **Requirement: Failed subagent delivers structured error code and partial results**
  - Scenario: subagent 超时 → 父 agent 收到 `subagent_timeout` code + 失败前最后文本快照
  - Scenario: subagent 预算耗尽 → 父 agent 收到 `budget_exceeded` code + partial_result
  - Scenario: partial_result 为空 → `full_message()` 仅返回 message，不追加空 partial 段
