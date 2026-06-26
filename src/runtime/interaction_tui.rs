use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use super::interaction::{ConfirmPrompt, InteractionQuestion, InteractionService, UserAnswer};

/// TUI-based interaction service.
///
/// This skeleton defers the actual TUI event-channel connection to Task 11.
/// `event_tx` sends AppEvents to the TUI render loop; `response_rx` receives
/// user answers back from the TUI.
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
    async fn ask(&self, _question: &InteractionQuestion) -> anyhow::Result<UserAnswer> {
        todo!("TUI interaction ask() — will be wired in Task 11")
    }

    async fn confirm(&self, _prompt: &ConfirmPrompt) -> anyhow::Result<bool> {
        todo!("TUI interaction confirm() — will be wired in Task 11")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_service_creation_and_trait_impl() {
        let (_event_tx, _event_rx) = mpsc::unbounded_channel::<String>();
        let (_response_tx, response_rx) = mpsc::unbounded_channel::<UserAnswer>();
        let svc = TuiInteractionService::new(_event_tx, response_rx);

        // Statically verify that TuiInteractionService implements InteractionService
        fn _assert_trait(_s: &dyn InteractionService) {}
        _assert_trait(&svc);
    }
}
