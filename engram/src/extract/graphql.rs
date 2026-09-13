use crate::types::{ExtractedSymbol, Extraction, ParseStatus, SymbolKind};
use tree_sitter::{Node, Parser};

const FILE_MODULE: &str = "<file>";

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
    Extraction {
        status: ParseStatus::Graph,
        symbols: vec![module_symbol(root)],
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
