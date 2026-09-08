use crate::error::Error;
use crate::hash::blake3_file;
use crate::store::Store;
use std::path::Path;
use tree_sitter::Parser;

const STALE_SAMPLE: usize = 20;

/// Multi-line diagnostic report: binary, grammars, DB, ignore files, harness presence.
pub fn run_doctor(root: &Path) -> Result<String, Error> {
    let mut lines = Vec::new();
    lines.push("binary: ok".to_string());
    lines.push(format!("grammars: {}", grammar_status()));

    let db = root.join(".engram/index.sqlite");
    let store = open_doctor_db(&db)?;
    let meta = store
        .meta()
        .map_err(|_| Error::Db(format!("unreadable {}", db.display())))?;
    lines.push(format!("db: {}", db.display()));
    lines.push(format!("schema_version: {}", meta.schema_version));
    lines.push(format!(
        "indexed_at: {}",
        meta.indexed_at.as_deref().unwrap_or("never")
    ));
    lines.push(format!(
        "counts: files={} symbols={} edges={}",
        meta.file_count, meta.symbol_count, meta.edge_count
    ));
    lines.extend(git_meta_lines(&meta));

    lines.push(present_line(
        ".engramignore",
        root.join(".engramignore").is_file(),
    ));
    lines.push(present_line(
        ".gitignore",
        root.join(".gitignore").is_file(),
    ));
    lines.push(present_line(
        "harness grok",
        root.join(".grok/config.toml").is_file(),
    ));
    lines.push(present_line(
        "harness copilot/claude",
        root.join(".mcp.json").is_file(),
    ));
    lines.push(present_line(
        "harness cursor",
        root.join(".cursor/mcp.json").is_file(),
    ));
    lines.push(present_line(
        "skill",
        root.join(".grok/skills/engram/SKILL.md").is_file(),
    ));

    Ok(lines.join("\n") + "\n")
}

/// DB path, schema, counts, last index, stale sample of on-disk hashes.
pub fn run_status(root: &Path) -> Result<String, Error> {
    let db = root.join(".engram/index.sqlite");
    let store = Store::open_read(&db)?;
    let meta = store.meta()?;
    let files = store.list_files()?;

    let mut checked = 0usize;
    let mut stale = 0usize;
    for f in files.iter().take(STALE_SAMPLE) {
        checked += 1;
        let disk = root.join(&f.path);
        match blake3_file(&disk) {
            Ok(h) if h == f.hash => {}
            _ => stale += 1,
        }
    }

    Ok(format!(
        "db: {}\n\
schema_version: {}\n\
root: {}\n\
files: {}\n\
symbols: {}\n\
edges: {}\n\
commits: {}\n\
git_head: {}\n\
git: {}\n\
indexed_at: {}\n\
stale_sample: {}/{}\n",
        db.display(),
        meta.schema_version,
        meta.root,
        meta.file_count,
        meta.symbol_count,
        meta.edge_count,
        meta.commit_count,
        meta.git_head.as_deref().unwrap_or("none"),
        meta.git_status,
        meta.indexed_at.as_deref().unwrap_or("never"),
        stale,
        checked
    ))
}

fn git_meta_lines(meta: &crate::store::Meta) -> [String; 3] {
    [
        format!("commits: {}", meta.commit_count),
        format!("git_head: {}", meta.git_head.as_deref().unwrap_or("none")),
        format!("git: {}", meta.git_status),
    ]
}

fn open_doctor_db(db: &Path) -> Result<Store, Error> {
    if !db.is_file() {
        return Err(Error::Db(format!("unreadable {}", db.display())));
    }
    match Store::open_read(db) {
        Ok(store) => Ok(store),
        Err(Error::NotInitialized) => Err(Error::Db(format!("unreadable {}", db.display()))),
        Err(e) => Err(e),
    }
}

fn present_line(label: &str, present: bool) -> String {
    if !present && label.starts_with("harness") {
        eprintln!("warn: {label} absent");
    }
    format!("{label}: {}", if present { "present" } else { "absent" })
}

fn grammar_status() -> String {
    let checks = [
        (
            "python",
            tree_sitter::Language::from(tree_sitter_python::LANGUAGE),
        ),
        (
            "ts",
            tree_sitter::Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        ),
        (
            "tsx",
            tree_sitter::Language::from(tree_sitter_typescript::LANGUAGE_TSX),
        ),
        (
            "js",
            tree_sitter::Language::from(tree_sitter_javascript::LANGUAGE),
        ),
    ];
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for (name, lang) in checks {
        let mut parser = Parser::new();
        if parser.set_language(&lang).is_ok() {
            ok.push(name);
        } else {
            bad.push(name);
        }
    }
    if bad.is_empty() {
        format!("{} ok", ok.join(","))
    } else {
        format!("{} ok; failed {}", ok.join(","), bad.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tempfile_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "engram-doctor-{}-{}-{}",
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn status_includes_git_meta_lines() {
        let root = tempfile_dir();
        std::fs::create_dir_all(root.join(".engram")).unwrap();
        let store =
            Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
        store.set_git_meta(Some("abc"), 3, "ok").unwrap();
        drop(store);
        let out = run_status(&root).unwrap();
        assert!(out.contains("commits: 3"), "status: {out}");
        assert!(out.contains("git: ok"), "status: {out}");
    }
}
