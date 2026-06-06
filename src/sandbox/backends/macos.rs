//! macOS Seatbelt sandbox backend.
//!
//! Uses `/usr/bin/sandbox-exec` with a dynamically generated `.sb` profile.
//! The profile is written to a temp file and cleaned up after use.
//! Seatbelt provides filesystem, network, and syscall-level restrictions.

use std::path::Path;

use crate::sandbox::{
    CleanupType, SandboxBackend, SandboxCleanup, SandboxError,
    SandboxProfile, SandboxedChild,
};
use super::super::profile::NetworkPolicy;

pub struct MacOSBackend;

impl Default for MacOSBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MacOSBackend {
    pub fn new() -> Self {
        Self
    }

    fn generate_profile(profile: &SandboxProfile) -> String {
        let mut sb = String::new();

        sb.push_str("(version 1)\n");
        sb.push_str("(deny default)\n\n");

        sb.push_str(";; Allow reading from approved paths and system libraries\n");
        sb.push_str("(allow file-read*\n");
        for path in &profile.readable_paths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path.display()));
        }
        sb.push_str("    (subpath \"/usr/lib\")\n");
        sb.push_str("    (subpath \"/System/Library\")\n");
        sb.push_str("    (subpath \"/Library\")\n");
        sb.push_str("    (subpath \"/private/var/db/dyld\")\n");
        sb.push_str("    (subpath \"/dev/dtracehelper\")\n");
        sb.push_str(")\n\n");

        sb.push_str(";; Allow writing to approved paths\n");
        sb.push_str("(allow file-write*\n");
        for path in &profile.writable_paths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path.display()));
        }
        sb.push_str(")\n\n");

        sb.push_str(";; Allow file deletion (unlink) within writable paths\n");
        sb.push_str("(allow file-write-unlink\n");
        for path in &profile.writable_paths {
            sb.push_str(&format!("    (subpath \"{}\")", path.display()));
            sb.push_str("\n");
        }
        sb.push_str(")\n\n");

        sb.push_str(";; Allow process execution\n");
        sb.push_str("(allow process-exec\n");
        sb.push_str("    (subpath \"/usr/bin\")\n");
        sb.push_str("    (subpath \"/bin\")\n");
        sb.push_str("    (subpath \"/usr/sbin\")\n");
        sb.push_str("    (subpath \"/sbin\")\n");
        sb.push_str("    (subpath \"/usr/local/bin\")\n");
        sb.push_str("    (subpath \"/opt/homebrew/bin\")\n");
        for path in &profile.writable_paths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path.display()));
        }
        sb.push_str(")\n\n");

        if profile.allow_subprocess {
            sb.push_str(";; Allow subprocess creation\n");
            sb.push_str("(allow process-fork)\n\n");
        }

        match profile.network {
            NetworkPolicy::None => {
                sb.push_str(";; Deny all network access\n");
                sb.push_str("(deny network*)\n\n");
            }
            NetworkPolicy::LoopbackOnly => {
                sb.push_str(";; Allow loopback only\n");
                sb.push_str("(allow network* (local ip))\n");
                sb.push_str("(deny network*)\n\n");
            }
            NetworkPolicy::Full => {
                sb.push_str(";; Allow full network access\n");
                sb.push_str("(allow network*)\n\n");
            }
        }

        sb.push_str(";; System necessities\n");
        // /dev — read+writes device files that shell commands actually need
        sb.push_str("(allow file-read* file-write*\n");
        sb.push_str("    (subpath \"/dev/null\")\n");
        sb.push_str("    (subpath \"/dev/zero\")\n");
        sb.push_str("    (subpath \"/dev/random\")\n");
        sb.push_str("    (subpath \"/dev/urandom\")\n");
        sb.push_str("    (subpath \"/dev/stdin\")\n");
        sb.push_str("    (subpath \"/dev/stdout\")\n");
        sb.push_str("    (subpath \"/dev/stderr\")\n");
        sb.push_str("    (subpath \"/dev/tty\")\n");
        sb.push_str("    (subpath \"/dev/dtracehelper\")\n");
        sb.push_str(")\n");
        // DNS resolution — needed for network-capable tools
        sb.push_str("(allow file-read*\n");
        sb.push_str("    (literal \"/private/etc/hosts\")\n");
        sb.push_str("    (literal \"/private/etc/resolv.conf\")\n");
        sb.push_str("    (literal \"/etc/hosts\")\n");
        sb.push_str("    (literal \"/etc/resolv.conf\")\n");
        sb.push_str(")\n");
        // macOS dynamic linker cache (already above in system libs, reaffirm here)
        sb.push_str("(allow file-read*\n");
        sb.push_str("    (subpath \"/private/var/db/dyld\")\n");
        sb.push_str(")\n");
        sb.push_str("(allow sysctl-read)\n");
        sb.push_str("(allow signal)\n");
        sb.push_str("(allow process-info*)\n");
        sb.push_str("(allow mach-lookup\n");
        sb.push_str("    (global-name \"com.apple.trustd.agent\")\n");
        sb.push_str("    (global-name \"com.apple.distributed_notifications@Uv3\")\n");
        sb.push_str("    (global-name \"com.apple.FontObjectsServer\")\n");
        sb.push_str(")\n");


        sb
    }

}

impl SandboxBackend for MacOSBackend {
    fn name(&self) -> &str {
        "seatbelt"
    }

    fn is_available() -> bool {
        cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists()
    }

    fn is_hardware_enforced(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<&str> {
        vec!["filesystem", "network", "syscall-filter"]
    }

    fn spawn(
        &self,
        profile: &SandboxProfile,
        command: &str,
        workdir: Option<&Path>,
    ) -> Result<SandboxedChild, SandboxError> {
        let sb_profile = Self::generate_profile(profile);

        let tmp_dir = std::env::temp_dir();
        let profile_path =
            tmp_dir.join(format!("claude-sandbox-{}.sb", std::process::id()));
        std::fs::write(&profile_path, sb_profile.as_bytes()).map_err(|e| {
            SandboxError::ProfileBuild {
                reason: format!("failed to write seatbelt profile: {}", e),
            }
        })?;

        let mut cmd = tokio::process::Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-f")
            .arg(&profile_path)
            .arg("sh")
            .arg("-c")
            .arg(command);

        if !profile.env_allowlist.is_empty() && !profile.env_allowlist.iter().any(|v| v == "*") {
            cmd.env_clear();
            for var in &profile.env_allowlist {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }

        if let Some(dir) = workdir.or(profile.workdir.as_deref()) {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| SandboxError::Spawn {
            io_error: format!("{}", e),
        })?;

        let profile_path_owned = profile_path.clone();
        let cleanup = SandboxCleanup {
            cleanup_type: CleanupType::DeleteTempFile,
            resource_handle: Some(profile_path_owned.to_string_lossy().to_string()),
        };

        Ok(SandboxedChild {
            child,
            backend_name: "seatbelt".into(),
            cleanup: Some(cleanup),
        })
    }
}
