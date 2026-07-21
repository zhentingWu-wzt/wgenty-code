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
    /// Per-endpoint context window override (tokens). When set, this takes
    /// priority over both the built-in model lookup ([`known_context_window`])
    /// and the global [`ModelsConfig::context_window`]. Use this for relays
    /// or custom models that expose a non-standard window.
    #[serde(default)]
    pub context_window: Option<usize>,
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

/// Known context-window sizes (in tokens) for common model names/IDs.
///
/// Returns `None` for unrecognized models so callers can fall back to a
/// configured default. This is a static table built from public model docs;
/// it does not query the API. Model names are matched case-insensitively
/// against both friendly aliases (`sonnet`) and full IDs
/// (`claude-sonnet-4-6-20250514`).
pub fn known_context_window(model: &str) -> Option<usize> {
    let lower = model.to_ascii_lowercase();
    // Anthropic Claude family - all current models expose 200k context.
    if lower.starts_with("claude") || matches!(lower.as_str(), "sonnet" | "opus" | "haiku") {
        return Some(1_024_000);
    }
    // DeepSeek - 64k context.
    if lower.starts_with("deepseek") || matches!(lower.as_str(), "v3" | "r1" | "reasoner") {
        return Some(1_024_000);
    }
    // OpenAI gpt-4o / gpt-4-turbo family - 128k.
    if lower.starts_with("gpt-4o") || lower.starts_with("gpt-4-turbo") {
        return Some(128_000);
    }
    // Legacy gpt-4 (non-turbo) - 8k.
    if lower.starts_with("gpt-4") || lower == "gpt-4" {
        return Some(8_000);
    }
    if lower.starts_with("gpt-3.5") {
        return Some(16_000);
    }
    if lower.starts_with("gpt") {
        return Some(1_024_000);
    }
    // Qwen (DashScope).
    if lower.starts_with("qwen-long") {
        return Some(1_000_000);
    }
    if lower.starts_with("qwen-plus") || lower.starts_with("qwen-turbo") {
        return Some(128_000);
    }
    if lower.starts_with("qwen-max") {
        return Some(32_000);
    }
    None
}

/// Resolve the effective context window (tokens) for an endpoint.
///
/// Priority:
/// 1. Explicit [`ModelEndpoint::context_window`] (user override)
/// 2. Built-in [`known_context_window`] lookup by model name
/// 3. `global_fallback` (the top-level [`ModelsConfig::context_window`])
///
/// This lets each model use its real window instead of the single global
/// value, so `needs_compaction` triggers at the right point for small-window
/// models (e.g. DeepSeek 64k) while staying zero-config for known models.
pub fn resolve_context_window(endpoint: &ModelEndpoint, global_fallback: usize) -> usize {
    endpoint
        .context_window
        .or_else(|| known_context_window(&endpoint.name))
        .unwrap_or(global_fallback)
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
                context_window: None,
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

    #[test]
    fn test_known_context_window_matches_common_models() {
        // Anthropic aliases and full IDs.
        assert_eq!(known_context_window("sonnet"), Some(200_000));
        assert_eq!(
            known_context_window("Claude-Sonnet-4-6-20250514"),
            Some(200_000)
        );
        assert_eq!(known_context_window("haiku"), Some(200_000));
        // DeepSeek.
        assert_eq!(known_context_window("deepseek-chat"), Some(64_000));
        assert_eq!(known_context_window("v3"), Some(64_000));
        // OpenAI.
        assert_eq!(known_context_window("gpt-4o"), Some(128_000));
        assert_eq!(known_context_window("gpt-4"), Some(8_000));
        // Unknown -> None (caller falls back to config).
        assert_eq!(known_context_window("my-custom-llm"), None);
    }

    #[test]
    fn test_resolve_context_window_priority() {
        // 1. Explicit endpoint override wins.
        let ep = ModelEndpoint {
            name: "sonnet".to_string(),
            context_window: Some(150_000),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 200_000), 150_000);
        // 2. Known model lookup when no override (sonnet -> 200k, ignores fallback).
        let ep = ModelEndpoint {
            name: "sonnet".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 999_999), 200_000);
        // 3. Unknown model falls back to global.
        let ep = ModelEndpoint {
            name: "my-custom-llm".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 200_000), 200_000);
        // DeepSeek: known lookup returns 64k even when global default is 200k.
        let ep = ModelEndpoint {
            name: "deepseek-chat".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 200_000), 64_000);
    }

    #[test]
    fn test_model_endpoint_context_window_deserialize() {
        let json = r#"{"name":"sonnet","context_window":150000}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(ep.context_window, Some(150_000));
    }
}
