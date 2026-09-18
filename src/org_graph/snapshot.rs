//! 只读 Work-Graph 快照：供 UI 可视化（Web WorkGraphPanel）消费的纯数据投影。
//!
//! 本模块只依赖 org_graph 自身类型（plan / 审计事件 / 分解单元），不反向依赖
//! exec_session——节点链条目由上层（runtime_store）从 `Node` 投影为
//! [`NodeChainEntry`] 后传入。全部字段可 serde 往返，无 async / I/O。

use serde::{Deserialize, Serialize};

use super::audit::WorkGraphAuditSummary;
use super::{DecomposedUnit, GraphAuditEvent, WorkGraphPlan};

/// 会话节点链上的一条节点（从 `exec_session::Node` 投影，剥离命令细节）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeChainEntry {
    pub id: String,
    /// 节点契约里的目标描述。
    pub goal: String,
    /// snake_case 状态标签（pending / running / verifying / verified / failed）。
    pub status: String,
    pub retry_count: u32,
    /// 节点起始 turn（演进历史对齐用）。
    pub start_turn_id: String,
    pub created_at: String,
}

/// 单个会话的 Work-Graph 可视化快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionGraphSnapshot {
    pub session_id: String,
    /// 本图在递归分解树中的深度（根图为 0）。
    pub graph_depth: u32,
    /// 当前节点选中的 Work-Graph 计划；无活动节点时为 `None`。
    pub plan: Option<WorkGraphPlan>,
    /// 线性节点链（session.json 的 node_states，按创建顺序）。
    pub nodes: Vec<NodeChainEntry>,
    /// 活动节点的分解子单元（含终态 outcome）。
    pub units: Vec<DecomposedUnit>,
    /// 由全部持久化审计事件推导的汇总计数。
    pub audit_summary: WorkGraphAuditSummary,
    /// 尾部审计事件（最旧在前，最多 `recent_event_cap` 条），供演进时间线。
    pub recent_events: Vec<GraphAuditEvent>,
}

/// 快照端点响应信封（预留后续分页/过滤字段）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkGraphSnapshotResponse {
    pub sessions: Vec<SessionGraphSnapshot>,
}

/// 组装单个会话的图快照。纯函数：只读入参，输出确定性投影。
pub fn build_session_snapshot(
    session_id: String,
    graph_depth: u32,
    plan: Option<&WorkGraphPlan>,
    nodes: Vec<NodeChainEntry>,
    units: &[DecomposedUnit],
    events: &[GraphAuditEvent],
    recent_event_cap: usize,
) -> SessionGraphSnapshot {
    let recent_start = events.len().saturating_sub(recent_event_cap);
    SessionGraphSnapshot {
        session_id,
        graph_depth,
        plan: plan.cloned(),
        nodes,
        units: units.to_vec(),
        audit_summary: WorkGraphAuditSummary::from_events(events),
        recent_events: events[recent_start..].to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org_graph::{
        AuditCommandRun, Budget, GraphAuditAnchor, GraphAuditKind, GraphAuditProfile,
        GraphAuditRoute, WorkGraphRequest,
    };

    fn event(node_id: &str, kind: GraphAuditKind) -> GraphAuditEvent {
        GraphAuditEvent {
            node_id: node_id.into(),
            attempt: 1,
            kind,
            anchor: Some(GraphAuditAnchor::Test),
            commands: vec![AuditCommandRun {
                command: "cargo test".into(),
                exit_code: Some(1),
                stderr: String::new(),
            }],
            route: Some(GraphAuditRoute::TestAnchor),
            profile: Some(GraphAuditProfile::Rust),
            resolved_commands: None,
            adapted: None,
            parent_node_id: None,
            budget: Some(Budget {
                max_iter: 3,
                iter_used: 1,
                token_used: 0,
            }),
            timestamp: "2026-09-18T00:00:00Z".into(),
        }
    }

    fn node_chain(id: &str) -> Vec<NodeChainEntry> {
        vec![NodeChainEntry {
            id: id.into(),
            goal: "visualize the graph".into(),
            status: "running".into(),
            retry_count: 0,
            start_turn_id: "turn-0".into(),
            created_at: "2026-09-18T00:00:00Z".into(),
        }]
    }

    #[test]
    fn snapshot_summarizes_audit_and_caps_recent_events() {
        let plan = crate::org_graph::compose_work_graph(&WorkGraphRequest::default())
            .expect("compose default plan");
        let events = vec![
            event("n1", GraphAuditKind::ProfileResolved),
            event("n1", GraphAuditKind::AnchorCompleted),
            event("n1", GraphAuditKind::AnchorCompleted),
        ];
        let snapshot = build_session_snapshot(
            "s-viz".into(),
            0,
            Some(&plan),
            node_chain("n1"),
            &[],
            &events,
            2,
        );

        assert_eq!(snapshot.session_id, "s-viz");
        assert_eq!(snapshot.audit_summary.profiles_resolved, 1);
        assert_eq!(snapshot.audit_summary.anchors_completed, 2);
        assert_eq!(snapshot.audit_summary.test_failures, 2);
        assert_eq!(snapshot.recent_events.len(), 2, "recent events capped");
        assert_eq!(
            snapshot.recent_events[0].kind,
            GraphAuditKind::AnchorCompleted,
            "cap keeps the newest tail, oldest-first"
        );
        assert_eq!(snapshot.nodes.len(), 1);
        assert!(snapshot.plan.is_some());

        let json = serde_json::to_string(&snapshot).expect("serialize");
        let round: SessionGraphSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round, snapshot);
    }

    #[test]
    fn snapshot_without_active_plan_serializes_null_plan() {
        let snapshot = build_session_snapshot("s-idle".into(), 0, None, Vec::new(), &[], &[], 50);
        assert!(snapshot.plan.is_none());
        assert_eq!(snapshot.audit_summary, WorkGraphAuditSummary::default());

        let json = serde_json::to_string(&snapshot).expect("serialize");
        assert!(json.contains("\"plan\":null"), "null plan stays explicit");
    }

    #[test]
    fn snapshot_response_envelope_wraps_sessions() {
        let response = WorkGraphSnapshotResponse {
            sessions: vec![build_session_snapshot(
                "s-1".into(),
                0,
                None,
                node_chain("n1"),
                &[],
                &[],
                50,
            )],
        };
        let json = serde_json::to_value(&response).expect("serialize");
        let sessions = json
            .get("sessions")
            .and_then(|s| s.as_array())
            .expect("sessions array");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0]
                .get("nodes")
                .and_then(|n| n.as_array())
                .expect("nodes")[0]
                .get("status")
                .and_then(|s| s.as_str()),
            Some("running")
        );
    }
}
