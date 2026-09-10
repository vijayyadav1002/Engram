use engram::compile::{get_context, get_context_with, GetContextOpts};
use engram::index::index_repo;
use engram::palace::{FakePalaceSearch, PalaceDrawer, PalaceError, PalaceSearch};
use engram::store::Store;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn repo() -> PathBuf {
    let root = std::env::temp_dir().join(format!("engram-palace-bridge-{}", unique_name()));
    fs::create_dir_all(root.join("src/auth")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 1 }\n",
    )
    .unwrap();
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

fn est_token_cost(text: &str) -> u32 {
    text.split_whitespace().count() as u32 + 2
}

struct CountingSearch {
    inner: FakePalaceSearch,
    calls: Arc<AtomicUsize>,
}

impl PalaceSearch for CountingSearch {
    fn search(
        &self,
        query: &str,
        limit: usize,
        wing: Option<&str>,
        room: Option<&str>,
    ) -> Result<Vec<PalaceDrawer>, PalaceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.search(query, limit, wing, room)
    }
}

fn write_wing(root: &std::path::Path, wing: &str) {
    std::fs::write(
        root.join(".engram/config.toml"),
        format!("palace_wing = \"{wing}\"\n"),
    )
    .unwrap();
}

#[test]
fn disabled_omits_palace_key() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    let v = serde_json::to_value(&pkg).unwrap();
    assert!(v["stats"].get("palace").is_none());
    assert!(pkg
        .items
        .iter()
        .all(|i| i.kind.as_deref() != Some("palace")));
}

#[test]
fn missing_searcher_sets_not_installed() {
    let root = repo();
    index_repo(&root, true).unwrap();
    write_wing(&root, "w");
    let fake = Arc::new(FakePalaceSearch {
        drawers: vec![],
        error: Some(PalaceError::NotInstalled),
    });
    let pkg = get_context_with(
        &root,
        "createSession",
        3000,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    assert_eq!(pkg.stats.palace.as_ref().unwrap().status, "not_installed");
    assert!(pkg
        .items
        .iter()
        .any(|i| i.symbol.as_deref() == Some("createSession")));
}

#[test]
fn fake_three_drawers_budget_keeps_two() {
    let root = repo();
    index_repo(&root, true).unwrap();
    write_wing(&root, "w");
    let drawer_text = "word ".repeat(80);
    let drawers: Vec<PalaceDrawer> = (0..3)
        .map(|i| PalaceDrawer {
            wing: "w".into(),
            room: format!("r{i}"),
            source: "s".into(),
            text: drawer_text.clone(),
            cosine: None,
        })
        .collect();
    let fake = Arc::new(FakePalaceSearch {
        drawers,
        error: None,
    });
    let code = get_context(&root, "createSession", 3000).unwrap();
    let cost = est_token_cost(&drawer_text);
    // Remaining must be >= 200 to search; third drawer must not fit.
    let extra = (cost * 2).max(200) + 1;
    let budget = code.used_tokens + extra;
    let pkg = get_context_with(
        &root,
        "createSession",
        budget,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    let p = pkg.stats.palace.unwrap();
    assert_eq!(p.attempted, 3);
    assert_eq!(p.included, 2);
    assert_eq!(p.dropped_for_budget, 1);
    assert_eq!(
        pkg.items
            .iter()
            .filter(|i| i.why.iter().any(|w| w == "palace"))
            .count(),
        2
    );
    assert!(pkg.items.iter().any(|i| i.path.starts_with("palace://")));
    assert!(pkg
        .edges
        .iter()
        .all(|e| !e.from.starts_with("palace://") && !e.to.starts_with("palace://")));
}

#[test]
fn timeout_keeps_code_items() {
    let root = repo();
    index_repo(&root, true).unwrap();
    write_wing(&root, "w");
    let fake = Arc::new(FakePalaceSearch {
        drawers: vec![],
        error: Some(PalaceError::Timeout),
    });
    let pkg = get_context_with(
        &root,
        "createSession",
        3000,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    assert_eq!(pkg.stats.palace.as_ref().unwrap().status, "timeout");
    assert!(!pkg.items.is_empty());
}

#[test]
fn remaining_below_min_skips_searcher() {
    let root = repo();
    index_repo(&root, true).unwrap();
    write_wing(&root, "w");
    let code = get_context(&root, "createSession", 3000).unwrap();
    let budget = code.used_tokens + 50;
    let fake = Arc::new(FakePalaceSearch {
        drawers: vec![],
        error: Some(PalaceError::NotInstalled),
    });
    let pkg = get_context_with(
        &root,
        "createSession",
        budget,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    let p = pkg.stats.palace.as_ref().unwrap();
    assert_eq!(p.status, "ok");
    assert_eq!(p.attempted, 0);
    assert_eq!(p.included, 0);
    assert_eq!(p.dropped_for_budget, 0);
    assert!(pkg
        .items
        .iter()
        .all(|i| i.kind.as_deref() != Some("palace")));
}

#[test]
fn missing_wing_is_unscoped_disabled_and_does_not_search() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let fake = Arc::new(CountingSearch {
        inner: FakePalaceSearch {
            drawers: vec![PalaceDrawer {
                wing: "sessions".into(),
                room: "technical".into(),
                source: "s".into(),
                text: "should not attach".into(),
                cosine: Some(0.9),
            }],
            error: None,
        },
        calls: calls.clone(),
    });
    let pkg = get_context_with(
        &root,
        "createSession",
        3000,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        pkg.stats.palace.as_ref().unwrap().status,
        "unscoped_disabled"
    );
    assert!(pkg
        .items
        .iter()
        .all(|i| i.kind.as_deref() != Some("palace")));
}
