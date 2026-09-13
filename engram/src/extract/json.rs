use crate::types::{ExtractedSymbol, Extraction, ParseStatus, SymbolKind};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> Extraction {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_json::LANGUAGE.into())
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
    if root.has_error() && json_root_object(root).is_none() && !is_json_root_array_or_scalar(root) {
        return error_extraction();
    }
    let mut symbols = Vec::new();
    if let Some(obj) = json_root_object(root) {
        emit_object_keys(source, obj, &mut symbols);
    }
    Extraction {
        status: ParseStatus::Outline,
        symbols,
        edges: vec![],
    }
}

fn is_json_root_array_or_scalar(root: Node<'_>) -> bool {
    let mut walk = root.walk();
    let found = root.named_children(&mut walk).any(|c| {
        matches!(
            c.kind(),
            "array" | "string" | "number" | "true" | "false" | "null"
        )
    });
    found
}

fn json_root_object(root: Node<'_>) -> Option<Node<'_>> {
    if root.kind() == "object" {
        return Some(root);
    }
    let mut walk = root.walk();
    let found = root
        .named_children(&mut walk)
        .find(|c| c.kind() == "object" && !c.has_error());
    found
}

fn emit_object_keys(source: &str, obj: Node<'_>, symbols: &mut Vec<ExtractedSymbol>) {
    let mut walk = obj.walk();
    for pair in obj.named_children(&mut walk) {
        if pair.kind() != "pair" || pair.has_error() {
            continue;
        }
        let Some(name) = json_key_text(source, pair) else {
            continue;
        };
        push_heading(symbols, name.clone(), pair);
        if let Some(nested) = json_object_value(pair) {
            let mut inner = nested.walk();
            for nested_pair in nested.named_children(&mut inner) {
                if nested_pair.kind() != "pair" || nested_pair.has_error() {
                    continue;
                }
                let Some(child) = json_key_text(source, nested_pair) else {
                    continue;
                };
                push_heading(symbols, format!("{name}.{child}"), nested_pair);
            }
        }
    }
}

fn json_object_value(pair: Node<'_>) -> Option<Node<'_>> {
    let value = pair.child_by_field_name("value")?;
    if value.kind() == "object" && !value.has_error() {
        Some(value)
    } else {
        None
    }
}

fn json_key_text(source: &str, pair: Node<'_>) -> Option<String> {
    let key = pair.child_by_field_name("key")?;
    if key.has_error() {
        return None;
    }
    let mut walk = key.walk();
    for child in key.named_children(&mut walk) {
        if child.kind() == "string_content" {
            return nonempty_trim(text(source, child));
        }
    }
    nonempty_trim(strip_one_quote_pair(&text(source, key)).to_string())
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
