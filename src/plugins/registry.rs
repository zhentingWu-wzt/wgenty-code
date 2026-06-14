//! Plugin Registry - Plugin registration and management

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{LoadedPlugin, PluginInfo, PluginManifest, PluginStatus};

pub struct PluginRegistry {
    manifests: Arc<RwLock<HashMap<String, PluginManifest>>>,
    loaded: Arc<RwLock<HashMap<String, LoadedPlugin>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            manifests: Arc::new(RwLock::new(HashMap::new())),
            loaded: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register(&self, manifest: PluginManifest) -> anyhow::Result<()> {
        let name = manifest.name.clone();
        let mut manifests = self.manifests.write().await;

        if manifests.contains_key(&name) {
            return Err(anyhow::anyhow!("Plugin already registered: {}", name));
        }

        manifests.insert(name, manifest);
        Ok(())
    }

    pub async fn unregister(&self, name: &str) -> anyhow::Result<()> {
        let mut manifests = self.manifests.write().await;
        let mut loaded = self.loaded.write().await;

        manifests.remove(name);
        loaded.remove(name);

        Ok(())
    }

    pub async fn get(&self, name: &str) -> anyhow::Result<Option<PluginManifest>> {
        let manifests = self.manifests.read().await;
        Ok(manifests.get(name).cloned())
    }

    pub async fn list(&self) -> Vec<PluginInfo> {
        let manifests = self.manifests.read().await;
        let loaded = self.loaded.read().await;

        manifests
            .values()
            .map(|m| {
                let loaded_plugin = loaded.get(&m.name);
                PluginInfo {
                    name: m.name.clone(),
                    version: m.version.clone(),
                    description: m.description.clone(),
                    author: m.author.clone(),
                    status: if loaded_plugin.is_some() {
                        PluginStatus::Loaded
                    } else if m.enabled {
                        PluginStatus::Installed
                    } else {
                        PluginStatus::Disabled
                    },
                    enabled: m.enabled,
                    loaded_at: loaded_plugin.map(|_p| chrono::Utc::now()),
                    commands_count: m.commands.len(),
                    hooks_count: m.hooks.len(),
                }
            })
            .collect()
    }

    pub async fn set_loaded(&self, name: &str, plugin: LoadedPlugin) -> anyhow::Result<()> {
        let mut loaded = self.loaded.write().await;
        loaded.insert(name.to_string(), plugin);
        Ok(())
    }

    pub async fn set_unloaded(&self, name: &str) -> anyhow::Result<()> {
        let mut loaded = self.loaded.write().await;
        loaded.remove(name);
        Ok(())
    }

    pub async fn set_enabled(&self, name: &str, enabled: bool) -> anyhow::Result<()> {
        let mut manifests = self.manifests.write().await;
        if let Some(manifest) = manifests.get_mut(name) {
            manifest.enabled = enabled;
        }
        Ok(())
    }

    pub async fn is_loaded(&self, name: &str) -> bool {
        let loaded = self.loaded.read().await;
        loaded.contains_key(name)
    }

    pub async fn is_enabled(&self, name: &str) -> bool {
        let manifests = self.manifests.read().await;
        manifests.get(name).map(|m| m.enabled).unwrap_or(false)
    }

    pub async fn get_loaded(&self, name: &str) -> Option<LoadedPlugin> {
        let loaded = self.loaded.read().await;
        loaded.get(name).cloned()
    }

    pub async fn count(&self) -> (usize, usize) {
        let manifests = self.manifests.read().await;
        let loaded = self.loaded.read().await;
        (manifests.len(), loaded.len())
    }

    pub async fn update_manifest(
        &self,
        name: &str,
        manifest: PluginManifest,
    ) -> anyhow::Result<()> {
        let mut manifests = self.manifests.write().await;
        manifests.insert(name.to_string(), manifest);
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Vec<PluginInfo> {
        let query_lower = query.to_lowercase();
        let manifests = self.manifests.read().await;

        manifests
            .values()
            .filter(|m| {
                m.name.to_lowercase().contains(&query_lower)
                    || m.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
                    || m.author
                        .as_ref()
                        .map(|a| a.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            })
            .map(|m| PluginInfo {
                name: m.name.clone(),
                version: m.version.clone(),
                description: m.description.clone(),
                author: m.author.clone(),
                status: PluginStatus::Installed,
                enabled: m.enabled,
                loaded_at: None,
                commands_count: m.commands.len(),
                hooks_count: m.hooks.len(),
            })
            .collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Installed Plugin Entry — represents a single installation record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstalledPluginEntry {
    pub scope: String,
    #[serde(rename = "installPath")]
    pub install_path: std::path::PathBuf,
    pub version: String,
    #[serde(rename = "installedAt")]
    pub installed_at: String,
    #[serde(rename = "lastUpdated")]
    pub last_updated: String,
    #[serde(rename = "gitCommitSha")]
    #[serde(default)]
    pub git_commit_sha: Option<String>,
}

/// Installed Plugins Registry — persistent format for CC-compatible plugin tracking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstalledPluginsRegistry {
    pub version: u32,
    pub plugins: HashMap<String, Vec<InstalledPluginEntry>>,
}

/// Load installed plugins registry from the given path.
pub fn load_installed_registry(path: &std::path::Path) -> anyhow::Result<InstalledPluginsRegistry> {
    if !path.exists() {
        return Ok(InstalledPluginsRegistry {
            version: 2,
            plugins: HashMap::new(),
        });
    }
    let content = std::fs::read_to_string(path)?;
    let registry: InstalledPluginsRegistry = serde_json::from_str(&content)?;
    Ok(registry)
}

/// Save installed plugins registry to the given path.
pub fn save_installed_registry(
    registry: &InstalledPluginsRegistry,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Atomic write: write to temp file then rename
    let tmp_path = path.with_extension("tmp");
    let content = serde_json::to_string_pretty(registry)?;
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_registry() -> InstalledPluginsRegistry {
        let mut plugins = HashMap::new();
        plugins.insert(
            "superpowers@claude-plugins-official".to_string(),
            vec![InstalledPluginEntry {
                scope: "user".to_string(),
                install_path: std::path::PathBuf::from(
                    "/home/user/.wgenty-code/plugins/cache/claude-plugins-official/superpowers/5.1.0",
                ),
                version: "5.1.0".to_string(),
                installed_at: "2026-06-14T10:00:00Z".to_string(),
                last_updated: "2026-06-14T10:00:00Z".to_string(),
                git_commit_sha: Some("abc123def".to_string()),
            }],
        );
        InstalledPluginsRegistry {
            version: 2,
            plugins,
        }
    }

    #[test]
    fn test_installed_registry_serialize_deserialize() {
        let registry = sample_registry();
        let json = serde_json::to_string_pretty(&registry).unwrap();

        // Verify key fields in JSON
        assert!(json.contains("\"version\": 2"));
        assert!(json.contains("superpowers@claude-plugins-official"));
        assert!(json.contains("\"installPath\""));
        assert!(json.contains("\"gitCommitSha\""));
        assert!(json.contains("abc123def"));

        // Round-trip
        let parsed: InstalledPluginsRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.plugins.len(), 1);
        let entries = &parsed.plugins["superpowers@claude-plugins-official"];
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version, "5.1.0");
        assert_eq!(entries[0].git_commit_sha.as_deref(), Some("abc123def"));
    }

    #[test]
    fn test_load_installed_registry_nonexistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let registry = load_installed_registry(&path).unwrap();
        assert_eq!(registry.version, 2);
        assert!(registry.plugins.is_empty());
    }

    #[test]
    fn test_save_and_load_installed_registry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("installed_plugins.json");
        let registry = sample_registry();

        save_installed_registry(&registry, &path).unwrap();
        assert!(path.exists());

        let loaded = load_installed_registry(&path).unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.plugins.len(), 1);
        let entries = &loaded.plugins["superpowers@claude-plugins-official"];
        assert_eq!(entries[0].version, "5.1.0");
        assert_eq!(entries[0].git_commit_sha.as_deref(), Some("abc123def"));
    }

    #[test]
    fn test_save_installed_registry_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("subdir").join("installed_plugins.json");
        let registry = sample_registry();

        save_installed_registry(&registry, &path).unwrap();
        assert!(path.exists());
    }
}
