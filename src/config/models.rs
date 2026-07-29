use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Capability tier for model routing. A profile declares which tier it serves so
/// the router can pick by capability rather than by name. This decouples the
/// profile name (e.g. `scout`, `vanguard`) from its role in routing.
///
/// - [`Light`](ModelTier::Light): fast/cheap, for simple self-contained tasks
///   (file reads, single commands, searches).
/// - [`Medium`](ModelTier::Medium): balanced; falls back to [`ModelsConfig::main`]
///   when no profile declares this tier.
/// - [`Heavy`](ModelTier::Heavy): strongest reasoning, for multi-step or
///   high-stakes work; falls back to [`ModelsConfig::main`] as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    Light,
    Medium,
    Heavy,
}

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
    /// `"deepseek"` → `/v1/chat/completions` (OpenAI format); `"openai"` →
    /// `/v1/chat/completions`. Also supports `"anthropic"` for Anthropic API.
    #[serde(default)]
    pub provider: Option<String>,
    /// Per-endpoint context window override (tokens). When set, this takes
    /// priority over both the built-in model lookup ([`known_context_window`])
    /// and the global [`ModelsConfig::context_window`]. Use this for relays
    /// or custom models that expose a non-standard window.
    #[serde(default)]
    pub context_window: Option<usize>,
    /// Human-readable label shown in the `/model` picker (e.g. "Claude Sonnet",
    /// "DeepSeek Chat"). When absent, the picker falls back to [`Self::name`].
    /// Display-only; the code path always keys off `name` for API calls.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Capability tier this profile serves in automatic subagent model routing
    /// ([`ModelTier::Light`] / [`ModelTier::Medium`] / [`ModelTier::Heavy`]).
    /// When set on a `models.profiles` entry, the router can pick this profile
    /// for tasks of the matching complexity. `None` means "not eligible for
    /// auto-routing" (manual `/model` switch only).
    #[serde(default)]
    pub tier: Option<ModelTier>,
}

impl ModelEndpoint {
    /// Resolve the effective base_url for this endpoint. If `self.base_url` is None,
    /// fall back to env var `API_BASE_URL`, then "https://api.deepseek.com".
    pub fn endpoint_base_url(&self) -> String {
        if let Some(u) = &self.base_url {
            return u.clone();
        }
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".to_string())
    }

    /// Resolve the effective api_key for this endpoint, checking env first.
    pub fn endpoint_api_key(&self) -> Option<String> {
        std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
            .or_else(|| self.api_key.clone())
    }
}

/// Known context-window sizes (in tokens) for common model names/IDs.
///
/// Returns `None` for unrecognized models so callers can fall back to a
/// configured default. This is a static table built from public model docs;
/// it does not query the API. Model names are matched case-insensitively
/// against both friendly aliases and full IDs.
pub fn known_context_window(model: &str) -> Option<usize> {
    let lower = model.to_ascii_lowercase();
    // Multi-provider: all current models expose ~1M context.
    if lower.starts_with("claude")
        || matches!(
            lower.as_str(),
            "sonnet" | "opus" | "haiku" | "deepseek-v4-pro"
        )
    {
        return Some(1_024_000);
    }
    // DeepSeek - ~1M context.
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
    if lower.starts_with("glm") {
        return Some(1_024_000);
    }
    if lower.starts_with("kimi") {
        return Some(1_024_000);
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

/// Configuration for automatic per-subagent model routing based on task
/// complexity. When enabled, subagent tasks whose `use_small_model` field is
/// **absent** are routed to a [`ModelTier`] by a heuristic score (and an
/// optional LLM fallback for borderline cases). Explicit `use_small_model =
/// true/false` always wins and is unaffected by this config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoutingConfig {
    /// Master switch. When `false`, absent `use_small_model` always falls back
    /// to the main model (pre-routing behavior).
    #[serde(default = "default_routing_enabled")]
    pub enabled: bool,
    /// When `true`, borderline heuristic scores (between `boundary_low` and
    /// `boundary_high`) trigger a one-shot LLM classification call (on the
    /// small model) for a more accurate score. When `false`, borderline cases
    /// resolve to `Medium` without an extra call.
    #[serde(default = "default_llm_fallback")]
    pub llm_fallback: bool,
    /// Heuristic scores below this threshold route to [`ModelTier::Light`].
    #[serde(default = "default_boundary_low")]
    pub boundary_low: f64,
    /// Heuristic scores above this threshold route to [`ModelTier::Heavy`].
    #[serde(default = "default_boundary_high")]
    pub boundary_high: f64,
}

fn default_routing_enabled() -> bool {
    true
}
fn default_llm_fallback() -> bool {
    true
}
fn default_boundary_low() -> f64 {
    0.3
}
fn default_boundary_high() -> f64 {
    0.7
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            enabled: default_routing_enabled(),
            llm_fallback: default_llm_fallback(),
            boundary_low: default_boundary_low(),
            boundary_high: default_boundary_high(),
        }
    }
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
    /// Named switchable model profiles. The key is the profile name the user
    /// types in `/model <name>`; the value is a full [`ModelEndpoint`] that,
    /// when activated, is copied into [`Self::main`] so every downstream path
    /// (subagent `small_model_settings`, fallback, planner) follows
    /// automatically. Empty by default — old configs with only `main` keep
    /// working unchanged.
    #[serde(default)]
    pub profiles: HashMap<String, ModelEndpoint>,
    /// Currently active profile key (`None` = no profile active, `main` is
    /// used as-is). Persisted so a restart preserves the user's last choice.
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Automatic per-subagent model routing based on task complexity.
    /// `None` uses the default ([`ModelRoutingConfig::default`]).
    #[serde(default)]
    pub routing: ModelRoutingConfig,
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
                name: "deepseek-v4-pro".to_string(),
                base_url: std::env::var("API_BASE_URL").ok(),
                api_key: std::env::var("DEEPSEEK_API_KEY")
                    .ok()
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                    .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok()),
                appkey: None,
                provider: None,
                context_window: None,
                display_name: None,
                tier: None,
            },
            small: None,
            planner: None,
            profiles: HashMap::new(),
            active_profile: None,
            routing: ModelRoutingConfig::default(),
            context_window: 200_000,
        }
    }
}

impl ModelsConfig {
    /// Resolve the [`ModelEndpoint`] to use for a given [`ModelTier`].
    ///
    /// Lookup order:
    /// 1. The first `models.profiles` entry whose `tier` matches → that entry.
    /// 2. `Light` falls back to `models.small` (the legacy cheap-model slot)
    ///    when present, then to `main`.
    /// 3. `Medium` / `Heavy` fall back to `main`.
    ///
    /// This keeps the legacy `small` slot working as an implicit `Light` tier
    /// even when the user hasn't declared any `profiles` with `tier`.
    pub fn endpoint_for_tier(&self, tier: ModelTier) -> ModelEndpoint {
        // 1. Prefer a profile that explicitly declares this tier.
        if let Some((_, ep)) = self.profiles.iter().find(|(_, ep)| ep.tier == Some(tier)) {
            return ep.clone();
        }
        // 2. Light → legacy `small` slot, else main.
        match tier {
            ModelTier::Light => self.small.clone().unwrap_or_else(|| self.main.clone()),
            ModelTier::Medium | ModelTier::Heavy => self.main.clone(),
        }
    }

    /// Whether any profile declares the given tier (used by the router to
    /// decide if auto-routing can actually select a non-default model).
    pub fn has_tier(&self, tier: ModelTier) -> bool {
        self.profiles.values().any(|ep| ep.tier == Some(tier))
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
    fn test_model_tier_serde_lowercase() {
        let json = r#"{"name":"x","tier":"light"}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(ep.tier, Some(ModelTier::Light));

        let json = r#"{"name":"x","tier":"heavy"}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(ep.tier, Some(ModelTier::Heavy));

        // Absent tier → None (manual-switch-only profile).
        let json = r#"{"name":"x"}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert!(ep.tier.is_none());
    }

    #[test]
    fn test_routing_config_defaults() {
        let cfg = ModelRoutingConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.llm_fallback);
        assert!((cfg.boundary_low - 0.3).abs() < 1e-9);
        assert!((cfg.boundary_high - 0.7).abs() < 1e-9);
    }

    #[test]
    fn test_routing_config_absent_in_legacy_config() {
        // Old config without a `routing` block deserializes to defaults.
        let json = r#"{"main":{"name":"deepseek-v4-pro"}}"#;
        let cfg: ModelsConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.routing.enabled);
    }

    #[test]
    fn test_endpoint_for_tier_prefers_profile_with_matching_tier() {
        let mut cfg = ModelsConfig::default();
        cfg.profiles.insert(
            "scout".to_string(),
            ModelEndpoint {
                name: "deepseek-chat".to_string(),
                tier: Some(ModelTier::Light),
                ..Default::default()
            },
        );
        cfg.profiles.insert(
            "vanguard".to_string(),
            ModelEndpoint {
                name: "claude-sonnet-4-5".to_string(),
                tier: Some(ModelTier::Heavy),
                ..Default::default()
            },
        );

        assert_eq!(
            cfg.endpoint_for_tier(ModelTier::Light).name,
            "deepseek-chat"
        );
        assert_eq!(
            cfg.endpoint_for_tier(ModelTier::Heavy).name,
            "claude-sonnet-4-5"
        );
        assert!(cfg.has_tier(ModelTier::Light));
        assert!(cfg.has_tier(ModelTier::Heavy));
        assert!(!cfg.has_tier(ModelTier::Medium));
    }

    #[test]
    fn test_endpoint_for_tier_light_falls_back_to_small_then_main() {
        // No profiles, but `small` is configured → Light uses small.
        let cfg = ModelsConfig {
            small: Some(ModelEndpoint {
                name: "deepseek-chat".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            cfg.endpoint_for_tier(ModelTier::Light).name,
            "deepseek-chat"
        );

        // No profiles, no small → Light falls back to main.
        let cfg2 = ModelsConfig::default();
        assert_eq!(
            cfg2.endpoint_for_tier(ModelTier::Light).name,
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn test_endpoint_for_tier_medium_heavy_fall_back_to_main() {
        let cfg = ModelsConfig::default();
        assert_eq!(
            cfg.endpoint_for_tier(ModelTier::Medium).name,
            "deepseek-v4-pro"
        );
        assert_eq!(
            cfg.endpoint_for_tier(ModelTier::Heavy).name,
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn test_context_window_deserialize_default() {
        let json = r#"{"main":{"name":"test"}}"#;
        let cfg: ModelsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.context_window, 200_000);
    }

    #[test]
    fn test_known_context_window_matches_common_models() {
        assert_eq!(known_context_window("deepseek-v4-pro"), Some(1_024_000));
        assert_eq!(known_context_window("deepseek-chat"), Some(1_024_000));
        assert_eq!(known_context_window("v3"), Some(1_024_000));
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
            name: "deepseek-v4-pro".to_string(),
            context_window: Some(150_000),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 200_000), 150_000);
        // 2. Known model lookup when no override (deepseek-v4-pro -> 1M, ignores fallback).
        let ep = ModelEndpoint {
            name: "deepseek-v4-pro".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 999_999), 1_024_000);
        // 3. Unknown model falls back to global.
        let ep = ModelEndpoint {
            name: "my-custom-llm".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 200_000), 200_000);
        // DeepSeek: known lookup returns 1M even when global default is 200k.
        let ep = ModelEndpoint {
            name: "deepseek-chat".to_string(),
            ..Default::default()
        };
        assert_eq!(resolve_context_window(&ep, 200_000), 1_024_000);
    }

    #[test]
    fn test_model_endpoint_context_window_deserialize() {
        let json = r#"{"name":"deepseek-v4-pro","context_window":150000}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(ep.context_window, Some(150_000));
    }

    #[test]
    fn test_display_name_optional_and_defaults_to_name() {
        let json = r#"{"name":"deepseek-chat","display_name":"DeepSeek Chat"}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert_eq!(ep.display_name.as_deref(), Some("DeepSeek Chat"));

        // Absent display_name still deserializes fine.
        let json = r#"{"name":"plain"}"#;
        let ep: ModelEndpoint = serde_json::from_str(json).unwrap();
        assert!(ep.display_name.is_none());
    }

    #[test]
    fn test_profiles_default_empty_for_legacy_config() {
        // Old config with only main — no profiles/active_profile keys.
        let json = r#"{"main":{"name":"deepseek-v4-pro"}}"#;
        let cfg: ModelsConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.profiles.is_empty());
        assert!(cfg.active_profile.is_none());
    }

    #[test]
    fn test_switch_to_profile_replaces_main_and_records_active() {
        use crate::config::Settings;
        let mut settings = Settings::default();
        settings.models.profiles.insert(
            "smart".to_string(),
            ModelEndpoint {
                name: "claude-sonnet-4-5".to_string(),
                base_url: Some("https://api.anthropic.com".to_string()),
                provider: Some("anthropic".to_string()),
                display_name: Some("Claude Sonnet".to_string()),
                ..Default::default()
            },
        );

        // Before switch, main is the default deepseek model.
        assert_eq!(settings.models.main.name, "deepseek-v4-pro");
        assert!(settings.models.active_profile.is_none());

        settings.switch_to_profile("smart").unwrap();

        // main now mirrors the profile endpoint entirely.
        assert_eq!(settings.models.main.name, "claude-sonnet-4-5");
        assert_eq!(
            settings.models.main.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(settings.models.main.provider.as_deref(), Some("anthropic"));
        assert_eq!(settings.models.active_profile.as_deref(), Some("smart"));
    }

    #[test]
    fn test_switch_to_unknown_profile_lists_available() {
        use crate::config::Settings;
        let mut settings = Settings::default();
        settings.models.profiles.insert(
            "fast".to_string(),
            ModelEndpoint {
                name: "deepseek-chat".to_string(),
                ..Default::default()
            },
        );

        let err = settings.switch_to_profile("nope").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown model profile 'nope'"), "{msg}");
        assert!(msg.contains("fast"), "available list should mention 'fast'");
    }
}
