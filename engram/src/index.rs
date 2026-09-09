use crate::error::Error;
use crate::extract::extract_path;
use crate::git::{CliGitSource, GitError, GitIndexStatus, GitRange, GitSource};
use crate::hash::blake3_hex;
use crate::ignore::{should_skip, SkipKind, MAX_FILE_BYTES};
use crate::store::{CommitHit, FileRow, IncomingEdge, Store};
use crate::types::{Confidence, EdgeKind, ExtractedEdge, Extraction, ParseStatus, SymbolKind};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    ".engram",
    ".next",
    "target",
];

const FILE_MODULE: &str = "<file>";

/// Indexer result counters. Totals (`files`/`symbols`/`edges`) are post-run DB counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub files: u64,
    pub symbols: u64,
    pub edges: u64,
    pub skipped_secret: u64,
    pub skipped_large: u64,
    pub skipped_ignore: u64,
    pub errors: u64,
    pub unchanged: u64,
    pub commits: u64,
    pub git: GitIndexStatus,
}

pub struct IndexOpts {
    pub git: Option<Arc<dyn GitSource>>,
    /// Nested directory to walk. `None` walks `root`. If canonical `walk` equals
    /// `root`, behave as `None` (unprefixed full index).
    pub walk: Option<PathBuf>,
}

impl Default for IndexOpts {
    fn default() -> Self {
        Self {
            git: None,
            walk: None,
        }
    }
}

struct WorkItem {
    rel: String,
    hash: String,
    size: i64,
    mtime: i64,
    language: Option<String>,
    extraction: Extraction,
    fts: String,
}

enum Outcome {
    Skipped(SkipKind),
    Unchanged { rel: String },
    Work(WorkItem),
    Failed,
}

/// Walk `root` and incrementally write `.engram/index.sqlite`.
///
/// The database must already exist (`Store::create` / `engram init`).
pub fn index_repo(root: &Path, force: bool) -> Result<IndexStats, Error> {
    index_repo_with(root, force, IndexOpts::default())
}

pub fn index_repo_with(root: &Path, force: bool, opts: IndexOpts) -> Result<IndexStats, Error> {
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

    let db_path = root.join(".engram/index.sqlite");
    let store = Store::open_write(&db_path)?;

    let paths = collect_paths(&walk, &workspace);
    let existing: HashMap<String, String> = store
        .list_files()?
        .into_iter()
        .map(|f| (f.path, f.hash))
        .collect();

    let outcomes: Vec<Outcome> = paths
        .into_par_iter()
        .map(|fs_rel| {
            let stored = match &prefix {
                Some(name) => format!("{name}/{fs_rel}"),
                None => fs_rel.clone(),
            };
            process_file(&walk, &fs_rel, &stored, &existing, force)
        })
        .collect();

    let mut stats = IndexStats::default();
    let mut seen = HashSet::new();
    let mut work_items = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Skipped(kind) => bump_skip(&mut stats, kind),
            Outcome::Unchanged { rel } => {
                stats.unchanged += 1;
                seen.insert(rel);
            }
            Outcome::Work(item) => {
                if item.extraction.status == ParseStatus::Error {
                    stats.errors += 1;
                }
                seen.insert(item.rel.clone());
                work_items.push(item);
            }
            Outcome::Failed => stats.errors += 1,
        }
    }

    let rewritten: HashSet<String> = work_items.iter().map(|i| i.rel.clone()).collect();
    let mut file_syms: HashMap<String, HashMap<String, i64>> = HashMap::new();
    let mut pending_incoming: Vec<(String, Vec<IncomingEdge>)> = Vec::new();
    for item in &work_items {
        let incoming = store.incoming_edges(&item.rel)?;
        let file_id = store.upsert_file(&FileRow {
            id: 0,
            path: item.rel.clone(),
            language: item.language.clone(),
            hash: item.hash.clone(),
            size: item.size,
            mtime: item.mtime,
            parse_status: item.extraction.status,
        })?;
        let pairs = store.replace_file_payload(
            file_id,
            &item.extraction.symbols,
            Some(&item.fts),
            &item.rel,
        )?;
        let mut map = HashMap::new();
        for (id, name) in pairs {
            map.entry(name).or_insert(id);
        }
        file_syms.insert(item.rel.clone(), map);
        pending_incoming.push((item.rel.clone(), incoming));
    }

    let mut triples = Vec::new();
    let import_index = ImportIndex::load(&store)?;
    for item in &work_items {
        let Some(local) = file_syms.get(&item.rel) else {
            continue;
        };
        let import_dsts: HashSet<String> = item
            .extraction
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Import)
            .map(|e| e.dst_name.clone())
            .collect();
        for edge in &item.extraction.edges {
            if let Some(triple) =
                resolve_edge(edge, &item.rel, local, &import_dsts, &store, &import_index)?
            {
                triples.push(triple);
            }
        }
    }
    for (dst_path, incoming) in pending_incoming {
        let Some(local) = file_syms.get(&dst_path) else {
            continue;
        };
        for inc in incoming {
            if rewritten.contains(&inc.src_path) {
                continue;
            }
            let Some(&dst_id) = local.get(&inc.dst_name) else {
                continue;
            };
            triples.push((inc.src_id, dst_id, inc.kind, inc.confidence));
        }
    }
    if !triples.is_empty() {
        store.insert_edges(&triples)?;
    }

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

    let (file_count, symbol_count, edge_count) = store.counts()?;
    store.set_meta(file_count, symbol_count, edge_count)?;
    stats.files = file_count as u64;
    stats.symbols = symbol_count as u64;
    stats.edges = edge_count as u64;
    index_git(
        &store,
        &workspace,
        &walk,
        prefix.as_deref(),
        force,
        opts,
        &mut stats,
    )?;
    Ok(stats)
}

fn map_git_err(e: GitError) -> GitIndexStatus {
    match e {
        GitError::NotInstalled => GitIndexStatus::NotInstalled,
        GitError::NoRepo => GitIndexStatus::Absent,
        GitError::Timeout => GitIndexStatus::Timeout,
        GitError::Unparseable => GitIndexStatus::Unparseable,
        GitError::Io(_) => GitIndexStatus::Unparseable,
    }
}

fn git_fail(store: &Store, stats: &mut IndexStats, status: GitIndexStatus) -> Result<(), Error> {
    let meta = store.meta()?;
    store.set_git_meta(meta.git_head.as_deref(), meta.commit_count, status.as_str())?;
    stats.git = status;
    stats.commits = meta.commit_count as u64;
    Ok(())
}

fn index_git(
    store: &Store,
    workspace: &Path,
    walk: &Path,
    prefix: Option<&str>,
    force: bool,
    opts: IndexOpts,
    stats: &mut IndexStats,
) -> Result<(), Error> {
    if let Some(name) = prefix {
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
        // insert_commit does not bump meta.commit_count; nested must not set_git_meta.
        let mut count = store.meta()?.commit_count;
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
            if store.insert_commit(&hit, &files)? {
                count += 1;
            }
        }
        stats.git = GitIndexStatus::Ok;
        stats.commits = count as u64;
        return Ok(());
    }

    if force {
        store.clear_commits()?;
        store.set_git_meta(None, 0, "absent")?;
    }

    if !workspace.join(".git").exists() {
        let meta = store.meta()?;
        stats.git = GitIndexStatus::Absent;
        stats.commits = meta.commit_count as u64;
        return Ok(());
    }

    let git = opts
        .git
        .unwrap_or_else(|| Arc::new(CliGitSource::default()));
    let head = match git.head_sha(workspace) {
        Ok(h) => h,
        Err(e) => return git_fail(store, stats, map_git_err(e)),
    };

    let meta = store.meta()?;
    let range = match &meta.git_head {
        Some(old) if git.is_ancestor(workspace, old, &head) == Ok(true) => {
            GitRange::After(old.clone())
        }
        _ => GitRange::Head,
    };

    let commits = match git.log(workspace, range) {
        Ok(c) => c,
        Err(e) => return git_fail(store, stats, map_git_err(e)),
    };

    let mut count = meta.commit_count;
    for c in commits {
        let hit = CommitHit {
            id: 0,
            sha: c.sha,
            author: c.author,
            authored_at: c.authored_at,
            subject: c.subject,
            body: c.body,
        };
        if store.insert_commit(&hit, &c.files)? {
            count += 1;
        }
    }
    store.set_git_meta(Some(&head), count, "ok")?;
    stats.git = GitIndexStatus::Ok;
    stats.commits = count as u64;
    Ok(())
}

fn bump_skip(stats: &mut IndexStats, kind: SkipKind) {
    match kind {
        SkipKind::Keep => {}
        SkipKind::SecretName | SkipKind::SecretContent => stats.skipped_secret += 1,
        SkipKind::Large => stats.skipped_large += 1,
        SkipKind::Ignore | SkipKind::Binary => stats.skipped_ignore += 1,
    }
}

fn collect_paths(walk: &Path, workspace: &Path) -> Vec<String> {
    let mut builder = ignore::WalkBuilder::new(walk);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .parents(false);
    if walk != workspace && workspace.join(".engramignore").is_file() {
        let _ = builder.add_ignore(workspace.join(".engramignore"));
    }
    if walk.join(".engramignore").is_file() {
        builder.add_custom_ignore_filename(".engramignore");
    }
    builder.filter_entry(|entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy();
        !SKIP_DIRS.contains(&name.as_ref())
    });

    let mut paths = Vec::new();
    for entry in builder.build() {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if let Some(rel) = posix_rel(walk, entry.path()) {
            paths.push(rel);
        }
    }
    paths
}

fn posix_rel(root: &Path, full: &Path) -> Option<String> {
    let rel = full.strip_prefix(root).ok()?;
    let s = rel.to_string_lossy().replace('\\', "/");
    let s = s.trim_start_matches("./");
    if s.is_empty() {
        return None;
    }
    Some(s.to_string())
}

fn process_file(
    fs_root: &Path,
    fs_rel: &str,
    stored_rel: &str,
    existing: &HashMap<String, String>,
    force: bool,
) -> Outcome {
    match should_skip(fs_root, fs_rel, None) {
        SkipKind::Keep => {}
        kind => return Outcome::Skipped(kind),
    }

    let full = fs_root.join(fs_rel);
    let meta = match fs::metadata(&full) {
        Ok(m) => m,
        Err(_) => return Outcome::Failed,
    };
    if meta.len() > MAX_FILE_BYTES {
        return Outcome::Skipped(SkipKind::Large);
    }

    let bytes = match fs::read(&full) {
        Ok(b) => b,
        Err(_) => return Outcome::Failed,
    };
    match should_skip(fs_root, fs_rel, Some(&bytes)) {
        SkipKind::Keep => {}
        kind => return Outcome::Skipped(kind),
    }

    let hash = blake3_hex(&bytes);
    if !force && existing.get(stored_rel).map(String::as_str) == Some(hash.as_str()) {
        return Outcome::Unchanged {
            rel: stored_rel.to_string(),
        };
    }

    let source = String::from_utf8_lossy(&bytes).into_owned();
    let stored_owned = stored_rel.to_string();
    let extraction = match catch_unwind(AssertUnwindSafe(|| extract_path(&stored_owned, &source))) {
        Ok(ext) => ext,
        Err(_) => Extraction {
            status: ParseStatus::Error,
            symbols: vec![],
            edges: vec![],
        },
    };

    Outcome::Work(WorkItem {
        rel: stored_owned,
        hash,
        size: bytes.len() as i64,
        mtime: mtime_secs(&meta),
        language: language_of(stored_rel).map(str::to_string),
        extraction,
        fts: source,
    })
}

fn mtime_secs(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn language_of(rel_posix: &str) -> Option<&'static str> {
    let lower = rel_posix.to_ascii_lowercase();
    if lower.ends_with(".py") {
        Some("python")
    } else if lower.ends_with(".tsx") || lower.ends_with(".jsx") {
        Some("tsx")
    } else if lower.ends_with(".ts") {
        Some("typescript")
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs") {
        Some("javascript")
    } else if lower.ends_with(".md") || lower.ends_with(".mdx") {
        Some("markdown")
    } else if lower.ends_with(".css") || lower.ends_with(".scss") {
        Some("css")
    } else {
        None
    }
}

struct ImportIndex {
    by_path: HashMap<String, i64>,
    by_no_ext: HashMap<String, i64>,
    by_stem: HashMap<String, i64>,
    path_by_id: HashMap<i64, String>,
}

impl ImportIndex {
    fn load(store: &Store) -> Result<Self, Error> {
        let modules = store.lookup_symbols_exact(FILE_MODULE, 100_000)?;
        let mut by_path = HashMap::new();
        let mut by_no_ext = HashMap::new();
        let mut by_stem = HashMap::new();
        let mut path_by_id = HashMap::new();
        for m in modules {
            if m.kind != SymbolKind::Module {
                continue;
            }
            if let Some(stem) = file_stem(&m.path) {
                by_stem.insert(stem.to_string(), m.id);
            }
            by_no_ext.insert(strip_ext(&m.path), m.id);
            path_by_id.insert(m.id, m.path.clone());
            by_path.insert(m.path, m.id);
        }
        Ok(Self {
            by_path,
            by_no_ext,
            by_stem,
            path_by_id,
        })
    }

    fn module_for_spec(&self, from_file: &str, spec: &str) -> Option<i64> {
        if let Some(resolved) = resolve_relative(from_file, spec) {
            if let Some(id) = self.lookup_pathish(&resolved) {
                return Some(id);
            }
        }
        self.lookup_pathish(spec)
            .or_else(|| self.by_stem.get(file_stem(spec).unwrap_or(spec)).copied())
    }

    fn lookup_pathish(&self, pathish: &str) -> Option<i64> {
        self.by_path
            .get(pathish)
            .or_else(|| self.by_no_ext.get(pathish))
            .copied()
    }

    fn path_for(&self, module_id: i64) -> Option<&str> {
        self.path_by_id.get(&module_id).map(String::as_str)
    }
}

fn resolve_edge(
    edge: &ExtractedEdge,
    file_path: &str,
    local: &HashMap<String, i64>,
    import_dsts: &HashSet<String>,
    store: &Store,
    imports: &ImportIndex,
) -> Result<Option<(i64, i64, EdgeKind, Confidence)>, Error> {
    let src = match local
        .get(&edge.src_name)
        .or_else(|| local.get(FILE_MODULE))
        .copied()
    {
        Some(id) => id,
        None => return Ok(None),
    };

    match edge.kind {
        EdgeKind::Call => {
            if let Some(dst) = local.get(&edge.dst_name).copied() {
                return Ok(Some((src, dst, EdgeKind::Call, Confidence::High)));
            }
            if let Some(dst) =
                resolve_call_via_imports(file_path, &edge.dst_name, import_dsts, store, imports)?
            {
                return Ok(Some((src, dst, EdgeKind::Call, Confidence::High)));
            }
            let hits = store.lookup_symbols_exact(&edge.dst_name, 8)?;
            let Some(hit) = hits.into_iter().next() else {
                return Ok(None);
            };
            Ok(Some((src, hit.id, EdgeKind::Call, Confidence::Low)))
        }
        EdgeKind::Import => {
            if let Some(dst) = imports.module_for_spec(file_path, &edge.dst_name) {
                return Ok(Some((src, dst, EdgeKind::Import, Confidence::High)));
            }
            if let Some(dst) = local.get(&edge.dst_name).copied() {
                return Ok(Some((src, dst, EdgeKind::Import, Confidence::High)));
            }
            let hits = store.lookup_symbols_exact(&edge.dst_name, 8)?;
            if let Some(hit) = hits.into_iter().next() {
                return Ok(Some((src, hit.id, EdgeKind::Import, Confidence::High)));
            }
            Ok(None)
        }
        EdgeKind::Supersedes => {
            let mut hits =
                store.lookup_symbols_kind_exact(&edge.dst_name, SymbolKind::Decision, 8)?;
            if hits.is_empty()
                && !edge.dst_name.is_empty()
                && edge.dst_name.chars().all(|c| c.is_ascii_digit())
            {
                hits = store.lookup_symbols_kind_exact(
                    &format!("ADR-{}", edge.dst_name),
                    SymbolKind::Decision,
                    8,
                )?;
            }
            let Some(hit) = hits.into_iter().next() else {
                return Ok(None);
            };
            Ok(Some((src, hit.id, EdgeKind::Supersedes, Confidence::High)))
        }
    }
}

fn resolve_call_via_imports(
    file_path: &str,
    name: &str,
    import_dsts: &HashSet<String>,
    store: &Store,
    imports: &ImportIndex,
) -> Result<Option<i64>, Error> {
    let mut specs: Vec<&str> = import_dsts.iter().map(String::as_str).collect();
    specs.sort_by_key(|s| (!is_module_spec(s), *s));
    for spec in specs {
        if spec == name {
            continue;
        }
        let Some(module_id) = imports.module_for_spec(file_path, spec) else {
            continue;
        };
        let Some(mod_path) = imports.path_for(module_id) else {
            continue;
        };
        if let Some(id) = symbol_id_in_path(store, name, mod_path)? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn is_module_spec(spec: &str) -> bool {
    spec.starts_with('.') || spec.starts_with('/') || spec.contains('/')
}

fn symbol_id_in_path(store: &Store, name: &str, path: &str) -> Result<Option<i64>, Error> {
    Ok(store
        .lookup_symbols_exact(name, 10_000)?
        .into_iter()
        .find(|h| h.path == path)
        .map(|h| h.id))
}

fn resolve_relative(from_file: &str, spec: &str) -> Option<String> {
    if !spec.starts_with('.') {
        return None;
    }
    let dir = match from_file.rsplit_once('/') {
        Some((d, _)) => d,
        None => "",
    };
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in spec.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn file_stem(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    match name.rfind('.') {
        Some(i) if i > 0 => Some(&name[..i]),
        _ => Some(name),
    }
}

fn strip_ext(path: &str) -> String {
    match file_stem(path) {
        Some(stem) => match path.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{stem}"),
            None => stem.to_string(),
        },
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{FakeGitSource, GitCommit, GitError, GitIndexStatus};
    use crate::store::Store;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("engram-index-{}-{}", std::process::id(), n))
    }

    fn indexed_repo() -> (std::path::PathBuf, FakeGitSource) {
        let dir = tmp();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join(".engram")).unwrap();
        std::fs::write(
            dir.join("src/a.ts"),
            "export function ping() { return 1 }\n",
        )
        .unwrap();
        Store::create(&dir.join(".engram/index.sqlite"), dir.to_str().unwrap()).unwrap();
        let fake = FakeGitSource {
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
        };
        (dir, fake)
    }

    #[test]
    fn language_strings_match_brief() {
        assert_eq!(language_of("src/app.py"), Some("python"));
        assert_eq!(language_of("src/auth/session.ts"), Some("typescript"));
        assert_eq!(language_of("src/auth/LoginBanner.tsx"), Some("tsx"));
        assert_eq!(language_of("pkg/App.jsx"), Some("tsx"));
        assert_eq!(language_of("lib/util.js"), Some("javascript"));
        assert_eq!(language_of("README.md"), Some("markdown"));
        assert_eq!(language_of("styles/auth.css"), Some("css"));
        assert_eq!(language_of("notes.txt"), None);
    }

    #[test]
    fn missing_db_is_not_initialized() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        let err = index_repo(&dir, false).unwrap_err();
        assert!(matches!(err, Error::NotInitialized));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_inserts_fake_commit() {
        let (dir, fake) = indexed_repo();
        let stats = index_repo_with(
            &dir,
            false,
            IndexOpts {
                git: Some(Arc::new(fake)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.git, GitIndexStatus::Ok);
        assert_eq!(stats.commits, 1);
        assert!(stats.files >= 1);
        let store = Store::open_read(&dir.join(".engram/index.sqlite")).unwrap();
        assert_eq!(store.search_commits_fts("websockets", 5).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incremental_second_commit_only_inserts_new() {
        let (dir, mut fake) = indexed_repo();
        index_repo_with(
            &dir,
            false,
            IndexOpts {
                git: Some(Arc::new(fake.clone())),
                ..Default::default()
            },
        )
        .unwrap();
        fake.ancestor = true;
        fake.head = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        fake.commits = vec![GitCommit {
            sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            author: "Ada".into(),
            authored_at: "2026-01-02T00:00:00Z".into(),
            subject: "tweak".into(),
            body: String::new(),
            files: vec!["src/a.ts".into()],
        }];
        let stats = index_repo_with(
            &dir,
            false,
            IndexOpts {
                git: Some(Arc::new(fake)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.commits, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_git_still_indexes_files() {
        let (dir, mut fake) = indexed_repo();
        fake.head_err = Some(GitError::NotInstalled);
        let stats = index_repo_with(
            &dir,
            false,
            IndexOpts {
                git: Some(Arc::new(fake)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.git, GitIndexStatus::NotInstalled);
        assert!(stats.files >= 1);
        assert_eq!(stats.commits, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_dot_git_is_absent() {
        let dir = tmp();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".engram")).unwrap();
        std::fs::write(
            dir.join("src/a.ts"),
            "export function ping() { return 1 }\n",
        )
        .unwrap();
        Store::create(&dir.join(".engram/index.sqlite"), dir.to_str().unwrap()).unwrap();
        let stats = index_repo(&dir, false).unwrap();
        assert_eq!(stats.git, GitIndexStatus::Absent);
        assert!(stats.files >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
