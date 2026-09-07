pub mod python;
pub mod ts;

use crate::types::{Extraction, ParseStatus};

/// Dispatch by file extension. Unknown paths are file-level (no symbols).
pub fn extract_path(rel_posix: &str, source: &str) -> Extraction {
    let lower = rel_posix.to_ascii_lowercase();
    if lower.ends_with(".py") {
        return python::extract(source);
    }
    if lower.ends_with(".tsx") || lower.ends_with(".jsx") {
        return ts::extract(source, ts::TsLang::Tsx);
    }
    if lower.ends_with(".ts") {
        return ts::extract(source, ts::TsLang::Typescript);
    }
    if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs") {
        return ts::extract(source, ts::TsLang::Javascript);
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

        let other = extract_path("pkg/auth.rs", src);
        assert_eq!(other.status, ParseStatus::File);
        assert!(other.symbols.is_empty());
        assert!(other.edges.is_empty());
    }

    #[test]
    fn extract_path_dispatches_ts_js() {
        let ts = extract_path(
            "pkg/auth.ts",
            "export interface User { id: string }\nexport class AuthService { login() {} }\n",
        );
        assert_eq!(ts.status, ParseStatus::Graph);
        assert!(ts
            .symbols
            .iter()
            .any(|s| s.name == "<file>" && s.kind == SymbolKind::Module));
        assert!(ts
            .symbols
            .iter()
            .any(|s| s.name == "User" && s.kind == SymbolKind::Interface));
        assert!(ts
            .symbols
            .iter()
            .any(|s| s.name == "AuthService" && s.kind == SymbolKind::Class));

        let tsx = extract_path(
            "pkg/LoginBanner.tsx",
            "export function LoginBanner() { return 1; }\n",
        );
        assert_eq!(tsx.status, ParseStatus::Graph);
        assert!(tsx
            .symbols
            .iter()
            .any(|s| s.name == "LoginBanner" && s.kind == SymbolKind::Component));

        let jsx = extract_path("pkg/App.jsx", "export const App = () => <div/>;\n");
        assert_eq!(jsx.status, ParseStatus::Graph);
        assert!(jsx
            .symbols
            .iter()
            .any(|s| s.name == "App" && s.kind == SymbolKind::Component));

        let js = extract_path("pkg/util.js", "export function helper() { return 1; }\n");
        assert_eq!(js.status, ParseStatus::Graph);
        assert!(js
            .symbols
            .iter()
            .any(|s| s.name == "helper" && s.kind == SymbolKind::Function));

        let mjs = extract_path("pkg/util.mjs", "export function helper() { return 1; }\n");
        assert_eq!(mjs.status, ParseStatus::Graph);
        let cjs = extract_path("pkg/util.cjs", "function helper() { return 1; }\n");
        assert_eq!(cjs.status, ParseStatus::Graph);
        assert!(cjs
            .symbols
            .iter()
            .any(|s| s.name == "helper" && s.kind == SymbolKind::Function));
    }
}
