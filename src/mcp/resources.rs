//! MCP Resources - Resource management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    pub server_name: Option<String>,
}

impl Resource {
    pub fn new(uri: &str, name: &str) -> Self {
        Self {
            uri: uri.to_string(),
            name: name.to_string(),
            description: None,
            mime_type: None,
            server_name: None,
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn with_mime_type(mut self, mime_type: &str) -> Self {
        self.mime_type = Some(mime_type.to_string());
        self
    }

    pub fn with_server(mut self, server_name: &str) -> Self {
        self.server_name = Some(server_name.to_string());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTemplate {
    pub uri_template: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob: Option<String>,
}

pub struct ResourceManager {
    resources: Arc<RwLock<HashMap<String, Resource>>>,
    templates: Arc<RwLock<Vec<ResourceTemplate>>>,
    /// Allowed root directory for file:// access. Paths outside this
    /// root are rejected to prevent arbitrary filesystem reads.
    workspace_root: Arc<RwLock<Option<std::path::PathBuf>>>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            resources: Arc::new(RwLock::new(HashMap::new())),
            templates: Arc::new(RwLock::new(Vec::new())),
            workspace_root: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the workspace root for file:// path validation.
    pub async fn set_workspace_root(&self, root: std::path::PathBuf) {
        let mut ws = self.workspace_root.write().await;
        *ws = Some(root);
    }

    pub async fn register(&self, resource: Resource) {
        let mut resources = self.resources.write().await;
        resources.insert(resource.uri.clone(), resource);
    }

    pub async fn unregister(&self, uri: &str) {
        let mut resources = self.resources.write().await;
        resources.remove(uri);
    }

    pub async fn get(&self, uri: &str) -> Option<Resource> {
        let resources = self.resources.read().await;
        resources.get(uri).cloned()
    }

    pub async fn list(&self) -> Vec<Resource> {
        let resources = self.resources.read().await;
        resources.values().cloned().collect()
    }

    pub async fn add_template(&self, template: ResourceTemplate) {
        let mut templates = self.templates.write().await;
        templates.push(template);
    }

    pub async fn list_templates(&self) -> Vec<ResourceTemplate> {
        let templates = self.templates.read().await;
        templates.clone()
    }

    pub async fn read(&self, uri: &str) -> anyhow::Result<ResourceContent> {
        let resources = self.resources.read().await;
        let resource = resources
            .get(uri)
            .ok_or_else(|| anyhow::anyhow!("Resource not found: {}", uri))?;

        if uri.starts_with("file://") {
            let path_str = uri.trim_start_matches("file://");
            let path = std::path::Path::new(path_str);

            // Security: validate the path is within the workspace root
            let ws = self.workspace_root.read().await;
            if let Some(ref root) = *ws {
                // Canonicalize both paths to resolve symlinks and `..`
                let canon_path = std::fs::canonicalize(path)
                    .unwrap_or_else(|_| path.to_path_buf());
                let canon_root = std::fs::canonicalize(root)
                    .unwrap_or_else(|_| root.clone());

                if !canon_path.starts_with(&canon_root) {
                    return Err(anyhow::anyhow!(
                        "Access denied: path {} is outside workspace root {}",
                        path_str,
                        canon_root.display()
                    ));
                }
            } else {
                // No workspace root set — reject all file:// reads for safety
                return Err(anyhow::anyhow!(
                    "file:// access denied: workspace root not configured"
                ));
            }

            let content = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;

            Ok(ResourceContent {
                uri: uri.to_string(),
                mime_type: resource.mime_type.clone(),
                text: Some(content),
                blob: None,
            })
        } else if uri.starts_with("memory://") {
            Ok(ResourceContent {
                uri: uri.to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some("{}\n".to_string()),
                blob: None,
            })
        } else {
            Err(anyhow::anyhow!("Unsupported URI scheme: {}", uri))
        }
    }

    pub async fn register_builtin_resources(&self, project_path: &std::path::Path) {
        // Set workspace root for file:// access validation
        self.set_workspace_root(project_path.to_path_buf()).await;

        self.register(
            Resource::new(
                &format!("file://{}/", project_path.display()),
                "Project Root",
            )
            .with_description("Project root directory")
            .with_mime_type("inode/directory"),
        )
        .await;

        self.add_template(ResourceTemplate {
            uri_template: "file://{path}".to_string(),
            name: "File Resource".to_string(),
            description: Some("Access any file in the project".to_string()),
            mime_type: None,
        })
        .await;

        self.add_template(ResourceTemplate {
            uri_template: "memory://{key}".to_string(),
            name: "Memory Resource".to_string(),
            description: Some("Access stored memory".to_string()),
            mime_type: Some("application/json".to_string()),
        })
        .await;
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}
