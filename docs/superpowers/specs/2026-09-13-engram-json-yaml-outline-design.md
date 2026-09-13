# Engram JSON / YAML Outline Design

Date: 2026-09-13
Status: implemented on main
Scope: outline extraction for `.json` / `.yaml` / `.yml` (two-level dotted key names)
Depends on: `docs/superpowers/specs/2026-09-07-engram-core-design.md`
Related: language-support scan in `.prompts/update.prompt.md` (Bruno, Cucumber, embedded GraphQL, and filename-aware package.json / Helm special cases are **not** this slice)

## 1. Executive summary

JSON and YAML config files are file-rows plus FTS only, so `get_context "scripts"` cannot quote the `scripts` block in `package.json`, and `get_context "services"` cannot quote a Helm `services:` mapping.

This slice adds tree-sitter outline extractors. After `engram index`, top-level object keys and one nested object level become `heading` symbols (`scripts`, `scripts.test`, `services`, `services.web`). `get_context` quotes those pair spans through the current compiler. No new `SymbolKind`, no edges, no schema migration, no ranking changes, no filename special cases, no `.jsonc` dispatch.

## 2. Goals

- After `engram index` on a repo that contains `.json` / `.yaml` / `.yml` files, `get_context "scripts"` quotes the top-level `scripts` pair span.
- `get_context "scripts.test"` quotes the nested `test` pair under `scripts`.
- `search_symbols` for `scripts.test` returns that nested key. Bare `test` is not a JSON/YAML symbol.
- Parse failure on one JSON/YAML file never aborts repo indexing (`parse_status=error`, `files` row kept).
- Extractive only: quote source spans. No LLM, no schema validation, no `$ref` resolution.

## 3. Non-goals

- Filename-aware special cases (`package.json` scripts, `tsconfig` `compilerOptions`, Helm `values.services`, GitHub `jobs`)
- `.jsonc` dispatch (stays file-only). Comments **inside** `.json` are allowed if the JSON grammar accepts them
- Array indices (`jobs[0]`, `containers.0`)
- Nested depth beyond two object levels (`a.b.c`)
- JSON Schema, OpenAPI `$ref`, YAML merge-key expansion, anchors as symbols
- Bruno / Cucumber extractors
- Embedded GraphQL in TypeScript
- New `SymbolKind` / `EdgeKind`
- Compiler ranking changes, new MCP tools, SQLite schema bump
- New skip rules for generated manifests beyond Core’s 1MB / lockfiles / secrets / `.engramignore`

Those stay later slices.

## 4. Locked decisions

1. JSON and YAML are **outline** languages for `.json`, `.yaml`, and `.yml` only.
2. Parsers are **tree-sitter**: `tree-sitter-json` 0.24.x and `tree-sitter-yaml` 0.7.x (both expose `LANGUAGE` as `LanguageFn`, same as GraphQL). Not `serde_json` value dumps, not `serde_yaml` / `serde-saphyr` for this slice (no reliable byte spans).
3. Reuse `SymbolKind::Heading`. No JSON/YAML-specific kind.
4. Names: top-level key `scripts`; nested object key `scripts.test`. Join with `.`. Do not emit the nested key as a bare `test`.
5. Arrays do not contribute names. A mapping whose value is an array still emits the parent key (`jobs`) and stops.
6. Span is the **pair / mapping entry** (key through value), so the quote is the useful block.
7. No edges. Outline row stays “no edges” (ADR `supersedes` is markdown-only).
8. Same rules for every file. `dependencies.react` is a symbol because `dependencies` is a mapping, not because the file is `package.json`.
9. Existing `get_context` path is unchanged. Prefix lookup of `scripts` also matches `scripts.test` (`LIKE 'scripts%'`). Exact `scripts` still outranks those prefix hits (weight 5 vs 3). Do not change ranking.
10. `language` on the `files` row is `json` for `.json` and `yaml` for `.yaml` / `.yml`.
11. `.jsonc` is not dispatched. Lockfiles stay skipped (`package-lock.json`, `pnpm-lock.yaml`, …).

## 5. Architecture

One process, one SQLite file. Indexer already calls `extract::extract_path`. This slice adds two language arms.

```
engram index
  → file walk (unchanged skip rules)
  → extract_path(".json") → extract/json.rs
  → extract_path(".yaml"|".yml") → extract/yaml.rs
  → heading symbols + FTS body (unchanged store)

get_context
  → plan_query → symbol exact/prefix + FTS
  → quote disk spans (unchanged)
```

Core language policy (spec 2026-09-07 §8) outline row becomes:

| Depth | Extensions | Extractor |
|---|---|---|
| Graph | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` `.py` `.graphql` `.gql` | unchanged |
| Outline | `.md` `.mdx` `.css` `.scss` **`.json` `.yaml` `.yml`** | headings, selectors, or two-level keys (`kind=heading`); no edges |
| File | everything else, including `.jsonc` | unchanged |

Unparsed languages still must not grow fake symbols. JSON/YAML parse failure is `error`, not invented outline keys.

Do not emit a `<file>` `Module` symbol (markdown and CSS outlines do not).

## 6. Components

### 6.1 `extract/json.rs`

`pub fn extract(source: &str) -> Extraction`

1. `Parser::set_language(&tree_sitter_json::LANGUAGE.into())`; on failure return `error_extraction()`.
2. Parse; if no tree, `error`. If root has no children and `has_error()`, `error`.
3. Walk the tree (not a ranking change). Query compile is not required; a cursor walk of `pair` nodes is the implementation.
4. Status `Outline` when the walk completes. Edges always empty.

JSON grammar nodes (`tree-sitter-json` 0.24.x): root `document` → `object` | `array` | scalar; `object` children are `pair` with fields `key` (`string`) and `value`; `comment` is a named node and is not a symbol.

### 6.2 `extract/yaml.rs`

`pub fn extract(source: &str) -> Extraction`

Same error/status shape as JSON, using `tree_sitter_yaml::LANGUAGE`.

YAML grammar nodes (`tree-sitter-yaml` 0.7.x): root `stream` → one or more `document`; mappings are `block_mapping` (`block_mapping_pair`) or `flow_mapping` (`flow_pair`); pair fields are `key` and `value`. Apply the two-level rule **independently to each document**.

### 6.3 Dispatch

`extract_path` (in `extract/mod.rs`), with the other outline arms:

- lowercased path ends with `.json` → `json::extract(source)`
- lowercased path ends with `.yaml` or `.yml` → `yaml::extract(source)`
- `.jsonc` is not dispatched (stays `ParseStatus::File`)

### 6.4 Indexer language tag

`language_of` in `index.rs`:

- `.json` → `Some("json")`
- `.yaml` / `.yml` → `Some("yaml")`

Update `language_strings_match_brief`.

### 6.5 Doctor

Extend the existing `grammar_status` probe list with `json` and `yaml`. Do not add a new doctor section or JSON/YAML file-count line.

## 7. Symbol table

| Construct | Grammar node | `name` | `kind` | `signature` | Span |
|---|---|---|---|---|---|
| Top-level object key | JSON `pair` / YAML `block_mapping_pair` or `flow_pair` on the document mapping | unquoted key (`scripts`) | `Heading` | `key` | whole pair |
| Nested object key (exactly one level) | pair whose parent mapping is the **value** of a top-level pair | `{parent}.{key}` (`scripts.test`) | `Heading` | `key` | nested pair |

`parent` is the unquoted top-level key. Do not walk deeper. Do not use array elements as parents.

**Key text:**

- JSON: the `string` key node. Prefer a `string_content` child if present; otherwise strip one pair of surrounding `"` from the node text. JSON escape sequences stay as source text (do not unescape `\uXXXX`). Empty or whitespace-only → skip.
- YAML: unwrap `flow_node` wrappers. Accept `string_scalar`, `boolean_scalar`, `integer_scalar`, `float_scalar`, `null_scalar`, `timestamp_scalar` (source text as name), `double_quote_scalar` / `single_quote_scalar` (strip one pair of matching quotes). Skip: aliases, merge key `<<`, complex keys (`block_node` / nested mapping/sequence as key), empty/whitespace names.

**Not symbols:**

- Array elements and numeric indices
- Keys at depth 3 or more
- Comments
- YAML anchors, aliases, tags, directives
- JSON/YAML scalars or arrays at the document root (file stays `outline` with zero key symbols)
- `.jsonc` files (never reach the extractor)

Duplicate keys in one file produce two rows (same as JS / GraphQL).

## 8. Two-level walk

For a document whose root value is an object/mapping `M`:

```
for pair in M:
    name = key_text(pair)
    if name empty: continue
    emit Heading(name, span=pair)
    V = value of pair
    if V is object/mapping:
        for nested in V:
            child = key_text(nested)
            if child empty: continue
            emit Heading(name + "." + child, span=nested)
    # if V is array or scalar: stop
```

If the document root is an array or scalar: emit no key symbols; status is still `Outline` when parse succeeded.

Examples (all intended):

| File | Symbols |
|---|---|
| `{ "scripts": { "test": "vitest" }, "name": "app" }` | `scripts`, `scripts.test`, `name` |
| `{ "dependencies": { "react": "18" } }` | `dependencies`, `dependencies.react` |
| Helm `services:\n  web:\n    port: 80` | `services`, `services.web` |
| GitHub `jobs:\n  build:\n    runs-on: ubuntu` | `jobs`, `jobs.build` |
| `{ "items": [ { "id": 1 } ] }` | `items` only |
| `[ { "a": 1 } ]` | none |
| YAML multi-doc `---\na: 1\n---\nb:\n  c: 2` | `a`, `b`, `b.c` |

## 9. Error handling

Match GraphQL/Python:

- Grammar load / parse failure → `ParseStatus::Error`, empty symbols and edges.
- Partial trees: keep pairs whose nodes do not `has_error()`.
- JSON `comment` nodes are not symbols. Extract pairs around them. If the file is invalid JSON/YAML, `error`.
- YAML `set_language` failure (ABI mismatch with `tree-sitter` 0.25) → `error` for that file; do not abort the repo index; doctor reports `yaml` as failed if the probe cannot `set_language`.
- Indexer `catch_unwind` per file already covers panics.
- Skip rules unchanged. A generated `values.yaml` under 1MB is indexed. Operators exclude noise with `.engramignore`.

## 10. Testing

TDD. Tests live next to the extractors (`extract/mod.rs` and/or `extract/json.rs` / `extract/yaml.rs`). Work in `engram/`. Run `cd engram && cargo test …`.

**Unit (string sources):**

1. JSON object top-level + nested object → `scripts` (`Heading`), `scripts.test` (`Heading`). Bare `test` is absent.
2. JSON mapping of packages → `dependencies`, `dependencies.react`.
3. JSON array value → parent key only (`items`), no `items.0` / `id`.
4. JSON root array / root scalar → `Outline`, zero heading symbols.
5. JSON junk `{` → `Error`, empty symbols.
6. JSON with a `//` comment node → keys still extracted.
7. YAML block mapping → `services`, `services.web`.
8. YAML flow mapping `{a: {b: 1}}` → `a`, `a.b`.
9. YAML sequence value → parent key only.
10. YAML multi-document → keys from each document.
11. YAML merge `<<` and alias keys → not symbols.
12. Dispatch: `.json` / `.yaml` / `.yml` are `Outline`; `.jsonc` remains `File`.
13. Quoted YAML / JSON keys strip surrounding quotes in `name`.

**Indexer / compiler fixture:**

Add `engram/testdata/miniapp/package.json` with `name` and a `scripts.test` mapping, and `engram/testdata/miniapp/values.yaml` with a `services.web` mapping. After `index_repo` of `testdata/miniapp`:

- `get_context "scripts"` includes `package.json` and the `scripts` pair span
- `get_context "scripts.test"` includes the nested pair span
- `get_context "services"` includes `values.yaml`
- `search_symbols` for `scripts.test` returns that symbol; `search_symbols` for `test` does not return a JSON/YAML heading named `test`

No golden-query overhaul. No Synxis-sized Helm tree in CI.

## 11. Files touched

| File | Responsibility |
|---|---|
| `engram/Cargo.toml` | `tree-sitter-json` 0.24.x, `tree-sitter-yaml` 0.7.x |
| `engram/src/extract/json.rs` | JSON walk, heading symbols |
| `engram/src/extract/yaml.rs` | YAML walk, heading symbols |
| `engram/src/extract/mod.rs` | `mod json` / `mod yaml`; dispatch; unit tests |
| `engram/src/index.rs` | `language_of("json"|"yaml")` |
| `engram/src/doctor.rs` | grammar probe list |
| `engram/testdata/miniapp/package.json` | tiny JSON fixture |
| `engram/testdata/miniapp/values.yaml` | tiny YAML fixture |
| `engram/testdata/README.md` | mention JSON/YAML in the fixture list |
| `engram/tests/json_yaml.rs` | index + `get_context` / `search_symbols` success bar |
| `docs/superpowers/specs/2026-09-07-engram-core-design.md` | add `.json` `.yaml` `.yml` to the §8 outline row |
| `README.md` | add those extensions to “What it understands” outline row |

No changes to `types.rs`, `store.rs`, `compile.rs`, or MCP tool schemas.

## 12. Success bar

All of the following after `engram index` on the fixture (and on a repo that has real JSON/YAML):

- `get_context "scripts"` package contains the `scripts` pair span from `package.json`.
- `get_context "scripts.test"` package contains the nested pair span.
- `search_symbols` for `scripts.test` returns the nested key.
- `search_symbols` for `test` does not return a JSON/YAML heading named `test`.
- `get_context "services"` package contains the Helm-style mapping span from `values.yaml`.
- `cargo test` in `engram/` passes.
- `engram doctor` grammar line includes `json` and `yaml` among the ok grammars when both crates link.

## 13. Later slices (explicitly not here)

1. Filename-aware outlines (package.json scripts vs dependencies, tsconfig `compilerOptions`, Helm, GitHub Actions `jobs`).
2. Bruno `.bru` and Cucumber `.feature` outlines.
3. `.jsonc` dispatch; JSON Schema / OpenAPI `$ref`.
4. Array-of-objects outlines (`items[].id`) if name lookup proves too coarse.
5. YAML merge-key expansion and anchor targets as symbols.
6. Depth > 2 if two-level misses real config questions.
