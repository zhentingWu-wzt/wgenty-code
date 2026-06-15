//! Java language adapter — parses `.java` files using tree-sitter-java.
//!
//! Extracts symbols (classes, interfaces, enums, methods, constructors, fields),
//! references (method invocations, object creations), and relationships
//! (inherits, implements, calls, contains, parameter, returns, typeof).

use crate::tools::codegraph::adapters::LanguageAdapter;
use crate::tools::codegraph::types::{
    Confidence, RefKind, RelKind, Reference, Relationship, Symbol, SymbolKind, Visibility,
};
use tree_sitter::{Language, Node, Tree};

/// Adapter for parsing and extracting Java source code.
pub struct JavaAdapter;

impl JavaAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for JavaAdapter {
    fn language(&self) -> &'static str {
        "java"
    }

    fn file_extensions(&self) -> &[&str] {
        &["java"]
    }

    fn tree_sitter_language(&self) -> Language {
        tree_sitter_java::LANGUAGE.into()
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<Symbol> {
        let mut extractor = JavaExtractor::new(source, file_path);
        extractor.collect_definitions(tree.root_node());
        extractor.symbols
    }

    fn extract_references(
        &self,
        tree: &Tree,
        source: &str,
        symbols: &[Symbol],
    ) -> Vec<Reference> {
        let mut extractor = JavaExtractor::new(source, "");
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
        let mut extractor = JavaExtractor::new(source, "");
        extractor.target_symbols = symbols.to_vec();
        extractor.collect_relationships_pass(tree.root_node());
        extractor.relationships
    }
}

// ── Internal extractor ──

struct JavaExtractor<'a> {
    source: &'a str,
    file_path: &'a str,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
    target_symbols: Vec<Symbol>,
}

impl<'a> JavaExtractor<'a> {
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

    fn resolve_symbol_id(&self, name: &str) -> Option<i64> {
        self.target_symbols
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.id)
    }

    // ── Symbol extraction ──

    fn collect_definitions(&mut self, node: Node<'_>) {
        let kind_str = node.kind();

        match kind_str {
            "class_declaration" => {
                if let Some(name_node) = child_of_kind(node, "identifier") {
                    let name = self.utf8_text(name_node).to_string();
                    if !name.is_empty() {
                        let pos = node.start_position();
                        self.symbols.push(Symbol {
                            id: Some(self.symbols.len() as i64),
                            name,
                            kind: SymbolKind::Struct,
                            file_path: self.file_path.to_string(),
                            line: pos.row + 1,
                            col: pos.column + 1,
                            signature: self.class_signature(node),
                            visibility: self.extract_modifiers(node),
                            parent_module: None,
                            language: "java".to_string(),
                        });
                    }
                }
            }
            "interface_declaration" => {
                if let Some(name_node) = child_of_kind(node, "identifier") {
                    let name = self.utf8_text(name_node).to_string();
                    if !name.is_empty() {
                        let pos = node.start_position();
                        self.symbols.push(Symbol {
                            id: Some(self.symbols.len() as i64),
                            name,
                            kind: SymbolKind::Trait,
                            file_path: self.file_path.to_string(),
                            line: pos.row + 1,
                            col: pos.column + 1,
                            signature: None,
                            visibility: self.extract_modifiers(node),
                            parent_module: None,
                            language: "java".to_string(),
                        });
                    }
                }
            }
            "enum_declaration" => {
                if let Some(name_node) = child_of_kind(node, "identifier") {
                    let name = self.utf8_text(name_node).to_string();
                    if !name.is_empty() {
                        let pos = node.start_position();
                        self.symbols.push(Symbol {
                            id: Some(self.symbols.len() as i64),
                            name,
                            kind: SymbolKind::Enum,
                            file_path: self.file_path.to_string(),
                            line: pos.row + 1,
                            col: pos.column + 1,
                            signature: None,
                            visibility: self.extract_modifiers(node),
                            parent_module: None,
                            language: "java".to_string(),
                        });
                    }
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child_of_kind(node, "identifier") {
                    let name = self.utf8_text(name_node).to_string();
                    if !name.is_empty() {
                        let pos = node.start_position();
                        self.symbols.push(Symbol {
                            id: Some(self.symbols.len() as i64),
                            name,
                            kind: SymbolKind::Function,
                            file_path: self.file_path.to_string(),
                            line: pos.row + 1,
                            col: pos.column + 1,
                            signature: self.method_signature(node),
                            visibility: self.extract_modifiers(node),
                            parent_module: self.enclosing_class_name(node),
                            language: "java".to_string(),
                        });
                    }
                }
            }
            "constructor_declaration" => {
                if let Some(name_node) = child_of_kind(node, "identifier") {
                    let name = self.utf8_text(name_node).to_string();
                    if !name.is_empty() {
                        let pos = node.start_position();
                        let sig = format!("{}(...)", name);
                        self.symbols.push(Symbol {
                            id: Some(self.symbols.len() as i64),
                            name,
                            kind: SymbolKind::Function,
                            file_path: self.file_path.to_string(),
                            line: pos.row + 1,
                            col: pos.column + 1,
                            signature: Some(sig),
                            visibility: self.extract_modifiers(node),
                            parent_module: self.enclosing_class_name(node),
                            language: "java".to_string(),
                        });
                    }
                }
            }
            _ => {}
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
        let kind_str = node.kind();

        if kind_str == "method_invocation" {
            self.extract_method_call_ref(node);
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_references_pass(child);
            }
        }
    }

    fn extract_method_call_ref(&mut self, node: Node<'_>) {
        // method_invocation: (identifier | field_access) argument_list
        let mut called_name: Option<String> = None;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" => {
                        called_name = Some(self.utf8_text(child).to_string());
                    }
                    "field_access" => {
                        // obj.method() → method is the last child identifier
                        called_name = self.last_identifier(child);
                    }
                    _ => {}
                }
            }
        }

        if let Some(name) = called_name {
            if !name.is_empty() && name != "super" && name != "this" {
                let pos = node.start_position();
                let symbol_id = self.resolve_symbol_id(&name).unwrap_or(-1);

                self.references.push(Reference {
                    id: None,
                    symbol_id,
                    file_path: String::new(),
                    line: pos.row + 1,
                    col: pos.column + 1,
                    ref_kind: RefKind::Call,
                    context: Some(name),
                });
            }
        }
    }

    // ── Relationship extraction ──

    fn collect_relationships_pass(&mut self, node: Node<'_>) {
        let kind_str = node.kind();

        match kind_str {
            "class_declaration" => {
                // extends → Inherits
                if let Some(superclass) = self.find_child_recursive(node, "superclass") {
                    if let Some(type_id) = child_of_kind(superclass, "type_identifier") {
                        let name = self.utf8_text(type_id).to_string();
                        if let Some(target_id) = self.resolve_symbol_id(&name) {
                            // source = this class
                            if let Some(name_node) = child_of_kind(node, "identifier") {
                                let class_name = self.utf8_text(name_node);
                                if let Some(source_id) = self.resolve_symbol_id(class_name) {
                                    let pos = superclass.start_position();
                                    self.relationships.push(Relationship {
                                        id: None,
                                        source_id,
                                        target_id,
                                        rel_kind: RelKind::Inherits,
                                        file_path: String::new(),
                                        line: pos.row + 1,
                                        confidence: Confidence::High,
                                    });
                                }
                            }
                        }
                    }
                }

                // implements → Implements
                if let Some(interfaces) = self.find_child_recursive(node, "super_interfaces") {
                    for i in 0..interfaces.child_count() {
                        if let Some(child) = interfaces.child(i) {
                            if child.kind() == "type_list" {
                                self.extract_implements_rels(child, node);
                            }
                        }
                    }
                }
            }
            "method_invocation" => {
                // Calls relationship
                if let Some(called_name) = self.method_call_name(node) {
                    let target_id = self.resolve_symbol_id(&called_name).unwrap_or(-1);
                    if target_id >= 0 {
                        let source_id = self.find_enclosing_method_id(node);
                        let pos = node.start_position();
                        self.relationships.push(Relationship {
                            id: None,
                            source_id,
                            target_id,
                            rel_kind: RelKind::Calls,
                            file_path: String::new(),
                            line: pos.row + 1,
                            confidence: Confidence::Medium,
                        });
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                // Parameter relationships
                self.extract_parameter_rels(node);

                // Returns relationship (for methods with return type)
                if kind_str == "method_declaration" {
                    self.extract_return_type_rel(node);
                }
            }
            _ => {}
        }

        // Recurse
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_relationships_pass(child);
            }
        }
    }

    fn extract_implements_rels(&mut self, type_list: Node<'_>, class_node: Node<'_>) {
        let class_name = child_of_kind(class_node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let source_id = self.resolve_symbol_id(&class_name).unwrap_or(-1);

        for i in 0..type_list.child_count() {
            if let Some(child) = type_list.child(i) {
                if child.kind() == "type_identifier" {
                    let iface_name = self.utf8_text(child).to_string();
                    if let Some(target_id) = self.resolve_symbol_id(&iface_name) {
                        let pos = child.start_position();
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
        }
    }

    fn extract_parameter_rels(&mut self, method_node: Node<'_>) {
        // Find formal_parameters → formal_parameter → type_identifier
        if let Some(params) = self.find_child_recursive(method_node, "formal_parameters") {
            for i in 0..params.child_count() {
                if let Some(param) = params.child(i) {
                    if param.kind() == "formal_parameter" {
                        if let Some(type_id) = child_of_kind(param, "type_identifier") {
                            let type_name = self.utf8_text(type_id).to_string();
                            if let Some(target_id) = self.resolve_symbol_id(&type_name) {
                                let method_name = child_of_kind(method_node, "identifier")
                                    .map(|n| self.utf8_text(n).to_string())
                                    .unwrap_or_default();
                                let source_id =
                                    self.resolve_symbol_id(&method_name).unwrap_or(-1);
                                let pos = type_id.start_position();
                                self.relationships.push(Relationship {
                                    id: None,
                                    source_id,
                                    target_id,
                                    rel_kind: RelKind::Parameter,
                                    file_path: String::new(),
                                    line: pos.row + 1,
                                    confidence: Confidence::High,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn extract_return_type_rel(&mut self, method_node: Node<'_>) {
        // Look for return type: e.g., "int", "String", "void", or custom types
        // In tree-sitter-java, the return type is a child of method_declaration
        // (before the identifier). Find type_identifier children.
        let method_name = child_of_kind(method_node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let source_id = self.resolve_symbol_id(&method_name).unwrap_or(-1);

        // Look for generic_type or type_identifier in the method
        for i in 0..method_node.child_count() {
            if let Some(child) = method_node.child(i) {
                let ck = child.kind();
                if ck == "type_identifier" {
                    let type_name = self.utf8_text(child).to_string();
                    if type_name != "void"
                        && type_name != "int"
                        && type_name != "boolean"
                        && type_name != "char"
                        && type_name != "byte"
                        && type_name != "short"
                        && type_name != "long"
                        && type_name != "float"
                        && type_name != "double"
                    {
                        if let Some(target_id) = self.resolve_symbol_id(&type_name) {
                            let pos = child.start_position();
                            self.relationships.push(Relationship {
                                id: None,
                                source_id,
                                target_id,
                                rel_kind: RelKind::Returns,
                                file_path: String::new(),
                                line: pos.row + 1,
                                confidence: Confidence::High,
                            });
                        }
                    }
                }
            }
        }
    }

    // ── Helpers ──

    fn extract_modifiers(&self, node: Node<'_>) -> Visibility {
        // Look for "public"/"private"/"protected" in modifiers
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "modifiers" {
                    let text = self.utf8_text(child);
                    if text.contains("public") {
                        return Visibility::Pub;
                    }
                    if text.contains("private") {
                        return Visibility::Private;
                    }
                    // "protected" maps to PubCrate (closest analog)
                    if text.contains("protected") {
                        return Visibility::PubCrate;
                    }
                }
            }
        }
        Visibility::Private // package-private = Private
    }

    fn enclosing_class_name(&self, node: Node<'_>) -> Option<String> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "class_declaration"
                || parent.kind() == "interface_declaration"
                || parent.kind() == "enum_declaration"
            {
                return child_of_kind(parent, "identifier").map(|n| self.utf8_text(n).to_string());
            }
            current = parent;
        }
        None
    }

    fn find_enclosing_method_id(&self, node: Node<'_>) -> i64 {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "method_declaration"
                || parent.kind() == "constructor_declaration"
            {
                if let Some(name_node) = child_of_kind(parent, "identifier") {
                    let name = self.utf8_text(name_node);
                    if let Some(id) = self.resolve_symbol_id(name) {
                        return id;
                    }
                }
            }
            // Stop at class boundary
            if parent.kind() == "class_declaration"
                || parent.kind() == "interface_declaration"
            {
                break;
            }
            current = parent;
        }
        -1
    }

    fn class_signature(&self, node: Node<'_>) -> Option<String> {
        let name = child_of_kind(node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let mut sig = format!("class {}", name);

        if let Some(superclass) = self.find_child_recursive(node, "superclass") {
            if let Some(type_id) = child_of_kind(superclass, "type_identifier") {
                sig.push_str(&format!(" extends {}", self.utf8_text(type_id)));
            }
        }
        Some(sig)
    }

    fn method_signature(&self, node: Node<'_>) -> Option<String> {
        let name = child_of_kind(node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        Some(format!("{}(...)", name))
    }

    fn method_call_name(&self, node: Node<'_>) -> Option<String> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" => return Some(self.utf8_text(child).to_string()),
                    "field_access" => {
                        return self.last_identifier(child);
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn last_identifier(&self, node: Node<'_>) -> Option<String> {
        if node.kind() == "identifier" {
            return Some(self.utf8_text(node).to_string());
        }
        // For field_access, find the last child (the method name after .)
        let count = node.child_count();
        if count > 0 {
            if let Some(last) = node.child(count - 1) {
                return self.last_identifier(last);
            }
        }
        None
    }

    fn find_child_recursive<'n>(
        &self,
        node: Node<'n>,
        target_kind: &str,
    ) -> Option<Node<'n>> {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == target_kind {
                    return Some(child);
                }
                if let Some(found) = self.find_child_recursive(child, target_kind) {
                    return Some(found);
                }
            }
        }
        None
    }
}

/// Find the first direct child with the given kind.
fn child_of_kind<'n>(node: Node<'n>, kind: &str) -> Option<Node<'n>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::adapters::LanguageAdapter;
    use crate::tools::codegraph::types::{RelKind, SymbolKind, Visibility};

    fn adapter() -> JavaAdapter {
        JavaAdapter::new()
    }

    #[test]
    fn test_language_name() {
        assert_eq!(adapter().language(), "java");
    }

    #[test]
    fn test_file_extensions() {
        assert_eq!(adapter().file_extensions(), &["java"]);
    }

    #[test]
    fn test_parse_valid_java() {
        let a = adapter();
        let tree = a.parse("class Hello {}").unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_extract_class_symbol() {
        let a = adapter();
        let source = "public class Hello { }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.java");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Hello");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
        assert_eq!(symbols[0].language, "java");
    }

    #[test]
    fn test_extract_method_symbol() {
        let a = adapter();
        let source = "class Hello { public void greet() {} }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.java");
        assert!(symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn test_extract_extends_relationship() {
        let a = adapter();
        let source = "class Parent {}\nclass Child extends Parent {}";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.java");
        let rels = a.extract_relationships(&tree, source, &symbols);
        assert!(
            rels.iter().any(|r| r.rel_kind == RelKind::Inherits),
            "Expected an Inherits relationship, got: {:?}",
            rels.iter().map(|r| r.rel_kind.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_interface_and_implements() {
        let a = adapter();
        let source = "interface Runner {}\nclass App implements Runner {}";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.java");
        // Should have Runner (interface→Trait) and App (class→Struct)
        assert!(symbols.iter().any(|s| s.name == "Runner" && s.kind == SymbolKind::Trait));
        assert!(symbols.iter().any(|s| s.name == "App" && s.kind == SymbolKind::Struct));
    }

    #[test]
    fn test_extract_enum() {
        let a = adapter();
        let source = "enum Color { RED, GREEN, BLUE }";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.java");
        assert!(symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum));
    }
}
