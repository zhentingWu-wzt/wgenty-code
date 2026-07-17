//! macOS Seatbelt sandbox backend.
//!
//! Uses `/usr/bin/sandbox-exec` with a dynamically generated `.sb` profile.
//! The profile is written to a temp file and cleaned up after use.
//! Seatbelt provides filesystem, network, and syscall-level restrictions.

use std::path::Path;

use super::super::profile::NetworkPolicy;
use crate::sandbox::{
    CleanupType, SandboxBackend, SandboxCleanup, SandboxError, SandboxProfile, SandboxedChild,
};

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

        // Common development tool directories under the user's home.
        // Cargo (registry cache, bin, git checkouts), rustup (toolchains,
        // std library), and other language runtimes need to be readable
        // and executable for build commands (cargo, npm, node, etc.) to
        // work inside the sandbox. Still needed for process-exec even when
        // full_disk_read grants unrestricted file-read*.
        let home_tool_paths: Vec<String> = dirs::home_dir()
            .map(|home| {
                [
                    ".cargo", ".rustup", ".nvm", ".bun", ".volta", ".deno", ".local",
                ]
                .iter()
                .map(|d| home.join(d).to_string_lossy().into_owned())
                .collect()
            })
            .unwrap_or_default();

        // Codex-aligned: workspace-write / read-only use unrestricted
        // `(allow file-read*)` (Root read). Path-scoped reads only for
        // Paranoid / explicit full_disk_read=false profiles.
        if profile.full_disk_read {
            sb.push_str(";; Codex-style full-disk read (workspace-write / read-only)\n");
            sb.push_str("(allow file-read*)\n\n");
        } else {
            sb.push_str(";; Allow reading from approved paths and system libraries\n");
            sb.push_str("(allow file-read*\n");
            for path in &profile.readable_paths {
                sb.push_str(&format!("    (subpath \"{}\")\n", path.display()));
            }
            // Root directory itself only (not children). On modern macOS, dyld/sh
            // resolve firmlinks via the root vnode; without this, even
            // `printf hello` is SIGABRT'd (-6) despite broad /bin+/usr reads.
            sb.push_str("    (literal \"/\")\n");
            sb.push_str("    (subpath \"/usr/lib\")\n");
            sb.push_str("    (subpath \"/System/Library\")\n");
            sb.push_str("    (subpath \"/Library\")\n");
            sb.push_str("    (subpath \"/private/var/db/dyld\")\n");
            sb.push_str("    (subpath \"/dev/dtracehelper\")\n");
            // process-exec does NOT imply file-read on macOS Seatbelt. Without these
            // paths, even `sh -c true` is killed because the shell binary cannot be
            // mapped/read before exec.
            sb.push_str("    (subpath \"/bin\")\n");
            sb.push_str("    (subpath \"/usr/bin\")\n");
            sb.push_str("    (subpath \"/usr/sbin\")\n");
            sb.push_str("    (subpath \"/sbin\")\n");
            sb.push_str("    (subpath \"/usr/local\")\n");
            sb.push_str("    (subpath \"/opt/homebrew\")\n");
            sb.push_str("    (subpath \"/Library/Developer/CommandLineTools\")\n");
            sb.push_str("    (subpath \"/private/var/folders\")\n");
            // macOS shell selector used by /bin/sh on some builds.
            sb.push_str("    (subpath \"/private/var/select\")\n");
            // Development tool paths: cargo registry, rustup toolchains, etc.
            for path in &home_tool_paths {
                sb.push_str(&format!("    (subpath \"{}\")\n", path));
            }
            sb.push_str(")\n\n");
        }

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
            sb.push('\n');
        }
        sb.push_str(")\n\n");

        // Codex allows unrestricted process-exec; we keep a practical allowlist
        // so arbitrary binaries outside common tool locations still need workspace
        // placement. full_disk_read only affects file-read*, not exec.
        sb.push_str(";; Allow process execution\n");
        sb.push_str("(allow process-exec\n");
        sb.push_str("    (subpath \"/usr/bin\")\n");
        sb.push_str("    (subpath \"/bin\")\n");
        sb.push_str("    (subpath \"/usr/sbin\")\n");
        sb.push_str("    (subpath \"/sbin\")\n");
        sb.push_str("    (subpath \"/usr/local/bin\")\n");
        sb.push_str("    (subpath \"/opt/homebrew/bin\")\n");
        // Xcode Command Line Tools: cc, ld, etc. for native compilation/linking
        sb.push_str("    (subpath \"/Library/Developer/CommandLineTools\")\n");
        for path in &profile.writable_paths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path.display()));
        }
        // Development tool binaries: cargo, rustc, node, npm, etc.
        for path in &home_tool_paths {
            sb.push_str(&format!("    (subpath \"{}\")\n", path));
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
        let profile_path = tmp_dir.join(format!("claude-sandbox-{}.sb", std::process::id()));
        std::fs::write(&profile_path, sb_profile.as_bytes()).map_err(|e| {
            SandboxError::ProfileBuild {
                reason: format!("failed to write seatbelt profile: {}", e),
            }
        })?;

        // Absolute paths only: after env_clear, bare `sh` can ENOENT if PATH is
        // missing/wrong. Codex also pins sandbox-exec to /usr/bin.
        let mut cmd = tokio::process::Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-f")
            .arg(&profile_path)
            .arg("/bin/sh")
            .arg("-c")
            .arg(command);

        // Keep child output off the parent TUI console.
        super::configure_captured_stdio(&mut cmd);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_profile_uses_full_disk_read() {
        let profile = SandboxProfile::default_for_workspace(Path::new("/tmp/test-ws"));
        assert!(profile.full_disk_read);
        let sb = MacOSBackend::generate_profile(&profile);
        // Codex workspace-write: unrestricted file-read*, not path allowlist.
        assert!(
            sb.contains("(allow file-read*)\n"),
            "full_disk_read must emit unrestricted (allow file-read*)"
        );
        // Writes stay workspace-scoped.
        assert!(sb.contains("(subpath \"/tmp/test-ws\")"));
    }

    #[test]
    fn profile_includes_dev_tool_home_paths_for_exec() {
        let profile = SandboxProfile::default_for_workspace(Path::new("/tmp/test-ws"));
        let sb = MacOSBackend::generate_profile(&profile);

        // process-exec still needs home tool paths even with full_disk_read.
        let home = dirs::home_dir().expect("home dir");
        for sub in [".cargo", ".rustup"] {
            let p = home.join(sub).to_string_lossy().into_owned();
            assert!(
                sb.contains(&p),
                "seatbelt profile should include {} for process-exec",
                p
            );
        }
    }

    #[test]
    fn profile_includes_xcode_clt_for_exec() {
        let profile = SandboxProfile::default_for_workspace(Path::new("/tmp/test-ws"));
        let sb = MacOSBackend::generate_profile(&profile);
        assert!(
            sb.contains("/Library/Developer/CommandLineTools"),
            "seatbelt profile should allow exec from Xcode CLT for native linking"
        );
    }

    #[test]
    fn path_scoped_read_includes_system_binaries() {
        // Paranoid / full_disk_read=false still needs explicit file-read for exec.
        let mut profile = SandboxProfile::default_for_workspace(Path::new("/tmp/test-ws"));
        profile.full_disk_read = false;
        let sb = MacOSBackend::generate_profile(&profile);
        for path in ["/bin", "/usr/bin", "/usr/sbin", "/sbin", "/usr/local", "/opt/homebrew"] {
            assert!(
                sb.contains(&format!("(subpath \"{path}\")")),
                "path-scoped seatbelt must file-read* {path} so process-exec can map binaries"
            );
        }
        assert!(
            sb.contains("(literal \"/\")"),
            "path-scoped seatbelt must file-read* root literal for firmlinks"
        );
        assert!(
            !sb.contains("(subpath \"/\")"),
            "path-scoped seatbelt must not grant subpath \"/\" (whole FS)"
        );
    }

    #[test]
    fn process_exec_allows_bin_for_absolute_sh() {
        let profile = SandboxProfile::default_for_workspace(Path::new("/tmp/test-ws"));
        let sb = MacOSBackend::generate_profile(&profile);
        assert!(
            sb.contains("(subpath \"/bin\")"),
            "absolute /bin/sh spawn requires process-exec /bin"
        );
    }
}
