use crate::secret::is_secret_content;
use ignore::gitignore::GitignoreBuilder;
use std::path::Path;

/// Files larger than this many bytes are skipped (`SkipKind::Large`).
pub const MAX_FILE_BYTES: u64 = 1_048_576;

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

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
];

const NOISE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "zip", "tar", "gz", "wasm", "woff", "woff2",
];

/// Why a path was skipped during indexing (aggregate counters).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkipReason {
    pub skipped_secret: bool,
    pub skipped_large: bool,
    pub skipped_ignore: bool,
}

/// Decision for a single relative path (and optional file bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipKind {
    Keep,
    Ignore,
    SecretName,
    SecretContent,
    Large,
    Binary,
}

/// Builtin + ignore-file skip rules. Order matches the task brief.
pub fn should_skip(root: &Path, rel_posix: &str, bytes: Option<&[u8]>) -> SkipKind {
    if path_has_skip_dir(rel_posix) {
        return SkipKind::Ignore;
    }

    let file_name = file_name_of(rel_posix);
    if is_secret_name(file_name) {
        return SkipKind::SecretName;
    }
    if is_noise_name(file_name) {
        return SkipKind::Ignore;
    }

    if let Some(b) = bytes {
        if b.contains(&0u8) {
            return SkipKind::Binary;
        }
        if b.len() as u64 > MAX_FILE_BYTES {
            return SkipKind::Large;
        }
        if is_secret_content(b) {
            return SkipKind::SecretContent;
        }
    }

    if matched_by_ignore_files(root, rel_posix) {
        return SkipKind::Ignore;
    }

    SkipKind::Keep
}

fn path_has_skip_dir(rel_posix: &str) -> bool {
    rel_posix.split('/').any(|c| !c.is_empty() && SKIP_DIRS.contains(&c))
}

fn file_name_of(rel_posix: &str) -> &str {
    rel_posix.rsplit('/').next().unwrap_or(rel_posix)
}

fn is_secret_name(name: &str) -> bool {
    if name == ".env" || name.starts_with(".env.") {
        return true;
    }
    if name == "id_rsa" || name == "credentials.json" {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".pem") || lower.ends_with(".key")
}

fn is_noise_name(name: &str) -> bool {
    if LOCKFILES.contains(&name) {
        return true;
    }
    if name.ends_with(".min.js") || name.ends_with(".map") {
        return true;
    }
    if let Some(ext) = name.rsplit('.').next() {
        if ext != name && NOISE_EXTS.contains(&ext) {
            return true;
        }
    }
    false
}

fn matched_by_ignore_files(root: &Path, rel_posix: &str) -> bool {
    let gitignore = root.join(".gitignore");
    let engramignore = root.join(".engramignore");
    let has_gitignore = gitignore.is_file();
    let has_engramignore = engramignore.is_file();
    if !has_gitignore && !has_engramignore {
        return false;
    }

    let mut builder = GitignoreBuilder::new(root);
    // Existing ignore files must apply. On read/parse failure, fail closed → Ignore
    // so we never silently treat a broken ignore file as "keep everything".
    if has_gitignore {
        if builder.add(&gitignore).is_some() {
            return true;
        }
    }
    if has_engramignore {
        if builder.add(&engramignore).is_some() {
            return true;
        }
    }
    let Ok(gi) = builder.build() else {
        return true;
    };

    gi.matched_path_or_any_parents(rel_posix, false).is_ignore()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::is_secret_content;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tempfile_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("engram-ignore-{}-{}", std::process::id(), n));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn skips_node_modules_and_env() {
        assert!(matches!(
            should_skip(Path::new("/p"), "node_modules/x.js", None),
            SkipKind::Ignore
        ));
        assert!(matches!(
            should_skip(Path::new("/p"), ".env", None),
            SkipKind::SecretName
        ));
        assert!(matches!(
            should_skip(Path::new("/p"), ".env.local", None),
            SkipKind::SecretName
        ));
    }

    #[test]
    fn skips_nul_and_large() {
        assert!(matches!(
            should_skip(Path::new("/p"), "a.bin", Some(&[0, 1, 2])),
            SkipKind::Binary
        ));
        let big = vec![b'a'; 1_048_577];
        assert!(matches!(
            should_skip(Path::new("/p"), "big.py", Some(&big)),
            SkipKind::Large
        ));
    }

    #[test]
    fn skips_pem_content_without_keeping_match() {
        let pem = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n";
        assert!(is_secret_content(pem));
        assert!(matches!(
            should_skip(Path::new("/p"), "oops.txt", Some(pem)),
            SkipKind::SecretContent
        ));
    }

    #[test]
    fn honors_gitignore_and_engramignore() {
        let root = tempfile_dir();
        std::fs::write(root.join(".gitignore"), "ignored.txt\ntmp/\n").unwrap();
        std::fs::write(root.join(".engramignore"), "scratch.rs\n").unwrap();

        assert!(matches!(
            should_skip(&root, "ignored.txt", None),
            SkipKind::Ignore
        ));
        assert!(matches!(
            should_skip(&root, "tmp/foo.rs", None),
            SkipKind::Ignore
        ));
        assert!(matches!(
            should_skip(&root, "scratch.rs", None),
            SkipKind::Ignore
        ));
        assert!(matches!(
            should_skip(&root, "keep.rs", None),
            SkipKind::Keep
        ));
    }
}
