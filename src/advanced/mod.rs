//! Advanced Features Module
//!
//! Features:
//! - SSH connection support
//! - Remote execution
//! - Project initialization

pub mod project_init;
pub mod remote;
pub mod ssh;

use serde::{Deserialize, Serialize};

pub use project_init::{ProjectConfig, ProjectInitializer, ProjectTemplate};
pub use remote::{RemoteConfig, RemoteExecutor, RemoteResult};
pub use ssh::{SshClient, SshConfig, SshSession};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedConfig {
    pub ssh: SshConfig,
    pub remote: RemoteConfig,
    pub project: ProjectConfig,
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            ssh: SshConfig::default(),
            remote: RemoteConfig::default(),
            project: ProjectConfig::default(),
        }
    }
}
