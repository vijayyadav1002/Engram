use crate::types::{
    Confidence, EdgeKind, ExtractedEdge, ExtractedSymbol, Extraction, ParseStatus, SymbolKind,
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
(interface_type_definition (name) @name) @def
(enum_type_definition (name) @name) @def
(union_type_definition (name) @name) @def
(input_object_type_definition (name) @name) @def
(scalar_type_definition (name) @name) @def
(field_definition (name) @name) @field
(input_value_definition (name) @name) @input_value
(enum_value_definition (enum_value) @name) @enum_value
(operation_definition (operation_type) @op_type (name) @name) @op
(fragment_definition (fragment_name) @name (type_condition (named_type) @frag_on)) @frag
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
    let mut edges = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    while let Some(m) = matches.next() {
        let mut def: Option<Node> = None;
        let mut field: Option<Node> = None;
        let mut input_value: Option<Node> = None;
        let mut enum_value: Option<Node> = None;
        let mut op: Option<Node> = None;
        let mut frag: Option<Node> = None;
        let mut op_type: Option<Node> = None;
        let mut frag_on: Option<Node> = None;
        let mut name: Option<Node> = None;
        for cap in m.captures {
            match query.capture_names()[cap.index as usize] {
                "def" => def = Some(cap.node),
                "field" => field = Some(cap.node),
                "input_value" => input_value = Some(cap.node),
                "enum_value" => enum_value = Some(cap.node),
                "op" => op = Some(cap.node),
                "frag" => frag = Some(cap.node),
                "op_type" => op_type = Some(cap.node),
                "frag_on" => frag_on = Some(cap.node),
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
            let (kind, signature) = match node.kind() {
                "object_type_definition" => (SymbolKind::Type, "type"),
                "interface_type_definition" => (SymbolKind::Interface, "interface"),
                "enum_type_definition" => (SymbolKind::Type, "enum"),
                "union_type_definition" => (SymbolKind::Type, "union"),
                "input_object_type_definition" => (SymbolKind::Type, "input"),
                "scalar_type_definition" => (SymbolKind::Type, "scalar"),
                _ => continue,
            };
            push_symbol(&mut symbols, ident.clone(), kind, node, Some(signature));
            for iface in implemented_names(source, node) {
                edges.push(ExtractedEdge {
                    src_name: ident.clone(),
                    dst_name: iface,
                    kind: EdgeKind::Import,
                    confidence: Confidence::High,
                });
            }
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
            continue;
        }
        if let Some(node) = input_value {
            if skip_node(node) {
                continue;
            }
            if !has_ancestor_in(node, &["input_fields_definition"]) {
                continue;
            }
            if has_ancestor_in(node, &["arguments_definition"]) {
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
            continue;
        }
        if let Some(node) = enum_value {
            if skip_node(node) {
                continue;
            }
            let Some(name_node) = name else { continue };
            let val = text(source, name_node);
            let Some(parent) = enclosing_parent_name(source, node) else {
                continue;
            };
            if val.is_empty() {
                continue;
            }
            push_symbol(
                &mut symbols,
                format!("{parent}.{val}"),
                SymbolKind::Method,
                node,
                Some("enum_value"),
            );
            continue;
        }
        if let Some(node) = op {
            if skip_node(node) {
                continue;
            }
            let Some(name_node) = name else { continue };
            let ident = text(source, name_node);
            if ident.is_empty() {
                continue;
            }
            let sig = op_type
                .map(|n| text(source, n))
                .filter(|s| !s.is_empty());
            push_symbol(
                &mut symbols,
                ident.clone(),
                SymbolKind::Function,
                node,
                sig.as_deref(),
            );
            if let Some(dst) = first_root_call_dest(source, node) {
                edges.push(ExtractedEdge {
                    src_name: ident,
                    dst_name: dst,
                    kind: EdgeKind::Call,
                    confidence: Confidence::Low,
                });
            }
            continue;
        }
        if let Some(node) = frag {
            if skip_node(node) {
                continue;
            }
            let Some(name_node) = name else { continue };
            let ident = text(source, name_node);
            if ident.is_empty() {
                continue;
            }
            push_symbol(
                &mut symbols,
                ident.clone(),
                SymbolKind::Type,
                node,
                Some("fragment"),
            );
            if let Some(on_node) = frag_on {
                let on_type = text(source, on_node);
                if !on_type.is_empty() {
                    edges.push(ExtractedEdge {
                        src_name: ident,
                        dst_name: on_type,
                        kind: EdgeKind::Import,
                        confidence: Confidence::High,
                    });
                }
            }
        }
    }

    Extraction {
        status: ParseStatus::Graph,
        symbols,
        edges,
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

fn implemented_names(source: &str, def: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    let mut walk = def.walk();
    for child in def.children(&mut walk) {
        if child.kind() == "implements_interfaces" {
            collect_named_types(source, child, &mut names);
        }
    }
    names
}

fn collect_named_types(source: &str, node: Node<'_>, out: &mut Vec<String>) {
    if node.kind() == "named_type" {
        let ident = text(source, node);
        if !ident.is_empty() {
            out.push(ident);
        }
        return;
    }
    let mut walk = node.walk();
    for child in node.children(&mut walk) {
        collect_named_types(source, child, out);
    }
}

fn first_root_call_dest(source: &str, op: Node<'_>) -> Option<String> {
    let mut op_type = None;
    let mut selection_set = None;
    let mut walk = op.walk();
    for child in op.children(&mut walk) {
        match child.kind() {
            "operation_type" => op_type = Some(text(source, child)),
            "selection_set" => selection_set = Some(child),
            _ => {}
        }
    }
    let root = match op_type.as_deref() {
        Some("query") => "Query",
        Some("mutation") => "Mutation",
        Some("subscription") => "Subscription",
        _ => return None,
    };
    let set = selection_set?;
    let mut walk = set.walk();
    for child in set.children(&mut walk) {
        let field = if child.kind() == "field" {
            child
        } else if child.kind() == "selection" {
            let mut sw = child.walk();
            let mut found = None;
            for n in child.children(&mut sw) {
                if n.kind() == "field" {
                    found = Some(n);
                    break;
                }
            }
            match found {
                Some(n) => n,
                None => continue,
            }
        } else {
            continue;
        };
        if let Some(field_ident) = field_response_name(source, field) {
            return Some(format!("{root}.{field_ident}"));
        }
    }
    None
}

fn field_response_name(source: &str, field: Node<'_>) -> Option<String> {
    let mut walk = field.walk();
    for child in field.children(&mut walk) {
        if child.kind() == "alias" {
            continue;
        }
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
