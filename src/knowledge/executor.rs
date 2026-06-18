//! Skill Executor
//!
//! Executes skills with parameter parsing and context management.

use super::{Skill, SkillContext, SkillError, SkillParams, SkillResult};
use std::sync::Arc;

/// Skill executor
pub struct SkillExecutor {
    registry: Arc<super::registry::SkillRegistry>,
}

impl SkillExecutor {
    /// Create a new skill executor
    pub fn new(registry: Arc<super::registry::SkillRegistry>) -> Self {
        Self { registry }
    }

    /// Parse skill input string into parameters
    pub fn parse_input(&self, input: &str) -> SkillParams {
        let mut args = Vec::new();
        let mut named_params = std::collections::HashMap::new();
        let mut flags = std::collections::HashMap::new();

        let tokens = tokenize_input(input);

        for token in tokens {
            if token.starts_with("--") {
                // Named parameter or flag
                let parameter = token.strip_prefix("--").unwrap_or_default();
                if let Some((key, value)) = parameter.split_once('=') {
                    // Named parameter: --key=value
                    named_params.insert(key.to_string(), value.to_string());
                } else {
                    // Flag: --flag
                    flags.insert(parameter.to_string(), true);
                }
            } else if token.starts_with('-') && token.len() > 1 {
                // Short flag: -f
                for flag in token.strip_prefix('-').unwrap_or_default().chars() {
                    flags.insert(flag.to_string(), true);
                }
            } else {
                // Positional argument
                args.push(token);
            }
        }

        SkillParams {
            raw_input: input.to_string(),
            args,
            named_params,
            flags,
        }
    }

    /// Execute a skill by name
    pub async fn execute(
        &self,
        skill_name: &str,
        input: &str,
        context: SkillContext,
    ) -> Result<SkillResult, SkillError> {
        let skill = self.registry.get(skill_name).ok_or_else(|| SkillError {
            message: format!("Skill not found: {}", skill_name),
            code: "skill_not_found".to_string(),
            details: None,
        })?;

        let params = self.parse_input(input);

        // Validate parameters against schema
        self.validate_params(skill.clone(), &params)?;

        // Execute the skill
        skill.execute(params, context).await
    }

    /// Validate parameters against skill schema
    fn validate_params(
        &self,
        _skill: Arc<dyn Skill>,
        _params: &SkillParams,
    ) -> Result<(), SkillError> {
        // Simple validation based on schema
        // In a full implementation, this would validate against JSON Schema
        Ok(())
    }

    /// List available skills
    pub fn list_skills(&self) -> Vec<(String, String)> {
        self.registry.list_all()
    }

    /// Search skills
    pub fn search_skills(&self, keyword: &str) -> Vec<(String, String)> {
        self.registry
            .search(keyword)
            .iter()
            .map(|skill| (skill.name().to_string(), skill.description().to_string()))
            .collect()
    }

    /// Get skill help
    pub fn get_help(&self, skill_name: &str) -> Result<String, SkillError> {
        let skill = self.registry.get(skill_name).ok_or_else(|| SkillError {
            message: format!("Skill not found: {}", skill_name),
            code: "skill_not_found".to_string(),
            details: None,
        })?;

        let schema = skill.parameter_schema();
        let examples = skill.examples();

        let mut help = format!(
            "Skill: {}\n\nDescription: {}\n\n",
            skill.name(),
            skill.description()
        );

        if !examples.is_empty() {
            help.push_str("Examples:\n");
            for (i, example) in examples.iter().enumerate() {
                help.push_str(&format!("  {}. {}\n", i + 1, example));
            }
            help.push('\n');
        }

        help.push_str(&format!(
            "Parameter Schema:\n{}\n",
            serde_json::to_string_pretty(&schema).unwrap_or_default()
        ));

        Ok(help)
    }
}

fn tokenize_input(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;

    for character in input.chars() {
        match (character, quote) {
            ('\'' | '"', None) => quote = Some(character),
            (quote_character, Some(active_quote)) if quote_character == active_quote => {
                quote = None;
            }
            (character, None) if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}
