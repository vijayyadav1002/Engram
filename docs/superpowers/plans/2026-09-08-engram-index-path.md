# Engram Index `--path` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `engram index --path <dir>` so a workspace can index nested projects into `{workspace}/.engram/index.sqlite` under a repo-name prefix, without writing `.engram` into those projects.

**Architecture:** Keep one sqlite at the workspace root. `--path` only changes the walk root, stored `files.path` prefix (`web/src/foo.ts`), scoped deletes, and nested git insert. Bare `engram index` stays the current walk. `get_context`, search, MCP, and `status` gain no flags.

**Tech Stack:** Existing Engram crate (Rust, clap, `ignore` 0.4, rusqlite, rayon). No new dependencies. No schema migration. No MCP `index`.

**Spec:** `docs/superpowers/specs/2026-09-08-engram-index-path-design.md`

## Global Constraints

- `{workspace}/.engram/index.sqlite` is the only store. `--path` never creates `.engram` under the target.
- Namespace = last component of canonical `--path` (`apps/web` → `web`). Not the git remote name.
- Stored path = `{name}/{posix rel from --path}`.
- If canonical `--path` equals the workspace root, treat as bare `engram index` (no prefix).
- Same basename ⇒ same prefix. No collision table.
- No `--path` keeps today’s walk, path scheme, deletes, `--force`, and git behavior.
- Nested `--force --path` does not `clear_commits` and does not delete other prefixes.
- Nested git: `GitRange::Head` + `INSERT OR IGNORE`; prefix `commit_files`; do not write `meta.git_head` / `git_status`.
- No auto-init. No MCP `index`. No `get_context --path`.
- Work in `engram/`. Run `cd engram && cargo test …`.
- TDD on every task.
- Do not add embeddings, a watcher, schema v2, or per-project sqlite files.

## File map

| File | Responsibility |
|---|---|
| `engram/src/root.rs` | `ResolvedWalk`, `resolve_index_walk` (canonicalize, inside-workspace, prefix or `None` if walk == root) |
| `engram/src/index.rs` | `IndexOpts.walk`, prefixed walk, scoped deletes, nested git split |
| `engram/src/main.rs` | `--path` on `Index`; resolve then `index_repo_with` |
| `engram/tests/index_path.rs` | Workspace fixture: prefix, scoped delete, no nested `.engram`, git meta |
| `engram/tests/cli_init.rs` | Binary `--path` usage errors + happy path |
| `engram/tests/git_decisions.rs` | `IndexOpts { ..Default::default() }` after new field |
| `README.md` | Workspace `--path` paragraph after Step B |
| `docs/superpowers/specs/2026-09-08-engram-index-path-design.md` | Status → implemented when the last task lands |

Do not change `mcp.rs`, `compile.rs`, or the sqlite DDL.

---

### Task 1: Resolve `--path` against the workspace root

**Files:**
- Modify: `engram/src/root.rs`
- Test: `engram/src/root.rs` (`mod tests`)

**Interfaces:**
- Consumes: existing `Error::Usage(String)` (exit 1), `Error::NotInitialized` unused here
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWalk {
    pub dir: PathBuf,
    /// `None` means index the whole workspace with unprefixed paths.
    pub prefix: Option<String>,
}

/// Canonicalize `path`. It must exist, be a directory, and sit inside `workspace`
/// (after both are canonicalized). If `path` is the workspace root, `prefix` is `None`.
/// Otherwise `prefix` is the last path component (`apps/web` → `Some("web")`).
pub fn resolve_index_walk(workspace: &Path, path: &Path) -> Result<ResolvedWalk, Error>
```

- [ ] **Step 1: Write the failing tests** in `engram/src/root.rs` tests module (keep existing tests).

```rust
    #[test]
    fn resolve_nested_prefix_is_basename() {
        let ws = tempfile_dir();
        let web = ws.join("apps/web");
        std::fs::create_dir_all(&web).unwrap();
        let got = resolve_index_walk(&ws, &web).unwrap();
        assert_eq!(got.prefix.as_deref(), Some("web"));
        assert_eq!(got.dir, web.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn resolve_workspace_root_has_no_prefix() {
        let ws = tempfile_dir();
        let got = resolve_index_walk(&ws, &ws).unwrap();
        assert_eq!(got.prefix, None);
        assert_eq!(got.dir, ws.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn resolve_missing_path_is_usage() {
        let ws = tempfile_dir();
        let err = resolve_index_walk(&ws, &ws.join("nope")).unwrap_err();
        assert!(matches!(err, Error::Usage(_)));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn resolve_file_is_usage() {
        let ws = tempfile_dir();
        let file = ws.join("readme");
        std::fs::write(&file, "x").unwrap();
        let err = resolve_index_walk(&ws, &file).unwrap_err();
        assert!(matches!(err, Error::Usage(_)));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn resolve_outside_workspace_is_usage() {
        let ws = tempfile_dir();
        let other = tempfile_dir();
        let err = resolve_index_walk(&ws, &other).unwrap_err();
        assert!(matches!(err, Error::Usage(_)));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&other);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib root::tests::resolve_nested_prefix_is_basename root::tests::resolve_workspace_root_has_no_prefix root::tests::resolve_missing_path_is_usage root::tests::resolve_file_is_usage root::tests::resolve_outside_workspace_is_usage`

Expected: FAIL compile (`resolve_index_walk` not found) or FAIL assertion.

- [ ] **Step 3: Implement `resolve_index_walk`** in `engram/src/root.rs` (next to `find_repo_root`).

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWalk {
    pub dir: PathBuf,
    pub prefix: Option<String>,
}

pub fn resolve_index_walk(workspace: &Path, path: &Path) -> Result<ResolvedWalk, Error> {
    if !path.exists() {
        return Err(Error::Usage(format!("path not found: {}", path.display())));
    }
    if !path.is_dir() {
        return Err(Error::Usage(format!(
            "path is not a directory: {}",
            path.display()
        )));
    }
    let dir = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    if dir != root && !dir.starts_with(&root) {
        return Err(Error::Usage(format!(
            "path is outside workspace: {}",
            path.display()
        )));
    }
    let prefix = if dir == root {
        None
    } else {
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Usage("path has no directory name".into()))?;
        Some(name.to_string())
    };
    Ok(ResolvedWalk { dir, prefix })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --lib root::`

Expected: PASS (old root tests + the five new ones).

- [ ] **Step 5: Commit**

```bash
git add engram/src/root.rs
git commit -m "feat: resolve engram index --path against workspace root"
```

---

### Task 2: Prefixed walk, scoped deletes, no nested `.engram`

**Files:**
- Modify: `engram/src/index.rs` (`IndexOpts`, `index_repo`, `index_repo_with`, `collect_paths`, `process_file`, delete loop)
- Modify: `engram/tests/git_decisions.rs` (`IndexOpts` literals)
- Create: `engram/tests/index_path.rs`

**Interfaces:**
- Consumes: `resolve_index_walk` is **not** required inside the indexer; CLI will resolve. Indexer trusts `opts.walk` when set.
- Produces:

```rust
pub struct IndexOpts {
    pub git: Option<Arc<dyn GitSource>>,
    /// Nested directory to walk. `None` walks `root`. If canonical `walk` equals
    /// `root`, behave as `None` (unprefixed full index).
    pub walk: Option<PathBuf>,
}

impl Default for IndexOpts {
    fn default() -> Self {
        Self { git: None, walk: None }
    }
}

pub fn index_repo(root: &Path, force: bool) -> Result<IndexStats, Error>
// body: index_repo_with(root, force, IndexOpts::default())

pub fn index_repo_with(root: &Path, force: bool, opts: IndexOpts) -> Result<IndexStats, Error>
```

`collect_paths(walk: &Path, workspace: &Path) -> Vec<String>` walks `walk`. Rels are relative to `walk`. If `workspace.join(".engramignore")` is a file and `walk` is not the workspace, `builder.add_ignore` that file. Keep `add_custom_ignore_filename(".engramignore")` when `walk` has one. Keep `parents(false)`, builtin `SKIP_DIRS`, secrets.

`process_file(fs_root, fs_rel, stored_rel, existing, force)`:
- `should_skip(fs_root, fs_rel, …)`
- read `fs_root.join(fs_rel)`
- hash compare against `existing.get(stored_rel)`
- `WorkItem.rel = stored_rel` (`web/src/a.ts`)
- `extract_path(stored_rel, source)` (extension still works)

Deletes: if prefix is `Some("web")`, only delete unseen keys that start with `web/`. If prefix is `None`, delete all unseen (today).

`--force` with a nested walk still re-parses that walk’s files (pass `force` into `process_file` as today). Do **not** change git in this task.

Existing `IndexOpts { git: Some(...) }` literals in `engram/src/index.rs` tests and `engram/tests/git_decisions.rs` must compile: add `..Default::default()`.

- [ ] **Step 1: Write the failing tests** — create `engram/tests/index_path.rs`:

```rust
use engram::index::{index_repo, index_repo_with, IndexOpts};
use engram::store::Store;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "engram-index-path-{}-{}-{}",
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn workspace() -> PathBuf {
    let root = tmp();
    fs::create_dir_all(root.join("apps/web/src")).unwrap();
    fs::create_dir_all(root.join("apps/api/src")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("apps/web/src/a.ts"),
        "export function ping() { return 1 }\n",
    )
    .unwrap();
    fs::write(
        root.join("apps/api/src/b.ts"),
        "export function pong() { return 2 }\n",
    )
    .unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    root
}

fn db(root: &std::path::Path) -> Store {
    Store::open_read(&root.join(".engram/index.sqlite")).unwrap()
}

#[test]
fn path_prefixes_rows_and_does_not_write_nested_engram() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("src/a.ts").unwrap().is_none());
    assert!(store.get_file("apps/web/src/a.ts").unwrap().is_none());
    assert!(root.join(".engram/index.sqlite").is_file());
    assert!(!root.join("apps/web/.engram").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn second_project_accumulates() {
    let root = workspace();
    let opts_web = IndexOpts {
        walk: Some(root.join("apps/web")),
        ..Default::default()
    };
    let opts_api = IndexOpts {
        walk: Some(root.join("apps/api")),
        ..Default::default()
    };
    index_repo_with(&root, false, opts_web).unwrap();
    index_repo_with(&root, false, opts_api).unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("api/src/b.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn path_delete_only_touches_that_prefix() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/api")),
            ..Default::default()
        },
    )
    .unwrap();
    fs::remove_file(root.join("apps/web/src/a.ts")).unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_none());
    assert!(store.get_file("api/src/b.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn force_path_does_not_drop_other_prefix() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/api")),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        true,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("api/src/b.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn walk_equals_root_is_unprefixed() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("apps/web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("web/src/a.ts").unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_engramignore_applies_to_nested_walk() {
    let root = workspace();
    fs::write(root.join(".engramignore"), "ignored.ts\n").unwrap();
    fs::write(root.join("apps/web/src/ignored.ts"), "export const x = 1\n").unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("web/src/ignored.ts").unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn bare_index_still_unprefixed() {
    let root = workspace();
    index_repo(&root, false).unwrap();
    let store = db(&root);
    assert!(store.get_file("apps/web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("web/src/a.ts").unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --test index_path -- --nocapture`

Expected: FAIL compile (`walk` field missing on `IndexOpts`) or FAIL assertions (unprefixed paths / nested deletes).

- [ ] **Step 3: Minimal implementation**

1. Add `walk: Option<PathBuf>` and `Default` on `IndexOpts`.
2. `index_repo` uses `IndexOpts::default()`.
3. Every existing `IndexOpts { git: Some(...) }` becomes:

```rust
IndexOpts {
    git: Some(Arc::new(fake)),
    ..Default::default()
}
```

in `engram/src/index.rs` tests and `engram/tests/git_decisions.rs` (two sites in git_decisions: the helper around line 43 and the one around line 189).

4. At the start of `index_repo_with`:

```rust
    let workspace = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let walk = opts
        .walk
        .as_ref()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
        .unwrap_or_else(|| workspace.clone());
    let prefix: Option<String> = if walk == workspace {
        None
    } else {
        walk.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    };
```

5. `let paths = collect_paths(&walk, &workspace);` then for each `fs_rel`:

```rust
    let stored = match &prefix {
        Some(name) => format!("{name}/{fs_rel}"),
        None => fs_rel.clone(),
    };
    process_file(&walk, &fs_rel, &stored, &existing, force)
```

6. Change `process_file` to take `fs_root`, `fs_rel`, `stored_rel`. Skip/read using `fs_root`/`fs_rel`. Unchanged/hash/WorkItem use `stored_rel`. Call `extract_path(stored_rel, &source)`.

7. Delete loop:

```rust
    for path in existing.keys() {
        if let Some(name) = prefix.as_deref() {
            let pfx = format!("{name}/");
            if !path.starts_with(&pfx) {
                continue;
            }
        }
        if !seen.contains(path) {
            store.delete_file_by_path(path)?;
        }
    }
```

8. `collect_paths(walk, workspace)`: `WalkBuilder::new(walk)`, same flags as today, `posix_rel(walk, …)`. If `walk != workspace` and `workspace.join(".engramignore").is_file()`, `let _ = builder.add_ignore(workspace.join(".engramignore"));`. If `walk.join(".engramignore").is_file()`, keep `add_custom_ignore_filename(".engramignore")`.

Leave `index_git(&store, root, force, opts, &mut stats)` using **workspace** `root` for this task (nested git is Task 3). Nested walks without workspace `.git` already hit `stats.git = absent`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --test index_path --lib index:: --test git_decisions --test index_incremental`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engram/src/index.rs engram/tests/index_path.rs engram/tests/git_decisions.rs
git commit -m "feat: index nested --path under repo-name prefix"
```

---

### Task 3: Nested git insert without touching workspace git meta

**Files:**
- Modify: `engram/src/index.rs` (`index_git`)
- Modify: `engram/tests/index_path.rs`

**Interfaces:**
- Consumes: `IndexOpts.walk`, `FakeGitSource`, `store.insert_commit` (`INSERT OR IGNORE` on sha), `store.meta()`, `store.clear_commits`, `store.set_git_meta`
- Produces: nested walk git behavior:

| `opts.walk` | `.git` at walk | force | git root | `clear_commits` / `set_git_meta` | commit file paths |
|---|---|---|---|---|---|
| `None` or walk == workspace | as today | as today | workspace | as today | unprefixed |
| nested | missing | ignored | skip | **no** | none |
| nested | present | **must not** clear | walk dir | **no** `set_git_meta` | `{name}/{git rel}` |

Nested log range is always `GitRange::Head`. On nested git errors, set `stats.git` from `map_git_err` and `stats.commits` from current `meta.commit_count`; do not call `git_fail` (it writes meta).

`index_git` must receive the same `walk` / `prefix` as the file walk. Change the signature to:

```rust
fn index_git(
    store: &Store,
    workspace: &Path,
    walk: &Path,
    prefix: Option<&str>,
    force: bool,
    opts: IndexOpts,
    stats: &mut IndexStats,
) -> Result<(), Error>
```

Call site in `index_repo_with` after file writes:

```rust
    index_git(
        &store,
        &workspace,
        &walk,
        prefix.as_deref(),
        force,
        opts,
        &mut stats,
    )?;
```

When `prefix` is `None`, keep the current body (`if force { clear_commits; set_git_meta None }`, git root = `workspace`).

When `prefix` is `Some(name)`:

```rust
    if !walk.join(".git").exists() {
        let meta = store.meta()?;
        stats.git = GitIndexStatus::Absent;
        stats.commits = meta.commit_count as u64;
        return Ok(());
    }
    let git = opts
        .git
        .unwrap_or_else(|| Arc::new(CliGitSource::default()));
    let head = match git.head_sha(walk) {
        Ok(h) => h,
        Err(e) => {
            let meta = store.meta()?;
            stats.git = map_git_err(e);
            stats.commits = meta.commit_count as u64;
            return Ok(());
        }
    };
    let _ = head;
    let commits = match git.log(walk, GitRange::Head) {
        Ok(c) => c,
        Err(e) => {
            let meta = store.meta()?;
            stats.git = map_git_err(e);
            stats.commits = meta.commit_count as u64;
            return Ok(());
        }
    };
    for c in commits {
        let files: Vec<String> = c
            .files
            .iter()
            .map(|f| {
                let f = f.trim_start_matches("./");
                format!("{name}/{f}")
            })
            .collect();
        let hit = CommitHit {
            id: 0,
            sha: c.sha,
            author: c.author,
            authored_at: c.authored_at,
            subject: c.subject,
            body: c.body,
        };
        store.insert_commit(&hit, &files)?;
    }
    let meta = store.meta()?;
    stats.git = GitIndexStatus::Ok;
    stats.commits = meta.commit_count as u64;
    Ok(())
```

Do not write `meta.git_head` on this branch. `force` is ignored on this branch (no `clear_commits`).

- [ ] **Step 1: Write the failing tests** at the bottom of `engram/tests/index_path.rs`:

```rust
use engram::git::{FakeGitSource, GitCommit, GitIndexStatus};
use std::sync::Arc;

fn fake_web() -> FakeGitSource {
    FakeGitSource {
        head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ancestor: false,
        commits: vec![GitCommit {
            sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            author: "Ada".into(),
            authored_at: "2026-01-01T00:00:00Z".into(),
            subject: "use websockets".into(),
            body: "replace polling".into(),
            files: vec!["src/a.ts".into()],
        }],
        head_err: None,
        log_err: None,
    }
}

#[test]
fn nested_git_prefixes_commit_files_and_leaves_meta_head() {
    let root = workspace();
    fs::create_dir_all(root.join("apps/web/.git")).unwrap();
    let before = db(&root).meta().unwrap().git_head.clone();
    let stats = index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.git, GitIndexStatus::Ok);
    assert!(stats.commits >= 1);
    let store = db(&root);
    assert_eq!(store.meta().unwrap().git_head, before);
    let hits = store.search_commits_fts("websockets", 5).unwrap();
    assert_eq!(hits.len(), 1);
    let files = store
        .list_files()
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect::<Vec<_>>();
    assert!(files.iter().any(|p| p == "web/src/a.ts"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nested_without_git_is_absent_and_inserts_no_commits() {
    let root = workspace();
    let stats = index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.git, GitIndexStatus::Absent);
    assert_eq!(stats.commits, 0);
    let store = db(&root);
    assert!(store.search_commits_fts("websockets", 5).unwrap().is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn force_path_does_not_clear_commits() {
    let root = workspace();
    fs::create_dir_all(root.join("apps/web/.git")).unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        true,
        IndexOpts {
            walk: Some(root.join("apps/api")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert_eq!(store.search_commits_fts("websockets", 5).unwrap().len(), 1);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}
```

`Store` has no public `commit_files` getter. Do **not** add one. The contract for this task is: FTS finds the nested commit, `files.path` is `web/src/a.ts`, and `meta.git_head` is unchanged. Prefixed `commit_files` rows are produced by the `format!("{name}/{f}")` call in `index_git`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --test index_path nested_git_prefixes_commit_files_and_leaves_meta_head nested_without_git_is_absent_and_inserts_no_commits force_path_does_not_clear_commits`

Expected: FAIL — `force --path` currently `clear_commits` (empty FTS) and/or nested git uses workspace (no `.git` → absent even when `apps/web/.git` exists).

- [ ] **Step 3: Implement the nested branch** of `index_git` as specified above. Keep today’s workspace branch byte-for-byte for `prefix == None` (including `git_fail` meta writes).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --test index_path --lib index:: --test git_decisions`

Expected: PASS. Existing `index_inserts_fake_commit` / `incremental_second_commit_only_inserts_new` still use workspace `.git` and still update `meta.git_head`.

- [ ] **Step 5: Commit**

```bash
git add engram/src/index.rs engram/tests/index_path.rs
git commit -m "feat: index nested repo git without clobbering workspace meta"
```

---

### Task 4: CLI `--path`, usage exits, README

**Files:**
- Modify: `engram/src/main.rs`
- Modify: `engram/tests/cli_init.rs`
- Modify: `README.md` (after Step B `engram index --force` block)
- Modify: `docs/superpowers/specs/2026-09-08-engram-index-path-design.md` (status line)

**Interfaces:**
- Consumes: `resolve_index_walk`, `index_repo_with`, `IndexOpts`, `require_root`, `current_dir`
- Produces: `engram index [--force] [--path <DIR>]`

```rust
    Index {
        #[arg(long)]
        force: bool,
        /// Nested project directory to index into the workspace DB
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
    },
```

Dispatch (replace `Commands::Index { force }`):

```rust
        Commands::Index { force, path } => {
            let root = require_root()?;
            let opts = match path {
                None => IndexOpts::default(),
                Some(p) => {
                    let abs = if p.is_absolute() {
                        p
                    } else {
                        current_dir()?.join(p)
                    };
                    let resolved = engram::root::resolve_index_walk(&root, &abs)?;
                    IndexOpts {
                        walk: resolved.prefix.as_ref().map(|_| resolved.dir),
                        ..Default::default()
                    }
                }
            };
            let stats = index_repo_with(&root, force, opts)?;
            print_index_stats(&stats);
            Ok(())
        }
```

When `resolved.prefix` is `None` (`--path` is the workspace root), `walk` is `None` → bare index. Need `use engram::index::index_repo_with` and `IndexOpts` (can drop `index_repo` import if unused).

README: after the `engram index --force` fence, add:

```markdown
A workspace that contains several git projects should keep `.engram/` at the
**workspace** root (so Grok / Copilot / other CLIs started there see the index).
Index a nested project without `cd` and without writing `.engram` into that repo:

```bash
engram index --path apps/web
```

Files are stored as `web/…` (the last component of `--path`). Re-run for each
project you want in the workspace index. Plain `engram index` still walks the
whole workspace with unprefixed paths.
```

Do not mention MCP `index`.

Spec status line: `Status: implemented on main` after tests pass.

- [ ] **Step 1: Write the failing CLI tests** in `engram/tests/cli_init.rs`:

```rust
#[test]
fn index_help_mentions_path() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["index", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--path"),
        "index --help must list --path; got:\n{stdout}"
    );
}

#[test]
fn index_path_missing_dir_exits_usage() {
    let root = tempfile_dir();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["init"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["index", "--path", "nope"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn index_path_outside_workspace_exits_usage() {
    let root = tempfile_dir();
    let other = tempfile_dir();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["init"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["index", "--path", other.to_str().unwrap()])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn index_path_writes_prefixed_rows_not_nested_engram() {
    let root = tempfile_dir();
    std::fs::create_dir_all(root.join("apps/web/src")).unwrap();
    std::fs::write(
        root.join("apps/web/src/a.ts"),
        "export function ping() { return 1 }\n",
    )
    .unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["init"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["index", "--path", "apps/web"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(root.join(".engram/index.sqlite").is_file());
    assert!(!root.join("apps/web/.engram").exists());
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["search-symbols", "ping"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("web/src/a.ts"),
        "expected prefixed path, got:\n{stdout}"
    );
    assert!(!stdout.contains("apps/web/src/a.ts"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --test cli_init index_help_mentions_path index_path_missing_dir_exits_usage index_path_outside_workspace_exits_usage index_path_writes_prefixed_rows_not_nested_engram`

Expected: FAIL — clap unknown argument `--path` (exit 2 from clap, not 1) / help has no `--path`.

- [ ] **Step 3: Wire `main.rs` and README** as specified. Update spec status to `implemented on main`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd engram && cargo test --test cli_init --test index_path --lib root:: --lib index::`

Expected: PASS.

Then: `cd engram && cargo test`

Expected: PASS (full crate).

- [ ] **Step 5: Commit**

```bash
git add engram/src/main.rs engram/tests/cli_init.rs README.md docs/superpowers/specs/2026-09-08-engram-index-path-design.md
git commit -m "feat: add engram index --path CLI for workspace projects"
```
