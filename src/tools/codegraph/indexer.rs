use crate::tools::codegraph::adapters::LanguageAdapter;
use crate::tools::codegraph::store::IndexStore;
use crate::tools::codegraph::types::*;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use walkdir::WalkDir;

pub struct IndexSummary {
    pub files_indexed: usize,
    pub symbols_extracted: usize,
    pub warnings: u32,
    pub elapsed_secs: f64,
}

/// Supported file extensions across all languages.
const SUPPORTED_EXTENSIONS: &[&str] = &["rs", "java", "py"];

pub struct Indexer {
    store: Arc<IndexStore>,
    adapters: Vec<Box<dyn LanguageAdapter>>,
}

impl Indexer {
    pub fn new(store: Arc<IndexStore>, adapters: Vec<Box<dyn LanguageAdapter>>) -> Self {
        Self { store, adapters }
    }

    /// Find the adapter for a given file path (by extension).
    fn adapter_for_path(&self, path: &Path) -> Option<&Box<dyn LanguageAdapter>> {
        let ext = path.extension()?.to_str()?;
        self.adapters
            .iter()
            .find(|a| a.file_extensions().contains(&ext))
    }

    /// Check if a file extension is supported.
    fn is_supported_extension(ext: Option<&std::ffi::OsStr>) -> bool {
        ext.and_then(|e| e.to_str())
            .map(|e| SUPPORTED_EXTENSIONS.contains(&e))
            .unwrap_or(false)
    }

    // ── Full indexing ──

    pub fn index_full(&self, project_root: &Path) -> anyhow::Result<IndexSummary> {
        let start = Instant::now();
        let files: Vec<_> = WalkDir::new(project_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| Self::is_supported_extension(e.path().extension()))
            .filter(|e| !e.path().to_string_lossy().contains("/target/"))
            .map(|e| e.path().to_path_buf())
            .collect();

        let mut file_data = Vec::with_capacity(files.len());
        let mut warnings = 0u32;

        for file_path in &files {
            match self.index_file(file_path, project_root) {
                Ok(result) => file_data.push(result),
                Err(e) => {
                    eprintln!(
                        "[codegraph] Warning: failed to index {}: {}",
                        file_path.display(),
                        e
                    );
                    warnings += 1;
                }
            }
        }

        self.store.begin_transaction()?;
        for data in &file_data {
            let file_id = self
                .store
                .upsert_file(&data.relative_path, &data.hash)?;
            self.store.delete_symbols_for_file(file_id)?;

            let mut symbol_id_map: Vec<(usize, i64)> = Vec::new();
            for (i, sym) in data.symbols.iter().enumerate() {
                let sym_id = self.store.insert_symbol(sym, file_id)?;
                symbol_id_map.push((i, sym_id));
            }
            for reference in &data.references {
                let mut r = reference.clone();
                let mapped = symbol_id_map.get(reference.symbol_id as usize).copied();
                match mapped {
                    Some((_idx, real_id)) => {
                        r.symbol_id = real_id;
                        self.store.insert_reference(&r, file_id)?;
                    }
                    None => {
                        // Reference points to an external symbol, skip
                    }
                }
            }
            for rel in &data.relationships {
                if rel.source_id < 0 || rel.target_id < 0 {
                    continue;
                }
                let mut r = rel.clone();
                let src_mapped = symbol_id_map.get(rel.source_id as usize).copied();
                let tgt_mapped = symbol_id_map.get(rel.target_id as usize).copied();
                if let (Some((_, src_real)), Some((_, tgt_real))) = (src_mapped, tgt_mapped) {
                    r.source_id = src_real;
                    r.target_id = tgt_real;
                    self.store.insert_relationship(&r)?;
                }
            }
        }
        self.store.commit()?;

        let symbol_count: usize = file_data.iter().map(|d| d.symbols.len()).sum();
        Ok(IndexSummary {
            files_indexed: files.len(),
            symbols_extracted: symbol_count,
            warnings,
            elapsed_secs: start.elapsed().as_secs_f64(),
        })
    }

    pub fn index_incremental(&self, project_root: &Path) -> anyhow::Result<IndexSummary> {
        let start = Instant::now();
        let tracked = self.store.get_all_files()?;
        let tracked_map: std::collections::HashMap<String, String> =
            tracked.into_iter().collect();

        let current: Vec<_> = WalkDir::new(project_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| Self::is_supported_extension(e.path().extension()))
            .filter(|e| !e.path().to_string_lossy().contains("/target/"))
            .map(|e| {
                let full = e.path().to_path_buf();
                let relative = full
                    .strip_prefix(project_root)
                    .unwrap_or(&full)
                    .to_string_lossy()
                    .to_string();
                (relative, full)
            })
            .collect();

        let mut modified = 0usize;
        let mut warnings = 0u32;
        let mut current_set: HashSet<String> = HashSet::new();
        let mut total_symbols = 0usize;

        self.store.begin_transaction()?;
        for (relative, full_path) in &current {
            current_set.insert(relative.clone());
            let hash = IndexStore::file_hash(full_path)?;
            match tracked_map.get(relative) {
                None => {
                    modified += 1;
                    match self.index_and_store_file(full_path, project_root, &hash) {
                        Ok(count) => total_symbols += count,
                        Err(e) => {
                            eprintln!("[codegraph] Warning: {}", e);
                            warnings += 1;
                        }
                    }
                }
                Some(old) if *old != hash => {
                    modified += 1;
                    self.store.delete_file(relative)?;
                    match self.index_and_store_file(full_path, project_root, &hash) {
                        Ok(count) => total_symbols += count,
                        Err(e) => {
                            eprintln!("[codegraph] Warning: {}", e);
                            warnings += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        for path in tracked_map.keys() {
            if !current_set.contains(path) {
                self.store.delete_file(path)?;
            }
        }
        self.store.commit()?;

        Ok(IndexSummary {
            files_indexed: modified,
            symbols_extracted: total_symbols,
            warnings,
            elapsed_secs: start.elapsed().as_secs_f64(),
        })
    }

    fn index_file(
        &self,
        file_path: &Path,
        project_root: &Path,
    ) -> anyhow::Result<FileIndexResult> {
        let relative = file_path
            .strip_prefix(project_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        let hash = IndexStore::file_hash(file_path)?;
        let source = std::fs::read_to_string(file_path)?;
        let (symbols, references, relationships) =
            self.extract_from_source(&source, &relative)?;
        Ok(FileIndexResult {
            relative_path: relative,
            hash,
            symbols,
            references,
            relationships,
        })
    }

    fn index_and_store_file(
        &self,
        file_path: &Path,
        project_root: &Path,
        hash: &str,
    ) -> anyhow::Result<usize> {
        let relative = file_path
            .strip_prefix(project_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        let source = std::fs::read_to_string(file_path)?;
        let (symbols, references, relationships) =
            self.extract_from_source(&source, &relative)?;
        let count = symbols.len();

        let file_id = self.store.upsert_file(&relative, hash)?;
        let mut symbol_id_map: Vec<(usize, i64)> = Vec::new();
        for (i, sym) in symbols.iter().enumerate() {
            let sym_id = self.store.insert_symbol(sym, file_id)?;
            symbol_id_map.push((i, sym_id));
        }
        for reference in &references {
            let mut r = reference.clone();
            if let Some(&(_idx, real_id)) = symbol_id_map.get(reference.symbol_id as usize) {
                r.symbol_id = real_id;
                self.store.insert_reference(&r, file_id)?;
            }
        }
        for rel in &relationships {
            if rel.source_id < 0 || rel.target_id < 0 {
                continue;
            }
            let mut r = rel.clone();
            let src = symbol_id_map.get(rel.source_id as usize).copied();
            let tgt = symbol_id_map.get(rel.target_id as usize).copied();
            if let (Some((_, s)), Some((_, t))) = (src, tgt) {
                r.source_id = s;
                r.target_id = t;
                self.store.insert_relationship(&r)?;
            }
        }
        Ok(count)
    }

    // ── Multi-language AST Extraction ──

    fn extract_from_source(
        &self,
        source: &str,
        relative_path: &str,
    ) -> anyhow::Result<(Vec<Symbol>, Vec<Reference>, Vec<Relationship>)> {
        let adapter = self
            .adapter_for_path(Path::new(relative_path))
            .ok_or_else(|| {
                anyhow::anyhow!("No language adapter found for: {}", relative_path)
            })?;

        let tree = adapter.parse(source)?;
        let symbols = adapter.extract_symbols(&tree, source, relative_path);
        let references = adapter.extract_references(&tree, source, &symbols);
        let relationships = adapter.extract_relationships(&tree, source, &symbols);

        Ok((symbols, references, relationships))
    }
}

struct FileIndexResult {
    relative_path: String,
    hash: String,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::adapters::rust::RustAdapter;
    use crate::tools::codegraph::store::IndexStore;
    use tempfile::TempDir;

    fn setup() -> (Arc<IndexStore>, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexStore::open(dir.path()).unwrap());
        (store, dir)
    }

    fn rust_indexer(store: Arc<IndexStore>) -> Indexer {
        let adapters: Vec<Box<dyn LanguageAdapter>> =
            vec![Box::new(RustAdapter::new())];
        Indexer::new(store, adapters)
    }

    #[test]
    fn test_extract_function_symbol() {
        let (store, _dir) = setup();
        let indexer = rust_indexer(store);
        let source = "pub fn hello(x: i32) -> bool { true }";
        let (symbols, _, _) = indexer.extract_from_source(source, "test.rs").unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].language, "rust");
    }

    #[test]
    fn test_extract_struct_and_enum() {
        let (store, _dir) = setup();
        let indexer = rust_indexer(store);
        let source = "pub struct Point { x: i32, y: i32 }\n\npub enum Color { Red, Green }";
        let (symbols, _, _) = indexer.extract_from_source(source, "test.rs").unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Point");
        assert_eq!(symbols[1].name, "Color");
    }

    #[test]
    fn test_extract_call_references() {
        let (store, _dir) = setup();
        let indexer = rust_indexer(store);
        let source = "fn foo() {}\n\nfn bar() {\n    foo();\n}";
        let (symbols, references, _relationships) =
            indexer.extract_from_source(source, "test.rs").unwrap();
        assert!(symbols.iter().any(|s| s.name == "foo"));
        assert!(symbols.iter().any(|s| s.name == "bar"));
        assert!(
            references
                .iter()
                .any(|r| r.context.as_deref() == Some("foo")),
            "Expected reference to foo"
        );
    }

    #[test]
    fn test_unknown_extension_errors() {
        let (store, _dir) = setup();
        let indexer = rust_indexer(store);
        let result = indexer.extract_from_source("let x = 1;", "test.js");
        assert!(result.is_err());
    }

    #[test]
    fn test_supported_extensions() {
        assert!(Indexer::is_supported_extension(Some(std::ffi::OsStr::new("rs"))));
        assert!(Indexer::is_supported_extension(Some(std::ffi::OsStr::new("java"))));
        assert!(Indexer::is_supported_extension(Some(std::ffi::OsStr::new("py"))));
        assert!(!Indexer::is_supported_extension(Some(std::ffi::OsStr::new("js"))));
    }

    #[test]
    fn test_adapter_for_path() {
        let (store, _dir) = setup();
        let indexer = rust_indexer(store);
        assert!(indexer.adapter_for_path(Path::new("test.rs")).is_some());
        assert!(indexer.adapter_for_path(Path::new("test.java")).is_none()); // only Rust adapter registered
        assert!(indexer.adapter_for_path(Path::new("test.js")).is_none());
    }
}
