use crate::types::{Extraction, ParseStatus};
use tree_sitter::Parser;

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
    Extraction {
        status: ParseStatus::Outline,
        symbols: vec![],
        edges: vec![],
    }
}

fn error_extraction() -> Extraction {
    Extraction {
        status: ParseStatus::Error,
        symbols: vec![],
        edges: vec![],
    }
}
