//! Cross-platform sandbox profile — describes what the sandboxed process may do.

use std::path::{Path, PathBuf};

/// A sandbox profile that is translated into platform-specific enforcement rules.
#[derive(Debug, Clone)]
pub struct SandboxProfile {
    /// Paths the process may read from.
    pub readable_paths: Vec<PathBuf>,

    /// Paths the process may write to.
    pub writable_paths: Vec<PathBuf>,

    /// Network access policy.
    pub network: NetworkPolicy,

    /// Resource limits.
    pub resources: ResourceLimits,

    /// Environment variables to forward to the child process.
    pub env_allowlist: Vec<String>,

    /// Whether the process may spawn subprocesses.
    pub allow_subprocess: bool,

    /// Working directory for the command (None = current directory).
    pub workdir: Option<PathBuf>,
}

/// Network access policy for sandboxed processes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No network access at all.
    None,
    /// Localhost only (127.0.0.1, ::1).
    LoopbackOnly,
    /// Full network access.
    Full,
}

/// Resource limits for sandboxed processes.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum memory in bytes.
    pub max_memory_bytes: u64,
    /// Maximum CPU time in seconds (Linux/Windows only).
    pub max_cpu_seconds: u64,
    /// Maximum wall-clock time in seconds.
    pub max_wall_seconds: u64,
    /// Maximum number of child processes.
    pub max_processes: u32,
    /// Maximum file size in bytes.
    pub max_file_size_bytes: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 512 * 1024 * 1024, // 512 MB
            max_cpu_seconds: 30,
            max_wall_seconds: 30,
            max_processes: 16,
            max_file_size_bytes: 100 * 1024 * 1024, // 100 MB
        }
    }
}

impl SandboxProfile {
    /// Create a default profile scoped to a workspace directory.
    /// The process can read/write within the workspace and /tmp, but has no network access.
    pub fn default_for_workspace(root: &Path) -> Self {
        let tmp = std::env::temp_dir();
        Self {
            readable_paths: vec![root.to_path_buf(), tmp.clone()],
            writable_paths: vec![root.to_path_buf(), tmp],
            network: NetworkPolicy::None,
            resources: ResourceLimits::default(),
            env_allowlist: vec![
                "PATH".into(), "HOME".into(), "USER".into(),
                "LANG".into(), "TMPDIR".into(), "TEMP".into(), "TMP".into(),
            ],
            allow_subprocess: true,
            workdir: Some(root.to_path_buf()),
        }
    }

    /// Create a more permissive profile suitable for interactive REPL use.
    pub fn repl_profile(root: &Path) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| root.to_path_buf());
        let tmp = std::env::temp_dir();
        Self {
            readable_paths: vec![root.to_path_buf(), home, tmp.clone()],
            writable_paths: vec![root.to_path_buf(), tmp],
            network: NetworkPolicy::Full,
            resources: ResourceLimits {
                max_memory_bytes: 2 * 1024 * 1024 * 1024, // 2 GB
                max_wall_seconds: 300,
                ..Default::default()
            },
            env_allowlist: vec!["*".into()],
            allow_subprocess: true,
            workdir: Some(root.to_path_buf()),
        }
    }

    /// Add a readable path to the profile.
    pub fn with_readable_path(mut self, path: PathBuf) -> Self {
        self.readable_paths.push(path);
        self
    }

    /// Add a writable path to the profile.
    pub fn with_writable_path(mut self, path: PathBuf) -> Self {
        self.writable_paths.push(path);
        self
    }

    /// Set the network policy.
    pub fn with_network(mut self, policy: NetworkPolicy) -> Self {
        self.network = policy;
        self
    }

    /// Set the wall-clock timeout.
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.resources.max_wall_seconds = seconds;
        self
    }
}
