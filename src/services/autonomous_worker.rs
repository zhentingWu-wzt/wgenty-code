//! Autonomous Worker (s11) - background claimer for ready task-groups.
//!
//! A daemon-side service that periodically claims ready root task-groups and
//! notifies the root agent (via its mailbox) that work is available for
//! synthesis. This keeps queued work from stranding when no TUI client is
//! actively polling.
//!
//! Design note: the worker does NOT run a full `run_agent_loop` itself by
//! default (that requires a root agent context + LLM ports wired daemon-side,
//! tracked as a follow-up). Instead it claims + notifies, so any connected
//! root agent (TUI continuation path, or a future daemon-embedded loop)
//! consumes the delivery. Idle timeout stops the worker when nothing is ready.

use crate::agent::{AgentCoordinator, AgentExecutionContext, SessionId};
use crate::teams::mailbox::{Mailbox, TeamMessage};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Configuration for the autonomous worker.
#[derive(Debug, Clone)]
pub struct AutonomousWorkerConfig {
    /// Poll interval for ready task-groups.
    pub poll_interval: Duration,
    /// Stop after this many consecutive idle polls with no ready work.
    pub max_idle_polls: u16,
    pub enabled: bool,
}

impl Default for AutonomousWorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(2),
            max_idle_polls: 30, // ~60s at default interval
            enabled: false,
        }
    }
}

/// Status snapshot for diagnostics.
#[derive(Debug, Clone, Default)]
pub struct WorkerStatus {
    pub running: bool,
    pub claims: usize,
    pub idle_polls: u16,
}

/// Background claimer. One per daemon. Owns a synthetic root identity used
/// only to drive `claim_ready_root_group` (it does not execute as that agent).
pub struct AutonomousWorker {
    config: AutonomousWorkerConfig,
    coordinator: Arc<AgentCoordinator>,
    status: Arc<Mutex<WorkerStatus>>,
}

impl AutonomousWorker {
    pub fn new(coordinator: Arc<AgentCoordinator>, config: AutonomousWorkerConfig) -> Self {
        Self {
            config,
            coordinator,
            status: Arc::new(Mutex::new(WorkerStatus::default())),
        }
    }

    pub async fn status(&self) -> WorkerStatus {
        self.status.lock().await.clone()
    }

    /// Run the poll loop until idle timeout or the daemon shuts down.
    ///
    /// `root_session_id` identifies the persistent root agent whose
    /// task-groups are claimed; `notify_agent_id` is the mailbox recipient
    /// to wake (the root agent's id).
    pub async fn run(self: Arc<Self>, root_session_id: &str, notify_agent_id: &str) {
        if !self.config.enabled {
            return;
        }
        let root = AgentExecutionContext::root(SessionId::new(root_session_id));
        {
            let mut s = self.status.lock().await;
            s.running = true;
        }
        let mut idle = 0u16;
        loop {
            tokio::time::sleep(self.config.poll_interval).await;
            let generation = self.coordinator.current_generation(&root.session_id).await;
            let claimed = match self
                .coordinator
                .claim_ready_root_group(&root, generation)
                .await
            {
                Ok(Some(delivery)) => {
                    // Notify the root agent that a group is ready to synthesize.
                    // Include result summaries so the notification is actionable
                    // even without an active TUI connection to consume the
                    // delivery directly.
                    let summaries: Vec<String> = delivery
                        .results
                        .iter()
                        .map(|r| {
                            let status = if r.status == crate::agent::ChildTerminalStatus::Completed
                            {
                                "completed"
                            } else {
                                "failed"
                            };
                            let summary = r.summary.chars().take(200).collect::<String>();
                            format!("- [{}] {}: {}", r.child_id, status, summary)
                        })
                        .collect();
                    let note = format!(
                        "A background task-group ({} result(s)) is ready for synthesis.\n\nResults:\n{}\n\n\
                         Use the task-group delivery to continue.",
                        delivery.results.len(),
                        summaries.join("\n")
                    );
                    if let Some(path) = mailbox_path(notify_agent_id) {
                        let mb = Mailbox::new(path);
                        let msg = TeamMessage::Broadcast {
                            from: "autonomous-worker".to_string(),
                            content: note,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        };
                        let _ = mb.send(&msg).await;
                    }
                    let mut s = self.status.lock().await;
                    s.claims += 1;
                    idle = 0;
                    true
                }
                Ok(None) => false,
                Err(e) => {
                    tracing::warn!(error = %e, "autonomous worker claim failed");
                    false
                }
            };
            if !claimed {
                idle += 1;
                let mut s = self.status.lock().await;
                s.idle_polls = idle;
                if idle >= self.config.max_idle_polls {
                    s.running = false;
                    tracing::info!(
                        idle_polls = idle,
                        "autonomous worker idle; stopping until next task arrives"
                    );
                    return;
                }
            }
        }
    }
}

fn mailbox_path(agent_id: &str) -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let safe: String = agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(
        cwd.join(".team")
            .join("inbox")
            .join(format!("{safe}.jsonl")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_disabled() {
        assert!(!AutonomousWorkerConfig::default().enabled);
    }

    #[test]
    fn mailbox_path_sanitizes() {
        let p = mailbox_path("agent-1").unwrap();
        assert!(p.to_string_lossy().ends_with("agent-1.jsonl"));
        let p2 = mailbox_path("../evil").unwrap();
        assert!(!p2.to_string_lossy().contains("../"));
    }
}
