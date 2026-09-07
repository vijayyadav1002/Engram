use crate::types::{
    Confidence, EdgeKind, ExtractedEdge, ExtractedSymbol, Extraction, ParseStatus, SymbolKind,
};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

const FILE_MODULE: &str = "<file>";

const QUERY_SRC: &str = r#"
(function_definition name: (identifier) @name) @def
(class_definition name: (identifier) @name) @def
(import_statement name: (dotted_name) @mod)
(import_statement name: (aliased_import name: (dotted_name) @mod))
(import_from_statement module_name: (dotted_name) @mod)
(import_from_statement module_name: (relative_import) @mod)
(import_from_statement name: (dotted_name) @imported)
(import_from_statement name: (aliased_import name: (dotted_name) @imported))
(call function: (identifier) @callee)
(call function: (attribute attribute: (identifier) @callee))
"#;

pub fn extract(source: &str) -> Extraction {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
        return error_extraction();
    }
    let Some(tree) = parser.parse(source, None) else {
        return error_extraction();
    };
    let root = tree.root_node();
    if root.child_count() == 0 && root.has_error() {
        return error_extraction();
    }

    let language = tree_sitter::Language::from(tree_sitter_python::LANGUAGE);
    let query = match Query::new(&language, QUERY_SRC) {
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
            let cap_name = query.capture_names()[cap.index as usize];
            match cap_name {
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
            let kind = if def_node.kind() == "class_definition" {
                SymbolKind::Class
            } else if is_method(def_node) {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
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
            let dst = text(source, mod_node);
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

fn is_method(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "class_definition" => return true,
            "function_definition" | "module" => return false,
            _ => node = parent,
        }
    }
    false
}

fn enclosing_name(source: &str, mut node: Node<'_>) -> String {
    while let Some(parent) = node.parent() {
        if parent.kind() == "function_definition" || parent.kind() == "class_definition" {
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
