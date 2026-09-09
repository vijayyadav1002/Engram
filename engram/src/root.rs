use crate::error::Error;
use std::path::{Path, PathBuf};

/// `ENGRAM_ROOT` if set and non-empty.
pub fn env_root() -> Option<PathBuf> {
    std::env::var_os("ENGRAM_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWalk {
    pub dir: PathBuf,
    /// `None` means index the whole workspace with unprefixed paths.
    pub prefix: Option<String>,
}

/// Canonicalize `path`. It must exist, be a directory, and sit inside `workspace`
/// (after both are canonicalized). If `path` is the workspace root, `prefix` is `None`.
/// Otherwise `prefix` is the last path component (`apps/web` → `Some("web")`).
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
}
