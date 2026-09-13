pub mod css;
pub mod graphql;
pub mod markdown;
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
    if lower.ends_with(".md") || lower.ends_with(".mdx") {
        return markdown::extract(source, rel_posix);
    }
    if lower.ends_with(".css") || lower.ends_with(".scss") {
        return css::extract(source);
    }
    if lower.ends_with(".graphql") || lower.ends_with(".gql") {
        return graphql::extract(source);
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

    #[test]
    fn extract_path_dispatches_graphql_not_graphqls() {
        let src = "type Reservation { id: ID! }\n";
        let gql = extract_path("schema.graphql", src);
        assert_eq!(gql.status, ParseStatus::Graph);
        assert!(gql
            .symbols
            .iter()
            .any(|s| s.name == "<file>" && s.kind == SymbolKind::Module));

        let short = extract_path("ops.gql", src);
        assert_eq!(short.status, ParseStatus::Graph);

        let sdl = extract_path("schema.graphqls", src);
        assert_eq!(sdl.status, ParseStatus::File);
        assert!(sdl.symbols.is_empty());
    }

    #[test]
    fn graphql_empty_source_is_error() {
        let ext = crate::extract::graphql::extract("");
        assert_eq!(ext.status, ParseStatus::Error);
        assert!(ext.symbols.is_empty());
        assert!(ext.edges.is_empty());
    }

    #[test]
    fn graphql_object_type_and_qualified_fields() {
        let src = "type Reservation {\n  id: ID!\n  name: String\n}\n";
        let ext = crate::extract::graphql::extract(src);
        assert_eq!(ext.status, ParseStatus::Graph);
        assert!(ext.symbols.iter().any(|s| {
            s.name == "Reservation"
                && s.kind == SymbolKind::Type
                && s.signature.as_deref() == Some("type")
        }));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "Reservation.id"
                && s.kind == SymbolKind::Method
                && s.signature.as_deref() == Some("field")
        }));
        assert!(ext.symbols.iter().any(|s| s.name == "Reservation.name"));
        assert!(!ext.symbols.iter().any(|s| s.name == "id"));
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "<file>" && s.kind == SymbolKind::Module));
    }

    #[test]
    fn graphql_sdl_kinds() {
        let src = r#"
interface Node { id: ID! }
enum Status { OPEN CLOSED }
union Vehicle = Car | Bike
input CreateBookingInput { checkIn: String! }
scalar DateTime
"#;
        let ext = crate::extract::graphql::extract(src);
        assert_eq!(ext.status, ParseStatus::Graph);
        assert!(ext.symbols.iter().any(|s| {
            s.name == "Node"
                && s.kind == SymbolKind::Interface
                && s.signature.as_deref() == Some("interface")
        }));
        assert!(ext.symbols.iter().any(|s| s.name == "Node.id"));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "Status"
                && s.kind == SymbolKind::Type
                && s.signature.as_deref() == Some("enum")
        }));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "Status.OPEN"
                && s.kind == SymbolKind::Method
                && s.signature.as_deref() == Some("enum_value")
        }));
        assert!(ext.symbols.iter().any(|s| s.name == "Status.CLOSED"));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "Vehicle"
                && s.kind == SymbolKind::Type
                && s.signature.as_deref() == Some("union")
        }));
        assert!(!ext.symbols.iter().any(|s| s.name.contains("Car")));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "CreateBookingInput"
                && s.kind == SymbolKind::Type
                && s.signature.as_deref() == Some("input")
        }));
        assert!(ext.symbols.iter().any(|s| s.name == "CreateBookingInput.checkIn"));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "DateTime"
                && s.kind == SymbolKind::Type
                && s.signature.as_deref() == Some("scalar")
        }));
    }

    #[test]
    fn graphql_skips_extend_directives_and_arguments() {
        let src = r#"
extend type Reservation { extra: String }
directive @auth on FIELD_DEFINITION
type Query {
  reservation(id: ID!): Reservation
}
"#;
        let ext = crate::extract::graphql::extract(src);
        assert_eq!(ext.status, ParseStatus::Graph);
        assert!(!ext.symbols.iter().any(|s| s.name == "Reservation"));
        assert!(!ext.symbols.iter().any(|s| s.name == "auth" || s.name == "@auth"));
        assert!(ext.symbols.iter().any(|s| s.name == "Query"));
        assert!(ext.symbols.iter().any(|s| s.name == "Query.reservation"));
        assert!(!ext.symbols.iter().any(|s| s.name == "Query.reservation.id"));
        assert!(!ext.symbols.iter().any(|s| s.name == "id" || s.name.ends_with(".id")));
    }

    #[test]
    fn graphql_named_operations_and_fragments() {
        let src = r#"
query GetReservation { reservation { id } }
mutation CreateBooking { createBooking { id } }
subscription OnUpdate { update { id } }
fragment ReservationFields on Reservation { id name }
"#;
        let ext = crate::extract::graphql::extract(src);
        assert_eq!(ext.status, ParseStatus::Graph);
        assert!(ext.symbols.iter().any(|s| {
            s.name == "GetReservation"
                && s.kind == SymbolKind::Function
                && s.signature.as_deref() == Some("query")
        }));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "CreateBooking"
                && s.kind == SymbolKind::Function
                && s.signature.as_deref() == Some("mutation")
        }));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "OnUpdate"
                && s.kind == SymbolKind::Function
                && s.signature.as_deref() == Some("subscription")
        }));
        assert!(ext.symbols.iter().any(|s| {
            s.name == "ReservationFields"
                && s.kind == SymbolKind::Type
                && s.signature.as_deref() == Some("fragment")
        }));
        assert!(!ext.symbols.iter().any(|s| s.name == "reservation"));
        assert!(!ext.symbols.iter().any(|s| s.name == "id"));
    }

    #[test]
    fn graphql_skips_anonymous_operations() {
        let src = "query { reservation { id } }\n{ leftover }\n";
        let ext = crate::extract::graphql::extract(src);
        assert_eq!(ext.status, ParseStatus::Graph);
        assert!(!ext
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Function));
        assert!(!ext.symbols.iter().any(|s| s.name == "reservation"));
    }

    #[test]
    fn graphql_implements_and_fragment_import_edges() {
        let src = r#"
interface Node { id: ID! }
interface Timestamped { updatedAt: String }
type Reservation implements Node & Timestamped { id: ID! }
fragment ReservationFields on Reservation { id }
"#;
        let ext = crate::extract::graphql::extract(src);
        assert!(ext.edges.iter().any(|e| {
            e.src_name == "Reservation"
                && e.dst_name == "Node"
                && e.kind == EdgeKind::Import
                && e.confidence == crate::types::Confidence::High
        }));
        assert!(ext.edges.iter().any(|e| {
            e.src_name == "Reservation" && e.dst_name == "Timestamped"
        }));
        assert!(ext.edges.iter().any(|e| {
            e.src_name == "ReservationFields"
                && e.dst_name == "Reservation"
                && e.kind == EdgeKind::Import
                && e.confidence == crate::types::Confidence::High
        }));
    }

    #[test]
    fn graphql_operation_call_uses_default_root_and_skips_alias() {
        let src = r#"
query GetReservation { hotel: reservation { id } }
query OnlyFrag { ...ReservationFields }
mutation CreateBooking { createBooking { id } }
"#;
        let ext = crate::extract::graphql::extract(src);
        assert!(ext.edges.iter().any(|e| {
            e.src_name == "GetReservation"
                && e.dst_name == "Query.reservation"
                && e.kind == EdgeKind::Call
                && e.confidence == crate::types::Confidence::Low
        }));
        assert!(!ext
            .edges
            .iter()
            .any(|e| e.dst_name == "Query.hotel"));
        assert!(!ext.edges.iter().any(|e| e.src_name == "OnlyFrag"));
        assert!(ext.edges.iter().any(|e| {
            e.src_name == "CreateBooking" && e.dst_name == "Mutation.createBooking"
        }));
    }
}
