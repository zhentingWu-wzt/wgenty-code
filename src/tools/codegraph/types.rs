use serde::{Deserialize, Serialize};

/// Kinds of symbols the indexer can extract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    TypeAlias,
    Const,
    Static,
    Mod,
    Macro,
}

impl SymbolKind {
    /// Parse a tree-sitter node type string into a SymbolKind.
    pub fn from_node_type(node_type: &str) -> Option<Self> {
        match node_type {
            "function_item" => Some(Self::Function),
            "struct_item" => Some(Self::Struct),
            "enum_item" => Some(Self::Enum),
            "trait_item" => Some(Self::Trait),
            "impl_item" => Some(Self::Impl),
            "type_item" => Some(Self::TypeAlias),
            "const_item" => Some(Self::Const),
            "static_item" => Some(Self::Static),
            "mod_item" => Some(Self::Mod),
            "macro_definition" => Some(Self::Macro),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::TypeAlias => "type_alias",
            Self::Const => "const",
            Self::Static => "static",
            Self::Mod => "mod",
            Self::Macro => "macro",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Pub,
    PubCrate,
    PubSuper,
    Private,
}

impl Visibility {
    pub fn from_visibility_modifier(modifier: &str) -> Self {
        match modifier {
            "pub" => Self::Pub,
            "pub(crate)" => Self::PubCrate,
            "pub(super)" => Self::PubSuper,
            _ => Self::Private,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pub => "pub",
            Self::PubCrate => "pub(crate)",
            Self::PubSuper => "pub(super)",
            Self::Private => "private",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: Option<i64>,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line: usize,
    pub col: usize,
    pub signature: Option<String>,
    pub visibility: Visibility,
    pub parent_module: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub id: Option<i64>,
    pub symbol_id: i64,
    pub file_path: String,
    pub line: usize,
    pub col: usize,
    pub ref_kind: RefKind,
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    Call,
    TypeRef,
    Import,
    MethodCall,
}

impl RefKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::TypeRef => "type_ref",
            Self::Import => "import",
            Self::MethodCall => "method_call",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "call" => Self::Call,
            "type_ref" => Self::TypeRef,
            "import" => Self::Import,
            "method_call" => Self::MethodCall,
            _ => Self::Call,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Option<i64>,
    pub source_id: i64,
    pub target_id: i64,
    pub rel_kind: RelKind,
    pub file_path: String,
    pub line: usize,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelKind {
    Calls,
    Implements,
    Contains,
    Imports,
}

impl RelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Implements => "implements",
            Self::Contains => "contains",
            Self::Imports => "imports",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "calls" => Self::Calls,
            "implements" => Self::Implements,
            "contains" => Self::Contains,
            "imports" => Self::Imports,
            _ => Self::Calls,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Unresolved,
}

impl Confidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unresolved => "unresolved",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Unresolved,
        }
    }

    /// Map a parse source to a confidence level.
    pub fn from_parse_source(source: &ParseSource) -> Self {
        match source {
            ParseSource::TreeSitter => Confidence::High,
            ParseSource::TextMatch => Confidence::Medium,
            ParseSource::Inferred => Confidence::Low,
            ParseSource::None => Confidence::Unresolved,
        }
    }
}

/// How a symbol or relationship was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseSource {
    /// Directly extracted from tree-sitter AST.
    #[serde(rename = "treesitter-ast")]
    TreeSitter,
    /// Matched via text/regex search.
    #[serde(rename = "regex-match")]
    TextMatch,
    /// Inferred from context (e.g., multi-hop relationship chain).
    #[serde(rename = "inferred")]
    Inferred,
    /// No parse source (not found).
    #[serde(rename = "none")]
    None,
}

impl ParseSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TreeSitter => "treesitter-ast",
            Self::TextMatch => "regex-match",
            Self::Inferred => "inferred",
            Self::None => "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_kind_roundtrip() {
        let kinds = vec![
            SymbolKind::Function,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::Impl,
            SymbolKind::TypeAlias,
            SymbolKind::Const,
            SymbolKind::Static,
            SymbolKind::Mod,
            SymbolKind::Macro,
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let deserialized: SymbolKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, deserialized);
        }
    }

    #[test]
    fn test_visibility_roundtrip() {
        let visibilities = vec![
            Visibility::Pub,
            Visibility::PubCrate,
            Visibility::PubSuper,
            Visibility::Private,
        ];
        for v in &visibilities {
            let json = serde_json::to_string(v).unwrap();
            let deserialized: Visibility = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, deserialized);
        }
    }

    #[test]
    fn test_symbol_serialization() {
        let sym = Symbol {
            id: Some(1),
            name: "foo".into(),
            kind: SymbolKind::Function,
            file_path: "src/lib.rs".into(),
            line: 10,
            col: 1,
            signature: Some("fn foo(x: i32) -> bool".into()),
            visibility: Visibility::Pub,
            parent_module: Some("my_module".into()),
        };
        let json = serde_json::to_string_pretty(&sym).unwrap();
        let deserialized: Symbol = serde_json::from_str(&json).unwrap();
        assert_eq!(sym.name, deserialized.name);
    }

    #[test]
    fn test_kind_from_node_type() {
        assert_eq!(
            SymbolKind::from_node_type("function_item"),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            SymbolKind::from_node_type("struct_item"),
            Some(SymbolKind::Struct)
        );
        assert_eq!(SymbolKind::from_node_type("unknown"), None);
    }
}
