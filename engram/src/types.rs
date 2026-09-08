use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseStatus {
    Graph,
    Outline,
    File,
    Skipped,
    Error,
}

impl ParseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ParseStatus::Graph => "graph",
            ParseStatus::Outline => "outline",
            ParseStatus::File => "file",
            ParseStatus::Skipped => "skipped",
            ParseStatus::Error => "error",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "graph" => Some(ParseStatus::Graph),
            "outline" => Some(ParseStatus::Outline),
            "file" => Some(ParseStatus::File),
            "skipped" => Some(ParseStatus::Skipped),
            "error" => Some(ParseStatus::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Module,
    Function,
    Method,
    Class,
    Interface,
    Type,
    Component,
    Heading,
    Selector,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
            SymbolKind::Component => "component",
            SymbolKind::Heading => "heading",
            SymbolKind::Selector => "selector",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "module" => Some(SymbolKind::Module),
            "function" => Some(SymbolKind::Function),
            "method" => Some(SymbolKind::Method),
            "class" => Some(SymbolKind::Class),
            "interface" => Some(SymbolKind::Interface),
            "type" => Some(SymbolKind::Type),
            "component" => Some(SymbolKind::Component),
            "heading" => Some(SymbolKind::Heading),
            "selector" => Some(SymbolKind::Selector),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Import,
    Call,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Import => "import",
            EdgeKind::Call => "call",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "import" => Some(EdgeKind::Import),
            "call" => Some(EdgeKind::Call),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Low,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Low => "low",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "high" => Some(Confidence::High),
            "low" => Some(Confidence::Low),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: u32, // 1-based
    pub end_line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedEdge {
    pub src_name: String,
    pub dst_name: String,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub status: ParseStatus,
    pub symbols: Vec<ExtractedSymbol>,
    pub edges: Vec<ExtractedEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContextItem {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    pub kind: Option<String>,
    pub text: String,
    pub why: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContextEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PalaceStats {
    pub status: String,
    pub attempted: u32,
    pub included: u32,
    pub dropped_for_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContextStats {
    pub files_considered: u32,
    pub symbols_considered: u32,
    pub dropped_for_budget: u32,
    pub stale_omitted: u32,
    pub stale_index: bool,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palace: Option<PalaceStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContextPackage {
    pub query: String,
    pub budget_tokens: u32,
    pub used_tokens: u32,
    pub items: Vec<ContextItem>,
    pub edges: Vec<ContextEdge>,
    pub stats: ContextStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_kind_roundtrip() {
        for k in [
            SymbolKind::Module,
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Type,
            SymbolKind::Component,
            SymbolKind::Heading,
            SymbolKind::Selector,
        ] {
            assert_eq!(SymbolKind::from_str(k.as_str()), Some(k));
        }
    }

    #[test]
    fn context_package_json_field_names() {
        let pkg = ContextPackage {
            query: "q".into(),
            budget_tokens: 3000,
            used_tokens: 0,
            items: vec![ContextItem {
                path: "a.ts".into(),
                start_line: 1,
                end_line: 2,
                symbol: Some("foo".into()),
                kind: Some("function".into()),
                text: "fn foo() {}".into(),
                why: vec!["exact_symbol".into()],
            }],
            edges: vec![],
            stats: ContextStats {
                files_considered: 1,
                symbols_considered: 1,
                dropped_for_budget: 0,
                stale_omitted: 0,
                stale_index: false,
                truncated: false,
                palace: None,
            },
        };
        let v = serde_json::to_value(&pkg).unwrap();
        assert!(v.get("budget_tokens").is_some());
        assert!(v.get("items").unwrap()[0].get("start_line").is_some());
        assert_eq!(v["stats"]["stale_index"], false);
    }

    #[test]
    fn palace_stats_omitted_when_none() {
        let pkg = ContextPackage {
            query: "q".into(),
            budget_tokens: 3000,
            used_tokens: 0,
            items: vec![],
            edges: vec![],
            stats: ContextStats {
                files_considered: 0,
                symbols_considered: 0,
                dropped_for_budget: 0,
                stale_omitted: 0,
                stale_index: false,
                truncated: false,
                palace: None,
            },
        };
        let v = serde_json::to_value(&pkg).unwrap();
        assert!(v["stats"].get("palace").is_none());
    }

    #[test]
    fn palace_stats_serialized_when_present() {
        let pkg = ContextPackage {
            query: "q".into(),
            budget_tokens: 3000,
            used_tokens: 0,
            items: vec![],
            edges: vec![],
            stats: ContextStats {
                files_considered: 0,
                symbols_considered: 0,
                dropped_for_budget: 0,
                stale_omitted: 0,
                stale_index: false,
                truncated: false,
                palace: Some(PalaceStats {
                    status: "ok".into(),
                    attempted: 3,
                    included: 2,
                    dropped_for_budget: 1,
                }),
            },
        };
        let v = serde_json::to_value(&pkg).unwrap();
        assert_eq!(v["stats"]["palace"]["status"], "ok");
        assert_eq!(v["stats"]["palace"]["included"], 2);
    }
}
