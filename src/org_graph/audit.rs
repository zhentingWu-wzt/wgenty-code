//! Pure, deterministic evaluation of persisted Work-Graph audit records.

use serde::{Deserialize, Serialize};

use super::{GraphAuditAnchor, GraphAuditEvent, GraphAuditKind, GraphAuditRoute};

/// Aggregate outcome counters for one persisted Work-Graph run or a collection
/// of runs. Counts are derived solely from durable audit events.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkGraphAuditSummary {
    /// Number of resolved verification profiles.
    pub profiles_resolved: usize,
    /// Number of completed external anchors.
    pub anchors_completed: usize,
    /// Compile anchors that exited unsuccessfully.
    pub compile_failures: usize,
    /// Test anchors that exited unsuccessfully.
    pub test_failures: usize,
    /// Final verification anchors that exited unsuccessfully.
    pub verify_failures: usize,
    /// Number of code-owned RootCause routes selected.
    pub root_cause_routes: usize,
    /// Number of routes released to implementation.
    pub implement_routes: usize,
    /// Number of completed graph runs.
    pub completed_routes: usize,
    /// Number of fail-closed escalation routes.
    pub escalated_routes: usize,
}

impl WorkGraphAuditSummary {
    /// Summarizes graph behavior without interpreting model output or mutable
    /// runtime state.
    pub fn from_events(events: &[GraphAuditEvent]) -> Self {
        let mut summary = Self::default();
        for event in events {
            match event.kind {
                GraphAuditKind::ProfileResolved => summary.profiles_resolved += 1,
                GraphAuditKind::AnchorCompleted => {
                    summary.anchors_completed += 1;
                    if event
                        .commands
                        .iter()
                        .all(|command| command.exit_code == Some(0))
                    {
                        continue;
                    }
                    match event.anchor {
                        Some(GraphAuditAnchor::Compile) => summary.compile_failures += 1,
                        Some(GraphAuditAnchor::Test) => summary.test_failures += 1,
                        Some(GraphAuditAnchor::Verify) => summary.verify_failures += 1,
                        None => {}
                    }
                }
                GraphAuditKind::RouteSelected => match event.route {
                    Some(GraphAuditRoute::RootCause) => summary.root_cause_routes += 1,
                    Some(GraphAuditRoute::Implement) => summary.implement_routes += 1,
                    Some(GraphAuditRoute::Complete) => summary.completed_routes += 1,
                    Some(GraphAuditRoute::Escalate) => summary.escalated_routes += 1,
                    _ => {}
                },
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org_graph::{AuditCommandRun, Budget, GraphAuditProfile, GraphAuditRoute};

    fn event(
        kind: GraphAuditKind,
        anchor: Option<GraphAuditAnchor>,
        route: Option<GraphAuditRoute>,
        exit_code: Option<i32>,
    ) -> GraphAuditEvent {
        GraphAuditEvent {
            node_id: "node-1".into(),
            attempt: 1,
            kind,
            anchor,
            commands: exit_code
                .map(|exit_code| {
                    vec![AuditCommandRun {
                        command: "cargo test".into(),
                        exit_code: Some(exit_code),
                        stderr: String::new(),
                    }]
                })
                .unwrap_or_default(),
            route,
            profile: Some(GraphAuditProfile::Rust),
            resolved_commands: None,
            budget: Some(Budget {
                max_iter: 2,
                iter_used: 1,
                token_used: 0,
            }),
            timestamp: "2026-08-12T00:00:00Z".into(),
        }
    }

    #[test]
    fn summary_counts_anchors_and_terminal_routes_from_durable_events() {
        let events = [
            event(GraphAuditKind::ProfileResolved, None, None, None),
            event(
                GraphAuditKind::AnchorCompleted,
                Some(GraphAuditAnchor::Compile),
                None,
                Some(1),
            ),
            event(
                GraphAuditKind::RouteSelected,
                None,
                Some(GraphAuditRoute::RootCause),
                None,
            ),
            event(
                GraphAuditKind::RouteSelected,
                None,
                Some(GraphAuditRoute::Implement),
                None,
            ),
            event(
                GraphAuditKind::RouteSelected,
                None,
                Some(GraphAuditRoute::Complete),
                None,
            ),
        ];

        assert_eq!(
            WorkGraphAuditSummary::from_events(&events),
            WorkGraphAuditSummary {
                profiles_resolved: 1,
                anchors_completed: 1,
                compile_failures: 1,
                test_failures: 0,
                verify_failures: 0,
                root_cause_routes: 1,
                implement_routes: 1,
                completed_routes: 1,
                escalated_routes: 0,
            }
        );
    }
}
