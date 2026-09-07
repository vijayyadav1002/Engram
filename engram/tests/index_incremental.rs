use engram::index::index_repo;
use engram::store::Store;
use std::fs;
use std::path::PathBuf;

fn setup() -> PathBuf {
    let root = std::env::temp_dir().join(format!("engram-idx-{}", unique_name()));
    // copy miniapp or write files
    fs::create_dir_all(root.join("src/auth")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 1 }\n",
    )
    .unwrap();
    fs::write(root.join(".env"), "SECRET=nope\n").unwrap();
    fs::write(root.join("README.md"), "## WebSockets\n").unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    root
}

fn unique_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{}",
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

#[test]
fn indexes_ts_and_skips_env() {
    let root = setup();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.symbols >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db
        .lookup_symbols_exact("createSession", 10)
        .unwrap()
        .iter()
        .any(|h| h.name == "createSession"));
    assert!(db.get_file(".env").unwrap().is_none());
    let fts = db.fts_search("SECRET", 10).unwrap();
    assert!(fts.iter().all(|h| h.path != ".env"));
}

#[test]
fn unchanged_hash_is_noop() {
    let root = setup();
    let a = index_repo(&root, false).unwrap();
    let b = index_repo(&root, false).unwrap();
    assert!(b.unchanged >= 1);
    assert_eq!(a.symbols, b.symbols);
}

#[test]
fn change_one_file_reparses_only_that() {
    let root = setup();
    index_repo(&root, false).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 2 }\nexport function extra() {}\n",
    )
    .unwrap();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.unchanged >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.lookup_symbols_exact("extra", 10).unwrap().len() == 1);
}

#[test]
fn deleted_path_dropped() {
    let root = setup();
    index_repo(&root, false).unwrap();
    fs::remove_file(root.join("README.md")).unwrap();
    index_repo(&root, false).unwrap();
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.get_file("README.md").unwrap().is_none());
}

#[test]
fn large_file_skipped() {
    let root = setup();
    let big = vec![b'x'; 1_048_577];
    fs::write(root.join("blob.txt"), &big).unwrap();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.skipped_large >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(db.get_file("blob.txt").unwrap().is_none());
}
