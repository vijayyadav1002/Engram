use crate::error::Error;
use crate::store::Store;
use std::path::{Path, PathBuf};

const DEFAULT_ENGRAMIGNORE: &str = "\
node_modules
.venv
dist
build
target
.next
*.min.js
";

/// Create `.engram/`, empty DB, default `.engramignore`, and a gitignore entry.
/// Does not index. Uses `cwd` as root even without repo markers.
pub fn run_init(cwd: &Path) -> Result<PathBuf, Error> {
    let engram_dir = cwd.join(".engram");
    std::fs::create_dir_all(&engram_dir)?;

    let db = engram_dir.join("index.sqlite");
    if !db.exists() {
        let root = cwd.to_string_lossy();
        Store::create(&db, &root)?;
    }

    let ignore = cwd.join(".engramignore");
    if !ignore.exists() {
        std::fs::write(&ignore, DEFAULT_ENGRAMIGNORE)?;
    }

    let gitignore = cwd.join(".gitignore");
    if gitignore.is_file() {
        append_engram_gitignore(&gitignore)?;
    }

    Ok(cwd.to_path_buf())
}

fn append_engram_gitignore(path: &Path) -> Result<(), Error> {
    let mut contents = std::fs::read_to_string(path)?;
    let has_entry = contents.lines().any(|line| {
        let t = line.trim();
        t == ".engram/" || t == ".engram"
    });
    if has_entry {
        return Ok(());
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(".engram/\n");
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tempfile_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("engram-init-{}-{}", std::process::id(), n));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn init_creates_db_and_gitignore_entry() {
        let root = tempfile_dir();
        std::fs::write(root.join(".gitignore"), "node_modules\n").unwrap();
        crate::init::run_init(&root).unwrap();
        assert!(root.join(".engram/index.sqlite").is_file());
        assert!(root.join(".engramignore").is_file());
        let gi = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(gi.contains(".engram/"));
    }
}
