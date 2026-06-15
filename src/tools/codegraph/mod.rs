//! CodeGraph — persistent code index and query engine.

pub mod adapters;
pub mod audit;
pub mod call_path;
pub mod fuzzy;
pub mod indexer;
pub mod parser;
pub mod query;
pub mod store;
pub mod tools;
pub mod types;

use std::path::PathBuf;
use std::sync::Arc;

/// Central engine holding indexer + query engine + store.
/// Shared across builtin tools and CLI via `Arc<CodegraphEngine>`.
pub struct CodegraphEngine {
    pub project_root: PathBuf,
    pub store: Arc<store::IndexStore>,
    pub indexer: Arc<indexer::Indexer>,
    pub query_engine: Arc<query::QueryEngine>,
}

impl CodegraphEngine {
    /// Create a new engine, optionally running an initial index.
    pub fn new(project_root: PathBuf, auto_index: bool) -> anyhow::Result<Self> {
        let store = Arc::new(store::IndexStore::open(&project_root)?);

        let adapters: Vec<Box<dyn adapters::LanguageAdapter>> = vec![
            Box::new(adapters::rust::RustAdapter::new()),
            Box::new(adapters::java::JavaAdapter::new()),
            Box::new(adapters::python::PythonAdapter::new()),
        ];
        let indexer = Arc::new(indexer::Indexer::new(store.clone(), adapters));
        let query_engine = Arc::new(query::QueryEngine::new(store.clone()));

        if auto_index && !store.has_index()? {
            eprintln!("[codegraph] No index found, running full indexing...");
            let summary = indexer.index_full(&project_root)?;
            eprintln!(
                "[codegraph] Indexed {} files, {} symbols in {:.1}s",
                summary.files_indexed, summary.symbols_extracted, summary.elapsed_secs
            );
        }

        Ok(Self {
            project_root,
            store,
            indexer,
            query_engine,
        })
    }

    /// Ensure the index is up-to-date (incremental refresh).
    pub fn refresh(&self) -> anyhow::Result<()> {
        self.indexer.index_incremental(&self.project_root)?;
        Ok(())
    }

    /// Check whether the index DB file exists on disk.
    pub fn has_index(&self) -> bool {
        self.store.has_index().unwrap_or(false)
    }
}
