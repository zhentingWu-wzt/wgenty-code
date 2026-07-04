use super::defaults::default_true;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptIncludes {
    #[serde(default = "default_true")]
    pub permissions: bool,
    #[serde(default = "default_true")]
    pub developer: bool,
    #[serde(default = "default_true")]
    pub collaboration: bool,
    #[serde(default = "default_true")]
    pub environment: bool,
    #[serde(default = "default_true")]
    pub skills: bool,
}

impl Default for PromptIncludes {
    fn default() -> Self {
        Self {
            permissions: true,
            developer: true,
            collaboration: true,
            environment: true,
            skills: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptConfig {
    #[serde(default)]
    pub include: PromptIncludes,
    #[serde(default)]
    pub developer_instructions: Option<String>,
    #[serde(default)]
    pub collaboration_mode: Option<String>,
    #[serde(default)]
    pub model_instructions_file: Option<String>,
}
