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
