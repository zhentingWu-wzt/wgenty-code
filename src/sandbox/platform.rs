//! Platform detection and capability probing.

/// Identifies the current operating system and its sandbox capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    Linux,
    Windows,
    Unknown,
}

impl Platform {
    /// Detect the current platform at runtime.
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOS
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Unknown
        }
    }

    /// Human-readable platform name.
    pub fn name(&self) -> &str {
        match self {
            Platform::MacOS => "macOS",
            Platform::Linux => "Linux",
            Platform::Windows => "Windows",
            Platform::Unknown => "unknown",
        }
    }

    /// Whether this platform has a kernel-level sandbox mechanism available.
    pub fn has_kernel_sandbox(&self) -> bool {
        match self {
            Platform::MacOS => {
                cfg!(target_os = "macos") && std::path::Path::new("/usr/bin/sandbox-exec").exists()
            }
            Platform::Linux => cfg!(target_os = "linux") && Self::linux_cgroup_v2_available(),
            Platform::Windows => cfg!(target_os = "windows"),
            Platform::Unknown => false,
        }
    }

    /// Check if cgroups v2 is mounted (Linux only).
    fn linux_cgroup_v2_available() -> bool {
        std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
    }
}
