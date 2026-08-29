//! HTTP request handlers for the daemon API.
//!
//! Organized by resource (`system`, `chat`, `tools`, `permissions`, `tasks`,
//! `mcp`, `sessions`, `checkpoints`, `agents`, `memory`); each submodule
//! inherits this module's imports via `use super::*` and its public handlers
//! are re-exported here so `handlers::<name>` paths stay unchanged.

use crate::api::{ApiClient, ToolDefinition};
use crate::daemon::models::*;
use crate::daemon::state::DaemonState;
use crate::permissions::{PolicyDecision, ToolPermissionPolicy};
use crate::tasks::management::{TaskPriority, TaskStatus};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        Json, Sse,
    },
};
use futures::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::Stream;
use tracing::error;

/// Header name carrying the trusted-UI viewer bearer token.
const VIEWER_TOKEN_HEADER: &str = "x-wgenty-viewer-token";

// Shared helpers used across handler submodules.
fn debug_log(msg: &str) {
    tracing::debug!("{}", msg);
}

/// Format an error with its full cause chain.
///
/// reqwest's `Display` only prints the outer kind (e.g. "error decoding
/// response body") and silently drops the actual cause - timeout vs.
/// connection reset vs. HTTP/2 stream error - which lives in
/// `std::error::Error::source()`. This walks the chain so the real reason a
/// stream was interrupted is visible in logs and in the error payload sent to
/// the client.
fn format_error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut current = e.source();
    while let Some(cause) = current {
        let cause_str = cause.to_string();
        if !cause_str.is_empty() {
            s.push_str(": ");
            s.push_str(&cause_str);
        }
        current = cause.source();
    }
    s
}

mod agents;
mod chat;
mod checkpoints;
mod mcp;
mod memory;
mod permissions;
mod sessions;
mod system;
mod tasks;
mod tools;

pub use agents::*;
pub use chat::*;
pub use checkpoints::*;
pub use mcp::*;
pub use memory::*;
pub use permissions::*;
pub use sessions::*;
pub use system::*;
pub use tasks::*;
pub use tools::*;

#[cfg(test)]
mod tests;
