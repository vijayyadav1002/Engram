# Engram JSON / YAML Outline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Index `.json` / `.yaml` / `.yml` as outline files so `get_context "scripts"` quotes the top-level pair and `get_context "scripts.test"` quotes the nested pair.

**Architecture:** Add `extract/json.rs` and `extract/yaml.rs` using `tree-sitter-json` and `tree-sitter-yaml`. Walk document mappings two object levels deep, emit `Heading` symbols with pair spans, dispatch from `extract_path`, tag `files.language`. No edges, no new kinds, no ranking changes.

**Tech Stack:** Existing Engram crate (Rust, clap, rusqlite, tree-sitter 0.25). New deps: `tree-sitter-json` 0.24.x, `tree-sitter-yaml` 0.7.x. TDD via `cd engram && cargo test …`.

**Spec:** `docs/superpowers/specs/2026-09-13-engram-json-yaml-outline-design.md`

## Global Constraints

- Outline language for `.json`, `.yaml`, and `.yml` only. `.jsonc` stays `ParseStatus::File`.
- Parsers are tree-sitter: `tree-sitter-json` 0.24.x and `tree-sitter-yaml` 0.7.x (`LANGUAGE` as `LanguageFn`). Not `serde_json` value dumps, not `serde_yaml` / `serde-saphyr`.
- Reuse `SymbolKind::Heading`. No JSON/YAML-specific `SymbolKind` or `EdgeKind`.
- Names: top-level `scripts`; nested object `scripts.test`. Join with `.`. Do not emit a bare nested name `test`.
- Arrays do not contribute names. A mapping whose value is an array still emits the parent key and stops.
- Span is the pair / mapping entry (key through value). Signature is `Some("key")`.
- No edges. Do not emit a `<file>` `Module` symbol.
- Same rules for every file. No filename special cases.
- Do not change `compile.rs` ranking, `types.rs`, `store.rs`, or MCP tool schemas.
- Skip rules unchanged (1MB, lockfiles, secrets, `.engramignore`).
- Work in `engram/` except the Core spec outline-row edit and README language table. Tests: `cd engram && cargo test …`. TDD on every task. Commit after every task.

## File map

| File | Responsibility |
|---|---|
| `engram/Cargo.toml` | `tree-sitter-json` 0.24.x, `tree-sitter-yaml` 0.7.x |
| `engram/src/extract/json.rs` | JSON walk, heading symbols |
| `engram/src/extract/yaml.rs` | YAML walk, heading symbols |
| `engram/src/extract/mod.rs` | `mod json` / `mod yaml`; dispatch; unit tests |
| `engram/src/index.rs` | `language_of` → `"json"` / `"yaml"` |
| `engram/src/doctor.rs` | grammar probe includes `json` and `yaml` |
| `engram/testdata/miniapp/package.json` | fixture: `name` + `scripts.test` |
| `engram/testdata/miniapp/values.yaml` | fixture: `services.web` |
| `engram/testdata/README.md` | mention JSON/YAML in the fixture list |
| `engram/tests/json_yaml.rs` | index + `get_context` / `search_symbols` success bar |
| `docs/superpowers/specs/2026-09-07-engram-core-design.md` | add `.json` `.yaml` `.yml` to §8 outline row |
| `README.md` | add those extensions to “What it understands” outline row |
| `docs/superpowers/specs/2026-09-13-engram-json-yaml-outline-design.md` | status → implemented when tests pass |

Do not add Bruno/Cucumber extractors, `.jsonc` dispatch, array indices, depth > 2, or ranking heuristics.

---

### Task 1: Crates, dispatch, error, language tags

**Files:**
- Modify: `engram/Cargo.toml`
- Create: `engram/src/extract/json.rs`
- Create: `engram/src/extract/yaml.rs`
- Modify: `engram/src/extract/mod.rs`
- Modify: `engram/src/index.rs` (`language_of`, `language_strings_match_brief`)

**Interfaces:**
- Consumes: existing `extract_path`, `Extraction`, `ParseStatus`, `language_of`
- Produces:
  - `tree-sitter-json = "0.24"` and `tree-sitter-yaml = "0.7"` in `[dependencies]`
  - `pub fn extract(source: &str) -> Extraction` in `extract/json.rs` and `extract/yaml.rs`
  - `extract_path` dispatches `.json` → json, `.yaml`/`.yml` → yaml (case-insensitive)
  - `.jsonc` remains `ParseStatus::File`
  - `language_of` returns `Some("json")` for `.json` and `Some("yaml")` for `.yaml`/`.yml`

- [ ] **Step 1: Write the failing tests**

Add to `engram/src/extract/mod.rs` tests (next to `extract_path_dispatches_graphql_not_graphqls`):

```rust
    #[test]
    fn extract_path_dispatches_json_yaml_not_jsonc() {
        let json = extract_path("package.json", "{}\n");
        assert_eq!(json.status, ParseStatus::Outline);
        assert!(json.symbols.is_empty());
        assert!(json.edges.is_empty());
        assert!(json.symbols.iter().all(|s| s.name != "<file>"));

        let yaml = extract_path("values.yaml", "{}\n");
        assert_eq!(yaml.status, ParseStatus::Outline);
        assert!(yaml.edges.is_empty());

        let yml = extract_path("app.YML", "{}\n");
        assert_eq!(yml.status, ParseStatus::Outline);

        let jsonc = extract_path("tsconfig.jsonc", "{}\n");
        assert_eq!(jsonc.status, ParseStatus::File);
        assert!(jsonc.symbols.is_empty());
    }

    #[test]
    fn json_yaml_empty_source_is_error() {
        let json = crate::extract::json::extract("");
        assert_eq!(json.status, ParseStatus::Error);
        assert!(json.symbols.is_empty());
        assert!(json.edges.is_empty());

        let yaml = crate::extract::yaml::extract("");
        assert_eq!(yaml.status, ParseStatus::Error);
        assert!(yaml.symbols.is_empty());
        assert!(yaml.edges.is_empty());
    }
```

YAML `{}\n` is a flow mapping; it must parse as `Outline` even before key walking exists. Do not use `a: 1` in this dispatch test (that would hide a missing walk later).

Extend `language_strings_match_brief` in `engram/src/index.rs` (keep existing asserts):

```rust
        assert_eq!(language_of("package.json"), Some("json"));
        assert_eq!(language_of("values.yaml"), Some("yaml"));
        assert_eq!(language_of("app.YML"), Some("yaml"));
        assert_eq!(language_of("tsconfig.jsonc"), None);
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::extract_path_dispatches_json_yaml_not_jsonc extract::tests::json_yaml_empty_source_is_error index::tests::language_strings_match_brief -- --nocapture`

Expected: compile error — `json` / `yaml` modules missing and/or `language_of` still returns `None` for `.json`.

- [ ] **Step 3: Minimal implementation**

Add to `engram/Cargo.toml` `[dependencies]` (keep existing tree-sitter pins):

```toml
tree-sitter-json = "0.24"
tree-sitter-yaml = "0.7"
```

Create `engram/src/extract/json.rs` that **parses** but does not yet emit key symbols (Task 2):

```rust
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
```

Create `engram/src/extract/yaml.rs` with the same shape, using `tree_sitter_yaml::LANGUAGE`.

In `engram/src/extract/mod.rs`, add `pub mod json;` and `pub mod yaml;` next to the other extract modules. Dispatch **after** the CSS arm and **before** GraphQL (or after GraphQL — either is fine as long as it is before the `File` fallback):

```rust
    if lower.ends_with(".json") {
        return json::extract(source);
    }
    if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        return yaml::extract(source);
    }
```

Do **not** match `.jsonc` here. `ends_with(".json")` does not match `.jsonc`.

In `language_of` (`engram/src/index.rs`), add before the final `else`:

```rust
    } else if lower.ends_with(".json") {
        Some("json")
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        Some("yaml")
```

If `cargo test` fails because a crate cannot `set_language` on tree-sitter 0.25, stop and pin a 0.24.x / 0.7.x version whose `LANGUAGE` loads. Do not swallow grammar-load failure as `File`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib extract::tests::extract_path_dispatches_json_yaml_not_jsonc extract::tests::json_yaml_empty_source_is_error index::tests::language_strings_match_brief`

Expected: PASS.

If empty input is a non-empty error tree, keep `ParseStatus::Error` for `source.trim().is_empty()`. Do not mark `{}` as `error`.

- [ ] **Step 5: Commit**

```bash
git add engram/Cargo.toml engram/Cargo.lock engram/src/extract/json.rs engram/src/extract/yaml.rs engram/src/extract/mod.rs engram/src/index.rs
git commit -m "feat: dispatch JSON and YAML files to tree-sitter outline extractors"
```

---

### Task 2: JSON two-level headings

**Files:**
- Modify: `engram/src/extract/json.rs`
- Modify: `engram/src/extract/mod.rs` (tests only)

**Interfaces:**
- Consumes: Task 1 `json::extract(source: &str) -> Extraction`
- Produces: `{ "scripts": { "test": "vitest" }, "name": "app" }` → `Heading` symbols `scripts`, `scripts.test`, `name` with `signature == Some("key")`, pair spans, no edges, no `<file>` module. Nested object depth stops at two. Array values emit the parent key only.

- [ ] **Step 1: Write the failing tests** in `engram/src/extract/mod.rs`:

```rust
    fn heading_names(ext: &Extraction) -> Vec<&str> {
        ext.symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Heading)
            .map(|s| s.name.as_str())
            .collect()
    }

    #[test]
    fn json_two_level_object_keys() {
        let src = r#"{ "scripts": { "test": "vitest" }, "name": "app" }"#;
        let ext = crate::extract::json::extract(src);
        assert_eq!(ext.status, ParseStatus::Outline);
        assert!(ext.edges.is_empty());
        let names = heading_names(&ext);
        assert!(names.contains(&"scripts"), "{names:?}");
        assert!(names.contains(&"scripts.test"), "{names:?}");
        assert!(names.contains(&"name"), "{names:?}");
        assert!(!names.contains(&"test"), "bare nested key must not be a symbol: {names:?}");
        assert!(ext.symbols.iter().all(|s| s.name != "<file>"));
        assert!(ext.symbols.iter().all(|s| s.kind == SymbolKind::Heading));
        assert!(ext
            .symbols
            .iter()
            .all(|s| s.signature.as_deref() == Some("key")));
        let scripts = ext.symbols.iter().find(|s| s.name == "scripts").unwrap();
        let slice = &src[scripts.start_byte as usize..scripts.end_byte as usize];
        assert!(slice.contains("\"test\""), "span must be the pair, got {slice:?}");
    }

    #[test]
    fn json_nested_mapping_and_array_and_root() {
        let deps = crate::extract::json::extract(r#"{ "dependencies": { "react": "18" } }"#);
        let names = heading_names(&deps);
        assert!(names.contains(&"dependencies") && names.contains(&"dependencies.react"), "{names:?}");

        let items = crate::extract::json::extract(r#"{ "items": [ { "id": 1 } ] }"#);
        let names = heading_names(&items);
        assert_eq!(names, vec!["items"]);

        let deep = crate::extract::json::extract(r#"{ "a": { "b": { "c": 1 } } }"#);
        let names = heading_names(&deep);
        assert!(names.contains(&"a") && names.contains(&"a.b"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("a.b.c")), "{names:?}");

        let arr = crate::extract::json::extract("[ { \"a\": 1 } ]");
        assert_eq!(arr.status, ParseStatus::Outline);
        assert!(heading_names(&arr).is_empty());

        let scalar = crate::extract::json::extract("\"hello\"\n");
        assert_eq!(scalar.status, ParseStatus::Outline);
        assert!(heading_names(&scalar).is_empty());
    }

    #[test]
    fn json_junk_comments_quotes_duplicates() {
        let junk = crate::extract::json::extract("{");
        assert_eq!(junk.status, ParseStatus::Error);
        assert!(junk.symbols.is_empty());

        let commented = crate::extract::json::extract("{\n  // keep\n  \"name\": \"app\"\n}\n");
        assert_eq!(commented.status, ParseStatus::Outline);
        assert!(heading_names(&commented).contains(&"name"));

        let quoted = crate::extract::json::extract("{ \"scripts\": 1 }\n");
        assert!(heading_names(&quoted).contains(&"scripts"));
        assert!(!heading_names(&quoted).iter().any(|n| n.contains('"')));

        let dup = crate::extract::json::extract("{ \"a\": 1, \"a\": 2 }\n");
        let count = heading_names(&dup).iter().filter(|n| **n == "a").count();
        assert_eq!(count, 2);
    }
```

`heading_names` is a test helper in the same `mod tests`. If a later YAML test needs it, reuse this function — do not duplicate the helper.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::json_two_level_object_keys extract::tests::json_nested_mapping_and_array_and_root extract::tests::json_junk_comments_quotes_duplicates -- --nocapture`

Expected: FAIL — `{}` parse works but heading names are missing (or junk `{` is still `Outline`).

- [ ] **Step 3: Minimal implementation** in `engram/src/extract/json.rs`

Replace the stub with a two-level walk. Keep `error_extraction` from Task 1.

```rust
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
    root.named_children(&mut walk).any(|c| {
        matches!(
            c.kind(),
            "array" | "string" | "number" | "true" | "false" | "null"
        )
    })
}

fn json_root_object(root: Node<'_>) -> Option<Node<'_>> {
    if root.kind() == "object" {
        return Some(root);
    }
    let mut walk = root.walk();
    root.named_children(&mut walk)
        .find(|c| c.kind() == "object" && !c.has_error())
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
    if b.len() >= 2 && ((b[0] == b'"' && *b.last().unwrap() == b'"') || (b[0] == b'\'' && *b.last().unwrap() == b'\''))
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
```

Junk `{`: after parse, `root.has_error()` is true and there is no root object and no array/scalar → `Error`. Do not invent keys from a broken tree.

`//` comments: `tree-sitter-json` has `comment` nodes; skip them by only walking `pair` children.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib extract::tests::json_two_level_object_keys extract::tests::json_nested_mapping_and_array_and_root extract::tests::json_junk_comments_quotes_duplicates extract::tests::extract_path_dispatches_json_yaml_not_jsonc`

Expected: PASS.

If the comment fixture is `Error` because this grammar build rejects `//`, change the fixture to a `/* keep */` comment (the grammar’s `comment` node). Do not strip comments with a regex before parse. If **both** comment forms error, keep the test asserting `Error` for that source and add a note in the test — do not invent keys.

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract/json.rs engram/src/extract/mod.rs
git commit -m "feat: extract JSON object keys as two-level headings"
```

---

### Task 3: YAML two-level headings

**Files:**
- Modify: `engram/src/extract/yaml.rs`
- Modify: `engram/src/extract/mod.rs` (tests only)

**Interfaces:**
- Consumes: Task 1 `yaml::extract(source: &str) -> Extraction`; Task 2 heading shape (`Heading`, signature `"key"`, pair span, no `<file>`)
- Produces: block mapping `services.web`, flow mapping `a.b`, multi-doc keys from each document, sequence values emit parent only, `<<` and alias keys skipped

- [ ] **Step 1: Write the failing tests** in `engram/src/extract/mod.rs` (reuse `heading_names`):

```rust
    #[test]
    fn yaml_block_flow_sequence_and_multidoc() {
        let block = crate::extract::yaml::extract("services:\n  web:\n    port: 80\n");
        assert_eq!(block.status, ParseStatus::Outline);
        assert!(block.edges.is_empty());
        let names = heading_names(&block);
        assert!(names.contains(&"services"), "{names:?}");
        assert!(names.contains(&"services.web"), "{names:?}");
        assert!(!names.contains(&"web"), "{names:?}");
        assert!(!names.iter().any(|n| *n == "services.web.port" || *n == "port"), "{names:?}");
        let svc = block.symbols.iter().find(|s| s.name == "services").unwrap();
        assert_eq!(svc.kind, SymbolKind::Heading);
        assert_eq!(svc.signature.as_deref(), Some("key"));

        let flow = crate::extract::yaml::extract("{a: {b: 1}}\n");
        let names = heading_names(&flow);
        assert!(names.contains(&"a") && names.contains(&"a.b"), "{names:?}");

        let seq = crate::extract::yaml::extract("items:\n  - id: 1\n");
        let names = heading_names(&seq);
        assert_eq!(names, vec!["items"]);

        let multi = crate::extract::yaml::extract("---\na: 1\n---\nb:\n  c: 2\n");
        let names = heading_names(&multi);
        assert!(names.contains(&"a"), "{names:?}");
        assert!(names.contains(&"b"), "{names:?}");
        assert!(names.contains(&"b.c"), "{names:?}");
    }

    #[test]
    fn yaml_skip_merge_alias_strip_quotes() {
        let src = "x: &anchor\n  k: 1\ny:\n  <<: *anchor\n  z: 2\n*anchor: 3\n";
        let ext = crate::extract::yaml::extract(src);
        assert_eq!(ext.status, ParseStatus::Outline);
        let names = heading_names(&ext);
        assert!(names.contains(&"x"), "{names:?}");
        assert!(names.contains(&"x.k"), "{names:?}");
        assert!(names.contains(&"y"), "{names:?}");
        assert!(names.contains(&"y.z"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("<<")), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with('*')), "{names:?}");

        let quoted = crate::extract::yaml::extract("\"web\": 1\n'svc': 2\n");
        let names = heading_names(&quoted);
        assert!(names.contains(&"web"), "{names:?}");
        assert!(names.contains(&"svc"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains('"') || n.contains('\'')), "{names:?}");
    }
```

If `*anchor: 3` as a document-level alias key is unparseable in this grammar, drop that line from the fixture and still assert no name starts with `*`. Do not emit alias names.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::tests::yaml_block_flow_sequence_and_multidoc extract::tests::yaml_skip_merge_alias_strip_quotes -- --nocapture`

Expected: FAIL — YAML parse returns `Outline` with no heading names.

- [ ] **Step 3: Minimal implementation** in `engram/src/extract/yaml.rs`

```rust
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
```

`mapping_of` must **not** recurse into `block_mapping_pair` / `flow_pair` / sequences, or a nested mapping would be treated as the document root.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib extract::tests::yaml_block_flow_sequence_and_multidoc extract::tests::yaml_skip_merge_alias_strip_quotes extract::tests::extract_path_dispatches_json_yaml_not_jsonc extract::tests::json_two_level_object_keys`

Expected: PASS.

If YAML `{}\n` from Task 1 now emits no symbols (still `Outline`), that is correct. If multi-doc `---` documents need `mapping_of(root)` because the grammar wraps them without `document` nodes, the fallback in `extract` covers it — do not walk nested pairs as top-level.

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract/yaml.rs engram/src/extract/mod.rs
git commit -m "feat: extract YAML mapping keys as two-level headings"
```

---

### Task 4: Doctor, fixtures, compiler success bar, docs

**Files:**
- Modify: `engram/src/doctor.rs` (`grammar_status` list + test)
- Create: `engram/testdata/miniapp/package.json`
- Create: `engram/testdata/miniapp/values.yaml`
- Modify: `engram/testdata/README.md`
- Create: `engram/tests/json_yaml.rs`
- Modify: `docs/superpowers/specs/2026-09-07-engram-core-design.md` (§8 outline row)
- Modify: `README.md` (“What it understands” outline row)
- Modify: `docs/superpowers/specs/2026-09-13-engram-json-yaml-outline-design.md` (Status)

**Interfaces:**
- Consumes: Task 1–3 extractors; `init::run_init`, `index::index_repo`, `compile::{get_context, search_symbols}`
- Produces:
  - doctor grammars line includes `json` and `yaml` among ok grammars
  - after indexing a copy of miniapp: `get_context "scripts"` / `"scripts.test"` quote `package.json`; `get_context "services"` quotes `values.yaml`; `search_symbols("scripts.test")` hits; `search_symbols("test")` does not return a JSON/YAML heading named `test`

- [ ] **Step 1: Write the failing tests**

In `engram/src/doctor.rs` tests, next to `grammars_line_includes_graphql`:

```rust
    #[test]
    fn grammars_line_includes_json_and_yaml() {
        let root = tempfile_dir();
        crate::init::run_init(&root).unwrap();
        let out = run_doctor(&root).unwrap();
        let grammars = out
            .lines()
            .find(|l| l.starts_with("grammars:"))
            .unwrap_or(&out);
        assert!(
            grammars.contains("json") && grammars.contains("yaml") && grammars.contains("ok"),
            "doctor grammars: {grammars}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
```

Write `engram/testdata/miniapp/package.json` **in this step** so the integration test has a file to copy:

```json
{
  "name": "miniapp",
  "scripts": {
    "test": "echo ok"
  }
}
```

Write `engram/testdata/miniapp/values.yaml`:

```yaml
services:
  web:
    port: 80
```

Create `engram/tests/json_yaml.rs` (copy the `tmp` / `copy_dir` / `indexed_miniapp` helpers from `engram/tests/graphql.rs`, using prefix `engram-json-yaml-`):

```rust
use engram::compile::{get_context, search_symbols};
use engram::index::index_repo;
use engram::init::run_init;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("engram-json-yaml-{}-{}", std::process::id(), n));
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
fn get_context_quotes_json_and_yaml_keys() {
    let root = indexed_miniapp();
    let scripts = get_context(&root, "scripts", 3000).unwrap();
    assert!(
        scripts.items.iter().any(|i| {
            i.path.ends_with("package.json") && i.text.contains("\"test\"")
        }),
        "scripts package: {:?}",
        scripts.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let nested = get_context(&root, "scripts.test", 3000).unwrap();
    assert!(
        nested.items.iter().any(|i| {
            i.path.ends_with("package.json") && i.text.contains("echo ok")
        }),
        "scripts.test package: {:?}",
        nested.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let services = get_context(&root, "services", 3000).unwrap();
    assert!(
        services.items.iter().any(|i| {
            i.path.ends_with("values.yaml") && i.text.contains("web")
        }),
        "services package: {:?}",
        services.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let fields = search_symbols(&root, "scripts.test", 20).unwrap();
    assert!(
        fields.iter().any(|h| h.name == "scripts.test"),
        "{fields:?}"
    );
    let bare = search_symbols(&root, "test", 50).unwrap();
    assert!(
        !bare.iter().any(|h| {
            (h.path.ends_with("package.json") || h.path.ends_with("values.yaml"))
                && h.name == "test"
        }),
        "bare test must not be a JSON/YAML heading: {bare:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib doctor::tests::grammars_line_includes_json_and_yaml --test json_yaml -- --nocapture`

Expected: doctor test fails (`json`/`yaml` missing from the grammars line). `json_yaml` fails because the fixture is missing or symbols are not indexed — after Step 1 the fixtures exist, so failure is doctor probe and/or language tag not enough for `get_context` if Task 2–3 were skipped (they must not be).

- [ ] **Step 3: Implementation**

In `engram/src/doctor.rs` `grammar_status`, append after the graphql tuple:

```rust
        (
            "json",
            tree_sitter::Language::from(tree_sitter_json::LANGUAGE),
        ),
        (
            "yaml",
            tree_sitter::Language::from(tree_sitter_yaml::LANGUAGE),
        ),
```

`doctor.rs` already has `use tree_sitter::Parser;`. No new doctor section.

In `engram/testdata/README.md`, change the fixture sentence to mention JSON and YAML:

```markdown
`miniapp/` is a tiny in-tree fixture for indexer and compiler tests: TSX,
TypeScript, Python, CSS, Markdown, GraphQL, JSON, YAML, and a `.env` that must not be indexed.
Do not treat it as a real application.
```

In `docs/superpowers/specs/2026-09-07-engram-core-design.md` §8, change the outline row only:

```markdown
| Outline + FTS | `.md` `.mdx` `.css` `.scss` `.json` `.yaml` `.yml` | headings (`kind=heading`), selectors (`kind=selector`), or two-level keys (`kind=heading`); no edges |
```

In `README.md` “What it understands”:

```markdown
| Outline + search | `.md` `.mdx` `.css` `.scss` `.json` `.yaml` `.yml` |
```

Do not add GraphQL to the README graph row in this task (out of scope unless it is already there).

In `docs/superpowers/specs/2026-09-13-engram-json-yaml-outline-design.md`, set:

```markdown
Status: implemented on main
```

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib doctor::tests::grammars_line_includes_json_and_yaml --test json_yaml && cargo test --lib extract::`

Expected: PASS.

Then: `cd engram && cargo test`

Expected: full suite PASS. If an existing test assumed `.json` is `File` / `language_of` `None`, update that assertion to the new tags — do not weaken skip-rule tests (lockfiles must still be skipped).

- [ ] **Step 5: Commit**

```bash
git add engram/src/doctor.rs engram/testdata/miniapp/package.json engram/testdata/miniapp/values.yaml engram/testdata/README.md engram/tests/json_yaml.rs docs/superpowers/specs/2026-09-07-engram-core-design.md README.md docs/superpowers/specs/2026-09-13-engram-json-yaml-outline-design.md
git commit -m "feat: index JSON/YAML outline fixtures and probe json/yaml grammars"
```

---

## Spec coverage

| Spec section | Task |
|---|---|
| §6.1–6.3 dispatch, parse, `.jsonc` file-only | Task 1 |
| §6.4 `language_of` | Task 1 |
| §7–8 JSON two-level walk, arrays, junk, comments, quotes, duplicates | Task 2 |
| §7–8 YAML block/flow, multi-doc, sequence, `<<`, aliases, quotes | Task 3 |
| §6.5 doctor, §10 fixture, §12 success bar, Core §8, README | Task 4 |
| Non-goals (Bruno, `.jsonc`, filename special cases, ranking, schema) | Global constraints; no task |

`json::extract` / `yaml::extract` signatures are introduced in Task 1 and used in Tasks 2–4. `heading_names` is introduced in Task 2 and reused in Task 3.
