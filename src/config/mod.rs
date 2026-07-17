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
    pub verbose: bool,
}
impl Settings {
    /// Resolve the path to ~/.wgenty-code/settings.json
    fn config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".wgenty-code").join("settings.json")
    }

    /// Load settings from file (disk form, no runtime path resolution).
    ///
    /// Prefer this for mutate-then-save paths so a runtime-resolved absolute
    /// `working_dir` is not persisted. For process use, call [`Self::load`].
    ///
    /// No backward-compatibility migration: an old settings.json containing
    /// flat fields will fail to deserialize.
    pub fn load_from_disk() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .context(format!("Failed to read config file: {}", path.display()))?;
            serde_json::from_str(&content)
                .context(format!("Failed to parse config file: {}", path.display()))
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

    /// Build a Settings clone configured for the small model.
    /// If `models.small` is None, returns a clone of self (no-op).
    /// If `models.small` is Some, overrides `models.main` name/base_url/api_key/appkey
    /// from the small endpoint where present. `transport.max_tokens` is inherited
    /// from the shared transport config (no longer forced to 2048), so subagents
    /// get the same output budget as the main model - a small max_tokens would
    /// truncate large tool-call arguments just like the main-model bug.
    /// (`appkey` if present overrides `api_key` — preserves prior behavior.)
    pub fn small_model_settings(&self) -> Self {
        let mut s = self.clone();
        if let Some(small) = &self.models.small {
            s.models.main.name = small.name.clone();
            if let Some(url) = &small.base_url {
                s.models.main.base_url = Some(url.clone());
            }
            if let Some(key) = &small.api_key {
                s.models.main.api_key = Some(key.clone());
            }
            if let Some(ak) = &small.appkey {
                s.models.main.api_key = Some(ak.clone());
            }
            if let Some(p) = &small.provider {
                s.models.main.provider = Some(p.clone());
            }
        }
        s
    }

    /// Build a Settings clone where subagent override fields (under agent.subagent)
    /// have been folded into the corresponding agent.* fields. Used at subagent spawn
    /// time so the subagent loop can read agent.* directly.
    ///
    /// Special cases:
    /// - max_rounds: subagent override `Some(0)` means "unlimited" (mapped to None).
    /// - subagent_default_k from token_budget is NOT consulted here; it is read by
    ///   the spawn caller separately as a fallback when no subagent override exists.
    pub fn resolve_subagent_config(&self) -> Self {
        let mut s = self.clone();
        let ov = &self.agent.subagent;

        if let Some(b) = ov.token_budget_k {
            s.agent.token_budget.main_k = b;
        }
        if let Some(r) = ov.max_rounds {
            s.agent.max_rounds = if r == 0 { None } else { Some(r) };
        }
        if let Some(p) = ov.plan_mode {
            s.agent.plan_mode = p;
        }

        if let Some(v) = ov.rlm.enabled {
            s.agent.rlm.enabled = v;
        }
        if let Some(v) = ov.rlm.delegate_tool {
            s.agent.rlm.delegate_tool = v;
        }
        if let Some(v) = ov.rlm.auto_routing {
            s.agent.rlm.auto_routing = v;
        }
        if let Some(v) = ov.rlm.retry_enabled {
            s.agent.rlm.retry_enabled = v;
        }
        if let Some(v) = ov.rlm.max_replan_cycles {
            s.agent.rlm.max_replan_cycles = v;
        }
        if let Some(v) = ov.rlm.jaccard_threshold {
            s.agent.rlm.jaccard_threshold = v;
        }

        if let Some(v) = ov.prompt.include.permissions {
            s.prompt.include.permissions = v;
        }
        if let Some(v) = ov.prompt.include.developer {
            s.prompt.include.developer = v;
        }
        if let Some(v) = ov.prompt.include.collaboration {
            s.prompt.include.collaboration = v;
        }
        if let Some(v) = ov.prompt.include.environment {
            s.prompt.include.environment = v;
        }
        if let Some(v) = ov.prompt.include.skills {
            s.prompt.include.skills = v;
        }

        if let Some(v) = &ov.prompt.developer_instructions {
            s.prompt.developer_instructions = Some(v.clone());
        }
        if let Some(v) = &ov.prompt.collaboration_mode {
            s.prompt.collaboration_mode = Some(v.clone());
        }
        if let Some(v) = &ov.prompt.model_instructions_file {
            s.prompt.model_instructions_file = Some(v.clone());
        }

        s
    }

    /// Set a configuration value via dotted path.
    /// Examples:
    ///   set("models.main.name", "sonnet")
    ///   set("agent.subagent.max_depth", "7")
    ///   set("prompt.include.skills", "false")
    ///   set("plugins.enabled_map.foo@bar", "true")
    /// Values are parsed as JSON literals first (so "true"/"42"/"3.14" become bool/number);
    /// on parse failure, the value is treated as a string.
    /// Type validation happens at deserialize time — invalid paths/types return Err
    /// and the on-disk settings.json is left unchanged.
    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        use serde_json::Value;
        // Load disk form so runtime-resolved absolute working_dir is not written back.
        let settings = Self::load_from_disk()?;
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

        set_at(&mut json, &parts, parsed)?;

        let new_settings: Settings = serde_json::from_value(json)
            .map_err(|e| anyhow::anyhow!("invalid setting at '{}': {}", key, e))?;
        new_settings.save()?;
        Ok(())
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
