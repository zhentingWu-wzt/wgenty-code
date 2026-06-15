//! Rust language adapter — parses `.rs` files using tree-sitter-rust.
//!
//! Extracts symbols (functions, structs, enums, traits, impls, type aliases,
//! consts, statics, mods, macros), references (calls, type refs, imports),
//! and relationships (calls, implements, contains, imports).

use crate::tools::codegraph::adapters::LanguageAdapter;
use crate::tools::codegraph::types::{
    Confidence, RefKind, RelKind, Reference, Relationship, Symbol, SymbolKind, Visibility,
};
use tree_sitter::{Language, Node, Tree};

/// Adapter for parsing and extracting Rust source code.
pub struct RustAdapter;

impl RustAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RustAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for RustAdapter {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn tree_sitter_language(&self) -> Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<Symbol> {
        let mut extractor = RustExtractor::new(source, file_path);
        extractor.collect_definitions(tree.root_node());
        extractor.symbols
    }

    fn extract_references(
        &self,
        tree: &Tree,
        source: &str,
        symbols: &[Symbol],
    ) -> Vec<Reference> {
        let mut extractor = RustExtractor::new(source, "");
        extractor.target_symbols = symbols.to_vec();
        extractor.collect_references_pass(tree.root_node());
        extractor.references
    }

    fn extract_relationships(
        &self,
        tree: &Tree,
        source: &str,
        symbols: &[Symbol],
    ) -> Vec<Relationship> {
        let mut extractor = RustExtractor::new(source, "");
        extractor.target_symbols = symbols.to_vec();
        extractor.collect_relationships_pass(tree.root_node());
        extractor.relationships
    }
}

// ── Internal extractor (shared tree-walking logic) ──

struct RustExtractor<'a> {
    source: &'a str,
    file_path: &'a str,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
    target_symbols: Vec<Symbol>,
}

impl<'a> RustExtractor<'a> {
    fn new(source: &'a str, file_path: &'a str) -> Self {
        Self {
            source,
            file_path,
            symbols: Vec::new(),
            references: Vec::new(),
            relationships: Vec::new(),
            target_symbols: Vec::new(),
        }
    }

    fn source_bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }

    fn utf8_text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.source_bytes()).unwrap_or("")
    }

    /// Look up a symbol by name in the target list, return its local ID.
    fn resolve_symbol_id(&self, name: &str) -> Option<i64> {
        self.target_symbols
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.id)
    }

    // ── Symbol extraction ──

    fn collect_definitions(&mut self, node: Node<'_>) {
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
                        file_path: self.file_path.to_string(),
                        line: pos.row + 1,
                        col: pos.column + 1,
                        signature: self.extract_signature(node, kind_str),
                        visibility,
                        parent_module: None,
                        language: "rust".to_string(),
                    };
                    self.symbols.push(sym);
                }
            }
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_definitions(child);
            }
        }
    }

    // ── Reference extraction ──

    fn collect_references_pass(&mut self, node: Node<'_>) {
        if node.kind() == "call_expression" {
            if let Some(func_node) = node.child(0) {
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
                    let symbol_id = self
                        .resolve_symbol_id(&called_name)
                        .unwrap_or(-1);

                    self.references.push(Reference {
                        id: None,
                        symbol_id,
                        file_path: String::new(),
                        line: pos.row + 1,
                        col: pos.column + 1,
                        ref_kind,
                        context: Some(called_name),
                    });
                }
            }
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_references_pass(child);
            }
        }
    }

    // ── Relationship extraction ──

    fn collect_relationships_pass(&mut self, node: Node<'_>) {
        let kind_str = node.kind();

        // Calls: call_expression → Calls relationship
        if kind_str == "call_expression" {
            if let Some(func_node) = node.child(0) {
                let called_name = self.utf8_text(func_node).to_string();
                if !called_name.is_empty()
                    && called_name != "self"
                    && called_name != "Self"
                {
                    let pos = func_node.start_position();
                    let target_id = self.resolve_symbol_id(&called_name).unwrap_or(-1);
                    // Find the enclosing function (source of the call)
                    let source_id = self.find_enclosing_function_id(node);

                    self.relationships.push(Relationship {
                        id: None,
                        source_id,
                        target_id,
                        rel_kind: RelKind::Calls,
                        file_path: String::new(),
                        line: pos.row + 1,
                        confidence: Confidence::Low,
                    });
                }
            }
        }

        // Implements: impl_item → Implements relationship
        if kind_str == "impl_item" {
            if let Some(type_node) = self.get_impl_type_node(node) {
                let type_name = self.utf8_text(type_node).to_string();
                if let Some(target_id) = self.resolve_symbol_id(&type_name) {
                    let pos = node.start_position();
                    let source_id = self.symbols.len() as i64; // fallback
                    self.relationships.push(Relationship {
                        id: None,
                        source_id,
                        target_id,
                        rel_kind: RelKind::Implements,
                        file_path: String::new(),
                        line: pos.row + 1,
                        confidence: Confidence::High,
                    });
                }
            }
        }

        // Imports: use_declaration → Imports relationship
        if kind_str == "use_declaration" {
            if let Some(name) = self.extract_use_target_name(node) {
                if let Some(target_id) = self.resolve_symbol_id(&name) {
                    // no meaningful source — mark as -1
                    let pos = node.start_position();
                    self.relationships.push(Relationship {
                        id: None,
                        source_id: -1,
                        target_id,
                        rel_kind: RelKind::Imports,
                        file_path: String::new(),
                        line: pos.row + 1,
                        confidence: Confidence::Medium,
                    });
                }
            }
        }

        // Contains: mod or block that contains other symbols
        if kind_str == "mod_item" {
            let pos = node.start_position();
            if let Some(name_node) = self.get_name_node(node, kind_str) {
                let name = self.utf8_text(name_node).to_string();
                if let Some(source_id) = self.resolve_symbol_id(&name) {
                    self.relationships.push(Relationship {
                        id: None,
                        source_id,
                        target_id: -1,
                        rel_kind: RelKind::Contains,
                        file_path: String::new(),
                        line: pos.row + 1,
                        confidence: Confidence::Medium,
                    });
                }
            }
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_relationships_pass(child);
            }
        }
    }

    // ── Helpers ──

    fn get_name_node<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        match kind {
            "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
            | "macro_definition"
            | "mod_item" => {
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

    fn extract_visibility(&self, node: Node<'_>) -> Visibility {
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

    fn extract_signature(&self, node: Node<'_>, kind: &str) -> Option<String> {
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

    /// Find the enclosing function's local symbol ID by walking up the tree.
    fn find_enclosing_function_id(&self, node: Node<'_>) -> i64 {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "function_item" {
                if let Some(name_node) = self.get_name_node(parent, parent.kind()) {
                    let name = self.utf8_text(name_node);
                    if let Some(id) = self.resolve_symbol_id(name) {
                        return id;
                    }
                }
            }
            // Also check impl_item
            if parent.kind() == "impl_item" {
                if let Some(type_node) = self.get_impl_type_node(parent) {
                    let name = self.utf8_text(type_node);
                    if let Some(id) = self.resolve_symbol_id(name) {
                        return id;
                    }
                }
            }
            current = parent;
        }
        -1
    }

    fn get_impl_type_node<'n>(&self, node: Node<'n>) -> Option<Node<'n>> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "type_identifier" {
                    return Some(child);
                }
            }
        }
        None
    }

    fn extract_use_target_name(&self, node: Node<'_>) -> Option<String> {
        // Walk children to find the last identifier/scope_identifier
        let mut last_name: Option<String> = None;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                let ck = child.kind();
                if ck == "identifier" {
                    last_name = Some(self.utf8_text(child).to_string());
                } else if ck == "scoped_identifier" {
                    // Find the last segment after ::
                    let text = self.utf8_text(child);
                    let parts: Vec<&str> = text.split("::").collect();
                    if let Some(last) = parts.last() {
                        last_name = Some(last.to_string());
                    }
                }
                // Recurse into scoped_use_list etc.
                if let Some(name) = self.extract_use_target_name(child) {
                    last_name = Some(name);
                }
            }
        }
        last_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::adapters::LanguageAdapter;
    use crate::tools::codegraph::types::{RelKind, RefKind, SymbolKind, Visibility};

    fn adapter() -> RustAdapter {
        RustAdapter::new()
    }

    #[test]
    fn test_language_name() {
        let a = adapter();
        assert_eq!(a.language(), "rust");
    }

    #[test]
    fn test_file_extensions() {
        let a = adapter();
        assert_eq!(a.file_extensions(), &["rs"]);
    }

    #[test]
    fn test_parse_valid_rust() {
        let a = adapter();
        let tree = a.parse("fn main() {}").unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_extract_function_symbol() {
        let a = adapter();
        let source = "pub fn hello(x: i32) -> bool { true }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Pub);
        assert_eq!(symbols[0].language, "rust");
    }

    #[test]
    fn test_extract_struct_and_enum() {
        let a = adapter();
        let source = "pub struct Point { x: i32, y: i32 }\npub enum Color { Red, Green }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.rs");
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Point");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
        assert_eq!(symbols[1].name, "Color");
        assert_eq!(symbols[1].kind, SymbolKind::Enum);
    }

    #[test]
    fn test_extract_call_references() {
        let a = adapter();
        let source = "fn foo() {}\nfn bar() { foo(); }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.rs");
        let refs = a.extract_references(&tree, source, &symbols);
        assert!(
            refs.iter().any(|r| r.context.as_deref() == Some("foo")),
            "Expected a reference to foo, got: {:?}",
            refs.iter().map(|r| &r.context).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_calls_relationship() {
        let a = adapter();
        let source = "fn foo() {}\nfn bar() { foo(); }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.rs");
        let rels = a.extract_relationships(&tree, source, &symbols);
        assert!(
            rels.iter().any(|r| r.rel_kind == RelKind::Calls),
            "Expected a Calls relationship"
        );
    }

    #[test]
    fn test_extract_impl_relationship() {
        let a = adapter();
        let source = "struct Foo;\nimpl Foo { fn method(&self) {} }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.rs");
        let rels = a.extract_relationships(&tree, source, &symbols);
        assert!(
            rels.iter().any(|r| r.rel_kind == RelKind::Implements),
            "Expected an Implements relationship, got: {:?}",
            rels.iter().map(|r| r.rel_kind.as_str()).collect::<Vec<_>>()
        );
    }
}
