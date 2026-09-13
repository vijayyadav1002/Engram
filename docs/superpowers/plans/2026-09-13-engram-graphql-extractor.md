# Engram GraphQL Extractor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Index standalone `.graphql` / `.gql` files as a graph language so `get_context "Reservation"` quotes the type span and `get_context "GetReservation"` quotes the named operation span.

**Architecture:** Add `extract/graphql.rs` using `tree-sitter-graphql`, dispatch from `extract_path`, tag `files.language = "graphql"`. Reuse existing `SymbolKind` / `EdgeKind`. No store schema bump, no compiler ranking changes, no new MCP tools.

**Tech Stack:** Existing Engram crate (Rust, rusqlite, tree-sitter 0.25). New dep: `tree-sitter-graphql` 0.2.x. TDD via `cd engram && cargo test …`.

**Spec:** `docs/superpowers/specs/2026-09-13-engram-graphql-extractor-design.md`

## Global Constraints

- Graph language for `.graphql` and `.gql` only. `.graphqls` stays `ParseStatus::File`.
- Parser is tree-sitter via `tree-sitter-graphql` 0.2.x (Engram’s `tree-sitter` 0.25). Not `async-graphql-parser`.
- Reuse existing kinds. No GraphQL-specific `SymbolKind` or `EdgeKind`.
- Field **definitions** are qualified `Parent.name`. Operation/fragment **selections** are not symbols. Bare `id` is not a GraphQL symbol.
- Name lookup is the product. Edges are secondary. Do not change `compile.rs` ranking.
- No YAML/JSON/Bruno/Cucumber extractors. No TS template-literal GraphQL. No cross-file schema resolution. No `extend type` symbols. No directive-definition or argument-name symbols.
- Operation `Call` dest always uses root names `Query` / `Mutation` / `Subscription`.
- Skip rules unchanged (1MB, `node_modules`, secrets, `.engramignore`).
- No changes to `types.rs`, `store.rs`, `compile.rs`, or MCP tool schemas.
- Work in `engram/` except the Core spec one-line language-policy edit. Tests: `cd engram && cargo test …`. TDD on every task. Commit after every task.

## File map

| File | Responsibility |
|---|---|
| `engram/Cargo.toml` | `tree-sitter-graphql` 0.2.x |
| `engram/src/extract/graphql.rs` | parse, query, symbols, edges |
| `engram/src/extract/mod.rs` | `mod graphql`; dispatch `.graphql`/`.gql`; dispatch tests |
| `engram/src/index.rs` | `language_of` → `"graphql"` |
| `engram/src/doctor.rs` | grammar probe includes `graphql` |
| `engram/testdata/miniapp/schema.graphql` | fixture: object type + named query + fragment |
| `engram/testdata/README.md` | mention GraphQL in the fixture list |
| `engram/tests/graphql.rs` | index + `get_context` / `search_symbols` success bar |
| `docs/superpowers/specs/2026-09-07-engram-core-design.md` | add `.graphql` `.gql` to §8 graph row |
| `docs/superpowers/specs/2026-09-13-engram-graphql-extractor-design.md` | status → implemented when tests pass |

Do not add a Rust/YAML/Bruno extractor, new skip kinds, or ranking heuristics.

---

### Task 1: Crate, dispatch, error, language tag

**Files:**
- Modify: `engram/Cargo.toml`
- Create: `engram/src/extract/graphql.rs`
- Modify: `engram/src/extract/mod.rs`
- Modify: `engram/src/index.rs` (`language_of`, `language_strings_match_brief`)

**Interfaces:**
- Consumes: existing `extract_path`, `Extraction`, `ParseStatus`, `language_of`
- Produces:
  - `tree-sitter-graphql` 0.2.x in `[dependencies]`
  - `pub fn extract(source: &str) -> Extraction` in `extract/graphql.rs`
  - `extract_path` dispatches `.graphql` / `.gql` (case-insensitive) to `graphql::extract`
  - `language_of` returns `Some("graphql")` for those two extensions

- [ ] **Step 1: Write the failing tests**

Add to `engram/src/extract/mod.rs` tests (next to `extract_path_dispatches_ts_js`):

```rust
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
```

Add to `engram/src/extract/graphql.rs` under `#[cfg(test)]` (file will not exist yet — create the test module as part of the new file in Step 3; for TDD, put this test in `extract/mod.rs` instead if the module is missing):

In `extract/mod.rs` tests, also add:

```rust
    #[test]
    fn graphql_empty_source_is_error() {
        let ext = crate::extract::graphql::extract("");
        assert_eq!(ext.status, ParseStatus::Error);
        assert!(ext.symbols.is_empty());
        assert!(ext.edges.is_empty());
    }
```

Extend `language_strings_match_brief` in `engram/src/index.rs`:

```rust
        assert_eq!(language_of("schema.graphql"), Some("graphql"));
        assert_eq!(language_of("ops.GQL"), Some("graphql"));
        assert_eq!(language_of("schema.graphqls"), None);
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::extract_path_dispatches_graphql_not_graphqls extract::tests::graphql_empty_source_is_error index::tests::language_strings_match_brief -- --nocapture`

Expected: compile error — `graphql` module missing and/or `language_of` still returns `None` for `.graphql`.

- [ ] **Step 3: Minimal implementation**

Add to `engram/Cargo.toml` dependencies (keep existing tree-sitter pins):

```toml
tree-sitter-graphql = "0.2"
```

Create `engram/src/extract/graphql.rs` that **parses** but does not yet emit type symbols (Task 2). Match Python’s error rule:

```rust
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
```

In `engram/src/extract/mod.rs`, add `pub mod graphql;` and dispatch **after** the CSS arm, **before** the file fallback:

```rust
    if lower.ends_with(".graphql") || lower.ends_with(".gql") {
        return graphql::extract(source);
    }
```

In `language_of` (`engram/src/index.rs`), add before the final `else`:

```rust
    } else if lower.ends_with(".graphql") || lower.ends_with(".gql") {
        Some("graphql")
```

If `cargo test` fails because `tree-sitter-graphql` 0.2 cannot `set_language` on tree-sitter 0.25, stop and pin a 0.2.x version whose `LANGUAGE` loads. Do not swallow grammar-load failure as `File`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib extract::tests::extract_path_dispatches_graphql_not_graphqls extract::tests::graphql_empty_source_is_error index::tests::language_strings_match_brief`

Expected: PASS.

If `graphql_empty_source_is_error` fails because the grammar yields a non-empty error tree for `""`, keep `ParseStatus::Error` for empty input with an explicit `if source.trim().is_empty() { return error_extraction(); }` **only if** empty also has `root.has_error()`. Do not mark valid SDL as `error`.

- [ ] **Step 5: Commit**

```bash
git add engram/Cargo.toml engram/Cargo.lock engram/src/extract/graphql.rs engram/src/extract/mod.rs engram/src/index.rs
git commit -m "feat: dispatch GraphQL files to a tree-sitter extractor"
```

---

### Task 2: Object types and qualified fields

**Files:**
- Modify: `engram/src/extract/graphql.rs`
- Modify: `engram/src/extract/mod.rs` (tests only)

**Interfaces:**
- Consumes: Task 1 `extract(source: &str) -> Extraction`
- Produces: `type Reservation { id: ID! }` → symbols `Reservation` (`Type`, signature `Some("type")`) and `Reservation.id` (`Method`, signature `Some("field")`), plus `<file>` `Module`. Field **selections** are not in scope yet (no operations in this task).

- [ ] **Step 1: Write the failing test** in `engram/src/extract/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engram && cargo test --lib extract::tests::graphql_object_type_and_qualified_fields -- --nocapture`

Expected: FAIL — `Reservation` symbol missing.

- [ ] **Step 3: Minimal implementation**

Replace `extract` in `graphql.rs` with a query-driven extractor. Keep Task 1 error/module behavior. Add these imports and helpers; do **not** emit operations, fragments, or edges yet.

```rust
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
```

If `Query::new` fails, return `error_extraction()`. Do not drop `error_extraction` or `module_symbol`.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib extract::tests::graphql_object_type_and_qualified_fields extract::tests::extract_path_dispatches_graphql_not_graphqls extract::tests::graphql_empty_source_is_error`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract/graphql.rs engram/src/extract/mod.rs
git commit -m "feat: extract GraphQL object types and qualified fields"
```

---

### Task 3: Remaining SDL symbols and skip-list

**Files:**
- Modify: `engram/src/extract/graphql.rs`
- Modify: `engram/src/extract/mod.rs` (tests only)

**Interfaces:**
- Consumes: Task 2 `extract`, `skip_node`, `enclosing_parent_name`, `push_symbol`
- Produces: interface / enum+values / union / input+fields / scalar symbols per spec §7. No symbols for `extend type`, `directive @x`, or field arguments.

- [ ] **Step 1: Write the failing tests** in `engram/src/extract/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::graphql_sdl_kinds extract::tests::graphql_skips_extend_directives_and_arguments -- --nocapture`

Expected: FAIL — `Node` / `Status` missing.

- [ ] **Step 3: Extend the query and match arms**

Replace `QUERY_SRC` with:

```rust
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
"#;
```

In the capture loop, also bind `"input_value"` and `"enum_value"`. After the `field` arm, add:

```rust
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
```

Change the `def` kind mapping from always `Type`/`"type"` to:

```rust
            let (kind, signature) = match node.kind() {
                "object_type_definition" => (SymbolKind::Type, "type"),
                "interface_type_definition" => (SymbolKind::Interface, "interface"),
                "enum_type_definition" => (SymbolKind::Type, "enum"),
                "union_type_definition" => (SymbolKind::Type, "union"),
                "input_object_type_definition" => (SymbolKind::Type, "input"),
                "scalar_type_definition" => (SymbolKind::Type, "scalar"),
                _ => continue,
            };
            push_symbol(&mut symbols, ident, kind, node, Some(signature));
```

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib extract::`

Expected: PASS (including Task 1–2 tests).

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract/graphql.rs engram/src/extract/mod.rs
git commit -m "feat: extract GraphQL interfaces, enums, unions, inputs, scalars"
```

---

### Task 4: Named operations and fragments

**Files:**
- Modify: `engram/src/extract/graphql.rs`
- Modify: `engram/src/extract/mod.rs` (tests only)

**Interfaces:**
- Consumes: Task 3 `extract` / `QUERY_SRC`
- Produces: named `query`/`mutation`/`subscription` → `Function` with signature `query`/`mutation`/`subscription`; named fragment → `Type` signature `fragment`. Anonymous operations and operation **selections** are not symbols. Edges still empty (Task 5).

- [ ] **Step 1: Write the failing tests** in `engram/src/extract/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::graphql_named_operations_and_fragments extract::tests::graphql_skips_anonymous_operations -- --nocapture`

Expected: FAIL — `GetReservation` missing.

- [ ] **Step 3: Add operation and fragment query patterns**

Append to `QUERY_SRC` (keep all Task 3 patterns):

```
(operation_definition (operation_type) @op_type (name) @name) @op
(fragment_definition (fragment_name) @name (type_condition (named_type) @frag_on)) @frag
```

Bind captures `"op"`, `"frag"`, `"op_type"`, `"frag_on"`. After enum-value handling, add:

```rust
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
                ident,
                SymbolKind::Function,
                node,
                sig.as_deref(),
            );
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
                ident,
                SymbolKind::Type,
                node,
                Some("fragment"),
            );
        }
```

Do not emit edges yet. Do not create symbols from `selection_set` / `field` under operations — those nodes are `field`, not `field_definition`, so the existing field query must not match them. If the test sees extra `id` symbols, the field query is too broad; keep it as `(field_definition (name) @name) @field` only.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib extract::`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract/graphql.rs engram/src/extract/mod.rs
git commit -m "feat: extract named GraphQL operations and fragments"
```

---

### Task 5: Edges

**Files:**
- Modify: `engram/src/extract/graphql.rs`
- Modify: `engram/src/extract/mod.rs` (tests only)

**Interfaces:**
- Consumes: Task 4 symbols; `Confidence`, `EdgeKind` (add to `use crate::types::{...}` if missing)
- Produces:
  - `implements A & B` → two `Import` edges, `Confidence::High`, `src_name` = type/interface name
  - fragment → `Import` onto type condition, `High`
  - named operation → `Call` to `{Query|Mutation|Subscription}.{first_root_field}`, `Low`
  - first root field skips aliases; if the top-level selection has no `field` (only fragment spreads), omit the `Call`

- [ ] **Step 1: Write the failing tests** in `engram/src/extract/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::graphql_implements_and_fragment_import_edges extract::tests::graphql_operation_call_uses_default_root_and_skips_alias -- --nocapture`

Expected: FAIL — `edges` empty.

- [ ] **Step 3: Emit edges**

Change `extract` to `let mut edges = Vec::new();` and return them.

After pushing an object/interface `def` symbol, call `implemented_names` and push `Import` edges.

When handling `frag`, if `frag_on` is present, push `Import` `{fragment} → {type}`.

When handling `op`, after `push_symbol`, if `let Some(dst) = first_root_call_dest(source, node)` then push `Call`.

Add helpers:

```rust
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
        if child.kind() != "field" {
            continue;
        }
        if let Some(field_ident) = field_response_name(source, child) {
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
```

`field_response_name` must skip `alias` so `hotel: reservation` yields `reservation`, not `hotel`.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib extract::`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract/graphql.rs engram/src/extract/mod.rs
git commit -m "feat: add GraphQL implements, fragment, and operation edges"
```

---

### Task 6: Doctor, fixture, compiler success bar, docs

**Files:**
- Modify: `engram/src/doctor.rs` (`grammar_status` list + test)
- Create: `engram/testdata/miniapp/schema.graphql`
- Modify: `engram/testdata/README.md`
- Create: `engram/tests/graphql.rs`
- Modify: `docs/superpowers/specs/2026-09-07-engram-core-design.md` (§8 graph row)
- Modify: `docs/superpowers/specs/2026-09-13-engram-graphql-extractor-design.md` (Status)

**Interfaces:**
- Consumes: Task 1–5 extractor; `init::run_init`, `index::index_repo`, `compile::{get_context, search_symbols}`
- Produces:
  - doctor grammars line includes `graphql` among ok grammars
  - fixture `engram/testdata/miniapp/schema.graphql`
  - after indexing a copy of miniapp: `get_context "Reservation"` and `get_context "GetReservation"` quote the fixture; `search_symbols("Reservation.id")` hits; `search_symbols("id")` does not return a GraphQL field named `id`

- [ ] **Step 1: Write the failing tests**

In `engram/src/doctor.rs` tests:

```rust
    #[test]
    fn grammars_line_includes_graphql() {
        let root = tempfile_dir();
        crate::init::run_init(&root).unwrap();
        let out = run_doctor(&root).unwrap();
        let grammars = out
            .lines()
            .find(|l| l.starts_with("grammars:"))
            .unwrap_or(&out);
        assert!(
            grammars.contains("graphql") && grammars.contains("ok"),
            "doctor grammars: {grammars}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
```

Create `engram/tests/graphql.rs`:

```rust
use engram::compile::{get_context, search_symbols};
use engram::index::index_repo;
use engram::init::run_init;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "engram-graphql-{}-{}",
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn indexed_miniapp() -> PathBuf {
    let dir = tmp();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/miniapp");
    copy_dir(&src, &dir);
    run_init(&dir).unwrap();
    index_repo(&dir, true).unwrap();
    dir
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn get_context_quotes_graphql_type_and_operation() {
    let root = indexed_miniapp();
    let types = get_context(&root, "Reservation", 3000).unwrap();
    assert!(
        types.items.iter().any(|i| {
            i.path.ends_with("schema.graphql")
                && i.text.contains("type Reservation")
        }),
        "Reservation package: {:?}",
        types.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let ops = get_context(&root, "GetReservation", 3000).unwrap();
    assert!(
        ops.items.iter().any(|i| {
            i.path.ends_with("schema.graphql")
                && i.text.contains("GetReservation")
        }),
        "GetReservation package: {:?}",
        ops.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let fields = search_symbols(&root, "Reservation.id", 20).unwrap();
    assert!(
        fields.iter().any(|h| h.name == "Reservation.id"),
        "{fields:?}"
    );
    let bare = search_symbols(&root, "id", 50).unwrap();
    assert!(
        !bare.iter().any(|h| h.path.ends_with("schema.graphql") && h.name == "id"),
        "bare id should not be a GraphQL field symbol: {bare:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
```

Write `engram/testdata/miniapp/schema.graphql` **in this step** so the integration test has a file to copy (the test will still fail until doctor/dispatch are fully wired if doctor test is the one that fails first; the get_context test fails until the fixture exists **and** Task 1–5 landed):

```graphql
type Reservation {
  id: ID!
  name: String
}

query GetReservation {
  reservation {
    id
    ...ReservationFields
  }
}

fragment ReservationFields on Reservation {
  name
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib doctor::tests::grammars_line_includes_graphql -- --nocapture`

Expected: FAIL — grammars line is `python,ts,tsx,js ok` without `graphql`.

Then: `cd engram && cargo test --test graphql -- --nocapture`

Expected: FAIL until the fixture exists (Step 1 creates it) and/or `get_context` cannot find `Reservation` if miniapp copy is missing `schema.graphql`. After the fixture file is on disk, this test should **pass** if Tasks 1–5 are done — if it passes immediately, still complete doctor + docs in Step 3. If it fails, fix extractor regressions before continuing.

- [ ] **Step 3: Implementation**

In `engram/src/doctor.rs` `grammar_status`, append to `checks`:

```rust
        (
            "graphql",
            tree_sitter::Language::from(tree_sitter_graphql::LANGUAGE),
        ),
```

Update `engram/testdata/README.md` to:

```markdown
# Testdata fixtures

`miniapp/` is a tiny in-tree fixture for indexer and compiler tests: TSX,
TypeScript, Python, CSS, Markdown, GraphQL, and a `.env` that must not be indexed.
Do not treat it as a real application.
```

In `docs/superpowers/specs/2026-09-07-engram-core-design.md` §8, change the graph extensions cell from:

`.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` `.py`

to:

`.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` `.py` `.graphql` `.gql`

Do not rewrite the rest of Core.

In `docs/superpowers/specs/2026-09-13-engram-graphql-extractor-design.md`, set `Status: implemented on main`.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib doctor::tests::grammars_line_includes_graphql && cargo test --test graphql && cargo test --lib extract::`

Expected: PASS.

Then run the full crate tests:

Run: `cd engram && cargo test`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engram/src/doctor.rs engram/testdata/miniapp/schema.graphql engram/testdata/README.md engram/tests/graphql.rs docs/superpowers/specs/2026-09-07-engram-core-design.md docs/superpowers/specs/2026-09-13-engram-graphql-extractor-design.md
git commit -m "feat: index GraphQL fixtures and probe the graphql grammar"
```

---

## Self-review

**Spec coverage**

| Spec section | Task |
|---|---|
| §4.1–2 graph language, tree-sitter-graphql 0.2.x | 1 |
| §6.2 dispatch `.graphql`/`.gql`; `.graphqls` file-only | 1 |
| §6.3 `language=graphql` | 1 |
| §9 empty/error | 1 |
| §7 object type + qualified fields | 2 |
| §7 interface/enum/union/input/scalar; skip extend/directive/args | 3 |
| §7 named operations/fragments; skip anonymous + selections | 4 |
| §8 implements / fragment Import / operation Call | 5 |
| §6.4 doctor probe | 6 |
| §10–12 fixture + `get_context` / `search_symbols` success bar | 6 |
| §11 Core §8 language-policy row | 6 |
| Non-goals (YAML, TS embedding, ranking, new kinds) | global constraints; no task |

**Placeholder scan:** no TBD/TODO; tests and implementation are inlined.

**Type consistency:** `pub fn extract(source: &str) -> Extraction` is unchanged across tasks. Qualified field names are always `{Parent}.{name}`. Edge kinds stay `Import`/`Call`.
