use crate::tools::codegraph::parser::CodeParser;
use crate::tools::codegraph::store::IndexStore;
use crate::tools::codegraph::types::*;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

pub struct IndexSummary {
    pub files_indexed: usize,
    pub symbols_extracted: usize,
    pub warnings: u32,
    pub elapsed_secs: f64,
}

pub struct Indexer {
    store: Arc<IndexStore>,
    parser: Arc<Mutex<CodeParser>>,
}

impl Indexer {
    pub fn new(store: Arc<IndexStore>, parser: Arc<Mutex<CodeParser>>) -> Self {
        Self { store, parser }
    }

    // ── Full indexing ──

    pub fn index_full(&self, project_root: &Path) -> anyhow::Result<IndexSummary> {
        let start = Instant::now();
        let files: Vec<_> = WalkDir::new(project_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|x| x == "rs")
                    .unwrap_or(false)
            })
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
            let file_id = self.store.upsert_file(&data.relative_path, &data.hash)?;
            self.store.delete_symbols_for_file(file_id)?;

            let mut symbol_id_map: Vec<(usize, i64)> = Vec::new();
            for (i, sym) in data.symbols.iter().enumerate() {
                let sym_id = self.store.insert_symbol(sym, file_id)?;
                symbol_id_map.push((i, sym_id));
            }
            for reference in &data.references {
                let mut r = reference.clone();
                if let Some(&(_idx, real_id)) = symbol_id_map.get(reference.symbol_id as usize) {
                    r.symbol_id = real_id;
                }
                self.store.insert_reference(&r, file_id)?;
            }
            for rel in &data.relationships {
                let mut r = rel.clone();
                if let Some(&(_idx, real_id)) = symbol_id_map.get(rel.source_id as usize) {
                    r.source_id = real_id;
                }
                if let Some(&(_idx, real_id)) = symbol_id_map.get(rel.target_id as usize) {
                    r.target_id = real_id;
                }
                self.store.insert_relationship(&r)?;
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
        let tracked_map: std::collections::HashMap<String, String> = tracked.into_iter().collect();

        let current: Vec<_> = WalkDir::new(project_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|x| x == "rs")
                    .unwrap_or(false)
            })
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
        let (symbols, references, relationships) = self.extract_from_source(&source, &relative)?;
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
            }
            self.store.insert_reference(&r, file_id)?;
        }
        for rel in &relationships {
            let mut r = rel.clone();
            if let Some(&(_idx, real_id)) = symbol_id_map.get(rel.source_id as usize) {
                r.source_id = real_id;
            }
            if let Some(&(_idx, real_id)) = symbol_id_map.get(rel.target_id as usize) {
                r.target_id = real_id;
            }
            self.store.insert_relationship(&r)?;
        }
        Ok(count)
    }

    // ── AST Extraction (with source text) ──

    fn extract_from_source(
        &self,
        source: &str,
        _relative_path: &str,
    ) -> anyhow::Result<(Vec<Symbol>, Vec<Reference>, Vec<Relationship>)> {
        let mut parser = self.parser.lock().unwrap();
        let tree = parser.parse(source)?;
        let root = tree.root_node();

        let mut ctx = ExtractCtx {
            symbols: Vec::new(),
            references: Vec::new(),
            relationships: Vec::new(),
            source,
        };

        ctx.collect_symbols(root);

        Ok((ctx.symbols, ctx.references, ctx.relationships))
    }
}

struct FileIndexResult {
    relative_path: String,
    hash: String,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
}

// ── Extraction context (carries source text) ──

struct ExtractCtx<'a> {
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
    source: &'a str,
}

impl<'a> ExtractCtx<'a> {
    fn source_bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }

    fn utf8_text(&self, node: tree_sitter::Node<'_>) -> &str {
        node.utf8_text(self.source_bytes()).unwrap_or("")
    }

    fn collect_symbols(&mut self, node: tree_sitter::Node<'_>) {
        let kind_str = node.kind();
        if let Some(sym_kind) = SymbolKind::from_node_type(kind_str) {
            if let Some(name_node) = self.get_name_node(node, kind_str) {
                let name = self.utf8_text(name_node).to_string();
                if !name.is_empty() && name != "_" {
                    let pos = node.start_position();
                    let visibility = self.extract_visibility(node);

                    let sym = Symbol {
                        id: Some(self.symbols.len() as i64),
                        name,
                        kind: sym_kind,
                        file_path: String::new(),
                        line: pos.row + 1,
                        col: pos.column + 1,
                        signature: self.extract_signature(node, kind_str),
                        visibility,
                        parent_module: None,
                    };
                    let idx = self.symbols.len() as i64;
                    self.symbols.push(sym);

                    // Extract body references
                    if let Some(body) = self.get_body_node(node, kind_str) {
                        self.collect_references(body, idx);
                    }
                }
            }
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_symbols(child);
            }
        }
    }

    fn collect_references(&mut self, node: tree_sitter::Node<'_>, parent_idx: i64) {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "call_expression" => {
                        if let Some(func_node) = child.child(0) {
                            let called_name = self.utf8_text(func_node).to_string();
                            if !called_name.is_empty()
                                && called_name != "self"
                                && called_name != "Self"
                            {
                                let pos = func_node.start_position();
                                let ref_kind = if func_node.kind() == "field_expression" {
                                    RefKind::MethodCall
                                } else {
                                    RefKind::Call
                                };

                                self.references.push(Reference {
                                    id: None,
                                    symbol_id: parent_idx,
                                    file_path: String::new(),
                                    line: pos.row + 1,
                                    col: pos.column + 1,
                                    ref_kind,
                                    context: Some(called_name.clone()),
                                });

                                self.relationships.push(Relationship {
                                    id: None,
                                    source_id: parent_idx,
                                    target_id: -1,
                                    rel_kind: RelKind::Calls,
                                    file_path: String::new(),
                                    line: pos.row + 1,
                                    confidence: Confidence::Low,
                                });
                            }
                        }
                    }
                    _ => {}
                }
                self.collect_references(child, parent_idx);
            }
        }
    }

    fn get_name_node<'n>(&self, node: tree_sitter::Node<'n>, kind: &str) -> Option<tree_sitter::Node<'n>> {
        match kind {
            "function_item" | "struct_item" | "enum_item" | "trait_item" | "type_item"
            | "const_item" | "static_item" | "macro_definition" | "mod_item" => {
                // Find the first identifier child (name)
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        let ck = child.kind();
                        if ck == "identifier" || ck == "type_identifier" {
                            return Some(child);
                        }
                    }
                }
                None
            }
            "impl_item" => {
                // For impl blocks, name is the type being implemented
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "type_identifier" {
                            return Some(child);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn extract_visibility(&self, node: tree_sitter::Node<'_>) -> Visibility {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "visibility_modifier" {
                    let vis_text = self.utf8_text(child);
                    return Visibility::from_visibility_modifier(vis_text);
                }
            }
        }
        Visibility::Private
    }

    fn extract_signature(&self, node: tree_sitter::Node<'_>, kind: &str) -> Option<String> {
        if kind == "function_item" {
            let start = node.start_position();
            let end = node.end_position();
            Some(format!(
                "fn at {}:{}-{}:{}",
                start.row + 1,
                start.column + 1,
                end.row + 1,
                end.column + 1
            ))
        } else {
            let text = self.utf8_text(node);
            Some(text.lines().next()?.trim().to_string())
        }
    }

    fn get_body_node<'n>(&self, node: tree_sitter::Node<'n>, kind: &str) -> Option<tree_sitter::Node<'n>> {
        match kind {
            "function_item" | "impl_item" | "trait_item" | "mod_item" => {
                for i in (0..node.child_count()).rev() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "block" || child.kind() == "declaration_list" {
                            return Some(child);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::parser::CodeParser;
    use crate::tools::codegraph::store::IndexStore;
    use tempfile::TempDir;

    fn setup() -> (Arc<IndexStore>, Arc<Mutex<CodeParser>>, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexStore::open(dir.path()).unwrap());
        let parser = Arc::new(Mutex::new(CodeParser::new()));
        (store, parser, dir)
    }

    #[test]
    fn test_extract_function_symbol() {
        let (store, parser, _dir) = setup();
        let indexer = Indexer::new(store, parser);
        let source = "pub fn hello(x: i32) -> bool { true }";
        let (symbols, _, _) = indexer.extract_from_source(source, "test.rs").unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_struct_and_enum() {
        let (store, parser, _dir) = setup();
        let indexer = Indexer::new(store, parser);
        let source = "pub struct Point { x: i32, y: i32 }\n\npub enum Color { Red, Green }";
        let (symbols, _, _) = indexer.extract_from_source(source, "test.rs").unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Point");
        assert_eq!(symbols[1].name, "Color");
    }

    #[test]
    fn test_extract_call_references() {
        let (store, parser, _dir) = setup();
        let indexer = Indexer::new(store, parser);
        let source = "fn foo() {}\n\nfn bar() {\n    foo();\n}";
        let (symbols, references, _relationships) =
            indexer.extract_from_source(source, "test.rs").unwrap();
        assert!(symbols.iter().any(|s| s.name == "foo"));
        assert!(symbols.iter().any(|s| s.name == "bar"));
        assert!(
            references.iter().any(|r| r.context.as_deref() == Some("foo")),
            "Expected reference to foo"
        );
    }
}
