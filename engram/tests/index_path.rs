use engram::index::{index_repo, index_repo_with, IndexOpts};
use engram::store::Store;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "engram-index-path-{}-{}-{}",
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn workspace() -> PathBuf {
    let root = tmp();
    fs::create_dir_all(root.join("apps/web/src")).unwrap();
    fs::create_dir_all(root.join("apps/api/src")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("apps/web/src/a.ts"),
        "export function ping() { return 1 }\n",
    )
    .unwrap();
    fs::write(
        root.join("apps/api/src/b.ts"),
        "export function pong() { return 2 }\n",
    )
    .unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    root
}

fn db(root: &std::path::Path) -> Store {
    Store::open_read(&root.join(".engram/index.sqlite")).unwrap()
}

#[test]
fn path_prefixes_rows_and_does_not_write_nested_engram() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("src/a.ts").unwrap().is_none());
    assert!(store.get_file("apps/web/src/a.ts").unwrap().is_none());
    assert!(root.join(".engram/index.sqlite").is_file());
    assert!(!root.join("apps/web/.engram").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn second_project_accumulates() {
    let root = workspace();
    let opts_web = IndexOpts {
        walk: Some(root.join("apps/web")),
        ..Default::default()
    };
    let opts_api = IndexOpts {
        walk: Some(root.join("apps/api")),
        ..Default::default()
    };
    index_repo_with(&root, false, opts_web).unwrap();
    index_repo_with(&root, false, opts_api).unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("api/src/b.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn path_delete_only_touches_that_prefix() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/api")),
            ..Default::default()
        },
    )
    .unwrap();
    fs::remove_file(root.join("apps/web/src/a.ts")).unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_none());
    assert!(store.get_file("api/src/b.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn force_path_does_not_drop_other_prefix() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/api")),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        true,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("api/src/b.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn walk_equals_root_is_unprefixed() {
    let root = workspace();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("apps/web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("web/src/a.ts").unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_engramignore_applies_to_nested_walk() {
    let root = workspace();
    fs::write(root.join(".engramignore"), "ignored.ts\n").unwrap();
    fs::write(root.join("apps/web/src/ignored.ts"), "export const x = 1\n").unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("web/src/ignored.ts").unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn bare_index_still_unprefixed() {
    let root = workspace();
    index_repo(&root, false).unwrap();
    let store = db(&root);
    assert!(store.get_file("apps/web/src/a.ts").unwrap().is_some());
    assert!(store.get_file("web/src/a.ts").unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}

use engram::git::{FakeGitSource, GitCommit, GitIndexStatus};
use std::sync::Arc;

fn fake_web() -> FakeGitSource {
    FakeGitSource {
        head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ancestor: false,
        commits: vec![GitCommit {
            sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            author: "Ada".into(),
            authored_at: "2026-01-01T00:00:00Z".into(),
            subject: "use websockets".into(),
            body: "replace polling".into(),
            files: vec!["src/a.ts".into()],
        }],
        head_err: None,
        log_err: None,
    }
}

#[test]
fn nested_git_prefixes_commit_files_and_leaves_meta_head() {
    let root = workspace();
    fs::create_dir_all(root.join("apps/web/.git")).unwrap();
    let before = db(&root).meta().unwrap().git_head.clone();
    let stats = index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.git, GitIndexStatus::Ok);
    assert!(stats.commits >= 1);
    let store = db(&root);
    assert_eq!(store.meta().unwrap().git_head, before);
    let hits = store.search_commits_fts("websockets", 5).unwrap();
    assert_eq!(hits.len(), 1);
    let files = store
        .list_files()
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect::<Vec<_>>();
    assert!(files.iter().any(|p| p == "web/src/a.ts"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn nested_without_git_is_absent_and_inserts_no_commits() {
    let root = workspace();
    let stats = index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.git, GitIndexStatus::Absent);
    assert_eq!(stats.commits, 0);
    let store = db(&root);
    assert!(store
        .search_commits_fts("websockets", 5)
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn force_path_does_not_clear_commits() {
    let root = workspace();
    fs::create_dir_all(root.join("apps/web/.git")).unwrap();
    index_repo_with(
        &root,
        false,
        IndexOpts {
            walk: Some(root.join("apps/web")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    index_repo_with(
        &root,
        true,
        IndexOpts {
            walk: Some(root.join("apps/api")),
            git: Some(Arc::new(fake_web())),
            ..Default::default()
        },
    )
    .unwrap();
    let store = db(&root);
    assert_eq!(store.search_commits_fts("websockets", 5).unwrap().len(), 1);
    assert!(store.get_file("web/src/a.ts").unwrap().is_some());
    let _ = fs::remove_dir_all(&root);
}
