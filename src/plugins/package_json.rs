//! CC format: package.json to PluginManifest adapter.
//!
//! Parses Claude Code plugin package.json format and maps
//! it to the internal PluginManifest type.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Claude Code package.json manifest format.
///
/// Supports the npm-style package.json with CC-specific
/// `.opencode` / `.claude` extension fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageJsonManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<AuthorField>,
    #[serde(default)]
    pub main: Option<String>,
    /// CC extension point (newer format)
    #[serde(default)]
    pub opencode: Option<serde_json::Value>,
    /// CC extension point (legacy format)
    #[serde(default)]
    pub claude: Option<serde_json::Value>,
    /// Capture unknown fields for forward compatibility
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Author field can be a string or an object with name/email.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthorField {
    String(String),
    Object {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        email: Option<String>,
    },
}

impl PackageJsonManifest {
    /// Extract the publisher (scope) from a potentially scoped name.
    ///
    /// Returns `("scope", "bare-name")` for `@scope/bare-name`,
    /// or `(None, "bare-name")` for unscoped names.
    pub fn split_name(name: &str) -> (Option<String>, String) {
        if let Some(rest) = name.strip_prefix('@') {
            if let Some((scope, bare)) = rest.split_once('/') {
                return (Some(scope.to_string()), bare.to_string());
            }
        }
        (None, name.to_string())
    }

    /// Get the publisher from the scoped name.
    pub fn publisher(&self) -> Option<String> {
        Self::split_name(&self.name).0
    }

    /// Get the bare name (without @scope/ prefix).
    pub fn bare_name(&self) -> String {
        Self::split_name(&self.name).1
    }

    /// Get author as a display string.
    pub fn author_string(&self) -> Option<String> {
        self.author.as_ref().map(|a| match a {
            AuthorField::String(s) => s.clone(),
            AuthorField::Object { name, email } => {
                match (name.as_deref(), email.as_deref()) {
                    (Some(n), Some(e)) => format!("{} <{}>", n, e),
                    (Some(n), None) => n.to_string(),
                    (None, Some(e)) => e.to_string(),
                    (None, None) => String::new(),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_name_unscoped() {
        let (publisher, bare) = PackageJsonManifest::split_name("my-plugin");
        assert_eq!(publisher, None);
        assert_eq!(bare, "my-plugin");
    }

    #[test]
    fn test_split_name_scoped() {
        let (publisher, bare) = PackageJsonManifest::split_name("@anthropic/superpowers");
        assert_eq!(publisher.as_deref(), Some("anthropic"));
        assert_eq!(bare, "superpowers");
    }

    #[test]
    fn test_split_name_no_slash_after_scope() {
        // Edge case: @something without /
        let (publisher, bare) = PackageJsonManifest::split_name("@noscope");
        assert_eq!(publisher, None);
        assert_eq!(bare, "@noscope");
    }

    #[test]
    fn test_author_string() {
        let m = PackageJsonManifest {
            name: "test".into(),
            version: "1.0.0".into(),
            description: None,
            author: Some(AuthorField::String("Alice".into())),
            main: None,
            opencode: None,
            claude: None,
            extra: HashMap::new(),
        };
        assert_eq!(m.author_string().as_deref(), Some("Alice"));
    }

    #[test]
    fn test_author_object_string() {
        let m = PackageJsonManifest {
            name: "test".into(),
            version: "1.0.0".into(),
            description: None,
            author: Some(AuthorField::Object {
                name: Some("Bob".into()),
                email: Some("bob@example.com".into()),
            }),
            main: None,
            opencode: None,
            claude: None,
            extra: HashMap::new(),
        };
        assert_eq!(m.author_string().as_deref(), Some("Bob <bob@example.com>"));
    }
}
