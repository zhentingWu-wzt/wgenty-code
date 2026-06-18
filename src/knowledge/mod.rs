//! Knowledge Module — on-demand skill loading and documentation maintenance.
//!
//! Corresponds to harness mechanism s05: load knowledge when needed, not upfront.
//! Skills are injected via tool_result, not the system prompt.

pub mod builtin;
pub mod docs;
pub mod executor;
pub mod external;
pub mod loader;
pub mod registry;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

/// Skill execution context
#[derive(Clone)]
pub struct SkillContext {
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub tool_registry: Option<std::sync::Arc<crate::tools::ToolRegistry>>,
    pub data: HashMap<String, serde_json::Value>,
}

impl std::fmt::Debug for SkillContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillContext")
            .field("cwd", &self.cwd)
            .field("env", &self.env)
            .field("tool_registry", &"[ToolRegistry]")
            .field("data", &self.data)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillParams {
    pub raw_input: String,
    pub args: Vec<String>,
    pub named_params: HashMap<String, String>,
    pub flags: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResult {
    pub success: bool,
    pub message: String,
    pub output: Option<serde_json::Value>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillError {
    pub message: String,
    pub code: String,
    pub details: Option<serde_json::Value>,
}

#[async_trait]
pub trait Skill: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn examples(&self) -> Vec<String>;
    fn parameter_schema(&self) -> serde_json::Value;
    async fn execute(
        &self,
        params: SkillParams,
        context: SkillContext,
    ) -> Result<SkillResult, SkillError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillCategory {
    Git,
    CodeReview,
    Testing,
    Documentation,
    ProjectSetup,
    Debugging,
    Utility,
}

impl std::fmt::Display for SkillCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillCategory::Git => write!(f, "git"),
            SkillCategory::CodeReview => write!(f, "code_review"),
            SkillCategory::Testing => write!(f, "testing"),
            SkillCategory::Documentation => write!(f, "documentation"),
            SkillCategory::ProjectSetup => write!(f, "project_setup"),
            SkillCategory::Debugging => write!(f, "debugging"),
            SkillCategory::Utility => write!(f, "utility"),
        }
    }
}

pub use builtin::BuiltinSkills;
pub use docs::{MagicDocHeader, MagicDocInfo, MagicDocsConfig, MagicDocsService, MagicDocsStatus};
pub use executor::SkillExecutor;
pub use external::{
    derive_canonical_skill_name, parse_external_skill_document, ExternalSkillDefinition,
    ExternalSkillError, ExternalSkillSource, ParsedExternalSkillDocument,
    ShadowedSkillDefinition, SkillFrontmatter,
};
pub use loader::{SkillInfo, SkillLoader};
pub use registry::SkillRegistry;
