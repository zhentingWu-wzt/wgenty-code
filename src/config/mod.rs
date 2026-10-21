//! Configuration Module

pub mod agent;
pub mod api_config;
mod defaults;
pub mod guardian;
pub mod mcp_config;
pub mod models;
pub mod prompts;
pub mod sandbox_settings;
pub mod services;
pub mod watcher;

pub use agent::*;
pub use api_config::ApiConfig;
pub use guardian::*;
pub use mcp_config::{McpConfig, McpServerStatus};
pub use models::*;
pub use prompts::*;
pub use sandbox_settings::*;
pub use services::*;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Main configuration structure (top-level grouped form).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub models: ModelsConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub prompt: PromptConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub integrations: IntegrationsConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub verbose: bool,
}

/// Daemon-process tuning. All fields optional; absent = defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Per-session SSE replay buffer capacity (Task: event replay).
    pub event_buffer_capacity: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            event_buffer_capacity: 1024,
        }
    }
}
impl Settings {
    /// Resolve the path to ~/.wgenty-code/settings.json
    fn config_path() -> PathBuf {
        // `WGENTY_HOME` overrides the user home directory (same layout:
        // $WGENTY_HOME/.wgenty-code/settings.json). This is the canonical way
        // to point the config at a custom location — required for hermetic
        // tests, and useful for running multiple profiles. On Windows,
        // dirs::home_dir() prefers the Known Folder API over the USERPROFILE
        // env var, so env injection must go through this explicit override.
        let home = std::env::var_os("WGENTY_HOME")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".wgenty-code").join("settings.json")
    }

    /// Load settings from file (disk form, no runtime path resolution).
    ///
    /// Prefer this for mutate-then-save paths so a runtime-resolved absolute
    /// `working_dir` is not persisted. For process use, call [`Self::load`].
    ///
    /// No backward-compatibility migration: an old settings.json containing
    /// flat fields will fail to deserialize.
    ///
    /// Model config IS migrated: legacy `models.main` / `models.small` /
    /// `models.planner` are normalized into `models.profiles` +
    /// `models.active_profile` + `models.roles` (see
    /// [`crate::config::models::ModelsConfig::migrate_legacy`]). A subsequent
    /// save upgrades the file to the new format (the legacy keys are no
    /// longer serialized).
    pub fn load_from_disk() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .context(format!("Failed to read config file: {}", path.display()))?;
            let mut value: serde_json::Value = serde_json::from_str(&content)
                .context(format!("Failed to parse config file: {}", path.display()))?;
            // Detect legacy model fields by presence BEFORE deserializing —
            // `migrate_legacy` needs to know whether `main` was explicit
            // (old format: main is authoritative) or absent (new format:
            // profiles + active_profile are the only truth).
            let legacy = value
                .get_mut("models")
                .and_then(|m| m.as_object_mut())
                .map(|m| (m.remove("main"), m.remove("small"), m.remove("planner")));
            let mut settings: Self = serde_json::from_value(value)
                .context(format!("Failed to parse config file: {}", path.display()))?;
            match legacy {
                Some((main, small, planner)) => {
                    if let Some(m) = main {
                        settings.models.main = serde_json::from_value(m).context(format!(
                            "Failed to parse models.main in config file: {}",
                            path.display()
                        ))?;
                    }
                    if let Some(s) = small {
                        settings.models.small =
                            Some(serde_json::from_value(s).context(format!(
                                "Failed to parse models.small in config file: {}",
                                path.display()
                            ))?);
                    }
                    if let Some(p) = planner {
                        settings.models.planner =
                            Some(serde_json::from_value(p).context(format!(
                                "Failed to parse models.planner in config file: {}",
                                path.display()
                            ))?);
                    }
                    settings.models.migrate_legacy(true);
                }
                None => {
                    settings.models.migrate_legacy(false);
                }
            }
            Ok(settings)
        } else {
            let s = Settings::default();
            s.save()?;
            Ok(s)
        }
    }

    /// Load settings for process use. After disk load, [`Self::resolve_working_dir`]
    /// rewrites `storage.working_dir` to an absolute project root so permission
    /// policy and team paths do not keep a fragile `"."` relative root.
    ///
    /// Disk mutation paths ([`Self::set`], [`Self::reset`]) use
    /// [`Self::load_from_disk`] so relative `"."` is preserved on disk.
    pub fn load() -> anyhow::Result<Self> {
        let mut settings = Self::load_from_disk()?;
        settings.resolve_working_dir();
        settings.reconcile_routing();
        Ok(settings)
    }

    /// Bind `storage.working_dir` to a stable absolute project root.
    ///
    /// - `"."` / empty → current process CWD (project root)
    /// - relative path → resolved against CWD
    /// - absolute path → canonicalized when possible
    ///
    /// This is runtime-only. Do not call before `save()` if you need to keep
    /// the on-disk default `"."` portable across machines.
    pub fn resolve_working_dir(&mut self) {
        let raw = &self.storage.working_dir;
        let candidate = if raw.as_os_str().is_empty() || raw.as_path() == Path::new(".") {
            crate::utils::current_project_root()
        } else if raw.is_relative() {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(raw),
                Err(_) => raw.clone(),
            }
        } else {
            raw.clone()
        };
        self.storage.working_dir = candidate
            .canonicalize()
            .unwrap_or_else(|_| crate::utils::current_project_root());
    }

    /// Reconcile the legacy `agent.rlm.auto_routing` switch with the newer
    /// `models.routing.enabled` master switch.
    ///
    /// Routing is enabled only when **both** are on: `models.routing` is the
    /// granular, per-tier control surface, while `auto_routing` is the legacy
    /// kill-switch that long-time users may have set to `false`. Treating the
    /// two as an AND preserves the meaning of disabling `auto_routing`.
    pub fn reconcile_routing(&mut self) {
        if !self.agent.rlm.auto_routing {
            self.models.routing.enabled = false;
        }
    }

    /// Save settings to file (~/.wgenty-code/settings.json) as pretty JSON.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).context(format!(
                "Failed to create config directory: {}",
                dir.display()
            ))?;
        }
        let content = serde_json::to_string_pretty(self).context("Failed to serialize settings")?;
        std::fs::write(&path, content)
            .context(format!("Failed to write config file: {}", path.display()))?;
        Ok(())
    }

    /// Reload settings from file, returning a new instance.
    /// This is intentionally a full reload rather than merge to avoid stale partial state.
    /// Working dir is re-resolved against the current process CWD.
    pub fn reload() -> anyhow::Result<Self> {
        Self::load()
    }

    /// Build a Settings clone configured for the light-tier model (the
    /// cheap-model role: subagent clients and the routing classifier).
    ///
    /// Resolution follows [`crate::config::models::ModelsConfig::endpoint_for_tier`]
    /// (light-tier profile → legacy `models.small` → main), so this is a no-op
    /// clone when no light endpoint is distinct from main. The resolved
    /// endpoint replaces `models.main` wholesale, so per-endpoint overrides
    /// (`context_window`, `temperature`) apply. `transport.max_tokens` is
    /// inherited from the shared transport config — a small max_tokens would
    /// truncate large tool-call arguments just like the main-model bug.
    pub fn small_model_settings(&self) -> Self {
        let mut s = self.clone();
        s.models.main = self
            .models
            .endpoint_for_tier(crate::config::models::ModelTier::Light);
        s
    }

    /// Build a Settings clone for plan-mode generation. `None` when no planner
    /// role is bound (use the active model). Resolution: `models.roles.planner`
    /// profile key → legacy in-memory `models.planner` endpoint → `None`.
    pub fn planner_settings(&self) -> Option<Self> {
        if let Some(key) = &self.models.roles.planner {
            if let Some(ep) = self.models.profiles.get(key) {
                let mut s = self.clone();
                s.models.main = ep.clone();
                return Some(s);
            }
            tracing::warn!(
                key = %key,
                "models.roles.planner points at an unknown profile; falling back to main"
            );
        }
        if let Some(pm) = &self.models.planner {
            let mut s = self.clone();
            // Legacy slot: `None` fields inherit from main (documented
            // contract) — same overlay the load-time migration applies when
            // converting `planner` into a profile.
            s.models.main = crate::config::models::overlay_endpoint(&self.models.main, pm);
            return Some(s);
        }
        None
    }

    /// Build a Settings clone where `models.main.name` is overridden by the
    /// given fallback model name. All other endpoint fields (base_url, api_key,
    /// appkey, provider) are preserved from `self` -- the fallback reuses the
    /// original endpoint. If the endpoint itself is down, the fallback fails
    /// (single-shot constraint terminates, degrades to parent/root model).
    pub fn fallback_model_settings(&self, model_name: &str) -> Self {
        let mut s = self.clone();
        s.models.main.name = model_name.to_string();
        s
    }

    /// Activate a named profile in place: record `active_profile` and sync
    /// the `models.main` runtime cache. Non-destructive — the previously
    /// active model always remains available as its profile.
    /// Returns `Err` if the profile key is not in `models.profiles`, with the
    /// list of available keys for actionable feedback.
    ///
    /// This is the single source of truth for the `/model` switch — the daemon
    /// handler and any future switch surface should call this so behavior stays
    /// consistent. Because it replaces `models.main`, every downstream consumer
    /// (subagent `small_model_settings`, fallback, planner) follows
    /// automatically on the next request.
    pub fn switch_to_profile(&mut self, profile: &str) -> anyhow::Result<()> {
        match self.models.profiles.get(profile) {
            Some(endpoint) => {
                self.models.active_profile = Some(profile.to_string());
                self.models.main = endpoint.clone();
                Ok(())
            }
            None => {
                let mut keys: Vec<&String> = self.models.profiles.keys().collect();
                keys.sort();
                Err(anyhow::anyhow!(
                    "unknown model profile '{}'; available: [{}]",
                    profile,
                    keys.iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }

    /// Human-readable label for `models.main`: `display_name` if set, else
    /// `name`. Used by the TUI picker and status bar.
    pub fn main_model_label(&self) -> String {
        self.models
            .main
            .display_name
            .clone()
            .unwrap_or_else(|| self.models.main.name.clone())
    }

    /// Select the first fallback model name different from `failed_model`.
    /// Returns `None` if `fallback_models` is empty or every entry matches the
    /// failed model.
    pub fn select_fallback_model(&self, failed_model: &str) -> Option<&str> {
        self.agent
            .subagent
            .fallback_models
            .iter()
            .find(|m| m.as_str() != failed_model)
            .map(|m| m.as_str())
    }

    /// Effective subagent LLM round cap.
    ///
    /// The subagent override wins when set (`Some(0)` = unlimited); `None`
    /// inherits `agent.max_rounds`; when both are unset the
    /// [`DEFAULT_MAX_ROUNDS`](crate::config::DEFAULT_MAX_ROUNDS) default
    /// applies.
    pub fn subagent_effective_max_rounds(&self) -> usize {
        resolve_max_rounds(self.agent.subagent.max_rounds.or(self.agent.max_rounds))
    }

    /// Set a configuration value via dotted path.
    /// Examples:
    ///   set("models.profiles.<key>.name", "sonnet")
    ///   set("agent.subagent.max_depth", "7")
    ///   set("prompt.include.skills", "false")
    ///   set("plugins.enabled_map.foo@bar", "true")
    /// Legacy model paths (`models.main.*`, `models.small.*`,
    /// `models.planner.*`) are transparently remapped onto their profiles
    /// equivalents (see [`Self::remap_legacy_models_key`]).
    /// Values are parsed as JSON literals first (so "true"/"42"/"3.14" become bool/number);
    /// on parse failure, the value is treated as a string.
    /// Type validation happens at deserialize time — invalid paths/types return Err
    /// and the on-disk settings.json is left unchanged.
    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        use serde_json::Value;
        // Load disk form so runtime-resolved absolute working_dir is not written back.
        let settings = Self::load_from_disk()?;
        let remapped = Self::remap_legacy_models_key(&settings.models, key);
        let key = remapped.as_str();
        let mut json = serde_json::to_value(&settings)?;

        let parsed: Value =
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()));

        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err(anyhow::anyhow!("Invalid empty key segment in '{}'", key));
        }

        fn set_at(node: &mut Value, parts: &[&str], val: Value) -> anyhow::Result<()> {
            let (head, rest) = parts
                .split_first()
                .ok_or_else(|| anyhow::anyhow!("empty path"))?;
            if rest.is_empty() {
                match node {
                    Value::Object(map) => {
                        map.insert(head.to_string(), val);
                        Ok(())
                    }
                    _ => Err(anyhow::anyhow!(
                        "path segment '{}' is not under an object",
                        head
                    )),
                }
            } else {
                let next = match node {
                    Value::Object(map) => map
                        .entry(head.to_string())
                        .or_insert(Value::Object(Default::default())),
                    _ => {
                        return Err(anyhow::anyhow!(
                            "path segment '{}' is not under an object",
                            head
                        ))
                    }
                };
                set_at(next, rest, val)
            }
        }

        set_at(&mut json, &parts, parsed.clone())?;

        let mut new_settings: Settings = serde_json::from_value(json)
            .map_err(|e| anyhow::anyhow!("invalid setting at '{}': {}", key, e))?;
        // A legacy `models.small.*` set may create/update the "small"
        // profile without a tier — keep it serving the light role so the
        // old key's meaning survives the remap.
        new_settings.models.ensure_small_profile_tier();
        // Unknown keys are silently DROPPED by serde's default (ignore
        // unknown fields) — the incident where a top-level `exec_session.*`
        // write happily "succeeded" while the real key lived under
        // `agent.exec_session.*`. Verify the written path actually survives
        // a serialization round-trip before saving.
        let round_trip = serde_json::to_value(&new_settings)?;
        let mut cursor = &round_trip;
        for (index, segment) in parts.iter().enumerate() {
            let Some(next) = cursor.get(segment) else {
                return Err(anyhow::anyhow!(
                    "unknown setting key '{}' (segment '{}' is not part of the settings schema)",
                    key,
                    segment
                ));
            };
            if index + 1 == parts.len() && next != &parsed {
                return Err(anyhow::anyhow!(
                    "setting '{}' did not round-trip; check the value type",
                    key
                ));
            }
            cursor = next;
        }
        new_settings.save()?;
        Ok(())
    }

    /// Translate legacy `models.main` / `models.small` / `models.planner`
    /// keys onto their profiles equivalents. The remapped path patches the
    /// existing profile object in the JSON view, so `models.main.name = X`
    /// no longer risks clobbering sibling fields (the legacy path replaced
    /// the whole `main` object whenever it was absent from the view).
    fn remap_legacy_models_key(models: &crate::config::models::ModelsConfig, key: &str) -> String {
        use crate::config::models::ModelTier;
        let candidates = [
            (
                "models.main",
                models
                    .active_profile
                    .clone()
                    .unwrap_or_else(|| "main".to_string()),
            ),
            (
                "models.small",
                models
                    .tier_profile_key(ModelTier::Light)
                    .cloned()
                    .unwrap_or_else(|| "small".to_string()),
            ),
            (
                "models.planner",
                models
                    .roles
                    .planner
                    .clone()
                    .unwrap_or_else(|| "planner".to_string()),
            ),
        ];
        for (prefix, target) in candidates {
            if key == prefix || key.starts_with(&format!("{prefix}.")) {
                let rest = &key[prefix.len()..];
                return format!("models.profiles.{target}{rest}");
            }
        }
        key.to_string()
    }

    /// Reset settings to defaults
    pub fn reset() -> anyhow::Result<()> {
        let settings = Settings::default();
        settings.save()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
