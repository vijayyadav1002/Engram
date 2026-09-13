use crate::types::{
    ExtractedEdge, ExtractedSymbol, Extraction, ParseStatus, SymbolKind,
};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

const FILE_MODULE: &str = "<file>";

const EXTENSION_KINDS: &[&str] = &[
    "type_extension",
    "schema_extension",
    "scalar_type_extension",
    "object_type_extension",
    "interface_type_extension",
    "union_type_extension",
    "enum_type_extension",
    "input_object_type_extension",
];

const QUERY_SRC: &str = r#"
(object_type_definition (name) @name) @def
(field_definition (name) @name) @field
"#;

pub fn extract(source: &str) -> Extraction {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_graphql::LANGUAGE.into())
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
    let language = tree_sitter::Language::from(tree_sitter_graphql::LANGUAGE);
    let query = match Query::new(&language, QUERY_SRC) {
        Ok(q) => q,
        Err(_) => return error_extraction(),
    };

    let mut symbols = vec![module_symbol(root)];
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    while let Some(m) = matches.next() {
        let mut def: Option<Node> = None;
        let mut field: Option<Node> = None;
        let mut name: Option<Node> = None;
        for cap in m.captures {
            match query.capture_names()[cap.index as usize] {
                "def" => def = Some(cap.node),
                "field" => field = Some(cap.node),
                "name" => name = Some(cap.node),
                _ => {}
            }
        }
        if let Some(node) = def {
            if skip_node(node) {
                continue;
            }
            let Some(name_node) = name else { continue };
            let ident = text(source, name_node);
            if ident.is_empty() {
                continue;
            }
            push_symbol(&mut symbols, ident, SymbolKind::Type, node, Some("type"));
            continue;
        }
        if let Some(node) = field {
            if skip_node(node) {
                continue;
            }
            let Some(name_node) = name else { continue };
            let field_ident = text(source, name_node);
            let Some(parent) = enclosing_parent_name(source, node) else {
                continue;
            };
            if field_ident.is_empty() {
                continue;
            }
            push_symbol(
                &mut symbols,
                format!("{parent}.{field_ident}"),
                SymbolKind::Method,
                node,
                Some("field"),
            );
        }
    }

    Extraction {
        status: ParseStatus::Graph,
        symbols,
        edges: Vec::<ExtractedEdge>::new(),
    }
}

fn skip_node(node: Node<'_>) -> bool {
    node.has_error() || has_ancestor_in(node, EXTENSION_KINDS)
}

fn has_ancestor_in(mut node: Node<'_>, kinds: &[&str]) -> bool {
    while let Some(parent) = node.parent() {
        if kinds.iter().any(|k| parent.kind() == *k) {
            return true;
        }
        node = parent;
    }
    false
}

fn enclosing_parent_name(source: &str, mut node: Node<'_>) -> Option<String> {
    while let Some(parent) = node.parent() {
        if has_ancestor_in(parent, EXTENSION_KINDS) {
            return None;
        }
        match parent.kind() {
            "object_type_definition"
            | "interface_type_definition"
            | "enum_type_definition"
            | "input_object_type_definition" => {
                return name_of_def(source, parent);
            }
            _ => node = parent,
        }
    }
    None
}

fn name_of_def(source: &str, def: Node<'_>) -> Option<String> {
    let mut walk = def.walk();
    for child in def.children(&mut walk) {
        if child.kind() == "name" {
            let ident = text(source, child);
            if !ident.is_empty() {
                return Some(ident);
            }
        }
    }
    None
}

fn push_symbol(
    symbols: &mut Vec<ExtractedSymbol>,
    name: String,
    kind: SymbolKind,
    node: Node<'_>,
    signature: Option<&str>,
) {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    symbols.push(ExtractedSymbol {
        name,
        kind,
        start_line,
        end_line: end_line.max(start_line),
        start_byte: node.start_byte() as u32,
        end_byte: node.end_byte() as u32,
        signature: signature.map(str::to_string),
    });
}

fn text(source: &str, node: Node<'_>) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or("")
        .to_string()
}

fn error_extraction() -> Extraction {
    Extraction {
        status: ParseStatus::Error,
        symbols: vec![],
        edges: vec![],
    }
}

fn module_symbol(root: Node<'_>) -> ExtractedSymbol {
    let start_line = root.start_position().row as u32 + 1;
    let end_line = root.end_position().row as u32 + 1;
    ExtractedSymbol {
        name: FILE_MODULE.into(),
        kind: SymbolKind::Module,
        start_line,
        end_line: end_line.max(start_line),
        start_byte: root.start_byte() as u32,
        end_byte: root.end_byte() as u32,
        signature: None,
    }
}
