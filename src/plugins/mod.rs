//! Plugins Module - Complete plugin system
//!
//! Features:
//! - Custom commands
//! - Hook system
//! - Hot loading support
//! - Plugin isolation

pub mod commands;
pub mod hooks;
pub mod isolation;
pub mod loader;
pub mod package_json;
pub mod registry;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};

pub use commands::{CommandRegistry, PluginCommand};
pub use hooks::{Hook, HookManager, HookPoint};
pub use isolation::{IsolationConfig, PluginSandbox};
pub use loader::{LoadedPlugin, PluginLoader};
pub use registry::PluginRegistry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub main: String,
    pub commands: Vec<PluginCommandDef>,
    pub hooks: Vec<String>,
    pub dependencies: HashMap<String, String>,
    pub permissions: Vec<String>,
    pub enabled: bool,
    /// Publisher name extracted from @scope prefix (e.g., "anthropic" from "@anthropic/test")
    #[serde(default)]
    pub publisher: Option<String>,
    /// Install path for CC-format plugins (cache/<publisher>/<name>/<version>)
    #[serde(default)]
    pub install_path: Option<PathBuf>,
    /// Git commit SHA for plugins installed from git repositories
    #[serde(default)]
    pub git_commit_sha: Option<String>,
    /// Source format marker: "cc" for Claude Code format, "wgenty" for legacy
    #[serde(default)]
    pub source_format: Option<String>,
}

impl PluginManifest {
    pub fn new(name: &str, version: &str, main: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            description: None,
            author: None,
            license: None,
            repository: None,
            main: main.to_string(),
            commands: Vec::new(),
            hooks: Vec::new(),
            dependencies: HashMap::new(),
            permissions: Vec::new(),
            enabled: true,
            publisher: None,
            install_path: None,
            git_commit_sha: None,
            source_format: None,
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn with_author(mut self, author: &str) -> Self {
        self.author = Some(author.to_string());
        self
    }

    pub fn with_command(mut self, command: PluginCommandDef) -> Self {
        self.commands.push(command);
        self
    }

    pub fn with_hook(mut self, hook: &str) -> Self {
        self.hooks.push(hook.to_string());
        self
    }

    pub fn with_permission(mut self, permission: &str) -> Self {
        self.permissions.push(permission.to_string());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCommandDef {
    pub name: String,
    pub description: String,
    pub usage: Option<String>,
    pub examples: Vec<String>,
}

impl PluginCommandDef {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            usage: None,
            examples: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub status: PluginStatus,
    pub enabled: bool,
    pub loaded_at: Option<DateTime<Utc>>,
    pub commands_count: usize,
    pub hooks_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginStatus {
    Installed,
    Loaded,
    Error,
    Disabled,
}

pub struct PluginManager {
    registry: Arc<PluginRegistry>,
    loader: Arc<PluginLoader>,
    sandbox: Arc<PluginSandbox>,
    hook_manager: Arc<HookManager>,
    command_registry: Arc<CommandRegistry>,
    plugins_dir: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let plugins_dir = home.join(".wgenty-code").join("plugins");

        Self {
            registry: Arc::new(PluginRegistry::new()),
            loader: Arc::new(PluginLoader::new()),
            sandbox: Arc::new(PluginSandbox::new(Default::default())),
            hook_manager: Arc::new(HookManager::new()),
            command_registry: Arc::new(CommandRegistry::new()),
            plugins_dir,
        }
    }

    pub fn with_plugins_dir(mut self, dir: PathBuf) -> Self {
        self.plugins_dir = dir;
        self
    }

    pub async fn list(&self) -> anyhow::Result<Vec<PluginInfo>> {
        let plugins = self.registry.list().await;
        Ok(plugins)
    }

    pub async fn install(&self, source: &str) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.plugins_dir)?;

        let plugin_name = source.rsplit('/').next().unwrap_or(source);
        let plugin_dir = self.plugins_dir.join(plugin_name);

        if source.starts_with("http") || source.starts_with("git") {
            tracing::info!(source, "cloning plugin");
            let output = tokio::process::Command::new("git")
                .args(["clone", source, &plugin_dir.to_string_lossy()])
                .output()
                .await?;

            if !output.status.success() {
                return Err(anyhow::anyhow!(
                    "Failed to clone plugin: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else if std::path::Path::new(source).exists() {
            tracing::info!(source, "copying plugin");
            fs_extra::dir::copy(
                source,
                &self.plugins_dir,
                &fs_extra::dir::CopyOptions::new(),
            )?;
        } else {
            return Err(anyhow::anyhow!("Plugin source not found: {}", source));
        }

        let manifest = self.loader.load_manifest(&plugin_dir).await?;
        self.registry.register(manifest).await?;

        tracing::info!(plugin_name, "plugin installed");
        Ok(())
    }

    pub async fn remove(&self, name: &str) -> anyhow::Result<()> {
        self.registry.unregister(name).await?;

        let plugin_dir = self.plugins_dir.join(name);
        if plugin_dir.exists() {
            std::fs::remove_dir_all(&plugin_dir)?;
        }

        tracing::info!(name, "plugin removed");
        Ok(())
    }

    pub async fn load(&self, name: &str) -> anyhow::Result<()> {
        let manifest = self
            .registry
            .get(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;

        let plugin_dir = self.plugins_dir.join(name);
        let loaded = self.loader.load(&plugin_dir, &manifest).await?;

        for cmd in &manifest.commands {
            self.command_registry.register(cmd.clone()).await;
        }

        for hook in &manifest.hooks {
            self.hook_manager.register(hook.parse()?, name).await;
        }

        self.registry.set_loaded(name, loaded).await?;
        tracing::info!(name, "plugin loaded");
        Ok(())
    }

    pub async fn unload(&self, name: &str) -> anyhow::Result<()> {
        let manifest = self
            .registry
            .get(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;

        for cmd in &manifest.commands {
            self.command_registry.unregister(&cmd.name).await;
        }

        for hook in &manifest.hooks {
            self.hook_manager.unregister(&hook.parse()?, name).await;
        }

        self.registry.set_unloaded(name).await?;
        tracing::info!(name, "plugin unloaded");
        Ok(())
    }

    pub async fn reload(&self, name: &str) -> anyhow::Result<()> {
        self.unload(name).await?;
        self.load(name).await
    }

    pub async fn enable(&self, name: &str) -> anyhow::Result<()> {
        self.registry.set_enabled(name, true).await?;
        self.load(name).await
    }

    pub async fn disable(&self, name: &str) -> anyhow::Result<()> {
        self.unload(name).await?;
        self.registry.set_enabled(name, false).await
    }

    pub async fn update(&self, name: &str) -> anyhow::Result<()> {
        let plugin_dir = self.plugins_dir.join(name);

        if plugin_dir.join(".git").exists() {
            tracing::info!(name, "updating plugin");
            let output = tokio::process::Command::new("git")
                .args(["pull"])
                .current_dir(&plugin_dir)
                .output()
                .await?;

            if output.status.success() {
                self.reload(name).await?;
                tracing::info!(name, "plugin updated");
            } else {
                return Err(anyhow::anyhow!(
                    "Failed to update plugin: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else {
            tracing::warn!(name, "plugin is not a git repository, skipping update");
        }

        Ok(())
    }

    pub async fn update_all(&self) -> anyhow::Result<()> {
        let plugins = self.registry.list().await;
        for plugin in plugins {
            if plugin.enabled {
                let _ = self.update(&plugin.name).await;
            }
        }
        Ok(())
    }

    pub async fn load_all(&self) -> anyhow::Result<()> {
        if !self.plugins_dir.exists() {
            return Ok(());
        }

        // Phase 1: Scan cache/<publisher>/<plugin>/<version>/ (CC format)
        let cache_dir = self.plugins_dir.join("cache");
        let mut loaded_names = std::collections::HashSet::new();

        if cache_dir.exists() {
            // Walk up to 3 levels: cache/<publisher>/<plugin>/<version>/package.json
            self.scan_cache_dir(&cache_dir, &mut loaded_names).await;
        }

        // Phase 2: Scan flat directories (legacy format)
        self.scan_flat_dirs(&loaded_names).await;

        // Phase 3: Merge installed_plugins.json metadata
        let installed_path = self.plugins_dir.join("installed_plugins.json");
        if installed_path.exists() {
            if let Ok(registry) = crate::plugins::registry::load_installed_registry(&installed_path)
            {
                for (key, entries) in &registry.plugins {
                    if let Some(entry) = entries.first() {
                        if let Some(manifest) = self.registry.get(key).await.unwrap_or(None) {
                            let mut enriched = manifest;
                            enriched.install_path = Some(entry.install_path.clone());
                            enriched.git_commit_sha = entry.git_commit_sha.clone();
                            let _ = self.registry.register(enriched).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Scan cache/<publisher>/<plugin>/<version>/ for CC-format plugins.
    async fn scan_cache_dir(
        &self,
        cache_dir: &std::path::Path,
        loaded_names: &mut std::collections::HashSet<String>,
    ) {
        // Walk publisher dirs
        if let Ok(publisher_entries) = std::fs::read_dir(cache_dir) {
            for pub_entry in publisher_entries.flatten() {
                if !pub_entry.path().is_dir() {
                    continue;
                }
                // Walk plugin dirs
                if let Ok(plugin_entries) = std::fs::read_dir(pub_entry.path()) {
                    for plug_entry in plugin_entries.flatten() {
                        if !plug_entry.path().is_dir() {
                            continue;
                        }
                        // Walk version dirs
                        if let Ok(version_entries) = std::fs::read_dir(plug_entry.path()) {
                            for ver_entry in version_entries.flatten() {
                                if !ver_entry.path().is_dir() {
                                    continue;
                                }
                                let pkg_json = ver_entry.path().join("package.json");
                                if pkg_json.exists() {
                                    match self.loader.load_manifest(&ver_entry.path()).await {
                                        Ok(manifest) => {
                                            let key = format!(
                                                "{}@{}",
                                                manifest.name,
                                                manifest.publisher.as_deref().unwrap_or("unknown")
                                            );
                                            loaded_names.insert(key.clone());
                                            loaded_names.insert(manifest.name.clone());
                                            let _ = self.registry.register(manifest).await;
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                path = %ver_entry.path().display(),
                                                error = %e,
                                                "failed to load CC plugin manifest"
                                            );
                                        }
                                    }
                                } else {
                                    // Also try plugin.json in version dir
                                    let plugin_json = ver_entry.path().join("plugin.json");
                                    if plugin_json.exists() {
                                        match self.loader.load_manifest(&ver_entry.path()).await {
                                            Ok(manifest) => {
                                                let key = format!(
                                                    "{}@{}",
                                                    manifest.name,
                                                    manifest
                                                        .publisher
                                                        .as_deref()
                                                        .unwrap_or("unknown")
                                                );
                                                loaded_names.insert(key);
                                                loaded_names.insert(manifest.name.clone());
                                                let _ = self.registry.register(manifest).await;
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    path = %ver_entry.path().display(),
                                                    error = %e,
                                                    "failed to load CC plugin manifest"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Scan flat plugin dirs (legacy format), skipping names already loaded from cache.
    async fn scan_flat_dirs(&self, loaded_names: &std::collections::HashSet<String>) {
        if let Ok(entries) = std::fs::read_dir(&self.plugins_dir) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                // Skip the cache directory itself
                if entry.file_name() == "cache" {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip if already loaded from cache (CC format priority)
                if loaded_names.contains(&name) {
                    tracing::info!(name, "skipping legacy plugin, CC format already loaded");
                    continue;
                }
                match self.loader.load_manifest(&entry.path()).await {
                    Ok(manifest) => {
                        let _ = self.registry.register(manifest).await;
                    }
                    Err(e) => {
                        tracing::warn!(name, error = %e, "failed to load legacy plugin");
                    }
                }
            }
        }
    }

    pub fn registry(&self) -> Arc<PluginRegistry> {
        self.registry.clone()
    }

    pub fn hook_manager(&self) -> Arc<HookManager> {
        self.hook_manager.clone()
    }

    pub fn command_registry(&self) -> Arc<CommandRegistry> {
        self.command_registry.clone()
    }

    pub fn sandbox(&self) -> Arc<PluginSandbox> {
        self.sandbox.clone()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cc_compat_fields_serde() {
        let json = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "main": "index.js",
            "commands": [],
            "hooks": [],
            "dependencies": {},
            "permissions": [],
            "enabled": true,
            "publisher": "anthropic",
            "install_path": "/home/user/.wgenty-code/plugins/cache/anthropic/test-plugin/1.0.0",
            "git_commit_sha": "abc123def456",
            "source_format": "cc"
        }"#;

        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.publisher.as_deref(), Some("anthropic"));
        assert!(manifest.install_path.is_some());
        assert_eq!(manifest.git_commit_sha.as_deref(), Some("abc123def456"));
        assert_eq!(manifest.source_format.as_deref(), Some("cc"));
    }

    #[test]
    fn test_cc_compat_fields_default_none() {
        // Old JSON without CC fields should deserialize with None defaults
        let json = r#"{
            "name": "legacy-plugin",
            "version": "1.0.0",
            "main": "index.js",
            "commands": [],
            "hooks": [],
            "dependencies": {},
            "permissions": [],
            "enabled": true
        }"#;

        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.publisher, None);
        assert_eq!(manifest.install_path, None);
        assert_eq!(manifest.git_commit_sha, None);
        assert_eq!(manifest.source_format, None);
    }

    #[test]
    fn manifest_new_sets_defaults() {
        let m = PluginManifest::new("test", "1.0.0", "main.js");
        assert_eq!(m.name, "test");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.main, "main.js");
        assert_eq!(m.description, None);
        assert_eq!(m.author, None);
        assert!(m.commands.is_empty());
        assert!(m.hooks.is_empty());
        assert!(m.enabled);
    }

    #[test]
    fn manifest_builder_sets_fields() {
        let cmd = PluginCommandDef::new("greet", "Say hello");
        let m = PluginManifest::new("test", "1.0.0", "main.js")
            .with_description("A test plugin")
            .with_author("tester")
            .with_command(cmd);
        assert_eq!(m.description.as_deref(), Some("A test plugin"));
        assert_eq!(m.author.as_deref(), Some("tester"));
        assert_eq!(m.commands.len(), 1);
        assert_eq!(m.commands[0].name, "greet");
    }
}
