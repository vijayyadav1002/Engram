use crate::types::{ExtractedSymbol, Extraction, ParseStatus, SymbolKind};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> Extraction {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_yaml::LANGUAGE.into())
        .is_err()
    {
        return error_extraction();
    }
    let Some(tree) = parser.parse(source, None) else {
        return error_extraction();
    };
    let root = tree.root_node();
    if source.trim().is_empty() || (root.child_count() == 0 && root.has_error()) {
        return error_extraction();
    }
    let mut symbols = Vec::new();
    let mut walk = root.walk();
    for child in root.named_children(&mut walk) {
        if child.kind() == "document" {
            if let Some(mapping) = mapping_of(child) {
                emit_mapping_keys(source, mapping, &mut symbols);
            }
        }
    }
    if symbols.is_empty() {
        if let Some(mapping) = mapping_of(root) {
            emit_mapping_keys(source, mapping, &mut symbols);
        }
    }
    Extraction {
        status: ParseStatus::Outline,
        symbols,
        edges: vec![],
    }
}

fn mapping_of(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "block_mapping" | "flow_mapping" => Some(node),
        "document" | "block_node" | "flow_node" | "stream" => {
            let mut walk = node.walk();
            for child in node.named_children(&mut walk) {
                if matches!(child.kind(), "comment" | "anchor" | "tag") {
                    continue;
                }
                if let Some(found) = mapping_of(child) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn emit_mapping_keys(source: &str, mapping: Node<'_>, symbols: &mut Vec<ExtractedSymbol>) {
    let mut walk = mapping.walk();
    for pair in mapping.named_children(&mut walk) {
        if !matches!(pair.kind(), "block_mapping_pair" | "flow_pair") || pair.has_error() {
            continue;
        }
        let Some(name) = yaml_key_text(source, pair) else {
            continue;
        };
        push_heading(symbols, name.clone(), pair);
        if let Some(nested) = yaml_mapping_value(pair) {
            let mut inner = nested.walk();
            for nested_pair in nested.named_children(&mut inner) {
                if !matches!(nested_pair.kind(), "block_mapping_pair" | "flow_pair")
                    || nested_pair.has_error()
                {
                    continue;
                }
                let Some(child) = yaml_key_text(source, nested_pair) else {
                    continue;
                };
                push_heading(symbols, format!("{name}.{child}"), nested_pair);
            }
        }
    }
}

fn yaml_mapping_value(pair: Node<'_>) -> Option<Node<'_>> {
    let value = pair.child_by_field_name("value")?;
    mapping_of(value)
}

fn yaml_key_text(source: &str, pair: Node<'_>) -> Option<String> {
    let key = pair.child_by_field_name("key")?;
    if key.has_error() {
        return None;
    }
    let node = unwrap_yaml(key);
    match node.kind() {
        "alias" | "block_mapping" | "flow_mapping" | "block_sequence" | "flow_sequence" => {
            return None;
        }
        _ => {}
    }
    let raw = match node.kind() {
        "double_quote_scalar" | "single_quote_scalar" => {
            strip_one_quote_pair(&text(source, node)).to_string()
        }
        "string_scalar"
        | "boolean_scalar"
        | "integer_scalar"
        | "float_scalar"
        | "null_scalar"
        | "timestamp_scalar" => text(source, node),
        "plain_scalar" => text(source, node),
        _ => text(source, node),
    };
    let name = nonempty_trim(raw)?;
    if name == "<<" || name.starts_with('*') {
        None
    } else {
        Some(name)
    }
}

fn unwrap_yaml(node: Node<'_>) -> Node<'_> {
    let mut cur = node;
    for _ in 0..8 {
        match cur.kind() {
            "flow_node" | "block_node" | "plain_scalar" => {
                let mut walk = cur.walk();
                let next = cur.named_children(&mut walk).find(|c| {
                    !matches!(c.kind(), "comment" | "anchor" | "tag")
                });
                match next {
                    Some(n) => cur = n,
                    None => return cur,
                }
            }
            _ => return cur,
        }
    }
    cur
}

fn strip_one_quote_pair(raw: &str) -> &str {
    let s = raw.trim();
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && *b.last().unwrap() == b'"')
            || (b[0] == b'\'' && *b.last().unwrap() == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn nonempty_trim(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn push_heading(symbols: &mut Vec<ExtractedSymbol>, name: String, node: Node<'_>) {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    symbols.push(ExtractedSymbol {
        name,
        kind: SymbolKind::Heading,
        start_line,
        end_line: end_line.max(start_line),
        start_byte: node.start_byte() as u32,
        end_byte: node.end_byte() as u32,
        signature: Some("key".into()),
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
