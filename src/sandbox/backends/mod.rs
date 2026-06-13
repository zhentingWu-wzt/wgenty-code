//! Backend selection and platform-specific sandbox implementations.

pub mod linux;
pub mod macos;
pub mod none;
pub mod windows;

use crate::sandbox::SandboxBackend;

/// Auto-select the best available backend for the current platform.
pub fn create_backend() -> Box<dyn SandboxBackend> {
    #[cfg(target_os = "macos")]
    {
        if macos::MacOSBackend::is_available() {
            tracing::info!("Selected macOS Seatbelt sandbox backend");
            return Box::new(macos::MacOSBackend::new());
        }
    }

    #[cfg(target_os = "linux")]
    {
        if linux::LinuxBackend::is_available() {
            tracing::info!("Selected Linux seccomp+namespace sandbox backend");
            return Box::new(linux::LinuxBackend::new());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if windows::WindowsBackend::is_available() {
            tracing::info!("Selected Windows Job Object sandbox backend");
            return Box::new(windows::WindowsBackend::new());
        }
    }

    tracing::warn!("No kernel sandbox available; using no-op fallback (policy-only enforcement)");
    Box::new(none::NoneBackend)
}
