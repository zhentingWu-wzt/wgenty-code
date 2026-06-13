//! CodeGraph — persistent code index and query engine.

pub mod types;
pub mod store;
pub mod parser;
pub mod indexer;
pub mod query;
pub mod tools;

use std::path::PathBuf;

/// Central engine holding indexer + query engine + store.
/// Shared across builtin tools and CLI via `Arc<CodegraphEngine>`.
pub struct CodegraphEngine {
    pub project_root: PathBuf,
}

impl CodegraphEngine {
    /// Create a new engine, optionally running an initial index.
    pub fn new(project_root: PathBuf, _auto_index: bool) -> anyhow::Result<Self> {
        Ok(Self { project_root })
    }
}
