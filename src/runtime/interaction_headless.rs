use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::interaction::{ConfirmPrompt, InteractionQuestion, InteractionService, UserAnswer};

/// Pre-configured answer for a specific question id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerMap {
    pub question_id: String,
    pub answer: Vec<String>,
}

/// Policy for headless interaction: deny all or use pre-configured answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeadlessPolicy {
    /// Reject any user interaction request.
    Deny,
    /// Answer interaction requests from a pre-configured map.
    PreConfiguredAnswers(Vec<AnswerMap>),
}

/// Headless interaction service that never prompts the user interactively.
///
/// Uses a `HeadlessPolicy` to either deny all interactions or respond
/// from a set of pre-configured answers.
pub struct HeadlessInteractionService {
    policy: HeadlessPolicy,
}

impl HeadlessInteractionService {
    pub fn new(policy: HeadlessPolicy) -> Self {
        HeadlessInteractionService { policy }
    }
}

#[async_trait]
impl InteractionService for HeadlessInteractionService {
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer> {
        match &self.policy {
            HeadlessPolicy::Deny => Err(anyhow::anyhow!(
                "Interaction denied: headless mode does not support user input"
            )),
            HeadlessPolicy::PreConfiguredAnswers(maps) => {
                for map in maps {
                    if map.question_id == question.id {
                        return Ok(UserAnswer {
                            selected: map.answer.clone(),
                        });
                    }
                }
                Err(anyhow::anyhow!(
                    "No pre-configured answer for question: {}",
                    question.id
                ))
            }
        }
    }

    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool> {
        match &self.policy {
            HeadlessPolicy::Deny => Err(anyhow::anyhow!(
                "Interaction denied: headless mode does not support user input"
            )),
            HeadlessPolicy::PreConfiguredAnswers(_maps) => {
                // default_yes as fallback if no specific answer
                Ok(prompt.default_yes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::interaction::InteractionService;

    fn make_question(id: &str) -> crate::runtime::interaction::InteractionQuestion {
        crate::runtime::interaction::InteractionQuestion {
            id: id.to_string(),
            message: "Test question".to_string(),
            options: vec![crate::runtime::interaction::InteractionOption {
                label: "A".to_string(),
                value: "a".to_string(),
                description: None,
            }],
        }
    }

    fn make_prompt() -> crate::runtime::interaction::ConfirmPrompt {
        crate::runtime::interaction::ConfirmPrompt {
            message: "Confirm?".to_string(),
            default_yes: true,
        }
    }

    #[tokio::test]
    async fn test_deny_ask_returns_error() {
        let svc = HeadlessInteractionService::new(HeadlessPolicy::Deny);
        let q = make_question("q1");
        let result = svc.ask(&q).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_deny_confirm_returns_error() {
        let svc = HeadlessInteractionService::new(HeadlessPolicy::Deny);
        let p = make_prompt();
        let result = svc.confirm(&p).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_preconfigured_matching_ask() {
        let maps = vec![AnswerMap {
            question_id: "q1".to_string(),
            answer: vec!["a".to_string()],
        }];
        let svc = HeadlessInteractionService::new(HeadlessPolicy::PreConfiguredAnswers(maps));
        let q = make_question("q1");
        let result = svc.ask(&q).await.unwrap();
        assert_eq!(result.selected, vec!["a"]);
    }

    #[tokio::test]
    async fn test_preconfigured_nonmatching_ask() {
        let maps = vec![AnswerMap {
            question_id: "other".to_string(),
            answer: vec!["b".to_string()],
        }];
        let svc = HeadlessInteractionService::new(HeadlessPolicy::PreConfiguredAnswers(maps));
        let q = make_question("q1");
        let result = svc.ask(&q).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_preconfigured_confirm_returns_default() {
        let svc = HeadlessInteractionService::new(HeadlessPolicy::PreConfiguredAnswers(vec![]));
        let p = make_prompt();
        let result = svc.confirm(&p).await.unwrap();
        assert!(result);
    }

    #[test]
    fn test_answer_map_serialization_roundtrip() {
        let am = AnswerMap {
            question_id: "q1".to_string(),
            answer: vec!["yes".to_string()],
        };
        let json = serde_json::to_string(&am).unwrap();
        let am2: AnswerMap = serde_json::from_str(&json).unwrap();
        assert_eq!(am.question_id, am2.question_id);
        assert_eq!(am.answer, am2.answer);
    }

    #[test]
    fn test_headless_policy_deny_serialization_roundtrip() {
        let policy = HeadlessPolicy::Deny;
        let json = serde_json::to_string(&policy).unwrap();
        let policy2: HeadlessPolicy = serde_json::from_str(&json).unwrap();
        assert!(matches!(policy2, HeadlessPolicy::Deny));
    }

    #[test]
    fn test_headless_policy_preconfigured_serialization_roundtrip() {
        let policy = HeadlessPolicy::PreConfiguredAnswers(vec![AnswerMap {
            question_id: "q1".to_string(),
            answer: vec!["a".to_string()],
        }]);
        let json = serde_json::to_string(&policy).unwrap();
        let policy2: HeadlessPolicy = serde_json::from_str(&json).unwrap();
        match policy2 {
            HeadlessPolicy::PreConfiguredAnswers(maps) => {
                assert_eq!(maps.len(), 1);
                assert_eq!(maps[0].question_id, "q1");
                assert_eq!(maps[0].answer, vec!["a"]);
            }
            _ => panic!("Expected PreConfiguredAnswers"),
        }
    }
}
