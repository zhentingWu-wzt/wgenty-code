//! WorkState: per-task 结构化工作产物 schema（完整 7+1 字段）+ 字段权限读写 API。
//!
//! 本模块保持 org_graph「纯数据 + 纯函数」风格：无 async / I/O / 状态。
//! `exec_session` 侧负责把 `VerifyResult` 投影成 `VerifyOutcome` 并调用读写 API。
//! pilot 仅锚定 verify_result（唯一真实闭环）；compile/test/human_review/budget/
//! generated_diff 类型与权限就绪，生产写入点待将来新增节点的 change 接入（design §1.5）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::contract::NodeType;
use super::work_graph_plan::WorkGraphPlan;
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
    /// 工作图执行审计轨迹（跨节点、pass 与 turn 保留，兼容既有 checkpoint）。
    #[serde(default)]
    graph_audit: Vec<GraphAuditEvent>,
    /// 专用子 Agent 的结构化交接报告（兼容既有 checkpoint）。
    #[serde(default)]
    specialist_reports: Vec<SpecialistReport>,
    /// Durable binding between a graph role and a spawned child Agent.
    #[serde(default)]
    graph_child_bindings: Vec<GraphChildBinding>,
    /// Coordinator-selected bounded Work-Graph for the active node. This is
    /// configuration, not an agent-produced artifact, so only the runtime may
    /// replace it.
    #[serde(default)]
    selected_work_graph: Option<WorkGraphPlan>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphChildBinding {
    pub node_id: String,
    pub attempt: u32,
    pub role: NodeType,
    pub child_agent_id: String,
    pub timestamp: String,
}

/// A specialist role's typed report category.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SpecialistReportKind {
    /// Read-only repository exploration findings.
    Exploration,
    /// A diagnosis grounded in source and external-anchor evidence.
    RootCause,
    /// An actionable implementation plan.
    ImplementationPlan,
}

/// One evidence item cited by a specialist report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialistEvidence {
    /// Repository-relative path that supports the finding.
    pub path: String,
    /// Concise observation derived from the path.
    pub detail: String,
}

/// A validated, checkpointed handoff product from a specialist sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialistReport {
    /// Trusted producer persisted alongside its self-described report.
    pub producer: NodeType,
    /// The report's structured purpose.
    pub kind: SpecialistReportKind,
    /// Bounded conclusion for the next node.
    pub summary: String,
    /// Non-empty source observations supporting the conclusion.
    pub evidence: Vec<SpecialistEvidence>,
    /// Deduplicated files likely to require attention.
    pub suspected_files: Vec<String>,
    /// Ordered, non-empty recommended next actions.
    pub recommended_actions: Vec<String>,
}

/// 单个外部命令的工作图审计投影。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditCommandRun {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stderr: String,
}

/// Resolved command anchors selected for a work-graph node profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphAuditCommands {
    /// Commands executed by the compile anchor.
    pub compile_commands: Vec<String>,
    /// Commands executed by the test anchor.
    pub test_commands: Vec<String>,
    /// Commands executed by the final verification gate.
    pub verify_commands: Vec<String>,
}

/// 工作图节点执行期间记录的一条审计事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphAuditEvent {
    pub node_id: String,
    pub attempt: u32,
    pub kind: GraphAuditKind,
    pub anchor: Option<GraphAuditAnchor>,
    pub commands: Vec<AuditCommandRun>,
    pub route: Option<GraphAuditRoute>,
    pub profile: Option<GraphAuditProfile>,
    /// Present on `profile_resolved`; absent from historical events.
    #[serde(default)]
    pub resolved_commands: Option<GraphAuditCommands>,
    pub budget: Option<Budget>,
    pub timestamp: String,
}

/// 工作图审计事件类别。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphAuditKind {
    ProfileResolved,
    AnchorCompleted,
    RouteSelected,
}

/// 已执行的外部锚点类别。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphAuditAnchor {
    Compile,
    Test,
    Verify,
}

/// 工作图下一步路由决策。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphAuditRoute {
    RootCause,
    Implement,
    CompileAnchor,
    TestAnchor,
    VerifyGate,
    HumanReview,
    Complete,
    Escalate,
}

/// 工作图执行使用的项目配置。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphAuditProfile {
    None,
    Rust,
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
    SpecialistReports,
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
    /// - Verification：执行外部锚点 → 写 compile_result/test_result/verify_result；读
    ///   requirement/verify_result/compile_result/test_result/step_log。step_log 由授权写
    ///   自动记入，不直接 set。
    /// - GeneralPurpose（协调/工作节点）：写 generated_diff/budget；广泛读工作产物。
    /// - Explore / Plan：读 requirement，写自己的 specialist_reports。
    /// - WgentyCodeGuide：只读 requirement，不写任何字段。
    ///
    /// human_review 对所有现存 NodeType 的 writable 都为 {}（仍需人工审批节点）。
    pub fn field_perms(&self) -> FieldPerms {
        match self {
            NodeType::Verification => FieldPerms {
                readable: [
                    WorkField::Requirement,
                    WorkField::VerifyResult,
                    WorkField::CompileResult,
                    WorkField::TestResult,
                    WorkField::SpecialistReports,
                    WorkField::StepLog,
                ]
                .into_iter()
                .collect(),
                writable: [
                    WorkField::CompileResult,
                    WorkField::TestResult,
                    WorkField::VerifyResult,
                ]
                .into_iter()
                .collect(),
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
                    WorkField::SpecialistReports,
                    WorkField::StepLog,
                ]
                .into_iter()
                .collect(),
                writable: [WorkField::GeneratedDiff, WorkField::Budget]
                    .into_iter()
                    .collect(),
            },
            NodeType::RootCause => FieldPerms {
                readable: [
                    WorkField::Requirement,
                    WorkField::GeneratedDiff,
                    WorkField::VerifyResult,
                    WorkField::CompileResult,
                    WorkField::TestResult,
                    WorkField::SpecialistReports,
                ]
                .into_iter()
                .collect(),
                writable: [WorkField::SpecialistReports].into_iter().collect(),
            },
            NodeType::HumanReview => FieldPerms {
                readable: [
                    WorkField::Requirement,
                    WorkField::VerifyResult,
                    WorkField::HumanReview,
                ]
                .into_iter()
                .collect(),
                writable: [WorkField::HumanReview].into_iter().collect(),
            },
            NodeType::Explore | NodeType::Plan => FieldPerms {
                readable: [WorkField::Requirement].into_iter().collect(),
                writable: [WorkField::SpecialistReports].into_iter().collect(),
            },
            NodeType::WgentyCodeGuide => FieldPerms {
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

    /// 读取跨 checkpoint 保留的工作图审计记录。
    pub fn graph_audit(&self) -> &[GraphAuditEvent] {
        &self.graph_audit
    }

    /// Return the code-selected Work-Graph persisted for the active node.
    pub fn selected_work_graph(&self) -> Option<&WorkGraphPlan> {
        self.selected_work_graph.as_ref()
    }

    pub fn graph_child_bindings(&self) -> &[GraphChildBinding] {
        &self.graph_child_bindings
    }

    pub(crate) fn append_graph_child_binding(&mut self, binding: GraphChildBinding) {
        self.graph_child_bindings.push(binding);
    }

    /// Persist a bounded Work-Graph selected by the trusted coordinator.
    pub(crate) fn set_selected_work_graph(&mut self, plan: WorkGraphPlan) {
        self.selected_work_graph = Some(plan);
    }

    /// Read all persisted specialist reports when the caller's contract allows
    /// access to the shared handoff field.
    pub fn specialist_reports(
        &self,
        caller: NodeType,
    ) -> Result<&[SpecialistReport], CoordinatorError> {
        if !caller
            .field_perms()
            .readable
            .contains(&WorkField::SpecialistReports)
        {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read specialist_reports".into(),
            });
        }
        Ok(&self.specialist_reports)
    }

    /// Persist one validated specialist report through the caller's field
    /// permission. A repeated `(producer, kind)` handoff replaces its prior
    /// report so retries do not leave ambiguous competing products.
    pub fn set_specialist_report(
        &mut self,
        caller: NodeType,
        report: SpecialistReport,
    ) -> Result<(), CoordinatorError> {
        if !caller
            .field_perms()
            .writable
            .contains(&WorkField::SpecialistReports)
        {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to write specialist_reports".into(),
            });
        }
        if report.producer != caller {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "specialist report producer does not match trusted caller".into(),
            });
        }
        validate_specialist_report_kind(&caller, report.kind)?;
        validate_specialist_report_contents(&report)?;

        if let Some(existing) = self
            .specialist_reports
            .iter_mut()
            .find(|existing| existing.producer == report.producer && existing.kind == report.kind)
        {
            *existing = report;
        } else {
            self.specialist_reports.push(report);
        }
        self.push_step(caller, WorkField::SpecialistReports, StepAction::Wrote);
        Ok(())
    }

    /// coordinator 专用：追加一条工作图审计事件。
    pub(crate) fn append_graph_audit(&mut self, event: GraphAuditEvent) {
        self.graph_audit.push(event);
    }

    /// 写 verify_result：pilot 核心字段。查 caller 的 field_perms，越权 →
    /// ContractViolation{State}。写成功自动追加 step_log。
    pub fn set_verify_result(
        &mut self,
        caller: NodeType,
        outcome: VerifyOutcome,
    ) -> Result<(), CoordinatorError> {
        if !caller
            .field_perms()
            .writable
            .contains(&WorkField::VerifyResult)
        {
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
        if !caller
            .field_perms()
            .readable
            .contains(&WorkField::VerifyResult)
        {
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
        if !caller
            .field_perms()
            .writable
            .contains(&WorkField::GeneratedDiff)
        {
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
        if !caller
            .field_perms()
            .readable
            .contains(&WorkField::GeneratedDiff)
        {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read generated_diff".into(),
            });
        }
        Ok(self.generated_diff.as_ref())
    }

    /// 写 budget：deferred 字段。
    pub fn set_budget(&mut self, caller: NodeType, budget: Budget) -> Result<(), CoordinatorError> {
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

    /// 写 compile_result：仅 Verification 外部锚点可写。
    pub fn set_compile_result(
        &mut self,
        caller: NodeType,
        result: CompileResult,
    ) -> Result<(), CoordinatorError> {
        if !caller
            .field_perms()
            .writable
            .contains(&WorkField::CompileResult)
        {
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
        if !caller
            .field_perms()
            .readable
            .contains(&WorkField::CompileResult)
        {
            return Err(CoordinatorError::ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,
                reason: "node type not permitted to read compile_result".into(),
            });
        }
        Ok(self.compile_result.as_ref())
    }

    /// 写 test_result：仅 Verification 外部锚点可写。
    pub fn set_test_result(
        &mut self,
        caller: NodeType,
        result: TestResult,
    ) -> Result<(), CoordinatorError> {
        if !caller
            .field_perms()
            .writable
            .contains(&WorkField::TestResult)
        {
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

    pub fn test_result(&self, caller: NodeType) -> Result<Option<&TestResult>, CoordinatorError> {
        if !caller
            .field_perms()
            .readable
            .contains(&WorkField::TestResult)
        {
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
        if !caller
            .field_perms()
            .writable
            .contains(&WorkField::HumanReview)
        {
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

    pub fn human_review(&self, caller: NodeType) -> Result<Option<&HumanReview>, CoordinatorError> {
        if !caller
            .field_perms()
            .readable
            .contains(&WorkField::HumanReview)
        {
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

    /// Start a new node without inheriting products or retry accounting from
    /// the previously completed node in the same turn. The turn-level
    /// requirement and audit log remain intact.
    pub fn reset_for_new_node(&mut self) {
        self.generated_diff = None;
        self.compile_result = None;
        self.test_result = None;
        self.verify_result = None;
        self.budget = None;
        self.human_review = None;
        self.specialist_reports.clear();
        self.selected_work_graph = None;
    }

    /// Invalidate every result derived from a prior work-graph pass before a
    /// fresh compile anchor runs. Retry budget is deliberately retained.
    pub fn reset_for_work_graph_pass(&mut self) {
        self.generated_diff = None;
        self.compile_result = None;
        self.test_result = None;
        self.verify_result = None;
        self.human_review = None;
        self.specialist_reports.clear();
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
            graph_audit: self.graph_audit.clone(),
            specialist_reports: self.specialist_reports.clone(),
            graph_child_bindings: self.graph_child_bindings.clone(),
            selected_work_graph: self.selected_work_graph.clone(),
        }
    }
}

fn validate_specialist_report_kind(
    caller: &NodeType,
    kind: SpecialistReportKind,
) -> Result<(), CoordinatorError> {
    let allowed = matches!(
        (caller, kind),
        (NodeType::Explore, SpecialistReportKind::Exploration)
            | (NodeType::RootCause, SpecialistReportKind::RootCause)
            | (NodeType::Plan, SpecialistReportKind::ImplementationPlan)
    );
    if allowed {
        return Ok(());
    }
    Err(CoordinatorError::ContractViolation {
        node_type: caller.clone(),
        dimension: ContractDimension::State,
        reason: "specialist report kind is not allowed for node type".into(),
    })
}

fn validate_specialist_report_contents(report: &SpecialistReport) -> Result<(), CoordinatorError> {
    let contents_valid = !report.summary.trim().is_empty()
        && !report.evidence.is_empty()
        && report
            .evidence
            .iter()
            .all(|evidence| !evidence.path.trim().is_empty() && !evidence.detail.trim().is_empty())
        && !report.recommended_actions.is_empty()
        && report
            .recommended_actions
            .iter()
            .all(|action| !action.trim().is_empty());
    let mut paths = HashSet::new();
    let suspected_files_unique = report
        .suspected_files
        .iter()
        .all(|path| !path.trim().is_empty() && paths.insert(path));
    if contents_valid && suspected_files_unique {
        return Ok(());
    }
    Err(CoordinatorError::ContractViolation {
        node_type: report.producer.clone(),
        dimension: ContractDimension::State,
        reason: "specialist report must contain non-empty evidence/actions and unique files".into(),
    })
}

impl VerifyOutcome {
    /// 从「外部强类型 verify 结果」投影构造。exec_session 侧负责字段映射（见
    /// node_runtime.rs 的 project_failure / project_outcome），调用本方法传入
    /// 已解构的原语字段，避免 org_graph 反向依赖 exec_session::VerifyResult。
    pub fn from_parts(success: bool, fail_kind: Option<VerifyFailureKind>) -> Self {
        Self {
            success,
            fail_reason: fail_kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn exploration_report() -> SpecialistReport {
        SpecialistReport {
            producer: NodeType::Explore,
            kind: SpecialistReportKind::Exploration,
            summary: "the parser owns the failing branch".into(),
            evidence: vec![SpecialistEvidence {
                path: "src/parser.rs".into(),
                detail: "parse() rejects empty input".into(),
            }],
            suspected_files: vec!["src/parser.rs".into()],
            recommended_actions: vec!["add an empty-input regression test".into()],
        }
    }

    #[test]
    fn specialist_report_is_authorized_checkpoint_compatible_and_inherited() {
        let mut state = WorkState::default();
        state
            .set_specialist_report(NodeType::Explore, exploration_report())
            .expect("explore may write an exploration report");
        assert_eq!(
            state
                .specialist_reports(NodeType::GeneralPurpose)
                .expect("general purpose may read reports")
                .len(),
            1
        );
        assert_eq!(
            state.step_log().last().expect("report write log").field,
            WorkField::SpecialistReports
        );

        state.reset_for_work_graph_pass();
        assert!(state
            .specialist_reports(NodeType::GeneralPurpose)
            .expect("general purpose may read reports")
            .is_empty());
        state
            .set_specialist_report(NodeType::Explore, exploration_report())
            .expect("record retry report");
        let inherited = state.inherit_for_new_turn();
        assert_eq!(
            inherited
                .specialist_reports(NodeType::GeneralPurpose)
                .expect("general purpose may read inherited reports"),
            state
                .specialist_reports(NodeType::GeneralPurpose)
                .expect("general purpose may read current reports")
        );
        assert_eq!(
            serde_json::from_str::<WorkState>(&serde_json::to_string(&inherited).unwrap()).unwrap(),
            inherited
        );
    }

    #[test]
    fn specialist_report_rejects_spoofed_producer_and_invalid_contents_without_mutation() {
        let mut state = WorkState::default();
        let mut report = exploration_report();
        report.producer = NodeType::Plan;
        assert!(state
            .set_specialist_report(NodeType::Explore, report)
            .is_err());
        assert!(state
            .specialist_reports(NodeType::GeneralPurpose)
            .expect("general purpose may read reports")
            .is_empty());

        let mut report = exploration_report();
        report.recommended_actions.clear();
        assert!(state
            .set_specialist_report(NodeType::Explore, report)
            .is_err());
        assert!(state
            .specialist_reports(NodeType::GeneralPurpose)
            .expect("general purpose may read reports")
            .is_empty());
    }

    fn event(node_id: &str, attempt: u32, kind: GraphAuditKind) -> GraphAuditEvent {
        GraphAuditEvent {
            node_id: node_id.into(),
            attempt,
            kind,
            anchor: Some(GraphAuditAnchor::Compile),
            commands: vec![AuditCommandRun {
                command: "cargo check".into(),
                exit_code: Some(0),
                stderr: String::new(),
            }],
            route: Some(GraphAuditRoute::CompileAnchor),
            profile: Some(GraphAuditProfile::Rust),
            resolved_commands: None,
            budget: Some(Budget {
                max_iter: 3,
                iter_used: 1,
                token_used: 42,
            }),
            timestamp: "2026-08-12T00:00:00Z".into(),
        }
    }

    #[test]
    fn graph_audit_round_trips_and_survives_all_work_state_resets() {
        let mut state = WorkState::default();
        state.append_graph_audit(event("n1", 1, GraphAuditKind::ProfileResolved));
        state.reset_for_new_node();
        state.reset_for_work_graph_pass();
        let inherited = state.inherit_for_new_turn();

        assert_eq!(inherited.graph_audit().len(), 1);
        assert_eq!(
            serde_json::from_str::<WorkState>(&serde_json::to_string(&inherited).unwrap()).unwrap(),
            inherited
        );
    }

    #[test]
    fn graph_audit_enums_serialize_with_snake_case_values() {
        assert_eq!(
            serde_json::to_string(&GraphAuditKind::ProfileResolved).unwrap(),
            "\"profile_resolved\""
        );
        assert_eq!(
            serde_json::to_string(&GraphAuditAnchor::Compile).unwrap(),
            "\"compile\""
        );
        assert_eq!(
            serde_json::to_string(&GraphAuditRoute::CompileAnchor).unwrap(),
            "\"compile_anchor\""
        );
        assert_eq!(
            serde_json::to_string(&GraphAuditProfile::Rust).unwrap(),
            "\"rust\""
        );
    }

    #[test]
    fn historical_graph_audit_event_defaults_missing_resolved_commands() {
        let json = r#"{
            "node_id":"n1",
            "attempt":1,
            "kind":"profile_resolved",
            "anchor":null,
            "commands":[],
            "route":null,
            "profile":"rust",
            "budget":null,
            "timestamp":"2026-08-12T00:00:00Z"
        }"#;

        let event: GraphAuditEvent = serde_json::from_str(json).expect("deserialize old event");
        assert_eq!(event.resolved_commands, None);
    }

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
            graph_audit: vec![event("verify", 1, GraphAuditKind::AnchorCompleted)],
            specialist_reports: Vec::new(),
            graph_child_bindings: Vec::new(),
            selected_work_graph: None,
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
        let cr = CompileResult {
            ok: true,
            stderr: String::new(),
        };
        let back: CompileResult =
            serde_json::from_str(&serde_json::to_string(&cr).unwrap()).unwrap();
        assert_eq!(cr, back);

        // TestResult
        let tr = TestResult {
            pass: false,
            failed_cases: vec!["c1".into()],
        };
        let back: TestResult = serde_json::from_str(&serde_json::to_string(&tr).unwrap()).unwrap();
        assert_eq!(tr, back);

        // HumanReview（两个变体）
        for hr in [HumanReview::Approve, HumanReview::Reject] {
            let back: HumanReview =
                serde_json::from_str(&serde_json::to_string(&hr).unwrap()).unwrap();
            assert_eq!(hr, back);
        }

        // Budget
        let b = Budget {
            max_iter: 3,
            iter_used: 1,
            token_used: 500,
        };
        let back: Budget = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
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
    fn workfield_has_nine_variants() {
        // 全字段权限矩阵依赖 9 个 WorkField 变体；缺一个会让 Task 2 矩阵不完整。
        let all = [
            WorkField::Requirement,
            WorkField::GeneratedDiff,
            WorkField::CompileResult,
            WorkField::TestResult,
            WorkField::HumanReview,
            WorkField::VerifyResult,
            WorkField::Budget,
            WorkField::SpecialistReports,
            WorkField::StepLog,
        ];
        // 9 个互异（Hash 去重后仍 9 个）。
        let set: HashSet<WorkField> = all.iter().copied().collect();
        assert_eq!(set.len(), 9);
    }

    #[test]
    fn field_perms_verification_writes_verify_result_reads_broad() {
        let perms = NodeType::Verification.field_perms();
        // 写：三个外部锚点结果（step_log 由授权写自动记入，不直接 set）。
        assert!(perms.writable.contains(&WorkField::VerifyResult));
        assert!(perms.writable.contains(&WorkField::CompileResult));
        assert!(perms.writable.contains(&WorkField::TestResult));
        assert!(!perms.writable.contains(&WorkField::StepLog));
        // 读：requirement/verify_result/compile_result/test_result/step_log。
        for f in [
            WorkField::Requirement,
            WorkField::VerifyResult,
            WorkField::CompileResult,
            WorkField::TestResult,
            WorkField::StepLog,
        ] {
            assert!(
                perms.readable.contains(&f),
                "Verification should read {f:?}"
            );
        }
        // 人工审批字段对 Verification 不可写。
        let f = WorkField::HumanReview;
        assert!(
            !perms.writable.contains(&f),
            "Verification must not write {f:?}"
        );
    }

    #[test]
    fn verification_can_write_compile_and_test_anchor_results() {
        let mut state = WorkState::default();
        state
            .set_compile_result(
                NodeType::Verification,
                CompileResult {
                    ok: false,
                    stderr: "compile failed".into(),
                },
            )
            .expect("verification anchor should record compile result");
        state
            .set_test_result(
                NodeType::Verification,
                TestResult {
                    pass: false,
                    failed_cases: vec!["unit::fails".into()],
                },
            )
            .expect("verification anchor should record test result");

        assert!(state
            .compile_result(NodeType::Verification)
            .expect("verification may read compile result")
            .is_some());
        assert!(state
            .test_result(NodeType::Verification)
            .expect("verification may read test result")
            .is_some());
    }

    #[test]
    fn field_perms_generalpurpose_writes_diff_budget_reads_all() {
        let perms = NodeType::GeneralPurpose.field_perms();
        // 写：generated_diff + budget。
        assert!(perms.writable.contains(&WorkField::GeneratedDiff));
        assert!(perms.writable.contains(&WorkField::Budget));
        // 读：全部工作产物字段（协调者需广视野）。
        for f in [
            WorkField::Requirement,
            WorkField::GeneratedDiff,
            WorkField::VerifyResult,
            WorkField::CompileResult,
            WorkField::TestResult,
            WorkField::HumanReview,
            WorkField::Budget,
            WorkField::SpecialistReports,
            WorkField::StepLog,
        ] {
            assert!(
                perms.readable.contains(&f),
                "GeneralPurpose should read {f:?}"
            );
        }
        // GeneralPurpose 不可写 verify_result / 预留字段。
        assert!(!perms.writable.contains(&WorkField::VerifyResult));
        assert!(!perms.writable.contains(&WorkField::HumanReview));
    }

    #[test]
    fn field_perms_specialists_write_reports_while_guide_remains_read_only() {
        for nt in [NodeType::Explore, NodeType::Plan] {
            let perms = nt.field_perms();
            assert!(perms.readable.contains(&WorkField::Requirement));
            assert_eq!(
                perms.readable.len(),
                1,
                "{nt:?} should only read requirement"
            );
            assert_eq!(
                perms.writable,
                [WorkField::SpecialistReports].into_iter().collect(),
                "{nt:?} should write only specialist reports"
            );
        }

        let guide = NodeType::WgentyCodeGuide.field_perms();
        assert_eq!(
            guide.readable,
            [WorkField::Requirement].into_iter().collect()
        );
        assert!(guide.writable.is_empty());
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
        let outcome = VerifyOutcome {
            success: true,
            fail_reason: None,
        };
        let err = state
            .set_verify_result(NodeType::Explore, outcome.clone())
            .expect_err("Explore must not write verify_result");
        match err {
            crate::agent::coordinator::CoordinatorError::ContractViolation {
                dimension,
                reason,
                ..
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
        let diff = GeneratedDiff {
            summary: "改 1 文件".into(),
            files: vec!["a.rs".into()],
        };
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
        let budget = Budget {
            max_iter: 5,
            iter_used: 1,
            token_used: 100,
        };
        state
            .set_budget(NodeType::GeneralPurpose, budget.clone())
            .expect("GeneralPurpose authorized to write budget");
        assert_eq!(state.budget, Some(budget));
    }

    #[test]
    fn unauthorized_nodes_cannot_write_anchor_or_human_review_fields() {
        // 真强制核心保证：只有 Verification 能写外部锚点，HumanReview 对全部节点
        // 不可写。逐一验证越权写返回 ContractViolation{State}。
        let all_nodes = [
            NodeType::Explore,
            NodeType::Plan,
            NodeType::GeneralPurpose,
            NodeType::WgentyCodeGuide,
        ];
        for nt in all_nodes {
            let mut state = WorkState::default();
            let err = state
                .set_compile_result(
                    nt.clone(),
                    CompileResult {
                        ok: true,
                        stderr: String::new(),
                    },
                )
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
                .set_test_result(
                    nt.clone(),
                    TestResult {
                        pass: true,
                        failed_cases: vec![],
                    },
                )
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
        let state = WorkState {
            verify_result: Some(VerifyOutcome {
                success: true,
                fail_reason: None,
            }),
            ..Default::default()
        };
        let got = state
            .verify_result(NodeType::Verification)
            .expect("Verification authorized to read verify_result");
        assert!(got.is_some());
        assert!(state.step_log.is_empty(), "read must not append step_log");
    }

    #[test]
    fn verify_result_read_rejects_unauthorized_node() {
        let state = WorkState {
            verify_result: Some(VerifyOutcome {
                success: true,
                fail_reason: None,
            }),
            ..Default::default()
        };
        let err = state
            .verify_result(NodeType::Explore)
            .expect_err("Explore must not read verify_result");
        match err {
            crate::agent::coordinator::CoordinatorError::ContractViolation {
                dimension, ..
            } => {
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
            generated_diff: Some(GeneratedDiff {
                summary: "s".into(),
                files: vec![],
            }),
            compile_result: Some(CompileResult {
                ok: true,
                stderr: String::new(),
            }),
            test_result: Some(TestResult {
                pass: true,
                failed_cases: vec![],
            }),
            human_review: Some(HumanReview::Approve),
            verify_result: Some(VerifyOutcome {
                success: true,
                fail_reason: None,
            }),
            budget: Some(Budget {
                max_iter: 1,
                iter_used: 1,
                token_used: 1,
            }),
            step_log: vec![StepRecord {
                node_type: NodeType::Verification,
                field: WorkField::VerifyResult,
                action: StepAction::Wrote,
                timestamp: "2026-08-11T00:00:00Z".into(),
            }],
            graph_audit: vec![event("verify", 1, GraphAuditKind::AnchorCompleted)],
            specialist_reports: Vec::new(),
            graph_child_bindings: Vec::new(),
            selected_work_graph: None,
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
        assert_eq!(next.graph_audit.len(), 1);
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
            Some(VerifyFailureKind::CommandFailed {
                exit_code: Some(1),
                stderr: "e".into(),
            }),
        );
        assert!(!outcome.success);
        assert!(matches!(
            outcome.fail_reason,
            Some(VerifyFailureKind::CommandFailed {
                exit_code: Some(1),
                ..
            })
        ));
    }
}
