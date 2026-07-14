//! Task dependency graph helpers (`blockedBy` edges).
//!
//! Edge direction: `A ∈ B.blocked_by` means **A blocks B** (B cannot start until
//! A is completed). Pure functions over an in-memory task map — no I/O.

use super::types::{Task, TaskStatus};
use std::collections::{HashMap, HashSet};

/// Whether `task` is currently blocked by at least one unfinished dependency.
///
/// Missing blocker IDs are treated as still-blocking (fail closed) so a stale
/// reference cannot silently unlock work.
pub fn is_blocked(task: &Task, all: &HashMap<String, Task>) -> bool {
    if task.status == TaskStatus::Completed || task.status == TaskStatus::Deleted {
        return false;
    }
    task.blocked_by.iter().any(|blocker_id| {
        match all.get(blocker_id) {
            Some(blocker) => blocker.status != TaskStatus::Completed,
            // Unknown blocker keeps the task blocked until dependencies are fixed.
            None => true,
        }
    })
}

/// IDs of blockers that are not yet completed (including missing IDs).
pub fn open_blockers(task: &Task, all: &HashMap<String, Task>) -> Vec<String> {
    task.blocked_by
        .iter()
        .filter(|id| match all.get(*id) {
            Some(b) => b.status != TaskStatus::Completed,
            None => true,
        })
        .cloned()
        .collect()
}

/// Tasks that still have open blockers (pending / in_progress only).
pub fn blocked_tasks(all: &HashMap<String, Task>) -> Vec<Task> {
    all.values()
        .filter(|t| {
            t.status != TaskStatus::Deleted
                && t.status != TaskStatus::Completed
                && is_blocked(t, all)
        })
        .cloned()
        .collect()
}

/// Tasks that are pending (or optionally in_progress) and have no open blockers.
pub fn ready_tasks(all: &HashMap<String, Task>) -> Vec<Task> {
    all.values()
        .filter(|t| {
            matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress) && !is_blocked(t, all)
        })
        .cloned()
        .collect()
}

/// Detect a cycle if `task_id` adopted `proposed_blocked_by` as its blockers.
///
/// Walks ancestors of each proposed blocker; if `task_id` is reachable, the
/// assignment would create a cycle.
pub fn would_create_cycle(
    task_id: &str,
    proposed_blocked_by: &[String],
    all: &HashMap<String, Task>,
) -> bool {
    // Self-dependency is always a cycle.
    if proposed_blocked_by.iter().any(|b| b == task_id) {
        return true;
    }

    let mut stack: Vec<&str> = proposed_blocked_by.iter().map(|s| s.as_str()).collect();
    let mut visited: HashSet<String> = HashSet::new();

    while let Some(current) = stack.pop() {
        if current == task_id {
            return true;
        }
        if !visited.insert(current.to_string()) {
            continue;
        }
        if let Some(node) = all.get(current) {
            for parent in &node.blocked_by {
                stack.push(parent.as_str());
            }
        }
    }
    false
}

/// Validate that every blocker ID exists and is not deleted.
pub fn validate_blockers_exist(
    blocked_by: &[String],
    all: &HashMap<String, Task>,
) -> Result<(), String> {
    for id in blocked_by {
        match all.get(id) {
            None => return Err(format!("Blocker task not found: {id}")),
            Some(t) if t.status == TaskStatus::Deleted => {
                return Err(format!("Blocker task is deleted: {id}"));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::TaskPriority;
    use chrono::Utc;

    fn task(id: &str, blocked_by: &[&str], status: TaskStatus) -> Task {
        Task {
            id: id.to_string(),
            subject: id.to_string(),
            description: String::new(),
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: HashMap::new(),
            tags: vec![],
            priority: TaskPriority::Medium,
            blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn map(tasks: Vec<Task>) -> HashMap<String, Task> {
        tasks.into_iter().map(|t| (t.id.clone(), t)).collect()
    }

    #[test]
    fn is_blocked_when_blocker_pending() {
        let all = map(vec![
            task("a", &[], TaskStatus::Pending),
            task("b", &["a"], TaskStatus::Pending),
        ]);
        assert!(is_blocked(all.get("b").unwrap(), &all));
        assert!(!is_blocked(all.get("a").unwrap(), &all));
    }

    #[test]
    fn not_blocked_when_blocker_completed() {
        let all = map(vec![
            task("a", &[], TaskStatus::Completed),
            task("b", &["a"], TaskStatus::Pending),
        ]);
        assert!(!is_blocked(all.get("b").unwrap(), &all));
    }

    #[test]
    fn missing_blocker_keeps_task_blocked() {
        let all = map(vec![task("b", &["ghost"], TaskStatus::Pending)]);
        assert!(is_blocked(all.get("b").unwrap(), &all));
        assert_eq!(open_blockers(all.get("b").unwrap(), &all), vec!["ghost"]);
    }

    #[test]
    fn cycle_self_edge() {
        let all = map(vec![task("a", &[], TaskStatus::Pending)]);
        assert!(would_create_cycle("a", &["a".into()], &all));
    }

    #[test]
    fn cycle_a_blocks_b_blocks_a() {
        // Existing: B blocked_by [A]. Proposing A blocked_by [B] → cycle.
        let all = map(vec![
            task("a", &[], TaskStatus::Pending),
            task("b", &["a"], TaskStatus::Pending),
        ]);
        assert!(would_create_cycle("a", &["b".into()], &all));
    }

    #[test]
    fn no_cycle_on_linear_chain() {
        // A free; B blocked by A; proposing C blocked by B is fine.
        let all = map(vec![
            task("a", &[], TaskStatus::Pending),
            task("b", &["a"], TaskStatus::Pending),
        ]);
        assert!(!would_create_cycle("c", &["b".into()], &all));
    }

    #[test]
    fn ready_vs_blocked_lists() {
        let all = map(vec![
            task("a", &[], TaskStatus::Pending),
            task("b", &["a"], TaskStatus::Pending),
            task("c", &[], TaskStatus::Completed),
            task("d", &["c"], TaskStatus::Pending), // c done → d ready
        ]);
        let ready_ids: HashSet<_> = ready_tasks(&all).into_iter().map(|t| t.id).collect();
        let blocked_ids: HashSet<_> = blocked_tasks(&all).into_iter().map(|t| t.id).collect();
        assert!(ready_ids.contains("a"));
        assert!(ready_ids.contains("d"));
        assert!(!ready_ids.contains("b"));
        assert!(blocked_ids.contains("b"));
        assert!(!blocked_ids.contains("d"));
    }

    #[test]
    fn validate_blockers_exist_rejects_missing() {
        let all = map(vec![task("a", &[], TaskStatus::Pending)]);
        assert!(validate_blockers_exist(&["a".into()], &all).is_ok());
        assert!(validate_blockers_exist(&["nope".into()], &all).is_err());
    }
}
