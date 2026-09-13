# Engram GraphQL Extractor Design

Date: 2026-09-13
Status: implemented on main
Scope: graph extraction for standalone `.graphql` / `.gql` files (name lookup)
Depends on: `docs/superpowers/specs/2026-09-07-engram-core-design.md`
Related: language-support scan in `.prompts/update.prompt.md` (YAML/JSON, Bruno, Cucumber, and embedded GraphQL in TypeScript are **not** this slice)

## 1. Executive summary

Engram’s graph languages are TypeScript/JavaScript and Python. GraphQL schema and operation files are file-rows plus FTS only, so `get_context "Reservation"` cannot quote the type definition.

This slice adds a tree-sitter GraphQL extractor. After `engram index`, named types, operations, fragments, and qualified fields are symbols in the existing SQLite tables. `get_context` quotes those spans through the current compiler. No new MCP tools, no schema migration, no ranking changes, no TypeScript template-literal extraction, no cross-language edges to resolvers.

## 2. Goals

- After `engram index` on a repo that contains `.graphql` / `.gql` files, `get_context "Reservation"` quotes the `type Reservation` (or enum/union/input/scalar) span.
- `get_context "GetReservation"` quotes the named operation span. `get_context "ReservationFields"` quotes the named fragment span.
- Field definitions are retrievable as qualified names (`Reservation.id`, `Mutation.createBooking`) without making bare `id` a symbol.
- Parse failure on one GraphQL file never aborts repo indexing (`parse_status=error`, `files` row kept).
- Extractive only: quote source spans. No LLM, no schema stitching, no SDL validation.

## 3. Non-goals

- YAML / JSON / Bruno / Cucumber extractors
- Embedded GraphQL in TypeScript/JavaScript template literals
- Cross-language edges (UI call → operation → schema → resolver)
- `extend type` / `extend interface` / other type-system extensions
- Anonymous operations (`{ field }` or `query { field }` with no name)
- Directive definitions as symbols; argument names as symbols
- `.graphqls` dispatch (stays file-only)
- Field-type edges (`Reservation.id` → `ID`); union-member edges; cross-file schema resolution
- Custom `schema { query: RootQuery }` root-type names (operation `Call` targets always use `Query` / `Mutation` / `Subscription`)
- New `SymbolKind` / `EdgeKind` variants
- New skip rules for generated schemas beyond Core’s 1MB / `node_modules` / secrets / `.engramignore`
- Compiler ranking changes, new MCP tools, SQLite schema bump

Those stay later slices.

## 4. Locked decisions

1. GraphQL is a **graph** language for `.graphql` and `.gql` only.
2. Parser is **tree-sitter** via `tree-sitter-graphql` 0.2.x (compatible with Engram’s `tree-sitter` 0.25). Not `async-graphql-parser`.
3. Reuse existing kinds. No GraphQL-specific `SymbolKind`.
4. Field **definitions** (SDL object/interface/input fields and enum values) use qualified names `Parent.name`. Selections inside operations and fragments are **not** symbols.
5. Name lookup is the product. Edges exist but are not the ranking target.
6. Existing `get_context` path is unchanged: exact / prefix symbol match, FTS, 1-hop neighbors, same caps. Unqualified names such as `createBooking` may still retrieve the file via FTS and Core’s file-symbol promotion; they are not extra aliases.
7. Prefix lookup of `Reservation` will also match `Reservation.id` (`LIKE 'Reservation%'`). Exact match of the type still outranks those prefix hits (weight 5 vs 3). Do not change ranking in this slice.
8. `language` on the `files` row is `graphql` for both extensions.

## 5. Architecture

One process, one SQLite file. Indexer already calls `extract::extract_path`. This slice only adds a language arm.

```
engram index
  → file walk (unchanged skip rules)
  → extract_path(".graphql"|".gql") → extract/graphql.rs
  → symbols + edges + FTS body (unchanged store)

get_context
  → plan_query → symbol exact/prefix + FTS + 1-hop
  → quote disk spans (unchanged)
```

Core language policy (spec 2026-09-07 §8) gains one row:

| Depth | Extensions | Extractor |
|---|---|---|
| Graph | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` `.py` **`.graphql` `.gql`** | tree-sitter |
| Outline | `.md` `.mdx` `.css` `.scss` | unchanged |
| File | everything else, including `.graphqls` | unchanged |

Unparsed languages still must not grow fake symbols. GraphQL parse failure is `error`, not invented outline symbols.

## 6. Components

### 6.1 `extract/graphql.rs`

`pub fn extract(source: &str) -> Extraction`

Same shape as `extract/python.rs`:

1. `Parser::set_language(&tree_sitter_graphql::LANGUAGE.into())`; on failure return `error_extraction()`.
2. Parse; if no tree, `error`. If root has no children and `has_error()`, `error`.
3. Run a tree-sitter query. Query compile failure → `error`.
4. Always emit `<file>` `Module` covering the root node when status is `graph`.
5. Ignore captures whose node `has_error()` or whose ancestor is a `type_extension` / `schema_extension`.

### 6.2 Dispatch

`extract_path` (in `extract/mod.rs`): after the JS/TS/Python arms, if the lowercased path ends with `.graphql` or `.gql`, call `graphql::extract(source)`.

`.graphqls` is not dispatched.

### 6.3 Indexer language tag

`language_of` in `index.rs` returns `Some("graphql")` for those two extensions. Update the existing `language_strings_match_brief` test.

### 6.4 Doctor

Extend the existing `grammar_status` probe list with `graphql`. Do not add a new doctor section or GraphQL file-count line.

## 7. Symbol table

| Construct | Grammar node | `name` | `kind` | `signature` | Span |
|---|---|---|---|---|---|
| Object type | `object_type_definition` | `Reservation` | `Type` | `type` | whole definition |
| Interface | `interface_type_definition` | `Node` | `Interface` | `interface` | whole definition |
| Enum | `enum_type_definition` | `Status` | `Type` | `enum` | whole definition |
| Union | `union_type_definition` | `Vehicle` | `Type` | `union` | whole definition |
| Input | `input_object_type_definition` | `CreateBookingInput` | `Type` | `input` | whole definition |
| Scalar | `scalar_type_definition` | `DateTime` | `Type` | `scalar` | whole definition |
| Named operation | `operation_definition` with a `name` | `GetReservation` | `Function` | `query` / `mutation` / `subscription` (from `operation_type`) | whole operation |
| Named fragment | `fragment_definition` | `ReservationFields` | `Type` | `fragment` | whole fragment |
| Object/interface field definition | `field_definition` | `{Parent}.{field}` | `Method` | `field` | field definition |
| Input field | `input_value_definition` directly under `input_fields_definition` | `{Parent}.{field}` | `Method` | `field` | input field |
| Enum value | `enum_value_definition` | `{Parent}.{VALUE}` | `Method` | `enum_value` | enum value |
| File | root | `<file>` | `Module` | none | file |

`Parent` is the nearest enclosing type/interface/enum/input **definition** name. Do not use extension nodes as parents.

**Not symbols:**

- Anonymous operations (no `name` child on `operation_definition`, including shorthand `{ ... }`)
- `type_extension` / `schema_extension` / `directive_definition` / `schema_definition`
- Arguments (`arguments_definition` / `input_value_definition` under a field, not under `input_fields_definition`)
- Operation/fragment **selections** (`field` under `selection_set`)
- Union members
- Descriptions, comments, `#import` preludes

Duplicate names in one file produce two rows (same as JS).

## 8. Edges

Existing `EdgeKind` only. Emit after symbols for the same file.

| When | `src_name` | `kind` | `dst_name` | `confidence` |
|---|---|---|---|---|
| `implements_interfaces` on an object or interface definition | type/interface name | `Import` | each implemented `named_type` | `High` |
| `fragment_definition` | fragment name | `Import` | type condition `named_type` | `High` |
| Named operation with a selection set | operation name | `Call` | `{Root}.{first_root_field}` | `Low` |

`Root` is `Query`, `Mutation`, or `Subscription` from `operation_type`. First root field is the first `field` (not fragment spread) in the operation’s top-level `selection_set`, using the field’s `name` (not alias). If that selection is only a fragment spread, omit the `Call`.

No field-type `Import`s. No edges from selections to schema types in other files. Neighbor resolution uses existing name lookup; missing dest symbols simply produce no 1-hop hit.

## 9. Error handling

Match Python/TS:

- Grammar load / parse / query compile failure → `ParseStatus::Error`, empty symbols and edges.
- Partial trees: keep captures on nodes that do not `has_error()`.
- Mixed schema + operations in one file: extract every matching definition; do not require a pure SDL or pure executable document.
- Indexer `catch_unwind` per file already covers panics.
- Skip rules unchanged. A generated `schema.graphql` under 1MB is indexed.

## 10. Testing

TDD. Tests live next to the extractor (`extract/mod.rs` and/or `extract/graphql.rs`). Work in `engram/`. Run `cd engram && cargo test …`.

**Unit (string sources):**

1. Object type + fields → `Reservation` (`Type`, signature `type`), `Reservation.id` (`Method`).
2. Interface, enum + values, union, input + fields, scalar.
3. Named query/mutation/subscription → `Function` with the matching signature.
4. Named fragment → `Type` signature `fragment`, `Import` onto the type condition.
5. `implements A & B` → two `Import` edges, high confidence.
6. `query GetReservation { reservation { id } }` → `Call` `GetReservation` → `Query.reservation`, low confidence.
7. Anonymous operation, `extend type`, `directive @x on FIELD`, field arguments → no symbols for those constructs.
8. Dispatch: `.graphql` and `.gql` are `Graph`; `.graphqls` and `.rs` remain `File`.
9. Junk source → `Error`, empty symbols.

**Indexer / compiler fixture:**

Add `engram/testdata/miniapp/schema.graphql` with one object type, one named query, and one fragment. After `index_repo` of `testdata/miniapp`, `get_context "Reservation"` includes that path and the type span; `get_context "GetReservation"` includes the operation span.

No golden-query overhaul. No Synxis-sized schema in CI.

## 11. Files touched

| File | Responsibility |
|---|---|
| `engram/Cargo.toml` | `tree-sitter-graphql` 0.2.x |
| `engram/src/extract/graphql.rs` | parse, query, symbols, edges |
| `engram/src/extract/mod.rs` | `mod graphql`; dispatch; unit tests |
| `engram/src/index.rs` | `language_of("graphql")` |
| `engram/src/doctor.rs` | grammar probe list |
| `engram/testdata/miniapp/schema.graphql` | tiny schema + operation fixture |
| `docs/superpowers/specs/2026-09-07-engram-core-design.md` | add `.graphql` `.gql` to the §8 graph row; do not rewrite Core |

No changes to `types.rs`, `store.rs`, `compile.rs`, or MCP tool schemas.

## 12. Success bar

All of the following after `engram index` on the fixture (and on a repo that has real `.graphql` files):

- `get_context "Reservation"` package contains the object-type span.
- `get_context "GetReservation"` package contains the named operation span.
- `search_symbols` for `Reservation.id` returns the field.
- `search_symbols` for `id` does not return a GraphQL field named `id`.
- `cargo test` in `engram/` passes.
- `engram doctor` grammar line includes `graphql` among the ok grammars when the crate links.

## 13. Later slices (explicitly not here)

1. Filename-aware YAML/JSON outlines (package.json, tsconfig, Helm, CI).
2. Bruno `.bru` and Cucumber `.feature` outlines.
3. Embedded GraphQL in TS/JS template literals.
4. Generated-schema ignore rules.
5. Operation → schema field resolution across files; TS ↔ GraphQL edges.
6. `.graphqls` alias; `extend type`; GraphQL-specific `SymbolKind`s if name lookup proves too coarse.
