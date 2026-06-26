use async_trait::async_trait;
use tokio::task;

use super::interaction::{ConfirmPrompt, InteractionQuestion, InteractionService, UserAnswer};

/// CLI-based interaction service that uses stdin/stdout.
///
/// Suitable for headless terminal sessions or debugging.
pub struct CliInteractionService;

#[async_trait]
impl InteractionService for CliInteractionService {
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer> {
        println!("\n{}", question.message);
        for (i, opt) in question.options.iter().enumerate() {
            let desc = opt
                .description
                .as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {}. {}{}", i + 1, opt.label, desc);
        }
        print!("Enter choice (1-{}): ", question.options.len());
        let input = task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            buf.trim().to_string()
        })
        .await?;
        let idx: usize = input.parse().unwrap_or(0);
        if idx > 0 && idx <= question.options.len() {
            Ok(UserAnswer {
                selected: vec![question.options[idx - 1].value.clone()],
            })
        } else {
            Ok(UserAnswer { selected: vec![] })
        }
    }

    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool> {
        let default = if prompt.default_yes { "Y/n" } else { "y/N" };
        println!("{} [{}]", prompt.message, default);
        let input = task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            buf.trim().to_lowercase()
        })
        .await?;
        Ok(match input.as_str() {
            "y" | "yes" => true,
            "n" | "no" => false,
            "" => prompt.default_yes,
            _ => prompt.default_yes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::interaction::InteractionService;

    #[test]
    fn test_cli_service_creation_and_trait_impl() {
        let svc = CliInteractionService;

        // Statically verify that CliInteractionService implements InteractionService
        fn _assert_trait(_s: &dyn InteractionService) {}
        _assert_trait(&svc);
    }
}
