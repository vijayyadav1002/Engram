pub mod python;

use crate::types::{Extraction, ParseStatus};

/// Dispatch by file extension. Non-Python paths are file-level (no symbols).
pub fn extract_path(rel_posix: &str, source: &str) -> Extraction {
    let lower = rel_posix.to_ascii_lowercase();
    if lower.ends_with(".py") {
        return python::extract(source);
    }
    Extraction {
        status: ParseStatus::File,
        symbols: vec![],
        edges: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, ParseStatus, SymbolKind};

    #[test]
    fn python_functions_classes_imports_and_calls() {
        let src = r#"
import os
from session import create_session

class Auth:
    def login(self):
        return create_session()

def helper():
    return login()
"#;
        let ext = crate::extract::python::extract(src);
        assert_eq!(ext.status, ParseStatus::Graph);
        let names: Vec<_> = ext.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Auth"));
        assert!(names.contains(&"login"));
        assert!(names.contains(&"helper"));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "Auth" && s.kind == SymbolKind::Class));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "login" && s.kind == SymbolKind::Method));
        assert!(ext
            .edges
            .iter()
            .any(|e| e.dst_name == "create_session" && e.kind == EdgeKind::Call));
        assert!(ext.edges.iter().any(|e| e.kind == EdgeKind::Import
            && (e.dst_name == "os"
                || e.dst_name == "session"
                || e.dst_name.contains("create_session"))));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "<file>" && s.kind == SymbolKind::Module));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "helper" && s.kind == SymbolKind::Function));
        assert!(ext.edges.iter().any(|e| e.kind == EdgeKind::Call
            && e.src_name == "login"
            && e.dst_name == "create_session"
            && e.confidence == crate::types::Confidence::Low));
        assert!(ext
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Import && e.src_name == "<file>"));
    }

    #[test]
    fn extract_path_dispatches_python_and_file_fallback() {
        let src = "def helper():\n    pass\n";
        let py = extract_path("pkg/auth.py", src);
        assert_eq!(py.status, ParseStatus::Graph);
        assert!(py
            .symbols
            .iter()
            .any(|s| s.name == "<file>" && s.kind == SymbolKind::Module));
        assert!(py
            .symbols
            .iter()
            .any(|s| s.name == "helper" && s.kind == SymbolKind::Function));

        let other = extract_path("pkg/auth.ts", src);
        assert_eq!(other.status, ParseStatus::File);
        assert!(other.symbols.is_empty());
        assert!(other.edges.is_empty());
    }
}
