//! SandboxConfig — builder API and predefined security levels.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::profile::{NetworkPolicy, ResourceLimits, SandboxProfile};

/// Predefined security levels for common use cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityLevel {
    /// Yolo / loose metadata: full network, 2GB, 300s. (Yolo disables OS sandbox.)
    Minimal,
    /// Normal day-to-day: full-disk **read**, workspace write, full network, 2GB, 300s.
    Standard,
    /// Plan / tighter: full-disk **read**, workspace write, no network, 1GB, 120s.
    High,
    /// Untrusted plugins: path-scoped reads (no full-disk read), tight limits, no net/subprocess.
    Paranoid,
}

/// Builder for constructing a SandboxProfile.
pub struct SandboxConfig {
    security_level: SecurityLevel,
    workspace: PathBuf,
    extra_readable: Vec<PathBuf>,
    extra_writable: Vec<PathBuf>,
    network: Option<NetworkPolicy>,
    memory_limit_mb: Option<u64>,
    wall_timeout_secs: Option<u64>,
    cpu_timeout_secs: Option<u64>,
    max_processes: Option<u32>,
    max_file_size_mb: Option<u64>,
    allow_subprocess: Option<bool>,
    env_allowlist: Option<Vec<String>>,
}

impl SandboxConfig {
    /// Start building a sandbox profile.
    pub fn builder(workspace: impl Into<PathBuf>) -> Self {
        Self {
            security_level: SecurityLevel::Standard,
            workspace: workspace.into(),
            extra_readable: Vec::new(),
            extra_writable: Vec::new(),
            network: None,
            memory_limit_mb: None,
            wall_timeout_secs: None,
            cpu_timeout_secs: None,
            max_processes: None,
            max_file_size_mb: None,
            allow_subprocess: None,
            env_allowlist: None,
        }
    }

    /// Set the base security level (preset defaults).
    pub fn security_level(mut self, level: SecurityLevel) -> Self {
        self.security_level = level;
        self
    }

    /// Add a readable path beyond the workspace.
    pub fn readable_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.extra_readable.push(path.into());
        self
    }

    /// Add a writable path beyond the workspace.
    pub fn writable_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.extra_writable.push(path.into());
        self
    }

    /// Override network policy.
    pub fn network(mut self, policy: NetworkPolicy) -> Self {
        self.network = Some(policy);
        self
    }

    /// Override memory limit (MB).
    pub fn memory_limit_mb(mut self, mb: u64) -> Self {
        self.memory_limit_mb = Some(mb);
        self
    }

    /// Override wall-clock timeout (seconds).
    pub fn wall_timeout_secs(mut self, secs: u64) -> Self {
        self.wall_timeout_secs = Some(secs);
        self
    }

    /// Override CPU timeout (seconds).
    pub fn cpu_timeout_secs(mut self, secs: u64) -> Self {
        self.cpu_timeout_secs = Some(secs);
        self
    }

    /// Maximum number of child processes.
    pub fn max_processes(mut self, n: u32) -> Self {
        self.max_processes = Some(n);
        self
    }

    /// Whether subprocess spawning is allowed.
    pub fn allow_subprocess(mut self, allowed: bool) -> Self {
        self.allow_subprocess = Some(allowed);
        self
    }

    /// Environment variable allowlist.
    pub fn env_allowlist(mut self, vars: Vec<String>) -> Self {
        self.env_allowlist = Some(vars);
        self
    }

    /// Build the SandboxProfile.
    pub fn build(self) -> SandboxProfile {
        let defaults = self.level_defaults();
        let tmp = std::env::temp_dir();

        let mut readable = vec![self.workspace.clone(), tmp.clone()];
        readable.extend(self.extra_readable);

        let mut writable = vec![self.workspace, tmp];
        writable.extend(self.extra_writable);

        SandboxProfile {
            readable_paths: readable,
            writable_paths: writable,
            // Codex workspace-write / read-only: unrestricted file-read*, write roots only.
            // Paranoid keeps path-scoped reads for untrusted plugins.
            full_disk_read: defaults.full_disk_read,
            network: self.network.unwrap_or(defaults.network),
            resources: ResourceLimits {
                max_memory_bytes: self.memory_limit_mb.unwrap_or(defaults.memory_mb) * 1024 * 1024,
                max_cpu_seconds: self.cpu_timeout_secs.unwrap_or(defaults.cpu_secs),
                max_wall_seconds: self.wall_timeout_secs.unwrap_or(defaults.wall_secs),
                max_processes: self.max_processes.unwrap_or(defaults.processes),
                max_file_size_bytes: self.max_file_size_mb.unwrap_or(defaults.file_size_mb)
                    * 1024
                    * 1024,
            },
            env_allowlist: self.env_allowlist.unwrap_or(defaults.env_vars),
            allow_subprocess: self.allow_subprocess.unwrap_or(defaults.subprocess),
            workdir: None,
        }
    }

    fn level_defaults(&self) -> LevelDefaults {
        match self.security_level {
            SecurityLevel::Minimal => LevelDefaults {
                network: NetworkPolicy::Full,
                memory_mb: 2048,
                wall_secs: 300,
                cpu_secs: 60,
                processes: 32,
                file_size_mb: 500,
                subprocess: true,
                full_disk_read: true,
                env_vars: vec!["*".into()],
            },
            // Standard: Codex workspace-write style. Full-disk read + workspace write;
            // network Full so cargo/npm/git remotes work without forcing Yolo.
            SecurityLevel::Standard => LevelDefaults {
                network: NetworkPolicy::Full,
                memory_mb: 2048,
                wall_secs: 300,
                cpu_secs: 120,
                processes: 32,
                file_size_mb: 500,
                subprocess: true,
                full_disk_read: true,
                env_vars: vec![
                    "PATH".into(),
                    "HOME".into(),
                    "USER".into(),
                    "LANG".into(),
                    "TMPDIR".into(),
                    "TEMP".into(),
                    "TMP".into(),
                    "CARGO_HOME".into(),
                    "RUSTUP_HOME".into(),
                    "NPM_CONFIG_CACHE".into(),
                    "HTTP_PROXY".into(),
                    "HTTPS_PROXY".into(),
                    "NO_PROXY".into(),
                    "http_proxy".into(),
                    "https_proxy".into(),
                    "no_proxy".into(),
                ],
            },
            // High: Plan / Codex read-only style for network (off). Still full-disk read;
            // writes stay workspace-scoped (shell may still write workspace under Plan —
            // permission layer blocks file tools separately).
            SecurityLevel::High => LevelDefaults {
                network: NetworkPolicy::None,
                memory_mb: 1024,
                wall_secs: 120,
                cpu_secs: 60,
                processes: 16,
                file_size_mb: 100,
                subprocess: true,
                full_disk_read: true,
                env_vars: vec![
                    "PATH".into(),
                    "HOME".into(),
                    "USER".into(),
                    "LANG".into(),
                    "TMPDIR".into(),
                    "TEMP".into(),
                    "TMP".into(),
                ],
            },
            SecurityLevel::Paranoid => LevelDefaults {
                network: NetworkPolicy::None,
                memory_mb: 128,
                wall_secs: 10,
                cpu_secs: 5,
                processes: 0,
                file_size_mb: 10,
                subprocess: false,
                full_disk_read: false,
                env_vars: vec!["PATH".into()],
            },
        }
    }
}

struct LevelDefaults {
    network: NetworkPolicy,
    memory_mb: u64,
    wall_secs: u64,
    cpu_secs: u64,
    processes: u32,
    file_size_mb: u64,
    subprocess: bool,
    full_disk_read: bool,
    env_vars: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults_to_standard() {
        let cfg = SandboxConfig::builder("/tmp/ws");
        assert_eq!(cfg.security_level, SecurityLevel::Standard);
        assert_eq!(cfg.workspace, PathBuf::from("/tmp/ws"));
    }

    #[test]
    fn minimal_level_defaults() {
        let profile = SandboxConfig::builder("/tmp/ws")
            .security_level(SecurityLevel::Minimal)
            .build();
        assert_eq!(profile.network, NetworkPolicy::Full);
        assert_eq!(profile.resources.max_memory_bytes, 2048 * 1024 * 1024);
        assert_eq!(profile.resources.max_wall_seconds, 300);
        assert_eq!(profile.resources.max_processes, 32);
        assert!(profile.allow_subprocess);
        assert_eq!(profile.env_allowlist, vec!["*"]);
    }

    #[test]
    fn standard_level_defaults() {
        let profile = SandboxConfig::builder("/tmp/ws")
            .security_level(SecurityLevel::Standard)
            .build();
        // Full network + full-disk read (Codex workspace-write); writes workspace-scoped.
        assert_eq!(profile.network, NetworkPolicy::Full);
        assert!(profile.full_disk_read);
        assert_eq!(profile.resources.max_memory_bytes, 2048 * 1024 * 1024);
        assert_eq!(profile.resources.max_wall_seconds, 300);
        assert_eq!(profile.resources.max_cpu_seconds, 120);
        assert_eq!(profile.resources.max_processes, 32);
        assert_eq!(profile.resources.max_file_size_bytes, 500 * 1024 * 1024);
        assert!(profile.allow_subprocess);
        assert!(profile.env_allowlist.contains(&"PATH".to_string()));
        assert!(profile.env_allowlist.contains(&"HOME".to_string()));
        assert!(profile.env_allowlist.contains(&"CARGO_HOME".to_string()));
    }

    #[test]
    fn high_level_defaults() {
        let profile = SandboxConfig::builder("/tmp/ws")
            .security_level(SecurityLevel::High)
            .build();
        assert_eq!(profile.network, NetworkPolicy::None);
        assert!(
            profile.full_disk_read,
            "Plan/High matches Codex read-only: unrestricted reads"
        );
        assert_eq!(profile.resources.max_memory_bytes, 1024 * 1024 * 1024);
        assert_eq!(profile.resources.max_wall_seconds, 120);
        assert_eq!(profile.resources.max_cpu_seconds, 60);
        assert_eq!(profile.resources.max_processes, 16);
        assert_eq!(profile.resources.max_file_size_bytes, 100 * 1024 * 1024);
        assert!(profile.env_allowlist.contains(&"PATH".to_string()));
        assert!(profile.env_allowlist.contains(&"HOME".to_string()));
        assert!(profile.env_allowlist.contains(&"USER".to_string()));
    }

    #[test]
    fn paranoid_level_defaults() {
        let profile = SandboxConfig::builder("/tmp/ws")
            .security_level(SecurityLevel::Paranoid)
            .build();
        assert_eq!(profile.network, NetworkPolicy::None);
        assert!(
            !profile.full_disk_read,
            "Paranoid keeps path-scoped reads for untrusted plugins"
        );
        assert_eq!(profile.resources.max_memory_bytes, 128 * 1024 * 1024);
        assert_eq!(profile.resources.max_wall_seconds, 10);
        assert_eq!(profile.resources.max_cpu_seconds, 5);
        assert_eq!(profile.resources.max_processes, 0);
        assert_eq!(profile.resources.max_file_size_bytes, 10 * 1024 * 1024);
        assert!(!profile.allow_subprocess);
        assert_eq!(profile.env_allowlist, vec!["PATH"]);
    }

    #[test]
    fn builder_overrides_level_defaults() {
        let profile = SandboxConfig::builder("/tmp/ws")
            .security_level(SecurityLevel::Standard)
            .memory_limit_mb(256)
            .wall_timeout_secs(120)
            .cpu_timeout_secs(60)
            .max_processes(4)
            .allow_subprocess(false)
            .env_allowlist(vec!["PATH".into()])
            .build();
        assert_eq!(profile.resources.max_memory_bytes, 256 * 1024 * 1024);
        assert_eq!(profile.resources.max_wall_seconds, 120);
        assert_eq!(profile.resources.max_cpu_seconds, 60);
        assert_eq!(profile.resources.max_processes, 4);
        assert!(!profile.allow_subprocess);
        assert_eq!(profile.env_allowlist, vec!["PATH"]);
    }

    #[test]
    fn build_includes_workspace_and_tmp_in_paths() {
        let profile = SandboxConfig::builder("/tmp/ws").build();
        assert!(profile
            .readable_paths
            .iter()
            .any(|p| p == &PathBuf::from("/tmp/ws")));
        assert!(profile
            .writable_paths
            .iter()
            .any(|p| p == &PathBuf::from("/tmp/ws")));
        // tmp dir should be in both readable and writable
        let tmp = std::env::temp_dir();
        assert!(profile.readable_paths.iter().any(|p| p == &tmp));
        assert!(profile.writable_paths.iter().any(|p| p == &tmp));
    }

    #[test]
    fn extra_paths_are_added() {
        let profile = SandboxConfig::builder("/tmp/ws")
            .readable_path("/etc")
            .writable_path("/var/log")
            .build();
        assert!(profile
            .readable_paths
            .iter()
            .any(|p| p == &PathBuf::from("/etc")));
        assert!(profile
            .writable_paths
            .iter()
            .any(|p| p == &PathBuf::from("/var/log")));
    }
}
