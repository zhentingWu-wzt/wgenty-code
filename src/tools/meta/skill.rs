//! Skill Runtime Action — Claude Code-compatible `skill` tool for nested external skill loading.
//!
//! When the external skill registry is not wired this tool returns a clear
//! not-configured error, signalling that the runtime needs to be set up before
//! skill resolution is available.

use crate::knowledge::{
    ExternalSkillRegistry, LoadedSkillContext, LoadedSkillRecord, PolicyDecision, SkillLoadEvent,
    SkillPolicy,
};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// Claude Code-compatible runtime action for loading external skills.
///
/// When constructed via `SkillTool::new()` without a registry, the tool
/// returns a clear not-configured error.  Call `with_registry()` to wire
/// the external skill registry.
pub struct SkillTool {
    registry: Option<Arc<ExternalSkillRegistry>>,
    loaded_context: Arc<Mutex<LoadedSkillContext>>,
    policy: Option<Arc<dyn SkillPolicy>>,
}

impl SkillTool {
    /// Create a SkillTool that returns not-configured errors.
    pub fn new() -> Self {
        Self {
            registry: None,
            loaded_context: Arc::new(Mutex::new(LoadedSkillContext::default())),
            policy: None,
        }
    }

    /// Wire the skill registry so the tool can resolve external skills.
    pub fn with_registry(
        registry: Arc<ExternalSkillRegistry>,
        loaded_context: LoadedSkillContext,
    ) -> Self {
        Self {
            registry: Some(registry),
            loaded_context: Arc::new(Mutex::new(loaded_context)),
            policy: None,
        }
    }

    /// Wire the external skill registry into an already-constructed tool.
    pub fn set_registry(&mut self, registry: Arc<ExternalSkillRegistry>) {
        self.registry = Some(registry);
    }

    /// Set the policy hook for skill lifecycle events.
    pub fn set_policy(&mut self, policy: Arc<dyn SkillPolicy>) {
        self.policy = Some(policy);
    }
}

impl Default for SkillTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Load a Claude Code-compatible external skill by canonical name. \
         Use for nested skill invocation. \
         Leave skill name empty to list available external skills."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Canonical external skill name to load. Omit or leave empty to list available skills."
                },
                "args": {
                    "type": "string",
                    "description": "Optional raw arguments passed to the skill"
                },
                "depth": {
                    "type": "integer",
                    "description": "Nested skill depth"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let registry = self.registry.as_ref().ok_or_else(|| ToolError {
            message: "External skill registry is not configured".to_string(),
            code: Some("skill_registry_unconfigured".to_string()),
        })?;

        let skill_name = input["skill"].as_str().unwrap_or("");
        let args = input["args"].as_str().map(|v| v.to_string());
        let depth = input["depth"].as_u64().unwrap_or(0) as usize;

        // List available skills when name is empty
        if skill_name.is_empty() {
            let skills_list: Vec<serde_json::Value> = registry
                .list()
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.canonical_name,
                        "description": s.description,
                        "source": s.source.label(),
                        "source_path": s.source_path.display().to_string(),
                    })
                })
                .collect();

            if skills_list.is_empty() {
                return Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: "No external skills found. Place SKILL.md files in \
                              ~/.wgenty-code/skills/<name>/ or \
                              .wgenty-code/skills/<name>/."
                        .to_string(),
                    metadata: std::collections::HashMap::new(),
                });
            }

            return Ok(ToolOutput {
                output_type: "json".to_string(),
                content: serde_json::to_string_pretty(&serde_json::json!({
                    "skills": skills_list,
                    "count": skills_list.len(),
                    "hint": "Use skill with a specific name to load full instructions."
                }))
                .expect("serializing json! value is infallible"),
                metadata: std::collections::HashMap::new(),
            });
        }

        let mut context = self.loaded_context.lock().map_err(|_| ToolError {
            message: "Loaded skill context lock poisoned".to_string(),
            code: Some("skill_context_error".to_string()),
        })?;

        if !context.depth_allowed(depth) {
            return Err(ToolError {
                message: format!(
                    "Nested skill depth {} exceeds maximum depth {}",
                    depth,
                    crate::knowledge::MAX_NESTED_SKILL_DEPTH
                ),
                code: Some("skill_depth_exceeded".to_string()),
            });
        }

        let skill = registry.resolve(skill_name).ok_or_else(|| {
            let suggestions = registry.suggest(skill_name, 3);
            let suffix = if suggestions.is_empty() {
                String::new()
            } else {
                format!(" Did you mean: {}?", suggestions.join(", "))
            };
            ToolError {
                message: format!("Skill '{}' not found.{}", skill_name, suffix),
                code: Some("skill_not_found".to_string()),
            }
        })?;

        // Policy hook: check before loading
        if let Some(ref policy) = self.policy {
            let event = SkillLoadEvent {
                skill_name: skill_name.to_string(),
                args: args.clone(),
                depth,
                loaded_context: context.clone(),
            };
            match policy.before_skill_load(&event) {
                PolicyDecision::Deny { message } => {
                    return Err(ToolError {
                        message,
                        code: Some("skill_policy_denied".to_string()),
                    });
                }
                PolicyDecision::Warn { message } => {
                    tracing::warn!("Skill policy warning for '{}': {}", skill_name, message);
                }
                PolicyDecision::Allow => {}
            }
        }

        let was_new = context.record_load(LoadedSkillRecord {
            name: skill.canonical_name.clone(),
            source_path: skill.source_path.clone(),
            base_dir: skill.base_dir.clone(),
            args: args.clone(),
            parent: None,
            depth,
            turn_id: 0,
        });

        let content = if was_new {
            format!(
                "Base directory for this skill: {}\n\n{}\n\nARGUMENTS: {}",
                skill.base_dir.display(),
                skill.body,
                args.as_deref().unwrap_or("")
            )
        } else {
            format!(
                "Skill '{}' is already loaded from {}. Invocation recorded.\n\nARGUMENTS: {}",
                skill.canonical_name,
                skill.source_path.display(),
                args.as_deref().unwrap_or("")
            )
        };

        Ok(ToolOutput {
            output_type: "markdown".to_string(),
            content,
            metadata: std::collections::HashMap::from([
                (
                    "skill_name".to_string(),
                    serde_json::Value::String(skill.canonical_name.clone()),
                ),
                (
                    "source_path".to_string(),
                    serde_json::Value::String(skill.source_path.display().to_string()),
                ),
            ]),
        })
    }
}
