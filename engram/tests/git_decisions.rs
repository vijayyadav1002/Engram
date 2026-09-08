use engram::compile::{get_context, get_context_with, GetContextOpts};
use engram::git::{FakeGitSource, GitCommit};
use engram::index::{index_repo_with, IndexOpts};
use engram::store::Store;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn repo() -> PathBuf {
    let root = std::env::temp_dir().join(format!("engram-git-dec-{}", unique_name()));
    fs::create_dir_all(root.join("src/auth")).unwrap();
    fs::create_dir_all(root.join("docs/adr")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 1 }\n",
    )
    .unwrap();
    fs::write(
        root.join("docs/adr/007-websockets.md"),
        "# Use WebSockets\n\n## Decision\n\nUse WebSockets instead of polling.\n",
    )
    .unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    let fake = FakeGitSource {
        head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ancestor: false,
        commits: vec![GitCommit {
            sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            author: "Ada".into(),
            authored_at: "2026-01-01T00:00:00Z".into(),
            subject: "use websockets".into(),
            body: "replace polling".into(),
            files: vec!["src/auth/session.ts".into()],
        }],
        head_err: None,
        log_err: None,
    };
    index_repo_with(
        &root,
        true,
        IndexOpts {
            git: Some(Arc::new(fake)),
        },
    )
    .unwrap();
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
fn why_websockets_includes_decision_commit_and_code() {
    let root = repo();
    let pkg = get_context(&root, "why WebSockets", 3000).unwrap();
    assert!(pkg
        .items
        .iter()
        .any(|i| i.kind.as_deref() == Some("decision")));
    assert!(pkg
        .items
        .iter()
        .any(|i| { i.kind.as_deref() == Some("commit") && i.path.starts_with("git://") }));
    assert!(pkg
        .items
        .iter()
        .any(|i| i.symbol.as_deref() == Some("createSession")));
    assert_eq!(pkg.stats.git.status, "ok");
    assert!(pkg.stats.git.commits_considered >= 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn exact_symbol_outranks_weak_commit() {
    let root = repo();
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    let first_symbol = pkg
        .items
        .iter()
        .find(|i| i.symbol.as_deref() == Some("createSession"));
    assert!(first_symbol.is_some());
    let pos_sym = pkg
        .items
        .iter()
        .position(|i| i.symbol.as_deref() == Some("createSession"))
        .unwrap();
    if let Some(pos_commit) = pkg
        .items
        .iter()
        .position(|i| i.kind.as_deref() == Some("commit"))
    {
        assert!(pos_sym < pos_commit);
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn small_budget_can_drop_commit() {
    let root = repo();
    let pkg = get_context(&root, "why WebSockets", 400).unwrap();
    assert!(pkg
        .items
        .iter()
        .any(|i| i.symbol.as_deref() == Some("createSession")
            || i.kind.as_deref() == Some("decision")));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn palace_still_omitted_by_default() {
    let root = repo();
    let pkg = get_context_with(&root, "why WebSockets", 3000, GetContextOpts::default()).unwrap();
    assert!(pkg.stats.palace.is_none());
    let _ = fs::remove_dir_all(&root);
}
