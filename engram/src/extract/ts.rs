use crate::types::{
    Confidence, EdgeKind, ExtractedEdge, ExtractedSymbol, Extraction, ParseStatus, SymbolKind,
};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

const FILE_MODULE: &str = "<file>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsLang {
    Typescript,
    Tsx,
    Javascript,
}

/// JS-compatible patterns only. TS-only node types would fail `Query::new` on the JS grammar.
const QUERY_JS: &str = r#"
(function_declaration name: (identifier) @name) @def
(generator_function_declaration name: (identifier) @name) @def
(function_expression name: (identifier) @name) @def
(generator_function name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(method_definition name: (private_property_identifier) @name) @def
(class_declaration name: (_) @name) @def
(class name: (_) @name) @def
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)]) @def)
(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)]) @def)
(import_statement (import_clause (identifier) @imported))
(import_statement (import_clause (named_imports (import_specifier name: (identifier) @imported))))
(import_statement (import_clause (namespace_import (identifier) @imported)))
(import_statement source: (string) @mod)
(call_expression function: (identifier) @callee)
(call_expression function: (member_expression property: (property_identifier) @callee))
"#;

const QUERY_TS: &str = r#"
(function_declaration name: (identifier) @name) @def
(generator_function_declaration name: (identifier) @name) @def
(function_expression name: (identifier) @name) @def
(generator_function name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(method_definition name: (private_property_identifier) @name) @def
(class_declaration name: (_) @name) @def
(class name: (_) @name) @def
(abstract_class_declaration name: (_) @name) @def
(interface_declaration name: (_) @name) @def
(type_alias_declaration name: (_) @name) @def
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)]) @def)
(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)]) @def)
(import_statement (import_clause (identifier) @imported))
(import_statement (import_clause (named_imports (import_specifier name: (identifier) @imported))))
(import_statement (import_clause (namespace_import (identifier) @imported)))
(import_statement source: (string) @mod)
(call_expression function: (identifier) @callee)
(call_expression function: (member_expression property: (property_identifier) @callee))
"#;

pub fn extract(source: &str, lang: TsLang) -> Extraction {
    let language = language(lang);
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return error_extraction();
    }
    let Some(tree) = parser.parse(source, None) else {
        return error_extraction();
    };
    let root = tree.root_node();
    if root.child_count() == 0 && root.has_error() {
        return error_extraction();
    }

    let query_src = match lang {
        TsLang::Javascript => QUERY_JS,
        TsLang::Typescript | TsLang::Tsx => QUERY_TS,
    };
    let query = match Query::new(&language, query_src) {
        Ok(q) => q,
        Err(_) => return error_extraction(),
    };

    let mut symbols = vec![module_symbol(root)];
    let mut edges = Vec::new();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    while let Some(m) = matches.next() {
        let mut def: Option<Node> = None;
        let mut name: Option<Node> = None;
        let mut import_mod: Option<Node> = None;
        let mut imported: Option<Node> = None;
        let mut callee: Option<Node> = None;

        for cap in m.captures {
            match query.capture_names()[cap.index as usize] {
                "def" => def = Some(cap.node),
                "name" => name = Some(cap.node),
                "mod" => import_mod = Some(cap.node),
                "imported" => imported = Some(cap.node),
                "callee" => callee = Some(cap.node),
                _ => {}
            }
        }

        if let (Some(def_node), Some(name_node)) = (def, name) {
            let ident = text(source, name_node);
            if ident.is_empty() {
                continue;
            }
            let kind = symbol_kind(def_node, &ident, lang);
            let (start_line, end_line, start_byte, end_byte) = node_span(def_node);
            symbols.push(ExtractedSymbol {
                name: ident,
                kind,
                start_line,
                end_line,
                start_byte,
                end_byte,
                signature: None,
            });
            continue;
        }

        if let Some(mod_node) = import_mod {
            let dst = unquote(&text(source, mod_node));
            if !dst.is_empty() {
                edges.push(import_edge(dst));
            }
        }
        if let Some(imported_node) = imported {
            let dst = text(source, imported_node);
            if !dst.is_empty() {
                edges.push(import_edge(dst));
            }
        }
        if let Some(callee_node) = callee {
            let dst = text(source, callee_node);
            if dst.is_empty() {
                continue;
            }
            edges.push(ExtractedEdge {
                src_name: enclosing_name(source, callee_node),
                dst_name: dst,
                kind: EdgeKind::Call,
                confidence: Confidence::Low,
            });
        }
    }

    Extraction {
        status: ParseStatus::Graph,
        symbols,
        edges,
    }
}

fn language(lang: TsLang) -> Language {
    match lang {
        TsLang::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TsLang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        TsLang::Javascript => tree_sitter_javascript::LANGUAGE.into(),
    }
}

fn error_extraction() -> Extraction {
    Extraction {
        status: ParseStatus::Error,
        symbols: vec![],
        edges: vec![],
    }
}

fn module_symbol(root: Node<'_>) -> ExtractedSymbol {
    let (start_line, end_line, start_byte, end_byte) = node_span(root);
    ExtractedSymbol {
        name: FILE_MODULE.into(),
        kind: SymbolKind::Module,
        start_line,
        end_line,
        start_byte,
        end_byte,
        signature: None,
    }
}

fn import_edge(dst_name: String) -> ExtractedEdge {
    ExtractedEdge {
        src_name: FILE_MODULE.into(),
        dst_name,
        kind: EdgeKind::Import,
        confidence: Confidence::High,
    }
}

fn text(source: &str, node: Node<'_>) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or("")
        .to_string()
}

fn unquote(raw: &str) -> String {
    raw.trim_matches(['"', '\'', '`']).to_string()
}

fn node_span(node: Node<'_>) -> (u32, u32, u32, u32) {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    (
        start_line,
        end_line.max(start_line),
        node.start_byte() as u32,
        node.end_byte() as u32,
    )
}

fn symbol_kind(def_node: Node<'_>, name: &str, lang: TsLang) -> SymbolKind {
    let kind = match def_node.kind() {
        "class_declaration" | "class" | "abstract_class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "type_alias_declaration" => SymbolKind::Type,
        "method_definition" => SymbolKind::Method,
        _ if is_method(def_node) => SymbolKind::Method,
        _ => SymbolKind::Function,
    };
    if lang == TsLang::Tsx && matches!(kind, SymbolKind::Function) && is_pascal_case(name) {
        SymbolKind::Component
    } else {
        kind
    }
}

fn is_pascal_case(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn is_method(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "class_declaration" | "class" | "abstract_class_declaration" => return true,
            "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
            | "program" => return false,
            _ => node = parent,
        }
    }
    false
}

fn enclosing_name(source: &str, mut node: Node<'_>) -> String {
    while let Some(parent) = node.parent() {
        if is_named_scope(parent.kind()) {
            if let Some(name) = parent.child_by_field_name("name") {
                let ident = text(source, name);
                if !ident.is_empty() {
                    return ident;
                }
            }
        }
        node = parent;
    }
    FILE_MODULE.into()
}

fn is_named_scope(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "method_definition"
            | "class_declaration"
            | "class"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "variable_declarator"
    )
}

#[cfg(test)]
mod tests {
    use crate::types::{EdgeKind, ParseStatus, SymbolKind};

    #[test]
    fn tsx_component_and_import() {
        let src = r#"
import { createSession } from "./session";
export function LoginBanner() {
  return createSession();
}
export function helper() {
  return LoginBanner();
}
"#;
        let ext = crate::extract::ts::extract(src, crate::extract::ts::TsLang::Tsx);
        assert_eq!(ext.status, ParseStatus::Graph);
        let names: Vec<_> = ext.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"LoginBanner"));
        assert!(names.contains(&"helper"));
        assert!(ext.symbols.iter().any(|s| s.name == "LoginBanner"
            && (s.kind == SymbolKind::Function || s.kind == SymbolKind::Component)));
        assert!(ext.edges.iter().any(|e| e.dst_name == "createSession"));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "<file>" && s.kind == SymbolKind::Module));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "LoginBanner" && s.kind == SymbolKind::Component));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "helper" && s.kind == SymbolKind::Function));
        assert!(ext.edges.iter().any(|e| e.kind == EdgeKind::Import
            && e.src_name == "<file>"
            && e.dst_name == "createSession"));
        assert!(ext.edges.iter().any(|e| e.kind == EdgeKind::Call
            && e.src_name == "LoginBanner"
            && e.dst_name == "createSession"
            && e.confidence == crate::types::Confidence::Low));
    }

    #[test]
    fn ts_interface_and_class() {
        let src = "export interface User { id: string }\nexport class AuthService { login() {} }\n";
        let ext = crate::extract::ts::extract(src, crate::extract::ts::TsLang::Typescript);
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "User" && s.kind == SymbolKind::Interface));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "AuthService" && s.kind == SymbolKind::Class));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "login" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn jsx_uses_tsx_grammar_not_ts() {
        let src = "export const App = () => <div/>;";
        let ext = crate::extract::ts::extract(src, crate::extract::ts::TsLang::Tsx);
        assert_eq!(ext.status, ParseStatus::Graph);
        assert!(ext.symbols.iter().any(|s| s.name == "App"));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "App" && s.kind == SymbolKind::Component));
    }
}
