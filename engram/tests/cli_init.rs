use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tempfile_dir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "engram-cli-{}-{}-{}",
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
fn bin_init_exit_zero() {
    let root = tempfile_dir();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["init"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(root.join(".engram/index.sqlite").exists());
}

#[test]
fn bin_status_without_init_exits_not_initialized() {
    let root = tempfile_dir();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["status"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn get_context_help_mentions_palace() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["get-context", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--palace"),
        "get-context --help must list --palace; got:\n{stdout}"
    );
}

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
