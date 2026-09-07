# Engram Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a local Rust CLI/MCP binary that indexes a repo into SQLite and answers `get_context` with a budgeted extractive `ContextPackage`.

**Architecture:** One `engram` binary. Indexer walks the repo, parses TS/JS/TSX/JSX/Python with tree-sitter and MD/CSS as outlines, writes `<repo>/.engram/index.sqlite`. Compiler is a pure read: symbol + FTS + 1-hop edges → rank → budget → verbatim spans. MCP is stdio JSON-RPC, read-only.

**Tech Stack:** Rust 2021, clap, rusqlite (bundled + FTS5), tree-sitter (+ python/javascript/typescript grammars), blake3, rayon, serde/serde_json, ignore, regex.

**Spec:** `docs/superpowers/specs/2026-09-07-engram-core-design.md`

## Global Constraints

- No LLM, no embeddings, no network on Core paths.
- No daemon / file watcher.
- MCP stdout is JSON-RPC only; logs go to stderr.
- CLI exit codes: `0` ok, `1` usage, `2` not initialized, `3` index/IO error.
- SQLite schema, closed `kind` sets, ranking weights, caps (50 / 30 / 40), default `budget_tokens=3000`, JSON byte cap 16KB — copy the spec verbatim; do not invent columns or tools.
- MCP tools never write the index.
- Retrieved `text` is untrusted data, never instructions.
- Work in `engram/` as the Cargo package. Run tests from that directory: `cargo test …`.
- TDD on every task: failing test first, watch it fail, minimal impl, watch it pass, commit.

## File map

| File | Responsibility |
|---|---|
| `engram/Cargo.toml` | package `engram`, binary `engram` |
| `engram/src/main.rs` | clap dispatch, map `Error` to exit codes |
| `engram/src/error.rs` | `Error` enum + `exit_code()` |
| `engram/src/types.rs` | `ParseStatus`, `SymbolKind`, `EdgeKind`, `Confidence`, extraction + `ContextPackage` types |
| `engram/src/root.rs` | `find_repo_root` (`ENGRAM_ROOT` or walk to `.engram` / `.git`) |
| `engram/src/store.rs` | schema, pragmas, CRUD, FTS, symbol/edge lookups |
| `engram/src/ignore.rs` | builtin skips + `.gitignore` + `.engramignore` |
| `engram/src/secret.rs` | secret filenames + high-confidence content regex |
| `engram/src/hash.rs` | blake3 file hash as lowercase hex |
| `engram/src/extract/mod.rs` | extension dispatch → `Extraction` |
| `engram/src/extract/python.rs` | Python graph extract |
| `engram/src/extract/ts.rs` | TS/JS/TSX/JSX graph extract |
| `engram/src/extract/markdown.rs` | ATX headings |
| `engram/src/extract/css.rs` | class / id / custom-property selectors |
| `engram/src/index.rs` | walk, incremental index, `IndexStats` |
| `engram/src/compile.rs` | query plan, retrieve, rank, budget, `get_context` |
| `engram/src/mcp.rs` | newline-delimited JSON-RPC stdio |
| `engram/src/init.rs` | `engram init`, harness files, skill |
| `engram/src/doctor.rs` | `engram doctor` checks |
| `engram/src/render.rs` | text digest of `ContextPackage` |
| `engram/testdata/miniapp/` | fixture repo |
| `engram/tests/index_incremental.rs` | incremental + skip integration |
| `engram/tests/compiler_pkg.rs` | compiler + budget + stale |
| `engram/tests/mcp_stdio.rs` | MCP framing + tools |
| `engram/tests/scale.rs` | opt-in 10k-file smoke |

Do not add Git tables, embedding columns, HTTP MCP, or a watcher.

---

### Task 1: Crate, errors, domain types

**Files:**
- Create: `engram/Cargo.toml`
- Create: `engram/src/main.rs`
- Create: `engram/src/error.rs`
- Create: `engram/src/types.rs`
- Create: `engram/src/lib.rs` (so integration tests and unit tests share the crate)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `engram::Error` with variants `NotInitialized`, `IndexBusy`, `Usage(String)`, `Io(std::io::Error)`, `Db(String)` and `fn exit_code(&self) -> i32`
  - `ParseStatus`, `SymbolKind`, `EdgeKind`, `Confidence` with `as_str()` / `from_str()` matching spec strings
  - `ExtractedSymbol`, `ExtractedEdge`, `Extraction`
  - `ContextItem`, `ContextEdge`, `ContextStats`, `ContextPackage` (`Serialize`/`Deserialize`, field names exactly as spec JSON)

- [ ] **Step 1: Scaffold the crate**

```toml
# engram/Cargo.toml
[package]
name = "engram"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
```

```rust
// engram/src/lib.rs
pub mod error;
pub mod types;
pub use error::Error;
pub use types::*;
```

```rust
// engram/src/main.rs
fn main() {
    eprintln!("not implemented");
    std::process::exit(1);
}
```

- [ ] **Step 2: Write the failing tests**

```rust
// engram/src/error.rs — bottom of file, or start with tests in error.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_spec() {
        assert_eq!(Error::Usage("x".into()).exit_code(), 1);
        assert_eq!(Error::NotInitialized.exit_code(), 2);
        assert_eq!(Error::IndexBusy.exit_code(), 3);
        assert_eq!(Error::Db("locked".into()).exit_code(), 3);
    }
}
```

```rust
// engram/src/types.rs tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_kind_roundtrip() {
        for k in [
            SymbolKind::Module,
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Type,
            SymbolKind::Component,
            SymbolKind::Heading,
            SymbolKind::Selector,
        ] {
            assert_eq!(SymbolKind::from_str(k.as_str()), Some(k));
        }
    }

    #[test]
    fn context_package_json_field_names() {
        let pkg = ContextPackage {
            query: "q".into(),
            budget_tokens: 3000,
            used_tokens: 0,
            items: vec![ContextItem {
                path: "a.ts".into(),
                start_line: 1,
                end_line: 2,
                symbol: Some("foo".into()),
                kind: Some("function".into()),
                text: "fn foo() {}".into(),
                why: vec!["exact_symbol".into()],
            }],
            edges: vec![],
            stats: ContextStats {
                files_considered: 1,
                symbols_considered: 1,
                dropped_for_budget: 0,
                stale_omitted: 0,
                stale_index: false,
                truncated: false,
            },
        };
        let v = serde_json::to_value(&pkg).unwrap();
        assert!(v.get("budget_tokens").is_some());
        assert!(v.get("items").unwrap()[0].get("start_line").is_some());
        assert_eq!(v["stats"]["stale_index"], false);
    }
}
```

Put `ContextPackage` structs in `types.rs` so the test compiles against real types. If types are missing, the test fails to compile — that is the red.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd engram && cargo test --lib`
Expected: FAIL or compile error because `Error` / `SymbolKind` / `ContextPackage` do not exist yet.

- [ ] **Step 4: Write minimal implementation**

```rust
// engram/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not initialized; run `engram init`")]
    NotInitialized,
    #[error("index busy")]
    IndexBusy,
    #[error("{0}")]
    Usage(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("db: {0}")]
    Db(String),
}

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 1,
            Error::NotInitialized => 2,
            Error::IndexBusy | Error::Io(_) | Error::Db(_) => 3,
        }
    }
}
```

Implement `ParseStatus`, `SymbolKind`, `EdgeKind`, `Confidence` with `as_str`/`from_str` using spec strings (`graph`, `function`, `import`, `high`, …). Implement the four `Context*` structs with `#[serde(rename_all = "snake_case")]` and `ContextItem.symbol` / `kind` as `Option`.

```rust
pub struct ExtractedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: u32, // 1-based
    pub end_line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
    pub signature: Option<String>,
}

pub struct ExtractedEdge {
    pub src_name: String,
    pub dst_name: String,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

pub struct Extraction {
    pub status: ParseStatus,
    pub symbols: Vec<ExtractedSymbol>,
    pub edges: Vec<ExtractedEdge>,
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd engram && cargo test --lib`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add engram/Cargo.toml engram/src
git commit -m "feat: add engram crate with errors and domain types"
```

---

### Task 2: SQLite store

**Files:**
- Create: `engram/src/store.rs`
- Modify: `engram/src/lib.rs` (add `pub mod store`)
- Modify: `engram/Cargo.toml` (add rusqlite)

**Interfaces:**
- Consumes: `Error`, `ExtractedSymbol`, `EdgeKind`, `Confidence`, `ParseStatus`
- Produces:
  - `Store::SCHEMA_VERSION = 1`
  - `Store::create(path: &Path, root: &str) -> Result<Store, Error>`
  - `Store::open_write(path: &Path) -> Result<Store, Error>`
  - `Store::open_read(path: &Path) -> Result<Store, Error>` — `query_only` pragma
  - `FileRow { id: i64, path: String, language: Option<String>, hash: String, size: i64, mtime: i64, parse_status: ParseStatus }`
  - `fn upsert_file(&self, row: &FileRow) -> Result<i64, Error>` (path unique; `id` ignored on insert)
  - `fn delete_file_by_path(&self, path: &str) -> Result<(), Error>` (CASCADE symbols/edges)
  - `fn get_file(&self, path: &str) -> Result<Option<FileRow>, Error>`
  - `fn replace_file_payload(&self, file_id: i64, symbols: &[ExtractedSymbol], fts_content: Option<&str>, path: &str) -> Result<Vec<(i64, String)>, Error>` returns `(symbol_id, name)`
  - `fn insert_edges(&self, triples: &[(i64, i64, EdgeKind, Confidence)]) -> Result<(), Error>`
  - `fn lookup_symbols_exact(&self, name: &str, limit: usize) -> Result<Vec<SymbolHit>, Error>`
  - `fn lookup_symbols_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<SymbolHit>, Error>`
  - `fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<FtsHit>, Error>`
  - `fn neighbors(&self, symbol_id: i64, cap: usize) -> Result<Vec<NeighborHit>, Error>`
  - `fn set_meta(&self, file_count: i64, symbol_count: i64, edge_count: i64) -> Result<(), Error>`
  - `fn meta(&self) -> Result<Meta, Error>`
  - `SymbolHit { id, file_id, path, name, kind, start_line, end_line, start_byte, end_byte, signature }`
  - `FtsHit { path, rank: f64 }`
  - `NeighborHit { src_id, dst_id, src_name, dst_name, src_path, dst_path, kind, confidence }`
  - `Meta { schema_version, indexed_at: Option<String>, root, file_count, symbol_count, edge_count }`

- [ ] **Step 1: Add rusqlite**

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

- [ ] **Step 2: Write the failing test**

```rust
// engram/src/store.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::path::PathBuf;

    fn tmp_db() -> PathBuf {
        let p = std::env::temp_dir().join(format!("engram-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn create_schema_and_cascade_delete() {
        let path = tmp_db();
        let store = Store::create(&path, "/tmp/proj").unwrap();
        assert_eq!(store.meta().unwrap().schema_version, 1);

        let id = store
            .upsert_file(&FileRow {
                id: 0,
                path: "a.py".into(),
                language: Some("python".into()),
                hash: "abc".into(),
                size: 10,
                mtime: 1,
                parse_status: ParseStatus::Graph,
            })
            .unwrap();

        let syms = store
            .replace_file_payload(
                id,
                &[ExtractedSymbol {
                    name: "foo".into(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 2,
                    start_byte: 0,
                    end_byte: 10,
                    signature: Some("def foo()".into()),
                }],
                Some("def foo():\n  pass\n"),
                "a.py",
            )
            .unwrap();
        assert_eq!(syms[0].1, "foo");

        store.delete_file_by_path("a.py").unwrap();
        assert!(store.get_file("a.py").unwrap().is_none());
        assert!(store.lookup_symbols_exact("foo", 10).unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fts_finds_content() {
        let path = tmp_db();
        let store = Store::create(&path, "/tmp/proj").unwrap();
        let id = store
            .upsert_file(&FileRow {
                id: 0,
                path: "README.md".into(),
                language: Some("markdown".into()),
                hash: "h".into(),
                size: 20,
                mtime: 1,
                parse_status: ParseStatus::Outline,
            })
            .unwrap();
        store
            .replace_file_payload(id, &[], Some("WebSockets replaced polling"), "README.md")
            .unwrap();
        let hits = store.fts_search("WebSockets", 10).unwrap();
        assert_eq!(hits[0].path, "README.md");
        let _ = std::fs::remove_file(&path);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd engram && cargo test --lib store::`
Expected: FAIL — `Store` not found

- [ ] **Step 4: Write minimal implementation**

Open with:

```rust
conn.pragma_update(None, "journal_mode", "WAL")?;
conn.pragma_update(None, "synchronous", "NORMAL")?;
conn.pragma_update(None, "foreign_keys", "ON")?;
```

`open_read` also sets `query_only = ON`.

Execute the spec DDL exactly (tables `meta`, `files`, `symbols`, `edges`, `file_fts`, and the five indexes). Insert `meta` row `schema_version=1`.

`replace_file_payload`: `DELETE FROM symbols WHERE file_id=?` (edges cascade), insert symbols, `INSERT INTO file_fts(path, content)` after deleting existing FTS row for that path.

`lookup_symbols_exact`: `WHERE name = ? COLLATE NOCASE LIMIT ?`

`lookup_symbols_prefix`: `WHERE name LIKE ? ESCAPE '\' COLLATE NOCASE` with `prefix` + `%`

`fts_search`: `SELECT path, rank FROM file_fts WHERE file_fts MATCH ? ORDER BY rank LIMIT ?` — if `rank` is unavailable on the bundled FTS, `SELECT path FROM file_fts WHERE file_fts MATCH ? LIMIT ?` and set `rank` to descending position.

Map rusqlite errors: if message contains `database is locked`, return `Error::IndexBusy`, else `Error::Db`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd engram && cargo test --lib store::`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add engram/Cargo.toml engram/src/store.rs engram/src/lib.rs
git commit -m "feat: add SQLite store with FTS5 and cascade deletes"
```

---

### Task 3: Repo root, ignore, secrets, hash

**Files:**
- Create: `engram/src/root.rs`
- Create: `engram/src/ignore.rs`
- Create: `engram/src/secret.rs`
- Create: `engram/src/hash.rs`
- Modify: `engram/src/lib.rs`
- Modify: `engram/Cargo.toml` (add `ignore`, `blake3`, `regex`)

**Interfaces:**
- Consumes: `Error`
- Produces:
  - `fn find_repo_root(cwd: &Path, env_root: Option<&Path>) -> Result<PathBuf, Error>` — if `env_root` set, use it if it exists; else walk up from `cwd` until a directory contains `.engram` or `.git`; else `Error::NotInitialized`. Never return `$HOME` just because it exists.
  - `struct SkipReason { pub skipped_secret: bool, pub skipped_large: bool, pub skipped_ignore: bool }`
  - `fn should_skip(root: &Path, rel_posix: &str, bytes: Option<&[u8]>) -> SkipKind` where `SkipKind` is `Keep | Ignore | SecretName | SecretContent | Large | Binary`
  - `const MAX_FILE_BYTES: u64 = 1_048_576`
  - builtin dir/file globs from spec §10.1
  - `fn is_secret_content(bytes: &[u8]) -> bool`
  - `fn blake3_hex(bytes: &[u8]) -> String` and `fn blake3_file(path: &Path) -> Result<String, Error>`

- [ ] **Step 1: Write failing tests**

```rust
// engram/src/root.rs tests
#[test]
fn missing_markers_is_not_initialized() {
    let tmp = tempfile_dir(); // use std::env::temp_dir + unique name, create_dir
    let err = find_repo_root(&tmp, None).unwrap_err();
    assert!(matches!(err, Error::NotInitialized));
}

#[test]
fn finds_engram_dir() {
    let tmp = tempfile_dir();
    std::fs::create_dir(tmp.join(".engram")).unwrap();
    let nested = tmp.join("src");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(find_repo_root(&nested, None).unwrap(), tmp);
}

#[test]
fn env_root_wins() {
    let a = tempfile_dir();
    let b = tempfile_dir();
    std::fs::create_dir(b.join(".engram")).unwrap();
    assert_eq!(find_repo_root(&a, Some(&b)).unwrap(), b);
}
```

```rust
// engram/src/ignore.rs + secret.rs tests
#[test]
fn skips_node_modules_and_env() {
    assert!(matches!(should_skip(Path::new("/p"), "node_modules/x.js", None), SkipKind::Ignore));
    assert!(matches!(should_skip(Path::new("/p"), ".env", None), SkipKind::SecretName));
    assert!(matches!(should_skip(Path::new("/p"), ".env.local", None), SkipKind::SecretName));
}

#[test]
fn skips_nul_and_large() {
    assert!(matches!(should_skip(Path::new("/p"), "a.bin", Some(&[0, 1, 2])), SkipKind::Binary));
    let big = vec![b'a'; 1_048_577];
    assert!(matches!(should_skip(Path::new("/p"), "big.py", Some(&big)), SkipKind::Large));
}

#[test]
fn skips_pem_content_without_keeping_match() {
    let pem = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n";
    assert!(is_secret_content(pem));
    assert!(matches!(should_skip(Path::new("/p"), "oops.txt", Some(pem)), SkipKind::SecretContent));
}
```

```rust
// engram/src/hash.rs
#[test]
fn blake3_stable_hex() {
    assert_eq!(blake3_hex(b"hi").len(), 64);
    assert_eq!(blake3_hex(b"hi"), blake3_hex(b"hi"));
    assert_ne!(blake3_hex(b"hi"), blake3_hex(b"ho"));
}
```

Helper `tempfile_dir()`: create `std::env::temp_dir().join(format!("engram-root-{}-{}", std::process::id(), n))`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib root:: ignore:: secret:: hash::`
Expected: FAIL — modules missing

- [ ] **Step 3: Implement**

`find_repo_root`: if `env_root` is `Some(p)` and `p.is_dir()`, return `p` (canonicalize). Else loop `cwd` and parents; if `dir.join(".engram").is_dir() || dir.join(".git").exists()` return `dir`. End → `NotInitialized`.

`should_skip`:
1. If any path component is in `{.git, node_modules, .venv, venv, __pycache__, dist, build, .engram, .next, target}` → `Ignore`
2. Filename matches `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `credentials.json` → `SecretName`
3. Filename matches lockfiles (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`, `poetry.lock`, `uv.lock`), `*.min.js`, `*.map`, or image/archive extensions (`png jpg jpeg gif webp zip tar gz wasm woff woff2`) → `Ignore`
4. If `bytes` contains `0u8` → `Binary`
5. If `bytes.len() as u64 > MAX_FILE_BYTES` → `Large`
6. If `bytes` is `Some` and `is_secret_content` → `SecretContent`
7. Else if `root` has `.gitignore` / `.engramignore`, use the `ignore` crate `WalkBuilder` override: for a single relative path, build a `gitignore::Gitignore` from those files plus builtins. Match `rel_posix` → `Ignore`

`is_secret_content` (regex crate, whole-file search, no capture storage):

```text
-----BEGIN [A-Z ]*PRIVATE KEY-----
AKIA[0-9A-Z]{16}
ghp_[A-Za-z0-9]{20,}
sk-[A-Za-z0-9]{20,}
```

`blake3_hex`: `blake3::hash(bytes)` hex lowercase.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/root.rs engram/src/ignore.rs engram/src/secret.rs engram/src/hash.rs engram/src/lib.rs engram/Cargo.toml
git commit -m "feat: add repo root discovery, ignore rules, secret skips, blake3"
```

---

### Task 4: Python extractor

**Files:**
- Create: `engram/src/extract/mod.rs`
- Create: `engram/src/extract/python.rs`
- Modify: `engram/src/lib.rs` (`pub mod extract`)
- Modify: `engram/Cargo.toml` (`tree-sitter`, `tree-sitter-python`)

**Interfaces:**
- Consumes: `Extraction`, `ExtractedSymbol`, `ExtractedEdge`, `ParseStatus`, `SymbolKind`, `EdgeKind`, `Confidence`
- Produces:
  - `extract::python::extract(source: &str) -> Extraction` with `status = Graph` on success, `Error` on parse failure (empty tree)
  - `extract::extract_path(rel_posix: &str, source: &str) -> Extraction` (dispatch; Python only in this task, other extensions `ParseStatus::File` empty symbols)

- [ ] **Step 1: Write the failing test**

```rust
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
    assert!(ext.symbols.iter().any(|s| s.name == "Auth" && s.kind == SymbolKind::Class));
    assert!(ext.symbols.iter().any(|s| s.name == "login" && s.kind == SymbolKind::Method));
    assert!(ext.edges.iter().any(|e| e.dst_name == "create_session" && e.kind == EdgeKind::Call));
    assert!(ext.edges.iter().any(|e| e.kind == EdgeKind::Import && (e.dst_name == "os" || e.dst_name == "session" || e.dst_name.contains("create_session"))));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engram && cargo test --lib extract::`
Expected: FAIL — `extract` missing

- [ ] **Step 3: Implement with tree-sitter**

```toml
tree-sitter = "0.25"
tree-sitter-python = "0.25"
```

If those versions fail to resolve, use `cargo add tree-sitter tree-sitter-python` and pin whatever compiles together. API: `parser.set_language(&tree_sitter_python::LANGUAGE.into())`.

Queries (adjust capture names to the grammar if a capture is empty — print `tree.root_node().to_sexp()` in a skipped test if needed):

```text
(function_definition name: (identifier) @name) @def
(class_definition name: (identifier) @name) @def
(import_statement name: (dotted_name (identifier) @mod))
(import_from_statement module_name: (dotted_name (identifier) @mod) name: (dotted_name (identifier) @imported))
(call function: (identifier) @callee)
(call function: (attribute attribute: (identifier) @callee))
```

Line numbers: `node.start_position().row + 1`. Methods: if a `function_definition` node’s parent (or parent.parent) is `class_definition`, `kind = Method`, else `Function`. Also emit a `module` symbol named from nothing — skip module symbol unless the file has a docstring-only need; spec allows `module` but tests above do not require it.

Calls: `src_name` = nearest enclosing function/class name, `dst_name` = callee identifier, `kind = Call`, `confidence = Low` (indexer upgrades later).

Imports: `src_name` = `"<module>"` or the file’s module stem passed later; for this function use `"<file>"` as src. Indexer maps `"<file>"` to the file’s module symbol. **Add a file-level `SymbolKind::Module` named `"<file>"`** so resolution works.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib extract::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract engram/src/lib.rs engram/Cargo.toml
git commit -m "feat: extract Python symbols, imports, and calls"
```

---

### Task 5: TypeScript / JavaScript / TSX extractor

**Files:**
- Create: `engram/src/extract/ts.rs`
- Modify: `engram/src/extract/mod.rs`
- Modify: `engram/Cargo.toml` (`tree-sitter-javascript`, `tree-sitter-typescript`)

**Interfaces:**
- Consumes: same extraction types
- Produces:
  - `extract::ts::extract(source: &str, lang: TsLang) -> Extraction`
  - `enum TsLang { Typescript, Tsx, Javascript }`
  - `extract_path` maps `.ts` → Typescript, `.tsx`/`.jsx` → Tsx, `.js`/`.mjs`/`.cjs` → Javascript

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn tsx_component_and_import() {
    let src = r#"
import { createSession } from "./session";
export function LoginBanner() {
  return createSession();
}
export function helper() {
  return LoginBanner();
}
"#;
    let ext = crate::extract::ts::extract(src, crate::extract::ts::TsLang::Tsx);
    assert_eq!(ext.status, ParseStatus::Graph);
    let names: Vec<_> = ext.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"LoginBanner"));
    assert!(names.contains(&"helper"));
    assert!(ext.symbols.iter().any(|s| s.name == "LoginBanner" && (s.kind == SymbolKind::Function || s.kind == SymbolKind::Component)));
    assert!(ext.edges.iter().any(|e| e.dst_name == "createSession"));
}

#[test]
fn ts_interface_and_class() {
    let src = "export interface User { id: string }\nexport class AuthService { login() {} }\n";
    let ext = crate::extract::ts::extract(src, crate::extract::ts::TsLang::Typescript);
    assert!(ext.symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Interface));
    assert!(ext.symbols.iter().any(|s| s.name == "AuthService" && s.kind == SymbolKind::Class));
}

#[test]
fn jsx_uses_tsx_grammar_not_ts() {
    let src = "export const App = () => <div/>;";
    let ext = crate::extract::ts::extract(src, crate::extract::ts::TsLang::Tsx);
    assert_eq!(ext.status, ParseStatus::Graph);
    assert!(ext.symbols.iter().any(|s| s.name == "App"));
}
```

Treat PascalCase function/const as `Component` when `TsLang::Tsx`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engram && cargo test --lib extract::ts::`
Expected: FAIL

- [ ] **Step 3: Implement**

Languages:
- `tree_sitter_typescript::LANGUAGE_TYPESCRIPT`
- `tree_sitter_typescript::LANGUAGE_TSX` for `.tsx` and `.jsx`
- `tree_sitter_javascript::LANGUAGE` for `.js`/`.mjs`/`.cjs`

Queries (adapt to grammar; common node types): `function_declaration`, `method_definition`, `class_declaration`, `interface_declaration`, `type_alias_declaration`, `lexical_declaration` / `variable_declarator` with `identifier` + `arrow_function`/`function`, `import_statement`, `call_expression`.

`extract_path` in `mod.rs`:

```rust
pub fn extract_path(rel_posix: &str, source: &str) -> Extraction {
    let lower = rel_posix.to_ascii_lowercase();
    if lower.ends_with(".py") { return python::extract(source); }
    if lower.ends_with(".tsx") || lower.ends_with(".jsx") {
        return ts::extract(source, ts::TsLang::Tsx);
    }
    if lower.ends_with(".ts") { return ts::extract(source, ts::TsLang::Typescript); }
    if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs") {
        return ts::extract(source, ts::TsLang::Javascript);
    }
    Extraction { status: ParseStatus::File, symbols: vec![], edges: vec![] }
}
```

On parser error: `status = ParseStatus::Error`, empty symbols.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib extract::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract engram/Cargo.toml
git commit -m "feat: extract TS/JS/TSX symbols and edges"
```

---

### Task 6: Markdown and CSS outline extractors

**Files:**
- Create: `engram/src/extract/markdown.rs`
- Create: `engram/src/extract/css.rs`
- Modify: `engram/src/extract/mod.rs`

**Interfaces:**
- Consumes: extraction types
- Produces: `markdown::extract`, `css::extract`; `extract_path` maps `.md`/`.mdx` → outline headings, `.css`/`.scss` → outline selectors; **no edges**

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn markdown_atx_headings() {
    let src = "# Title\n\n## WebSockets\n\ntext\n";
    let ext = crate::extract::markdown::extract(src);
    assert_eq!(ext.status, ParseStatus::Outline);
    assert!(ext.edges.is_empty());
    assert!(ext.symbols.iter().any(|s| s.name == "WebSockets" && s.kind == SymbolKind::Heading));
}

#[test]
fn css_class_id_custom_prop() {
    let src = ".auth-panel { color: red; }\n#root { }\n:root { --brand: #00f; }\n";
    let ext = crate::extract::css::extract(src);
    assert_eq!(ext.status, ParseStatus::Outline);
    assert!(ext.edges.is_empty());
    let names: Vec<_> = ext.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&".auth-panel") || names.contains(&"auth-panel"));
    assert!(names.contains(&"#root") || names.contains(&"root"));
    assert!(names.iter().any(|n| n.contains("--brand") || *n == "brand"));
}
```

Normalize stored names to include the sigil: `.auth-panel`, `#root`, `--brand` so FTS/symbol search for `.auth-panel` works.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib extract::markdown:: extract::css::`
Expected: FAIL

- [ ] **Step 3: Implement without tree-sitter**

Markdown: for each line matching `^(#{1,6})\s+(.+?)\s*#*\s*$`, emit `Heading` with that line’s 1-based number as start/end.

CSS: regexes on the raw text:

```text
\.([A-Za-z_][\w-]*)
#([A-Za-z_][\w-]*)
--([A-Za-z_][\w-]*)
```

Emit `Selector` with names `.class`, `#id`, `--prop`. Skip matches inside `url(` if easy; do not parse full SCSS. Nested SCSS classes still match the regex.

Wire `extract_path` for `.md` `.mdx` `.css` `.scss`.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib extract::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/extract
git commit -m "feat: extract markdown headings and CSS selectors"
```

---

### Task 7: Indexer

**Files:**
- Create: `engram/src/index.rs`
- Create: `engram/tests/index_incremental.rs`
- Create: `engram/testdata/miniapp/` (minimal files listed below)
- Modify: `engram/src/lib.rs`

**Interfaces:**
- Consumes: `Store`, `find_repo_root`, `should_skip`, `SkipKind`, `blake3_file`/`blake3_hex`, `extract_path`
- Produces:
  - `struct IndexStats { files: u64, symbols: u64, edges: u64, skipped_secret: u64, skipped_large: u64, skipped_ignore: u64, errors: u64, unchanged: u64 }`
  - `fn index_repo(root: &Path, force: bool) -> Result<IndexStats, Error>`
  - Language string: `python` / `typescript` / `tsx` / `javascript` / `markdown` / `css` / `None` for file-only
  - Edge resolution: map name edges to symbol ids. Same-file definition → `Call` + `High`. Call to a name only found as callee with a same-file symbol → `High`. Call to a name not defined in-file → `Low` if any symbol in the same file has that name, else drop. Import: try to find a `Module` or file whose path stem matches `dst_name`, or a symbol named `dst_name`; `Import` + `High` if found, else drop.

- [ ] **Step 1: Create miniapp fixture**

```
engram/testdata/miniapp/
  src/auth/session.ts     export function createSession() { return 1 }
  src/auth/LoginBanner.tsx  import { createSession } from "./session"; export function LoginBanner() { return createSession(); }
  src/app.py              def helper():\n    return 1
  README.md               ## WebSockets\n\nWe use websockets.
  styles/auth.css         .auth-panel { color: red }
  .env                    SECRET=do-not-index
  huge.bin                (do not commit a 1MB file; the test creates it)
```

Keep fixture tiny. `.env` committed with dummy content.

- [ ] **Step 2: Write failing integration tests**

```rust
// engram/tests/index_incremental.rs
use engram::index::index_repo;
use engram::store::Store;
use std::fs;
use std::path::PathBuf;

fn setup() -> PathBuf {
    let root = std::env::temp_dir().join(format!("engram-idx-{}", uuidish()));
    // copy miniapp or write files
    fs::create_dir_all(root.join("src/auth")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(root.join("src/auth/session.ts"), "export function createSession() { return 1 }\n").unwrap();
    fs::write(root.join(".env"), "SECRET=nope\n").unwrap();
    fs::write(root.join("README.md"), "## WebSockets\n").unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    root
}

fn uuidish() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
}

#[test]
fn indexes_ts_and_skips_env() {
    let root = setup();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.symbols >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.lookup_symbols_exact("createSession", 10).unwrap().iter().any(|h| h.name == "createSession"));
    assert!(db.get_file(".env").unwrap().is_none());
    let fts = db.fts_search("SECRET", 10).unwrap();
    assert!(fts.iter().all(|h| h.path != ".env"));
}

#[test]
fn unchanged_hash_is_noop() {
    let root = setup();
    let a = index_repo(&root, false).unwrap();
    let b = index_repo(&root, false).unwrap();
    assert!(b.unchanged >= 1);
    assert_eq!(a.symbols, b.symbols);
}

#[test]
fn change_one_file_reparses_only_that() {
    let root = setup();
    index_repo(&root, false).unwrap();
    fs::write(root.join("src/auth/session.ts"), "export function createSession() { return 2 }\nexport function extra() {}\n").unwrap();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.unchanged >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.lookup_symbols_exact("extra", 10).unwrap().len() == 1);
}

#[test]
fn deleted_path_dropped() {
    let root = setup();
    index_repo(&root, false).unwrap();
    fs::remove_file(root.join("README.md")).unwrap();
    index_repo(&root, false).unwrap();
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.get_file("README.md").unwrap().is_none());
}

#[test]
fn large_file_skipped() {
    let root = setup();
    let big = vec![b'x'; 1_048_577];
    fs::write(root.join("blob.txt"), &big).unwrap();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.skipped_large >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.get_file("blob.txt").unwrap().is_none());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd engram && cargo test --test index_incremental`
Expected: FAIL — `index_repo` missing

- [ ] **Step 4: Implement `index_repo`**

Algorithm:
1. Open `root.join(".engram/index.sqlite")` with `open_write`; if missing, `Error::NotInitialized`.
2. Collect relative POSIX paths via `ignore::WalkBuilder::new(root)` with hidden=true, git_ignore=true, plus add `.engramignore` if present. Skip directories in the builtin list even without gitignore.
3. Load existing `files` rows into `HashMap<path, hash>`.
4. `let mut seen = HashSet::new()`.
5. For each file (rayon `into_par_iter` for hash+read+extract):
   - `should_skip`; bump counters; continue
   - hash bytes; if `!force && existing_hash == hash` → unchanged, `seen.insert`, continue
   - `extract_path`; on panic/error, `ParseStatus::Error` empty symbols
   - Produce a `WorkItem { rel, hash, size, mtime, language, extraction, fts: String }`
6. On the main thread, transaction: for each work item `delete_file_by_path` if exists, `upsert_file`, `replace_file_payload`, collect symbol id maps per file. Then resolve edges across the just-written files (and existing unchanged symbols via `lookup_symbols_exact` for import targets). `insert_edges`.
7. For existing paths not in `seen`, `delete_file_by_path`.
8. `set_meta` with counts. Return `IndexStats`.

Path format: POSIX relative (`src/auth/session.ts`), no leading `./`.

Parse rayon pool: extract in parallel, **SQLite writes serialized**.

- [ ] **Step 5: Run tests**

Run: `cd engram && cargo test --test index_incremental && cargo test --lib`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add engram/src/index.rs engram/tests/index_incremental.rs engram/testdata engram/src/lib.rs
git commit -m "feat: incremental repo indexer with secret and size skips"
```

---

### Task 8: Compiler (`get_context`)

**Files:**
- Create: `engram/src/compile.rs`
- Create: `engram/tests/compiler_pkg.rs`
- Modify: `engram/src/lib.rs`

**Interfaces:**
- Consumes: `Store`, `ContextPackage`, `hash::blake3_file`
- Produces:
  - `struct QueryPlan { symbol_terms: Vec<String>, fts_query: String, path_hints: Vec<String> }`
  - `fn plan_query(query: &str) -> QueryPlan`
  - `fn get_context(root: &Path, query: &str, budget_tokens: u32) -> Result<ContextPackage, Error>`
  - `fn search_symbols(root: &Path, name: &str, limit: usize) -> Result<Vec<SymbolHit>, Error>`
  - `fn search_code(root: &Path, query: &str, limit: usize) -> Result<Vec<FtsHit>, Error>`
  - constants: `CAP_SYMBOLS=50`, `CAP_FTS=30`, `CAP_NEIGHBORS=40`, `DEFAULT_BUDGET=3000`, `MAX_JSON_BYTES=16_384`

- [ ] **Step 1: Write failing unit tests for the planner**

```rust
#[test]
fn plan_extracts_quotes_camel_paths() {
    let p = plan_query(r#"where is "createSession" in src/auth for LoginBanner?"#);
    assert!(p.symbol_terms.iter().any(|t| t == "createSession"));
    assert!(p.symbol_terms.iter().any(|t| t == "LoginBanner"));
    assert!(p.path_hints.iter().any(|h| h.contains("src/auth")));
}
```

Quoted strings → symbol terms. Tokens matching `[A-Za-z_][\w]*` with a capital or `_` → symbol terms. Tokens containing `/` or starting with `.` and a letter → path hints. Remaining words joined as FTS query (drop stopwords `the a an is in for of where how what`).

- [ ] **Step 2: Run planner test (fail then implement `plan_query` only)**

Run: `cd engram && cargo test --lib compile::plan_extracts`
Expected: fail then pass after implementing `plan_query`.

- [ ] **Step 3: Write failing compiler integration tests**

```rust
// engram/tests/compiler_pkg.rs
use engram::compile::{get_context, plan_query};
use engram::index::index_repo;
use engram::store::Store;
use engram::types::ContextPackage;

fn repo() -> std::path::PathBuf { /* same pattern as indexer tests: session.ts + README + css */ }

#[test]
fn exact_symbol_quotes_span() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    assert!(pkg.items.iter().any(|i| i.symbol.as_deref() == Some("createSession")));
    assert!(pkg.items.iter().any(|i| i.text.contains("createSession")));
    assert!(pkg.items.iter().any(|i| i.why.iter().any(|w| w == "exact_symbol")));
}

#[test]
fn fts_hits_heading() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "WebSockets", 3000).unwrap();
    assert!(pkg.items.iter().any(|i| i.path.ends_with("README.md") && i.text.contains("WebSockets")));
}

#[test]
fn small_budget_drops_low_rank() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let tiny = get_context(&root, "createSession WebSockets auth-panel", 20).unwrap();
    let big = get_context(&root, "createSession WebSockets auth-panel", 3000).unwrap();
    assert!(tiny.items.len() <= big.items.len());
    assert!(tiny.used_tokens <= 20 || tiny.items.is_empty());
}

#[test]
fn stale_hash_omits_span() {
    let root = repo();
    index_repo(&root, true).unwrap();
    std::fs::write(root.join("src/auth/session.ts"), "export function createSession() { return 99 }\n").unwrap();
    // do not reindex
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    assert!(pkg.stats.stale_omitted >= 1 || pkg.stats.stale_index);
    assert!(!pkg.items.iter().any(|i| i.text.contains("return 99"))); // extractive from matching hash only; omitted if mismatch
}

#[test]
fn empty_query_has_no_invented_text() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "zzzxnotarealsymbolzzz", 3000).unwrap();
    assert!(pkg.items.is_empty() || pkg.items.iter().all(|i| !i.text.is_empty()));
    // never a prose summary field
    let json = serde_json::to_string(&pkg).unwrap();
    assert!(!json.contains("summary"));
}
```

For the stale test: compiler re-reads disk and compares `blake3_file` to `files.hash`. On mismatch, omit item and `stale_omitted += 1`. Do not quote the new bytes.

- [ ] **Step 4: Run compiler tests to verify they fail**

Run: `cd engram && cargo test --test compiler_pkg`
Expected: FAIL — `get_context` missing

- [ ] **Step 5: Implement `get_context`**

1. `open_read(root.join(".engram/index.sqlite"))` else `NotInitialized`.
2. `plan = plan_query(query)`.
3. Candidates:
   - For each `symbol_terms`: `lookup_symbols_exact` then `lookup_symbols_prefix` until 50.
   - `fts_search(plan.fts_query, 30)` if fts_query nonempty; also search each symbol term.
   - Outline symbols already in the 50 if their kind is heading/selector.
   - Neighbors: for each accepted symbol id, `neighbors(id, remaining)` until 40. Always take `High`; take `Low` only if cap not full.
4. Convert each hit to a span:
   - Symbol → `(path, start_line, end_line, symbol, kind, why)`
   - FTS → whole file is too big: use first 40 lines as a provisional span, `why=fts`. If a heading symbol exists in that file matching the query, prefer the heading span (`why=heading`).
5. Fuse by `(path, start_line, end_line)`; union `why`.
6. Score with spec weights. Normalize FTS rank to 0–1 as `1.0 / (1.0 + index_in_fts_list)`.
7. Dedupe: sort by path, merge overlapping ranges; if symbol span inside FTS span, drop FTS. First pass: max 2 spans per path; if `budget` still remaining after first pass, allow more.
8. Walk score desc. For each span, `get_file` hash vs `blake3_file(root.join(path))`. Mismatch → `stale_omitted++`, skip. Match → read lines `start_line..=end_line` (1-based). `token_cost = whitespace split of text + 2` (path header). If `used + cost > budget_tokens`, `dropped_for_budget++`, stop adding. After building package, `serde_json::to_vec`; if `len() > 16_384`, pop last items until it fits and set `truncated=true`.
9. `stale_index = stale_omitted * 2 > (items.len() + stale_omitted)` (majority of attempted quotes).
10. `edges` on the package: only neighbor edges whose both ends appear in `items`, formatted `path:name`.

Token estimate: `text.split_whitespace().count() as u32 + 2`.

- [ ] **Step 6: Run tests**

Run: `cd engram && cargo test --test compiler_pkg && cargo test --lib compile::`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add engram/src/compile.rs engram/tests/compiler_pkg.rs engram/src/lib.rs
git commit -m "feat: extractive get_context compiler with budget and stale checks"
```

---

### Task 9: CLI

**Files:**
- Create: `engram/src/init.rs`
- Create: `engram/src/doctor.rs`
- Create: `engram/src/render.rs`
- Modify: `engram/src/main.rs`
- Modify: `engram/Cargo.toml` (`clap` with derive)
- Modify: `engram/src/lib.rs`

**Interfaces:**
- Consumes: `index_repo`, `get_context`, `search_symbols`, `search_code`, `Store`, `find_repo_root`, `Error`
- Produces: CLI subcommands matching spec §13. `init` in this task creates `.engram/`, empty DB, default `.engramignore`, gitignore entry — **not** harness/skill yet (Task 11).
  - `fn run_init(cwd: &Path) -> Result<PathBuf, Error>`
  - `fn render_digest(pkg: &ContextPackage) -> String`
  - `fn run_doctor(root: &Path) -> Result<String, Error>`
  - `fn env_root() -> Option<PathBuf>` reads `ENGRAM_ROOT`

- [ ] **Step 1: Write failing tests for init + digest**

```rust
#[test]
fn init_creates_db_and_gitignore_entry() {
    let root = tempfile_dir();
    std::fs::write(root.join(".gitignore"), "node_modules\n").unwrap();
    engram::init::run_init(&root).unwrap();
    assert!(root.join(".engram/index.sqlite").is_file());
    assert!(root.join(".engramignore").is_file());
    let gi = std::fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(gi.contains(".engram/"));
}

#[test]
fn digest_is_extractive() {
    let pkg = sample_package(); // helper with one item
    let t = engram::render::render_digest(&pkg);
    assert!(t.contains("a.ts"));
    assert!(t.contains("```"));
    assert!(t.contains("fn foo"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib init:: render::`
Expected: FAIL

- [ ] **Step 3: Implement init, render, doctor, clap**

Default `.engramignore` body:

```
node_modules
.venv
dist
build
target
.next
*.min.js
```

`run_init`: create `.engram`, `Store::create`, write `.engramignore` if absent, append `.engram/\n` to `.gitignore` if that file exists and the line is missing. Do not call `index_repo`.

`render_digest`:

```text
# query
budget 3000 used 12

## path:start-end (symbol) [why]
```code
text
```
```

`main.rs` clap:

```text
engram init
engram index [--force]
engram status
engram get-context <QUERY> [--json] [--budget N]
engram search-symbols <NAME>
engram search-code <QUERY>
engram mcp
engram doctor
```

Resolve root: `find_repo_root(&cwd, env_root().as_deref())`. `init` uses `cwd` as root even without markers (it creates them). All other commands require `find_repo_root`.

`mcp` subcommand in this task can `unimplemented` or call a stub `engram::mcp::run()` that returns `Usage("mcp not ready")` — Task 10 replaces it. Prefer a stub that compiles.

Map errors: `main` prints `eprintln!("{err}")` and `process::exit(err.exit_code())`.

- [ ] **Step 4: Add a CLI smoke test via `assert_cmd` or `std::process` after implementing**

Optional: `engram/tests/cli_init.rs` spawning `env!("CARGO_BIN_EXE_engram")`. Add to Cargo.toml:

```toml
[[bin]]
name = "engram"
path = "src/main.rs"
```

(package default bin is already `src/main.rs` if we keep both lib and bin — add `src/main.rs` uses `engram::...`.)

```rust
#[test]
fn bin_init_exit_zero() {
    let root = tempfile_dir();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["init"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(root.join(".engram/index.sqlite").exists());
}
```

This requires `engram` as both lib and bin. In `main.rs`: `use engram::*;` or call `engram::init::run_init`.

- [ ] **Step 5: Run tests**

Run: `cd engram && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add engram/src/main.rs engram/src/init.rs engram/src/doctor.rs engram/src/render.rs engram/Cargo.toml engram/tests
git commit -m "feat: add engram CLI (init, index, status, get-context, search, doctor)"
```

---

### Task 10: MCP stdio

**Files:**
- Create: `engram/src/mcp.rs`
- Create: `engram/tests/mcp_stdio.rs`
- Modify: `engram/src/main.rs` (`engram mcp` → `mcp::run()`)
- Modify: `engram/src/lib.rs`

**Interfaces:**
- Consumes: `get_context`, `search_symbols`, `search_code`, `Store::open_read`, `find_repo_root`
- Produces:
  - `fn run() -> Result<(), Error>` — read stdin, write stdout
  - `fn handle_line(root: &Path, line: &str) -> Option<String>` — one JSON-RPC request → one response JSON (no trailing extra). Notifications (`initialize` may still get a result). Returns `None` for `notifications/*` that need no reply.
  - Tools: `get_context`, `search_symbols`, `search_code`, `index_status`
  - Transport: **newline-delimited JSON-RPC** (MCP stdio). Requests must not be logged to stdout.

- [ ] **Step 1: Write failing tests**

```rust
// engram/src/mcp.rs tests + engram/tests/mcp_stdio.rs
#[test]
fn tools_list_contains_get_context() {
    let root = mini_indexed_repo();
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    assert!(resp.contains("get_context"));
    assert!(resp.contains("search_symbols"));
    assert!(resp.contains("search_code"));
    assert!(resp.contains("index_status"));
    assert!(resp.contains("before searching the repo") || resp.contains("before grepping") || resp.contains("before searching"));
}

#[test]
fn get_context_tool_returns_package_under_16kb() {
    let root = mini_indexed_repo();
    let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_context","arguments":{"query":"createSession"}}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    assert!(resp.len() < 16_384);
    assert!(resp.contains("createSession") || resp.contains("items"));
}

#[test]
fn missing_index_is_not_initialized() {
    let root = tempfile_dir(); // no .engram
    let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"index_status","arguments":{}}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    assert!(resp.contains("not_initialized") || resp.contains("not initialized"));
}

#[test]
fn stdout_has_no_logs() {
    // handle_line must not print
    let root = mini_indexed_repo();
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
}
```

`get_context` tool description string **must** include the phrase `call this before searching the repo`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test mcp`
Expected: FAIL

- [ ] **Step 3: Implement**

`initialize` result: `{ protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "engram", version: "0.1.0" } }`

`tools/list` tool schemas:

- `get_context`: required `query` string, optional `budget_tokens` number. Description: `Compile a small extractive context package for a question about this repository. Call this before searching the repo.`
- `search_symbols`: required `name`, optional `limit`
- `search_code`: required `query`, optional `limit`
- `index_status`: no required args

`tools/call` `get_context`: parse args, `budget_tokens` default 3000, `get_context(root, query, budget)`, return as MCP `content: [{type:"text", text: json}]`.

`index_status`: if no db, JSON-RPC error or tool error `{ "error": "not_initialized" }` — pick **tool result** `isError: true` with text `not_initialized` so clients show it. If db locked, `index_busy`.

`run()`: `find_repo_root(cwd, ENGRAM_ROOT)`. Loop `stdin.lock().lines()`. For each non-empty line, if `handle_line` returns `Some(s)`, `writeln!(stdout, "{s}")`. Flush. Never `println!` debug.

Invalid JSON: JSON-RPC error `id: null` code `-32700`.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test mcp && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/mcp.rs engram/src/main.rs engram/tests/mcp_stdio.rs engram/src/lib.rs
git commit -m "feat: add read-only stdio MCP (get_context first)"
```

---

### Task 11: Harness wiring, skill, miniapp polish

**Files:**
- Modify: `engram/src/init.rs`
- Modify: `engram/src/main.rs` (`--harness`, `--skill`, `--write-agents`)
- Create: `engram/src/skill_template.rs` (or a `const SKILL_MD: &str` in `init.rs`)
- Modify: `engram/testdata/miniapp/` (LoginBanner.tsx, app.py, auth.css as in spec tests)

**Interfaces:**
- Consumes: `run_init`
- Produces:
  - `fn write_harness(root: &Path, id: &str) -> Result<(), Error>` ids: `grok`, `copilot`, `claude`, `cursor`, `all`
  - `fn write_skill(root: &Path, also_claude: bool, write_agents: bool) -> Result<(), Error>`
  - Merge JSON/TOML: do not delete unrelated MCP servers

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn harness_grok_writes_toml() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    engram::init::write_harness(&root, "grok").unwrap();
    let t = std::fs::read_to_string(root.join(".grok/config.toml")).unwrap();
    assert!(t.contains("[mcp_servers.engram]"));
    assert!(t.contains("command = \"engram\""));
    assert!(t.contains("mcp"));
}

#[test]
fn harness_copilot_merges_mcp_json() {
    let root = tempfile_dir();
    std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
    engram::init::run_init(&root).unwrap();
    engram::init::write_harness(&root, "copilot").unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join(".mcp.json")).unwrap()).unwrap();
    assert!(v["mcpServers"]["other"].is_object() || v["servers"]["other"].is_object());
    // accept either mcpServers or servers key; write the one already present, else mcpServers
    let engram = v.pointer("/mcpServers/engram").or_else(|| v.pointer("/servers/engram"));
    assert!(engram.is_some());
}

#[test]
fn skill_does_not_create_agents_unless_asked() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    engram::init::write_skill(&root, false, false).unwrap();
    assert!(root.join(".grok/skills/engram/SKILL.md").is_file());
    assert!(!root.join("AGENTS.md").exists());
    std::fs::write(root.join("AGENTS.md"), "# hi\n").unwrap();
    engram::init::write_skill(&root, false, false).unwrap();
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(agents.contains("get_context"));
}

#[test]
fn skill_write_agents_creates_file() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    engram::init::write_skill(&root, true, true).unwrap();
    assert!(root.join("AGENTS.md").is_file());
    assert!(root.join(".claude/skills/engram/SKILL.md").is_file());
}
```

SKILL.md body (exact enough to test): must contain `get_context` and `stale_index` and “do not grep”.

`.mcp.json` snippet:

```json
{
  "mcpServers": {
    "engram": {
      "type": "stdio",
      "command": "engram",
      "args": ["mcp"]
    }
  }
}
```

`.cursor/mcp.json` same. Claude uses `.mcp.json` (same as copilot). `all` writes grok + copilot + cursor.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib init::`
Expected: FAIL on `write_harness`

- [ ] **Step 3: Implement merge writers + clap flags**

`engram init --harness grok --skill --write-agents`

If existing `.grok/config.toml` has other tables, append `[mcp_servers.engram]` only if missing.

- [ ] **Step 4: Fill `engram/testdata/miniapp`** with TSX, py, css, README, `.env` as specified in Task 7. Add a README in testdata explaining it is a fixture.

- [ ] **Step 5: Run `cargo test`**

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add engram/src/init.rs engram/src/main.rs engram/testdata
git commit -m "feat: write project MCP snippets and get_context skill"
```

---

### Task 12: Doctor, JSON byte cap, opt-in scale smoke

**Files:**
- Modify: `engram/src/doctor.rs`
- Modify: `engram/src/compile.rs` (ensure 16KB cap tested)
- Create: `engram/tests/scale.rs`

**Interfaces:**
- Consumes: `Store`, `get_context`, `index_repo`
- Produces: `run_doctor` prints: binary ok, grammars linked (python/ts/tsx/js), db path, schema version, ignore files present, harness files present/absent. Exit 0 even if harness missing (warn on stderr). Exit 3 if db unreadable.
- Scale test gated on `ENGRAM_SCALE_TEST=1`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn json_cap_sets_truncated() {
    // index many unique symbols then get_context with huge budget but compiler must still cap JSON
    let root = many_symbol_repo(200); // 200 tiny .py files each with unique def name_i
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "name", 100_000).unwrap();
    let bytes = serde_json::to_vec(&pkg).unwrap();
    assert!(bytes.len() <= 16_384);
    assert!(pkg.stats.truncated || bytes.len() < 16_384);
}

#[test]
fn scale_p95_optional() {
    if std::env::var("ENGRAM_SCALE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    let root = many_symbol_repo(10_000);
    index_repo(&root, true).unwrap();
    let start = std::time::Instant::now();
    let mut times = vec![];
    for i in 0..20 {
        let q = format!("name_{}", i * 10);
        let t0 = std::time::Instant::now();
        let _ = get_context(&root, &q, 3000).unwrap();
        times.push(t0.elapsed());
    }
    times.sort();
    let p95 = times[(times.len() * 95) / 100];
    assert!(p95.as_millis() <= 200, "p95 {:?} too slow (index {:?})", p95, start.elapsed());
}
```

`many_symbol_repo(n)` writes `f{i}.py` with `def name_{i}():\n    return {i}\n` plus `.engram` db created via `run_init`.

- [ ] **Step 2: Run tests to verify json cap behavior**

Run: `cd engram && cargo test --test scale json_cap`
Expected: FAIL if truncated flag not set; then implement pop-until-fits if not already done in Task 8.

- [ ] **Step 3: Doctor test**

```rust
#[test]
fn doctor_reports_db() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    let out = engram::doctor::run_doctor(&root).unwrap();
    assert!(out.contains("schema_version") || out.contains("schema"));
    assert!(out.contains("index.sqlite"));
}
```

Implement `run_doctor` as a multi-line string.

- [ ] **Step 4: Run full suite**

Run: `cd engram && cargo test`
Expected: PASS (scale test skips unless env set)

- [ ] **Step 5: Commit**

```bash
git add engram/src/doctor.rs engram/src/compile.rs engram/tests/scale.rs
git commit -m "test: JSON size cap, doctor, optional 10k-file scale smoke"
```

---

## Self-review (spec coverage)

| Spec section | Task |
|---|---|
| Complement MemPalace / no drawers | Global constraints; no task adds memory |
| Compiler-first `get_context` | 8, 10 |
| TS/JS/TSX/JSX/Python graph | 4, 5 |
| MD/CSS outline | 6 |
| Extractive, no LLM | 8 |
| SQLite schema, FTS5, WAL | 2 |
| Ignore, secrets, 1MB skip | 3, 7 |
| Incremental index, rayon, serialized writes | 7 |
| Ranking weights, caps, 16KB, stale | 8, 12 |
| MCP stdio, read-only tools, harnesses | 10, 11 |
| CLI + exit codes | 1, 9 |
| Large-repo query O(candidates) | 8 (caps), 2 (indexes), 12 (opt-in) |
| Skill / AGENTS.md | 11 |
| Non-goals (git, embeddings, watcher, HTTP) | not scheduled |

No TBD/TODO placeholders in tasks. Type names (`Store`, `get_context`, `ExtractedSymbol`, `ContextPackage`, `index_repo`) are consistent across tasks.

---

## Notes for the executor

- If tree-sitter crate versions fight, `cargo add` the latest compatible set; keep the query tests as the contract, not a particular crate minor.
- Prefer fixing queries over weakening tests. If a grammar node name differs, update the query, not the fixture.
- Do not add embeddings, Git parsing, or a watcher “while you are here.”
