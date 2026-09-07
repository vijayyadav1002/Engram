use crate::error::Error;
use std::path::{Path, PathBuf};

/// Resolve the Engram repo root.
///
/// If `env_root` is set and is a directory, that path wins (canonicalized when possible).
/// Otherwise walk upward from `cwd` looking for a `.engram` directory or a `.git` entry.
/// Never treats `$HOME` as a repo merely because the directory exists.
pub fn find_repo_root(cwd: &Path, env_root: Option<&Path>) -> Result<PathBuf, Error> {
    if let Some(p) = env_root {
        if p.is_dir() {
            return Ok(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        }
    }

    let mut dir = if cwd.is_dir() {
        cwd.to_path_buf()
    } else {
        cwd.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf())
    };

    loop {
        if dir.join(".engram").is_dir() || dir.join(".git").exists() {
            return Ok(dir);
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return Err(Error::NotInitialized),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tempfile_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("engram-root-{}-{}", std::process::id(), n));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_markers_is_not_initialized() {
        let tmp = tempfile_dir();
        let err = find_repo_root(&tmp, None).unwrap_err();
        assert!(matches!(err, Error::NotInitialized));
    }

    #[test]
    fn finds_engram_dir() {
        let tmp = tempfile_dir();
        std::fs::create_dir(tmp.join(".engram")).unwrap();
        let nested = tmp.join("src");
        std::fs::create_dir(&nested).unwrap();
        assert_eq!(find_repo_root(&nested, None).unwrap(), tmp);
    }

    #[test]
    fn env_root_wins() {
        let a = tempfile_dir();
        let b = tempfile_dir();
        std::fs::create_dir(b.join(".engram")).unwrap();
        let expected = b.canonicalize().unwrap_or_else(|_| b.clone());
        assert_eq!(find_repo_root(&a, Some(&b)).unwrap(), expected);
    }
}
