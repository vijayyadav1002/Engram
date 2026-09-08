# Engram Architecture

Engram is a local, extractive context compiler for AI coding agents. One Rust binary indexes a repository into SQLite, then answers a question with a budgeted package of **verbatim source spans** — not summaries, embeddings, or cloud RAG.

This document describes the system as implemented in `engram/`. Product narrative and interview framing live in [`.doc/PROJECT_EXPLANATION.md`](.doc/PROJECT_EXPLANATION.md). Slice-level design specs live under `docs/superpowers/specs/`.

## 1. Position

| | Engram | MemPalace |
|---|---|---|
| Job | Compile engineering context from the **current repo** | Remember conversations, decisions, and notes |
| Unit | Symbol / span / edge | Drawer of text |
| Retrieval | Symbol lookup + FTS5 + 1-hop graph + commit FTS + ADR decisions | Semantic / hybrid over memories |
| Output | Budgeted `ContextPackage` of source | Quoted memories |
| LLM on the path | No | No (search) |

Engram does not store conversations, mine transcripts, or ship a daemon. Indexing is a CLI command. Querying is a read-only compiler, also exposed as MCP over stdio.

## 2. Runtime shape

One process. One database. No network on core paths.

```
Agent / human
    │
    ├── engram CLI  (init, index, get-context, status, doctor, search-*)
    └── engram mcp  (stdio JSON-RPC, read-only)
            │
            ├── Indexer  ──writes──►  SQLite  .engram/index.sqlite
            │     files + git log                 (WAL, FTS5)
            └── Compiler ──reads──►   SQLite + live disk spans
                    │                   (never spawns git)
                    └── optional: spawn local `mempalace search`
```

Repo root resolution (`engram/src/root.rs`):

1. `ENGRAM_ROOT` if set and is a directory.
2. Else walk up from cwd until `.engram/` or `.git` exists.
3. Never treat `$HOME` as a repo just because it exists. Missing root → `not_initialized` (CLI exit 2).

## 3. Crate map

The crate is `engram/` (binary + library). Modules:

| Module | Role |
|---|---|
| `main.rs` | Clap CLI |
| `init.rs` | `.engram/`, empty DB, harness snippets, skill / `AGENTS.md` |
| `index.rs` | Parallel walk, hash, parse, git log, SQLite write |
| `extract/` | Tree-sitter (TS/JS/Python), markdown headings/ADRs, CSS selectors |
| `store.rs` | SQLite schema, lookups, FTS, neighbors, commits |
| `compile.rs` | Query plan → retrieve → fuse → score → budget → `ContextPackage` |
| `mcp.rs` | Stdio MCP (`get_context`, debug tools, `index_status`) |
| `git.rs` | `GitSource` adapter; CLI `git log` at **index** time only |
| `palace.rs` | Optional local MemPalace CLI adapter |
| `hash.rs` | Blake3 file/content hashes |
| `ignore.rs` / `secret.rs` | Skip rules and secret-content regex |
| `root.rs` | Repo root discovery |
| `render.rs` | Text digest of a package |
| `doctor.rs` | `status` / `doctor` |
| `types.rs` | Shared enums and `ContextPackage` |

## 4. Language policy

Dispatch is by extension (`engram/src/extract/mod.rs`). Unparsed languages never invent symbols or edges.

| Depth | Extensions | What is stored |
|---|---|---|
| Graph | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` `.py` | Symbols + `import`/`call` edges |
| Outline | `.md` `.mdx` `.css` `.scss` | Headings or selectors. ADR markdown also gets a `decision` symbol and optional `supersedes` edges |
| File | everything else not ignored | `files` row + FTS body only |

`parse_status` on each file: `graph` | `outline` | `file` | `skipped` | `error`.

Rust, Go, and other languages are searchable as text in this version. They do not get a symbol graph.

Closed symbol kinds: `module`, `function`, `method`, `class`, `interface`, `type`, `component`, `heading`, `selector`, `decision`.

Edge kinds: `import` | `call` | `supersedes` (ADR → ADR only). Confidence: `high` (import-resolved, same-file unambiguous, or a `supersedes` target that exists) | `low` (name-only guess).

## 5. Data store

Path: `<repo>/.engram/index.sqlite` (gitignored). Schema version `2`. Opening a v1 DB on write migrates in place (commit tables + `meta` git columns). Read-only v1 treats git as empty until the next `engram index`.

Pragmas: `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`. Query paths open with `query_only=ON`.

```
meta          schema_version, indexed_at, root, file/symbol/edge counts,
              git_head, commit_count, git_status
files         path, language, blake3 hash, size, mtime, parse_status
symbols       name, kind, line/byte range, signature
edges         (src, dst, kind) + confidence
file_fts      FTS5(path, content) tokenize=unicode61
commits       sha, author, authored_at, subject, body (body capped at 800 chars)
commit_files  (commit_id, path)     — ranking metadata only; never quoted
commit_fts    FTS5(sha, subject, body)
```

Indexes: `symbols(name COLLATE NOCASE)`, `symbols(file_id)`, `edges` src/dst, `files(hash)`, `commits(sha)`.

File **bodies are not the source of truth for quotes**. FTS stores content for ranking. The compiler re-reads code/ADR spans from disk at query time and checks the live Blake3 hash against `files.hash`. Git items are prequoted from the `commits` row (`git://<full sha>`); they are not re-read from `git show`.

## 6. Indexer (`engram index`)

Batch command. No file watcher.

### 6.1 Walk and skip

`ignore::WalkBuilder` honors `.gitignore` and `.engramignore`. Built-in skips even if ignore files are missing:

- Dirs: `.git`, `node_modules`, `.venv`, `venv`, `__pycache__`, `dist`, `build`, `.engram`, `.next`, `target`
- Secret names: `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `credentials.json`
- Noise: lockfiles, `*.min.js`, `*.map`, images, archives, fonts, wasm
- Binary: NUL byte in the file
- Size: over 1 MiB (`MAX_FILE_BYTES`)
- Secret content (match only, never stored): PEM private-key headers, `AKIA…` AWS keys, `ghp_…`, `sk-…`

Skipped files are counted (`skipped_secret` / `skipped_large` / `skipped_ignore`); contents are not written.

### 6.2 Incremental update

For each surviving file: Blake3 the bytes. Unchanged hash → keep rows. Changed → replace that file’s symbols, edges, and FTS row. Missing paths → delete. `meta` updates in the final write.

`--force` rebuilds files **and** commit tables.

### 6.3 Parallelism

Hash and parse with Rayon. SQLite writes on one connection. One file’s AST is dropped after its rows are queued. Query memory is O(candidates), not O(repo).

Parse failure sets `parse_status=error` and continues; indexing does not abort the repo.

### 6.4 Git history (index time only)

After the file walk, if `root/.git` exists, the indexer runs `git log` via `GitSource` (`engram/src/git.rs`). Production is argv-only (`git -C <root> …`, `ENGRAM_GIT_BIN` override, 120s timeout). Tests inject `FakeGitSource`. **`get_context` never spawns git.**

- Range: ancestors of `HEAD`. If stored `git_head` is an ancestor of current HEAD → `git_head..HEAD`; else `git log HEAD` with `INSERT OR IGNORE`.
- Stores sha, author name (no email), date, subject, body, and paths touched. No diffs, blame, or `--all`.
- Missing git, no `.git`, timeout, or unparseable log → skip history, still finish the file index. `git_status`: `ok` | `absent` | `not_installed` | `timeout` | `unparseable`.

ADR detection (markdown only): a path is an ADR if a directory segment is `adr` / `adrs` / `decisions`, or the filename matches `adr[-_ ]?<digits>`. One `decision` symbol covers the `## Decision` section (else H1 / first 80 lines). `supersedes ADR-N` becomes a `supersedes` edge when the target decision symbol exists.

## 7. Compiler (`get_context`)

Pure read. Default budget **3000 tokens**. Serialized JSON cap **16 384 bytes** (under typical 20KB MCP tool-result limits).

```
query
  → plan_query          (no LLM)
  → symbol hits (cap 50)
  → FTS hits    (cap 30 files)
  → 1-hop neighbors of accepted symbols (cap 40; high first, then low)
  → decision symbols (cap 10) + supersedes neighbors (cap 10)
  → commit FTS + subject-token match (cap 20; subject hits kept even if FTS is full)
  → fuse identical (path, start, end)
  → score
  → merge overlapping spans; keep tighter symbol span inside an FTS hit
  → first pass: at most 2 spans per file; overflow if budget remains
  → re-read disk + Blake3 for file/ADR spans; omit stale
  → git:// items use stored subject+body (no hash re-read)
  → pack until token budget, then JSON cap
  → optional palace attachment
```

### 7.1 Query plan

Deterministic (`compile::plan_query`):

- Quoted strings → `symbol_terms`
- Path-like tokens (`src/auth`, `.tsx`) → `path_hints`
- CamelCase / `snake_case` / dotted identifiers → `symbol_terms`
- Stopwords (`the`, `a`, `an`, `is`, `in`, `for`, `of`, `where`, `how`, `what`) dropped
- Remainder → `fts_query`

### 7.2 Rank

Sum of weights. No learned model.

| Signal | Weight |
|---|---|
| Exact symbol name | +5.0 |
| Prefix symbol | +3.0 |
| FTS | +2.0 × reciprocal rank (`1 / (1 + hit_index)`). SQLite FTS5 BM25 orders the hits; the compiler does not ingest the raw BM25 value |
| Heading or selector | +2.0 |
| High-confidence neighbor | +1.5 |
| Low-confidence neighbor | +0.5 |
| Path hint match | +1.0 |
| Same file as an exact symbol | +1.0 |
| Commit FTS | +2.0 × reciprocal rank |
| Commit path overlap (`commit_path`) | +1.0 |

Exact/prefix **decision** names reuse the +5 / +3 symbol weights (also tagged `decision`). No extra boost for the word “why”. Git and ADRs compete with code for the same budget. Palace still attaches last, only if opted in.

Git item: `path` = `git://<full sha>`, `kind` = `commit`, `symbol` = 7-char sha, `text` = `{subject}\n\n{body}`. ADR item: real file path, `kind` = `decision`, disk re-read like code. `commit_files` never become items.

`why` tags on items: `exact_symbol`, `prefix_symbol`, `fts`, `import_neighbor`, `call_neighbor`, `heading`, `selector`, `path_hint`, `commit`, `commit_fts`, `commit_path`, `decision`, `supersedes_neighbor`, and optionally `palace`.

Bare FTS hits with no heading become a span of the first **40 lines** of the file.

### 7.3 Budget, staleness, tokens

- Token cost = whitespace-separated count of emitted `text` plus 2. This is a cap estimator, not a billing tokenizer.
- Stale hash or unreadable file → omit item, increment `stale_omitted`.
- `stats.stale_index` is true when a **majority** of attempted quotes were stale: `stale_omitted * 2 > items.len + stale_omitted`.
- JSON over 16KB pops items from the tail and sets `stats.truncated`.
- Empty retrieval → empty `items`, populated `stats`. Never invents text.

## 8. `ContextPackage`

Returned by MCP `get_context` and `engram get-context --json`:

```json
{
  "query": "where is auth handled?",
  "budget_tokens": 3000,
  "used_tokens": 1180,
  "items": [
    {
      "path": "src/auth/session.ts",
      "start_line": 12,
      "end_line": 48,
      "symbol": "createSession",
      "kind": "function",
      "text": "export function createSession(...) { ... }",
      "why": ["exact_symbol"]
    }
  ],
  "edges": [
    {
      "from": "src/middleware.ts:requireAuth",
      "to": "src/auth/session.ts:createSession",
      "kind": "call",
      "confidence": "high"
    }
  ],
  "stats": {
    "files_considered": 24,
    "symbols_considered": 18,
    "dropped_for_budget": 7,
    "stale_omitted": 0,
    "stale_index": false,
    "truncated": false,
    "git": {
      "status": "ok",
      "commits_considered": 3,
      "included": 1
    }
  }
}
```

`stats.git` is always present (`ok` if the index has commit rows, else `absent`). `stats.palace` is omitted when attachment is off.

CLI default is the same package rendered as a compact text digest (path, line range, fenced text). `text` is untrusted repository data, never instructions.

## 9. MCP

Command: `engram mcp`. Transport: newline-delimited JSON-RPC 2.0 on stdio. Protocol version `2024-11-05`. Logs on stderr; stdout is protocol only.

The server never writes the index. Indexing stays `engram index`.

| Tool | Arguments | Role |
|---|---|---|
| `get_context` | `query` (required), `budget_tokens` (default 3000), `include_palace` | Product. Call this before grepping the repo. |
| `search_symbols` | `name`, `limit` (default 20) | Debug: symbol stage only |
| `search_code` | `query`, `limit` (default 20) | Debug: FTS stage only |
| `index_status` | none | DB path, schema, counts, `indexed_at`, stale-hash sample |

Missing index → `not_initialized`. Indexer holding the write lock → `index_busy`.

Harness wiring (`engram init --harness`):

| id | File |
|---|---|
| `grok` | `.grok/config.toml` |
| `copilot` / `claude` | `.mcp.json` |
| `cursor` | `.cursor/mcp.json` |
| `all` | merge into all of the above |

Same argv everywhere: `engram` `["mcp"]`. `--skill` writes `.grok/skills/engram/SKILL.md`. `AGENTS.md` is created only with `--write-agents`; if it already exists the blurb is appended.

## 10. Optional MemPalace bridge

Off by default. After the code package is packed, Engram may attach up to **3 verbatim drawers** if:

1. Caller opted in: MCP `include_palace: true` / CLI `--palace`, else `ENGRAM_PALACE=1`, else `.engram/config.toml` `palace = true`. Explicit disable always wins.
2. `mempalace` is on `PATH` (or `ENGRAM_PALACE_BIN`).
3. Remaining token budget ≥ 200 (`PALACE_MIN_REMAINING`).

It spawns `mempalace search --results 3 <query>` in the repo root (timeout 8s). Palace failure or absence never fails `get_context`. Drawers do not participate in symbol ranking or stale-hash checks. Each body is truncated at 1200 characters.

Palace items reuse `ContextItem`: `path` is `palace://{wing}/{room}`, `kind` is `palace`, `why` contains `palace`. Code items keep budget priority. If code and palace disagree, **current code wins**.

## 11. CLI

| Command | Behavior |
|---|---|
| `engram init` | Create `.engram/` and empty DB. Optional `--harness`, `--skill`, `--write-agents` |
| `engram index` | Incremental index. `--force` rebuilds |
| `engram get-context "…"` | Compiler. `--json`, `--budget N`, `--palace` |
| `engram search-symbols NAME` | Debug |
| `engram search-code "…"` | Debug |
| `engram status` | DB path, schema, file/symbol/commit counts, `git_head`, last index, stale sample |
| `engram doctor` | Binary, grammars, DB, ignore rules, harness config |
| `engram mcp` | Stdio MCP server |

Exit codes: `0` ok, `1` usage, `2` not initialized, `3` index/IO error.

No telemetry.

## 12. Performance envelope

Targets from the core spec (laptop, SSD, after ignores). Not a measured SLA in this tree.

| Operation | Target |
|---|---|
| `get_context` p95 | ≤ 200ms on a ~50k-file index |
| `engram mcp` startup | ≤ 100ms |
| Incremental index | proportional to changed files |
| Cold index ~10k files | ≤ 30s |
| Cold index ~100k files | must complete without OOM |

Retrieval caps keep the query path O(candidates). MCP does not preload the graph into RAM.

## 13. Explicit non-goals (this version)

- Embeddings / vector search / LLM rerank
- File-watcher daemon, HTTP/SSE MCP, cloud relay, team server
- Git diffs, blame, co-change (`git log` messages and ADR files **are** indexed)
- A `save_decision` store or `search_git` MCP tool
- Cross-language call graph; CSS-modules-to-TSX edges
- Tree-sitter graphs for Rust, Go, and other non-listed languages
- Multi-repo search (one index per repo root)
- Writing to MemPalace from `get_context`

## 14. Related documents

- [`docs/superpowers/specs/2026-09-07-engram-core-design.md`](docs/superpowers/specs/2026-09-07-engram-core-design.md) — core spec
- [`docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md`](docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md) — palace attachment
- [`docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md`](docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md) — MemPalace 3.3.x CLI parse
- [`docs/superpowers/specs/2026-09-08-engram-git-decisions-design.md`](docs/superpowers/specs/2026-09-08-engram-git-decisions-design.md) — git commits + ADR extract
- [`README.md`](README.md) — install and usage
- [`AGENTS.md`](AGENTS.md) — agent router (code vs palace)
