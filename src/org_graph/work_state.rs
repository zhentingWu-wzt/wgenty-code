//! WorkState: per-task 结构化工作产物 schema（完整 7+1 字段）+ 字段权限读写 API。
//!
//! 本模块保持 org_graph「纯数据 + 纯函数」风格：无 async / I/O / 状态。
//! `exec_session` 侧负责把 `VerifyResult` 投影成 `VerifyOutcome` 并调用读写 API。
//! pilot 仅锚定 verify_result（唯一真实闭环）；compile/test/human_review/budget/
//! generated_diff 类型与权限就绪，生产写入点待将来新增节点的 change 接入（design §1.5）。

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
}
