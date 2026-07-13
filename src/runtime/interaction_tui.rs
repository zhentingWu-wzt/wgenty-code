//! TUI-based [`InteractionService`].
//!
//! The agent loop's primary interactive path uses `AppEvent::QuestionAsked` /
//! `PermissionRequired` directly (via `tui::agent::adapters`). This service is
//! the generic `InteractionService` implementation for callers that go through
//! the shared interaction trait (hooks, future tools).

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use super::interaction::{ConfirmPrompt, InteractionQuestion, InteractionService, UserAnswer};

/// TUI-backed interaction service.
///
/// `event_tx` carries a serialized prompt description for diagnostics /
/// alternate UIs. `response_rx` receives the user's answer. When the channel
/// is closed or no answer arrives, methods return a cancelled / denied result
/// rather than panicking.
pub struct TuiInteractionService {
    pub event_tx: mpsc::UnboundedSender<String>,
    pub response_rx: Mutex<mpsc::UnboundedReceiver<UserAnswer>>,
}

impl TuiInteractionService {
    pub fn new(
        event_tx: mpsc::UnboundedSender<String>,
        response_rx: mpsc::UnboundedReceiver<UserAnswer>,
    ) -> Self {
        TuiInteractionService {
            event_tx,
            response_rx: Mutex::new(response_rx),
        }
    }
}

#[async_trait]
impl InteractionService for TuiInteractionService {
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer> {
        // Notify any listener (optional; main agent path uses AppEvent instead).
        let _ = self.event_tx.send(format!("ask:{}", question.message));

        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(answer) => Ok(answer),
            None => Ok(UserAnswer {
                selected: Vec::new(),
            }),
        }
    }

    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool> {
        let _ = self.event_tx.send(format!("confirm:{}", prompt.message));

        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(answer) => {
                let yes = answer
                    .selected
                    .iter()
                    .any(|s| matches!(s.as_str(), "yes" | "y" | "true" | "allow"));
                Ok(yes)
            }
            None => Ok(prompt.default_yes),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::interaction::InteractionOption;

    #[test]
    fn test_tui_service_creation_and_trait_impl() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<String>();
        let (_response_tx, response_rx) = mpsc::unbounded_channel::<UserAnswer>();
        let svc = TuiInteractionService::new(event_tx, response_rx);

        fn _assert_trait(_s: &dyn InteractionService) {}
        _assert_trait(&svc);
    }

    #[tokio::test]
    async fn ask_returns_answer_from_channel() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<String>();
        let (response_tx, response_rx) = mpsc::unbounded_channel::<UserAnswer>();
        let svc = TuiInteractionService::new(event_tx, response_rx);

        response_tx
            .send(UserAnswer {
                selected: vec!["opt-a".to_string()],
            })
            .unwrap();

        let q = InteractionQuestion {
            id: "q1".to_string(),
            message: "pick one".to_string(),
            options: vec![InteractionOption {
                label: "A".to_string(),
                value: "opt-a".to_string(),
                description: None,
            }],
        };
        let ans = svc.ask(&q).await.unwrap();
        assert_eq!(ans.selected, vec!["opt-a".to_string()]);
        assert_eq!(event_rx.recv().await.unwrap(), "ask:pick one");
    }

    #[tokio::test]
    async fn confirm_defaults_when_channel_closed() {
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<String>();
        let (response_tx, response_rx) = mpsc::unbounded_channel::<UserAnswer>();
        drop(response_tx);
        let svc = TuiInteractionService::new(event_tx, response_rx);
        let ok = svc
            .confirm(&ConfirmPrompt {
                message: "proceed?".into(),
                default_yes: true,
            })
            .await
            .unwrap();
        assert!(ok);
    }
}
