//! Structured error types for sandbox operations.

use std::fmt;

/// Errors that can occur during sandbox setup or execution.
#[derive(Debug)]
pub enum SandboxError {
    /// The selected backend is not available on this platform.
    BackendUnavailable {
        platform: String,
        reason: String,
    },

    /// Failed to build a sandbox profile (e.g. invalid path).
    ProfileBuild {
        reason: String,
    },

    /// Failed to spawn the child process.
    Spawn {
        io_error: String,
    },

    /// The command exceeded its wall-clock time limit.
    Timeout {
        elapsed_secs: u64,
        limit_secs: u64,
    },

    /// The command exceeded its memory limit.
    MemoryExceeded {
        limit_bytes: u64,
        used_bytes: u64,
    },

    /// A sandbox policy violation was detected (e.g. blocked syscall).
    SandboxViolation {
        detail: String,
    },

    /// Cleanup of sandbox resources failed.
    Cleanup {
        reason: String,
    },
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendUnavailable { platform, reason } => {
                write!(f, "sandbox backend unavailable on {platform}: {reason}")
            }
            Self::ProfileBuild { reason } => {
                write!(f, "failed to build sandbox profile: {reason}")
            }
            Self::Spawn { io_error } => {
                write!(f, "failed to spawn sandboxed process: {io_error}")
            }
            Self::Timeout { elapsed_secs, limit_secs } => {
                write!(f, "command timed out after {elapsed_secs}s (limit: {limit_secs}s)")
            }
            Self::MemoryExceeded { limit_bytes, used_bytes } => {
                write!(f, "memory limit exceeded: used {used_bytes} bytes, limit {limit_bytes} bytes")
            }
            Self::SandboxViolation { detail } => {
                write!(f, "sandbox violation: {detail}")
            }
            Self::Cleanup { reason } => {
                write!(f, "sandbox cleanup failed: {reason}")
            }
        }
    }
}

impl std::error::Error for SandboxError {}
