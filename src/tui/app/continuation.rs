//! Synthetic continuation-turn scheduling for the unified subagent lifecycle.
//!
//! When the persistent main agent is idle (no turn running) and the daemon has
//! a ready root-direct task group, the app claims it atomically and starts a
//! hidden continuation turn that injects the delivered child results as a
//! structured system message. A queued user turn wins over a continuation: if
//! a user input is already queued, the delivery is attached to it instead of
//! spawning a standalone continuation.

use super::App;

impl App {
    /// Poll the daemon for a ready root-direct task group and, when one is
    /// claimed, schedule a continuation turn (or attach the delivery to a
    /// queued user input). No-op while a turn is running or a claim is not
    /// ready. Atomicity is daemon-owned: a claimed group is delivered at most
    /// once.
    pub(super) async fn poll_ready_task_groups(&mut self) {
        if self.current_turn_handle.is_some() {
            return;
        }
        let delivery = match self
            .daemon_client
            .claim_task_group(&self.session_id, self.agent_generation)
            .await
        {
            Ok(Some(d)) => d,
            Ok(None) => return,
            // Network blips / daemon restarts must not crash the TUI; retry next tick.
            Err(error) => {
                tracing::debug!(error = %error, "claim_task_group poll failed; retrying next tick");
                return;
            }
        };
        tracing::info!(
            group_id = %delivery.group_id,
            generation = delivery.generation,
            results = delivery.results.len(),
            "Claimed ready task group; scheduling continuation turn"
        );
        // If a user turn is already queued, attach the delivery so the user's
        // prompt and the subagent results are consumed together.
        if let Some(next) = self.pending_inputs.front_mut() {
            if next.continuation.is_none() {
                next.continuation = Some(delivery);
                return;
            }
        }
        // Otherwise enqueue a standalone hidden continuation turn.
        self.pending_inputs
            .push_back(super::PendingInput::continuation(delivery));
        self.start_next_turn();
    }
}

#[cfg(test)]
mod tests {
    use super::super::PendingInput;
    use crate::agent::{ChildResult, ChildTerminalStatus};
    use crate::tui::client::TaskGroupDeliveryResponse;

    fn delivery(group: &str) -> TaskGroupDeliveryResponse {
        TaskGroupDeliveryResponse {
            group_id: group.to_string(),
            generation: 0,
            results: vec![ChildResult {
                child_id: crate::agent::AgentId::new("child-a"),
                status: ChildTerminalStatus::Completed,
                summary: "done".to_string(),
                error_code: None,
                partial_result: None,
            }],
        }
    }

    #[test]
    fn continuation_pending_input_has_no_visible_text() {
        let p = PendingInput::continuation(delivery("g1"));
        assert!(p.is_continuation());
        // No visible user row and no agent user-prompt text.
        assert!(p.display_text.is_empty());
        assert!(p.agent_input.is_empty());
        assert_eq!(p.continuation.as_ref().unwrap().group_id, "g1");
    }

    #[test]
    fn user_pending_input_is_not_a_continuation() {
        let p = PendingInput::new("hello".to_string());
        assert!(!p.is_continuation());
        assert!(p.continuation.is_none());
        assert_eq!(p.display_text, "hello");
    }

    #[test]
    fn queued_user_input_can_attach_a_delivery() {
        // A queued user turn wins: the delivery attaches to it rather than
        // spawning a standalone continuation.
        let mut p = PendingInput::new("next question".to_string());
        assert!(p.continuation.is_none());
        p.continuation = Some(delivery("g2"));
        assert!(p.is_continuation());
        // The user's display text is preserved alongside the attached delivery.
        assert_eq!(p.display_text, "next question");
    }
}
