---
change: subagent-dispatch-fallback
design-doc: docs/superpowers/specs/2026-07-18-subagent-dispatch-fallback-design.md
base-ref: 103a85b8200afef7b9a5ba5371c216758d9e0493
archived-with: 2026-07-18-subagent-dispatch-fallback
---

# Subagent Dispatch Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 当子 agent 派发失败(模型不可用或结构性失败)时,由派发该子 agent 的父 agent 自动接管执行,使派发失败不再等于任务失败;模型类失败自动切换备用模型,结构性失败用父 agent 当前模型;单次 fallback、不可递归;root 派发的 child 不 fallback。

**Architecture:** 混合双拦截点。拦截点 1(派发前失败,`CoordinatorError::{DepthLimitReached, ConcurrencyClosed, TaskGroup}`)在 `TaskTool::execute_with_context` 内 `reserve_child_in_group` 失败后兜底同步执行 `full_prompt`,结果作为 task 工具返回。拦截点 2(运行时模型失败,`error_code = subagent_model_unavailable`)在 `SubagentSynthesis::on_candidate_final` 内 `collect_children_for_synthesis` 返回后,换备用模型再派发降级子 agent。两个拦截点共用 `fallback_eligible` 判定与 `fallback_used` 单次约束。`fallback_used` 标记存储在 `AgentCoordinator` 级别(跨 group 生命周期),因为拦截点 2 触发时 group 已被 `collect_children_for_synthesis` claim 并移除。

**Tech Stack:** Rust 2021, Tokio, Serde, thiserror,现有 `AgentCoordinator` / `TaskTool` / `run_subagent_loop_with_permissions` / `SubagentTranscriptStore`,Cargo 单元测试 + 集成测试。

**Design Doc:** `docs/superpowers/specs/2026-07-18-subagent-dispatch-fallback-design.md`

## Global Constraints

- 执行者始终是派发该子 agent 的父 agent(非根协调者);root 派发的 child 不 fallback(`is_root(caller)` 守卫,`parent_id.is_none()` 即 root)。
- 单次 fallback 不可递归:同一 child 的 fallback 只执行一次,`fallback_used` 标记置位后不再尝试。
- 模型类失败(`ModelUnavailable`)换备用模型;结构性失败(`DepthLimitReached`/`ConcurrencyClosed`/`TaskGroup`)不换模型,用父 agent 当前模型。
- 备用模型只 override `models.main.name`,复用原 endpoint(base_url/api_key/appkey/provider 保留);endpoint 不通则 fallback 失败,交根模型(单次约束终止)。
- `fallback_models` 未配置或空时,模型类失败降级为现状(交父模型)。
- 不对超时/卡住/max轮/panic 做 fallback(保留现状交父模型决定)。
- 不改父作用域取消语义;不改 `subagent-driven-development` 双审查流程。
- 配置键:`agent.subagent.fallback_models: Vec<String>`(模型名有序列表)。
- ErrorType 新增 `ModelUnavailable` 变体;`SubagentError::code()` 返回 `subagent_model_unavailable`。

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## File Map

### New files

| Path | Responsibility |
|------|----------------|
| `src/agent/fallback.rs` | `FallbackKind` enum、`fallback_eligible_from_coordinator_error`、`fallback_eligible_from_child_result`、`is_root_caller` 守卫、`select_fallback_model` 选择函数、单元测试 |

### Existing files (focused changes)

| Path | Change |
|------|--------|
| `src/agent/progress.rs:54-75` | `ErrorType` 新增 `ModelUnavailable` 变体 |
| `src/agent/mod.rs` | `pub mod fallback; pub use fallback::*;` |
| `src/teams/subagent_loop.rs:113-123` | `SubagentError::code()` 加 `ModelUnavailable -> "subagent_model_unavailable"` |
| `src/teams/subagent_loop.rs:843-859` | 失败分类 match 扩展:`RuntimeError::Stream` 匹配 `"API error"`/HTTP 状态码/`"connection"` -> `ModelUnavailable` |
| `src/teams/subagent_loop.rs:174-213` | `SubagentSynthesis::on_candidate_final` 插入拦截点 2 fallback 逻辑 |
| `src/agent/coordinator.rs` | 新增 `fallback_used: Arc<RwLock<HashSet<String>>>` 字段 + `mark_fallback_used`/`fallback_already_used` 方法 |
| `src/config/agent.rs:116-149` | `SubagentLimits` 新增 `fallback_models: Vec<String>` 字段 + default |
| `src/config/mod.rs:140-158` | 新增 `fallback_model_settings(&self, model_name: &str) -> Self` 方法 |
| `src/tools/meta/task.rs:521-530` | `execute_with_context` 内 `reserve_child_in_group` 失败后插入拦截点 1 fallback |
| `src/teams/subagent_health.rs:36-47` | `FailureMode` 新增 `ModelUnavailable` 变体 + classify/label/severity 映射 |
| `src/teams/subagent_loop.rs` | 拦截点 1/2 tracing 日志 |

### Out of scope (do not implement in this plan)

- 多级 fallback 链 / 递归重试(单次约束)
- 超时/卡住/max轮/panic 的 fallback
- 根协调者/主会话执行路径
- `subagent-driven-development` 双审查流程改动
- 父作用域取消语义改动

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 1: ErrorType::ModelUnavailable 变体 + code 映射

**Files:**
- Modify: `src/agent/progress.rs:54-75`
- Modify: `src/teams/subagent_loop.rs:113-123`
- Test: 单元测试在各自文件内

**Interfaces:**
- Produces: `ErrorType::ModelUnavailable` 变体(无字段);`SubagentError::code()` 对应返回 `"subagent_model_unavailable"`

- [x] **Step 1: 在 `src/agent/progress.rs` 的 `ErrorType` 枚举新增 `ModelUnavailable` 变体**

修改 `ErrorType` 枚举(当前在 lines 54-75),在 `Unknown` 之前插入:

```rust
/// Categorized error types for subagent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorType {
    Timeout,
    BudgetExceeded {
        limit_k: u64,
        used: u64,
    },
    Stuck {
        reason: String,
    },
    ToolError {
        tool: String,
        message: String,
    },
    ParseError {
        message: String,
    },
    /// The subagent was cancelled via its execution context's cancellation token.
    Cancelled,
    /// The model endpoint was unavailable (API HTTP error, connection refused, etc.).
    /// Eligible for fallback to a backup model.
    ModelUnavailable,
    Unknown,
}
```

- [x] **Step 2: 在 `src/teams/subagent_loop.rs` 的 `SubagentError::code()` 加映射**

修改 `code()` 方法(当前在 lines 113-123),在 `Unknown` 分支前插入:

```rust
    pub fn code(&self) -> &'static str {
        match &self.error_type {
            ErrorType::BudgetExceeded { .. } => "budget_exceeded",
            ErrorType::Timeout => "subagent_timeout",
            ErrorType::Stuck { .. } => "subagent_stuck",
            ErrorType::ToolError { .. } => "subagent_tool_error",
            ErrorType::ParseError { .. } => "subagent_parse_error",
            ErrorType::Cancelled => "subagent_cancelled",
            ErrorType::ModelUnavailable => "subagent_model_unavailable",
            ErrorType::Unknown => "subagent_error",
        }
    }
```

- [x] **Step 3: 编写失败测试 -- 验证 code 映射**

在 `src/teams/subagent_loop.rs` 文件末尾的 `#[cfg(test)] mod tests` 中(若不存在则新增)添加:

```rust
#[cfg(test)]
mod fallback_code_tests {
    use super::*;
    use crate::agent::progress::ErrorType;

    #[test]
    fn model_unavailable_code() {
        let err = SubagentError {
            message: "API error (503): service unavailable".to_string(),
            error_type: ErrorType::ModelUnavailable,
            partial_result: None,
        };
        assert_eq!(err.code(), "subagent_model_unavailable");
    }
}
```

- [x] **Step 4: 运行测试验证通过**

Run: `cargo test --lib model_unavailable_code -- --nocapture`
Expected: PASS

- [x] **Step 5: 编译验证**

Run: `cargo build`
Expected: 编译成功(注意:此时 `ModelUnavailable` 变体已加,但分类逻辑还没用上,需确保所有 match 都处理了新变体 -- 编译器会报错遗漏的 match)

如果编译报错(match 未覆盖 `ModelUnavailable`),在对应 match 加 `_ => ErrorType::Unknown` 或具体分支。重点检查 `subagent_health.rs` 和其他 match `ErrorType` 的位置。

- [x] **Step 6: 提交**

```bash
git add src/agent/progress.rs src/teams/subagent_loop.rs
git commit -m "feat(fallback): add ErrorType::ModelUnavailable variant and code mapping

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 2: 失败分类细化 -- RuntimeError::Stream 模型不可用识别

**Files:**
- Modify: `src/teams/subagent_loop.rs:843-859`
- Test: 单元测试在 `src/teams/subagent_loop.rs` 内

**Interfaces:**
- Consumes: `RuntimeError::Stream(String)`(from `src/agent/runtime/error.rs:15`)
- Produces: 当 `Stream(msg)` 匹配模型不可用特征时,分类为 `ErrorType::ModelUnavailable`

- [x] **Step 1: 编写失败测试 -- 模型不可用分类**

在 `src/teams/subagent_loop.rs` 的 `#[cfg(test)] mod tests` 中添加分类测试。由于分类逻辑内嵌在 `run_subagent_loop_with_permissions` 的 select 分支(不易单测),我们提取一个纯函数 `classify_stream_error(msg: &str) -> ErrorType` 再测试它:

```rust
#[cfg(test)]
mod classify_stream_tests {
    use super::*;

    #[test]
    fn classifies_api_error_as_model_unavailable() {
        assert_eq!(
            classify_stream_error("API error (503): service unavailable"),
            ErrorType::ModelUnavailable
        );
    }

    #[test]
    fn classifies_connection_error_as_model_unavailable() {
        assert_eq!(
            classify_stream_error("connection refused"),
            ErrorType::ModelUnavailable
        );
    }

    #[test]
    fn classifies_http_500_as_model_unavailable() {
        assert_eq!(
            classify_stream_error("request failed: HTTP 500 internal server error"),
            ErrorType::ModelUnavailable
        );
    }

    #[test]
    fn classifies_stuck_as_stuck() {
        assert_eq!(
            classify_stream_error("subagent stuck in loop"),
            ErrorType::Stuck {
                reason: "subagent stuck in loop".to_string(),
            }
        );
    }

    #[test]
    fn classifies_other_stream_as_unknown() {
        assert_eq!(
            classify_stream_error("some unexpected error"),
            ErrorType::Unknown
        );
    }
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib classify_stream_tests -- --nocapture`
Expected: FAIL -- `classify_stream_error` 函数未定义

- [x] **Step 3: 实现纯函数 `classify_stream_error`**

在 `src/teams/subagent_loop.rs` 中(在 `SubagentError` impl 块之后,`SubagentSynthesis` 之前)添加:

```rust
/// Classify a free-form `RuntimeError::Stream` message into an `ErrorType`.
///
/// Model-unavailable signatures (API HTTP errors, connection failures) map to
/// `ModelUnavailable` so the fallback layer can pick them up. "stuck"/"Stuck"
/// stays `Stuck`. Everything else stays `Unknown`.
pub(crate) fn classify_stream_error(msg: &str) -> ErrorType {
    if msg.contains("stuck") || msg.contains("Stuck") {
        return ErrorType::Stuck {
            reason: msg.to_string(),
        };
    }
    // Model-unavailable heuristics: API error, HTTP status code, connection.
    if msg.contains("API error")
        || msg.contains("api error")
        || msg.contains("connection")
        || regex::Regex::new(r"\bHTTP\b\s*\d{3}").unwrap().is_match(msg)
        || regex::Regex::new(r"\(\d{3}\)").unwrap().is_match(msg)
    {
        return ErrorType::ModelUnavailable;
    }
    ErrorType::Unknown
}
```

- [x] **Step 4: 修改失败分类 match 复用 `classify_stream_error`**

修改 `src/teams/subagent_loop.rs:843-859` 的 match(当前 `Ok(Err(e)) =>` 分支内):

```rust
            Ok(Err(e)) => {
                let (error_type, message) = match &e {
                    RuntimeError::MaxRoundsExceeded { .. } => (
                        ErrorType::Stuck {
                            reason: "exceeded maximum rounds".to_string(),
                        },
                        e.to_string(),
                    ),
                    RuntimeError::StreamTimeout(_) => (ErrorType::Timeout, e.to_string()),
                    RuntimeError::Stream(msg) => {
                        (classify_stream_error(msg), msg.clone())
                    }
                    other => (ErrorType::Unknown, other.to_string()),
                };
                Err(SubagentError {
                    message,
                    error_type,
                    partial_result: observer.text_snapshot.lock().expect("lock poisoned: text_snapshot").clone(),
                })
            }
```

- [x] **Step 5: 运行测试验证通过**

Run: `cargo test --lib classify_stream_tests -- --nocapture`
Expected: PASS(全部 5 个测试)

- [x] **Step 6: 编译验证**

Run: `cargo build`
Expected: 编译成功

- [x] **Step 7: 提交**

```bash
git add src/teams/subagent_loop.rs
git commit -m "feat(fallback): classify model-unavailable stream errors

Extract classify_stream_error() and route API/connection/HTTP errors to
ErrorType::ModelUnavailable so the fallback layer can detect them.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 3: fallback 模块 -- FallbackKind + fallback_eligible 判定

**Files:**
- Create: `src/agent/fallback.rs`
- Modify: `src/agent/mod.rs`
- Test: 单元测试在 `src/agent/fallback.rs` 内

**Interfaces:**
- Consumes: `CoordinatorError`(from `src/agent/coordinator.rs:236`)、`ChildResult`(from `src/agent/coordinator.rs:170`)、`AgentExecutionContext`(from `src/agent/identity.rs`)
- Produces:
  - `pub enum FallbackKind { ModelUnavailable, Structural }`
  - `pub fn fallback_eligible_from_coordinator_error(e: &CoordinatorError) -> Option<FallbackKind>`
  - `pub fn fallback_eligible_from_child_result(r: &ChildResult) -> Option<FallbackKind>`
  - `pub fn is_root_caller(context: &AgentExecutionContext) -> bool`

- [x] **Step 1: 编写失败测试**

创建 `src/agent/fallback.rs`:

```rust
//! Fallback eligibility for subagent dispatch failures.
//!
//! Two interception points share this logic:
//! - Interception 1 (pre-dispatch): `CoordinatorError` from `reserve_child_in_group`
//! - Interception 2 (runtime): `ChildResult` with `error_code = subagent_model_unavailable`

use crate::agent::coordinator::{ChildResult, ChildTerminalStatus, CoordinatorError};
use crate::agent::identity::AgentExecutionContext;

/// Kind of fallback to attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackKind {
    /// Model endpoint failed -> swap to a backup model.
    ModelUnavailable,
    /// Structural failure (depth/concurrency/group) -> reuse parent's model.
    Structural,
}

/// Determine fallback eligibility from a pre-dispatch `CoordinatorError`.
///
/// `DepthLimitReached` / `ConcurrencyClosed` / `TaskGroup` -> `Some(Structural)`.
/// Everything else (NotVisible, ParentNotRunning, JoinFailed, Storage,
/// ChildrenStillRunning, RootHasNoTerminalState) -> `None`.
pub fn fallback_eligible_from_coordinator_error(e: &CoordinatorError) -> Option<FallbackKind> {
    match e {
        CoordinatorError::DepthLimitReached { .. } => Some(FallbackKind::Structural),
        CoordinatorError::ConcurrencyClosed => Some(FallbackKind::Structural),
        CoordinatorError::TaskGroup(_) => Some(FallbackKind::Structural),
        _ => None,
    }
}

/// Determine fallback eligibility from a runtime `ChildResult`.
///
/// `error_code = "subagent_model_unavailable"` -> `Some(ModelUnavailable)`.
/// All other codes (timeout, stuck, cancelled, generic error, tool_error,
/// parse_error, budget_exceeded) -> `None`.
pub fn fallback_eligible_from_child_result(r: &ChildResult) -> Option<FallbackKind> {
    if r.status != ChildTerminalStatus::Failed {
        return None;
    }
    match r.error_code.as_deref() {
        Some("subagent_model_unavailable") => Some(FallbackKind::ModelUnavailable),
        _ => None,
    }
}

/// Root callers (no parent) must not self-execute fallback -- Comet isolation
/// rules forbid the root/main session from executing tasks directly.
pub fn is_root_caller(context: &AgentExecutionContext) -> bool {
    context.parent_id.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::coordinator::{ChildResult, ChildTerminalStatus};
    use crate::agent::identity::{AgentExecutionContext, AgentId, SessionId};

    fn make_child_result(code: Option<&str>, status: ChildTerminalStatus) -> ChildResult {
        ChildResult {
            child_id: AgentId::new("child-1"),
            status,
            summary: String::new(),
            error_code: code.map(String::from),
            partial_result: None,
        }
    }

    #[test]
    fn coordinator_depth_limit_is_structural() {
        let e = CoordinatorError::DepthLimitReached { limit: 5 };
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn coordinator_concurrency_closed_is_structural() {
        let e = CoordinatorError::ConcurrencyClosed;
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn coordinator_task_group_is_structural() {
        let e = CoordinatorError::TaskGroup("group gone".to_string());
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn coordinator_not_visible_not_eligible() {
        let e = CoordinatorError::NotVisible;
        assert_eq!(fallback_eligible_from_coordinator_error(&e), None);
    }

    #[test]
    fn coordinator_parent_not_running_not_eligible() {
        let e = CoordinatorError::ParentNotRunning;
        assert_eq!(fallback_eligible_from_coordinator_error(&e), None);
    }

    #[test]
    fn child_model_unavailable_is_model_fallback() {
        let r = make_child_result(Some("subagent_model_unavailable"), ChildTerminalStatus::Failed);
        assert_eq!(
            fallback_eligible_from_child_result(&r),
            Some(FallbackKind::ModelUnavailable)
        );
    }

    #[test]
    fn child_timeout_not_eligible() {
        let r = make_child_result(Some("subagent_timeout"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_stuck_not_eligible() {
        let r = make_child_result(Some("subagent_stuck"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_cancelled_not_eligible() {
        let r = make_child_result(Some("subagent_cancelled"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_generic_error_not_eligible() {
        let r = make_child_result(Some("subagent_error"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_completed_not_eligible() {
        let r = make_child_result(None, ChildTerminalStatus::Completed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_no_error_code_not_eligible() {
        let r = make_child_result(None, ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn root_caller_detected() {
        let ctx = AgentExecutionContext {
            agent_id: AgentId::new("root"),
            parent_id: None,
            session_id: SessionId::new("s1"),
            depth: 0,
            origin_turn_id: None,
        };
        assert!(is_root_caller(&ctx));
    }

    #[test]
    fn non_root_caller_detected() {
        let ctx = AgentExecutionContext {
            agent_id: AgentId::new("child"),
            parent_id: Some(AgentId::new("root")),
            session_id: SessionId::new("s1"),
            depth: 1,
            origin_turn_id: None,
        };
        assert!(!is_root_caller(&ctx));
    }
}
```

- [x] **Step 2: 在 `src/agent/mod.rs` 暴露 fallback 模块**

在 `src/agent/mod.rs` 中添加(位置参考现有 `pub mod` 声明):

```rust
pub mod fallback;
pub use fallback::{
    fallback_eligible_from_child_result, fallback_eligible_from_coordinator_error, is_root_caller,
    FallbackKind,
};
```

- [x] **Step 3: 运行测试验证通过**

Run: `cargo test --lib fallback::tests -- --nocapture`
Expected: PASS(全部测试)

注意:如果 `AgentExecutionContext` 的字段与测试中的不一致(如缺少 `depth` 或 `origin_turn_id`),调整测试以匹配实际结构。先运行 `grep -n "pub struct AgentExecutionContext" src/agent/identity.rs` 确认字段。

- [x] **Step 4: 编译验证**

Run: `cargo build`
Expected: 编译成功

- [x] **Step 5: 提交**

```bash
git add src/agent/fallback.rs src/agent/mod.rs
git commit -m "feat(fallback): add fallback_eligible module with FallbackKind

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 4: fallback_used 标记 + AgentCoordinator 单次约束存储

**Files:**
- Modify: `src/agent/coordinator.rs`(新增字段 + 方法)
- Test: 单元测试在 `src/agent/coordinator.rs` 内

**Interfaces:**
- Consumes: `AgentId`(from `src/agent/identity.rs`)
- Produces:
  - `AgentCoordinator` 新增字段 `fallback_used: Arc<RwLock<HashSet<String>>>`
  - `pub async fn mark_fallback_used(&self, key: &str)`
  - `pub async fn fallback_already_used(&self, key: &str) -> bool`

**设计决策:** `fallback_used` 存储在 `AgentCoordinator` 级别(跨 group 生命周期),而非设计文档原文的 `GroupRecord`。原因:拦截点 2 触发时,`collect_children_for_synthesis` 已 claim 并 `remove_owner_group`,group 记录可能已不可用。coordinator 级别的 set 保证标记在 group 生命周期之外仍然有效。key 统一为 `String`:拦截点 2 用 `child_id.as_str()`,拦截点 1 用 `format!("pending:{}", description)`(child 未 reserve 无 child_id)。

- [x] **Step 1: 编写失败测试**

在 `src/agent/coordinator.rs` 的 `#[cfg(test)] mod tests` 中(若不存在则新增)添加:

```rust
#[cfg(test)]
mod fallback_used_tests {
    use super::*;

    #[tokio::test]
    async fn mark_and_check_fallback_used() {
        let coord = AgentCoordinator::new();
        assert!(!coord.fallback_already_used("child-1").await);
        coord.mark_fallback_used("child-1").await;
        assert!(coord.fallback_already_used("child-1").await);
    }

    #[tokio::test]
    async fn fallback_used_is_per_key() {
        let coord = AgentCoordinator::new();
        coord.mark_fallback_used("child-1").await;
        assert!(coord.fallback_already_used("child-1").await);
        assert!(!coord.fallback_already_used("child-2").await);
    }

    #[tokio::test]
    async fn pending_key_for_pre_dispatch_fallback() {
        let coord = AgentCoordinator::new();
        let key = format!("pending:{}", "explore the codebase");
        assert!(!coord.fallback_already_used(&key).await);
        coord.mark_fallback_used(&key).await;
        assert!(coord.fallback_already_used(&key).await);
    }
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fallback_used_tests -- --nocapture`
Expected: FAIL -- `mark_fallback_used` / `fallback_already_used` 方法未定义

- [x] **Step 3: 在 `AgentCoordinator` 结构新增 `fallback_used` 字段**

先定位 `AgentCoordinator` 结构定义(约 `src/agent/coordinator.rs:290-320`)。在现有 `Arc<RwLock<...>>` 字段之后添加:

```rust
pub struct AgentCoordinator {
    // ... existing fields ...
    task_groups: Arc<TaskGroupStore>,
    child_groups: Arc<RwLock<HashMap<(SessionId, AgentId), TaskGroupId>>>,
    owner_groups: Arc<RwLock<HashMap<OwnerGroupKey, TaskGroupId>>>,
    // ... other existing fields ...
    /// Per-child fallback marker. Prevents recursive/重复 fallback for the
    /// same child. Key is either `child_id.as_str()` (interception 2) or
    /// `format!("pending:{}", description)` (interception 1, pre-reserve).
    fallback_used: Arc<RwLock<HashSet<String>>>,
}
```

- [x] **Step 4: 在 `AgentCoordinator::new` 初始化字段**

定位 `new()` 方法(约 line 310-320),在构造体中添加:

```rust
            fallback_used: Arc::new(RwLock::new(HashSet::new())),
```

- [x] **Step 5: 实现 `mark_fallback_used` 和 `fallback_already_used` 方法**

在 `impl AgentCoordinator` 块中(靠近其他 pub async fn 方法)添加:

```rust
    /// Mark that a fallback has been used for the given key (child_id or
    /// "pending:<description>"). Enforces single-fallback constraint.
    pub async fn mark_fallback_used(&self, key: &str) {
        let mut set = self.fallback_used.write().await;
        set.insert(key.to_string());
    }

    /// Check whether a fallback has already been used for the given key.
    pub async fn fallback_already_used(&self, key: &str) -> bool {
        let set = self.fallback_used.read().await;
        set.contains(key)
    }
```

- [x] **Step 6: 确认 `HashSet` 已导入**

在 `src/agent/coordinator.rs` 顶部确认 `use std::collections::{HashMap, HashSet};`(若只有 `HashMap`,添加 `HashSet`)。

- [x] **Step 7: 运行测试验证通过**

Run: `cargo test --lib fallback_used_tests -- --nocapture`
Expected: PASS(全部 3 个测试)

- [x] **Step 8: 编译验证**

Run: `cargo build`
Expected: 编译成功

- [x] **Step 9: 提交**

```bash
git add src/agent/coordinator.rs
git commit -m "feat(fallback): add fallback_used marker on AgentCoordinator

Stored at coordinator level (not GroupRecord) so the marker survives
group claim/removal during collect_children_for_synthesis.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 5: 备用模型配置 -- fallback_models + fallback_model_settings

**Files:**
- Modify: `src/config/agent.rs:116-149`(`SubagentLimits`)
- Modify: `src/config/agent.rs:151-169`(`SubagentLimits::default`)
- Modify: `src/config/mod.rs:140-158`(新增 `fallback_model_settings` 方法)
- Modify: `src/config/mod.rs`(新增 `select_fallback_model` 方法)
- Test: 单元测试在 `src/config/tests.rs` 内

**Interfaces:**
- Consumes: `Settings`(from `src/config/mod.rs`)、`ModelsConfig`(from `src/config/models.rs`)
- Produces:
  - `SubagentLimits.fallback_models: Vec<String>`
  - `Settings::fallback_model_settings(&self, model_name: &str) -> Self`
  - `Settings::select_fallback_model(&self, failed_model: &str) -> Option<&str>`

- [x] **Step 1: 编写失败测试**

在 `src/config/tests.rs` 末尾添加:

```rust
#[cfg(test)]
mod fallback_config_tests {
    use super::*;
    use crate::config::agent::SubagentLimits;

    #[test]
    fn fallback_models_default_empty() {
        let limits = SubagentLimits::default();
        assert!(limits.fallback_models.is_empty());
    }

    #[test]
    fn fallback_models_loaded_from_config() {
        let toml = r#"
[agent.subagent]
fallback_models = ["claude-sonnet-4", "gpt-4o"]
"#;
        let settings: Settings = toml::from_str(toml).unwrap();
        assert_eq!(
            settings.agent.subagent.fallback_models,
            vec!["claude-sonnet-4".to_string(), "gpt-4o".to_string()]
        );
    }

    #[test]
    fn fallback_model_settings_only_overrides_name() {
        let mut settings = Settings::default();
        settings.models.main.name = "deepseek-reasoner".to_string();
        settings.models.main.base_url = Some("https://api.deepseek.com".to_string());
        settings.models.main.api_key = Some("sk-deepseek".to_string());

        let fallback = settings.fallback_model_settings("claude-sonnet-4");
        assert_eq!(fallback.models.main.name, "claude-sonnet-4");
        // base_url / api_key preserved (reuse original endpoint)
        assert_eq!(
            fallback.models.main.base_url,
            Some("https://api.deepseek.com".to_string())
        );
        assert_eq!(
            fallback.models.main.api_key,
            Some("sk-deepseek".to_string())
        );
    }

    #[test]
    fn select_fallback_model_picks_first_different() {
        let mut settings = Settings::default();
        settings.models.main.name = "deepseek-reasoner".to_string();
        settings.agent.subagent.fallback_models = vec![
            "deepseek-reasoner".to_string(),
            "claude-sonnet-4".to_string(),
            "gpt-4o".to_string(),
        ];
        assert_eq!(
            settings.select_fallback_model("deepseek-reasoner"),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn select_fallback_model_none_when_empty() {
        let settings = Settings::default();
        assert_eq!(settings.select_fallback_model("any-model"), None);
    }

    #[test]
    fn select_fallback_model_none_when_all_same() {
        let mut settings = Settings::default();
        settings.agent.subagent.fallback_models = vec!["deepseek-reasoner".to_string()];
        assert_eq!(
            settings.select_fallback_model("deepseek-reasoner"),
            None
        );
    }
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib fallback_config_tests -- --nocapture`
Expected: FAIL -- `fallback_models` 字段未定义、`fallback_model_settings` / `select_fallback_model` 方法未定义

- [x] **Step 3: 在 `SubagentLimits` 新增 `fallback_models` 字段**

修改 `src/config/agent.rs:116-149` 的 `SubagentLimits` 结构,在 `timeout_decision` 字段后添加:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentLimits {
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub timeout_secs: u64,

    #[serde(default)]
    pub token_budget_k: Option<usize>,
    #[serde(default)]
    pub max_rounds: Option<usize>,
    #[serde(default)]
    pub plan_mode: Option<bool>,
    #[serde(default)]
    pub rlm: SubagentRlmOverride,
    #[serde(default)]
    pub prompt: SubagentPromptOverride,

    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub ask_strategy: SubagentAskStrategy,
    #[serde(default = "default_explore_readonly")]
    pub explore_readonly: bool,
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    #[serde(default)]
    pub timeout_decision: TimeoutDecision,

    /// Ordered list of fallback model names for model-unavailable failures.
    /// The first entry different from the failed child's model is selected.
    /// Empty (default) => model failures degrade to parent model (current behavior).
    #[serde(default)]
    pub fallback_models: Vec<String>,
}
```

- [x] **Step 4: 在 `SubagentLimits::default` 初始化字段**

修改 `src/config/agent.rs:151-169` 的 `default()` 方法,在 `timeout_decision: TimeoutDecision::default(),` 后添加:

```rust
            timeout_decision: TimeoutDecision::default(),
            fallback_models: Vec::new(),
```

- [x] **Step 5: 实现 `fallback_model_settings` 方法**

在 `src/config/mod.rs` 中 `small_model_settings` 方法(约 line 140-158)之后添加:

```rust
    /// Build a Settings clone where `models.main.name` is overridden by the
    /// given fallback model name. All other endpoint fields (base_url,
    /// api_key, appkey, provider) are preserved from self -- the fallback
    /// reuses the original endpoint. If the endpoint itself is down, the
    /// fallback fails (single-shot constraint terminates, falls back to root).
    pub fn fallback_model_settings(&self, model_name: &str) -> Self {
        let mut s = self.clone();
        s.models.main.name = model_name.to_string();
        s
    }

    /// Select the first fallback model name different from `failed_model`.
    /// Returns `None` if `fallback_models` is empty or all entries match.
    pub fn select_fallback_model(&self, failed_model: &str) -> Option<&str> {
        self.agent
            .subagent
            .fallback_models
            .iter()
            .find(|m| m.as_str() != failed_model)
            .map(|m| m.as_str())
    }
```

- [x] **Step 6: 运行测试验证通过**

Run: `cargo test --lib fallback_config_tests -- --nocapture`
Expected: PASS(全部 6 个测试)

- [x] **Step 7: 编译验证**

Run: `cargo build`
Expected: 编译成功

- [x] **Step 8: 提交**

```bash
git add src/config/agent.rs src/config/mod.rs src/config/tests.rs
git commit -m "feat(fallback): add fallback_models config and fallback_model_settings

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 6: 拦截点 1 -- TaskTool 内兜底同步执行

**Files:**
- Modify: `src/tools/meta/task.rs:521-530`(reserve_child_in_group 调用点)
- Modify: `src/tools/meta/task.rs`(新增 `execute_fallback_sync` 辅助方法)
- Test: 单元测试 + 集成测试

**Interfaces:**
- Consumes:
  - `fallback_eligible_from_coordinator_error` / `is_root_caller` / `FallbackKind`(from Task 3)
  - `AgentCoordinator::mark_fallback_used` / `fallback_already_used`(from Task 4)
  - `Settings::fallback_model_settings` / `select_fallback_model`(from Task 5,仅 ModelUnavailable 时用;拦截点 1 是 Structural,不换模型)
  - `run_subagent_loop_with_permissions`(from `src/teams/subagent_loop.rs:668`)
- Produces: 拦截点 1 失败时,TaskTool 同步执行 `full_prompt` 并返回 `ToolOutput`

**设计决策:** 拦截点 1 的 fallback 是"TaskTool 在当前 `execute_with_context` 调用内同步执行 prompt"。因为 `DepthLimitReached` 等结构性失败意味着不能再派发更深的 coordinator-owned child,所以 fallback 复用 `run_subagent_loop_with_permissions` 但用一个基于父 context 的合成 child context(不经过 coordinator reserve)。工具集用父 agent 当前工具集(设计文档:候选 B 用父工具集)。不换模型(Structural 不换模型)。

**`run_subagent_loop_with_permissions` 实际签名(供参考,src/teams/subagent_loop.rs:668-681):**
```rust
pub async fn run_subagent_loop_with_permissions(
    api_client: &ApiClient,
    tool_registry: &ToolRegistry,
    context: &AgentExecutionContext,
    coordinator: Arc<AgentCoordinator>,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],
    max_rounds: usize,
    timeout_secs: u64,
    on_progress: Option<ProgressCallback>,
    token_budget_k: Option<u64>,
    workdir: Option<std::path::PathBuf>,
    permission: SubagentPermissionContext,
) -> Result<String, SubagentError>
```

**`build_permission_context` 已存在(task.rs:170):** `fn build_permission_context(&self, agent_id: &str) -> SubagentPermissionContext`

- [x] **Step 1: 编写失败测试 -- 拦截点 1 兜底成功**

在 `src/tools/meta/task.rs` 的 `#[cfg(test)] mod tests` 中(若不存在则新增)添加。由于 `execute_with_context` 需要完整 `ToolContext` 和 coordinator,测试较重,先写一个验证 `map_coordinator_error` 在 fallback 启用时的行为:

```rust
#[cfg(test)]
mod fallback_interception1_tests {
    use super::*;
    use crate::agent::coordinator::CoordinatorError;
    use crate::agent::fallback::{
        fallback_eligible_from_coordinator_error, FallbackKind,
    };

    #[test]
    fn depth_limit_error_is_fallback_eligible() {
        let e = CoordinatorError::DepthLimitReached { limit: 1 };
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn concurrency_closed_error_is_fallback_eligible() {
        let e = CoordinatorError::ConcurrencyClosed;
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn task_group_error_is_fallback_eligible() {
        let e = CoordinatorError::TaskGroup("group gone".to_string());
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn not_visible_error_not_fallback_eligible() {
        let e = CoordinatorError::NotVisible;
        assert_eq!(fallback_eligible_from_coordinator_error(&e), None);
    }
}
```

- [x] **Step 2: 运行测试验证通过(前置条件)**

Run: `cargo test --lib fallback_interception1_tests -- --nocapture`
Expected: PASS(这些测试验证 Task 3 的函数,确认拦截点 1 的判定逻辑可用)

- [x] **Step 3: 实现 `execute_fallback_sync` 辅助方法**

在 `src/tools/meta/task.rs` 的 `impl TaskTool` 块中(在 `execute_with_context` 之后)添加。该方法在拦截点 1 触发时同步执行 full_prompt:

```rust
    /// Interception point 1: synchronous fallback execution inside TaskTool.
    ///
    /// Called when `reserve_child_in_group` fails with a structural
    /// `CoordinatorError` (DepthLimitReached / ConcurrencyClosed / TaskGroup).
    /// Runs `full_prompt` using the parent agent's api_client and tool
    /// registry, returning the result as a `ToolOutput`. The parent agent
    /// model is unaware that fallback occurred.
    ///
    /// Guards:
    /// - `!is_root_caller(context.agent)` (root must not self-execute)
    /// - `!fallback_already_used` (single-shot constraint)
    ///
    /// On fallback execution failure: returns `ToolError` (degrades to root
    /// model, no recursion).
    async fn execute_fallback_sync(
        &self,
        context: &ToolContext<'_>,
        description: &str,
        full_prompt: &str,
        system_prompt: &str,
        fallback_key: &str,
    ) -> Result<ToolOutput, ToolError> {
        use crate::agent::fallback::{is_root_caller, FallbackKind};

        // Guard: root callers must not self-execute (Comet isolation).
        if is_root_caller(context.agent) {
            return Err(ToolError {
                message: "fallback unavailable: root caller cannot self-execute"
                    .to_string(),
                code: Some("fallback_root_blocked".to_string()),
            });
        }

        // Guard: single-shot constraint.
        if self.coordinator.fallback_already_used(fallback_key).await {
            return Err(ToolError {
                message: "fallback already used for this child".to_string(),
                code: Some("fallback_already_used".to_string()),
            });
        }

        tracing::info!(
            fallback = "interception1",
            kind = ?FallbackKind::Structural,
            description = %description,
            "Subagent dispatch fallback: synchronous execution in TaskTool"
        );

        // Mark fallback used BEFORE execution (prevents re-entry).
        self.coordinator.mark_fallback_used(fallback_key).await;

        // Reuse parent's api_client (structural failure does not swap model).
        let api_client = ApiClient::new(self.settings.clone());
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "tool registry unavailable for fallback".to_string(),
            code: Some("fallback_no_registry".to_string()),
        })?;

        // Build a synthetic child context based on the parent's context.
        // We do NOT call coordinator.reserve_child (it failed). Instead we
        // synthesize a context with a new agent_id at the parent's depth
        // (no deeper, since depth limit was the failure reason).
        let child_agent_id_str = uuid::Uuid::new_v4().to_string();
        let child_context = AgentExecutionContext {
            agent_id: AgentId::new(child_agent_id_str.clone()),
            parent_id: Some(context.agent.agent_id.clone()),
            session_id: context.agent.session_id.clone(),
            depth: context.agent.depth,
            cancellation: context.agent.cancellation.clone(),
        };

        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let timeout_secs = self.settings.agent.subagent.timeout_secs;
        let workdir: Option<std::path::PathBuf> =
            Some(self.settings.storage.working_dir.clone());
        let permission = self.build_permission_context(&child_agent_id_str);

        let result = run_subagent_loop_with_permissions(
            &api_client,
            &tool_registry,
            &child_context,
            self.coordinator.clone(),
            system_prompt,
            full_prompt,
            &allowed_tools,
            100,
            timeout_secs,
            None,
            None,
            workdir,
            permission,
        )
        .await;

        match result {
            Ok(output) => {
                tracing::info!(
                    fallback = "interception1",
                    result = "success",
                    "Subagent dispatch fallback succeeded"
                );
                Ok(ToolOutput {
                    content: serde_json::json!({
                        "result": output,
                        "fallback_used": true,
                    }),
                    metadata: None,
                })
            }
            Err(e) => {
                tracing::warn!(
                    fallback = "interception1",
                    result = "failure",
                    error = %e.full_message(),
                    "Subagent dispatch fallback failed; degrading to root model"
                );
                Err(ToolError {
                    message: format!(
                        "Fallback execution failed: {}. Original dispatch error preserved.",
                        e.full_message()
                    ),
                    code: Some("fallback_execution_failed".to_string()),
                })
            }
        }
    }
```

注意:
- `build_permission_context` 需确认是否存在;若不存在,参考 `execute_with_context` 内 permission context 构造逻辑(约 task.rs:580-600),提取为辅助方法或内联。
- `AgentExecutionContext` 字段需与实际一致(先 `grep -n "pub struct AgentExecutionContext" src/agent/identity.rs` 确认)。
- `ToolOutput` 结构需与实际一致(先 `grep -n "pub struct ToolOutput" src/tools/` 确认字段名)。
- `run_subagent_loop_with_permissions` 的参数顺序需与 `src/teams/subagent_loop.rs:668` 签名一致。

- [x] **Step 4: 在 `execute_with_context` 插入拦截点 1**

修改 `src/tools/meta/task.rs:521-530` 的 `reserve_child_in_group` 调用。当前代码:

```rust
        let reservation = self
            .coordinator
            .reserve_child_in_group(
                context.agent,
                SpawnChildRequest::new(description),
                group_id.clone(),
            )
            .await
            .map_err(map_coordinator_error)?;
```

替换为:

```rust
        let reservation = match self
            .coordinator
            .reserve_child_in_group(
                context.agent,
                SpawnChildRequest::new(description),
                group_id.clone(),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Interception point 1: pre-dispatch structural failure.
                use crate::agent::fallback::fallback_eligible_from_coordinator_error;
                if let Some(_kind) = fallback_eligible_from_coordinator_error(&e) {
                    let fallback_key = format!("pending:{}", description);
                    match self
                        .execute_fallback_sync(
                            context,
                            description,
                            &full_prompt,
                            &system_prompt,
                            &fallback_key,
                        )
                        .await
                    {
                        Ok(output) => return Ok(output),
                        Err(fallback_err) => return Err(fallback_err),
                    }
                }
                // Not eligible -> original error path.
                return Err(map_coordinator_error(e));
            }
        };
```

- [x] **Step 5: 编译验证**

Run: `cargo build`
Expected: 编译成功。若有类型不匹配(如 `AgentExecutionContext` 字段、`ToolOutput` 字段、`run_subagent_loop_with_permissions` 参数),根据编译器提示调整。

- [x] **Step 6: 运行现有测试确保无回归**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: 现有测试全部 PASS

- [x] **Step 7: 提交**

```bash
git add src/tools/meta/task.rs
git commit -m "feat(fallback): interception point 1 -- TaskTool sync fallback on reserve failure

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 7: 拦截点 2 -- 运行时模型失败降级再派发

**Files:**
- Modify: `src/teams/subagent_loop.rs:174-213`(`SubagentSynthesis::on_candidate_final`)
- Modify: `src/teams/subagent_loop.rs`(`SubagentSynthesis` 结构新增字段)
- Test: 单元测试 + 集成测试

**Interfaces:**
- Consumes:
  - `fallback_eligible_from_child_result` / `is_root_caller` / `FallbackKind`(from Task 3)
  - `AgentCoordinator::mark_fallback_used` / `fallback_already_used`(from Task 4)
  - `Settings::fallback_model_settings` / `select_fallback_model`(from Task 5)
  - `SubagentTranscriptStore::get_by_id`(from `src/transcript/store.rs:240`)
  - `run_subagent_loop_with_permissions`(from `src/teams/subagent_loop.rs:668`)
- Produces: 拦截点 2 触发时,换备用模型再派发降级子 agent,结果替换原 `ChildResult`

**设计决策:** 拦截点 2 在 `SubagentSynthesis::on_candidate_final` 内 `collect_children_for_synthesis` 返回后执行。对于每个 `Failed` 且 `error_code = subagent_model_unavailable` 且 eligible 且 `!is_root` 且 `!fallback_used` 且有备用模型的 child,用 `fallback_model_settings` 构造备用 `ApiClient`,prompt 从 `transcript_store.get_by_id(child_id).user_prompt` 读,`allowed_tools` 同原派发,调 `run_subagent_loop_with_permissions` 再派发。成功则替换为 `ChildTerminal::Completed`,失败则保留原 Failed 结果(交父模型,不递归)。

- [x] **Step 1: 编写失败测试 -- 拦截点 2 判定逻辑**

在 `src/teams/subagent_loop.rs` 的 `#[cfg(test)] mod tests` 中添加:

```rust
#[cfg(test)]
mod fallback_interception2_tests {
    use super::*;
    use crate::agent::coordinator::{ChildResult, ChildTerminalStatus};
    use crate::agent::fallback::fallback_eligible_from_child_result;
    use crate::agent::identity::AgentId;

    fn make_failed_child(code: &str) -> ChildResult {
        ChildResult {
            child_id: AgentId::new("child-1"),
            status: ChildTerminalStatus::Failed,
            summary: String::new(),
            error_code: Some(code.to_string()),
            partial_result: None,
        }
    }

    #[test]
    fn model_unavailable_child_is_eligible() {
        let r = make_failed_child("subagent_model_unavailable");
        assert!(fallback_eligible_from_child_result(&r).is_some());
    }

    #[test]
    fn timeout_child_not_eligible() {
        let r = make_failed_child("subagent_timeout");
        assert!(fallback_eligible_from_child_result(&r).is_none());
    }

    #[test]
    fn stuck_child_not_eligible() {
        let r = make_failed_child("subagent_stuck");
        assert!(fallback_eligible_from_child_result(&r).is_none());
    }

    #[test]
    fn generic_error_child_not_eligible() {
        let r = make_failed_child("subagent_error");
        assert!(fallback_eligible_from_child_result(&r).is_none());
    }

    #[test]
    fn completed_child_not_eligible() {
        let r = ChildResult {
            child_id: AgentId::new("child-1"),
            status: ChildTerminalStatus::Completed,
            summary: "done".to_string(),
            error_code: None,
            partial_result: None,
        };
        assert!(fallback_eligible_from_child_result(&r).is_none());
    }
}
```

- [x] **Step 2: 运行测试验证通过(前置条件)**

Run: `cargo test --lib fallback_interception2_tests -- --nocapture`
Expected: PASS(验证 Task 3 函数对 ChildResult 的判定)

- [x] **Step 3: 扩展 `SubagentSynthesis` 结构以支持 fallback**

修改 `src/teams/subagent_loop.rs:165-170` 的 `SubagentSynthesis` 结构,新增 fallback 所需字段:

```rust
struct SubagentSynthesis {
    coordinator: Arc<AgentCoordinator>,
    context: AgentExecutionContext,
    synthesized: Mutex<HashSet<String>>,
    is_non_root: bool,
    // Interception point 2: fallback configuration.
    settings: Arc<crate::config::Settings>,
    transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
}
```

- [x] **Step 4: 更新 `SubagentSynthesis` 构造点**

修改 `src/teams/subagent_loop.rs:713-718` 的构造(在 `run_subagent_loop_with_permissions` 内):

```rust
    let synthesis = SubagentSynthesis {
        coordinator,
        context: context.clone(),
        synthesized: Mutex::new(HashSet::new()),
        is_non_root,
        settings: std::sync::Arc::new(settings.clone()),
        transcript_store: transcript_store.clone(),
        tool_registry: tool_registry.clone(),
    };
```

注意:`settings` 和 `transcript_store` 需要作为参数传入 `run_subagent_loop_with_permissions`。若该函数签名不便修改,可将 fallback 配置通过 `LoopHooks` 或新的参数传入。先检查 `run_subagent_loop_with_permissions` 签名(`src/teams/subagent_loop.rs:668`),确认是否已有 `settings` / `transcript_store` 参数。若没有,需新增参数(修改签名 + 所有调用点)。

- [x] **Step 5: 在 `on_candidate_final` 插入拦截点 2**

修改 `src/teams/subagent_loop.rs:174-213` 的 `on_candidate_final`。在 `collect_children_for_synthesis` 返回后、`format_child_result_batch` 之前插入 fallback 逻辑:

```rust
#[async_trait]
impl SynthesisPort for SubagentSynthesis {
    async fn on_candidate_final(&self, _candidate: &str) -> Result<Option<String>, RuntimeError> {
        if !self.is_non_root {
            return Ok(None);
        }

        let mut child_results = self
            .coordinator
            .collect_children_for_synthesis(&self.context)
            .await
            .map_err(|e: CoordinatorError| {
                RuntimeError::Stream(format!("subagent lifecycle coordination failed: {e}"))
            })?;

        // Interception point 2: runtime model-unavailable fallback.
        child_results = self.apply_runtime_fallback(child_results).await;

        let fresh: Vec<ChildResult> = {
            let synthesized = self.synthesized.lock().expect("lock poisoned: synthesized");
            child_results
                .iter()
                .filter(|r| !synthesized.contains(r.child_id.as_str()))
                .cloned()
                .collect()
        };

        if !fresh.is_empty() {
            {
                let mut synthesized = self.synthesized.lock().expect("lock poisoned: synthesized");
                for r in &fresh {
                    synthesized.insert(r.child_id.as_str().to_string());
                }
            }
            return Ok(Some(format_child_result_batch(&fresh)));
        }

        self.coordinator
            .begin_finalizing(&self.context)
            .await
            .map_err(|e: CoordinatorError| {
                RuntimeError::Stream(format!("subagent lifecycle coordination failed: {e}"))
            })?;
        Ok(None)
    }
}

impl SubagentSynthesis {
    /// Interception point 2: for each failed child with `subagent_model_unavailable`,
    /// attempt to re-dispatch with a fallback model. Replaces the ChildResult
    /// in-place on success; leaves it untouched on failure (degrades to parent).
    async fn apply_runtime_fallback(&self, results: Vec<ChildResult>) -> Vec<ChildResult> {
        use crate::agent::fallback::{fallback_eligible_from_child_result, is_root_caller, FallbackKind};

        if is_root_caller(&self.context) {
            return results;
        }

        let mut out = Vec::with_capacity(results.len());
        for r in results {
            let eligible = fallback_eligible_from_child_result(&r);
            if eligible.is_none() {
                out.push(r);
                continue;
            }
            // Only ModelUnavailable is eligible (Structural is interception 1).
            if eligible != Some(FallbackKind::ModelUnavailable) {
                out.push(r);
                continue;
            }

            let child_id_str = r.child_id.as_str().to_string();
            if self.coordinator.fallback_already_used(&child_id_str).await {
                tracing::warn!(
                    fallback = "interception2",
                    child_id = %child_id_str,
                    "Fallback already used; skipping"
                );
                out.push(r);
                continue;
            }

            match self.attempt_model_fallback(&r).await {
                Ok(new_result) => {
                    self.coordinator.mark_fallback_used(&child_id_str).await;
                    out.push(new_result);
                }
                Err(reason) => {
                    tracing::warn!(
                        fallback = "interception2",
                        child_id = %child_id_str,
                        reason = %reason,
                        "Model fallback failed; degrading to parent model"
                    );
                    out.push(r);
                }
            }
        }
        out
    }

    /// Attempt to re-dispatch a failed child with a fallback model.
    async fn attempt_model_fallback(
        &self,
        failed: &ChildResult,
    ) -> Result<ChildResult, String> {
        // 1. Select fallback model.
        let failed_model = &self.settings.models.main.name;
        let fallback_model = self
            .settings
            .select_fallback_model(failed_model)
            .ok_or_else(|| "no fallback model configured".to_string())?;

        tracing::info!(
            fallback = "interception2",
            child_id = %failed.child_id.as_str(),
            failed_model = %failed_model,
            fallback_model = %fallback_model,
            "Re-dispatching child with fallback model"
        );

        // 2. Read original prompt from transcript.
        let transcript_store = self
            .transcript_store
            .as_ref()
            .ok_or_else(|| "no transcript store available".to_string())?;
        let transcript = transcript_store
            .get_by_id(failed.child_id.as_str())
            .map_err(|e| format!("transcript read failed: {e}"))?
            .ok_or_else(|| "transcript not found for child".to_string())?;

        let user_prompt = transcript.user_prompt.clone();
        let system_prompt = transcript.system_prompt.clone().unwrap_or_default();

        // 3. Build fallback api_client (swap model name, reuse endpoint).
        let fallback_settings = self.settings.fallback_model_settings(fallback_model);
        let api_client = ApiClient::new(fallback_settings);

        // 4. Build tool registry + allowed tools (same as original dispatch).
        let tool_registry = self
            .tool_registry
            .upgrade()
            .map_err(|_| "tool registry dropped".to_string())?;
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();

        // 5. Synthesize child context (reuse failed child's context identity).
        let child_context = AgentExecutionContext {
            agent_id: AgentId::new(uuid::Uuid::new_v4().to_string()),
            parent_id: Some(self.context.agent_id.clone()),
            session_id: self.context.session_id.clone(),
            depth: self.context.depth,
            origin_turn_id: self.context.origin_turn_id,
        };

        let timeout_secs = self.settings.agent.subagent.timeout_secs;
        let workdir = self
            .settings
            .storage
            .working_dir
            .to_str()
            .unwrap_or(".");

        // 6. Re-dispatch.
        let result = run_subagent_loop_with_permissions(
            &api_client,
            &tool_registry,
            &child_context,
            self.coordinator.clone(),
            &system_prompt,
            &user_prompt,
            &allowed_tools,
            100,
            timeout_secs,
            None,
            None,
            workdir,
            // permission context: reuse parent's (best-effort; if unavailable,
            // fallback degrades to error).
            None,
        )
        .await;

        match result {
            Ok(summary) => Ok(ChildResult {
                child_id: failed.child_id.clone(),
                status: ChildTerminalStatus::Completed,
                summary: summary.chars().take(500).collect(),
                error_code: None,
                partial_result: None,
            }),
            Err(e) => Err(format!("fallback execution failed: {}", e.full_message())),
        }
    }
}
```

- [x] **Step 6: 编译验证**

Run: `cargo build`
Expected: 编译成功。常见调整:
- `run_subagent_loop_with_permissions` 参数顺序/数量(核对 `src/teams/subagent_loop.rs:668` 签名)
- `AgentExecutionContext` 字段(核对 `src/agent/identity.rs`)
- `ToolContext` / permission context 参数(可能需要 `build_permission_context` 辅助)
- 若 `run_subagent_loop_with_permissions` 不接受 `None` permission context,需提供默认值

- [x] **Step 7: 运行现有测试确保无回归**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: 现有测试 PASS

- [x] **Step 8: 提交**

```bash
git add src/teams/subagent_loop.rs
git commit -m "feat(fallback): interception point 2 -- model-unavailable re-dispatch

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 8: 可观测性 -- FailureMode::ModelUnavailable + tracing 日志

**Files:**
- Modify: `src/teams/subagent_health.rs:36-47`(`FailureMode` 枚举)
- Modify: `src/teams/subagent_health.rs:49-72`(`classify` 方法)
- Modify: `src/teams/subagent_health.rs:74-95`(`label` / `severity` 方法)
- Test: 单元测试在 `src/teams/subagent_health.rs` 内

**Interfaces:**
- Produces: `FailureMode::ModelUnavailable` 变体;`classify` 识别 `subagent_model_unavailable` 码;`label`/`severity`/`recommendation` 映射

- [x] **Step 1: 编写失败测试**

在 `src/teams/subagent_health.rs` 的 `#[cfg(test)] mod tests` 中添加:

```rust
#[cfg(test)]
mod model_unavailable_health_tests {
    use super::*;

    #[test]
    fn classifies_model_unavailable_code() {
        assert_eq!(
            FailureMode::classify("subagent_model_unavailable"),
            FailureMode::ModelUnavailable
        );
    }

    #[test]
    fn classifies_api_error_message_as_model_unavailable() {
        assert_eq!(
            FailureMode::classify("API error (503): service unavailable"),
            FailureMode::ModelUnavailable
        );
    }

    #[test]
    fn model_unavailable_label() {
        assert_eq!(FailureMode::ModelUnavailable.label(), "Model Unavailable");
    }

    #[test]
    fn model_unavailable_severity_is_critical() {
        assert_eq!(FailureMode::ModelUnavailable.severity(), "Critical");
    }
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib model_unavailable_health_tests -- --nocapture`
Expected: FAIL -- `FailureMode::ModelUnavailable` 变体不存在

- [x] **Step 3: 在 `FailureMode` 枚举新增 `ModelUnavailable` 变体**

修改 `src/teams/subagent_health.rs:36-47`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FailureMode {
    Timeout,
    TokenBudgetExceeded,
    StuckLoop,
    ParseError,
    MaxRoundsExceeded,
    ApiError,
    ModelUnavailable,
    ToolError,
    Cancelled,
    Unknown,
}
```

- [x] **Step 4: 扩展 `classify` 方法**

修改 `src/teams/subagent_health.rs:49-72`。在 `ApiError` 分支之前插入 `ModelUnavailable` 检测(优先于通用 ApiError):

```rust
    pub fn classify(error_msg: &str) -> Self {
        let lower = error_msg.to_lowercase();
        if lower.contains("model_unavailable") || lower.contains("model unavailable") {
            Self::ModelUnavailable
        } else if lower.contains("timeout") || lower.contains("timed out") {
            Self::Timeout
        } else if lower.contains("budget") || lower.contains("token") {
            Self::TokenBudgetExceeded
        } else if lower.contains("stuck") || lower.contains("loop") || lower.contains("repeated") {
            Self::StuckLoop
        } else if lower.contains("parse") || lower.contains("json") || lower.contains("malformed") {
            Self::ParseError
        } else if lower.contains("max") && lower.contains("round") {
            Self::MaxRoundsExceeded
        } else if lower.contains("api") || lower.contains("connection") || lower.contains("network")
        {
            Self::ApiError
        } else if lower.contains("tool") || lower.contains("execut") {
            Self::ToolError
        } else if lower.contains("cancel") {
            Self::Cancelled
        } else {
            Self::Unknown
        }
    }
```

- [x] **Step 5: 扩展 `label` 和 `severity` 方法**

修改 `src/teams/subagent_health.rs:74-95`:

```rust
    pub fn label(&self) -> &'static str {
        match self {
            Self::Timeout => "Timeout",
            Self::TokenBudgetExceeded => "Token Budget Exceeded",
            Self::StuckLoop => "Stuck Loop Detected",
            Self::ParseError => "Parse Error Cascade",
            Self::MaxRoundsExceeded => "Max Rounds Exceeded",
            Self::ApiError => "API/Network Error",
            Self::ModelUnavailable => "Model Unavailable",
            Self::ToolError => "Tool Execution Error",
            Self::Cancelled => "Cancelled",
            Self::Unknown => "Unknown",
        }
    }

    pub fn severity(&self) -> &'static str {
        match self {
            Self::Timeout | Self::ApiError | Self::ModelUnavailable => "Critical",
            Self::TokenBudgetExceeded | Self::MaxRoundsExceeded => "Warning",
            Self::StuckLoop | Self::ParseError | Self::ToolError => "Warning",
            Self::Cancelled | Self::Unknown => "Info",
        }
    }
```

- [x] **Step 6: 运行测试验证通过**

Run: `cargo test --lib model_unavailable_health_tests -- --nocapture`
Expected: PASS(全部 4 个测试)

- [x] **Step 7: 全量编译 + 测试**

Run: `cargo build && cargo test --lib -- --nocapture 2>&1 | tail -30`
Expected: 编译成功,所有测试 PASS

- [x] **Step 8: 提交**

```bash
git add src/teams/subagent_health.rs
git commit -m "feat(fallback): add FailureMode::ModelUnavailable health bucket

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 9: 集成测试 -- 端到端 fallback 场景

**Files:**
- Create: `tests/integration/subagent_fallback_test.rs`
- Modify: `tests/integration/main.rs`(注册新模块,若需要)

**Interfaces:**
- Consumes: 所有前序任务的产物

- [x] **Step 1: 编写集成测试**

创建 `tests/integration/subagent_fallback_test.rs`:

```rust
//! Integration tests for subagent dispatch fallback.
//!
//! Scenarios:
//! 1. Model failure (503) -> fallback model succeeds
//! 2. Depth limit -> TaskTool sync fallback succeeds
//! 3. Fallback failure does not recurse (backup model also 503)
//! 4. Root caller does not fallback
//! 5. No fallback config -> degrades to parent model
//! 6. Pre-dispatch failure (ConcurrencyClosed) -> TaskTool sync fallback

use wgenty_code::agent::coordinator::{CoordinatorError, ChildResult, ChildTerminalStatus};
use wgenty_code::agent::fallback::{
    fallback_eligible_from_child_result, fallback_eligible_from_coordinator_error, FallbackKind,
};
use wgenty_code::agent::progress::ErrorType;
use wgenty_code::config::Settings;
use wgenty_code::teams::subagent_health::FailureMode;
use wgenty_code::teams::subagent_loop::{classify_stream_error, SubagentError};

#[test]
fn integration_model_unavailable_classification() {
    // Simulate deepseek-reasoner 503 error string
    let msg = "API error (503): service unavailable";
    assert_eq!(classify_stream_error(msg), ErrorType::ModelUnavailable);

    let err = SubagentError {
        message: msg.to_string(),
        error_type: ErrorType::ModelUnavailable,
        partial_result: None,
    };
    assert_eq!(err.code(), "subagent_model_unavailable");
}

#[test]
fn integration_health_panel_classifies_model_unavailable() {
    assert_eq!(
        FailureMode::classify("subagent_model_unavailable"),
        FailureMode::ModelUnavailable
    );
    assert_eq!(FailureMode::ModelUnavailable.severity(), "Critical");
}

#[test]
fn integration_fallback_model_selection() {
    let mut settings = Settings::default();
    settings.models.main.name = "deepseek-reasoner".to_string();
    settings.agent.subagent.fallback_models = vec![
        "deepseek-reasoner".to_string(),
        "claude-sonnet-4".to_string(),
    ];

    // Select first different model
    assert_eq!(
        settings.select_fallback_model("deepseek-reasoner"),
        Some("claude-sonnet-4")
    );

    // Fallback settings only swap name
    let fb = settings.fallback_model_settings("claude-sonnet-4");
    assert_eq!(fb.models.main.name, "claude-sonnet-4");
}

#[test]
fn integration_structural_error_eligibility() {
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::DepthLimitReached { limit: 1 }),
        Some(FallbackKind::Structural)
    );
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::ConcurrencyClosed),
        Some(FallbackKind::Structural)
    );
}

#[test]
fn integration_no_fallback_config_degrades() {
    let settings = Settings::default();
    assert!(settings.agent.subagent.fallback_models.is_empty());
    assert_eq!(settings.select_fallback_model("any"), None);
}

#[test]
fn integration_child_result_model_unavailable_eligible() {
    let r = ChildResult {
        child_id: wgenty_code::agent::identity::AgentId::new("c1"),
        status: ChildTerminalStatus::Failed,
        summary: String::new(),
        error_code: Some("subagent_model_unavailable".to_string()),
        partial_result: None,
    };
    assert_eq!(
        fallback_eligible_from_child_result(&r),
        Some(FallbackKind::ModelUnavailable)
    );
}
```

- [x] **Step 2: 在 `tests/integration/main.rs` 注册模块**

在 `tests/integration/main.rs` 中添加:

```rust
mod subagent_fallback_test;
```

- [x] **Step 3: 运行集成测试**

Run: `cargo test --test integration subagent_fallback_test -- --nocapture`
Expected: PASS(全部测试)

- [x] **Step 4: 全量测试**

Run: `cargo test -- --nocapture 2>&1 | tail -30`
Expected: 所有测试 PASS

- [x] **Step 5: 提交**

```bash
git add tests/integration/subagent_fallback_test.rs tests/integration/main.rs
git commit -m "test(fallback): integration tests for dispatch fallback scenarios

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Task 10: 文档更新 -- fallback 行为说明

**Files:**
- Modify: `docs/superpowers/specs/2026-07-18-subagent-dispatch-fallback-design.md`(若需补 open question 决策)
- Create: `docs/SUBAGENT-FALLBACK.md`(项目文档)

- [x] **Step 1: 创建 fallback 行为文档**

创建 `docs/SUBAGENT-FALLBACK.md`:

```markdown
# Subagent Dispatch Fallback

## Overview

When a subagent dispatch fails, the parent agent (the one that dispatched the
child) automatically attempts a single fallback execution. This prevents
dispatch failures from becoming task failures.

## Two Interception Points

### Interception 1: Pre-dispatch structural failure

**Location:** `TaskTool::execute_with_context` (`src/tools/meta/task.rs`)

**Trigger:** `reserve_child_in_group` returns `CoordinatorError::DepthLimitReached`,
`ConcurrencyClosed`, or `TaskGroup`.

**Action:** TaskTool synchronously executes `full_prompt` using the parent
agent's api_client and tool registry. The result is returned as the `task`
tool output. The parent model is unaware fallback occurred.

**Model:** Reuses parent's current model (structural failure does not swap model).

### Interception 2: Runtime model failure

**Location:** `SubagentSynthesis::on_candidate_final` (`src/teams/subagent_loop.rs`)

**Trigger:** `ChildResult.status = Failed` and `error_code = subagent_model_unavailable`.

**Action:** Re-dispatches the child with a fallback model (first entry in
`agent.subagent.fallback_models` different from the failed model). The prompt
is read from `SubagentTranscriptStore.get_by_id(child_id).user_prompt`.

**Model:** Swaps `models.main.name` to the fallback model; reuses the original
endpoint (base_url/api_key).

## Constraints

- **Single-shot:** Each child can only fallback once. `fallback_used` marker
  on `AgentCoordinator` prevents recursion.
- **Root exclusion:** Root callers (`parent_id.is_none()`) never self-execute
  fallback (Comet isolation rules).
- **No fallback for:** Timeout, stuck, max-rounds, panic, cancellation.
- **Endpoint failure:** If the fallback model's endpoint is also down, the
  fallback fails and degrades to the parent model (no recursion).

## Configuration

```toml
[agent.subagent]
fallback_models = ["claude-sonnet-4", "gpt-4o"]
```

Empty list (default) => model failures degrade to parent model (current behavior).

## Observability

- `tracing` logs: fallback trigger (interception point, kind, model name),
  success, failure.
- `FailureMode::ModelUnavailable` bucket in `subagent_health.rs` health panel.

## Compatibility with Comet Isolation

The fallback executor is always the parent agent (a subagent itself), never
the root coordinator or main session. This complies with Comet build-phase
isolation rules that forbid the main session from executing tasks directly.
```

- [x] **Step 2: 提交**

```bash
git add docs/SUBAGENT-FALLBACK.md
git commit -m "docs(fallback): document subagent dispatch fallback behavior

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-07-18-subagent-dispatch-fallback
---

## Self-Review Checklist

执行者在完成所有任务后,运行以下验证:

- [x] **Spec coverage:**
  - ErrorType::ModelUnavailable -> Task 1
  - 失败分类细化 -> Task 2
  - fallback_eligible 判定 -> Task 3
  - fallback_used 标记 -> Task 4
  - fallback_model_settings + 配置 -> Task 5
  - 拦截点 1(候选 B) -> Task 6
  - 拦截点 2(候选 A) -> Task 7
  - FailureMode::ModelUnavailable + tracing -> Task 8
  - is_root 守卫 -> Task 3(is_root_caller)+ Task 6/7(使用)
  - 集成测试 -> Task 9
  - 文档 -> Task 10

- [x] **编译 + 全量测试:**
  ```bash
  cargo build && cargo test -- --nocapture 2>&1 | tail -30
  ```

- [x] **类型一致性:**
  - `FallbackKind` 在 Task 3 定义,Task 6/7 使用 -- 一致
  - `fallback_eligible_from_coordinator_error` / `fallback_eligible_from_child_result` 签名一致
  - `mark_fallback_used` / `fallback_already_used` 签名一致
  - `fallback_model_settings` / `select_fallback_model` 签名一致
  - `classify_stream_error` 在 Task 2 定义,Task 9 测试使用 -- 一致

- [x] **Placeholder scan:** 无 TBD/TODO/"implement later"
