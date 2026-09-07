use crate::error::Error;
use crate::extract::extract_path;
use crate::hash::blake3_hex;
use crate::ignore::{should_skip, SkipKind, MAX_FILE_BYTES};
use crate::store::{FileRow, Store};
use crate::types::{Confidence, EdgeKind, ExtractedEdge, Extraction, ParseStatus, SymbolKind};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
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
    let db_path = root.join(".engram/index.sqlite");
    let store = Store::open_write(&db_path)?;

    let paths = collect_paths(root);
    let existing: HashMap<String, String> = store
        .list_files()?
        .into_iter()
        .map(|f| (f.path, f.hash))
        .collect();

    let outcomes: Vec<Outcome> = paths
        .into_par_iter()
        .map(|rel| process_file(root, &rel, &existing, force))
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

    let mut file_syms: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for item in &work_items {
        store.delete_file_by_path(&item.rel)?;
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
    }

    let mut triples = Vec::new();
    let import_index = ImportIndex::load(&store)?;
    for item in &work_items {
        let Some(local) = file_syms.get(&item.rel) else {
            continue;
        };
        for edge in &item.extraction.edges {
            if let Some(triple) = resolve_edge(edge, &item.rel, local, &store, &import_index)? {
                triples.push(triple);
            }
        }
    }
    if !triples.is_empty() {
        store.insert_edges(&triples)?;
    }

    for path in existing.keys() {
        if !seen.contains(path) {
            store.delete_file_by_path(path)?;
        }
    }

    let (file_count, symbol_count, edge_count) = store.counts()?;
    store.set_meta(file_count, symbol_count, edge_count)?;
    stats.files = file_count as u64;
    stats.symbols = symbol_count as u64;
    stats.edges = edge_count as u64;
    Ok(stats)
}

fn bump_skip(stats: &mut IndexStats, kind: SkipKind) {
    match kind {
        SkipKind::Keep => {}
        SkipKind::SecretName | SkipKind::SecretContent => stats.skipped_secret += 1,
        SkipKind::Large => stats.skipped_large += 1,
        SkipKind::Ignore | SkipKind::Binary => stats.skipped_ignore += 1,
    }
}

fn collect_paths(root: &Path) -> Vec<String> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .parents(false);
    if root.join(".engramignore").is_file() {
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
        if let Some(rel) = posix_rel(root, entry.path()) {
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
    root: &Path,
    rel: &str,
    existing: &HashMap<String, String>,
    force: bool,
) -> Outcome {
    match should_skip(root, rel, None) {
        SkipKind::Keep => {}
        kind => return Outcome::Skipped(kind),
    }

    let full = root.join(rel);
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
    match should_skip(root, rel, Some(&bytes)) {
        SkipKind::Keep => {}
        kind => return Outcome::Skipped(kind),
    }

    let hash = blake3_hex(&bytes);
    if !force && existing.get(rel).map(String::as_str) == Some(hash.as_str()) {
        return Outcome::Unchanged {
            rel: rel.to_string(),
        };
    }

    let source = String::from_utf8_lossy(&bytes).into_owned();
    let rel_owned = rel.to_string();
    let extraction = match catch_unwind(AssertUnwindSafe(|| extract_path(&rel_owned, &source))) {
        Ok(ext) => ext,
        Err(_) => Extraction {
            status: ParseStatus::Error,
            symbols: vec![],
            edges: vec![],
        },
    };

    Outcome::Work(WorkItem {
        rel: rel_owned,
        hash,
        size: bytes.len() as i64,
        mtime: mtime_secs(&meta),
        language: language_of(rel).map(str::to_string),
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
}

impl ImportIndex {
    fn load(store: &Store) -> Result<Self, Error> {
        let modules = store.lookup_symbols_exact(FILE_MODULE, 100_000)?;
        let mut by_path = HashMap::new();
        let mut by_no_ext = HashMap::new();
        let mut by_stem = HashMap::new();
        for m in modules {
            if m.kind != SymbolKind::Module {
                continue;
            }
            if let Some(stem) = file_stem(&m.path) {
                by_stem.insert(stem.to_string(), m.id);
            }
            by_no_ext.insert(strip_ext(&m.path), m.id);
            by_path.insert(m.path, m.id);
        }
        Ok(Self {
            by_path,
            by_no_ext,
            by_stem,
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
}

fn resolve_edge(
    edge: &ExtractedEdge,
    file_path: &str,
    local: &HashMap<String, i64>,
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
            Ok(None)
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
    }
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
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("engram-index-{}-{}", std::process::id(), n))
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
}
