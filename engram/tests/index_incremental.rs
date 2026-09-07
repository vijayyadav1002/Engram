use engram::index::index_repo;
use engram::store::Store;
use engram::types::{Confidence, EdgeKind};
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

fn setup_auth() -> PathBuf {
    let root = setup();
    fs::write(
        root.join("src/auth/LoginBanner.tsx"),
        "import { createSession } from \"./session\"; export function LoginBanner() { return createSession(); }\n",
    )
    .unwrap();
    root
}

fn banner_links_create_session(db: &Store) -> bool {
    db.lookup_symbols_exact("createSession", 10)
        .unwrap()
        .into_iter()
        .chain(db.lookup_symbols_exact("LoginBanner", 10).unwrap())
        .flat_map(|h| db.neighbors(h.id, 40).unwrap())
        .any(|n| {
            let to_cs = n.dst_name == "createSession" || n.src_name == "createSession";
            let from_banner = n.src_name == "LoginBanner"
                || n.src_path.contains("LoginBanner")
                || n.dst_path.contains("LoginBanner");
            to_cs && from_banner && matches!(n.kind, EdgeKind::Call | EdgeKind::Import)
        })
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

#[test]
fn indexes_call_or_import_to_create_session() {
    let root = setup_auth();
    index_repo(&root, false).unwrap();
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(
        banner_links_create_session(&db),
        "expected Call or Import edge involving createSession"
    );
    let call = db
        .lookup_symbols_exact("LoginBanner", 10)
        .unwrap()
        .into_iter()
        .flat_map(|h| db.neighbors(h.id, 40).unwrap())
        .find(|n| n.kind == EdgeKind::Call && n.dst_name == "createSession");
    assert!(
        call.is_some(),
        "imported createSession() call should resolve to a Call edge"
    );
    assert_eq!(call.unwrap().confidence, Confidence::High);
}

#[test]
fn reindex_session_keeps_banner_edge() {
    let root = setup_auth();
    index_repo(&root, false).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 2 }\n",
    )
    .unwrap();
    let stats = index_repo(&root, false).unwrap();
    assert!(stats.unchanged >= 1);
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    assert!(
        banner_links_create_session(&db),
        "unchanged LoginBanner must keep its edge into rewritten createSession"
    );
}

#[test]
fn name_only_call_is_low() {
    let root = setup();
    fs::write(
        root.join("src/auth/other.ts"),
        "export function other() { return createSession(); }\n",
    )
    .unwrap();
    index_repo(&root, false).unwrap();
    let db = Store::open_read(&root.join(".engram/index.sqlite")).unwrap();
    let call = db
        .lookup_symbols_exact("other", 10)
        .unwrap()
        .into_iter()
        .flat_map(|h| db.neighbors(h.id, 40).unwrap())
        .find(|n| n.kind == EdgeKind::Call && n.dst_name == "createSession");
    assert!(
        call.is_some(),
        "name-only call should resolve if dest exists"
    );
    assert_eq!(call.unwrap().confidence, Confidence::Low);
}
