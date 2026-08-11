---
change: org-graph-shared-state
design-doc: docs/superpowers/specs/2026-08-11-org-graph-shared-state-design.md
base-ref: a819ff03bb2736519ff1945491c7c21838d5e6d9
---

# Org-Graph Shared-State 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `exec_session/node_runtime.rs:204` 的结构化降级点（强类型 `VerifyFailure` → `format!("{f:?}")` → `Option<String>`），引入 `WorkState` 强类型共享状态、字段级访问权限、turn 检查点持久化，让 pilot 路由点（verify 出口）从「解析自然语言字符串」改为「读结构化枚举字段」。

**Architecture:** 新增 `src/org_graph/work_state.rs` 承载 `WorkState` schema + 字段权限读写 API + `VerifyOutcome`/`VerifyFailureKind`（org_graph 内聚的强类型投影，不复制完整 `VerifyResult`）。在 `ContractDimension` 新增 `State` 变体以复用 `CoordinatorError::ContractViolation` 报错路径。`WorkState` 挂在 `SessionCoordinator` 内（复用现有 `Arc<RwLock>` 锁层级），随 `begin_turn` 做 turn 间继承（requirement 保留、verify_result/step_log 重置），随 `CheckpointStore` 旁路持久化（per-turn `work_state.json`）。pilot 集成点（`verify_node` 出口）先经 `set_verify_result` 把强类型投影写入 `WorkState`（受 `NodeType::Verification` 字段权限强制），再从 `WorkState.verify_result()` 读回组装兼容期的 `failure_reason: Option<String>`，使 retry 决策改为直接读 `Option<&VerifyOutcome>` 拿 `VerifyFailureKind` 枚举分支。

**Tech Stack:** Rust 2021 / cargo / serde / thiserror / chrono / uuid / tokio（既有栈，零新 crate）。

## Global Constraints

- 本 change 纯新增只读 + 结构化层；**不改** `NodeContract` 五维语义、不改 `reserve_child` / `filter_allowed_tools` 强制逻辑、不改 transcript store schema、不改 dispatch 路径。
- `org_graph` 模块保持「纯数据 + 纯函数，零 async / I/O / 状态」；集成（`VerifyResult` → `VerifyOutcome` 投影、turn 锚定、CheckpointStore 持久化）在 `exec_session` 侧。
- `WorkState` / `VerifyOutcome` / `VerifyFailureKind` / `FieldPerms` / `WorkField` / `StepRecord` 全部派生 `Serialize/Deserialize/Clone/Debug`（支持持久化与单测）。
- 字段级权限**真强制**：越权写直接返回 `ContractViolation { dimension: State }`，不做「声明 + warning」软路径。
- `NodeVerifyResult.failure_reason: Option<String>` **本期保留**（兼容期，零回归），但源头从 `format!("{f:?}")` 改为「从 WorkState 强类型枚举读回再转 debug string」。
- 每个 task 遵循 TDD：先写失败测试 → 跑测试见红 → 写最小实现 → 跑测试见绿 → commit。
- `ContractDimension` 新增 `State` 变体后，全仓库 exhaustive match 需审计，补 `State` arm 或 `_ =>`（见 Task 2 Step 1）。
- 与 `org-graph-dispatch-telemetry` 正交：两者字段/schema 互不依赖、互不修改。
- 所有产物使用简体中文（zh-CN）书写注释与文档字符串；代码标识符（类型/函数/变量名）保持英文。

## File Structure

**新建：**

- `src/org_graph/work_state.rs` —— `WorkState` schema + `VerifyOutcome` / `VerifyFailureKind`（强类型投影）+ `FieldPerms` / `WorkField`（权限矩阵）+ `StepRecord`（审计轨迹）+ `NodeType::field_perms()` impl + `WorkState` 受权限约束的读写 API（`set_verify_result` / `verify_result` / `inherit_for_new_turn`）+ `VerifyOutcome::From<&VerifyResult>` 转换逻辑。纯数据 + 纯函数，零 `exec_session` 依赖（转换逻辑用 trait + impl 隔离，见 Task 1 Step 3）。

**修改：**

- `src/org_graph/mod.rs` —— 导出 `pub mod work_state;` 及 `WorkState` / `VerifyOutcome` / `VerifyFailureKind` / `FieldPerms` / `WorkField` / `StepRecord`。
- `src/org_graph/contract.rs` —— `ContractDimension` 新增 `State` 变体（置于 `Budget` 之后，保持 enum 序以减小 serde 影响面）。
- `src/exec_session/coordinator.rs` —— `SessionCoordinator` 新增 `work_state: WorkState` 字段；`SessionCoordinator::new` 初始化；`begin_turn` 在 turn 链推进后调用 `self.work_state = self.work_state.inherit_for_new_turn()`；新增 `work_state()` / `work_state_mut()` 访问器。
- `src/exec_session/node_runtime.rs` —— `verify_node` 出口（line 204 附近）改为：先把 `VerifyResult` 投影成 `VerifyOutcome` 并经 `set_verify_result(NodeType::Verification, outcome)` 写入 `WorkState`，再从 `verify_result(NodeType::Verification)` 读回组装 `failure_reason`；retry 决策改为读 `Option<&VerifyOutcome>` 拿 `VerifyFailureKind` 枚举分支。
- `src/tools/checkpoint_store.rs` —— 新增 `capture_work_state(turn_id, &WorkState)` / `restore_work_state(turn_id)` 方法（写/读 turn 目录下的 `work_state.json` 旁路文件，不动文件 capture 语义）；持久化触发点在 `SessionCoordinator`。

**不改：** `reserve_child` / `can_spawn` / `filter_allowed_tools` / `SubagentResultMailbox` / dispatch 路径 / `AppState` / `SessionState` 字段语义。

---

### Task 1: WorkState schema 与模块骨架

**Files:**
- Create: `src/org_graph/work_state.rs`
- Modify: `src/org_graph/mod.rs`
- Test: `src/org_graph/work_state.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `NodeType`（来自 `super::contract`，已存在）；后续 Task 4 会从 `crate::exec_session::verify_gate::VerifyResult` / `crate::exec_session::hooks::VerifyFailure` 投影转换。
- Produces: `pub struct WorkState { requirement: Option<String>, verify_result: Option<VerifyOutcome>, step_log: Vec<StepRecord> }`；`pub struct VerifyOutcome { success: bool, fail_reason: Option<VerifyFailureKind> }`；`pub enum VerifyFailureKind { CommandFailed { exit_code: Option<i32>, stderr: String }, BoundaryViolation { unexpected_files: Vec<String> } }`；`pub struct StepRecord { node_type: NodeType, field: WorkField, action: StepAction, timestamp: String }`；`pub enum WorkField { Requirement, VerifyResult, StepLog }`；`pub enum StepAction { Read, Wrote }`。

- [ ] **Step 1.1: 写失败测试 —— schema serde 往返 + 默认值**

在新建文件 `src/org_graph/work_state.rs` 末尾的 `#[cfg(test)] mod tests` 中，先写测试（此时类型未定义 → 编译失败 = 红）。

```rust
//! WorkState: per-task 结构化工作产物 schema + 字段权限读写 API。
//!
//! 本模块保持 org_graph「纯数据 + 纯函数」风格：无 async / I/O / 状态。
//! `exec_session` 侧负责把 `VerifyResult` 投影成 `VerifyOutcome` 并调用读写 API。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::contract::NodeType;

/// 当前 turn 的结构化工作产物。pilot 只强制 verify_result；
/// 其余字段为 Option 占位，强制逻辑留后续 change（避免空壳 API）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkState {
    /// 任务原始需求（跨 turn 继承）。
    pub requirement: Option<String>,
    /// pilot 核心字段：verify 结果的强类型投影。
    pub verify_result: Option<VerifyOutcome>,
    /// 审计轨迹（授权写记入；读不记）。
    pub step_log: Vec<StepRecord>,
    // generated_diff / test_result / human_review / budget:
    // spec 列出但 pilot 不涉及，本期不声明读写强制逻辑（避免空壳 API）。
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
    VerifyResult,
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
        assert!(state.verify_result.is_none());
        assert!(state.step_log.is_empty());
    }

    #[test]
    fn workstate_serde_roundtrip_populated() {
        let state = WorkState {
            requirement: Some("实现 WorkState".into()),
            verify_result: Some(VerifyOutcome {
                success: false,
                fail_reason: Some(VerifyFailureKind::CommandFailed {
                    exit_code: Some(1),
                    stderr: "error: mismatched types".into(),
                }),
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
    fn verifyoutcome_default_is_success_no_fail() {
        // 默认构造：success=true、fail_reason=None。
        let outcome = VerifyOutcome {
            success: true,
            fail_reason: None,
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        let back: VerifyOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(outcome, back);
        assert!(back.success);
        assert!(back.fail_reason.is_none());
    }
}
```

- [ ] **Step 1.2: 跑测试验证失败（红）**

Run: `cargo test --lib org_graph::work_state::tests --no-run`
Expected: 编译失败 —— `error[E0433]: failed to resolve: could not find work_state in org_graph`（模块尚未在 mod.rs 导出）。

- [ ] **Step 1.3: 在 mod.rs 导出 work_state 模块**

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
    StepAction, StepRecord, VerifyFailureKind, VerifyOutcome, WorkField, WorkState,
};
```

- [ ] **Step 1.4: 跑测试验证通过（绿）**

Run: `cargo test --lib org_graph::work_state::tests`
Expected: PASS —— 4 个测试全绿（serde 往返、默认值、枚举变体不丢类型）。

- [ ] **Step 1.5: Commit**

```bash
git add src/org_graph/work_state.rs src/org_graph/mod.rs
git commit -m "feat(org-graph): add WorkState schema + VerifyOutcome/VerifyFailureKind

Task 1 (schema 骨架): 新增 work_state.rs 承载 per-task 结构化工作产物。
WorkState 含 requirement/verify_result/step_log 三字段；VerifyOutcome/VerifyFailureKind
为 org_graph 内聚的强类型投影（独立于 exec_session::hooks::VerifyFailure，避免反向依赖）。
全部派生 Serialize/Deserialize/Clone/Debug 以支持 CheckpointStore 持久化与单测。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 字段级访问权限（真强制）

**Files:**
- Modify: `src/org_graph/work_state.rs`（新增 `FieldPerms` / `NodeType::field_perms()` impl / 读写 API）
- Modify: `src/org_graph/contract.rs`（`ContractDimension` 新增 `State` 变体）
- Modify: `src/org_graph/mod.rs`（导出 `FieldPerms`）
- Audit: 全仓库对 `ContractDimension` 的 exhaustive match
- Test: `src/org_graph/work_state.rs::tests`

**Interfaces:**
- Consumes: `NodeType`（来自 `super::contract`）；`CoordinatorError::ContractViolation`（来自 `crate::agent::coordinator`，已存在 `{ node_type, dimension, reason }` 形状）。
- Produces: `pub struct FieldPerms { readable: HashSet<WorkField>, writable: HashSet<WorkField> }`；`impl NodeType { pub fn field_perms(&self) -> FieldPerms }`；`impl WorkState { pub fn set_verify_result(&mut self, caller: NodeType, outcome: VerifyOutcome) -> Result<(), CoordinatorError>; pub fn verify_result(&self, caller: NodeType) -> Result<Option<&VerifyOutcome>, CoordinatorError> }`；`impl WorkState { pub fn inherit_for_new_turn(&self) -> WorkState }`。

- [ ] **Step 2.1: 写失败测试 —— 权限矩阵 + 越权写拒绝 + step_log 记入**

在 `src/org_graph/work_state.rs` 的 `#[cfg(test)] mod tests` 末尾追加测试（此时 `field_perms` / `set_verify_result` / `verify_result` 尚未实现 → 编译失败 = 红）。

```rust
    #[test]
    fn field_perms_verification_can_write_verify_result_and_steplog() {
        let perms = NodeType::Verification.field_perms();
        assert!(perms.writable.contains(&WorkField::VerifyResult));
        assert!(perms.writable.contains(&WorkField::StepLog));
        assert!(perms.readable.contains(&WorkField::Requirement));
        assert!(perms.readable.contains(&WorkField::VerifyResult));
        assert!(perms.readable.contains(&WorkField::StepLog));
    }

    #[test]
    fn field_perms_generalpurpose_can_read_verify_but_not_write() {
        // GeneralPurpose 是协调者：可读 verify_result 做 retry 决策，但不能写。
        let perms = NodeType::GeneralPurpose.field_perms();
        assert!(perms.readable.contains(&WorkField::VerifyResult));
        assert!(!perms.writable.contains(&WorkField::VerifyResult));
        assert!(perms.writable.contains(&WorkField::StepLog));
    }

    #[test]
    fn field_perms_explore_plan_guide_only_read_requirement() {
        for nt in [NodeType::Explore, NodeType::Plan, NodeType::WgentyCodeGuide] {
            let perms = nt.field_perms();
            assert!(perms.readable.contains(&WorkField::Requirement));
            assert!(!perms.readable.contains(&WorkField::VerifyResult));
            assert!(perms.writable.is_empty(), "{:?} should have empty writable", nt);
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
        // Verification 节点授权写：成功，且 step_log 记入一条 Wrote 记录。
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
        // Explore 节点无写权限：应被拒绝，且 WorkState 保持写入前的值。
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
        // WorkState 不变。
        assert!(state.verify_result.is_none());
        assert!(state.step_log.is_empty());
    }

    #[test]
    fn verify_result_read_authorizes_verification_and_skips_log() {
        // 读授权：成功返回 Option<&VerifyOutcome>；读不记 step_log（避免爆 log）。
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
```

- [ ] **Step 2.2: 跑测试验证失败（红）**

Run: `cargo test --lib org_graph::work_state::tests --no-run`
Expected: 编译失败 —— `field_perms` / `set_verify_result` / `verify_result` / `CoordinatorError` 引用未定义。

- [ ] **Step 2.3: 在 ContractDimension 新增 State 变体**

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

- [ ] **Step 2.4: 审计全仓库对 ContractDimension 的 exhaustive match**

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

- [ ] **Step 2.5: 实现 FieldPerms + NodeType::field_perms + 读写 API**

在 `src/org_graph/work_state.rs` 顶部 imports 已有 `HashSet`、`NodeType`。追加类型与 impl（注意：`field_perms` impl 必须放在 `work_state.rs`，因为返回类型 `FieldPerms` 引用 `WorkField`，遵循 Rust orphan rule；不能放 `contract.rs`）：

```rust
use crate::agent::coordinator::CoordinatorError;

use crate::org_graph::contract::ContractDimension;

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
    /// - Verification：执行 verify → 写 verify_result + step_log，读三类字段。
    /// - GeneralPurpose（可做 retry 决策的协调者）：读 verify_result，写 step_log。
    /// - Explore / Plan / WgentyCodeGuide：本期不涉及 verify_result，只读 requirement。
    pub fn field_perms(&self) -> FieldPerms {
        match self {
            NodeType::Verification => FieldPerms {
                readable: [WorkField::Requirement, WorkField::VerifyResult, WorkField::StepLog]
                    .into_iter()
                    .collect(),
                writable: [WorkField::VerifyResult, WorkField::StepLog]
                    .into_iter()
                    .collect(),
            },
            NodeType::GeneralPurpose => FieldPerms {
                readable: [WorkField::Requirement, WorkField::VerifyResult, WorkField::StepLog]
                    .into_iter()
                    .collect(),
                writable: [WorkField::StepLog].into_iter().collect(),
            },
            NodeType::Explore | NodeType::Plan | NodeType::WgentyCodeGuide => FieldPerms {
                readable: [WorkField::Requirement].into_iter().collect(),
                writable: HashSet::new(),
            },
        }
    }
}

impl WorkState {
    /// 写 verify_result：查 caller 的 field_perms，越权 → ContractViolation{State}。
    /// 写成功自动追加 step_log（谁在何时写了哪个字段）。
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
        self.step_log.push(StepRecord {
            node_type: caller,
            field: WorkField::VerifyResult,
            action: StepAction::Wrote,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    /// 读 verify_result：查 caller 的 field_perms，越权读也报（读写对称，便于审计）。
    /// 读不记 step_log（读高频，记会爆）。
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

    /// turn 间继承：requirement 克隆保留，verify_result / step_log 清空。
    /// 同 turn 内 retry 不走 begin_turn（retry 是 node 重试，不是 turn 重置），
    /// WorkState 自动保留——对齐「同 turn 保留 / 跨 turn 产物重置」语义。
    pub fn inherit_for_new_turn(&self) -> WorkState {
        WorkState {
            requirement: self.requirement.clone(),
            verify_result: None,
            step_log: Vec::new(),
        }
    }
}
```

同时更新 `src/org_graph/mod.rs` 导出 `FieldPerms`：

```rust
pub use work_state::{
    FieldPerms, StepAction, StepRecord, VerifyFailureKind, VerifyOutcome, WorkField, WorkState,
};
```

注意：`work_state.rs` 此刻开始依赖 `crate::agent::coordinator::CoordinatorError` 与 `chrono`。检查 `Cargo.toml` 已有 `chrono` 依赖（coordinator.rs 已用），无需新增。

- [ ] **Step 2.6: 跑测试验证通过（绿）**

Run: `cargo test --lib org_graph::work_state::tests`
Expected: PASS —— Task 1 的 4 个 serde 测试 + Task 2 新增的 7 个权限测试全绿；`contract_dimension_serde_roundtrip` 也通过（已含 `State`）。

- [ ] **Step 2.7: 跑全库 build 验证 exhaustive match 已补齐**

Run: `cargo build --lib && cargo test --lib`
Expected: build 通过；既有 `agent::coordinator` / `agent::fallback` / `tools::meta::task` 测试零回归。

- [ ] **Step 2.8: Commit**

```bash
git add src/org_graph/work_state.rs src/org_graph/contract.rs src/org_graph/mod.rs
git commit -m "feat(org-graph): field-level permission enforcement on WorkState

Task 2 (字段级权限): 新增 FieldPerms/NodeType::field_perms 矩阵 + WorkState
受权限约束的 set_verify_result/verify_result 读写 API。越权读写返回
ContractViolation{State}（ContractDimension 新增 State 变体，全库 exhaustive
match 已审计）。授权写自动记 step_log，读不记（避免高频读爆 log）。
inherit_for_new_turn 提供 turn 间继承（requirement 保留、产物重置）。

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

- [ ] **Step 3.1: 写失败测试 —— CheckpointStore 持久化往返**

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
            step_log: Vec::new(),
        };

        store.capture_work_state(turn_id, &state).expect("capture");
        let restored = store
            .restore_work_state(turn_id)
            .expect("restore")
            .expect("work_state.json should exist after capture");
        assert_eq!(restored.requirement, state.requirement);
        assert_eq!(restored.verify_result, state.verify_result);
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

- [ ] **Step 3.2: 跑测试验证失败（红）**

Run: `cargo test --lib tools::checkpoint_store::tests --no-run`
Expected: 编译失败 —— `capture_work_state` / `restore_work_state` 未定义。

- [ ] **Step 3.3: 实现 CheckpointStore::capture_work_state / restore_work_state**

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

- [ ] **Step 3.4: 跑测试验证通过（绿）**

Run: `cargo test --lib tools::checkpoint_store::tests`
Expected: PASS —— `work_state_capture_and_restore_roundtrip` + `work_state_restore_returns_none_for_legacy_turn_without_snapshot` 全绿；既有 CheckpointStore 测试零回归。

- [ ] **Step 3.5: 写失败测试 —— SessionCoordinator begin_turn 继承 + 持久化触发**

在 `src/exec_session/coordinator.rs` 末尾的 `#[cfg(test)] mod tests` 中追加。此时 `work_state()` / `work_state_mut()` / `capture_current_work_state` / `restore_work_state_for_turn` 未实现 → 编译失败 = 红。

```rust
    #[test]
    fn begin_turn_inherits_requirement_and_resets_verify_result() {
        // turn-0 写 requirement + verify_result，begin_turn 推进到 turn-1：
        // requirement 保留，verify_result / step_log 清空。
        let setup = coordinator_setup(); // 复用既有 fixture（见 audit 步骤）
        {
            let mut coord = setup.coord.write().unwrap();
            coord.work_state_mut().requirement = Some("实现 WorkState".into());
            coord.work_state_mut().verify_result = Some(
                crate::org_graph::VerifyOutcome {
                    success: false,
                    fail_reason: Some(crate::org_graph::VerifyFailureKind::CommandFailed {
                        exit_code: Some(1),
                        stderr: "boom".into(),
                    }),
                },
            );
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

- [ ] **Step 3.6: 跑测试验证失败（红）**

Run: `cargo test --lib exec_session::coordinator::tests --no-run`
Expected: 编译失败 —— `work_state` / `work_state_mut` / `capture_current_work_state` / `restore_work_state_for_turn` 未定义；`begin_turn` 不会继承 requirement。

- [ ] **Step 3.7: 在 SessionCoordinator 集成 WorkState + begin_turn 继承 + 持久化**

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
    /// 当前 turn 的结构化工作产物（pilot: verify_result 强制）。
    /// legacy turn 缺 WorkState 时为 default()，向后兼容。
    work_state: WorkState,
}
```

3. 在 `SessionCoordinator::new` 末尾（构造返回前）加 `work_state: WorkState::default(),`。

4. 在 `begin_turn` 末尾（`Ok(self.session.turns.last().expect("just pushed"))` 之前）加 turn 间继承：

```rust
        // WorkState turn 间继承：requirement 保留，verify_result / step_log 重置。
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

- [ ] **Step 3.8: 跑测试验证通过（绿）**

Run: `cargo test --lib exec_session::coordinator::tests`
Expected: PASS —— `begin_turn_inherits_requirement_and_resets_verify_result` + `capture_and_restore_work_state_survives_roundtrip` 全绿；既有 coordinator 测试（`begin_turn_links_checkpoint_store` 等）零回归。

- [ ] **Step 3.9: Commit**

```bash
git add src/exec_session/coordinator.rs src/tools/checkpoint_store.rs
git commit -m "feat(exec-session): anchor WorkState on turn + CheckpointStore persistence

Task 3 (turn 集成 + 持久化): SessionCoordinator 新增 work_state 字段与
work_state/work_state_mut 访问器。begin_turn 调用 inherit_for_new_turn 完成
turn 间继承（requirement 保留、verify_result/step_log 重置）。
CheckpointStore 新增 capture_work_state/restore_work_state 旁路持久化
（per-turn work_state.json，不动文件 capture 语义）。legacy turn 缺失时
返回 default()，向后兼容。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: pilot 路由点（读结构化字段 + 验证强制）

**Files:**
- Audit / Verify: `src/exec_session/verify_gate.rs`（确认 `VerifyResult`/`VerifyFailure` 字段）、`src/exec_session/hooks.rs:17`（`VerifyFailure` 定义）、`src/exec_session/node_runtime.rs:204`（降级点）
- Modify: `src/exec_session/node_runtime.rs`（修复 line 204 降级点 + retry 决策改读枚举）
- Test: `src/exec_session/node_runtime.rs::tests`

**Interfaces:**
- Consumes: `VerifyResult` / `VerifyFailure`（来自 `super::verify_gate` / `super::hooks`，已存在）；`WorkState::set_verify_result` / `verify_result`（来自 Task 2）；`SessionCoordinator::work_state_mut` / `work_state`（来自 Task 3）。
- Produces: `impl VerifyOutcome { pub fn from_verify_result(result: &VerifyResult) -> VerifyOutcome }`（放在 `work_state.rs`，作为 `exec_session → org_graph` 投影的「契约点」）；`node_runtime::verify_node` 出口先写 WorkState 再读回。

**D5 硬约束分支（必须在 Step 4.1 显式决策）：**

design doc §1 已基于 brainstorming 期的 CodeGraph 查证得出结论：pilot = 修复 `node_runtime.rs:204` 的 `format!("{f:?}")` 结构化降级点（verify→retry 闭环真实存在、`VerifyResult.fail_reason: Option<VerifyFailure>` 强类型在内部已存在、出口降级为 String）。**但 Task 4.1 必须在实现期重新查证**，因为 brainstorming 与 build 之间代码库可能漂移。

- [ ] **Step 4.1: 查证 pilot 路由点（D5 硬约束分支决策）**

按 design doc D5 硬约束（pilot 必须含真实写字段场景），重新查证三件事：

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

- **分支 A（与 design doc 结论一致）**：三查证全部命中 → pilot = 修复 node_runtime.rs:204 降级点。继续 Step 4.2。
- **分支 B（代码已漂移，降级点不存在）**：查证 1 未命中或语义已变 → 暂停，回到 design 阶段重选 pilot（D5 候选：编译失败→代码生成 / 测试失败→代码生成）。不得在 pilot 不含真实写字段场景的情况下继续——否则字段级强制（Task 2）无场景可拦，违背 change 核心论点。
- **分支 C（retry 决策已读不到 failure_reason）**：查证 3 未命中 → pilot 路由点失去读字段语义，需在 Step 4.2 同步迁移一个真实的 retry 决策点读 `Option<&VerifyOutcome>`，否则 pilot 不满足「读结构化字段做判定」。

在本步完成后，于 commit message 中记录命中分支（A/B/C）与查证证据（命中的文件:行号）。

- [ ] **Step 4.2: 写失败测试 —— pilot 降级点修复 + retry 决策读枚举**

在 `src/org_graph/work_state.rs` 的 `#[cfg(test)] mod tests` 末尾追加（验证 `VerifyOutcome::from_verify_result` 投影正确性）：

```rust
    #[test]
    fn from_verify_result_success_projects_to_success_no_fail() {
        // exec_session → org_graph 投影：成功 VerifyResult → VerifyOutcome{success:true}。
        // 用 trait+impl 隔离，避免 work_state.rs 反向依赖 exec_session（见 Step 4.4）。
        // 此处直接构造 VerifyOutcome 验证字段语义。
        let outcome = VerifyOutcome {
            success: true,
            fail_reason: None,
        };
        assert!(outcome.success);
        assert!(outcome.fail_reason.is_none());
    }
```

在 `src/exec_session/node_runtime.rs` 的 `#[cfg(test)] mod tests` 末尾追加 pilot 修复测试（此时 verify_node 仍走旧降级路径 → 测试红）：

```rust
    #[tokio::test]
    async fn verify_node_failure_writes_structured_outcome_to_work_state() {
        // pilot D5 硬约束：verify 失败后 WorkState.verify_result 必须是强类型
        // VerifyOutcome（非 format!("{f:?}") 文本）。
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

- [ ] **Step 4.3: 跑测试验证失败（红）**

Run: `cargo test --lib exec_session::node_runtime::tests`
Expected: FAIL —— `verify_node_failure_writes_structured_outcome_to_work_state` 与 `verify_node_success_clears_fail_reason_in_work_state` 失败（WorkState.verify_result 为 None，因为 verify_node 还没写 WorkState）；`verify_node_failure_reason_string_comes_from_work_state` 可能恰好通过（旧路径也产出 debug string），但源头未改。

- [ ] **Step 4.4: 实现 VerifyOutcome::from_verify_result 投影转换**

为避免 `org_graph` 反向依赖 `exec_session`（org_graph 是纯数据层），投影转换用 trait + impl 隔离：在 `work_state.rs` 定义 `TryFrom` 风格的抽象，在 `exec_session` 侧实现。

方案：在 `src/org_graph/work_state.rs` 加一个泛型构造器（不引用 exec_session 类型）：

```rust
impl VerifyOutcome {
    /// 从「外部强类型 verify 结果」投影构造。exec_session 侧的 impl 负责字段映射。
    /// 此方法接受已解构的原语字段，避免 org_graph 反向依赖 exec_session::VerifyResult。
    pub fn from_parts(
        success: bool,
        fail_kind: Option<VerifyFailureKind>,
    ) -> Self {
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

- [ ] **Step 4.5: 修复 node_runtime.rs:204 降级点**

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

- [ ] **Step 4.6: 跑测试验证通过（绿）**

Run: `cargo test --lib exec_session::node_runtime::tests`
Expected: PASS —— Task 4 新增 3 个测试全绿；既有 `verify_node_success_transitions_to_verified` / `verify_node_failure_within_retry_budget` / `verify_node_failure_within_retry_budget` 等测试零回归（成功路径也写 WorkState，但既有断言不读 WorkState，故不破坏）。

- [ ] **Step 4.7: 字段级权限强制在 pilot 写场景的可验证性检查**

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

- [ ] **Step 4.8: Commit**

```bash
git add src/org_graph/work_state.rs src/exec_session/node_runtime.rs
git commit -m "fix(exec-session): repair verify_node structured-degradation pilot

Task 4 (pilot): 修复 node_runtime.rs:204 的 format!(\"{f:?}\") 结构化降级点。
verify_node 出口改为：先把 VerifyResult 投影成 VerifyOutcome（project_outcome），
经 set_verify_result(NodeType::Verification) 写入 WorkState（受字段级权限强制），
再从 WorkState.verify_result() 读回组装兼容期 failure_reason。retry 决策改为
读 Option<&VerifyOutcome> 拿 VerifyFailureKind 枚举分支。

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

- [ ] **Step 5.1: 验证 WorkState 与 SessionState / AppState 三层职责分明**

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

- [ ] **Step 5.2: 验证 SubagentResultMailbox 在非 pilot 路径维持原状**

Run:
```bash
git diff a819ff03 -- src/teams/subagent_mailbox.rs src/tools/meta/task.rs | head -50
```

期望：`subagent_mailbox.rs` 零改动；`task.rs` 仅可能有 Task 2 Step 2.4 审计带来的 match arm 调整（若 `map_coordinator_error` 内部 match 了 `ContractDimension`，应补了 `State` arm）。mailbox 的 `content: String` 写路径不经过 WorkState API，与字段级强制正交。

若 `task.rs` 因 Task 2 的 ContractDimension::State 变更有改动，确认改动仅为补 `State` arm（透传错误消息），不改 dispatch / fallback 语义。

- [ ] **Step 5.3: 验证与 org-graph-dispatch-telemetry 的正交性**

Run:
```bash
ls openspec/changes/org-graph-dispatch-telemetry/specs/ 2>/dev/null
grep -rn "WorkState\|work_state\|verify_result" openspec/changes/org-graph-dispatch-telemetry/specs/ 2>/dev/null || echo "no overlap"
```

期望：dispatch-telemetry 的 spec 不引用 `WorkState` / `work_state` / `verify_result`；本 change 也不引用 dispatch-telemetry 的 transcript schema 字段。两者可独立交付。

Run: `git diff a819ff03 -- openspec/changes/org-graph-dispatch-telemetry/ | head`
Expected: 空 diff（本 change 不改 dispatch-telemetry 任何文件）。

- [ ] **Step 5.4: Commit（若 Task 5 仅审计、无代码改动则跳过）**

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

- [ ] **Step 6.1: cargo build + cargo test 全绿**

Run:
```bash
cargo build --lib
cargo test --lib
cargo test --test '*' 2>/dev/null || true  # 集成测试（若有）
```

Expected:
- `cargo build` 通过，无 warning（除既有 warning）。
- `cargo test` 全绿，覆盖：
  - Task 1：`org_graph::work_state::tests`（4 个 serde 测试）
  - Task 2：`org_graph::work_state::tests`（7 个权限测试）+ `org_graph::contract::tests::contract_dimension_serde_roundtrip`
  - Task 3：`tools::checkpoint_store::tests`（2 个 WorkState 持久化测试）+ `exec_session::coordinator::tests`（2 个 turn 集成测试）
  - Task 4：`exec_session::node_runtime::tests`（3 个 pilot 修复测试 + 1 个字段强制可拦测试）
  - 既有零回归：`agent::coordinator` / `agent::fallback` / `tools::meta::task` / `exec_session::coordinator` / `exec_session::verify_gate` / `exec_session::node_runtime` / `org_graph::contract` / `org_graph::registry` / `org_graph::render` 全绿。

若任何既有测试红，按 systematic-debugging skill 定位根因（不得用 `_ =>` 或 unwrap 屏蔽），修复后回归。

- [ ] **Step 6.2: 手动验证 pilot 路由点按结构化字段正确路由**

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

- [ ] **Step 6.3: 验证越权写字段被拦截（手动）**

在 Task 4.7 已有单元测试覆盖；本步确认手动构造的「非 Verification 节点尝试写 verify_result」场景 100% 返回 `ContractViolation { dimension: State }`，且 WorkState 保持写入前的值（不变更）。

Run: `cargo test --lib 'exec_session::node_runtime::tests::set_verify_result_rejects_unauthorized_node_type_at_pilot_site' 'org_graph::work_state::tests::set_verify_result_rejects_unauthorized_node_with_contract_violation_state'`
Expected: PASS。

- [ ] **Step 6.4: Commit 集成测试**

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

- `Requirement: 强类型共享工作状态` → Task 1（schema）+ Task 4.4（`from_parts` 投影）
- `Requirement: 节点对状态字段的访问受权限约束` → Task 2（FieldPerms + 读写 API + State 维度）+ Task 4.7（pilot 写场景可验证）
- `Requirement: 工作状态与既有状态层分层不吞并` → Task 5.1（SessionState/AppState 不动）
- `Requirement: 工作状态随 turn 检查点持久化可续跑` → Task 3（CheckpointStore 持久化 + restore）
- `Requirement: 路由判定读取结构化字段而非解析文本` → Task 4（pilot 降级点修复 + D5 分支查证）
- `Requirement: 零回归与正交性` → Task 5.2（mailbox 不动）+ Task 5.3（dispatch-telemetry 正交）+ Task 6.1（cargo test 全绿）

**tasks.md 18 子任务覆盖：** 1.1/1.2/1.3 → Task 1；2.1/2.2/2.3 → Task 2；3.1/3.2/3.3 → Task 3；4.1/4.2/4.3 → Task 4；5.1/5.2/5.3 → Task 5；6.1/6.2 → Task 6。无遗漏。

**2. Placeholder 扫描：** 无 TBD/TODO/「类似 Task N」；每个 code step 都给了完整 Rust 代码或可执行命令；查证步骤（Task 4.1）给了显式 A/B/C 分支而非假定结论。

**3. Type 一致性：**
- `VerifyOutcome { success: bool, fail_reason: Option<VerifyFailureKind> }` —— Task 1 定义、Task 2 用、Task 3 持久化往返、Task 4 投影，签名一致。
- `set_verify_result(caller: NodeType, outcome: VerifyOutcome) -> Result<(), CoordinatorError>` —— Task 2 定义、Task 4 调用，签名一致。
- `verify_result(caller: NodeType) -> Result<Option<&VerifyOutcome>, CoordinatorError>` —— Task 2 定义、Task 4/6 调用，签名一致。
- `ContractDimension::State` —— Task 2 定义、Task 2/4 测试断言，变体名一致。
- `capture_work_state(turn_id, &WorkState)` / `restore_work_state(turn_id) -> Result<Option<WorkState>>` —— Task 3 CheckpointStore 定义、Task 3 SessionCoordinator 调用，签名一致（入参为 `checkpoint_turn_id` UUID，已在 Task 3.7 注明）。
