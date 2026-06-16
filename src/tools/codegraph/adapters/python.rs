//! Python language adapter — parses `.py` files using tree-sitter-python.
//!
//! Extracts symbols (functions, classes, methods), references (calls),
//! and relationships (calls, inherits, parameter, returns).

use crate::tools::codegraph::adapters::LanguageAdapter;
use crate::tools::codegraph::types::{
    Confidence, RefKind, Reference, RelKind, Relationship, Symbol, SymbolKind, Visibility,
};
use tree_sitter::{Language, Node, Tree};

/// Adapter for parsing and extracting Python source code.
pub struct PythonAdapter;

impl PythonAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PythonAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language(&self) -> &'static str {
        "python"
    }

    fn file_extensions(&self) -> &[&str] {
        &["py"]
    }

    fn tree_sitter_language(&self) -> Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<Symbol> {
        let mut extractor = PythonExtractor::new(source, file_path);
        extractor.collect_definitions(tree.root_node());
        extractor.symbols
    }

    fn extract_references(&self, tree: &Tree, source: &str, symbols: &[Symbol]) -> Vec<Reference> {
        let mut extractor = PythonExtractor::new(source, "");
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
        let mut extractor = PythonExtractor::new(source, "");
        extractor.target_symbols = symbols.to_vec();
        extractor.collect_relationships_pass(tree.root_node());
        extractor.relationships
    }
}

// ── Internal extractor ──

struct PythonExtractor<'a> {
    source: &'a str,
    file_path: &'a str,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
    target_symbols: Vec<Symbol>,
}

impl<'a> PythonExtractor<'a> {
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
            "function_definition" => {
                if let Some(name_node) = child_of_kind(node, "identifier") {
                    let name = self.utf8_text(name_node).to_string();
                    if !name.is_empty() && !name.starts_with("__") {
                        let pos = node.start_position();
                        self.symbols.push(Symbol {
                            id: Some(self.symbols.len() as i64),
                            name,
                            kind: SymbolKind::Function,
                            file_path: self.file_path.to_string(),
                            line: pos.row + 1,
                            col: pos.column + 1,
                            signature: self.function_signature(node),
                            visibility: Visibility::Pub, // Python has no access modifiers
                            parent_module: self.enclosing_class_name(node),
                            language: "python".to_string(),
                        });
                    }
                }
            }
            "class_definition" => {
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
                            visibility: Visibility::Pub,
                            parent_module: None,
                            language: "python".to_string(),
                        });
                    }
                }
            }
            _ => {}
        }

        // Recurse into children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_definitions(child);
            }
        }
    }

    // ── Reference extraction ──

    fn collect_references_pass(&mut self, node: Node<'_>) {
        if node.kind() == "call" {
            self.extract_call_ref(node);
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.collect_references_pass(child);
            }
        }
    }

    fn extract_call_ref(&mut self, node: Node<'_>) {
        // In Python tree-sitter, `call` nodes have:
        //   call: (identifier | attribute) argument_list
        // For `obj.method()`, it's: call (attribute object: (identifier) attribute: (identifier))
        let called_name = self.call_target_name(node);
        if let Some(name) = called_name {
            if !name.is_empty() && name != "self" {
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
            "class_definition" => {
                // Inheritance: class Child(Parent) → Inherits
                self.extract_inherits_rels(node);
            }
            "call" => {
                // Calls relationship
                if let Some(called_name) = self.call_target_name(node) {
                    if called_name != "self" {
                        if let Some(target_id) = self.resolve_symbol_id(&called_name) {
                            if target_id >= 0 {
                                let source_id = self.find_enclosing_function_id(node);
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
                }
            }
            "function_definition" => {
                // Parameter relationships (typed params)
                self.extract_parameter_rels(node);

                // Returns relationship (-> ReturnType)
                self.extract_return_type_rel(node);
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

    fn extract_inherits_rels(&mut self, class_node: Node<'_>) {
        let class_name = child_of_kind(class_node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let source_id = self.resolve_symbol_id(&class_name).unwrap_or(-1);

        // Python inheritance: class Child(Parent1, Parent2)
        // The parent class names are in argument_list
        for i in 0..class_node.child_count() {
            if let Some(child) = class_node.child(i) {
                if child.kind() == "argument_list" {
                    for j in 0..child.child_count() {
                        if let Some(arg) = child.child(j) {
                            if arg.kind() == "identifier" {
                                let parent_name = self.utf8_text(arg).to_string();
                                if let Some(target_id) = self.resolve_symbol_id(&parent_name) {
                                    let pos = arg.start_position();
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
            }
        }
    }

    fn extract_parameter_rels(&mut self, func_node: Node<'_>) {
        let func_name = child_of_kind(func_node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let source_id = self.resolve_symbol_id(&func_name).unwrap_or(-1);

        // Find `parameters` node
        for i in 0..func_node.child_count() {
            if let Some(child) = func_node.child(i) {
                if child.kind() == "parameters" {
                    // Each typed parameter looks like: (identifier type: (type ...))
                    for j in 0..child.child_count() {
                        if let Some(param) = child.child(j) {
                            if param.kind() == "typed_parameter"
                                || param.kind() == "typed_default_parameter"
                            {
                                self.extract_typed_param_rel(param, source_id);
                            }
                        }
                    }
                }
            }
        }
    }

    fn extract_typed_param_rel(&mut self, param_node: Node<'_>, source_id: i64) {
        // typed_parameter: (identifier type: (type ...))
        for i in 0..param_node.child_count() {
            if let Some(child) = param_node.child(i) {
                if child.kind() == "type" {
                    let type_text = self.utf8_text(child).to_string();
                    if !type_text.is_empty()
                        && type_text != "int"
                        && type_text != "str"
                        && type_text != "float"
                        && type_text != "bool"
                        && type_text != "list"
                        && type_text != "dict"
                        && type_text != "None"
                    {
                        if let Some(target_id) = self.resolve_symbol_id(&type_text) {
                            let pos = child.start_position();
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

    fn extract_return_type_rel(&mut self, func_node: Node<'_>) {
        let func_name = child_of_kind(func_node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let source_id = self.resolve_symbol_id(&func_name).unwrap_or(-1);

        // In tree-sitter-python, the return type is `-> type` after parameters
        for i in 0..func_node.child_count() {
            if let Some(child) = func_node.child(i) {
                if child.kind() == "type" && i > 0 {
                    // Make sure it's the return type (after parameters/colon)
                    let type_text = self.utf8_text(child).to_string();
                    if !type_text.is_empty()
                        && type_text != "None"
                        && type_text != "int"
                        && type_text != "str"
                        && type_text != "float"
                        && type_text != "bool"
                        && type_text != "list"
                        && type_text != "dict"
                    {
                        if let Some(target_id) = self.resolve_symbol_id(&type_text) {
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

    fn enclosing_class_name(&self, node: Node<'_>) -> Option<String> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "class_definition" {
                return child_of_kind(parent, "identifier").map(|n| self.utf8_text(n).to_string());
            }
            current = parent;
        }
        None
    }

    fn find_enclosing_function_id(&self, node: Node<'_>) -> i64 {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "function_definition" {
                if let Some(name_node) = child_of_kind(parent, "identifier") {
                    let name = self.utf8_text(name_node);
                    if let Some(id) = self.resolve_symbol_id(name) {
                        return id;
                    }
                }
            }
            if parent.kind() == "class_definition" {
                break;
            }
            current = parent;
        }
        // If we're inside a class method, the enclosing class could be the source
        // Find the class
        let mut c = node;
        while let Some(parent) = c.parent() {
            if parent.kind() == "class_definition" {
                if let Some(name_node) = child_of_kind(parent, "identifier") {
                    let name = self.utf8_text(name_node);
                    if let Some(id) = self.resolve_symbol_id(name) {
                        return id;
                    }
                }
            }
            c = parent;
        }
        -1
    }

    fn call_target_name(&self, node: Node<'_>) -> Option<String> {
        // call: (identifier | attribute) argument_list
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" => return Some(self.utf8_text(child).to_string()),
                    "attribute" => {
                        // obj.method() → method is the attribute name
                        // attribute: object . attribute
                        if child.child_count() >= 3 {
                            // The last child (attribute name) is at index 2
                            if let Some(attr) = child.child(2) {
                                return Some(self.utf8_text(attr).to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn function_signature(&self, node: Node<'_>) -> Option<String> {
        let name = child_of_kind(node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        Some(format!("def {}(...)", name))
    }

    fn class_signature(&self, node: Node<'_>) -> Option<String> {
        let name = child_of_kind(node, "identifier")
            .map(|n| self.utf8_text(n).to_string())
            .unwrap_or_default();
        let mut sig = format!("class {}", name);

        // Check for parent classes in argument_list
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "argument_list" {
                    let parents: Vec<String> = (0..child.child_count())
                        .filter_map(|j| child.child(j))
                        .filter(|c| c.kind() == "identifier")
                        .map(|c| self.utf8_text(c).to_string())
                        .collect();
                    if !parents.is_empty() {
                        sig.push_str(&format!("({})", parents.join(", ")));
                    }
                }
            }
        }
        Some(sig)
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

    fn adapter() -> PythonAdapter {
        PythonAdapter::new()
    }

    #[test]
    fn test_language_name() {
        assert_eq!(adapter().language(), "python");
    }

    #[test]
    fn test_file_extensions() {
        assert_eq!(adapter().file_extensions(), &["py"]);
    }

    #[test]
    fn test_parse_valid_python() {
        let a = adapter();
        let tree = a.parse("def hello():\n    pass\n").unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_extract_function_symbol() {
        let a = adapter();
        let source = "def hello(x: int) -> bool:\n    return True\n";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.py");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].language, "python");
    }

    #[test]
    fn test_extract_class_symbol() {
        let a = adapter();
        let source = "class MyClass:\n    pass\n";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.py");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MyClass");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
        assert_eq!(symbols[0].language, "python");
    }

    #[test]
    fn test_extract_class_with_method() {
        let a = adapter();
        let source = "class Dog:\n    def bark(self):\n        pass\n";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.py");
        assert!(symbols
            .iter()
            .any(|s| s.name == "Dog" && s.kind == SymbolKind::Struct));
        assert!(symbols
            .iter()
            .any(|s| s.name == "bark" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn test_extract_inherits_relationship() {
        let a = adapter();
        let source = "class Parent:\n    pass\n\nclass Child(Parent):\n    pass\n";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.py");
        let rels = a.extract_relationships(&tree, source, &symbols);
        assert!(
            rels.iter().any(|r| r.rel_kind == RelKind::Inherits),
            "Expected an Inherits relationship, got: {:?}",
            rels.iter().map(|r| r.rel_kind.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_call_relationship() {
        let a = adapter();
        let source = "def foo():\n    pass\n\ndef bar():\n    foo()\n";
        let tree = a.parse(source).unwrap();
        let symbols = a.extract_symbols(&tree, source, "test.py");
        let rels = a.extract_relationships(&tree, source, &symbols);
        assert!(
            rels.iter().any(|r| r.rel_kind == RelKind::Calls),
            "Expected a Calls relationship"
        );
    }
}
