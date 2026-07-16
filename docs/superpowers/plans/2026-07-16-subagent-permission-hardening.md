---
archived-with: 2026-07-16-subagent-permission-hardening
status: final
---
# Subagent Permission Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Route all subagent tool calls through a shared permission pipeline (visibility → policy → Ask resolution → guardian → execute), with user-facing escalation, explore/plan read-only enforcement, and parent-visible denial summaries.

**Architecture:** Introduce `GuardingToolPort` as a `ToolPort` wrapper used by `run_subagent_loop`. Extract shared validation from `ToolExecutor` / `ToolPermissionPolicy` so root and child cannot drift. Share `session_rules` via `Arc<RwLock<HashSet<String>>>`. On policy `Ask`, escalate to the user through a system-side approval bridge (TUI/daemon), not the main LLM. Role filters remove mutating FS tools for explore/plan when `explore_readonly` is true.

**Tech Stack:** Rust 2021, Tokio, `async_trait`, Serde, existing `ToolPermissionPolicy` / `ToolExecutor` / mailbox / TUI `PermissionRequired` path, Cargo tests.

**Design Doc:** `docs/superpowers/specs/2026-07-16-subagent-permission-hardening-design.md`  
**OpenSpec change:** `openspec/changes/subagent-permission-hardening/`  
**Base ref:** record at plan start via `git rev-parse HEAD` into `.comet.yaml` `base_ref`.

---

## File Map

### New files

- `src/teams/guarding_tool_port.rs` — `GuardingToolPort`, ask resolution, permission event counters
- `src/teams/permission_bridge.rs` — session-scoped pending policy approvals (oneshot + timeout + structured payload)
- `src/config/subagent_permissions.rs` (or fields on `SubagentLimits` in `agent.rs`) — settings types + defaults
- `tests/subagent_permission_hardening.rs` — integration: Ask approve/deny/timeout, explore filter (where feasible without live LLM)

### Existing files (focused changes)

- `src/teams/subagent_loop.rs` — replace `FilteredToolPort` usage; wire shared policy/rules/bridge; denial summary on finish
- `src/teams/mod.rs` — export new modules
- `src/tools/executor.rs` — `session_rules: Arc<RwLock<HashSet<String>>>`; shared validate helper; optional bridge hook for root (keep existing API working)
- `src/permissions/policy.rs` — keep `validate_tool_call` as source of truth; add helpers for structured ask metadata if needed (minimal)
- `src/tools/meta/task.rs` — explore/plan mutating-tool filter; pass permission config into spawn/loop
- `src/tools/meta/run_script.rs`, `src/tools/meta/rlm/pipeline.rs` — same port construction as task
- `src/teams/mailbox.rs` — optional structured fields on `ApprovalRequest` (serde default-compatible)
- `src/teams/approval_registry.rs` — reuse/extend for policy-ask waiters if cleaner than separate bridge
- `src/daemon/state.rs` / `handlers.rs` — expose shared session_rules + pending subagent permission events
- `src/tui/agent/adapters.rs` (and/or poll path) — drain permission bridge → existing `PermissionRequired` UI
- `src/config/agent.rs`, `settings.json.template`, `WGENTY.md` — new settings + docs
- Unit tests adjacent to changed modules

### Out of scope (do not do)

- Rewriting TaskGroup / claim delivery
- Full PermissionMode matrix (current policy has no accept_edits/bypass enum). **Interpretation for this codebase:** “follow root mode” = **share the same `ToolPermissionPolicy` + `session_rules` as the root session**. Optional `permission_mode` override is reserved; if set to a future mode string, document and either ignore with warn or implement only if already present—**do not invent a large mode system in this change**.
- MCP/plugin permission overhaul

---

## Task 1: Settings + defaults

**Files:**
- Modify: `src/config/agent.rs`
- Modify: `settings.json.template`
- Modify: `WGENTY.md` (settings table rows only; full behavior note can wait until Task 8 if preferred)
- Test: unit tests in `src/config/agent.rs` or existing config tests

- [x] **Step 1: Write failing default/serde tests**

```rust
#[test]
fn subagent_permission_defaults() {
    let limits = SubagentLimits::default();
    assert!(limits.explore_readonly); // default true
    assert_eq!(limits.ask_strategy, SubagentAskStrategy::EscalateToUser);
    assert_eq!(limits.approval_timeout_secs, 60);
    assert_eq!(limits.timeout_decision, TimeoutDecision::Deny);
    assert!(limits.permission_mode.is_none()); // follow root
}
```

- [x] **Step 2: Run test — expect FAIL** (fields missing)

Run: `cargo test subagent_permission_defaults -- --nocapture`

- [x] **Step 3: Add types + fields on `SubagentLimits`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentAskStrategy {
    #[default]
    EscalateToUser,
    Deny,
    // EscalateToParent reserved — optional stub, not required in P0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutDecision {
    #[default]
    Deny,
}

// On SubagentLimits (serde defaults):
// permission_mode: Option<String> = None  // null = follow root (shared policy/rules)
// ask_strategy: SubagentAskStrategy
// explore_readonly: bool = true
// approval_timeout_secs: u64 = 60
// timeout_decision: TimeoutDecision
```

Update `Default for SubagentLimits` and `settings.json.template` under `agent.subagent`.

- [x] **Step 4: Run test — expect PASS**

Run: `cargo test subagent_permission_defaults -- --nocapture`  
Also: `cargo check`

- [x] **Step 5: Commit**

```bash
git add src/config/agent.rs settings.json.template
git commit -m "feat(config): add subagent permission settings defaults"
```

---

## Task 2: Share session_rules on ToolExecutor

**Files:**
- Modify: `src/tools/executor.rs`
- Modify: daemon/TUI call sites that construct `ToolExecutor` or call `approve_rule` / `session_rules` if fields become private
- Test: unit tests in `executor.rs` or daemon tests

- [x] **Step 1: Write failing test for shared Arc rules**

```rust
#[test]
fn session_rules_are_shareable_via_arc() {
    let exec = ToolExecutor::new(Arc::new(ToolRegistry::new()), ToolPermissionPolicy::new(".".into()));
    let rules = exec.session_rules_handle(); // new accessor
    {
        let mut g = rules.blocking_write();
        g.insert("tool:file_write".into());
    }
    assert!(exec.session_rules_contains("tool:file_write"));
}
```

- [x] **Step 2: Run — expect FAIL**

- [x] **Step 3: Change storage**

```rust
// Before: session_rules: RwLock<HashSet<String>>
// After:
session_rules: Arc<RwLock<HashSet<String>>>,
```

- Keep `approve_rule` / `unapprove` / `validate_tool_call` working.
- Add `session_rules_handle(&self) -> Arc<RwLock<HashSet<String>>>`.
- Add constructor/helper `with_shared_session_rules(arc)` for child ports if useful.
- Extract pure helper (preferred location: keep calling `policy.validate_tool_call` from both places; no need to move policy out yet):

```rust
pub fn validate_tool_call_shared(
    registry: &ToolRegistry,
    policy: &ToolPermissionPolicy,
    session_rules: &HashSet<String>,
    tool_name: &str,
    args: &Value,
) -> Result<PolicyDecision, ToolError>
```

Implement by looking up tool + `policy.validate_tool_call(...)`. Use from `ToolExecutor::validate_tool_call` and later `GuardingToolPort`.

- [x] **Step 4: Run tests — PASS**

Run: `cargo test session_rules_are_shareable -- --nocapture`  
Run: `cargo test --lib tools::executor`

- [x] **Step 5: Commit**

```bash
git add src/tools/executor.rs
git commit -m "refactor(tools): share session_rules Arc and validate helper"
```

---

## Task 3: Permission bridge (structured Ask waiters)

**Files:**
- Create: `src/teams/permission_bridge.rs`
- Modify: `src/teams/mod.rs`
- Optionally extend: `src/teams/mailbox.rs` (structured optional fields)
- Test: unit tests in `permission_bridge.rs`

- [x] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn approve_resolves_waiter() {
    let bridge = PermissionBridge::new(Duration::from_secs(5));
    let req = StructuredApproval {
        request_id: "r1".into(),
        from: "child-a".into(),
        tool: "file_write".into(),
        policy_reason: "outside workspace".into(),
        session_rule: "path:/tmp/x".into(),
        paths: vec!["/tmp/x".into()],
        command: None,
        risk: None,
        human_summary: "write /tmp/x".into(),
    };
    let wait = bridge.request(req.clone());
    assert!(bridge.pending().iter().any(|p| p.request_id == "r1"));
    bridge.resolve("r1", true, /* always */ true);
    assert!(wait.await.unwrap());
}

#[tokio::test]
async fn timeout_denies() {
    let bridge = PermissionBridge::new(Duration::from_millis(30));
    let wait = bridge.request(StructuredApproval { request_id: "r2".into(), /* ... */ });
    assert!(!wait.await.unwrap());
}
```

- [x] **Step 2: Run — FAIL**

- [x] **Step 3: Implement `PermissionBridge`**

Requirements:
- `Arc`-friendly
- `request(approval) -> impl Future<Output = bool>` (or oneshot Receiver)
- timeout → false (deny)
- `resolve(request_id, approved, always_allow)`
- `pending() -> Vec<StructuredApproval>` for TUI/daemon poll
- On always_allow, caller inserts `session_rule` into shared rules (bridge may accept optional `session_rules` Arc and insert on Always)
- Thread-safe cleanup of waiters

Structured payload fields (design D3):  
`from`, `request_id`, `kind` (`policy_ask`), `tool`, `paths`, `command`, `risk`, `policy_reason`, `human_summary`, `session_rule`.

Optional: when escalating, also append mailbox `TeamMessage::ApprovalRequest` with `payload` = JSON of structured fields for observability; **resolution must not depend on LLM**.

- [x] **Step 4: Tests PASS**

- [x] **Step 5: Commit**

```bash
git add src/teams/permission_bridge.rs src/teams/mod.rs src/teams/mailbox.rs
git commit -m "feat(teams): add structured permission bridge for subagent Ask"
```

---

## Task 4: GuardingToolPort (P0 core)

**Files:**
- Create: `src/teams/guarding_tool_port.rs`
- Modify: `src/teams/subagent_loop.rs` — use GuardingToolPort instead of FilteredToolPort
- Modify: `src/teams/mod.rs`
- Test: unit tests in `guarding_tool_port.rs`

- [x] **Step 1: Write failing unit tests**

Cases:
1. Tool not in `allowed` → error code `tool_not_allowed`, registry not called (use mock registry or thin fake ToolPort inner).
2. Read-only in-workspace tool → Allow path executes.
3. Write outside workspace → policy Ask; with empty bridge / ask_strategy Deny → `permission_denied` or `approval_unavailable`, no execute.
4. session_rules pre-insert matching `session_rule` → executes without bridge.
5. `exec_command` high-risk: guardian_check blocks when guardian would block (reuse `CommandGuardian` defaults / inject guardian).

Minimal structure:

```rust
pub struct GuardingToolPort {
    inner_registry: Arc<ToolRegistry>, // or Box<dyn ToolPort> for testability
    allowed: HashSet<String>,
    policy: ToolPermissionPolicy, // or Arc
    session_rules: Arc<RwLock<HashSet<String>>>,
    guardian: CommandGuardian, // or Arc / shared with ToolExecutor
    bridge: Option<Arc<PermissionBridge>>,
    ask_strategy: SubagentAskStrategy,
    approval_timeout: Duration,
    // metrics
    denial_log: Arc<Mutex<Vec<String>>>,
}
```

Execute pipeline:
1. allowed check
2. `validate_tool_call_shared`
3. On `Allow` → continue
4. On `Ask`:
   - if rules hit (policy already checks rules; double-check)
   - `ask_strategy == Deny` or `bridge is None` → fail closed (`approval_unavailable` / `permission_denied`)
   - else `bridge.request(structured)` await → false = deny
   - if Always was chosen, rules already updated by bridge/resolve
5. `guardian_check` for exec tools (copy logic from `ToolExecutor::guardian_check`)
6. `registry.execute_with_context`

Return `ToolError` with stable codes: `tool_not_allowed`, `permission_denied`, `approval_unavailable`.

- [x] **Step 2: Run — FAIL**

- [x] **Step 3: Implement GuardingToolPort + wire into `run_subagent_loop`**

Signature change (additive preferred):

```rust
pub async fn run_subagent_loop(
    // existing params...
    permission: SubagentPermissionContext, // new: policy, session_rules, bridge, settings slice, guardian
)
```

Or overload via a small struct to avoid 15-arg explosion:

```rust
pub struct SubagentPermissionContext {
    pub policy: ToolPermissionPolicy,
    pub session_rules: Arc<RwLock<HashSet<String>>>,
    pub bridge: Option<Arc<PermissionBridge>>,
    pub ask_strategy: SubagentAskStrategy,
    pub approval_timeout_secs: u64,
    pub guardian: CommandGuardian,
}
```

Update call sites: `task.rs`, `run_script.rs`, `rlm/pipeline.rs`.  
If a call site lacks bridge (tests/headless), pass `bridge: None` → Ask fail closed.

**Deprecate path:** keep `FilteredToolPort` only if tests still need it; otherwise replace entirely and delete dead code.

- [x] **Step 4: Unit tests PASS; `cargo check`**

- [x] **Step 5: Commit**

```bash
git add src/teams/guarding_tool_port.rs src/teams/subagent_loop.rs src/teams/mod.rs src/tools/meta/task.rs src/tools/meta/run_script.rs src/tools/meta/rlm/pipeline.rs
git commit -m "feat(teams): GuardingToolPort unifies subagent tool permissions"
```

---

## Task 5: Explore/plan true read-only filter

**Files:**
- Modify: `src/tools/meta/task.rs` (`allowed_tools` filter ~L327–352)
- Test: unit test for filter function (extract pure fn)

- [x] **Step 1: Write failing test**

```rust
#[test]
fn explore_readonly_filters_mutating_fs_tools() {
    let all = vec![
        "file_read", "file_write", "file_edit", "apply_patch",
        "grep", "exec_command", "task", "delegate",
    ];
    let filtered = filter_allowed_tools(all, "explore", /*depth*/ 0, /*max_depth*/ 1, /*explore_readonly*/ true);
    assert!(filtered.contains(&"file_read".into()));
    assert!(filtered.contains(&"grep".into()));
    assert!(filtered.contains(&"exec_command".into())); // design: exec remains
    assert!(!filtered.contains(&"file_write".into()));
    assert!(!filtered.contains(&"file_edit".into()));
    assert!(!filtered.contains(&"apply_patch".into()));
    assert!(!filtered.contains(&"task".into()));
}
```

- [x] **Step 2: Run — FAIL**

- [x] **Step 3: Implement filter**

```rust
const MUTATING_FS: &[&str] = &["file_write", "file_edit", "apply_patch"];

fn filter_allowed_tools(
    names: impl IntoIterator<Item = String>,
    subagent_type: &str,
    depth: usize,
    max_depth: usize,
    explore_readonly: bool,
) -> Vec<String> {
    let is_leaf = matches!(subagent_type, "explore" | "plan");
    names.into_iter().filter(|name| {
        let is_spawn = name == "task" || name == "delegate";
        if is_spawn {
            return !is_leaf && depth < max_depth;
        }
        if explore_readonly && is_leaf && MUTATING_FS.contains(&name.as_str()) {
            return false;
        }
        true
    }).collect()
}
```

Wire `explore_readonly` from `settings.agent.subagent.explore_readonly`.

- [x] **Step 4: Tests PASS**

- [x] **Step 5: Commit**

```bash
git add src/tools/meta/task.rs
git commit -m "feat(task): enforce explore/plan readonly tool visibility"
```

---

## Task 6: Root/TUI/Daemon approval bridge consumption

**Files:**
- Modify: `src/daemon/state.rs` (store `Arc<PermissionBridge>` and/or per-session)
- Modify: `src/daemon/handlers.rs` (poll/resolve endpoints or piggyback existing approve flow)
- Modify: `src/tui/agent/adapters.rs` / client poll — present `PermissionRequired` for pending bridge items
- Test: integration or daemon unit test with mock bridge resolve

- [x] **Step 1: Write failing integration-style test**

Prefer a daemon/state unit test without full TUI:

```rust
#[tokio::test]
async fn bridge_pending_surfaces_and_resolve_unblocks() {
    // create bridge, push request, assert state.pending_permissions() non-empty
    // resolve via same API TUI would call
    // waiter completes true and session_rule present if always
}
```

- [x] **Step 2: Implement wiring**

Recommended approach (minimal UI churn):
1. Session owns `Arc<PermissionBridge>` next to `ToolExecutor`.
2. When TUI is idle / before next tool poll, call `bridge.pending()`.
3. For each pending item, send existing `AppEvent::PermissionRequired { reason, rule, responder }`.
4. Map user response:
   - AllowOnce → `resolve(id, true, always=false)`
   - AlwaysAllow → insert rule + `resolve(id, true, always=true)`
   - Deny → `resolve(id, false, false)`
5. Headless: no bridge consumer → GuardingToolPort already fail closed.

**Do not** require main agent LLM to read mailbox.

Reuse `approve_tool` / session_rules for Always so root and children share rules.

- [x] **Step 3: Manual checklist** (document in commit body if full TUI e2e hard in CI)

- Child triggers outside-workspace write → TUI permission prompt appears  
- Always allow → second similar call skips prompt  
- Deny → tool error, no file written  

- [x] **Step 4: Automated tests PASS; `cargo check`**

- [x] **Step 5: Commit**

```bash
git add src/daemon src/tui src/teams
git commit -m "feat(tui,daemon): bridge subagent policy Ask to PermissionRequired UI"
```

---

## Task 7: Observability (denial summary)

**Files:**
- Modify: `src/teams/guarding_tool_port.rs` (denial_log)
- Modify: `src/teams/subagent_loop.rs` (append summary on completion / finish_child path in `task.rs`)
- Test: unit test that formats summary suffix

- [x] **Step 1: Write failing test**

```rust
#[test]
fn denial_summary_suffix_format() {
    let reasons = vec!["tool_not_allowed".into(), "permission_denied".into()];
    let s = format_permission_summary(&reasons);
    assert!(s.contains("2 denials"));
    assert!(s.contains("permission_denied"));
}
```

- [x] **Step 2: Implement**

- On each deny in GuardingToolPort, push short reason to shared `denial_log`.
- Optionally emit progress/action_log events: `permission_denied`, `approval_requested`, `approval_resolved`.
- When building final text / `ChildResult.summary` in `task.rs` finish path, if denials non-empty append:

```text
[permissions: 2 denials; last: permission_denied, tool_not_allowed]
```

**Do not** change `ChildResult` field set (still five fields).

- [x] **Step 3: Tests PASS**

- [x] **Step 4: Commit**

```bash
git add src/teams src/tools/meta/task.rs
git commit -m "feat(teams): surface subagent permission denials in summary"
```

---

## Task 8: Docs + OpenSpec task checkoff + regression

**Files:**
- Modify: `WGENTY.md` (subagent permission settings rows)
- Modify: `openspec/changes/subagent-permission-hardening/tasks.md` (check boxes as done)
- Modify: `.comet.yaml` `base_ref` / `plan` if not already
- Tests: targeted + broader

- [x] **Step 1: Document settings in WGENTY.md**

Rows:
- `agent.subagent.permission_mode` — Option, default null (follow root shared policy/rules)
- `agent.subagent.ask_strategy` — default `escalate_to_user`
- `agent.subagent.explore_readonly` — default true
- `agent.subagent.approval_timeout_secs` — default 60
- `agent.subagent.timeout_decision` — default `deny`

- [x] **Step 2: Run verification suite**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib teams::
cargo test --lib tools::executor
cargo test --lib tools::meta::task
cargo test subagent_permission
# plus any existing task-group / subagent delivery tests:
cargo test task_group
cargo test subagent
```

Fix failures related to this change only.

- [x] **Step 3: Check off completed items in `tasks.md`**

- [x] **Step 4: Commit**

```bash
git add WGENTY.md openspec/changes/subagent-permission-hardening settings.json.template
git commit -m "docs: subagent permission hardening settings and task checkoff"
```

---

## Task 9: Final self-check against design success criteria

- [x] **Step 1: Walk Success Criteria**

1. Child dangerous exec goes through policy + guardian (no registry bypass) — covered by Task 4 tests  
2. explore `file_write` rejected by allow-list — Task 5  
3. Ask escalate + approve continues; timeout Deny — Tasks 3–6  
4. Root handles structured approval without LLM — Task 6  
5. Denials visible in summary — Task 7  
6. Settings configurable — Task 1/8  
7. fmt/clippy/tests — Task 8  

- [x] **Step 2: Note residual risks** in PR/summary (headless CI needs pre-seeded rules for Ask paths)

- [x] **Step 3: No further commit required unless fixes**

---

## Execution notes for build mode

- Prefer **subagent-driven-development** with one task per implementer, review between tasks.
- TDD: for each task, red → green → commit as written.
- If isolation worktree is enabled, run all commands inside that worktree with absolute paths.
- Do not expand scope to full PermissionMode product matrix.
- Security-sensitive: prefer fail closed on any uncertainty (missing bridge, timeout, parse error).

## Dependency order

```
Task1 settings
  → Task2 shared session_rules
    → Task3 permission bridge
      → Task4 GuardingToolPort + call sites
        → Task5 explore filter (can parallelize after Task1)
        → Task6 TUI/daemon bridge consumer
          → Task7 observability
            → Task8 docs/regression
              → Task9 checklist
```

Tasks 5 can start after Task 1 in parallel with Task 2–3 if staffing allows.
