use serde::{Deserialize, Serialize};

/// One model endpoint: name + optional override of base_url/api_key/appkey.
/// On `models.small` / `models.planner`, `None` for url/key/appkey means inherit from `models.main`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelEndpoint {
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub appkey: Option<String>,
    /// Force the API format regardless of base_url auto-detection.
    /// `"anthropic"` → `/v1/messages` (Anthropic format); `"openai"` →
    /// `/v1/chat/completions`. Needed for relays like packyapi that speak
    /// Anthropic natively but whose URL lacks "anthropic" (auto-detect would
    /// wrongly pick OpenAI and hit a flaky compat layer).
    #[serde(default)]
    pub provider: Option<String>,
}

impl ModelEndpoint {
    /// Resolve the effective base_url for this endpoint. If `self.base_url` is None,
    /// fall back to env var `API_BASE_URL`, then "https://api.anthropic.com".
    pub fn endpoint_base_url(&self) -> String {
        if let Some(u) = &self.base_url {
            return u.clone();
        }
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string())
    }

    /// Resolve the effective api_key for this endpoint, checking env first.
    pub fn endpoint_api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
            .or_else(|| self.api_key.clone())
    }
}

/// HTTP/SSE transport-layer config shared by all model endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    pub max_tokens: usize,
    pub timeout: u64,
    pub streaming: bool,
    pub beta_headers: Vec<String>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            timeout: 120,
            streaming: true,
            beta_headers: vec![],
        }
    }
}

fn default_context_window() -> usize {
    200_000
}

/// All model endpoints + shared transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default)]
    pub transport: TransportConfig,
    pub main: ModelEndpoint,
    #[serde(default)]
    pub small: Option<ModelEndpoint>,
    #[serde(default)]
    pub planner: Option<ModelEndpoint>,
    /// Maximum context window size in tokens. Used by the TUI to display
    /// context usage percentage. Default: 200_000 (200k tokens).
    #[serde(default = "default_context_window")]
    pub context_window: usize,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            transport: TransportConfig::default(),
            main: ModelEndpoint {
                name: "sonnet".to_string(),
                base_url: std::env::var("API_BASE_URL").ok(),
                api_key: std::env::var("ANTHROPIC_API_KEY")
                    .ok()
                    .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
                    .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok()),
                appkey: None,
                provider: None,
            },
            small: None,
            planner: None,
            context_window: 200_000,
        }
    }
}

/// Token budgets for main agent and subagents (units of 1000 tokens; 0 = unlimited).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenBudget {
    #[serde(default)]
    pub main_k: usize,
    #[serde(default)]
    pub subagent_default_k: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_context_window() {
        assert_eq!(ModelsConfig::default().context_window, 200_000);
    }

    #[test]
    fn test_context_window_deserialize_default() {
        let json = r#"{"main":{"name":"test"}}"#;
        let cfg: ModelsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.context_window, 200_000);
    }
}
