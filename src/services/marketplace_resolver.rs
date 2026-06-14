//! Marketplace resolver — loads known marketplaces from GitHub repos,
//! parses .claude-plugin/marketplace.json, resolves plugin sources (3 types).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A known marketplace entry from known_marketplaces.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub source: MarketplaceSourceRef,
    #[serde(rename = "installLocation")]
    pub install_location: PathBuf,
    #[serde(rename = "lastUpdated", default)]
    pub last_updated: Option<String>,
    #[serde(rename = "autoUpdate", default)]
    pub auto_update: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSourceRef {
    pub source: String,
    pub repo: String,
}

/// Known marketplaces registry loaded from ~/.wgenty-code/plugins/known_marketplaces.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownMarketplaces {
    #[serde(flatten)]
    pub marketplaces: HashMap<String, MarketplaceEntry>,
}

/// Load known marketplaces from the given path.
pub fn load_known_marketplaces(path: &Path) -> anyhow::Result<KnownMarketplaces> {
    if !path.exists() {
        return Ok(KnownMarketplaces {
            marketplaces: HashMap::new(),
        });
    }
    let content = std::fs::read_to_string(path)?;
    let registry: KnownMarketplaces = serde_json::from_str(&content)?;
    Ok(registry)
}

/// Plugin entry from .claude-plugin/marketplace.json
#[derive(Debug, Clone, Deserialize)]
pub struct MarketplacePluginEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: PluginSource,
    #[serde(default)]
    pub author: Option<serde_json::Value>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The marketplace index file format (.claude-plugin/marketplace.json)
#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceIndex {
    pub name: String,
    pub owner: String,
    pub plugins: Vec<MarketplacePluginEntry>,
}

/// Three types of plugin sources supported by CC marketplace.
/// Uses `#[serde(untagged)]` for auto-detection:
/// - String → LocalPath
/// - Object with `url` field → GitSource (covers both git-subdir and url)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PluginSource {
    /// Local path in the marketplace repo: `"./plugins/some-plugin"`
    LocalPath(String),
    /// Git-based source (covers both "git-subdir" and "url" CC source types).
    /// - git-subdir: `{"source": "git-subdir", "url": "...", "path": "...", "ref": "..."}`
    /// - url: `{"source": "url", "url": "https://github.com/..."}`
    GitSource {
        url: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        path: Option<String>,
        #[serde(rename = "ref", default)]
        ref_: Option<String>,
    },
}

/// Parse a marketplace index from the repo (tries .claude-plugin/marketplace.json first).
pub fn parse_marketplace_index(repo_path: &Path) -> anyhow::Result<MarketplaceIndex> {
    let index_path = repo_path.join(".claude-plugin").join("marketplace.json");
    if index_path.exists() {
        let content = std::fs::read_to_string(&index_path)?;
        return Ok(serde_json::from_str(&content)?);
    }
    anyhow::bail!("No marketplace index found in {}", repo_path.display())
}

/// Clone (or update) a marketplace repo.
pub async fn ensure_marketplace_cloned(
    entry: &MarketplaceEntry,
    base_cache_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let repo_url = format!("https://github.com/{}", entry.source.repo);
    let target_dir = entry.install_location.clone();

    if target_dir.join(".git").exists() {
        // Already cloned — return the path
        return Ok(target_dir);
    }

    // Ensure parent dirs exist
    if let Some(parent) = target_dir.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo_url])
        .arg(&target_dir)
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to clone marketplace {}: {}",
            repo_url,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(target_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_plugin_source_local_path() {
        let json = "\"./plugins/my-plugin\"";
        let source: PluginSource = serde_json::from_str(json).unwrap();
        match source {
            PluginSource::LocalPath(p) => assert_eq!(p, "./plugins/my-plugin"),
            _ => panic!("Expected LocalPath"),
        }
    }

    #[test]
    fn test_parse_plugin_source_git_subdir() {
        let json = r#"{"source": "git-subdir", "url": "https://github.com/user/repo", "path": "plugins/x", "ref": "main"}"#;
        let source: PluginSource = serde_json::from_str(json).unwrap();
        match source {
            PluginSource::GitSource {
                url, path, ref_, ..
            } => {
                assert_eq!(url, "https://github.com/user/repo");
                assert_eq!(path.as_deref(), Some("plugins/x"));
                assert_eq!(ref_.as_deref(), Some("main"));
            }
            _ => panic!("Expected GitSource"),
        }
    }

    #[test]
    fn test_parse_plugin_source_remote_url() {
        let json = r#"{"source": "url", "url": "https://github.com/user/plugin-repo"}"#;
        let source: PluginSource = serde_json::from_str(json).unwrap();
        match source {
            PluginSource::GitSource { url, .. } => {
                assert_eq!(url, "https://github.com/user/plugin-repo");
            }
            _ => panic!("Expected GitSource"),
        }
    }

    #[test]
    fn test_load_known_marketplaces_nonexistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let registry = load_known_marketplaces(&path).unwrap();
        assert!(registry.marketplaces.is_empty());
    }

    #[test]
    fn test_parse_marketplace_index_nonexistent() {
        let dir = TempDir::new().unwrap();
        let result = parse_marketplace_index(dir.path());
        assert!(result.is_err());
    }
}
