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
    /// Sampling temperature for requests to this endpoint. When `None`, the
    /// field is omitted from the request entirely so the provider applies the
    /// model's own default. Some models (e.g. kimi-k3) reject any value other
    /// than their fixed one — leaving this unset is the safe choice for them,
    /// or set it explicitly (e.g. `1.0`) if the provider requires the field.
    #[serde(default)]
    pub temperature: Option<f32>,
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

/// Overlay the legacy `small` / `planner` slot (`over`) onto `main` (`base`):
/// `Some` fields in `over` win, `None` fields inherit from `base`. `name`
/// always comes from `over`. This is the documented contract for the legacy
/// slots ("`None` for url/key/appkey means inherit from `models.main`") —
/// profiles are complete endpoints and never inherit.
pub(crate) fn overlay_endpoint(base: &ModelEndpoint, over: &ModelEndpoint) -> ModelEndpoint {
    ModelEndpoint {
        name: over.name.clone(),
        base_url: over.base_url.clone().or_else(|| base.base_url.clone()),
        api_key: over.api_key.clone().or_else(|| base.api_key.clone()),
        appkey: over.appkey.clone().or_else(|| base.appkey.clone()),
        provider: over.provider.clone().or_else(|| base.provider.clone()),
        context_window: over.context_window.or(base.context_window),
        temperature: over.temperature.or(base.temperature),
        // No display_name inheritance — labeling the cheap model with the
        // main model's display name would mislead the picker.
        display_name: over.display_name.clone(),
        tier: over.tier,
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

/// Named-role bindings over `models.profiles` — a role points at a profile
/// *key* rather than embedding an endpoint. Keeps "which model serves X" in
/// one place (the profile) while letting multiple roles share it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRoles {
    /// Profile key serving plan-mode generation. `None` = use the active
    /// profile (`models.main` runtime cache).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<String>,
}

/// All model endpoints + shared transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default)]
    pub transport: TransportConfig,
    /// Runtime-materialized view of the active profile (`active_profile`).
    /// NOT serialized — the disk source of truth is `profiles` +
    /// `active_profile`. Kept as a field so the many read-only consumers
    /// (`ApiClient`, TUI status bar, daemon handlers) need no changes; it is
    /// resynced by `migrate_legacy` (load) and `switch_to_profile`.
    #[serde(default = "default_main", skip_serializing)]
    pub main: ModelEndpoint,
    /// Legacy cheap-model slot. Consumed by `migrate_legacy` at load into an
    /// implicit `tier: light` profile; kept deserializable (and honored by
    /// [`Self::endpoint_for_tier`]) so hand-constructed in-memory `Settings`
    /// that never pass through the load path keep working.
    #[serde(default, skip_serializing)]
    pub small: Option<ModelEndpoint>,
    /// Legacy plan-mode endpoint. Consumed by `migrate_legacy` at load into a
    /// profile + `roles.planner` reference.
    #[serde(default, skip_serializing)]
    pub planner: Option<ModelEndpoint>,
    /// Named switchable model profiles. The key is the profile name the user
    /// types in `/model <name>`; the value is a full [`ModelEndpoint`] that,
    /// when activated, is copied into [`Self::main`] so every downstream path
    /// (subagent `small_model_settings`, fallback, planner) follows
    /// automatically. Empty by default — old configs with only `main` keep
    /// working unchanged. Null values are tolerated on load (dropped with a
    /// warning; see [`deserialize_profiles_skip_nulls`]).
    #[serde(default, deserialize_with = "deserialize_profiles_skip_nulls")]
    pub profiles: HashMap<String, ModelEndpoint>,
    /// Currently active profile key (`None` = no profile active, `main` is
    /// used as-is). Persisted so a restart preserves the user's last choice.
    /// After `migrate_legacy` this always points at a real profile key.
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Named-role bindings (planner, …) over `profiles`.
    #[serde(default)]
    pub roles: ModelRoles,
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
                temperature: None,
                display_name: None,
                tier: None,
            },
            small: None,
            planner: None,
            profiles: HashMap::new(),
            active_profile: None,
            roles: ModelRoles::default(),
            routing: ModelRoutingConfig::default(),
            context_window: 200_000,
        }
    }
}

/// Serde default for [`ModelsConfig::main`] — mirrors the endpoint built by
/// [`ModelsConfig::default`] so a new-format file without legacy `main`
/// deserializes identically.
fn default_main() -> ModelEndpoint {
    ModelEndpoint {
        name: "deepseek-v4-pro".to_string(),
        base_url: std::env::var("API_BASE_URL").ok(),
        api_key: std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok()),
        appkey: None,
        provider: None,
        context_window: None,
        temperature: None,
        display_name: None,
        tier: None,
    }
}

/// Serde helper for [`ModelsConfig::profiles`]: tolerate `null` entry values.
/// Files written by intermediate builds of the profiles change stored role
/// bindings as null profile entries; dropping them (with a warning) beats
/// failing the whole config load. A `null`-pointing `active_profile` is
/// recovered by `migrate_legacy`; a `null`-pointing role binding falls back
/// at resolution time. Serialization is unaffected — the in-memory map never
/// holds nulls.
fn deserialize_profiles_skip_nulls<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ModelEndpoint>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    HashMap::<String, Option<ModelEndpoint>>::deserialize(deserializer).map(|raw| {
        raw.into_iter()
            .filter_map(|(key, endpoint)| {
                if endpoint.is_none() {
                    tracing::warn!(profile = %key, "null entry in models.profiles; dropping");
                }
                endpoint.map(|endpoint| (key, endpoint))
            })
            .collect()
    })
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
        // 1. Prefer a profile that explicitly declares this tier. Multiple
        //    candidates resolve deterministically to the alphabetically-first
        //    key (HashMap iteration order must not leak into model choice).
        let mut matching: Vec<(&String, &ModelEndpoint)> = self
            .profiles
            .iter()
            .filter(|(_, ep)| ep.tier == Some(tier))
            .collect();
        if matching.len() > 1 {
            let keys: Vec<&str> = matching.iter().map(|(k, _)| k.as_str()).collect();
            tracing::warn!(
                tier = ?tier,
                candidates = ?keys,
                "multiple profiles declare this tier; using the alphabetically-first key"
            );
        }
        matching.sort_by(|a, b| a.0.cmp(b.0));
        if let Some((_, ep)) = matching.first() {
            return (*ep).clone();
        }
        // 2. Light → legacy `small` slot, else main.
        match tier {
            ModelTier::Light => self
                .small
                .as_ref()
                .map(|small| overlay_endpoint(&self.main, small))
                .unwrap_or_else(|| self.main.clone()),
            ModelTier::Medium | ModelTier::Heavy => self.main.clone(),
        }
    }

    /// Whether any profile declares the given tier (used by the router to
    /// decide if auto-routing can actually select a non-default model).
    pub fn has_tier(&self, tier: ModelTier) -> bool {
        self.profiles.values().any(|ep| ep.tier == Some(tier))
    }

    /// Keep a profile literally named `"small"` serving the light role: when
    /// it exists without a tier and no other profile declares `light`, mark
    /// it. Supports the legacy `config set models.small.*` remap — the
    /// created profile must behave like the legacy `small` slot did.
    pub fn ensure_small_profile_tier(&mut self) {
        let shadowed = self
            .profiles
            .iter()
            .any(|(k, ep)| k != "small" && ep.tier == Some(ModelTier::Light));
        if !shadowed {
            if let Some(ep) = self.profiles.get_mut("small") {
                if ep.tier.is_none() {
                    ep.tier = Some(ModelTier::Light);
                }
            }
        }
    }

    /// The profile key bound to a tier, resolved the same way as
    /// [`Self::endpoint_for_tier`] (alphabetically-first declaring profile).
    /// Does not consider the legacy `small` slot — callers remapping legacy
    /// config paths need a real profile key.
    pub fn tier_profile_key(&self, tier: ModelTier) -> Option<&String> {
        self.profiles
            .iter()
            .filter(|(_, ep)| ep.tier == Some(tier))
            .min_by(|a, b| a.0.cmp(b.0))
            .map(|(k, _)| k)
    }

    /// Normalize legacy `main` / `small` / `planner` fields into the
    /// profiles + roles model. Idempotent; runs at every disk load so a
    /// save naturally upgrades the file to the new format.
    ///
    /// `legacy_fields_present`: the file contained at least one legacy model
    /// key (`main`/`small`/`planner`). When true, the `main` cache is
    /// authoritative for resolving a missing/stale `active_profile`
    /// (name-match first, else synthesize a `"main"` profile). When false
    /// (new format), profiles are the only truth — the alphabetically-first
    /// profile becomes active.
    ///
    /// 1. `active_profile` must point at a real profile (see above).
    /// 2. Legacy `small` becomes an implicit `tier: light` profile (unless a
    ///    light profile already shadows it).
    /// 3. Legacy `planner` becomes a profile + `roles.planner` reference.
    pub fn migrate_legacy(&mut self, legacy_fields_present: bool) {
        // ── 1. active_profile → real profile ─────────────────────────────
        let active_valid = self
            .active_profile
            .as_ref()
            .is_some_and(|k| self.profiles.contains_key(k));
        if !active_valid {
            let resolved = if legacy_fields_present {
                // Old format: legacy main is authoritative. Reuse a profile
                // with the same model name, else preserve main as its own
                // profile so it stays switchable.
                self.profiles
                    .iter()
                    .filter(|(_, ep)| ep.name == self.main.name)
                    .min_by(|a, b| a.0.cmp(b.0))
                    .map(|(k, _)| k.clone())
                    .or_else(|| {
                        let key = unique_profile_key(&self.profiles, "main");
                        self.profiles.insert(key.clone(), self.main.clone());
                        Some(key)
                    })
            } else if !self.profiles.is_empty() {
                // New format without a valid active pointer: profiles are the
                // only truth — deterministically activate the first key.
                self.profiles.keys().min().cloned()
            } else {
                // No profiles, no legacy fields: `main` stands alone.
                None
            };
            self.active_profile = resolved;
        }
        // Resync the runtime cache from the (now valid) active profile.
        if let Some(ep) = self
            .active_profile
            .as_ref()
            .and_then(|k| self.profiles.get(k))
        {
            self.main = ep.clone();
        }

        // ── 2. legacy small → light profile ──────────────────────────────
        if let Some(small) = self.small.take() {
            if self.has_tier(ModelTier::Light) {
                tracing::warn!(
                    "legacy models.small is shadowed by a light-tier profile; dropping it"
                );
            } else {
                let key = unique_profile_key(&self.profiles, "small");
                // Overlay on main so the legacy slot's `None`-fields-inherit
                // contract survives the conversion into a complete profile.
                let mut ep = overlay_endpoint(&self.main, &small);
                ep.tier = Some(ModelTier::Light);
                self.profiles.insert(key, ep);
            }
        }

        // ── 3. legacy planner → profile + role ref ───────────────────────
        if let Some(planner) = self.planner.take() {
            let key = match self
                .profiles
                .iter()
                .filter(|(_, ep)| ep.name == planner.name)
                .min_by(|a, b| a.0.cmp(b.0))
                .map(|(k, _)| k.clone())
            {
                Some(existing) => existing,
                None => {
                    let key = unique_profile_key(&self.profiles, "planner");
                    self.profiles
                        .insert(key.clone(), overlay_endpoint(&self.main, &planner));
                    key
                }
            };
            self.roles.planner.get_or_insert(key);
        }
    }
}

/// `"main"` / `"main-2"` / … — first non-colliding key with the given base.
fn unique_profile_key(profiles: &HashMap<String, ModelEndpoint>, base: &str) -> String {
    let mut candidate = base.to_string();
    let mut n = 2;
    while profiles.contains_key(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
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
    fn test_endpoint_for_tier_light_small_inherits_from_main() {
        // Legacy small with `None` fields inherits them from main (the
        // documented slot contract), instead of replacing main wholesale.
        let mut cfg = ModelsConfig::default();
        cfg.main.base_url = Some("https://relay.example".to_string());
        cfg.main.api_key = Some("main-key".to_string());
        cfg.small = Some(ModelEndpoint {
            name: "haiku".to_string(),
            ..Default::default()
        });
        let light = cfg.endpoint_for_tier(ModelTier::Light);
        assert_eq!(light.name, "haiku");
        assert_eq!(light.base_url.as_deref(), Some("https://relay.example"));
        assert_eq!(light.api_key.as_deref(), Some("main-key"));
    }

    #[test]
    fn test_migrate_synthesizes_main_profile_from_legacy() {
        let mut cfg = ModelsConfig::default();
        cfg.main.name = "glm-5.3".to_string();
        assert!(cfg.profiles.is_empty());
        cfg.migrate_legacy(true);
        assert_eq!(cfg.active_profile.as_deref(), Some("main"));
        assert_eq!(cfg.profiles["main"].name, "glm-5.3");
        // Runtime cache resynced from the active profile.
        assert_eq!(cfg.main.name, "glm-5.3");
    }

    #[test]
    fn test_migrate_reuses_profile_matching_main_by_name() {
        // Old format where a profile already carries main's model: activate
        // it instead of synthesizing a duplicate "main" profile.
        let mut cfg = ModelsConfig::default();
        cfg.main.name = "glm-5.3".to_string();
        cfg.profiles.insert(
            "glm".to_string(),
            ModelEndpoint {
                name: "glm-5.3".to_string(),
                ..Default::default()
            },
        );
        cfg.migrate_legacy(true);
        assert_eq!(cfg.active_profile.as_deref(), Some("glm"));
        assert!(!cfg.profiles.contains_key("main"));
    }

    #[test]
    fn test_migrate_small_becomes_light_profile_with_inheritance() {
        let mut cfg = ModelsConfig::default();
        cfg.main.name = "glm-5.3".to_string();
        cfg.main.base_url = Some("https://relay.example".to_string());
        cfg.small = Some(ModelEndpoint {
            name: "haiku".to_string(),
            ..Default::default()
        });
        cfg.migrate_legacy(true);
        let small = &cfg.profiles["small"];
        assert_eq!(small.tier, Some(ModelTier::Light));
        assert_eq!(small.base_url.as_deref(), Some("https://relay.example"));
        assert!(cfg.small.is_none(), "legacy slot consumed");
        assert!(cfg.has_tier(ModelTier::Light));
    }

    #[test]
    fn test_migrate_small_shadowed_by_light_profile_is_dropped() {
        let mut cfg = ModelsConfig::default();
        cfg.profiles.insert(
            "scout".to_string(),
            ModelEndpoint {
                name: "haiku".to_string(),
                tier: Some(ModelTier::Light),
                ..Default::default()
            },
        );
        cfg.small = Some(ModelEndpoint {
            name: "other".to_string(),
            ..Default::default()
        });
        cfg.migrate_legacy(true);
        assert!(!cfg.profiles.contains_key("small"));
        assert_eq!(cfg.profiles.len(), 2); // scout + synthesized main
    }

    #[test]
    fn test_migrate_planner_becomes_role_ref() {
        let mut cfg = ModelsConfig::default();
        cfg.main.name = "glm-5.3".to_string();
        cfg.planner = Some(ModelEndpoint {
            name: "planner-x".to_string(),
            ..Default::default()
        });
        cfg.migrate_legacy(true);
        assert_eq!(cfg.roles.planner.as_deref(), Some("planner"));
        assert_eq!(cfg.profiles["planner"].name, "planner-x");
        assert!(cfg.planner.is_none(), "legacy slot consumed");
    }

    #[test]
    fn test_migrate_new_format_without_active_activates_first_profile() {
        // New-format file (no legacy keys) missing a valid active pointer:
        // profiles are the only truth — deterministically pick the first key.
        let mut cfg = ModelsConfig::default();
        cfg.profiles.insert(
            "b".to_string(),
            ModelEndpoint {
                name: "beta".to_string(),
                ..Default::default()
            },
        );
        cfg.profiles.insert(
            "a".to_string(),
            ModelEndpoint {
                name: "alpha".to_string(),
                ..Default::default()
            },
        );
        cfg.migrate_legacy(false);
        assert_eq!(cfg.active_profile.as_deref(), Some("a"));
        assert_eq!(cfg.main.name, "alpha", "cache resynced from active");
    }

    #[test]
    fn test_migrate_is_idempotent() {
        let mut cfg = ModelsConfig::default();
        cfg.main.name = "glm-5.3".to_string();
        cfg.main.base_url = Some("https://relay.example".to_string());
        cfg.small = Some(ModelEndpoint {
            name: "haiku".to_string(),
            ..Default::default()
        });
        cfg.migrate_legacy(true);
        let after_first = cfg.clone();
        cfg.migrate_legacy(false); // second pass sees no legacy fields
        assert_eq!(
            serde_json::to_string(&after_first).unwrap(),
            serde_json::to_string(&cfg).unwrap()
        );
        // And the active resolution is stable.
        assert_eq!(cfg.active_profile, after_first.active_profile);
    }

    #[test]
    fn test_tier_profile_key_is_deterministic() {
        let mut cfg = ModelsConfig::default();
        for key in ["b", "a"] {
            cfg.profiles.insert(
                key.to_string(),
                ModelEndpoint {
                    name: format!("{key}-model"),
                    tier: Some(ModelTier::Light),
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            cfg.tier_profile_key(ModelTier::Light).map(String::as_str),
            Some("a")
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

    #[test]
    fn test_profiles_null_entries_are_skipped_on_deserialize() {
        // Files written by intermediate builds of the profiles change stored
        // role bindings as null profile entries; they must not fail the load.
        let json = r#"{
            "profiles": {
                "glm": {"name": "glm-5.3"},
                "planner": null
            },
            "active_profile": "glm"
        }"#;
        let cfg: ModelsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.profiles.len(), 1);
        assert_eq!(cfg.profiles["glm"].name, "glm-5.3");
    }

    #[test]
    fn test_migrate_recovers_active_profile_dropped_as_null() {
        // active_profile pointing at a null (dropped) entry resolves back to
        // a real profile via the new-format branch of migrate_legacy.
        let json = r#"{
            "profiles": {"glm": {"name": "glm-5.3"}, "planner": null},
            "active_profile": "planner"
        }"#;
        let mut cfg: ModelsConfig = serde_json::from_str(json).unwrap();
        cfg.migrate_legacy(false);
        assert_eq!(cfg.active_profile.as_deref(), Some("glm"));
        assert_eq!(cfg.main.name, "glm-5.3");
    }
}
