---
change: subagent-chain-tracing
design-doc: openspec/changes/subagent-chain-tracing/design.md
base-ref: 21637d09c45264b696785cdbb3b590e2d5430a5d
---

# Subagent Chain Tracing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add structured failure diagnostics, real-time trace streaming (JSONL file + daemon SSE), and an offline CLI (`wgenty-code subagent list|trace|health`) so subagent execution can be fully reconstructed, observed live, and reviewed offline.

**Architecture:** Extend `ErrorInfo` with structured failure diagnostics (root cause, failed tool-call sequence, failed-round context, retry history); persist them via idempotent `ALTER TABLE` migration on the global transcript DB; emit live trace events through an async buffered `TraceSink` to a project-local JSONL file and a global daemon SSE endpoint; expose a new read-only `Subagent` CLI subcommand reading directly from the transcript store.

**Tech Stack:** Rust 2021 (MSRV 1.75), tokio, rusqlite, axum (daemon, feature-gated), clap, serde. Tests via `cargo test`.

## Global Constraints

- MSRV 1.75；`cargo clippy --all-targets -- -D warnings` 零 warning（CI 强制）
- `cargo fmt --check` 必须通过
- 跨平台编译（linux/macos/windows）；daemon SSE endpoint 必须 `#[cfg(feature = "daemon")]` gate
- SQLite `ALTER TABLE ADD COLUMN` 不支持 `IF NOT EXISTS`，迁移须用 `PRAGMA table_info` 守卫
- 新增 serde 字段必须 `#[serde(default)]` 保证向后兼容
- 敏感参数（api_key/token/secret/password）在持久化/发射前必须脱敏
- 字符串截断必须 char-boundary 安全（不 panic on multi-byte UTF-8）
- transcript db 默认全局 `~/.wgenty-code/subagent_transcripts.db`（`settings.storage.transcript.db_path` 可配）

## File Structure

**新建：**
- `src/teams/trace_sink.rs` — `TraceSink`：async buffered writer（mpsc + spawn task），写 JSONL + 广播 SSE
- `src/teams/failure_diagnostics.rs` — `FailureRootCause`/`ToolCallStep`/`FailedRoundContext`/`RetryAttempt` 类型 + 脱敏 + char-boundary 截断工具
- `src/cli/subagent.rs` — `SubagentCommands` enum + `list`/`trace`/`health` 实现
- `src/daemon/trace_stream.rs` — SSE endpoint（feature-gated）

**修改：**
- `src/agent/progress.rs` — `ErrorInfo` 扩展新字段
- `src/teams/subagent_health.rs` — `FailureMode` 扩展类别 + 发射 `FailureRootCause`
- `src/teams/subagent_loop.rs` — 失败捕获填充诊断 + 重试历史
- `src/teams/mod.rs` — 注册新模块
- `src/transcript/store.rs` — schema 迁移加 4 列 + `SubagentTranscriptHeader` 扩展 + `save`/`get_by_id`/`list_by_session` 适配
- `src/cli/mod.rs` — `Commands::Subagent` 变体 + dispatch
- `src/daemon/routes.rs` — 挂载 trace stream 路由
- `src/tools/meta/subagent_trace.rs` — 渲染新诊断字段
- `src/teams/subagent_trace.rs` — 渲染逻辑扩展
- `src/config/` — 新增 `subagent.trace.*` 配置键
- `WGENTY.md` — CLI 命令表 + 配置表文档

---

## Phase 1: Failure Diagnostics Data Model

### Task 1: Define failure diagnostics types

**Files:**
- Create: `src/teams/failure_diagnostics.rs`
- Modify: `src/teams/mod.rs`（加 `pub mod failure_diagnostics;`）

**Interfaces:**
- Produces: `FailureRootCause`（enum）、`ToolCallStep`、`FailedRoundContext`、`RetryAttempt`、`RetryOutcome`、`redact_params(serde_json::Value) -> serde_json::Value`、`truncate_char_safe(&str, usize) -> String`

- [ ] **Step 1: Write failing test for redaction + truncation**

Create `src/teams/failure_diagnostics.rs` with a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_keys() {
        let params = json!({"api_key": "sk-123", "token": "abc", "query": "hello", "password": "pw"});
        let redacted = redact_params(params);
        assert_eq!(redacted["api_key"], "***REDACTED***");
        assert_eq!(redacted["token"], "***REDACTED***");
        assert_eq!(redacted["password"], "***REDACTED***");
        assert_eq!(redacted["query"], "hello");
    }

    #[test]
    fn truncate_is_char_boundary_safe() {
        let s = "中文测试─字符"; // multi-byte
        let t = truncate_char_safe(s, 4);
        assert!(t.chars().count() <= 4);
        assert!(s.starts_with(&t));
    }

    #[test]
    fn truncate_empty_and_short_unchanged() {
        assert_eq!(truncate_char_safe("", 10), "");
        assert_eq!(truncate_char_safe("abc", 10), "abc");
    }
}
```

Run: `cargo test --lib failure_diagnostics` → Expected: FAIL（函数未定义）

- [ ] **Step 2: Implement types + helpers**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureRootCause {
    TokenBudgetExceeded,
    GuardianRejected { reason: String },
    SandboxFailed,
    ApiError,
    ToolPanic,
    Timeout,
    UserCancelled,
    Unknown,
}

impl Default for FailureRootCause {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCallStep {
    pub tool_name: String,
    pub params_summary: serde_json::Value, // redacted
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FailedRoundContext {
    pub assistant_text: String,   // truncated
    pub final_tool_output: String, // truncated
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcome { Succeeded, Failed }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttempt {
    pub error: String,
    pub root_cause: FailureRootCause,
    pub strategy: String,
    pub outcome: RetryOutcome,
}

const SENSITIVE_KEYS: &[&str] = &["api_key", "token", "secret", "password", "apikey", "access_token", "refresh_token"];

pub fn redact_params(params: serde_json::Value) -> serde_json::Value {
    match params {
        serde_json::Value::Object(mut map) => {
            for (k, v) in map.iter_mut() {
                let lower = k.to_lowercase();
                if SENSITIVE_KEYS.iter().any(|s| lower.contains(s)) {
                    *v = serde_json::Value::String("***REDACTED***".into());
                } else {
                    *v = redact_params(v.clone());
                }
            }
            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(redact_params).collect())
        }
        other => other,
    }
}

pub fn truncate_char_safe(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib failure_diagnostics` → Expected: PASS
Run: `cargo clippy --all-targets -- -D warnings` → Expected: zero warning

- [ ] **Step 4: Commit**

```bash
git add src/teams/failure_diagnostics.rs src/teams/mod.rs
git commit -m "feat(teams): add failure diagnostics types with redaction and char-safe truncation"
```

---

### Task 2: Extend `ErrorInfo` with diagnostics fields

**Files:**
- Modify: `src/agent/progress.rs`（`ErrorInfo` 结构体）

**Interfaces:**
- Consumes: `failure_diagnostics::{FailureRootCause, ToolCallStep, FailedRoundContext, RetryAttempt}` (Task 1)
- Produces: 扩展后的 `ErrorInfo` 供 subagent_loop / transcript store / trace renderer 使用

- [ ] **Step 1: Read current `ErrorInfo` definition**

Run: `grep -n "struct ErrorInfo" -A 20 src/agent/progress.rs` to see current fields (`error_type`/`message`/`last_tool`/`last_params`/`round`/`retryable`).

- [ ] **Step 2: Add new fields with `#[serde(default)]`**

Add to `ErrorInfo` struct（保持现有字段不变）:

```rust
use crate::teams::failure_diagnostics::{
    FailureRootCause, ToolCallStep, FailedRoundContext, RetryAttempt,
};

// 在 ErrorInfo 现有字段后追加：
#[serde(default)]
pub root_cause: FailureRootCause,
#[serde(default)]
pub failed_tool_sequence: Vec<ToolCallStep>,
#[serde(default)]
pub failed_round_context: Option<FailedRoundContext>,
#[serde(default)]
pub retry_history: Vec<RetryAttempt>,
```

确保 `ErrorInfo` derive 了 `Default` 或所有新字段有 default（`FailureRootCause` 有 Default；Vec/Option 默认空）。

- [ ] **Step 3: Verify compile + backward-compat**

Run: `cargo build` → Expected: 编译通过
Run: `cargo test --lib` → Expected: 现有测试不回归（新字段 `#[serde(default)]`，旧 JSON 反序列化兼容）

- [ ] **Step 4: Commit**

```bash
git add src/agent/progress.rs
git commit -m "feat(agent): extend ErrorInfo with structured failure diagnostics fields"
```

---

### Task 3: Extend `FailureMode` and emit `FailureRootCause`

**Files:**
- Modify: `src/teams/subagent_health.rs`（`FailureMode` enum + `classify`）

**Interfaces:**
- Consumes: `FailureRootCause` (Task 1)
- Produces: `FailureMode::to_root_cause(&self) -> FailureRootCause`；扩展 `classify` 支持新类别

- [ ] **Step 1: Read current `FailureMode`**

Run: `grep -n "enum FailureMode\|fn classify" -A 30 src/teams/subagent_health.rs`

- [ ] **Step 2: Write failing test for new categories**

Add test in `subagent_health.rs`:

```rust
#[cfg(test)]
mod root_cause_tests {
    use super::*;
    use crate::teams::failure_diagnostics::FailureRootCause;

    #[test]
    fn classifies_guardian_rejection() {
        let mode = FailureMode::classify("guardian denied tool call: destructive", None);
        assert!(matches!(mode.to_root_cause(), FailureRootCause::GuardianRejected { .. }));
    }

    #[test]
    fn classifies_sandbox_failure() {
        let mode = FailureMode::classify("sandbox seatbelt EPERM", None);
        assert!(matches!(mode.to_root_cause(), FailureRootCause::SandboxFailed));
    }

    #[test]
    fn classifies_tool_panic() {
        let mode = FailureMode::classify("tool panicked at index out of bounds", None);
        assert!(matches!(mode.to_root_cause(), FailureRootCause::ToolPanic));
    }

    #[test]
    fn unknown_fallback() {
        let mode = FailureMode::classify("something weird happened", None);
        assert!(matches!(mode.to_root_cause(), FailureRootCause::Unknown));
    }
}
```

Run: `cargo test --lib root_cause_tests` → Expected: FAIL（新 variant / to_root_cause 未实现）

- [ ] **Step 3: Add variants + `to_root_cause` + extend `classify`**

Add to `FailureMode` enum: `GuardianRejected`, `SandboxFailed`, `ToolPanic`（如尚无）。实现：

```rust
pub fn to_root_cause(&self) -> FailureRootCause {
    use crate::teams::failure_diagnostics::FailureRootCause;
    match self {
        FailureMode::TokenLimit => FailureRootCause::TokenBudgetExceeded,
        FailureMode::Timeout => FailureRootCause::Timeout,
        FailureMode::ApiError => FailureRootCause::ApiError,
        FailureMode::GuardianRejected => FailureRootCause::GuardianRejected { reason: String::new() },
        FailureMode::SandboxFailed => FailureRootCause::SandboxFailed,
        FailureMode::ToolPanic => FailureRootCause::ToolPanic,
        FailureMode::UserCancelled => FailureRootCause::UserCancelled,
        _ => FailureRootCause::Unknown,
    }
}
```

Extend `classify` 字符串匹配增加 `"guardian"`/`"denied"` → GuardianRejected；`"sandbox"`/`"seatbelt"`/`"eperm"` → SandboxFailed；`"panic"`/`"panicked"` → ToolPanic。保留现有匹配优先级。

- [ ] **Step 4: Run tests**

Run: `cargo test --lib root_cause_tests` → Expected: PASS
Run: `cargo test --lib` → Expected: 无回归

- [ ] **Step 5: Commit**

```bash
git add src/teams/subagent_health.rs
git commit -m "feat(teams): extend FailureMode with guardian/sandbox/panic root-cause mapping"
```

---

### Task 4: Populate diagnostics in `subagent_loop` failure path

**Files:**
- Modify: `src/teams/subagent_loop.rs`（失败捕获处）

**Interfaces:**
- Consumes: `ErrorInfo` 扩展（Task 2）、`FailureMode::to_root_cause`（Task 3）、`redact_params`/`truncate_char_safe`（Task 1）
- Produces: 失败时 `SubagentProgress.error_details` 填充完整诊断

- [ ] **Step 1: Locate failure capture site**

Run: `grep -n "error_details\|ErrorInfo\|retryable\|failed" src/teams/subagent_loop.rs | head -30` 找到构造 `ErrorInfo` 的位置。

- [ ] **Step 2: Populate `root_cause` + `failed_tool_sequence` + `failed_round_context`**

在构造 `ErrorInfo` 处，从 `action_log`/`events` 切片失败轮次的工具调用，填充：

```rust
use crate::teams::failure_diagnostics::{
    ToolCallStep, FailedRoundContext, redact_params, truncate_char_safe,
};

// 失败轮次的工具调用序列
let failed_tool_sequence: Vec<ToolCallStep> = events_for_failing_round
    .iter()
    .filter(|e| e.event_type == "action" || e.event_type == "tool_result")
    .map(|e| ToolCallStep {
        tool_name: e.tool_name.clone().unwrap_or_default(),
        params_summary: e.tool_params.as_ref()
            .map(|p| redact_params(p.clone()))
            .unwrap_or(serde_json::Value::Null),
        elapsed_ms: e.elapsed_ms.unwrap_or(0),
    })
    .collect();

let failed_round_context = Some(FailedRoundContext {
    assistant_text: truncate_char_safe(&last_assistant_text, context_char_limit),
    final_tool_output: truncate_char_safe(&last_tool_output, context_char_limit),
});

let root_cause = failure_mode.to_root_cause();
// 如是 GuardianRejected 且有 reason，覆盖：
let root_cause = if let FailureRootCause::GuardianRejected { .. } = &root_cause {
    FailureRootCause::GuardianRejected { reason: denial_reason.clone() }
} else { root_cause };

// 写入 ErrorInfo
error_info.root_cause = root_cause;
error_info.failed_tool_sequence = failed_tool_sequence;
error_info.failed_round_context = failed_round_context;
```

`context_char_limit` 从 settings 读取（默认 2000，Task 25 加配置；此处先用常量 `2000`，Task 25 替换为配置值）。

- [ ] **Step 3: Build + smoke test**

Run: `cargo build` → Expected: 编译通过
Run: `cargo test --lib` → Expected: 无回归

- [ ] **Step 4: Commit**

```bash
git add src/teams/subagent_loop.rs
git commit -m "feat(teams): populate structured failure diagnostics in subagent loop"
```

---

### Task 5: Record retry history

**Files:**
- Modify: `src/teams/subagent_loop.rs`（重试路径）

**Interfaces:**
- Consumes: `RetryAttempt`/`RetryOutcome`（Task 1）
- Produces: `error_info.retry_history` 累积每次重试

- [ ] **Step 1: Locate retry path**

Run: `grep -n "retry\|attempt\|max_rounds\|retryable" src/teams/subagent_loop.rs | head -20`

- [ ] **Step 2: Push `RetryAttempt` per retry**

在每次重试发生时，记录上一次 attempt 的错误：

```rust
use crate::teams::failure_diagnostics::{RetryAttempt, RetryOutcome};

retry_history.push(RetryAttempt {
    error: prev_error_message.clone(),
    root_cause: prev_failure_mode.to_root_cause(),
    strategy: retry_strategy_name.to_string(), // 如 "round_increment" / "budget_retry"
    outcome: if succeeded { RetryOutcome::Succeeded } else { RetryOutcome::Failed },
});
```

最终失败时写入 `error_info.retry_history = retry_history`；成功则 `retry_history` 末项 outcome=Succeeded（如有重试）。

- [ ] **Step 3: Build + test**

Run: `cargo build && cargo test --lib` → Expected: 通过无回归

- [ ] **Step 4: Commit**

```bash
git add src/teams/subagent_loop.rs
git commit -m "feat(teams): record per-attempt retry history in failure diagnostics"
```

---

### Task 6: Integration test for diagnostics end-to-end

**Files:**
- Test: `src/teams/failure_diagnostics.rs`（或 `tests/` 集成测试）

- [ ] **Step 1: Write integration test**

构造一个模拟失败的 `SubagentProgress`，验证 `error_details` 含 root_cause + 非空 failed_tool_sequence + retry_history：

```rust
#[test]
fn failure_diagnostics_round_trip_serde() {
    let info = ErrorInfo {
        root_cause: FailureRootCause::GuardianRejected { reason: "destructive".into() },
        failed_tool_sequence: vec![ToolCallStep { tool_name: "file_write".into(), params_summary: json!({}), elapsed_ms: 12 }],
        failed_round_context: Some(FailedRoundContext { assistant_text: "x".into(), final_tool_output: "y".into() }),
        retry_history: vec![RetryAttempt { error: "e".into(), root_cause: FailureRootCause::Unknown, strategy: "s".into(), outcome: RetryOutcome::Failed }],
        // ... 现有字段 default
        ..Default::default()
    };
    let json = serde_json::to_string(&info).unwrap();
    let back: ErrorInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back.root_cause, info.root_cause);
    assert_eq!(back.failed_tool_sequence.len(), 1);
    assert_eq!(back.retry_history.len(), 1);
}
```

Run: `cargo test --lib failure_diagnostics_round_trip` → Expected: PASS

- [ ] **Step 2: Commit**

```bash
git add src/teams/failure_diagnostics.rs
git commit -m "test(teams): add failure diagnostics serde round-trip integration test"
```

---

## Phase 2: Transcript Storage Adaptation

### Task 7: Idempotent migration adding 4 columns

**Files:**
- Modify: `src/transcript/store.rs`（`run_migrations` 或等价迁移函数）

**Interfaces:**
- Consumes: 无
- Produces: `subagent_transcripts` 新增 `failure_diagnostics TEXT`/`root_cause TEXT`/`retry_history TEXT`/`project_path TEXT`

- [ ] **Step 1: Read current migration**

Run: `grep -n "run_migrations\|execute_batch\|CREATE TABLE" src/transcript/store.rs | head`

- [ ] **Step 2: Write failing test for migration idempotency**

```rust
#[test]
fn migration_adds_columns_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    // 模拟旧库：先建无新列的表
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE subagent_transcripts (id TEXT PRIMARY KEY, session_id TEXT);").unwrap();
    }
    let store = SubagentTranscriptStore::open(&db_path).unwrap(); // 触发迁移
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let cols: Vec<String> = conn.prepare("PRAGMA table_info(subagent_transcripts)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .filter_map(Result::ok).collect();
    assert!(cols.contains(&"failure_diagnostics".into()));
    assert!(cols.contains(&"root_cause".into()));
    assert!(cols.contains(&"retry_history".into()));
    assert!(cols.contains(&"project_path".into()));
    // 二次 open 不报错（幂等）
    let _ = SubagentTranscriptStore::open(&db_path).unwrap();
}
```

Run: `cargo test --lib migration_adds_columns` → Expected: FAIL（迁移未加列）

- [ ] **Step 3: Implement idempotent ALTER**

在迁移函数中，`PRAGMA table_info(subagent_transcripts)` 获取现有列名集合，对每个新列若不存在则 `ALTER TABLE subagent_transcripts ADD COLUMN <name> TEXT`：

```rust
fn ensure_column(conn: &rusqlite::Connection, table: &str, col: &str) -> rusqlite::Result<()> {
    let exists: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({})", table))?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();
    if !exists.contains(&col.to_string()) {
        conn.execute(&format!("ALTER TABLE {} ADD COLUMN {} TEXT", table, col), [])?;
    }
    Ok(())
}

// 在 run_migrations 中（execute_batch 之后）调用：
for col in &["failure_diagnostics", "root_cause", "retry_history", "project_path"] {
    ensure_column(conn, "subagent_transcripts", col)?;
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib migration_adds_columns` → Expected: PASS
Run: `cargo test --lib` → Expected: 无回归

- [ ] **Step 5: Commit**

```bash
git add src/transcript/store.rs
git commit -m "feat(transcript): idempotent migration adds failure diagnostics + project_path columns"
```

---

### Task 8: Extend `SubagentTranscriptHeader` + serialization

**Files:**
- Modify: `src/transcript/store.rs`（`SubagentTranscriptHeader` 结构体 + 行映射）

**Interfaces:**
- Consumes: `FailureRootCause`（Task 1）
- Produces: `SubagentTranscriptHeader` 含 `failure_diagnostics`/`root_cause`/`retry_history`/`project_path` 字段；NULL → `Unknown`/空

- [ ] **Step 1: Add fields to header struct**

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentTranscriptHeader {
    // ... 现有字段
    pub failure_diagnostics: Option<String>,  // JSON text
    pub root_cause: Option<String>,
    pub retry_history: Option<String>,        // JSON text
    pub project_path: Option<String>,
}
```

- [ ] **Step 2: Update row mapping in `get_by_id` / `list_by_session` / `search`**

在 `query_map` 闭包中读取新列（用 `row.get::<_, Option<String>>` 对应列索引）。NULL 自动为 None。提供辅助方法：

```rust
impl SubagentTranscriptHeader {
    pub fn root_cause_enum(&self) -> FailureRootCause {
        self.failure_diagnostics.as_ref()
            .and_then(|s| serde_json::from_str::<FailureDiagnosticsJson>(s).ok())
            .map(|d| d.root_cause)
            .unwrap_or(FailureRootCause::Unknown)
    }
}
```

- [ ] **Step 3: Build + test**

Run: `cargo build && cargo test --lib` → Expected: 通过

- [ ] **Step 4: Commit**

```bash
git add src/transcript/store.rs
git commit -m "feat(transcript): extend SubagentTranscriptHeader with diagnostics fields"
```

---

### Task 9: Write diagnostics on failure in `save`

**Files:**
- Modify: `src/transcript/store.rs`（`save`/insert 函数）

**Interfaces:**
- Consumes: `ErrorInfo` 诊断字段（Task 2）
- Produces: 失败时同一事务写入 `failure_diagnostics`/`root_cause`/`retry_history`/`project_path`

- [ ] **Step 1: Locate `save` INSERT**

Run: `grep -n "INSERT OR REPLACE\|fn save\|fn insert" src/transcript/store.rs`

- [ ] **Step 2: Add new columns to INSERT**

在 `INSERT OR REPLACE INTO subagent_transcripts (...)` 的列名 + 占位符 + 值中追加 4 列。失败时从 `error_details` 序列化：

```rust
let failure_diagnostics_json = error_details.as_ref().map(|e| serde_json::to_string(&FailureDiagnosticsJson {
    root_cause: e.root_cause.clone(),
    failed_tool_sequence: e.failed_tool_sequence.clone(),
    failed_round_context: e.failed_round_context.clone(),
}).unwrap_or_default());
let root_cause_str = error_details.as_ref().map(|e| serde_json::to_string(&e.root_cause).unwrap_or_default());
let retry_history_json = error_details.as_ref().map(|e| serde_json::to_string(&e.retry_history).unwrap_or_default());
let project_path = std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string());
```

成功时这些为 None/NULL。

- [ ] **Step 3: Test diagnostics written on failure**

```rust
#[test]
fn diagnostics_persisted_on_failure() {
    let store = SubagentTranscriptStore::open_memory().unwrap();
    let id = store.save(/* failed transcript with error_details.root_cause=GuardianRejected */).unwrap();
    let got = store.get_by_id(&id).unwrap();
    assert!(got.header.failure_diagnostics.is_some());
    assert!(got.header.root_cause.as_deref().unwrap().contains("guardian_rejected"));
}
```

Run: `cargo test --lib diagnostics_persisted_on_failure` → Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/transcript/store.rs
git commit -m "feat(transcript): persist failure diagnostics in same transaction as header"
```

---

### Task 10: Old-row graceful degradation test

**Files:**
- Test: `src/transcript/store.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn old_rows_without_diagnostics_degrade_to_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("old.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // 建旧 schema 并插入一行（无 diagnostics 列）
        conn.execute_batch("CREATE TABLE subagent_transcripts (id TEXT PRIMARY KEY, session_id TEXT, label TEXT, status TEXT, error_message TEXT);").unwrap();
        conn.execute("INSERT INTO subagent_transcripts (id, session_id, label, status, error_message) VALUES ('old1','s','lbl','failed','boom')", []).unwrap();
    }
    let store = SubagentTranscriptStore::open(&db_path).unwrap(); // 迁移加列
    let got = store.get_by_id("old1").unwrap();
    assert_eq!(got.header.root_cause_enum(), FailureRootCause::Unknown);
    assert!(got.header.retry_history.is_none());
}
```

Run: `cargo test --lib old_rows_without_diagnostics` → Expected: PASS

- [ ] **Step 2: Commit**

```bash
git add src/transcript/store.rs
git commit -m "test(transcript): verify old rows degrade to Unknown root cause"
```

---

## Phase 3: Trace Streaming (JSONL File + Daemon SSE)

### Task 11: `TraceSink` async buffered writer

**Files:**
- Create: `src/teams/trace_sink.rs`
- Modify: `src/teams/mod.rs`（`pub mod trace_sink;`）

**Interfaces:**
- Consumes: `SubagentProgress`（progress.rs）
- Produces: `TraceSink`（mpsc sender + spawn writer task），`TraceSink::spawn(session_id, dir) -> TraceSink`，`TraceSink::emit(&self, event: TraceEvent)`

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn appends_jsonl_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = TraceSink::spawn("sess1", dir.path().to_path_buf(), SinkMode::File).await;
        sink.emit(TraceEvent { node_id: "n1".into(), event_type: "action".into(), data: serde_json::json!({"tool":"grep"}), ts_ms: 1 }).await;
        sink.flush_and_close().await;
        let content = tokio::fs::read_to_string(dir.path().join("sess1.jsonl")).await.unwrap();
        assert!(content.contains("\"tool\":\"grep\""));
        assert!(content.ends_with('\n'));
    }

    #[tokio::test]
    async fn redacts_sensitive_in_emitted_data() {
        let dir = tempfile::tempdir().unwrap();
        let sink = TraceSink::spawn("sess2", dir.path().to_path_buf(), SinkMode::File).await;
        sink.emit(TraceEvent { node_id: "n1".into(), event_type: "action".into(), data: json!({"api_key":"sk"}), ts_ms: 1 }).await;
        sink.flush_and_close().await;
        let content = tokio::fs::read_to_string(dir.path().join("sess2.jsonl")).await.unwrap();
        assert!(content.contains("REDACTED"));
        assert!(!content.contains("\"sk\""));
    }
}
```

Run: `cargo test --lib trace_sink` → Expected: FAIL

- [ ] **Step 2: Implement `TraceSink`**

```rust
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub node_id: String,
    pub event_type: String,
    pub data: serde_json::Value,
    pub ts_ms: u64,
}

#[derive(Clone, Copy)]
pub enum SinkMode { File, Daemon, Both, Off }

pub struct TraceSink {
    tx: mpsc::Sender<TraceEvent>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl TraceSink {
    pub async fn spawn(session_id: &str, dir: PathBuf, mode: SinkMode) -> Self {
        tokio::fs::create_dir_all(&dir).await.ok();
        let path = dir.join(format!("{}.jsonl", session_id));
        let file = OpenOptions::new().create(true).append(true).open(&path).await
            .expect("open trace file");
        // 设置 0600（unix）
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)); }
        let (tx, mut rx) = mpsc::channel::<TraceEvent>(1024);
        let handle = tokio::spawn(async move {
            let mut file = file;
            while let Some(mut ev) = rx.recv().await {
                if matches!(mode, SinkMode::File | SinkMode::Both) {
                    ev.data = redact_params(ev.data);
                    let line = serde_json::to_string(&ev).unwrap_or_default();
                    if file.write_all(line.as_bytes()).await.is_err() { continue; }
                    let _ = file.write_all(b"\n").await;
                }
                // daemon 广播在 Task 12 接入
            }
        });
        Self { tx, handle: Some(handle) }
    }

    pub async fn emit(&self, event: TraceEvent) {
        let _ = self.tx.try_send(event); // 满则丢最旧（持久在 db 不受影响）
    }

    pub async fn flush_and_close(mut self) {
        drop(self.tx);
        if let Some(h) = self.handle.take() { let _ = h.await; }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib trace_sink` → Expected: PASS
Run: `cargo clippy --all-targets -- -D warnings` → Expected: 零 warning

- [ ] **Step 4: Commit**

```bash
git add src/teams/trace_sink.rs src/teams/mod.rs
git commit -m "feat(teams): add async buffered TraceSink writing redacted JSONL"
```

---

### Task 12: Wire `TraceSink` into dispatch path + config

**Files:**
- Modify: `src/teams/subagent_loop.rs` 或 dispatch 入口（注册 `ProgressCallback` 处）
- Modify: config 模块（`subagent.trace.sink`/`dir`）

**Interfaces:**
- Consumes: `TraceSink`（Task 11）、settings
- Produces: progress 事件投递到 `TraceSink`

- [ ] **Step 1: Add config keys**

在 settings 结构加（具体位置参考现有 `storage.transcript` 结构）：

```rust
pub struct SubagentTraceConfig {
    pub sink: String,        // "file"|"daemon"|"both"|"off", default "file"
    pub dir: Option<String>, // default <project>/.wgenty-code/traces
    pub context_char_limit: usize, // default 2000
}
```

默认值 `file` / 项目本地 traces 目录 / 2000。`SinkMode` 从字符串解析。

- [ ] **Step 2: Spawn `TraceSink` at session start, emit in callback**

在 subagent dispatch 入口（创建 `ProgressCallback` 处），按 `subagent.trace.sink` spawn `TraceSink`，callback 内 `emit`：

```rust
let trace_sink = if mode != SinkMode::Off {
    Some(TraceSink::spawn(&session_id, trace_dir.clone(), mode).await)
} else { None };
let sink_clone = trace_sink.clone(); // TraceSink 需 Arc 或共享 sender
let progress_cb: ProgressCallback = Arc::new(move |p: SubagentProgress| {
    if let Some(s) = &sink_clone {
        s.emit(TraceEvent::from_progress(&p)).await; // 注意：同步 callback 内不能 .await，用 try_send 或 channel
    }
    // ... 现有 TUI 更新
});
```

注：`ProgressCallback` 是同步 `Fn`，不能 `.await`。`TraceSink::emit` 改用 `try_send`（同步），或保留 `tx` 的 clone 在 callback 中 `try_send`。调整 Task 11 的 `emit` 为同步 `try_send` 版本。

- [ ] **Step 3: Build + smoke**

Run: `cargo build && cargo test --lib` → Expected: 通过

- [ ] **Step 4: Commit**

```bash
git add src/teams/subagent_loop.rs src/config/
git commit -m "feat(teams): wire TraceSink into subagent dispatch with config-gated sink"
```

---

### Task 13: Bounded broadcast channel for daemon

**Files:**
- Modify: `src/teams/trace_sink.rs`（加 broadcast）

**Interfaces:**
- Produces: `TraceSink` 持有 `tokio::sync::broadcast::Sender<TraceEvent>`（容量 1024，满丢最旧）

- [ ] **Step 1: Add broadcast sender**

```rust
use tokio::sync::broadcast;

// TraceSink 内加：
broadcast_tx: broadcast::Sender<TraceEvent>,
```

spawn 时创建 `broadcast::channel(1024)`。emit 时 `let _ = broadcast_tx.send(event)`（满则丢最旧，send 失败无 receiver 时忽略）。

- [ ] **Step 2: Expose subscribe**

```rust
pub fn subscribe(&self) -> broadcast::Receiver<TraceEvent> {
    self.broadcast_tx.subscribe()
}
```

- [ ] **Step 3: Test backpressure drops oldest, persistence intact**

```rust
#[tokio::test]
async fn broadcast_drops_oldest_when_full() {
    // 容量设小（如 4），发 10 条，subscribe 后只收到最近 4 条；文件含全部 10 条
}
```

Run: `cargo test --lib broadcast_drops_oldest` → Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/teams/trace_sink.rs
git commit -m "feat(teams): add bounded broadcast channel for live subscribers"
```

---

### Task 14: Daemon SSE endpoint

**Files:**
- Create: `src/daemon/trace_stream.rs`
- Modify: `src/daemon/routes.rs`（挂载路由，`#[cfg(feature="daemon")]`）
- Modify: `src/daemon/mod.rs`（`pub mod trace_stream;`）

**Interfaces:**
- Consumes: `TraceSink::subscribe()`（Task 13）、`require_auth`、`SubagentTranscriptStore`（冷启动回放）
- Produces: `GET /api/v1/subagents/trace/stream` SSE

- [ ] **Step 1: Write failing test for auth rejection**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // 复用现有 daemon 测试工具：未认证请求应 401
    #[tokio::test]
    async fn rejects_unauthenticated() {
        let app = build_test_app();
        let resp = app.oneshot(axum::http::Request::builder().uri("/api/v1/subagents/trace/stream").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), 401);
    }
}
```

Run: `cargo test --lib trace_stream` → Expected: FAIL

- [ ] **Step 2: Implement SSE handler**

```rust
use axum::extract::{Query, State};
use axum::response::sse::{Event, Sse};
use futures::stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub async fn trace_stream(
    State(state): State<AppState>,
    Query(params): Query<TraceStreamParams>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    // 冷启动回放：从 transcript_store 读取 session 历史（如 session_id 给定）
    let history = replay_history(&state, &params).await;
    let live = state.trace_sink.subscribe();
    let live_stream = BroadcastStream::new(live).filter_map(|r| r.ok());
    let combined = futures::stream::iter(history).chain(live_stream.map(|ev| ev));
    let s = combined.map(|ev: TraceEvent| {
        Ok(Event::default().data(serde_json::to_string(&ev).unwrap_or_default()))
    });
    Sse::new(s)
}

#[derive(Deserialize)]
pub struct TraceStreamParams {
    pub session_id: Option<String>,
    pub since: Option<u64>,
}
```

在 `routes.rs` 的 protected router 加 `.route("/api/v1/subagents/trace/stream", get(trace_stream))`。`AppState` 持有 `Arc<TraceSink>`（或共享句柄）。

- [ ] **Step 3: Run tests**

Run: `cargo test --lib trace_stream` → Expected: PASS（含 auth rejection + session filter）
Run: `cargo build --features daemon` → Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add src/daemon/trace_stream.rs src/daemon/routes.rs src/daemon/mod.rs
git commit -m "feat(daemon): add SSE trace stream endpoint with auth and cold-start replay"
```

---

### Task 15: Cold-start replay + session filter tests

**Files:**
- Test: `src/daemon/trace_stream.rs`

- [ ] **Step 1: Tests**

```rust
#[tokio::test]
async fn cold_start_replays_history_then_live() { /* 预填 db 历史，连接后先收历史再收 live */ }
#[tokio::test]
async fn session_filter_excludes_other_sessions() { /* ?session_id=A 不收 B 事件 */ }
```

Run: `cargo test --lib trace_stream` → Expected: PASS

- [ ] **Step 2: Commit**

```bash
git add src/daemon/trace_stream.rs
git commit -m "test(daemon): cold-start replay and session filter for SSE trace stream"
```

---

## Phase 4: CLI Subagent Subcommand

### Task 16: `Commands::Subagent` variant + dispatch

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/subagent.rs`
- Modify: `src/cli/mod.rs`（`pub mod subagent;`）

**Interfaces:**
- Produces: `Commands::Subagent { action: SubagentCommands }`、`SubagentCommands::{List, Trace, Health}`

- [ ] **Step 1: Define subcommands**

```rust
use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum SubagentCommands {
    /// List historical subagent runs
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a single subagent trace
    Trace {
        id: String,
        #[arg(long, default_value = "call_tree")]
        format: String,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        output: Option<String>,
    },
    /// Show subagent health statistics
    Health {
        #[arg(long, default_value = "24h")]
        period: String,
    },
}
```

在 `Commands` enum 加：
```rust
/// Inspect subagent traces and health (read-only)
Subagent {
    #[command(subcommand)]
    action: SubagentCommands,
},
```

- [ ] **Step 2: Dispatch in main**

在 CLI dispatch match 加 `Commands::Subagent { action } => crate::cli::subagent::run(action).await,`

- [ ] **Step 3: Build**

Run: `cargo build` → Expected: 编译通过（run 函数在 Task 17-19 实现，先 stub 返回 Ok）

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/subagent.rs
git commit -m "feat(cli): add Subagent subcommand skeleton with list/trace/health"
```

---

### Task 17: Implement `list`

**Files:**
- Modify: `src/cli/subagent.rs`

**Interfaces:**
- Consumes: `SubagentTranscriptStore`（Task 8）

- [ ] **Step 1: Implement**

```rust
pub async fn run(action: SubagentCommands) -> anyhow::Result<()> {
    let store = open_store()?;
    match action {
        SubagentCommands::List { session, status, limit } => list(&store, session, status, limit).await,
        SubagentCommands::Trace { id, format, raw, output } => trace(&store, &id, &format, raw, output).await,
        SubagentCommands::Health { period } => health(&store, &period).await,
    }
}

async fn list(store: &SubagentTranscriptStore, session: Option<String>, status: Option<String>, limit: usize) -> anyhow::Result<()> {
    let headers = if let Some(sid) = session {
        store.list_by_session(&sid)?
    } else {
        store.list_all(limit)?  // 需新增 list_all 方法
    };
    let filtered: Vec<_> = headers.into_iter()
        .filter(|h| status.as_ref().map(|s| h.status.eq_ignore_ascii_case(s)).unwrap_or(true))
        .take(limit)
        .collect();
    // 表格输出：id | label | status | root_cause | duration | started_at
    println!("{:<12} {:<20} {:<10} {:<18} {:>8} {}", "ID","LABEL","STATUS","ROOT_CAUSE","DUR_MS","STARTED");
    for h in filtered {
        let rc = h.root_cause_enum();
        println!("{:<12} {:<20} {:<10} {:<18} {:>8} {}", short(&h.id), truncate(&h.label,20), h.status, format!("{:?}",rc).to_lowercase(), h.duration_ms(), h.started_at);
    }
    Ok(())
}
```

如 `list_all` 不存在，在 store 加：`fn list_all(&self, limit: usize) -> Result<Vec<SubagentTranscriptHeader>>`（`SELECT ... ORDER BY started_at DESC LIMIT ?`）。

- [ ] **Step 2: Test list filter/sort**

```rust
#[test]
fn list_filters_by_status() { /* 填 failed+completed，filter status=failed 只剩 failed */ }
```

Run: `cargo test --lib list_filters` → Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/cli/subagent.rs src/transcript/store.rs
git commit -m "feat(cli): implement subagent list with filters and table output"
```

---

### Task 18: Implement `trace`

**Files:**
- Modify: `src/cli/subagent.rs`
- Modify: `src/teams/subagent_trace.rs`（复用渲染）

**Interfaces:**
- Consumes: `SubagentTraceTool` 渲染逻辑、`get_by_id`

- [ ] **Step 1: Implement**

```rust
async fn trace(store: &SubagentTranscriptStore, id: &str, format: &str, raw: bool, output: Option<String>) -> anyhow::Result<()> {
    let transcript = store.get_by_id(id).context("transcript not found")?;
    if raw {
        let json = transcript.header.failure_diagnostics.as_deref().unwrap_or("{}");
        print_to(output, json)?;
        return Ok(());
    }
    let rendered = render_trace(&transcript, format)?; // 复用 SubagentTraceTool 渲染
    print_to(output, &rendered)?;
    Ok(())
}
```

`render_trace` 调用现有 `subagent_trace` 模块的渲染函数（call_tree/error_timeline/chrome_trace/html）。未知 id 返回非零退出（main 层 `?` 传播 -> non-zero）。

- [ ] **Step 2: Test unknown id exit + format variants**

```rust
#[test]
fn trace_unknown_id_errors() { /* get_by_id 返回 None -> anyhow err */ }
```

Run: `cargo test --lib trace_unknown` → Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/cli/subagent.rs src/teams/subagent_trace.rs
git commit -m "feat(cli): implement subagent trace with format and raw modes"
```

---

### Task 19: Implement `health`

**Files:**
- Modify: `src/cli/subagent.rs`
- Modify: `src/teams/subagent_health.rs`（`compute_from_headers` 支持 period）

**Interfaces:**
- Consumes: `SubagentHealthAnalyzer::compute_from_headers`（已存在）

- [ ] **Step 1: Implement**

```rust
async fn health(store: &SubagentTranscriptStore, period: &str) -> anyhow::Result<()> {
    let period_enum = parse_period(period)?; // Last1h/24h/7d/30d/AllTime
    let headers = store.list_all(10000)?;
    let report = SubagentHealthAnalyzer::compute_from_headers(&headers, period_enum);
    println!("Total runs: {}", report.total_runs);
    println!("Completed:  {}", report.completed);
    println!("Failed:     {}", report.failed);
    println!("Success rate: {:.1}%", report.success_rate * 100.0);
    println!("Failure modes:");
    for (cause, count) in report.failure_modes_by_root_cause() { // 新增：按 FailureRootCause 分组
        println!("  {:<22} {}", format!("{:?}",cause).to_lowercase(), count);
    }
    Ok(())
}
```

如 `failure_modes_by_root_cause` 不存在，在 `SubagentHealthReport` 加（按 header.root_cause_enum() 分组计数）。

- [ ] **Step 2: Test period windows + grouping**

```rust
#[test]
fn health_groups_by_root_cause() { /* 填 guardian+timeout 失败，分组计数正确 */ }
```

Run: `cargo test --lib health_groups` → Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/cli/subagent.rs src/teams/subagent_health.rs
git commit -m "feat(cli): implement subagent health with period and root-cause grouping"
```

---

## Phase 5: Trace Rendering Adaptation

### Task 20: Surface diagnostics in `call_tree`

**Files:**
- Modify: `src/teams/subagent_trace.rs` / `src/tools/meta/subagent_trace.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn call_tree_shows_root_cause_and_sequence() {
    // 构造含 failed_tool_sequence 的 TraceNode，渲染 call_tree，断言含 root_cause + tool 名 + durations
}
```

- [ ] **Step 2: Extend call_tree rendering**

在渲染失败节点时追加 root_cause 行 + failed_tool_sequence 每步 `tool (Nms)`。

- [ ] **Step 3: Run + commit**

```bash
cargo test --lib call_tree_shows_root_cause
git add src/teams/subagent_trace.rs src/tools/meta/subagent_trace.rs
git commit -m "feat(teams): surface root cause and failed tool sequence in call_tree"
```

---

### Task 21: `error_timeline` groups by root cause + retry history

**Files:**
- Modify: `src/teams/subagent_trace.rs`

- [ ] **Step 1: Test + implement**

`error_timeline` 输出按 `FailureRootCause` 分组计数 + 每组 retry_history 摘要。

- [ ] **Step 2: Run + commit**

```bash
cargo test --lib error_timeline_groups
git add src/teams/subagent_trace.rs
git commit -m "feat(teams): group error_timeline by root cause with retry history"
```

---

### Task 22: HTML report diagnostics section

**Files:**
- Modify: `src/teams/subagent_trace.rs`

- [ ] **Step 1: Test + implement**

HTML 报告加 failure-diagnostics 区块（root cause / failed sequence / failed-round context / retry history），保持 self-contained、char-boundary 安全。

- [ ] **Step 2: Run + commit**

```bash
cargo test --lib html_diagnostics_section
git add src/teams/subagent_trace.rs
git commit -m "feat(teams): add failure diagnostics section to HTML trace report"
```

---

### Task 23: Raw diagnostics JSON mode

**Files:**
- Modify: `src/teams/subagent_trace.rs`

- [ ] **Step 1: Test + implement**

`--raw` 模式输出存储的 diagnostics 为 pretty JSON（已在 Task 18 CLI 接入，此处确保渲染层提供 `render_raw`）。

- [ ] **Step 2: Run + commit**

```bash
cargo test --lib raw_diagnostics_json
git add src/teams/subagent_trace.rs
git commit -m "feat(teams): add raw diagnostics JSON rendering mode"
```

---

## Phase 6: Config, Docs & Integration

### Task 24: Config keys + defaults

**Files:**
- Modify: config 模块、settings 默认值

- [ ] **Step 1: Add keys** `subagent.trace.sink`（default "file"）、`subagent.trace.dir`（default null→项目本地）、`subagent.trace.context_char_limit`（default 2000）。在 Task 12 已部分完成，此处补全默认值与序列化测试。

- [ ] **Step 2: Test defaults**

```rust
#[test]
fn trace_config_defaults() {
    let s = Settings::default();
    assert_eq!(s.subagent.trace.sink, "file");
    assert_eq!(s.subagent.trace.context_char_limit, 2000);
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test --lib trace_config_defaults
git add src/config/
git commit -m "feat(config): add subagent.trace config keys with defaults"
```

---

### Task 25: Replace hardcoded `context_char_limit` with config

**Files:**
- Modify: `src/teams/subagent_loop.rs`（Task 4 的常量 `2000` 替换为 settings 值）

- [ ] **Step 1: Read config in loop**

将 Task 4 的 `2000` 常量替换为从 settings 读取的 `context_char_limit`。

- [ ] **Step 2: Build + commit**

```bash
cargo build && cargo test --lib
git add src/teams/subagent_loop.rs
git commit -m "refactor(teams): use configured context_char_limit in failure capture"
```

---

### Task 26: Document CLI + config in WGENTY.md

**Files:**
- Modify: `WGENTY.md`

- [ ] **Step 1: Add to CLI subcommand table**

在 CLI 子命令列表加 `Subagent`：`list | trace <id> | health`。在配置表加 `subagent.trace.sink`/`dir`/`context_char_limit` 三行。

- [ ] **Step 2: Commit**

```bash
git add WGENTY.md
git commit -m "docs: document subagent tracing CLI and config in WGENTY.md"
```

---

### Task 27: Lint + format + full test

**Files:** 无（验证任务）

- [ ] **Step 1: Run**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Expected: fmt 干净、零 warning、全测试通过。失败则加载 systematic-debugging 定位根因（不盲目修补）。

- [ ] **Step 2: Commit fixes（如有）**

```bash
git add -A
git commit -m "chore: fmt/clippy/test fixes for subagent tracing"
```

---

### Task 28: Cross-platform + feature gate verification

**Files:** 无（验证任务）

- [ ] **Step 1: Verify**

```bash
cargo build --no-default-features      # 纯 CLI（无 daemon）应编译，SSE 不参与
cargo build --features daemon          # daemon 开启，SSE 编译
cargo build --features full            # 全量
```

确认 SSE endpoint 在 `--no-default-features` 下不编译（`#[cfg(feature="daemon")]` 正确）。确认 windows/linux/macos 均编译（CI 验证；本地至少 macos）。

- [ ] **Step 2: Commit（如有平台修复）**

```bash
git add -A && git commit -m "fix: cross-platform and feature-gate corrections"
```

---

## Self-Review

**Spec coverage:**
- `subagent-failure-diagnostics` → Tasks 1-6（root cause / sequence / context / retry history + 脱敏 + char-safe）✓
- `subagent-trace-streaming` → Tasks 11-15（JSONL sink / SSE / 冷启动回放 / 有界广播）✓
- `subagent-cli-tracing` → Tasks 16-19（list/trace/health + filters + 退出码）✓
- `subagent-transcript-storage`（MODIFIED）→ Tasks 7-10（4 列迁移 + 旧记录降级 + 同事务写入）✓
- `subagent-trace-html-report`（MODIFIED）→ Tasks 20-23（call_tree/error_timeline/html/raw）✓
- Config/docs → Tasks 24-26 ✓；lint/test/platform → Tasks 27-28 ✓

**Placeholder scan:** 实现步骤含具体 Rust 代码与类型签名；修改现有函数处标注定位 grep 命令 + 具体改动（新增列/字段），需 implementer 读取周边代码（既有 codebase 惯例，非占位符）。

**Type consistency:** `FailureRootCause`/`ToolCallStep`/`FailedRoundContext`/`RetryAttempt`/`RetryOutcome`/`TraceEvent`/`SinkMode`/`SubagentCommands`/`TraceStreamParams` 名称跨任务一致；`root_cause_enum()`/`list_all()`/`failure_modes_by_root_cause()`/`render_trace()`/`render_raw()` 跨任务引用一致。

**风险点:**
- `ProgressCallback` 是同步 `Fn`，`TraceSink::emit` 须用 `try_send`（Task 12 已注明）
- `list_all`/`failure_modes_by_root_cause` 为新增 store/health 方法（Tasks 17/19）
- daemon `AppState` 须持有共享 `TraceSink` 句柄（Task 14）
