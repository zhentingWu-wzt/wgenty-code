use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A question that the agent wants to ask the user during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionQuestion {
    pub id: String,
    pub message: String,
    pub options: Vec<InteractionOption>,
}

/// An option within an interaction question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

/// The user's answer to an interaction question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAnswer {
    pub selected: Vec<String>,
}

/// A yes/no confirmation prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmPrompt {
    pub message: String,
    pub default_yes: bool,
}

/// Service trait for interacting with the user during agent execution.
///
/// Implementors provide concrete interaction channels:
/// - TUI (terminal user interface)
/// - CLI (command-line stdin/stdout)
/// - Headless (pre-configured answers or deny)
#[async_trait]
pub trait InteractionService: Send + Sync {
    /// Pose a question to the user and collect the answer.
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer>;
    /// Ask the user to confirm an action.
    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_interaction_question_roundtrip_serialization() {
        let q = InteractionQuestion {
            id: "q1".to_string(),
            message: "Choose one:".to_string(),
            options: vec![InteractionOption {
                label: "Yes".to_string(),
                value: "yes".to_string(),
                description: Some("Confirm the action".to_string()),
            }],
        };
        let json = serde_json::to_string(&q).unwrap();
        let q2: InteractionQuestion = serde_json::from_str(&json).unwrap();
        assert_eq!(q.id, q2.id);
        assert_eq!(q.message, q2.message);
        assert_eq!(q.options.len(), q2.options.len());
        assert_eq!(q.options[0].label, q2.options[0].label);
        assert_eq!(q.options[0].value, q2.options[0].value);
        assert_eq!(q.options[0].description, q2.options[0].description);
    }

    #[test]
    fn test_user_answer_roundtrip_serialization() {
        let a = UserAnswer {
            selected: vec!["yes".to_string(), "no".to_string()],
        };
        let json = serde_json::to_string(&a).unwrap();
        let a2: UserAnswer = serde_json::from_str(&json).unwrap();
        assert_eq!(a.selected, a2.selected);
    }

    #[test]
    fn test_confirm_prompt_roundtrip_serialization() {
        let p = ConfirmPrompt {
            message: "Are you sure?".to_string(),
            default_yes: true,
        };
        let json = serde_json::to_string(&p).unwrap();
        let p2: ConfirmPrompt = serde_json::from_str(&json).unwrap();
        assert_eq!(p.message, p2.message);
        assert_eq!(p.default_yes, p2.default_yes);
    }

    #[test]
    fn test_confirm_prompt_default_no_serialization() {
        let p = ConfirmPrompt {
            message: "Delete?".to_string(),
            default_yes: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        let p2: ConfirmPrompt = serde_json::from_str(&json).unwrap();
        assert!(!p2.default_yes);
    }
}
