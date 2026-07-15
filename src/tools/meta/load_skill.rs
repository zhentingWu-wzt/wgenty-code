//! Load Skill Tool — inject full skill instructions on demand (Layer 2).
//!
//! When the model needs detailed guidance for a specific task, it calls
//! `load_skill` with the skill name. Calling with an empty name returns
//! the list of available skills (Layer 1 listing via tool_result).

use crate::knowledge::loader::SkillLoader;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::Arc;

pub struct LoadSkillTool {
    skill_loader: Arc<SkillLoader>,
}

impl LoadSkillTool {
    pub fn new(skill_loader: Arc<SkillLoader>) -> Self {
        Self { skill_loader }
    }
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &str {
        "load_skill"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Load a skill's full instructions by name. Use when you need detailed guidance \
         for a specific task (e.g., code review, testing, PDF processing). \
         Omit or leave name empty to list available skills."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name to load (e.g., 'code-review', 'testing'). Omit or leave empty to list available skills."
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let name = input["name"].as_str().unwrap_or("");

        if name.is_empty() {
            // Layer 1 — return list of available skills via tool_result
            let skills_list: Vec<serde_json::Value> = self
                .skill_loader
                .skill_names()
                .iter()
                .map(|n| {
                    let skill = self.skill_loader.load_skill(n);
                    serde_json::json!({
                        "name": n,
                        "description": skill.map(|s| s.description.clone()).unwrap_or_default(),
                    })
                })
                .collect();

            return Ok(ToolOutput {
                output_type: "json".to_string(),
                content: serde_json::to_string_pretty(&serde_json::json!({
                    "skills": skills_list,
                    "hint": "Use load_skill with a specific name to get full instructions."
                }))
                .expect("serializing json! value is infallible"),
                metadata: std::collections::HashMap::new(),
            });
        }

        // Layer 2 — return full skill body
        match self.skill_loader.load_skill(name) {
            Some(skill) => Ok(ToolOutput {
                output_type: "markdown".to_string(),
                content: skill.body.clone(),
                metadata: std::collections::HashMap::from([
                    (
                        "skill_name".to_string(),
                        serde_json::Value::String(skill.name.clone()),
                    ),
                    (
                        "skill_description".to_string(),
                        serde_json::Value::String(skill.description.clone()),
                    ),
                ]),
            }),
            None => {
                let available = self.skill_loader.skill_names().join(", ");
                Err(ToolError {
                    message: format!(
                        "Skill '{}' not found. Available skills: {}",
                        name,
                        if available.is_empty() {
                            "none".to_string()
                        } else {
                            available
                        }
                    ),
                    code: Some("skill_not_found".to_string()),
                })
            }
        }
    }
}
