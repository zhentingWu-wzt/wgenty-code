---
change: org-graph-shared-state
design-doc: docs/superpowers/specs/2026-08-11-org-graph-shared-state-design.md
base-ref: a819ff03bb2736519ff1945491c7c21838d5e6d9
archived-with: 2026-08-11-org-graph-shared-state
---

# Org-Graph Shared-State 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `exec_session/node_runtime.rs:204` 的结构化降级点（强类型 `VerifyFailure` → `format!("{f:?}")` → `Option<String>`），引入**完整 schema** 的 `WorkState` 强类型共享状态（7+1 字段 + 全字段权限真强制）、turn 检查点持久化，让 pilot 路由点（verify 出口）从「解析自然语言字符串」改为「读结构化枚举字段」。pilot 锚定唯一真实闭环 verify_result；compile/test/human_review/budget/generated_diff 类型与权限就绪、生产写入点 deferred（详见 design §1.5）。

**Architecture:** 新增 `src/org_graph/work_state.rs` 承载完整 `WorkState` schema（7+1 字段：`requirement` / `generated_diff` / `compile_result` / `test_result` / `human_review` / `verify_result` / `budget` / `step_log`）+ 全部子类型（`GeneratedDiff` / `CompileResult` / `TestResult` / `HumanReview` / `Budget` / `VerifyOutcome` / `VerifyFailureKind`）+ 字段权限读写 API + `FieldPerms` / `WorkField`（8 变体）。在 `ContractDimension` 新增 `State` 变体以复用 `CoordinatorError::ContractViolation` 报错路径。`WorkState` 挂在 `SessionCoordinator` 内（复用现有 `Arc<RwLock>` 锁层级），随 `begin_turn` 做 turn 间继承（requirement 保留、其余产物字段重置），随 `CheckpointStore` 旁路持久化（per-turn `work_state.json`）。pilot 集成点（`verify_node` 出口）先经 `set_verify_result` 把强类型投影写入 `WorkState`（受 `NodeType::Verification` 字段权限强制），再从 `WorkState.verify_result()` 读回组装兼容期的 `failure_reason: Option<String>`，使 retry 决策改为直接读 `Option<&VerifyOutcome>` 拿 `VerifyFailureKind` 枚举分支。其余字段的具名 setter（`set_generated_diff` / `set_budget` / `set_compile_result` / `set_test_result` / `set_human_review`）提供实现 + 全字段权限强制，本期无生产调用点，由单测合成写入覆盖。

**Tech Stack:** Rust 2021 / cargo / serde / thiserror / chrono / uuid / tokio（既有栈，零新 crate）。

## Global Constraints

- 本 change 纯新增只读 + 结构化层；**不改** `NodeContract` 五维语义、不改 `reserve_child` / `filter_allowed_tools` 强制逻辑、不改 transcript store schema、不改 dispatch 路径。
- `org_graph` 模块保持「纯数据 + 纯函数，零 async / I/O / 状态」；集成（`VerifyResult` → `VerifyOutcome` 投影、turn 锚定、CheckpointStore 持久化）在 `exec_session` 侧。
- 所有新类型派生 `Serialize/Deserialize/Clone/Debug`（支持持久化与单测）。
- 字段级权限**真强制**：越权写直接返回 `ContractViolation { dimension: State }`，不做「声明 + warning」软路径；**预留字段**（`compile_result` / `test_result` / `human_review`）对所有现存 `NodeType` 的 writable 强制为 `{}`。
- `NodeVerifyResult.failure_reason: Option<String>` **本期保留**（兼容期，零回归），但源头从 `format!("{f:?}")` 改为「从 WorkState 强类型枚举读回再转 debug string」。
- 每个 task 遵循 TDD：先写失败测试 → 跑测试见红 → 写最小实现 → 跑测试见绿 → commit。
- `ContractDimension` 新增 `State` 变体后，全仓库 exhaustive match 需审计，补 `State` arm 或 `_ =>`（见 Task 2 Step 3）。
- 与 `org-graph-dispatch-telemetry` 正交：两者字段/schema 互不依赖、互不修改。
- 所有产物使用简体中文（zh-CN）书写注释与文档字符串；代码标识符（类型/函数/变量名）保持英文。

## File Structure

**新建：**

- `src/org_graph/work_state.rs` —— 完整 `WorkState` schema（7+1 字段）+ 全部子类型（`GeneratedDiff` / `CompileResult` / `TestResult` / `HumanReview` / `Budget` / `VerifyOutcome` / `VerifyFailureKind`）+ `FieldPerms` / `WorkField`（8 变体，权限矩阵）+ `StepRecord` / `StepAction`（审计轨迹）+ `NodeType::field_perms()` impl + `WorkState` 受权限约束的全字段读写 API（pilot 集成的 `set_verify_result` / `verify_result` + deferred 字段的 `set_generated_diff` / `set_budget` / `set_compile_result` / `set_test_result` / `set_human_review` 及对应 getter + `inherit_for_new_turn`）+ `VerifyOutcome::from_parts` 构造器。纯数据 + 纯函数，零 `exec_session` 依赖（投影转换用原语参数隔离，见 Task 4 Step 4）。

**修改：**

- `src/org_graph/mod.rs` —— 导出 `pub mod work_state;` 及全部公开类型。
- `src/org_graph/contract.rs` —— `ContractDimension` 新增 `State` 变体（置于 `Budget` 之后，保持 enum 序以减小 serde 影响面）。
- `src/exec_session/coordinator.rs` —— `SessionCoordinator` 新增 `work_state: WorkState` 字段；`SessionCoordinator::new` 初始化；`begin_turn` 在 turn 链推进后调用 `self.work_state = self.work_state.inherit_for_new_turn()`；新增 `work_state()` / `work_state_mut()` 访问器。
- `src/exec_session/node_runtime.rs` —— `verify_node` 出口（line 204 附近）改为：先把 `VerifyResult` 投影成 `VerifyOutcome` 并经 `set_verify_result(NodeType::Verification, outcome)` 写入 `WorkState`，再从 `verify_result(NodeType::Verification)` 读回组装 `failure_reason`；retry 决策改为读 `Option<&VerifyOutcome>` 拿 `VerifyFailureKind` 枚举分支。
- `src/tools/checkpoint_store.rs` —— 新增 `capture_work_state(turn_id, &WorkState)` / `restore_work_state(turn_id)` 方法（写/读 turn 目录下的 `work_state.json` 旁路文件，不动文件 capture 语义）；持久化触发点在 `SessionCoordinator`。

**不改：** `reserve_child` / `can_spawn` / `filter_allowed_tools` / `SubagentResultMailbox` / dispatch 路径 / `AppState` / `SessionState` 字段语义。

---

### Task 1: WorkState 完整 schema 与模块骨架

**Files:**
- Create: `src/org_graph/work_state.rs`
- Modify: `src/org_graph/mod.rs`
- Test: `src/org_graph/work_state.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `NodeType`（来自 `super::contract`，已存在）；后续 Task 4 会从 `crate::exec_session::verify_gate::VerifyResult` / `crate::exec_session::hooks::VerifyFailure` 投影转换。
- Produces: 完整 schema 类型族（签名见 Step 1.3 代码块），后续 Task 2/3/4 全部依赖。

- [x] **Step 1.1: 写失败测试 —— schema serde 往返 + 默认值（全字段 + 全子类型）**

在新建文件 `src/org_graph/work_state.rs` 末尾的 `#[cfg(test)] mod tests` 中先写测试（此时类型未定义 → 编译失败 = 红）。

```rust
//! WorkState: per-task 结构化工作产物 schema（完整 7+1 字段）+ 字段权限读写 API。
//!
//! 本模块保持 org_graph「纯数据 + 纯函数」风格：无 async / I/O / 状态。
//! `exec_session` 侧负责把 `VerifyResult` 投影成 `VerifyOutcome` 并调用读写 API。
//! pilot 仅锚定 verify_result（唯一真实闭环）；compile/test/human_review/budget/
//! generated_diff 类型与权限就绪，生产写入点待将来新增节点的 change 接入（design §1.5）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::contract::NodeType;

/// 当前 turn 的结构化工作产物。完整 schema：全字段类型 + 全字段权限真强制。
/// pilot 仅锚定 verify_result（唯一真实闭环）；其余字段类型与权限就绪，
/// 生产写入点待将来新增 Compile/Test 等节点的 change 接入。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkState {
    /// 任务原始需求（跨 turn 继承；coordinator 在 turn 初始化时设置，不经节点权限 API）。
    pub requirement: Option<String>,
    /// GeneralPurpose 产出（类型就绪，生产写入待接入）。
    pub generated_diff: Option<GeneratedDiff>,
    /// 预留：将来 Compile 节点写入。
    pub compile_result: Option<CompileResult>,
    /// 预留：将来 Test 节点写入。
    pub test_result: Option<TestResult>,
    /// 预留：将来人工评审节点写入。
    pub human_review: Option<HumanReview>,
    /// pilot 核心字段：verify 结果的强类型投影。
    pub verify_result: Option<VerifyOutcome>,
    /// 预留：预算追踪（类型就绪，生产写入待接入）。
    pub budget: Option<Budget>,
    /// 审计轨迹（授权写记入；读不记）。
    pub step_log: Vec<StepRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedDiff {
    pub summary: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompileResult {
    pub ok: bool,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestResult {
    pub pass: bool,
    pub failed_cases: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HumanReview {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Budget {
    pub max_iter: u32,
    pub iter_used: u32,
    pub token_used: u64,
}

/// org_graph 内聚的独立类型（exec_session 集成时从 VerifyResult 投影转换）。
/// 只保留 retry 决策需要的 success + fail_reason 枚举，不复制完整 VerifyResult。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub success: bool,
    pub fail_reason: Option<VerifyFailureKind>,
}

/// org_graph 侧的失败枚举（独立于 exec_session::hooks::VerifyFailure，
/// 避免 org_graph 反向依赖 exec_session；投影转换在 Task 4 完成）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerifyFailureKind {
    CommandFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    BoundaryViolation {
        unexpected_files: Vec<String>,
    },
}

/// 字段枚举：用于权限矩阵与 step_log 审计。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum WorkField {
    Requirement,
    GeneratedDiff,
    CompileResult,
    TestResult,
    HumanReview,
    VerifyResult,
    Budget,
    StepLog,
}

/// 审计轨迹条目：谁、何时、读/写了哪个字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepRecord {
    pub node_type: NodeType,
    pub field: WorkField,
    pub action: StepAction,
    /// rfc3339 时间戳（与 coordinator.rs 的 turn 记录风格一致）。
    pub timestamp: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepAction {
    Read,
    Wrote,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workstate_serde_roundtrip_empty() {
        let state = WorkState::default();
        let json = serde_json::to_string(&state).expect("serialize");
        let back: WorkState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, back);
        assert!(state.requirement.is_none());
        assert!(state.generated_diff.is_none());
        assert!(state.compile_result.is_none());
        assert!(state.test_result.is_none());
        assert!(state.human_review.is_none());
        assert!(state.verify_result.is_none());
        assert!(state.budget.is_none());
        assert!(state.step_log.is_empty());
    }

    #[test]
    fn workstate_serde_roundtrip_populated_all_fields() {
        let state = WorkState {
            requirement: Some("实现 WorkState".into()),
            generated_diff: Some(GeneratedDiff {
                summary: "改 3 个文件".into(),
                files: vec!["a.rs".into(), "b.rs".into()],
            }),
            compile_result: Some(CompileResult {
                ok: false,
                stderr: "error: mismatched types".into(),
            }),
            test_result: Some(TestResult {
                pass: false,
                failed_cases: vec!["test_a".into()],
            }),
            human_review: Some(HumanReview::Reject),
            verify_result: Some(VerifyOutcome {
                success: false,
                fail_reason: Some(VerifyFailureKind::CommandFailed {
                    exit_code: Some(1),
                    stderr: "command failed".into(),
                }),
            }),
            budget: Some(Budget {
                max_iter: 5,
                iter_used: 2,
                token_used: 1024,
            }),
            step_log: vec![StepRecord {
                node_type: NodeType::Verification,
                field: WorkField::VerifyResult,
                action: StepAction::Wrote,
                timestamp: "2026-08-11T00:00:00Z".into(),
            }],
        };
        let json = serde_json::to_string(&state).expect("serialize");
        let back: WorkState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, back);
    }

    #[test]
    fn subtypes_serde_roundtrip() {
        // GeneratedDiff
        let gd = GeneratedDiff {
            summary: "s".into(),
            files: vec!["x".into()],
        };
        let back: GeneratedDiff =
            serde_json::from_str(&serde_json::to_string(&gd).unwrap()).unwrap();
        assert_eq!(gd, back);

        // CompileResult
        let cr = CompileResult { ok: true, stderr: String::new() };
        let back: CompileResult =
            serde_json::from_str(&serde_json::to_string(&cr).unwrap()).unwrap();
        assert_eq!(cr, back);

        // TestResult
        let tr = TestResult { pass: false, failed_cases: vec!["c1".into()] };
        let back: TestResult =
            serde_json::from_str(&serde_json::to_string(&tr).unwrap()).unwrap();
        assert_eq!(tr, back);

        // HumanReview（两个变体）
        for hr in [HumanReview::Approve, HumanReview::Reject] {
            let back: HumanReview =
                serde_json::from_str(&serde_json::to_string(&hr).unwrap()).unwrap();
            assert_eq!(hr, back);
        }

        // Budget
        let b = Budget { max_iter: 3, iter_used: 1, token_used: 500 };
        let back: Budget =
            serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn verify_failure_kind_serde_roundtrip_all_variants() {
        for kind in [
            VerifyFailureKind::CommandFailed {
                exit_code: Some(2),
                stderr: "boom".into(),
            },
            VerifyFailureKind::CommandFailed {
                exit_code: None,
                stderr: String::new(),
            },
            VerifyFailureKind::BoundaryViolation {
                unexpected_files: vec!["a.rs".into(), "b.rs".into()],
            },
        ] {
            let json = serde_json::to_string(&kind).expect("serialize");
            let back: VerifyFailureKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn workfield_has_eight_variants() {
        // 全字段权限矩阵依赖 8 个 WorkField 变体；缺一个会让 Task 2 矩阵不完整。
        let all = [
            WorkField::Requirement,
            WorkField::GeneratedDiff,
            WorkField::CompileResult,
            WorkField::TestResult,
            WorkField::HumanReview,
            WorkField::VerifyResult,
            WorkField::Budget,
            WorkField::StepLog,
        ];
        // 8 个互异（Hash 去重后仍 8 个）。
        let set: HashSet<WorkField> = all.iter().copied().collect();
        assert_eq!(set.len(), 8);
    }
}
```

- [x] **Step 1.2: 跑测试验证失败（红）**

Run: `cargo test --lib org_graph::work_state::tests --no-run`
Expected: 编译失败 —— `error[E0433]: failed to resolve: could not find work_state in org_graph`（模块尚未在 mod.rs 导出）。

- [x] **Step 1.3: 在 mod.rs 导出 work_state 模块**

编辑 `src/org_graph/mod.rs`：

```rust
//! Org-Graph 节点契约模块：纯数据 + 纯函数校验，无 async / I/O / 状态。

pub mod contract;
pub mod registry;
pub mod render;
pub mod work_state;

pub use contract::{
    Capability, ContractDimension, IoShape, NodeContract, NodeType, PermissionBoundary,
    ResourceBudget,
};
pub use registry::NodeRegistry;
pub use work_state::{
    Budget, CompileResult, GeneratedDiff, HumanReview, StepAction, StepRecord, TestResult,
    VerifyFailureKind, VerifyOutcome, WorkField, WorkState,
};
```

- [x] **Step 1.4: 跑测试验证通过（绿）**

Run: `cargo test --lib org_graph::work_state::tests`
Expected: PASS —— 5 个测试全绿（空 schema 往返、全字段填充往返、子类型往返、VerifyFailureKind 全变体、WorkField 8 变体）。

- [x] **Step 1.5: Commit**

```bash
git add src/org_graph/work_state.rs src/org_graph/mod.rs
git commit -m "feat(org-graph): add full WorkState schema (7+1 fields) + subtypes

Task 1 (完整 schema 骨架): 新增 work_state.rs 承载 per-task 结构化工作产物。
WorkState 完整 7+1 字段（requirement/generated_diff/compile_result/test_result/
human_review/verify_result/budget/step_log）+ 子类型 GeneratedDiff/CompileResult/
TestResult/HumanReview/Budget/VerifyOutcome/VerifyFailureKind。pilot 仅锚定
verify_result；其余字段类型就绪，生产写入点 deferred（design §1.5）。
全部派生 Serialize/Deserialize/Clone/Debug 以支持 CheckpointStore 持久化与单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 字段级访问权限（全字段真强制）

**Files:**
- Modify: `src/org_graph/work_state.rs`（新增 `FieldPerms` / `NodeType::field_perms()` impl / 全字段读写 API / `inherit_for_new_turn`）
- Modify: `src/org_graph/contract.rs`（`ContractDimension` 新增 `State` 变体）
- Modify: `src/org_graph/mod.rs`（导出 `FieldPerms`）
- Audit: 全仓库对 `ContractDimension` 的 exhaustive match
- Test: `src/org_graph/work_state.rs::tests`

**Interfaces:**
- Consumes: `NodeType`（来自 `super::contract`）；`CoordinatorError::ContractViolation`（来自 `crate::agent::coordinator`，已存在 `{ node_type, dimension, reason }` 形状）。
- Produces: `pub struct FieldPerms { readable: HashSet<WorkField>, writable: HashSet<WorkField> }`；`impl NodeType { pub fn field_perms(&self) -> FieldPerms }`；全字段 setter/getter（pilot: `set_verify_result` / `verify_result`；deferred: `set_generated_diff` / `generated_diff` / `set_budget` / `budget` / `set_compile_result` / `compile_result` / `set_test_result` / `test_result` / `set_human_review` / `human_review`）；`impl WorkState { pub fn inherit_for_new_turn(&self) -> WorkState }`。

- [x] **Step 2.1: 写失败测试 —— 全字段权限矩阵 + 越权写拒绝 + 预留字段强制为空 + step_log 记入**

在 `src/org_graph/work_state.rs` 的 `#[cfg(test)] mod tests` 末尾追加测试（此时 `field_perms` / setter / getter 尚未实现 → 编译失败 = 红）。

```rust
    #[test]
    fn field_perms_verification_writes_verify_result_reads_broad() {
        let perms = NodeType::Verification.field_perms();
        // 写：仅 verify_result（step_log 由授权写自动记入，不直接 set）。
        assert!(perms.writable.contains(&WorkField::VerifyResult));
        assert!(!perms.writable.contains(&WorkField::StepLog));
        // 读：requirement/verify_result/compile_result/test_result/step_log。
        for f in [
            WorkField::Requirement,
            WorkField::VerifyResult,
            WorkField::CompileResult,
            WorkField::TestResult,
            WorkField::StepLog,
        ] {
            assert!(perms.readable.contains(&f), "Verification should read {f:?}");
        }
        // 预留字段对 Verification 不可写。
        for f in [WorkField::CompileResult, WorkField::TestResult, WorkField::HumanReview] {
            assert!(!perms.writable.contains(&f), "Verification must not write {f:?}");
        }
    }

    #[test]
    fn field_perms_generalpurpose_writes_diff_budget_reads_all() {
        let perms = NodeType::GeneralPurpose.field_perms();
        // 写：generated_diff + budget。
        assert!(perms.writable.contains(&WorkField::GeneratedDiff));
        assert!(perms.writable.contains(&WorkField::Budget));
        // 读：全 8 字段中除 step_log 外基本可读（协调者需广视野）。
        for f in [
            WorkField::Requirement,
            WorkField::GeneratedDiff,
            WorkField::VerifyResult,
            WorkField::CompileResult,
            WorkField::TestResult,
            WorkField::HumanReview,
            WorkField::Budget,
            WorkField::StepLog,
        ] {
            assert!(perms.readable.contains(&f), "GeneralPurpose should read {f:?}");
        }
        // GeneralPurpose 不可写 verify_result / 预留字段。
        assert!(!perms.writable.contains(&WorkField::VerifyResult));
        assert!(!perms.writable.contains(&WorkField::HumanReview));
    }

    #[test]
    fn field_perms_explore_plan_guide_only_read_requirement() {
        for nt in [NodeType::Explore, NodeType::Plan, NodeType::WgentyCodeGuide] {
            let perms = nt.field_perms();
            assert!(perms.readable.contains(&WorkField::Requirement));
            assert_eq!(perms.readable.len(), 1, "{nt:?} should only read requirement");
            assert!(perms.writable.is_empty(), "{nt:?} should have empty writable");
        }
    }

    #[test]
    fn set_verify_result_authorizes_verification_node_and_logs_step() {
        let mut state = WorkState::default();
        let outcome = VerifyOutcome {
            success: false,
            fail_reason: Some(VerifyFailureKind::CommandFailed {
                exit_code: Some(1),
                stderr: "boom".into(),
            }),
        };
        state
            .set_verify_result(NodeType::Verification, outcome.clone())
            .expect("Verification authorized to write verify_result");
        assert_eq!(state.verify_result, Some(outcome));
        assert_eq!(state.step_log.len(), 1);
        let record = &state.step_log[0];
        assert_eq!(record.node_type, NodeType::Verification);
        assert_eq!(record.field, WorkField::VerifyResult);
        assert_eq!(record.action, StepAction::Wrote);
    }

    #[test]
    fn set_verify_result_rejects_unauthorized_node_with_contract_violation_state() {
        let mut state = WorkState::default();
        let outcome = VerifyOutcome { success: true, fail_reason: None };
        let err = state
            .set_verify_result(NodeType::Explore, outcome.clone())
            .expect_err("Explore must not write verify_result");
        match err {
            crate::agent::coordinator::CoordinatorError::ContractViolation {
                dimension, reason, ..
            } => {
                assert_eq!(dimension, crate::org_graph::ContractDimension::State);
                assert!(reason.contains("verify_result"));
            }
            other => panic!("expected ContractViolation(State), got {:?}", other),
        }
        assert!(state.verify_result.is_none());
        assert!(state.step_log.is_empty());
    }

    #[test]
    fn set_generated_diff_authorizes_generalpurpose_and_logs_step() {
        // deferred 字段：GeneralPurpose 合成写入验证（无生产调用点，单测覆盖权限强制）。
        let mut state = WorkState::default();
        let diff = GeneratedDiff { summary: "改 1 文件".into(), files: vec!["a.rs".into()] };
        state
            .set_generated_diff(NodeType::GeneralPurpose, diff.clone())
            .expect("GeneralPurpose authorized to write generated_diff");
        assert_eq!(state.generated_diff, Some(diff));
        assert_eq!(state.step_log.len(), 1);
        assert_eq!(state.step_log[0].field, WorkField::GeneratedDiff);
    }

    #[test]
    fn set_budget_authorizes_generalpurpose() {
        let mut state = WorkState::default();
        let budget = Budget { max_iter: 5, iter_used: 1, token_used: 100 };
        state
            .set_budget(NodeType::GeneralPurpose, budget.clone())
            .expect("GeneralPurpose authorized to write budget");
        assert_eq!(state.budget, Some(budget));
    }

    #[test]
    fn reserved_fields_reject_all_node_types() {
        // 真强制核心保证：compile_result/test_result/human_review 对所有现存 NodeType
        // writable 都为 {}。逐一验证越权写返回 ContractViolation{State}。
        let all_nodes = [
            NodeType::Explore,
            NodeType::Plan,
            NodeType::GeneralPurpose,
            NodeType::Verification,
            NodeType::WgentyCodeGuide,
        ];
        for nt in all_nodes {
            let mut state = WorkState::default();
            let err = state
                .set_compile_result(nt, CompileResult { ok: true, stderr: String::new() })
                .err();
            assert!(
                matches!(
                    err,
                    Some(crate::agent::coordinator::CoordinatorError::ContractViolation { .. })
                ),
                "set_compile_result must reject {nt:?}"
            );

            let mut state = WorkState::default();
            let err = state
                .set_test_result(nt, TestResult { pass: true, failed_cases: vec![] })
                .err();
            assert!(
                matches!(
                    err,
                    Some(crate::agent::coordinator::CoordinatorError::ContractViolation { .. })
                ),
                "set_test_result must reject {nt:?}"
            );

            let mut state = WorkState::default();
            let err = state
                .set_human_review(nt, HumanReview::Approve)
                .err();
            assert!(
                matches!(
                    err,
                    Some(crate::agent::coordinator::CoordinatorError::ContractViolation { .. })
                ),
                "set_human_review must reject {nt:?}"
            );
        }
    }

    #[test]
    fn verify_result_read_authorizes_verification_and_skips_log() {
        let mut state = WorkState::default();
        state.verify_result = Some(VerifyOutcome { success: true, fail_reason: None });
        let got = state
            .verify_result(NodeType::Verification)
            .expect("Verification authorized to read verify_result");
        assert!(got.is_some());
        assert!(state.step_log.is_empty(), "read must not append step_log");
    }

    #[test]
    fn verify_result_read_rejects_unauthorized_node() {
        let mut state = WorkState::default();
        state.verify_result = Some(VerifyOutcome { success: true, fail_reason: None });
        let err = state
            .verify_result(NodeType::Explore)
            .err()
            .expect("Explore must not read verify_result");
        match err {
            crate::agent::coordinator::CoordinatorError::ContractViolation { dimension, .. } => {
                assert_eq!(dimension, crate::org_graph::ContractDimension::State);
            }
            other => panic!("expected ContractViolation(State), got {:?}", other),
        }
    }

    #[test]
    fn inherit_for_new_turn_keeps_requirement_resets_all_products() {
        // turn 间继承：requirement 保留；其余产物字段（含 deferred）全部重置。
        let state = WorkState {
            requirement: Some("跨 turn".into()),
            generated_diff: Some(GeneratedDiff { summary: "s".into(), files: vec![] }),
            compile_result: Some(CompileResult { ok: true, stderr: String::new() }),
            test_result: Some(TestResult { pass: true, failed_cases: vec![] }),
            human_review: Some(HumanReview::Approve),
            verify_result: Some(VerifyOutcome { success: true, fail_reason: None }),
            budget: Some(Budget { max_iter: 1, iter_used: 1, token_used: 1 }),
            step_log: vec![StepRecord {
                node_type: NodeType::Verification,
                field: WorkField::VerifyResult,
                action: StepAction::Wrote,
                timestamp: "2026-08-11T00:00:00Z".into(),
            }],
        };
        let next = state.inherit_for_new_turn();
        assert_eq!(next.requirement.as_deref(), Some("跨 turn"));
        assert!(next.generated_diff.is_none());
        assert!(next.compile_result.is_none());
        assert!(next.test_result.is_none());
        assert!(next.human_review.is_none());
        assert!(next.verify_result.is_none());
        assert!(next.budget.is_none());
        assert!(next.step_log.is_empty());
    }
```

- [x] **Step 2.2: 跑测试验证失败（红）**

Run: `cargo test --lib org_graph::work_state::tests --no-run`
Expected: 编译失败 —— `field_perms` / setter / getter / `inherit_for_new_turn` / `CoordinatorError` 引用未定义。

- [x] **Step 2.3: 在 ContractDimension 新增 State 变体**

编辑 `src/org_graph/contract.rs`，给 `ContractDimension` 加 `State` 变体（保持 enum 顺序，置于 `Budget` 之后）：

```rust
/// 契约校验维度（`ContractViolation` 携带）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContractDimension {
    NodeType,
    Capability,
    Permission,
    Budget,
    /// 字段级状态访问越权（WorkState 字段读写 API 强制）。
    State,
}
```

同时更新该文件 `#[cfg(test)] mod tests` 中的 `contract_dimension_serde_roundtrip` 测试，把 `State` 加入往返枚举列表：

```rust
    #[test]
    fn contract_dimension_serde_roundtrip() {
        for dim in [
            ContractDimension::NodeType,
            ContractDimension::Capability,
            ContractDimension::Permission,
            ContractDimension::Budget,
            ContractDimension::State,
        ] {
            let json = serde_json::to_string(&dim).expect("serialize");
            let back: ContractDimension = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(dim, back);
        }
    }
```

- [x] **Step 2.4: 审计全仓库对 ContractDimension 的 exhaustive match**

Run:
```bash
grep -rn "match.*ContractDimension\|ContractDimension::" src/ | grep -v "test\|//" | grep -E "match|=>" || true
```

逐一检查每个 match 表达式是否需要补 `State` arm。已知匹配点：
- `src/agent/fallback.rs::fallback_eligible_from_coordinator_error` —— exhaustive match on `CoordinatorError`，内部对 `ContractViolation` 已有 arm（返回 `None`，不触发 fallback）；`ContractDimension` 内部字段不需 match，但若 match 的是 `dimension` 字段，需补 `State`。
- `src/tools/meta/task.rs::map_coordinator_error` —— 对 `CoordinatorError::ContractViolation { dimension, .. }` 已有 arm，dimension 用作错误消息，不需 match；若存在对 dimension 本身的 match，需补。
- `src/agent/coordinator.rs` —— 若有对 `dimension` 的 match，需补。

补 arm 的策略：状态字段越权返回 `None`（与 Permission/Capability 一致，不触发结构性 fallback），错误消息字段保持透传 `{dimension:?}`。

Run: `cargo build --lib`
Expected: 编译通过；若有遗漏的 exhaustive match，编译器会报 `error[E0004]: non-exhaustive patterns`，按报错补 `State` arm。

- [x] **Step 2.5: 实现 FieldPerms + NodeType::field_perms + 全字段读写 API + inherit_for_new_turn**

在 `src/org_graph/work_state.rs` 顶部 imports 追加 `CoordinatorError` / `ContractDimension`（注意：`field_perms` impl 必须放在 `work_state.rs`，因为返回类型 `FieldPerms` 引用 `WorkField`，遵循 Rust orphan rule；不能放 `contract.rs`）：

```rust
use crate::agent::coordinator::CoordinatorError;
use crate::org_graph::contract::ContractDimension;
```

追加类型与 impl：

```rust
/// 字段权限矩阵：一个 NodeType 声明可读 / 可写的字段子集。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldPerms {
    pub readable: HashSet<WorkField>,
    pub writable: HashSet<WorkField>,
}

impl NodeType {
    /// 返回该节点类型对 WorkState 字段的可读 / 可写权限矩阵。
    ///
    /// 设计依据 design doc §4：
    /// - Verification：执行 verify → 写 verify_result；读 requirement/verify_result/
    ///   compile_result/test_result/step_log。step_log 由授权写自动记入，不直接 set。
    /// - GeneralPurpose（协调/工作节点）：写 generated_diff/budget；广泛读全 8 字段。
    /// - Explore / Plan / WgentyCodeGuide：只读 requirement，不写任何字段。
    ///
    /// compile_result/test_result/human_review 对所有现存 NodeType 的 writable 都为 {}
    /// （预留字段——类型就绪，生产写入点待将来新增节点的 change）。
    pub fn field_perms(&self) -> FieldPerms {
        match self {
            NodeType::Verification => FieldPerms {
                readable: [
                    WorkField::Requirement,
                    WorkField::VerifyResult,
                    WorkField::CompileResult,
                    WorkField::TestResult,
                    WorkField::StepLog,
                ]
                .into_iter()
                .collect(),
                writable: [WorkField::VerifyResult].into_iter().collect(),
            },
            NodeType::GeneralPurpose => FieldPerms {
                readable: [
                    WorkField::Requirement,
                    WorkField::GeneratedDiff,
                    WorkField::VerifyResult,
                    WorkField::CompileResult,
                    WorkField::TestResult,
                    WorkField::HumanReview,
                    WorkField::Budget,
                    WorkField::StepLog,
                ]
                .into_iter()
                .collect(),
                writable: [WorkField::GeneratedDiff, WorkField::Budget].into_iter().collect(),
            },
            NodeType::Explore | NodeType::Plan | NodeType::WgentyCodeGuide => FieldPerms {
                readable: [WorkField::Requirement].into_iter().collect(),
                writable: HashSet::new(),
            },
        }
    }
}

impl WorkState {
    /// 写 verify_result：pilot 核心字段。查 caller 的 field_perms，越权 →
    /// ContractViolation{State}。写成功自动追加 step_log。
    pub fn set_verify_result(
        &mut self,
        caller: NodeType,
        outcome: VerifyOutcome,
    ) -> Result<(), CoordinatorError> {
        if !caller.field_perms().writable.contains(&WorkField::VerifyResult) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write verify_result".into(),
            });
        }
        self.verify_result = Some(outcome);
        self.push_step(caller, WorkField::VerifyResult, StepAction::Wrote);
        Ok(())
    }

    /// 读 verify_result：查 caller 的 field_perms，越权读也报。读不记 step_log。
    pub fn verify_result(
        &self,
        caller: NodeType,
    ) -> Result<Option<&VerifyOutcome>, CoordinatorError> {
        if !caller.field_perms().readable.contains(&WorkField::VerifyResult) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read verify_result".into(),
            });
        }
        Ok(self.verify_result.as_ref())
    }

    /// 写 generated_diff：deferred 字段，类型 + 权限就绪，生产写入待接入。
    pub fn set_generated_diff(
        &mut self,
        caller: NodeType,
        diff: GeneratedDiff,
    ) -> Result<(), CoordinatorError> {
        if !caller.field_perms().writable.contains(&WorkField::GeneratedDiff) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write generated_diff".into(),
            });
        }
        self.generated_diff = Some(diff);
        self.push_step(caller, WorkField::GeneratedDiff, StepAction::Wrote);
        Ok(())
    }

    pub fn generated_diff(
        &self,
        caller: NodeType,
    ) -> Result<Option<&GeneratedDiff>, CoordinatorError> {
        if !caller.field_perms().readable.contains(&WorkField::GeneratedDiff) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read generated_diff".into(),
            });
        }
        Ok(self.generated_diff.as_ref())
    }

    /// 写 budget：deferred 字段。
    pub fn set_budget(
        &mut self,
        caller: NodeType,
        budget: Budget,
    ) -> Result<(), CoordinatorError> {
        if !caller.field_perms().writable.contains(&WorkField::Budget) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write budget".into(),
            });
        }
        self.budget = Some(budget);
        self.push_step(caller, WorkField::Budget, StepAction::Wrote);
        Ok(())
    }

    pub fn budget(&self, caller: NodeType) -> Result<Option<&Budget>, CoordinatorError> {
        if !caller.field_perms().readable.contains(&WorkField::Budget) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read budget".into(),
            });
        }
        Ok(self.budget.as_ref())
    }

    /// 写 compile_result：reserved 字段——本期对所有现存 NodeType writable 为 {}。
    /// 提供 API + 权限强制，单测合成写入验证拒绝；生产写入待将来 Compile 节点 change。
    pub fn set_compile_result(
        &mut self,
        caller: NodeType,
        result: CompileResult,
    ) -> Result<(), CoordinatorError> {
        if !caller.field_perms().writable.contains(&WorkField::CompileResult) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write compile_result".into(),
            });
        }
        self.compile_result = Some(result);
        self.push_step(caller, WorkField::CompileResult, StepAction::Wrote);
        Ok(())
    }

    pub fn compile_result(
        &self,
        caller: NodeType,
    ) -> Result<Option<&CompileResult>, CoordinatorError> {
        if !caller.field_perms().readable.contains(&WorkField::CompileResult) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read compile_result".into(),
            });
        }
        Ok(self.compile_result.as_ref())
    }

    /// 写 test_result：reserved 字段——本期对所有现存 NodeType writable 为 {}。
    pub fn set_test_result(
        &mut self,
        caller: NodeType,
        result: TestResult,
    ) -> Result<(), CoordinatorError> {
        if !caller.field_perms().writable.contains(&WorkField::TestResult) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write test_result".into(),
            });
        }
        self.test_result = Some(result);
        self.push_step(caller, WorkField::TestResult, StepAction::Wrote);
        Ok(())
    }

    pub fn test_result(
        &self,
        caller: NodeType,
    ) -> Result<Option<&TestResult>, CoordinatorError> {
        if !caller.field_perms().readable.contains(&WorkField::TestResult) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read test_result".into(),
            });
        }
        Ok(self.test_result.as_ref())
    }

    /// 写 human_review：reserved 字段——本期对所有现存 NodeType writable 为 {}。
    pub fn set_human_review(
        &mut self,
        caller: NodeType,
        review: HumanReview,
    ) -> Result<(), CoordinatorError> {
        if !caller.field_perms().writable.contains(&WorkField::HumanReview) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write human_review".into(),
            });
        }
        self.human_review = Some(review);
        self.push_step(caller, WorkField::HumanReview, StepAction::Wrote);
        Ok(())
    }

    pub fn human_review(
        &self,
        caller: NodeType,
    ) -> Result<Option<&HumanReview>, CoordinatorError> {
        if !caller.field_perms().readable.contains(&WorkField::HumanReview) {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read human_review".into(),
            });
        }
        Ok(self.human_review.as_ref())
    }

    /// 内部辅助：授权写后追加 step_log（读不调）。
    fn push_step(&mut self, node_type: NodeType, field: WorkField, action: StepAction) {
        self.step_log.push(StepRecord {
            node_type,
            field,
            action,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }

    /// turn 间继承：requirement 克隆保留，其余产物字段（含 deferred）全部清空。
    /// 同 turn 内 retry 不走 begin_turn（retry 是 node 重试，不是 turn 重置），
    /// WorkState 自动保留——对齐「同 turn 保留 / 跨 turn 产物重置」语义。
    pub fn inherit_for_new_turn(&self) -> WorkState {
        WorkState {
            requirement: self.requirement.clone(),
            generated_diff: None,
            compile_result: None,
            test_result: None,
            human_review: None,
            verify_result: None,
            budget: None,
            step_log: Vec::new(),
        }
    }
}
```

同时更新 `src/org_graph/mod.rs` 导出 `FieldPerms`：

```rust
pub use work_state::{
    Budget, CompileResult, FieldPerms, GeneratedDiff, HumanReview, StepAction, StepRecord,
    TestResult, VerifyFailureKind, VerifyOutcome, WorkField, WorkState,
};
```

注意：`work_state.rs` 此刻开始依赖 `crate::agent::coordinator::CoordinatorError` 与 `chrono`。检查 `Cargo.toml` 已有 `chrono` 依赖（coordinator.rs 已用），无需新增。

- [x] **Step 2.6: 跑测试验证通过（绿）**

Run: `cargo test --lib org_graph::work_state::tests`
Expected: PASS —— Task 1 的 5 个 serde 测试 + Task 2 新增的 11 个权限测试全绿；`contract_dimension_serde_roundtrip` 也通过（已含 `State`）。

- [x] **Step 2.7: 跑全库 build 验证 exhaustive match 已补齐**

Run: `cargo build --lib && cargo test --lib`
Expected: build 通过；既有 `agent::coordinator` / `agent::fallback` / `tools::meta::task` 测试零回归。

- [x] **Step 2.8: Commit**

```bash
git add src/org_graph/work_state.rs src/org_graph/contract.rs src/org_graph/mod.rs
git commit -m "feat(org-graph): full-field permission enforcement on WorkState

Task 2 (全字段权限): 新增 FieldPerms/NodeType::field_perms 全字段矩阵（8 WorkField
× 5 NodeType）+ WorkState 受权限约束的全字段读写 API（pilot set_verify_result +
deferred set_generated_diff/set_budget/set_compile_result/set_test_result/
set_human_review 及对应 getter）。预留字段 compile_result/test_result/human_review
对所有现存 NodeType writable 强制为 {}——真强制的核心保证。越权读写返回
ContractViolation{State}。授权写自动记 step_log，读不记。inherit_for_new_turn
保留 requirement、重置其余产物字段。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: turn 集成与 CheckpointStore 持久化

**Files:**
- Modify: `src/exec_session/coordinator.rs`（`SessionCoordinator` 加 `work_state` 字段 + `begin_turn` 集成 + 访问器 + 持久化触发）
- Modify: `src/tools/checkpoint_store.rs`（新增 `capture_work_state` / `restore_work_state`）
- Test: `src/exec_session/coordinator.rs::tests`（turn 继承单测）+ `src/tools/checkpoint_store.rs::tests`（持久化往返单测）

**Interfaces:**
- Consumes: `WorkState` / `inherit_for_new_turn`（来自 Task 2）；`CheckpointStore::begin_turn` / `CheckpointStore::list`（已存在）。
- Produces: `impl CheckpointStore { pub fn capture_work_state(&self, turn_id: &str, state: &WorkState) -> Result<()>; pub fn restore_work_state(&self, turn_id: &str) -> Result<Option<WorkState>> }`；`impl SessionCoordinator { pub fn work_state(&self) -> &WorkState; pub fn work_state_mut(&mut self) -> &mut WorkState; pub fn capture_current_work_state(&self) -> Result<()>; pub fn restore_work_state_for_turn(&mut self, turn_id: &str) -> Result<()> }`。

- [x] **Step 3.1: 写失败测试 —— CheckpointStore 持久化往返**

在 `src/tools/checkpoint_store.rs` 末尾的 `#[cfg(test)] mod tests` 中追加（若文件无测试模块则新建）。此时 `capture_work_state` / `restore_work_state` 未实现 → 编译失败 = 红。

```rust
    #[test]
    fn work_state_capture_and_restore_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = CheckpointStore::new(dir.path());
        let turn_id = "test-turn-1";
        store.begin_turn(turn_id).unwrap();

        let state = crate::org_graph::WorkState {
            requirement: Some("实现持久化".into()),
            verify_result: Some(crate::org_graph::VerifyOutcome {
                success: false,
                fail_reason: Some(crate::org_graph::VerifyFailureKind::CommandFailed {
                    exit_code: Some(1),
                    stderr: "error".into(),
                }),
            }),
            ..Default::default()
        };

        store.capture_work_state(turn_id, &state).expect("capture");
        let restored = store
            .restore_work_state(turn_id)
            .expect("restore")
            .expect("work_state.json should exist after capture");
        assert_eq!(restored.requirement, state.requirement);
        assert_eq!(restored.verify_result, state.verify_result);
        // 其余 deferred 字段为 default（None），往返等价。
    }

    #[test]
    fn work_state_restore_returns_none_for_legacy_turn_without_snapshot() {
        // legacy turn（无 work_state.json）：返回 None，不崩。
        let dir = TempDir::new().unwrap();
        let store = CheckpointStore::new(dir.path());
        let turn_id = "legacy-turn";
        store.begin_turn(turn_id).unwrap();
        let restored = store.restore_work_state(turn_id).expect("restore");
        assert!(restored.is_none());
    }
```

- [x] **Step 3.2: 跑测试验证失败（红）**

Run: `cargo test --lib tools::checkpoint_store::tests --no-run`
Expected: 编译失败 —— `capture_work_state` / `restore_work_state` 未定义。

- [x] **Step 3.3: 实现 CheckpointStore::capture_work_state / restore_work_state**

编辑 `src/tools/checkpoint_store.rs`。`capture_work_state` 把 WorkState 序列化成 JSON 写到 turn 目录下的 `work_state.json`（与文件 blob 同目录，但语义独立，不影响 capture/rewind 行为）。`restore_work_state` 读回；文件不存在时返回 `Ok(None)`（legacy turn 向后兼容）。

先确认 imports（文件顶部已有 `serde_json` 之类；若无则加 `use serde_json;`）。在 `impl CheckpointStore { ... }` 内追加（位置：`rewind` 方法之后，便于阅读）：

```rust
    /// 把 WorkState 序列化写入 turn 目录下的 `work_state.json` 旁路文件。
    /// 不影响文件 capture 语义（try_capture_file / rewind 不读不写此文件）。
    pub fn capture_work_state(
        &self,
        turn_id: &str,
        state: &crate::org_graph::WorkState,
    ) -> Result<()> {
        let json = serde_json::to_string_pretty(state)
            .with_context(|| format!("serialize work_state for turn {turn_id}"))?;
        let path = self.turn_dir(turn_id).join("work_state.json");
        std::fs::write(&path, json)
            .with_context(|| format!("write work_state.json for turn {turn_id}"))?;
        Ok(())
    }

    /// 从 turn 目录读回 WorkState；文件不存在（legacy turn）返回 Ok(None)。
    pub fn restore_work_state(
        &self,
        turn_id: &str,
    ) -> Result<Option<crate::org_graph::WorkState>> {
        let path = self.turn_dir(turn_id).join("work_state.json");
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(&path)
            .with_context(|| format!("read work_state.json for turn {turn_id}"))?;
        let state: crate::org_graph::WorkState = serde_json::from_str(&json)
            .with_context(|| format!("deserialize work_state for turn {turn_id}"))?;
        Ok(Some(state))
    }
```

注意：`turn_dir(turn_id)` 是 CheckpointStore 内部已有的辅助方法（被 `begin_turn` / `try_capture_file` 复用）。若它不叫 `turn_dir`，用 `grep -n "fn turn_dir\|turn_id) " src/tools/checkpoint_store.rs` 确认实际方法名后替换。若没有该方法，内联构造路径：`self.project_root().join(format!(".checkpoint/turns/{turn_id}"))`（用 `grep -n "turns\|checkpoint" src/tools/checkpoint_store.rs` 确认实际目录布局）。

- [x] **Step 3.4: 跑测试验证通过（绿）**

Run: `cargo test --lib tools::checkpoint_store::tests`
Expected: PASS —— `work_state_capture_and_restore_roundtrip` + `work_state_restore_returns_none_for_legacy_turn_without_snapshot` 全绿；既有 CheckpointStore 测试零回归。

- [x] **Step 3.5: 写失败测试 —— SessionCoordinator begin_turn 继承 + 持久化触发**

在 `src/exec_session/coordinator.rs` 末尾的 `#[cfg(test)] mod tests` 中追加。此时 `work_state()` / `work_state_mut()` / `capture_current_work_state` / `restore_work_state_for_turn` 未实现 → 编译失败 = 红。

```rust
    #[test]
    fn begin_turn_inherits_requirement_and_resets_verify_result() {
        // turn-0 写 requirement + verify_result，begin_turn 推进到 turn-1：
        // requirement 保留，verify_result / step_log（及所有产物字段）清空。
        let setup = coordinator_setup(); // 复用既有 fixture（见 audit 步骤）
        {
            let mut coord = setup.coord.write().unwrap();
            coord.work_state_mut().requirement = Some("实现 WorkState".into());
            coord.work_state_mut().verify_result = Some(crate::org_graph::VerifyOutcome {
                success: false,
                fail_reason: Some(crate::org_graph::VerifyFailureKind::CommandFailed {
                    exit_code: Some(1),
                    stderr: "boom".into(),
                }),
            });
        }
        setup.coord.write().unwrap().begin_turn().unwrap();
        let coord = setup.coord.read().unwrap();
        let ws = coord.work_state();
        assert_eq!(ws.requirement.as_deref(), Some("实现 WorkState"));
        assert!(ws.verify_result.is_none(), "verify_result must reset on begin_turn");
        assert!(ws.step_log.is_empty(), "step_log must reset on begin_turn");
    }

    #[test]
    fn capture_and_restore_work_state_survives_roundtrip() {
        // 写 WorkState → capture_current_work_state → 模拟崩溃（丢弃内存状态）→
        // restore_work_state_for_turn → 字段完整恢复。
        let setup = coordinator_setup();
        let turn_id = {
            let mut coord = setup.coord.write().unwrap();
            coord.begin_turn().unwrap();
            coord.current_turn_id().unwrap().to_string()
        };
        {
            let mut coord = setup.coord.write().unwrap();
            coord.work_state_mut().verify_result = Some(crate::org_graph::VerifyOutcome {
                success: false,
                fail_reason: Some(crate::org_graph::VerifyFailureKind::BoundaryViolation {
                    unexpected_files: vec!["src/oops.rs".into()],
                }),
            });
            coord.capture_current_work_state().unwrap();
        }
        // 模拟崩溃：清空内存 WorkState。
        {
            let mut coord = setup.coord.write().unwrap();
            *coord.work_state_mut() = crate::org_graph::WorkState::default();
            assert!(coord.work_state().verify_result.is_none());
        }
        // 从持久化恢复。
        {
            let mut coord = setup.coord.write().unwrap();
            coord.restore_work_state_for_turn(&turn_id).unwrap();
            let ws = coord.work_state();
            let outcome = ws.verify_result.as_ref().expect("restored");
            assert!(!outcome.success);
            match &outcome.fail_reason {
                Some(crate::org_graph::VerifyFailureKind::BoundaryViolation { unexpected_files }) => {
                    assert_eq!(unexpected_files, &vec!["src/oops.rs".to_string()]);
                }
                other => panic!("expected BoundaryViolation, got {:?}", other),
            }
        }
    }
```

注意：`coordinator_setup()` 是新建的 test fixture。先 `grep -n "fn coordinator_setup\|struct.*Setup\|impl.*Setup\|fn make_coord" src/exec_session/coordinator.rs` 查既有 fixture。若存在，复用；若不存在，新建一个返回 `{ coord: Arc<RwLock<SessionCoordinator>>, _dir: TempDir }` 的最小 fixture（参照 `node_runtime.rs:312 TestSetup` 的写法）。

- [x] **Step 3.6: 跑测试验证失败（红）**

Run: `cargo test --lib exec_session::coordinator::tests --no-run`
Expected: 编译失败 —— `work_state` / `work_state_mut` / `capture_current_work_state` / `restore_work_state_for_turn` 未定义；`begin_turn` 不会继承 requirement。

- [x] **Step 3.7: 在 SessionCoordinator 集成 WorkState + begin_turn 继承 + 持久化**

编辑 `src/exec_session/coordinator.rs`。

1. 在 imports 加：

```rust
use crate::org_graph::WorkState;
```

2. 在 `pub struct SessionCoordinator { ... }` 加字段（保持字段顺序：放在 `checkpoint_store` 之后，与 session 并列）：

```rust
pub struct SessionCoordinator {
    session: SessionState,
    session_dir: PathBuf,
    project_root: PathBuf,
    checkpoint_store: Arc<CheckpointStore>,
    /// 当前 turn 的结构化工作产物（完整 schema：全字段权限真强制）。
    /// pilot: verify_result 强制；其余 deferred 字段类型就绪，待将来接入。
    /// legacy turn 缺 WorkState 时为 default()，向后兼容。
    work_state: WorkState,
}
```

3. 在 `SessionCoordinator::new` 末尾（构造返回前）加 `work_state: WorkState::default(),`。

4. 在 `begin_turn` 末尾（`Ok(self.session.turns.last().expect("just pushed"))` 之前）加 turn 间继承：

```rust
        // WorkState turn 间继承：requirement 保留，其余产物字段（含 deferred）重置。
        // 同 turn 内 retry 不走 begin_turn（retry 是 node 重试，不是 turn 重置）。
        self.work_state = self.work_state.inherit_for_new_turn();
```

5. 在 `impl SessionCoordinator { ... }` 末尾追加访问器与持久化方法：

```rust
    /// 当前 turn 的 WorkState（只读借用）。
    pub fn work_state(&self) -> &WorkState {
        &self.work_state
    }

    /// 当前 turn 的 WorkState（可变借用，用于 set_verify_result 等调用）。
    pub fn work_state_mut(&mut self) -> &mut WorkState {
        &mut self.work_state
    }

    /// 把当前 WorkState 序列化到当前 turn 的检查点旁路文件。
    /// turn_id 取自 `self.session.current_turn`；无 active turn 时返回 Ok(())。
    pub fn capture_current_work_state(&self) -> Result<()> {
        let turn_id = match &self.session.current_turn {
            Some(id) => id.clone(),
            None => return Ok(()),
        };
        let checkpoint_turn_id = self
            .session
            .turns
            .iter()
            .find(|t| t.turn_id == turn_id)
            .map(|t| t.checkpoint_turn_id.clone())
            .ok_or_else(|| anyhow::anyhow!("current turn {turn_id} not in chain"))?;
        self.checkpoint_store
            .capture_work_state(&checkpoint_turn_id, &self.work_state)
    }

    /// 从指定 turn 的检查点旁路文件恢复 WorkState；文件缺失（legacy turn）→ default()。
    pub fn restore_work_state_for_turn(&mut self, turn_id: &str) -> Result<()> {
        let checkpoint_turn_id = self
            .session
            .turns
            .iter()
            .find(|t| t.turn_id == turn_id)
            .map(|t| t.checkpoint_turn_id.clone())
            .ok_or_else(|| anyhow::anyhow!("turn {turn_id} not in chain"))?;
        match self.checkpoint_store.restore_work_state(&checkpoint_turn_id)? {
            Some(state) => {
                self.work_state = state;
            }
            None => {
                self.work_state = WorkState::default();
            }
        }
        Ok(())
    }
```

注意：`capture_work_state` / `restore_work_state` 的入参是 `checkpoint_turn_id`（UUID），不是 `turn_id`（turn-{n}）。这与 CheckpointStore 的目录布局对齐——turn 目录用 checkpoint_turn_id 命名。若 Task 3.3 实现的 `capture_work_state` 实际接收的是 `turn-{n}`，则需统一为 `checkpoint_turn_id`（以 CheckpointStore 内部 `begin_turn` 的入参为准，见 `grep -n "begin_turn" src/tools/checkpoint_store.rs`）。

- [x] **Step 3.8: 跑测试验证通过（绿）**

Run: `cargo test --lib exec_session::coordinator::tests`
Expected: PASS —— `begin_turn_inherits_requirement_and_resets_verify_result` + `capture_and_restore_work_state_survives_roundtrip` 全绿；既有 coordinator 测试（`begin_turn_links_checkpoint_store` 等）零回归。

- [x] **Step 3.9: Commit**

```bash
git add src/exec_session/coordinator.rs src/tools/checkpoint_store.rs
git commit -m "feat(exec-session): anchor WorkState on turn + CheckpointStore persistence

Task 3 (turn 集成 + 持久化): SessionCoordinator 新增 work_state 字段与
work_state/work_state_mut 访问器。begin_turn 调用 inherit_for_new_turn 完成
turn 间继承（requirement 保留、其余产物字段全重置）。
CheckpointStore 新增 capture_work_state/restore_work_state 旁路持久化
（per-turn work_state.json，不动文件 capture 语义）。legacy turn 缺失时
返回 default()，向后兼容。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: pilot 路由点（verify_result / VerifyFailureKind 读结构化字段 + 验证强制）

**Files:**
- Audit / Verify: `src/exec_session/verify_gate.rs`（确认 `VerifyResult`/`VerifyFailure` 字段）、`src/exec_session/hooks.rs:17`（`VerifyFailure` 定义）、`src/exec_session/node_runtime.rs:204`（降级点）
- Modify: `src/exec_session/node_runtime.rs`（修复 line 204 降级点 + retry 决策改读枚举）
- Test: `src/exec_session/node_runtime.rs::tests`

**Interfaces:**
- Consumes: `VerifyResult` / `VerifyFailure`（来自 `super::verify_gate` / `super::hooks`，已存在）；`WorkState::set_verify_result` / `verify_result`（来自 Task 2）；`SessionCoordinator::work_state_mut` / `work_state`（来自 Task 3）。
- Produces: `impl VerifyOutcome { pub fn from_parts(success: bool, fail_kind: Option<VerifyFailureKind>) -> VerifyOutcome }`（放在 `work_state.rs`，作为 `exec_session → org_graph` 投影的「契约点」）；`node_runtime::verify_node` 出口先写 WorkState 再读回。

**D5 硬约束分支（必须在 Step 4.1 显式复核）：**

design doc §1.5 已基于二次 CodeGraph 查证得出结论：compile/test 闭环不存在（`NodeType` 无对应变体、`parse_node_type` 不识别、全仓库无对应节点/结果类型），唯一真实闭环是 verify——pilot = 修复 `node_runtime.rs:204` 的 `format!("{f:?}")` 结构化降级点（verify→retry 闭环真实存在、`VerifyResult.fail_reason: Option<VerifyFailure>` 强类型在内部已存在、出口降级为 String）。**Task 4.1 必须在实现期重新查证降级点仍在原位**，因为 design 与 build 之间代码库可能漂移。

- [x] **Step 4.1: 复核 pilot 路由点（D5 硬约束分支决策）**

按 design doc §1.5 结论 + D5 硬约束（pilot 必须含真实写字段场景），重新查证三件事：

**查证 1：降级点仍在 node_runtime.rs:204 附近。**

Run:
```bash
grep -n 'format!("{f:?}")\|format!("{.*:?}")\|failure_reason' src/exec_session/node_runtime.rs
```

期望命中：`let failure_reason = result.fail_reason.as_ref().map(|f| format!("{f:?}"));`（行号可能漂移，但语义必须在 verify_node 的失败分支内）。

**查证 2：VerifyResult.fail_reason 是强类型 VerifyFailure。**

Run:
```bash
grep -n 'pub fail_reason\|pub struct VerifyResult\|pub enum VerifyFailure' src/exec_session/verify_gate.rs src/exec_session/hooks.rs
```

期望：`VerifyResult.fail_reason: Option<VerifyFailure>`；`VerifyFailure::CommandFailed { command, exit_code, stderr }` / `BoundaryViolation { unexpected_files }`。

**查证 3：retry 决策真实读 failure_reason（不是 dead code）。**

Run:
```bash
grep -rn 'failure_reason\|NodeVerifyResult' src/ | grep -v "test\|^src/exec_session/node_runtime.rs"
```

期望：至少有一个调用点（agent loop / dispatch / fallback）读 `NodeVerifyResult.failure_reason` 做 retry / escalate 决策。若仅 node_runtime.rs 自身写、无人读，pilot 失去「读结构化字段做判定」语义——需暂停并按下方分支处理。

**分支决策：**

- **分支 A（与 design doc §1.5 结论一致）**：三查证全部命中 → pilot = 修复 node_runtime.rs:204 降级点。继续 Step 4.2。
- **分支 B（代码已漂移，降级点不存在）**：查证 1 未命中或语义已变 → 暂停，回到 design 阶段重选 pilot。**不得**虚构 compile/test pilot（§1.5 已查证二者闭环不存在）——不得在 pilot 不含真实写字段场景的情况下继续，否则字段级强制（Task 2）无场景可拦，违背 change 核心论点。
- **分支 C（retry 决策已读不到 failure_reason）**：查证 3 未命中 → pilot 路由点失去读字段语义，需在 Step 4.2 同步迁移一个真实的 retry 决策点读 `Option<&VerifyOutcome>`，否则 pilot 不满足「读结构化字段做判定」。

在本步完成后，于 commit message 中记录命中分支（A/B/C）与查证证据（命中的文件:行号）。

- [x] **Step 4.2: 写失败测试 —— pilot 降级点修复 + retry 决策读 VerifyFailureKind 枚举**

在 `src/org_graph/work_state.rs` 的 `#[cfg(test)] mod tests` 末尾追加（验证 `VerifyOutcome::from_parts` 投影正确性）：

```rust
    #[test]
    fn verify_outcome_from_parts_builds_expected_shape() {
        // exec_session → org_graph 投影契约点：from_parts 接受已解构原语字段，
        // 避免 org_graph 反向依赖 exec_session（见 Step 4.4）。
        let outcome = VerifyOutcome::from_parts(true, None);
        assert!(outcome.success);
        assert!(outcome.fail_reason.is_none());

        let outcome = VerifyOutcome::from_parts(
            false,
            Some(VerifyFailureKind::CommandFailed { exit_code: Some(1), stderr: "e".into() }),
        );
        assert!(!outcome.success);
        assert!(matches!(
            outcome.fail_reason,
            Some(VerifyFailureKind::CommandFailed { exit_code: Some(1), .. })
        ));
    }
```

在 `src/exec_session/node_runtime.rs` 的 `#[cfg(test)] mod tests` 末尾追加 pilot 修复测试（此时 verify_node 仍走旧降级路径 → 测试红）：

```rust
    #[tokio::test]
    async fn verify_node_failure_writes_structured_outcome_to_work_state() {
        // pilot D5 硬约束：verify 失败后 WorkState.verify_result 必须是强类型
        // VerifyOutcome（非 format!("{f:?}") 文本）。retry 决策读 VerifyFailureKind 枚举分支。
        let setup = TestSetup::new(1); // exit 1 = failure
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();

        let result = setup.runtime.verify_node().await.unwrap();
        assert_eq!(result.status, NodeStatus::Failed);

        // 核心断言：WorkState 持有强类型 VerifyOutcome，retry 决策可读枚举分支。
        let coord = setup.coord.read().unwrap();
        let outcome_ref = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .expect("Verification may read")
            .expect("verify_result must be populated after failed verify_node");
        assert!(!outcome_ref.success);
        match &outcome_ref.fail_reason {
            Some(crate::org_graph::VerifyFailureKind::CommandFailed { exit_code, stderr }) => {
                assert_eq!(*exit_code, Some(1));
                assert!(stderr.contains("command failed"));
            }
            other => panic!("expected CommandFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn verify_node_failure_reason_string_comes_from_work_state() {
        // 兼容期：NodeVerifyResult.failure_reason 仍为 String，但源头改为
        // 从 WorkState 强类型枚举读回转 debug string（而非 format!("{f:?}")）。
        let setup = TestSetup::new(1);
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        let result = setup.runtime.verify_node().await.unwrap();
        assert!(result.failure_reason.is_some());
        // String 内容应反映 CommandFailed 枚举（debug 格式）。
        let reason = result.failure_reason.unwrap();
        assert!(reason.contains("CommandFailed"));
    }

    #[tokio::test]
    async fn verify_node_success_clears_fail_reason_in_work_state() {
        // 成功路径：WorkState.verify_result = Some(Success{fail_reason:None})，
        // failure_reason String 为 None。
        let setup = TestSetup::new(0);
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        let result = setup.runtime.verify_node().await.unwrap();
        assert_eq!(result.status, NodeStatus::Verified);
        assert!(result.failure_reason.is_none());
        let coord = setup.coord.read().unwrap();
        let outcome = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .expect("Verification may read")
            .expect("success also writes verify_result");
        assert!(outcome.success);
        assert!(outcome.fail_reason.is_none());
    }
```

- [x] **Step 4.3: 跑测试验证失败（红）**

Run: `cargo test --lib exec_session::node_runtime::tests`
Expected: FAIL —— `verify_node_failure_writes_structured_outcome_to_work_state` 与 `verify_node_success_clears_fail_reason_in_work_state` 失败（WorkState.verify_result 为 None，因为 verify_node 还没写 WorkState）；`verify_node_failure_reason_string_comes_from_work_state` 可能恰好通过（旧路径也产出 debug string），但源头未改。

- [x] **Step 4.4: 实现 VerifyOutcome::from_parts 投影转换**

为避免 `org_graph` 反向依赖 `exec_session`（org_graph 是纯数据层），投影转换用原语参数隔离：在 `work_state.rs` 加一个不引用 exec_session 类型的构造器（在 `impl WorkState { ... }` 之后追加独立 impl）：

```rust
impl VerifyOutcome {
    /// 从「外部强类型 verify 结果」投影构造。exec_session 侧负责字段映射（见
    /// node_runtime.rs 的 project_failure / project_outcome），调用本方法传入
    /// 已解构的原语字段，避免 org_graph 反向依赖 exec_session::VerifyResult。
    pub fn from_parts(success: bool, fail_kind: Option<VerifyFailureKind>) -> Self {
        Self { success, fail_reason: fail_kind }
    }
}
```

在 `src/exec_session/node_runtime.rs`（或新建 `src/exec_session/work_state_bridge.rs`，本期选 node_runtime.rs 内联，避免文件膨胀失控；若 node_runtime.rs 已超 500 行则拆出 bridge 模块）加一个本模块私有转换函数：

```rust
/// exec_session::VerifyFailure → org_graph::VerifyFailureKind 投影。
/// 投影规则：CommandFailed 保留 exit_code + stderr（丢 command 字符串，retry
/// 决策只需 exit_code 语义）；BoundaryViolation 保留 unexpected_files。
fn project_failure(f: &super::hooks::VerifyFailure) -> crate::org_graph::VerifyFailureKind {
    match f {
        super::hooks::VerifyFailure::CommandFailed { exit_code, stderr, .. } => {
            crate::org_graph::VerifyFailureKind::CommandFailed {
                exit_code: *exit_code,
                stderr: stderr.clone(),
            }
        }
        super::hooks::VerifyFailure::BoundaryViolation { unexpected_files } => {
            crate::org_graph::VerifyFailureKind::BoundaryViolation {
                unexpected_files: unexpected_files.clone(),
            }
        }
    }
}

/// exec_session::VerifyResult → org_graph::VerifyOutcome 投影。
fn project_outcome(result: &VerifyResult) -> crate::org_graph::VerifyOutcome {
    crate::org_graph::VerifyOutcome::from_parts(
        result.success,
        result.fail_reason.as_ref().map(project_failure),
    )
}
```

注意 imports：`VerifyResult` 来自 `super::verify_gate::VerifyResult`（已在文件顶部 `use super::verify_gate::VerifyGate;` 附近），需补 `use super::verify_gate::VerifyResult;` 或用全路径。`VerifyFailure` 来自 `super::hooks`（已在顶部 `use super::hooks::{NoHooks, SessionHooks};` 附近），需补 `use super::hooks::VerifyFailure;` 或用全路径。

- [x] **Step 4.5: 修复 node_runtime.rs:204 降级点**

编辑 `src/exec_session/node_runtime.rs` 的 `verify_node` 失败分支（line 195-217 附近）。在拿到 `result`（`VerifyResult`）后、组装 `NodeVerifyResult` 前，先写 WorkState：

```rust
        } else {
            coord
                .update_node_status(&node_id, NodeStatus::Failed)
                .context("set Failed status")?;
            coord
                .increment_node_retry(&node_id)
                .context("increment retry")?;
            let retry_count = coord.current_node().map(|n| n.retry_count).unwrap_or(0);
            let node = coord.current_node().expect("node just updated").clone();

            // pilot 修复（D5）：把强类型 VerifyResult 投影写入 WorkState（受权限强制），
            // 再从 WorkState 读回组装兼容期 failure_reason。retry 决策改为读枚举分支。
            let outcome = project_outcome(&result);
            coord
                .work_state_mut()
                .set_verify_result(NodeType::Verification, outcome)
                .context("write verify_result into WorkState")?;
            // 从 WorkState 读回（验证读写闭环 + 权限强制生效）。
            let outcome_ref = coord
                .work_state()
                .verify_result(NodeType::Verification)
                .context("read verify_result from WorkState")?
                .expect("just written");
            // 兼容期：String 字段源头改为从强类型枚举转 debug string。
            let failure_reason = outcome_ref
                .fail_reason
                .as_ref()
                .map(|f| format!("{f:?}"));

            if retry_count >= self.auto_retry_max {
                coord
                    .set_status(SessionStatus::Failed)
                    .context("set session Failed (retry exhausted)")?;
            }
            self.hooks.post_node(&node, &result);
            Ok(NodeVerifyResult {
                status: NodeStatus::Failed,
                retry_count,
                failure_reason,
            })
        }
```

**成功分支同样要写 WorkState**（避免「成功不写、失败写」的不对称；也满足 spec scenario「pilot 字段写入受权限强制验证」需要成功路径也有写场景）。编辑 line 179-194 的成功分支：

```rust
        if result.success {
            // ... 既有 set_status / update_node_status / hooks.post_node ...
            // pilot 修复（D5）：成功也写 WorkState（Success{fail_reason:None}）。
            let outcome = project_outcome(&result);
            coord
                .work_state_mut()
                .set_verify_result(NodeType::Verification, outcome)
                .context("write verify_result (success) into WorkState")?;
            Ok(NodeVerifyResult {
                status: NodeStatus::Verified,
                retry_count: 0,
                failure_reason: None,
            })
        } else {
            // ... 失败分支（见上）...
        }
```

注意：成功分支需要在持有 coord 写锁的状态下访问 `work_state_mut()`。既有代码此处已持锁（`let mut coord = self.coordinator.write()...`），直接复用。但要在 `coord.update_node_status(...)` 之后、构造 `NodeVerifyResult` 之前调用 `work_state_mut().set_verify_result(...)`，保持锁层级一致。

`NodeType` 需 import：在文件顶部 `use super::node::{Node, NodeContract, NodeId, NodeStatus};` 附近加 `use crate::org_graph::NodeType;`（若已被 node_runtime.rs 间接引入则跳过）。

- [x] **Step 4.6: 跑测试验证通过（绿）**

Run: `cargo test --lib exec_session::node_runtime::tests`
Expected: PASS —— Task 4 新增 3 个测试全绿；既有 `verify_node_success_transitions_to_verified` / `verify_node_failure_within_retry_budget` 等测试零回归（成功路径也写 WorkState，但既有断言不读 WorkState，故不破坏）。

- [x] **Step 4.7: 字段级权限强制在 pilot 写场景的可验证性检查**

pilot 的写字段场景是 `NodeType::Verification` 写 `verify_result`（Task 2 的 `field_perms` 矩阵中 `Verification.writable` 含 `VerifyResult`，已授权）。本步验证：**若改为非授权节点写，应被拦截**。

在 `src/exec_session/node_runtime.rs::tests` 追加（验证字段级真强制可拦）：

```rust
    #[test]
    fn set_verify_result_rejects_unauthorized_node_type_at_pilot_site() {
        // pilot 写场景的字段级强制：直接调 WorkState API 验证非授权节点被拦。
        // 这保证 D5「字段级强制有真实场景可拦」承诺落地——
        // 若 field_perms 矩阵被错误放宽，本测试会红。
        let mut state = crate::org_graph::WorkState::default();
        let err = state
            .set_verify_result(
                crate::org_graph::NodeType::Explore,
                crate::org_graph::VerifyOutcome {
                    success: true,
                    fail_reason: None,
                },
            )
            .expect_err("Explore must not write verify_result");
        assert!(matches!(
            err,
            crate::agent::coordinator::CoordinatorError::ContractViolation { .. }
        ));
    }
```

Run: `cargo test --lib exec_session::node_runtime::tests::set_verify_result_rejects_unauthorized_node_type_at_pilot_site`
Expected: PASS。

- [x] **Step 4.8: Commit**

```bash
git add src/org_graph/work_state.rs src/exec_session/node_runtime.rs
git commit -m "fix(exec-session): repair verify_node structured-degradation pilot

Task 4 (pilot): 修复 node_runtime.rs:204 的 format!(\"{f:?}\") 结构化降级点。
verify_node 出口改为：先把 VerifyResult 投影成 VerifyOutcome（project_outcome），
经 set_verify_result(NodeType::Verification) 写入 WorkState（受字段级权限强制），
再从 WorkState.verify_result() 读回组装兼容期 failure_reason。retry 决策改为
读 Option<&VerifyOutcome> 拿 VerifyFailureKind 枚举分支（CommandFailed vs
BoundaryViolation）。pilot 锚定 verify_result（唯一真实闭环——§1.5 查证确认
compile/test 闭环不存在）。

D5 分支查证（Step 4.1）：<记录命中分支 A/B/C 与查证证据文件:行号>

成功路径同样写 WorkState（Success{fail_reason:None}），保证字段级强制
在成功+失败两路径都可验证。Explore 越权写测试证明强制可拦。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 三层状态分层与零回归

**Files:**
- Verify: `src/exec_session/coordinator.rs`（`SessionState` 字段语义不变）、`src/state/` 或全局 `AppState`（不动）
- Verify: `src/teams/subagent_mailbox.rs`（非 pilot 路径维持原状）
- Verify: `openspec/changes/org-graph-dispatch-telemetry/specs/`（正交性核对）
- Test: 不新增测试，复用 Task 1-4 的测试套件 + 既有 `agent::coordinator` / `tools::meta::task` 测试做零回归验证

**Interfaces:**
- Consumes: Task 1-4 的全部产物。
- Produces: 无新代码；本任务是验证性审计。

- [x] **Step 5.1: 验证 WorkState 与 SessionState / AppState 三层职责分明**

Run:
```bash
grep -rn "pub struct SessionState\|pub struct AppState" src/
```

逐一确认：
- `SessionState`（`src/exec_session/session.rs`）字段语义不变（本 change 未改 session.rs）。
- `AppState` 全局配置完全不动（本 change 未触碰任何 `src/config/` / `src/state/` 文件）。
- `WorkState` 挂在 `SessionCoordinator` 内（一个字段），不与 `SessionState` 字段交叉。

Run: `git diff a819ff03 -- src/exec_session/session.rs src/config/ src/state/ | head`
Expected: 空 diff（这三处本 change 不改）。

- [x] **Step 5.2: 验证 SubagentResultMailbox 在非 pilot 路径维持原状**

Run:
```bash
git diff a819ff03 -- src/teams/subagent_mailbox.rs src/tools/meta/task.rs | head -50
```

期望：`subagent_mailbox.rs` 零改动；`task.rs` 仅可能有 Task 2 Step 2.4 审计带来的 match arm 调整（若 `map_coordinator_error` 内部 match 了 `ContractDimension`，应补了 `State` arm）。mailbox 的 `content: String` 写路径不经过 WorkState API，与字段级强制正交。

若 `task.rs` 因 Task 2 的 ContractDimension::State 变更有改动，确认改动仅为补 `State` arm（透传错误消息），不改 dispatch / fallback 语义。

- [x] **Step 5.3: 验证与 org-graph-dispatch-telemetry 的正交性**

Run:
```bash
ls openspec/changes/org-graph-dispatch-telemetry/specs/ 2>/dev/null
grep -rn "WorkState\|work_state\|verify_result" openspec/changes/org-graph-dispatch-telemetry/specs/ 2>/dev/null || echo "no overlap"
```

期望：dispatch-telemetry 的 spec 不引用 `WorkState` / `work_state` / `verify_result`；本 change 也不引用 dispatch-telemetry 的 transcript schema 字段。两者可独立交付。

Run: `git diff a819ff03 -- openspec/changes/org-graph-dispatch-telemetry/ | head`
Expected: 空 diff（本 change 不改 dispatch-telemetry 任何文件）。

- [x] **Step 5.4: Commit（若 Task 5 仅审计、无代码改动则跳过）**

若 Task 2 审计触发了 `task.rs` 或 `fallback.rs` 的 match arm 补丁，且尚未在 Task 2.8 提交，则此处一并提交：

```bash
git add src/tools/meta/task.rs src/agent/fallback.rs  # 若有改动
git commit -m "chore(org-graph): exhaustive match audit for ContractDimension::State

Task 2/5 审计：补 ContractDimension::State 的 match arm（fallback 返回 None，
不触发结构性 fallback；task.rs 错误消息透传 {dimension:?}）。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

若无改动，本步跳过（在 commit log 中记录审计结论）。

---

### Task 6: 集成验证

**Files:**
- 全仓库

**Interfaces:**
- Consumes: Task 1-5 全部产物。

- [x] **Step 6.1: cargo build + cargo test 全绿**

Run:
```bash
cargo build --lib
cargo test --lib
cargo test --test '*' 2>/dev/null || true  # 集成测试（若有）
```

Expected:
- `cargo build` 通过，无 warning（除既有 warning）。
- `cargo test` 全绿，覆盖：
  - Task 1：`org_graph::work_state::tests`（5 个 schema serde 测试，含全字段 + 全子类型）
  - Task 2：`org_graph::work_state::tests`（11 个权限测试，含预留字段强制为空 + deferred 写入）+ `org_graph::contract::tests::contract_dimension_serde_roundtrip`
  - Task 3：`tools::checkpoint_store::tests`（2 个 WorkState 持久化测试）+ `exec_session::coordinator::tests`（2 个 turn 集成测试）
  - Task 4：`exec_session::node_runtime::tests`（3 个 pilot 修复测试 + 1 个字段强制可拦测试）
  - 既有零回归：`agent::coordinator` / `agent::fallback` / `tools::meta::task` / `exec_session::coordinator` / `exec_session::verify_gate` / `exec_session::node_runtime` / `org_graph::contract` / `org_graph::registry` / `org_graph::render` 全绿。

若任何既有测试红，按 systematic-debugging skill 定位根因（不得用 `_ =>` 或 unwrap 屏蔽），修复后回归。

- [x] **Step 6.2: 手动验证 pilot 路由点按结构化字段正确路由**

构造一个最小手动场景（可作为 doc test 或 example，也可在 `exec_session/node_runtime.rs::tests` 加一个端到端测试）：

```rust
    #[tokio::test]
    async fn pilot_end_to_end_retry_reads_structured_failure_kind() {
        // 端到端：verify 失败 → WorkState 写入强类型 → retry 决策读 VerifyFailureKind
        // 分支（CommandFailed vs BoundaryViolation）做不同处理。
        let setup = TestSetup::new(1); // CommandFailed
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        setup.runtime.verify_node().await.unwrap();

        // 模拟 retry 决策点：读 WorkState.verify_result 拿强类型分支。
        let coord = setup.coord.read().unwrap();
        let outcome = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .unwrap()
            .unwrap();
        // 路由判定：CommandFailed → 回到代码生成（pilot 文本不再参与判定）。
        match &outcome.fail_reason {
            Some(crate::org_graph::VerifyFailureKind::CommandFailed { .. }) => {
                // 命中「回到代码生成」分支
            }
            Some(crate::org_graph::VerifyFailureKind::BoundaryViolation { .. }) => {
                panic!("expected CommandFailed for exit 1, got BoundaryViolation");
            }
            None => panic!("expected failure, got success"),
        }
    }
```

Run: `cargo test --lib exec_session::node_runtime::tests::pilot_end_to_end_retry_reads_structured_failure_kind`
Expected: PASS。

- [x] **Step 6.3: 验证越权写字段被拦截（手动）**

在 Task 4.7 已有单元测试覆盖；本步确认手动构造的「非 Verification 节点尝试写 verify_result」场景 100% 返回 `ContractViolation { dimension: State }`，且 WorkState 保持写入前的值（不变更）。同时确认预留字段（compile_result/test_result/human_review）对所有节点越权写一律拒绝（Task 2.1 的 `reserved_fields_reject_all_node_types` 测试覆盖）。

Run: `cargo test --lib 'exec_session::node_runtime::tests::set_verify_result_rejects_unauthorized_node_type_at_pilot_site' 'org_graph::work_state::tests::reserved_fields_reject_all_node_types' 'org_graph::work_state::tests::set_verify_result_rejects_unauthorized_node_with_contract_violation_state'`
Expected: PASS。

- [x] **Step 6.4: Commit 集成测试**

```bash
git add src/exec_session/node_runtime.rs
git commit -m "test(exec-session): pilot end-to-end retry reads structured failure kind

Task 6 (集成验证): 端到端测试 verify 失败 → WorkState 写入强类型 VerifyOutcome →
retry 决策读 VerifyFailureKind 枚举分支（CommandFailed/BoundaryViolation）。
零回归：既有 agent/coordinator/fallback/task/exec_session 测试全绿。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review 结论

**1. Spec coverage（spec.md ADDED Requirements → Task 映射）：**

- `Requirement: 强类型共享工作状态`（完整 7+1 字段 schema）→ Task 1（schema + 全子类型）+ Task 4.4（`from_parts` 投影）
- `Requirement: 节点对状态字段的访问受权限约束`（含预留字段强制为空 Scenario）→ Task 2（FieldPerms 全字段矩阵 + 全字段读写 API + State 维度 + 预留字段 `{}` 强制）+ Task 4.7（pilot 写场景可验证）
- `Requirement: 工作状态与既有状态层分层不吞并` → Task 5.1（SessionState/AppState 不动）
- `Requirement: 工作状态随 turn 检查点持久化可续跑` → Task 3（CheckpointStore 持久化 + restore）
- `Requirement: 路由判定读取结构化字段而非解析文本`（verify_result/VerifyFailureKind pilot）→ Task 4（pilot 降级点修复 + D5 分支查证）
- `Requirement: 零回归与正交性` → Task 5.2（mailbox 不动）+ Task 5.3（dispatch-telemetry 正交）+ Task 6.1（cargo test 全绿）
- spec「完整 schema 与 deferred-write-point」Scenario → Task 1（全字段类型）+ Task 2（全字段权限 + 预留字段 `{}` 强制 + deferred setter 单测覆盖）+ design §1.5 查证依据

**tasks.md 18 子任务覆盖：** 1.1/1.2/1.3 → Task 1；2.1/2.2/2.3 → Task 2；3.1/3.2/3.3 → Task 3；4.1/4.2/4.3 → Task 4；5.1/5.2/5.3 → Task 5；6.1/6.2 → Task 6。无遗漏。

**2. Placeholder 扫描：** 无 TBD/TODO/「类似 Task N」；每个 code step 都给了完整 Rust 代码或可执行命令；查证步骤（Task 4.1）给了显式 A/B/C 分支而非假定结论。

**3. Type 一致性：**
- `WorkState` 完整 8 字段（requirement/generated_diff/compile_result/test_result/human_review/verify_result/budget/step_log）—— Task 1 定义、Task 2 读写 API 操作、Task 3 持久化往返、Task 4 pilot 写 verify_result，字段集一致。
- `VerifyOutcome { success: bool, fail_reason: Option<VerifyFailureKind> }` —— Task 1 定义、Task 2 用、Task 3 持久化往返、Task 4 投影，签名一致。
- `set_verify_result(caller: NodeType, outcome: VerifyOutcome) -> Result<(), CoordinatorError>` —— Task 2 定义、Task 4 调用，签名一致。
- `verify_result(caller: NodeType) -> Result<Option<&VerifyOutcome>, CoordinatorError>` —— Task 2 定义、Task 4/6 调用，签名一致。
- deferred setter（`set_generated_diff`/`set_budget`/`set_compile_result`/`set_test_result`/`set_human_review`）签名与 Task 2 定义一致，Task 2 单测覆盖，无生产调用点（符合 deferred-write-point 设计）。
- `ContractDimension::State` —— Task 2 定义、Task 2/4 测试断言，变体名一致。
- `WorkField` 8 变体 —— Task 1 定义、Task 2 矩阵全覆盖，一致。
- `capture_work_state(turn_id, &WorkState)` / `restore_work_state(turn_id) -> Result<Option<WorkState>>` —— Task 3 CheckpointStore 定义、Task 3 SessionCoordinator 调用，签名一致（入参为 `checkpoint_turn_id` UUID，已在 Task 3.7 注明）。
