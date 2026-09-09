# Engram Index `--path` Design

Date: 2026-09-08
Status: draft, pending review
Scope: optional `--path` on `engram index` so a workspace can index nested projects into one root `.engram` without polluting those repos
Depends on: `docs/superpowers/specs/2026-09-07-engram-core-design.md`
Related: `docs/superpowers/specs/2026-09-08-engram-git-decisions-design.md` (git index stays; this slice adds a namespaced walk, not a second store)

## 1. Executive summary

`engram index` today always walks the Engram root and writes `{root}/.engram/index.sqlite`. That is correct for a single repo. It is wrong for a **workspace** that contains several git projects: you either index the whole tree under mixed paths, or you `init` inside each project and pollute those repos with `.engram/`.

This slice adds `engram index --path <dir>`. The workspace keeps a single `.engram` (and a single sqlite file) at its root. The nested project is read-only. Stored `files.path` values are prefixed with that project’s **repository name** (last path component) so projects do not overwrite each other. `get_context`, search, MCP, and `status` do not gain new flags: harnesses run from the workspace and already search the whole DB.

Bare `engram index` (no `--path`) is unchanged.

## 2. Problem

Grok / Copilot / other CLIs are started at the workspace root. Engram must live there so agents see context. Nested projects have their own `.git` and must stay clean (no `.engram/`, no harness files written in). Operators need to choose which projects to index, from outside those repos.

## 3. Goals

- `{workspace}/.engram/index.sqlite` is the only store. `--path` never creates `.engram` (or any other Engram file) under the target.
- `engram index --path apps/web` walks only that tree and records paths as `web/…`.
- Re-indexing `web` does not delete `api/…` rows.
- The target tree is read-only (hash/parse only).
- No `--path` keeps today’s walk, path scheme, deletes, `--force`, and git behavior.
- Workspace `get_context` / MCP see every indexed project with no new arguments.

## 4. Non-goals

- Schema v2 / a `namespace` column / per-project sqlite files
- MCP `index` tool
- Auto-`init` when `--path` is used
- `get_context --path` / `--namespace` filters (workspace-wide retrieval is the point)
- Multiple `--path` flags in one invocation (run the command twice)
- Git submodules as first-class objects
- Changing `find_repo_root` so a nested `.git` steals the Engram root
- Writing `.engramignore`, harness snippets, or `AGENTS.md` into the project

## 5. Locked decisions

1. **One DB at the workspace root.** `--path` is a walk + path-prefix, not a new root.
2. **Namespace = last component** of canonical `--path` (`apps/web` → `web`). Not the git remote name.
3. **Stored path** = `{name}/{posix rel from --path}`.
4. If canonical `--path` **equals** the workspace root, treat it as bare `engram index` (no prefix). That avoids `myworkspace/src/foo.ts` for a full-tree walk.
5. Same basename ⇒ same prefix. Two different folders named `web` in one workspace is user error; no extra collision table in this slice.
6. Point `--path` at the **project directory**, not a nested `src/`.
7. Retrieval APIs unchanged.

## 6. CLI

```text
engram index [--force] [--path <dir>]
```

`--path` is optional, cwd-relative or absolute. After canonicalize it must be an existing directory.

Workspace root is `find_repo_root(cwd, ENGRAM_ROOT)` as today. Canonical `--path` must be inside that root (symlink escape → usage error, exit 1). Missing / not a directory → usage error. No `.engram/index.sqlite` at the workspace → `NotInitialized` (exit 2), same as today.

`engram init` is still workspace-only. `--path` does not init.

## 7. Layout

```text
workspace/                 ← .engram/index.sqlite ; CLI/MCP cwd
  apps/web/                ← own .git ; no .engram
    src/foo.ts
  apps/api/                ← own .git ; no .engram
```

```bash
cd workspace
engram init
engram index --path apps/web
engram index --path apps/api
```

DB paths: `web/src/foo.ts`, `api/…`. Agents at `workspace/` call `get_context` as they do now.

## 8. Indexer

Open `{workspace}/.engram/index.sqlite` only.

**Walk.** `WalkBuilder` starts at `--path`, not the workspace root. Gitignore from that project applies. If `{workspace}/.engramignore` exists, add it as an extra ignore file so workspace skip rules apply without copying them into the repo. Built-in skip dirs/secrets/size/NUL unchanged.

**Process.** Same blake3 incremental hash, rayon, extractors, FTS. Read bytes from `{path}/{rel}`; upsert `files.path` = `{name}/{rel}`.

**Deletes.**

| Invocation | Delete unseen rows |
|---|---|
| no `--path`, or `--path` is workspace root | All unseen files (today) |
| `--path` nested | Only unseen rows with prefix `{name}/` |

**`--force`.**

| Invocation | Effect |
|---|---|
| no `--path` | Today: re-parse every file; `clear_commits`; rebuild workspace git |
| `--path` nested | Re-parse files under that walk only; **do not** `clear_commits`; **do not** delete other prefixes |

## 9. Git

| Invocation | Git root | `meta.git_head` / `clear_commits` |
|---|---|---|
| no `--path` | Workspace `{root}/.git` | Today |
| `--path` and `{path}/.git` exists | That directory | Insert commits only. Do **not** `clear_commits`. Do **not** write `meta.git_head` / `git_status` (those stay the workspace story). |
| `--path` and no `{path}/.git` | Skip git this run; `stats.git = absent` | No meta writes |

Commit `files` from `git log` are relative to that project. Prefix them with `{name}/` before `insert_commit` so they match `files.path`. `INSERT OR IGNORE` on sha already drops duplicates when two projects share a commit.

`--path` git uses `GitRange::Head` (full log) + ignore-duplicates, not `meta.git_head`. The single global head cannot represent several nested repos; this slice does not add per-namespace git cursors.

`engram status` still reports workspace `meta` git fields. Nested commits are still searchable via `get_context` because they sit in the same `commits` / `commit_fts` tables.

## 10. Errors

| Case | Error | Exit |
|---|---|---|
| `--path` missing, not a dir, or outside workspace | `Error::Usage` | 1 |
| Workspace has no `.engram/index.sqlite` | `Error::NotInitialized` | 2 |
| DB locked | `Error::IndexBusy` | 3 |

A single file parse error still does not abort the run (`parse_status = error`, counted in `errors`).

## 11. Files to change

| File | Responsibility |
|---|---|
| `engram/src/main.rs` | `--path` on `Index`; pass into indexer |
| `engram/src/index.rs` | Optional walk dir, name prefix, scoped deletes, `--force`/`git` split |
| `engram/tests/index_incremental.rs` (and/or lib tests in `index.rs`) | Namespace / scoped delete / no nested `.engram` |
| `README.md` | Workspace `--path` paragraph next to `engram index` |

No schema migration. No MCP surface change. `root.rs` stay as-is unless a test proves `--path` containment needs a helper there (then a small `path_inside_root` is fine).

## 12. Testing

TDD. Work in `engram/`. `cd engram && cargo test …`.

- No `--path`: existing index tests still pass (unprefixed paths, full deletes, `--force` clears git).
- `--path apps/web` after `init` at workspace: rows are `web/…`; `apps/web/.engram` does not exist; workspace `.engram` does.
- Index `web` then `api`: both prefixes present.
- Delete a file under `web`, reindex `--path apps/web`: that row gone; `api/…` stays.
- `--force --path apps/web`: `web` hashes refresh; `api` rows and existing commits remain.
- `--path` with `{path}/.git`: commit_files paths start with `web/`; `meta.git_head` unchanged from before the call.
- `--path` without `.git`: `stats.git` absent; no commit insert required.
- `--path` outside workspace / missing dir: usage, exit 1.
- `--path` at workspace root: same path scheme as bare `engram index`.

CLI coverage: `engram/tests/cli_init.rs` (or a sibling) for `--path` help/dispatch if argv is not already covered by lib tests.

## 13. Docs

README Step B: after the current `engram index` block, add that a workspace with nested projects can `engram index --path <project>` and that `.engram` stays at the workspace. Do not document MCP `index`.
