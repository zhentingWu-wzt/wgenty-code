//! WorkState: per-task 结构化工作产物 schema（完整 7+1 字段）+ 字段权限读写 API。
//!
//! 本模块保持 org_graph「纯数据 + 纯函数」风格：无 async / I/O / 状态。
//! `exec_session` 侧负责把 `VerifyResult` 投影成 `VerifyOutcome` 并调用读写 API。
//! pilot 仅锚定 verify_result（唯一真实闭环）；compile/test/human_review/budget/
//! generated_diff 类型与权限就绪，生产写入点待将来新增节点的 change 接入（design §1.5）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::contract::NodeType;
use crate::agent::coordinator::CoordinatorError;
use crate::org_graph::contract::ContractDimension;

/// 当前 turn 的结构化工作产物。完整 schema：全字段类型 + 全字段权限真强制。
/// pilot 仅锚定 verify_result（唯一真实闭环）；其余字段类型与权限就绪，
/// 生产写入点待将来新增 Compile/Test 等节点的 change 接入。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkState {
    /// 任务原始需求（跨 turn 继承；coordinator 在 turn 初始化时设置，不经节点权限 API）。
    requirement: Option<String>,
    /// GeneralPurpose 产出（类型就绪，生产写入待接入）。
    generated_diff: Option<GeneratedDiff>,
    /// 预留：将来 Compile 节点写入。
    compile_result: Option<CompileResult>,
    /// 预留：将来 Test 节点写入。
    test_result: Option<TestResult>,
    /// 预留：将来人工评审节点写入。
    human_review: Option<HumanReview>,
    /// pilot 核心字段：verify 结果的强类型投影。
    verify_result: Option<VerifyOutcome>,
    /// 预留：预算追踪（类型就绪，生产写入待接入）。
    budget: Option<Budget>,
    /// 审计轨迹（授权写记入；读不记）。
    step_log: Vec<StepRecord>,
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
                writable: [WorkField::GeneratedDiff, WorkField::Budget]
                    .into_iter()
                    .collect(),
            },
            NodeType::Explore | NodeType::Plan | NodeType::WgentyCodeGuide => FieldPerms {
                readable: [WorkField::Requirement].into_iter().collect(),
                writable: HashSet::new(),
            },
        }
    }
}

impl WorkState {
    /// coordinator 特权设置任务需求（不经节点权限 API；design §5）。
    /// 不记 step_log（requirement 是任务级常量，非节点工作产物）。
    pub fn set_requirement(&mut self, requirement: Option<String>) {
        self.requirement = requirement;
    }

    /// 读任务需求。
    pub fn requirement(&self) -> Option<&str> {
        self.requirement.as_deref()
    }

    /// 读步骤审计日志（只读；仅由授权写自动追加，不可外部直接写）。
    pub fn step_log(&self) -> &[StepRecord] {
        &self.step_log
    }

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

impl VerifyOutcome {
    /// 从「外部强类型 verify 结果」投影构造。exec_session 侧负责字段映射（见
    /// node_runtime.rs 的 project_failure / project_outcome），调用本方法传入
    /// 已解构的原语字段，避免 org_graph 反向依赖 exec_session::VerifyResult。
    pub fn from_parts(success: bool, fail_kind: Option<VerifyFailureKind>) -> Self {
        Self { success, fail_reason: fail_kind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
                .set_compile_result(nt.clone(), CompileResult { ok: true, stderr: String::new() })
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
                .set_test_result(nt.clone(), TestResult { pass: true, failed_cases: vec![] })
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
                .set_human_review(nt.clone(), HumanReview::Approve)
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
}
