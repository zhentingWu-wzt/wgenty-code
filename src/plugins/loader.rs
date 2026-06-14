//! Plugin Loader - Hot loading support

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{PluginManifest, PluginStatus};

pub struct PluginLoader {
    loaded: Arc<RwLock<Vec<LoadedPlugin>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedPlugin {
    pub name: String,
    pub manifest: PluginManifest,
    pub status: PluginStatus,
    pub module: Option<PluginModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginModule {
    Native,
    Wasm { bytes: Vec<u8> },
    Script { path: String },
}

impl PluginLoader {
    pub fn new() -> Self {
        Self {
            loaded: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn load_manifest(&self, plugin_dir: &Path) -> anyhow::Result<PluginManifest> {
        // Priority 1: package.json (Claude Code format)
        let pkg_json_path = plugin_dir.join("package.json");
        if pkg_json_path.exists() {
            return self.try_load_package_json(&pkg_json_path).await;
        }
        // Priority 2: plugin.json (wgenty-code legacy format)
        let plugin_json_path = plugin_dir.join("plugin.json");
        if plugin_json_path.exists() {
            return self.try_load_plugin_json(&plugin_json_path).await;
        }
        Err(anyhow::anyhow!(
            "No manifest found in {}",
            plugin_dir.display()
        ))
    }

    /// Try to load a package.json manifest (Claude Code format).
    async fn try_load_package_json(&self, path: &Path) -> anyhow::Result<PluginManifest> {
        use crate::plugins::package_json::PackageJsonManifest;
        let content = tokio::fs::read_to_string(path).await?;
        let pkg: PackageJsonManifest = serde_json::from_str(&content)?;
        Ok(pkg.into())
    }

    /// Try to load a plugin.json manifest (wgenty-code legacy format).
    async fn try_load_plugin_json(&self, path: &Path) -> anyhow::Result<PluginManifest> {
        let content = tokio::fs::read_to_string(path).await?;
        let manifest: PluginManifest = serde_json::from_str(&content)?;
        Ok(manifest)
    }

    pub async fn load(
        &self,
        plugin_dir: &Path,
        manifest: &PluginManifest,
    ) -> anyhow::Result<LoadedPlugin> {
        let main_path = plugin_dir.join(&manifest.main);

        let module = if main_path.extension().map(|e| e == "wasm").unwrap_or(false) {
            let bytes = tokio::fs::read(&main_path).await?;
            PluginModule::Wasm { bytes }
        } else if main_path.extension().map(|e| e == "rs").unwrap_or(false) {
            PluginModule::Native
        } else {
            PluginModule::Script {
                path: main_path.to_string_lossy().to_string(),
            }
        };

        let loaded = LoadedPlugin {
            name: manifest.name.clone(),
            manifest: manifest.clone(),
            status: PluginStatus::Loaded,
            module: Some(module),
        };

        let mut loaded_list = self.loaded.write().await;
        loaded_list.push(loaded.clone());

        Ok(loaded)
    }

    pub async fn unload(&self, name: &str) -> anyhow::Result<()> {
        let mut loaded_list = self.loaded.write().await;
        loaded_list.retain(|p| p.name != name);
        Ok(())
    }

    pub async fn reload(
        &self,
        plugin_dir: &Path,
        manifest: &PluginManifest,
    ) -> anyhow::Result<LoadedPlugin> {
        self.unload(&manifest.name).await?;
        self.load(plugin_dir, manifest).await
    }

    pub async fn get(&self, name: &str) -> Option<LoadedPlugin> {
        let loaded_list = self.loaded.read().await;
        loaded_list.iter().find(|p| p.name == name).cloned()
    }

    pub async fn list(&self) -> Vec<LoadedPlugin> {
        let loaded_list = self.loaded.read().await;
        loaded_list.clone()
    }

    pub async fn is_loaded(&self, name: &str) -> bool {
        let loaded_list = self.loaded.read().await;
        loaded_list.iter().any(|p| p.name == name)
    }

    pub async fn hot_reload(&self, plugin_dir: &Path) -> anyhow::Result<Option<LoadedPlugin>> {
        let manifest = self.load_manifest(plugin_dir).await?;

        let current = self.get(&manifest.name).await;
        if let Some(current) = current {
            if current.manifest.version != manifest.version {
                println!(
                    "🔄 Hot reloading plugin: {} ({} -> {})",
                    manifest.name, current.manifest.version, manifest.version
                );
                return Ok(Some(self.reload(plugin_dir, &manifest).await?));
            }
        }

        Ok(None)
    }

    pub async fn watch(&self, plugin_dir: &Path) -> anyhow::Result<()> {
        let manifest = self.load_manifest(plugin_dir).await?;
        let main_path = plugin_dir.join(&manifest.main);

        println!("👁️ Watching plugin: {} at {:?}", manifest.name, main_path);

        Ok(())
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    async fn write_file(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).await.unwrap();
    }

    #[tokio::test]
    async fn test_load_manifest_package_json_priority() {
        let dir = TempDir::new().unwrap();
        // Create package.json (CC format) and plugin.json (legacy) with different names
        write_file(
            dir.path(),
            "package.json",
            r#"{"name": "cc-plugin", "version": "2.0.0", "main": "index.js"}"#,
        )
        .await;
        write_file(dir.path(), "plugin.json", r#"{"name": "legacy-plugin", "version": "1.0.0", "main": "legacy.js", "commands": [], "hooks": [], "dependencies": {}, "permissions": [], "enabled": true}"#).await;

        let loader = PluginLoader::new();
        let manifest = loader.load_manifest(dir.path()).await.unwrap();
        // Should load package.json (CC format has priority)
        assert_eq!(manifest.name, "cc-plugin");
        assert_eq!(manifest.version, "2.0.0");
    }

    #[tokio::test]
    async fn test_load_manifest_fallback_to_plugin_json() {
        let dir = TempDir::new().unwrap();
        // Only plugin.json (legacy format)
        write_file(dir.path(), "plugin.json", r#"{"name": "legacy-only", "version": "1.0.0", "main": "legacy.js", "commands": [], "hooks": [], "dependencies": {}, "permissions": [], "enabled": true}"#).await;

        let loader = PluginLoader::new();
        let manifest = loader.load_manifest(dir.path()).await.unwrap();
        assert_eq!(manifest.name, "legacy-only");
        assert_eq!(manifest.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_load_manifest_no_manifest_found() {
        let dir = TempDir::new().unwrap();
        let loader = PluginLoader::new();
        let result = loader.load_manifest(dir.path()).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No manifest found"));
    }
}
