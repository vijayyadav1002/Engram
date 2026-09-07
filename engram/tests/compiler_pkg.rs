use engram::compile::{get_context, plan_query};
use engram::index::index_repo;
use engram::store::Store;
use engram::types::ContextPackage;
use std::fs;
use std::path::PathBuf;

fn repo() -> PathBuf {
    let root = std::env::temp_dir().join(format!("engram-cmp-{}", unique_name()));
    fs::create_dir_all(root.join("src/auth")).unwrap();
    fs::create_dir_all(root.join("styles")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 1 }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/auth/LoginBanner.tsx"),
        "import { createSession } from \"./session\"; export function LoginBanner() { return createSession(); }\n",
    )
    .unwrap();
    fs::write(
        root.join("README.md"),
        "## WebSockets\n\nWe use websockets.\n",
    )
    .unwrap();
    fs::write(root.join("styles/auth.css"), ".auth-panel { color: red }\n").unwrap();
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
fn exact_symbol_quotes_span() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    assert!(pkg
        .items
        .iter()
        .any(|i| i.symbol.as_deref() == Some("createSession")));
    assert!(pkg.items.iter().any(|i| i.text.contains("createSession")));
    assert!(pkg
        .items
        .iter()
        .any(|i| i.why.iter().any(|w| w == "exact_symbol")));
}

#[test]
fn fts_hits_heading() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "WebSockets", 3000).unwrap();
    assert!(pkg
        .items
        .iter()
        .any(|i| i.path.ends_with("README.md") && i.text.contains("WebSockets")));
}

#[test]
fn small_budget_drops_low_rank() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let tiny = get_context(&root, "createSession WebSockets auth-panel", 20).unwrap();
    let big = get_context(&root, "createSession WebSockets auth-panel", 3000).unwrap();
    assert!(tiny.items.len() <= big.items.len());
    assert!(tiny.used_tokens <= 20 || tiny.items.is_empty());
}

#[test]
fn stale_hash_omits_span() {
    let root = repo();
    index_repo(&root, true).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 99 }\n",
    )
    .unwrap();
    // do not reindex
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    assert!(pkg.stats.stale_omitted >= 1 || pkg.stats.stale_index);
    assert!(!pkg.items.iter().any(|i| i.text.contains("return 99")));
}

#[test]
fn empty_query_has_no_invented_text() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "zzzxnotarealsymbolzzz", 3000).unwrap();
    assert!(pkg.items.is_empty() || pkg.items.iter().all(|i| !i.text.is_empty()));
    let json = serde_json::to_string(&pkg).unwrap();
    assert!(!json.contains("summary"));
    let _: &ContextPackage = &pkg;
    let _ = plan_query("zzzxnotarealsymbolzzz");
}
