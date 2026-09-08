# Engram Git + Decisions Design

Date: 2026-09-08
Status: draft, pending user review
Scope: incremental git commit index + ADR extract as ranked `get_context` candidates (slice after Core + palace)
Depends on: `docs/superpowers/specs/2026-09-07-engram-core-design.md`
Related: `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md` (palace still last, opt-in)

## 1. Executive summary

Coding agents still run `git log` and hunt ADR files on “why did we…?” questions. Core already quotes current code. The palace bridge already quotes conversations when opted in. This slice adds the missing **repo-native history**: commit messages and existing ADR markdown, compiled into the same extractive `get_context` package.

Indexer walks `git log` incrementally into SQLite. Markdown ADRs become `decision` symbols (and optional `supersedes` edges). `get_context` treats those rows as extra candidates. They **compete with code for the same budget**. Query time never spawns git. Conversation decisions stay in MemPalace. There is no new decision store and no `save_decision`.

## 2. Goals

- After `engram index` on a git repo that contains ADRs, `get_context "why WebSockets"` can quote (a) the ADR Decision span, (b) a matching commit subject+body, and (c) the implementation span, in one package, under the existing token and 16KB JSON caps.
- Exact code symbols still outrank a weakly matching commit so “where is createSession?” stays code-first.
- Missing git, no `.git`, timeout, or a bad log never fail file indexing or `get_context`.
- Extractive only: quote commit messages and ADR file spans. No LLM, no diffs, no blame, no co-change.

## 3. Non-goals

- Diff hunks, blame, co-change, copy-paste detection
- `git log --all`, submodules, rename detection (`-M`), first-parent-only history
- A structured decision database or `save_decision` / `search_decisions` / `search_git` / `explain_change` MCP tools
- Nested git library (libgit2 / gitoxide)
- Spawning git at `get_context` time
- Query heuristics that skip history unless the question looks like “why”
- VS Code, embeddings, watcher, multi-repo
- Replacing MemPalace for conversation decisions

Those stay later slices.

## 4. Locked decisions

1. Evidence is **git commits + existing ADR markdown**. No new write API.
2. History is **ranked candidates** in every `get_context` call, not an opt-in second stage.
3. A git item quotes **subject + body only**. Sha lives in `path` (`git://<full sha>`) and `symbol` (7 chars). Author/date stay in SQLite, not in `text`. No patches.
4. **Incremental commit index** in SQLite. Git CLI at **index** time only.
5. Palace remains opt-in and runs **after** the ranked walk, filling leftover budget only.
6. MCP stays read-only. `get_context` is the product path.

## 5. Architecture

One process, one SQLite file. Indexer writes history; `get_context` only reads.

```
engram index
  → existing file walk (unchanged)
  → if `.git` exists: git log from last indexed SHA → commits + paths
  → markdown: ADR paths become `decision` symbols (+ `supersedes` if the file says so)

get_context
  → symbols + file FTS + graph  (as today)
  → + commit FTS + decision symbols + supersedes neighbors
  → fuse, rank, budget
  → palace still last, only if opted in
```

Git adapter is argv-only (`git -C <root> …`), no shell. `ENGRAM_GIT_BIN` overrides the program name (same idea as `ENGRAM_PALACE_BIN`). Tests inject a `GitSource` trait; default CI does not spawn git except an optional live smoke.

## 6. Schema (version 2)

Core tables are unchanged. `SCHEMA_VERSION` becomes **2**.

### 6.1 New tables

```sql
CREATE TABLE commits (
  id           INTEGER PRIMARY KEY,
  sha          TEXT NOT NULL UNIQUE,  -- full 40-char hex
  author       TEXT NOT NULL,         -- %an (name only, no email)
  authored_at  TEXT NOT NULL,         -- %aI ISO-8601
  subject      TEXT NOT NULL,         -- %s
  body         TEXT NOT NULL          -- %b, truncated at COMMIT_BODY_MAX_CHARS
);

CREATE TABLE commit_files (
  commit_id INTEGER NOT NULL REFERENCES commits(id) ON DELETE CASCADE,
  path      TEXT NOT NULL,            -- repo-relative POSIX as git prints it
  PRIMARY KEY (commit_id, path)
);

CREATE VIRTUAL TABLE commit_fts USING fts5(
  sha,
  subject,
  body,
  tokenize = 'unicode61'
);
```

`commit_files` is **ranking metadata only**. Never quoted into `ContextItem.text`.

### 6.2 `meta` columns

Add (SQLite `ALTER TABLE` on migrate):

| Column | Type | Meaning |
|---|---|---|
| `git_head` | TEXT NULL | HEAD sha last successfully walked |
| `commit_count` | INTEGER NOT NULL DEFAULT 0 | rows in `commits` |
| `git_status` | TEXT NOT NULL DEFAULT `'absent'` | see §11 |

### 6.3 Closed kinds

`SymbolKind` gains `decision` (`as_str` = `"decision"`).

`EdgeKind` gains `supersedes` (`as_str` = `"supersedes"`). Used only between `decision` symbols.

### 6.4 Migration

On `Store::open_write` / `create`:

- `create` writes schema 2 including commit tables and the extra `meta` columns.
- Opening a schema 1 DB: `CREATE TABLE` the three commit objects if missing; `ALTER TABLE meta ADD COLUMN` the three fields; set `schema_version = 2`. File/symbol/edge/FTS rows stay. Then index can fill commits.

Read-only open of schema 1: migrate is write-side. If MCP opens read-only against v1, treat commit queries as empty (`git` status `absent`) until the next `engram index` (which opens write and migrates). Do not fail `get_context`.

## 7. Git index

### 7.1 When it runs

After the file walk in `index_repo`. `--force` deletes all `commits` / `commit_files` / `commit_fts` rows (and resets `git_head`) before the walk.

If `root/.git` is missing → `git_status=absent`, no spawn.

### 7.2 Adapter

```rust
pub struct GitCommit {
    pub sha: String,
    pub author: String,
    pub authored_at: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<String>,
}

pub enum GitError { NotInstalled, NoRepo, Timeout, Unparseable, Io }

pub enum GitRange { Head, After(String) }  // After(sha) => sha..HEAD

pub trait GitSource {
    fn head_sha(&self, root: &Path) -> Result<String, GitError>;
    fn is_ancestor(&self, root: &Path, ancestor: &str, head: &str) -> Result<bool, GitError>;
    fn log(&self, root: &Path, range: GitRange) -> Result<Vec<GitCommit>, GitError>;
}
```

Production: `CliGitSource`. No shell; argv only. Stderr discarded except Engram stderr logs. Timeout **120s** per spawn (`GIT_TIMEOUT_MS = 120_000`).

`head_sha`: `git -C <root> rev-parse HEAD`

`is_ancestor`: `git -C <root> merge-base --is-ancestor <ancestor> <head>` (exit 0 = true, 1 = false, other = error)

`log`:

```text
git -C <root> log <range> --reverse --date=iso-strict
    --format=%x1e%H%x1f%an%x1f%aI%x1f%s%x1f%b
    --name-only --no-color
```

`<range>` is `HEAD` or `<git_head>..HEAD`. Parse records on `0x1e`, fields on `0x1f`. Paths are non-empty lines after the body field until the next record. Merge commits with no paths are stored. Truncate `body` to `COMMIT_BODY_MAX_CHARS` (800) and append `…` if truncated **when writing the row**.

### 7.3 Incremental rule

1. Resolve `head = head_sha()`.
2. If `meta.git_head` is `Some(old)` and `is_ancestor(old, head)` is `Ok(true)` → `log(After(old))`.
3. Else (including `is_ancestor` error or `Ok(false)`) → `log(Head)` and `INSERT OR IGNORE` by `sha` (branch switch / rewrite). Do not delete unreachable commits unless `--force`.
4. On `GitError` from `head_sha` or `log`: leave existing commit rows as they are (unless `--force` already wiped them), set `git_status` to `not_installed` | `timeout` | `unparseable` | `absent`, still finish file index as success.
5. On success: insert new commits + `commit_files` + `commit_fts` only for shas not already present (no duplicate FTS rows); set `git_head=head`, `commit_count`, `git_status=ok`.

Map `GitError::NoRepo` and missing `.git` to `absent`. Missing binary → `not_installed`.

### 7.4 Caps and constants

| Constant | Value |
|---|---|
| `COMMIT_BODY_MAX_CHARS` | 800 |
| `GIT_TIMEOUT_MS` | 120000 |
| `COMMIT_FTS_CAP` | 20 |
| `DECISION_SYMBOL_CAP` | 10 |
| `SUPERSEDES_NEIGHBOR_CAP` | 10 |

No cap on how many commits the index may store. The 120s timeout is the safety valve. Shallow clones index whatever history git has.

## 8. ADR extract

Applies only to markdown (`ParseStatus::Outline`). Other languages unchanged.

### 8.1 Path test

POSIX-normalize, lowercase, then **true** if any of:

1. A directory segment is exactly `adr`, `adrs`, or `decisions`.
2. The filename matches `^adr[-_ ]?[0-9]+` (e.g. `adr-007-websockets.md`, `adr_007.md`).

`README.md`, `docs/guide.md`, and `docs/superpowers/specs/*.md` are **not** ADRs unless they sit under an `adr`/`adrs`/`decisions` directory.

No user glob config in this slice.

### 8.2 Symbols

Keep existing ATX heading symbols.

If `is_adr_path`, also emit one `ExtractedSymbol`:

| Field | Value |
|---|---|
| `kind` | `decision` |
| `name` | first ATX H1 text, else filename stem |
| `start_line` / `end_line` / bytes | the **decision span** below |

**Decision span:**

1. If a heading whose text matches `(?i)^decision\s*$` exists, the span is that heading line through the line before the next heading of the same or higher level (or EOF).
2. Else if an H1 exists, H1 through the line before the next H1 (or EOF), capped at **80 lines**.
3. Else the first **80 lines** of the file.

If the computed span is empty, skip the `decision` symbol; headings still index. `parse_status` stays `outline`.

### 8.3 `supersedes` edges

Scan the file body (not only the span) with:

- `(?i)supersedes\s+ADR-?\s*([0-9]+)`
- markdown links whose target path is an ADR path (same `is_adr_path` rules)

`dst_name` is the target decision name if that file is already extracted in this run, else `ADR-<num>` / the link stem.

Confidence: `high` if the target ADR file exists in this index batch (path found), else `low`.

Indexer resolves `supersedes` like other name edges: `high` when the destination `decision` symbol exists in some file; `low` otherwise. Compiler uses `high` always and `low` only if the neighbor cap is not full (same as Core call/import).

Do not add a `status` column. Status text is quoted only if it sits inside the decision span.

## 9. Compiler

`get_context` stays a pure read. Git is not spawned.

### 9.1 Extra retrieval

Same query plan as Core (quoted identifiers, symbol terms, path hints, FTS remainder). Additional unions:

| Source | Cap |
|---|---|
| `commit_fts` BM25 on the FTS remainder, plus subject token match on `symbol_terms` | 20 |
| `decision` symbols, exact then prefix, `name` COLLATE NOCASE | 10 |
| 1-hop `supersedes` neighbors of accepted decisions | 10 |

These caps are **extra**. They do not shrink Core’s 50 / 30 / 40.

### 9.2 Git `ContextItem`

| Field | Value |
|---|---|
| `path` | `git://<full sha>` |
| `start_line` / `end_line` | 1 |
| `symbol` | first 7 chars of sha |
| `kind` | `"commit"` |
| `text` | `{subject}\n\n{body}` as stored (already ≤800 body chars) |
| `why` | see §9.4 |

No file hash re-read. No `edges` rows for commits. `commit_files` never appear as items.

### 9.3 ADR `ContextItem`

Real repo path. `kind` = `"decision"`. Span re-read from disk if the file hash still matches; else omit and count `stale_omitted` (same as code). `why` includes `decision`.

`supersedes` may appear in `ContextPackage.edges` with `kind` = `"supersedes"` and `from` / `to` = decision names, only when both items are in the package (same rule as Core: edges that justify items).

### 9.4 `why` tags (additions)

Closed extras: `commit`, `commit_fts`, `commit_path`, `decision`, `supersedes_neighbor`.

Existing tags unchanged (`exact_symbol`, `prefix_symbol`, `fts`, `heading`, `selector`, `path_hint`, palace, neighbor tags).

- Commit FTS/subject hit: `commit` + `commit_fts`.
- Commit whose `commit_files` intersect query `path_hints` **or** the set of top-ranked **code** files (non-`git://` paths already in the candidate set before final budget): also `commit_path`.
- Decision name exact/prefix: `decision` **and** `exact_symbol` / `prefix_symbol` (reuse Core weights; do **not** add a second +5).
- Decision file also in file-FTS hits: fuse `fts` onto the decision span when the FTS span overlaps (same fuse key `(path, start_line, end_line)` if they coincide; otherwise keep both spans and let rank/dedupe run).
- Supersedes neighbor: `decision` + `supersedes_neighbor`, and set `neighbor_high` / `neighbor_low` from edge confidence.

### 9.5 Rank (additions to Core table)

Core weights stay. Additional signals:

| Signal | Weight | How |
|---|---|---|
| Commit FTS BM25 normalized 0–1 | 2 | `why` contains `commit_fts`; same 0–1 norm as file FTS |
| Commit path overlap | 1 | `why` contains `commit_path` |
| Supersedes neighbor high | 1.5 | existing `neighbor_high` |
| Supersedes neighbor low | 0.5 | existing `neighbor_low` |

Exact/prefix decision names already get 5 / 3 via `exact_symbol` / `prefix_symbol`. Heading weight (+2) still applies to heading symbols in the same ADR file, not automatically to the `decision` span.

No bonus for the word “why”.

Walk rank order into the same `budget_tokens` and **16KB** JSON cap. A high-rank ADR or commit **can** take a slot a low-rank FTS file would have taken. Palace still only sees leftover budget.

### 9.6 `stats.git`

Always present after this slice (git is not opt-in):

```json
"git": {
  "status": "ok" | "absent",
  "commits_considered": 0,
  "included": 0
}
```

`status` is `ok` when `meta.commit_count > 0`, else `absent`. Query does not surface `not_installed` / `timeout` (those are index-time; `engram status` shows them). `commits_considered` is how many commit candidates retrieval returned (≤20). `included` is how many commit items survived budget.

## 10. CLI, MCP, skill

- No new CLI commands. `engram index` / `status` / `doctor` / `get-context` grow fields only.
- No `--git` flag. History is always considered when rows exist.
- MCP: no new tools. Update `get_context` description to say packages may include `kind=commit` (`git://…` messages) and `kind=decision` (ADR spans); `text` is still untrusted.
- Skill / `AGENTS.md`: “why / decided” still starts at `get_context`. Git/ADR evidence may already be in the package. `include_palace: true` remains for **conversation** memory. Do not tell the agent to shell out to `git log` before calling `get_context`.
- README: one short subsection — history is indexed with the repo; `get-context "why …"` can quote commits and ADRs; still no diffs.

### 10.1 `engram status` / `doctor`

Extra lines:

```text
schema_version: 2
commits: N
git_head: <sha|none>
git: ok|absent|not_installed|timeout|unparseable
```

`IndexStats` gains `commits: u64` and `git: GitIndexStatus` (same five statuses). Print them on `engram index` the way file counts are printed today.

## 11. Errors and safety

| Condition | Behavior |
|---|---|
| No `.engram` | unchanged (`not_initialized`, exit 2) |
| No `.git` | skip history; `git_status=absent`; ADRs still extract |
| `git` / `ENGRAM_GIT_BIN` missing | skip; `not_installed` |
| Git timeout or unparseable stdout | skip this git pass; keep previous commit rows unless `--force` already wiped them; `timeout` / `unparseable` |
| Schema 1 DB | migrate to 2 on write; MCP read on v1 → empty git candidates |
| ADR with no usable span | headings only; index continues |
| Git item `text` | untrusted data, same as source |
| File index IO errors | unchanged Core behavior |

Git failure never returns `Err` from `index_repo` by itself.

## 12. Testing

Inject `GitSource`. Default `cargo test` must not require a real `git` binary.

In-tree / temp fixtures:

- Markdown unit tests: ADR path yes/no; Decision-section span; no Decision heading → H1 + 80-line cap; `supersedes` high when target file exists; `README.md` is not a decision.
- Fake git: two commits with files; second incremental index inserts one; `--force` rebuilds; `NotInstalled` → files still indexed, `git=not_installed`, `commits=0`.
- Compiler: fixture with `src/auth/session.ts` (`createSession`), `docs/adr/007-websockets.md` (Decision: use WebSockets), and a fake commit subject/body mentioning WebSockets. `get_context "why WebSockets"` includes `kind=decision`, `kind=commit` with `path` starting `git://`, and the TS symbol. `get_context "createSession"` includes the TS symbol; a weakly matching commit must not outrank exact symbol. `budget=400` may drop the commit while keeping the symbol.
- Open a schema 1 fixture DB, `open_write` migrates to 2, `get_context` still returns code items.
- MCP `get_context` JSON ≤ 16KB; `tools/list` does not gain new tool names.

Optional live smoke: `ENGRAM_GIT_TEST=1` plus git on PATH may `git init` a temp repo. Default CI skips it.

## 13. Success

- Dogfood-style fixture: one `get_context` call returns ADR span + commit message + implementation span for “why WebSockets”, under budget, **without** spawning git at query time.
- Core tests updated only where package JSON now includes `stats.git`.
- Uninstalling git does not break `engram index` of files.
- Palace opt-in behavior unchanged.

## 14. Risks

- Huge histories: 120s timeout may leave `commits=0` on first index. Mitigation: incremental later; `--force` retry; no daemon.
- ADR path heuristics will miss odd layouts (`architecture/records/`). Mitigation: closed v1 rules; no config surface until it hurts.
- Commit messages can contain secrets. Mitigation: same untrusted-text rule as source; no email stored; no diffs (diffs are worse).
- `stats.git` always present is a package-shape change. Mitigation: additive object; bump tests.

## 15. Implementation note

Do not implement until this spec is approved and an implementation plan is written. Next step is the writing-plans skill, not code.
