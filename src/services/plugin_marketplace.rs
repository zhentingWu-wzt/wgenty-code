//! Plugin Marketplace Service - Plugin management and marketplace
//!
//! Background plugin and marketplace auto-install manager.
//! Supports installing, updating, and managing plugins from various sources.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::marketplace_resolver::{self, KnownMarketplaces};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub enabled: bool,
    pub auto_update: bool,
    pub marketplace_url: String,
    pub trusted_sources: Vec<String>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_update: true,
            marketplace_url: "https://plugins.claude.ai".to_string(),
            trusted_sources: vec!["official".to_string(), "community".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub source: String,
    pub installed_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub dependencies: Vec<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplacePlugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub downloads: usize,
    pub rating: f32,
    pub tags: Vec<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub main: String,
    pub dependencies: HashMap<String, String>,
    pub hooks: HashMap<String, String>,
    pub commands: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginStatus {
    pub enabled: bool,
    pub installed_count: usize,
    pub updates_available: usize,
    pub plugins: Vec<Plugin>,
}

pub struct PluginMarketplaceService {
    config: PluginConfig,
    installed_plugins: Arc<RwLock<HashMap<String, Plugin>>>,
    marketplace_cache: Arc<RwLock<Vec<MarketplacePlugin>>>,
}

impl PluginMarketplaceService {
    pub fn new(_state: Arc<RwLock<AppState>>, config: Option<PluginConfig>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            installed_plugins: Arc::new(RwLock::new(HashMap::new())),
            marketplace_cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn list_installed(&self) -> Vec<Plugin> {
        self.load_installed_plugins().await;
        let plugins = self.installed_plugins.read().await;
        plugins.values().cloned().collect()
    }

    async fn load_installed_plugins(&self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let plugin_dir = home.join(".wgenty-code").join("plugins");

        if !plugin_dir.exists() {
            return;
        }

        let mut plugins = self.installed_plugins.write().await;
        plugins.clear();

        if let Ok(mut entries) = tokio::fs::read_dir(&plugin_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    let manifest_path = path.join("plugin.json");
                    if manifest_path.exists() {
                        if let Ok(content) = tokio::fs::read_to_string(&manifest_path).await {
                            if let Ok(plugin) = serde_json::from_str::<Plugin>(&content) {
                                plugins.insert(plugin.name.clone(), plugin);
                            }
                        }
                    }
                }
            }
        }
    }

    pub async fn search(&self, query: &str) -> Vec<MarketplacePlugin> {
        println!("🔍 Searching marketplace for: {}", query);

        self.fetch_marketplace().await;

        let cache = self.marketplace_cache.read().await;
        cache
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query.to_lowercase())
                    || p.description.to_lowercase().contains(&query.to_lowercase())
                    || p.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query.to_lowercase()))
            })
            .cloned()
            .collect()
    }

    async fn fetch_marketplace(&self) {
        use super::marketplace_resolver;

        let mut cache = self.marketplace_cache.write().await;
        if !cache.is_empty() {
            return; // Already fetched
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let plugins_dir = home.join(".wgenty-code").join("plugins");
        let known_path = plugins_dir.join("known_marketplaces.json");

        let known = match marketplace_resolver::load_known_marketplaces(&known_path) {
            Ok(k) => k,
            Err(_) => return,
        };

        let mut results = Vec::new();
        let base_cache = plugins_dir.join("marketplaces");

        for (_name, entry) in &known.marketplaces {
            let repo_dir = entry.install_location.clone();
            // Try to clone if not yet cached
            if !repo_dir.join(".git").exists() {
                if let Err(e) = marketplace_resolver::ensure_marketplace_cloned(entry, &base_cache).await {
                    tracing::warn!(marketplace = %_name, error = %e, "failed to clone marketplace");
                    continue;
                }
            }

            // Parse index
            if let Ok(index) = marketplace_resolver::parse_marketplace_index(&repo_dir) {
                for p in &index.plugins {
                    results.push(MarketplacePlugin {
                        name: p.name.clone(),
                        version: p.version.clone(),
                        description: p.description.clone(),
                        author: p.author.as_ref()
                            .and_then(|a| a.as_str().map(|s| s.to_string()))
                            .unwrap_or_else(|| index.owner.clone()),
                        downloads: 0,
                        rating: 0.0,
                        tags: p.tags.clone(),
                        homepage: None,
                        repository: None,
                    });
                }
            }
        }

        cache.clear();
        cache.extend(results);
    }

    pub async fn install(&self, plugin_name: &str) -> anyhow::Result<Plugin> {
        use super::marketplace_resolver;

        // First, try to find the plugin in marketplace indexes
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let plugins_dir = home.join(".wgenty-code").join("plugins");
        let known_path = plugins_dir.join("known_marketplaces.json");

        let known = marketplace_resolver::load_known_marketplaces(&known_path)
            .unwrap_or(KnownMarketplaces {
                marketplaces: HashMap::new(),
            });

        let base_cache = plugins_dir.join("marketplaces");
        let mut found_source: Option<marketplace_resolver::PluginSource> = None;
        let mut found_version = String::new();
        let mut found_publisher = String::new();
        let mut found_desc = String::new();

        for (_mkt_name, entry) in &known.marketplaces {
            let repo_dir = entry.install_location.clone();
            if !repo_dir.join(".git").exists() {
                let _ = marketplace_resolver::ensure_marketplace_cloned(entry, &base_cache).await;
            }
            if let Ok(index) = marketplace_resolver::parse_marketplace_index(&repo_dir) {
                if let Some(p) = index.plugins.iter().find(|p| p.name == plugin_name) {
                    found_source = Some(p.source.clone());
                    found_version = p.version.clone();
                    found_publisher = index.owner.clone();
                    found_desc = p.description.clone();
                    break;
                }
            }
        }

        // Compute cache path
        let cache_dir = plugins_dir
            .join("cache")
            .join(&found_publisher)
            .join(plugin_name)
            .join(&found_version);
        tokio::fs::create_dir_all(&cache_dir).await?;

        // Install based on source type
        if let Some(source) = &found_source {
            match source {
                marketplace_resolver::PluginSource::LocalPath(rel_path) => {
                    let src = known.marketplaces.values().next()
                        .map(|e| e.install_location.join(rel_path.trim_start_matches("./")))
                        .unwrap_or_default();
                    if src.exists() {
                        let mut copy_opts = fs_extra::dir::CopyOptions::new();
                        copy_opts.overwrite = true;
                        fs_extra::dir::copy(&src, &cache_dir, &copy_opts)?;
                    }
                }
                marketplace_resolver::PluginSource::GitSource { url, ref_, .. } => {
                    let ref_val = ref_.as_deref().unwrap_or("main");
                    let output = tokio::process::Command::new("git")
                        .args(["clone", "--depth", "1", "--branch", ref_val, url.as_str()])
                        .arg(&cache_dir)
                        .output()
                        .await?;
                    if !output.status.success() {
                        return Err(anyhow::anyhow!(
                            "Failed to clone plugin: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }
                }
            }
        }

        let plugin = Plugin {
            name: plugin_name.to_string(),
            version: found_version,
            description: Some(found_desc),
            author: Some(found_publisher),
            source: "marketplace".to_string(),
            installed_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            enabled: true,
            dependencies: vec![],
            homepage: None,
            repository: None,
        };

        let mut installed = self.installed_plugins.write().await;
        installed.insert(plugin.name.clone(), plugin.clone());

        Ok(plugin)
    }

    pub async fn remove(&self, plugin_name: &str) -> anyhow::Result<()> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let plugin_dir = home.join(".wgenty-code").join("plugins").join(plugin_name);

        if plugin_dir.exists() {
            tokio::fs::remove_dir_all(&plugin_dir).await?;
        }

        let mut installed = self.installed_plugins.write().await;
        installed.remove(plugin_name);

        println!("🗑️ Plugin removed: {}", plugin_name);

        Ok(())
    }

    pub async fn update(&self, plugin_name: &str) -> anyhow::Result<Plugin> {
        println!("⬆️ Updating plugin: {}", plugin_name);

        let mut installed = self.installed_plugins.write().await;

        if let Some(plugin) = installed.get_mut(plugin_name) {
            plugin.version = "latest".to_string();
            plugin.updated_at = Some(Utc::now());

            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let plugin_dir = home.join(".wgenty-code").join("plugins").join(plugin_name);
            let manifest_path = plugin_dir.join("plugin.json");
            let manifest_content = serde_json::to_string_pretty(&*plugin)?;
            tokio::fs::write(&manifest_path, manifest_content).await?;

            println!("✅ Plugin updated: {} v{}", plugin.name, plugin.version);

            return Ok(plugin.clone());
        }

        Err(anyhow::anyhow!("Plugin not found: {}", plugin_name))
    }

    pub async fn update_all(&self) -> anyhow::Result<Vec<String>> {
        println!("⬆️ Updating all plugins...");

        let plugins = self.installed_plugins.read().await;
        let plugin_names: Vec<String> = plugins.keys().cloned().collect();
        drop(plugins);

        let mut updated = Vec::new();
        for name in plugin_names {
            if self.update(&name).await.is_ok() {
                updated.push(name);
            }
        }

        println!("✅ Updated {} plugins", updated.len());
        Ok(updated)
    }

    pub async fn enable(&self, plugin_name: &str) -> anyhow::Result<()> {
        let mut installed = self.installed_plugins.write().await;

        if let Some(plugin) = installed.get_mut(plugin_name) {
            plugin.enabled = true;
            println!("✅ Plugin enabled: {}", plugin_name);
        }

        Ok(())
    }

    pub async fn disable(&self, plugin_name: &str) -> anyhow::Result<()> {
        let mut installed = self.installed_plugins.write().await;

        if let Some(plugin) = installed.get_mut(plugin_name) {
            plugin.enabled = false;
            println!("⏸️ Plugin disabled: {}", plugin_name);
        }

        Ok(())
    }

    pub async fn get_status(&self) -> PluginStatus {
        self.load_installed_plugins().await;
        let plugins = self.installed_plugins.read().await;

        PluginStatus {
            enabled: self.config.enabled,
            installed_count: plugins.len(),
            updates_available: 0,
            plugins: plugins.values().cloned().collect(),
        }
    }

    pub async fn get_plugin(&self, name: &str) -> Option<Plugin> {
        let plugins = self.installed_plugins.read().await;
        plugins.get(name).cloned()
    }

    pub async fn check_updates(&self) -> Vec<(String, String, String)> {
        println!("🔍 Checking for plugin updates...");

        self.fetch_marketplace().await;

        let plugins = self.installed_plugins.read().await;
        let cache = self.marketplace_cache.read().await;

        let mut updates = Vec::new();
        for plugin in plugins.values() {
            if let Some(marketplace) = cache.iter().find(|m| m.name == plugin.name) {
                if marketplace.version != plugin.version {
                    updates.push((
                        plugin.name.clone(),
                        plugin.version.clone(),
                        marketplace.version.clone(),
                    ));
                }
            }
        }

        updates
    }
}
