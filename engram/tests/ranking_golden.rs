use engram::compile::{get_context, search_code};
use engram::index::index_repo;
use engram::store::Store;
use std::fs;
use std::path::PathBuf;

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

fn trash_fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("engram-rank-{}", unique_name()));
    fs::create_dir_all(root.join("services")).unwrap();
    fs::create_dir_all(root.join("docs/adrs")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("services/trash.ts"),
        "/** soft-delete trash items; retention purge */\n\
         export type TrashItemRow = { id: string };\n\
         export function purgeTrash() { return 1 }\n",
    )
    .unwrap();
    fs::write(
        root.join("config.ts"),
        "export const TRASH_RETENTION_DAYS = 30;\n",
    )
    .unwrap();
    fs::write(
        root.join("RemoveTagsDialog.tsx"),
        "export function RemoveTagsDialog() {\n\
         // delete tags; soft delete retention policy for tags\n\
         return 0\n\
         }\n",
    )
    .unwrap();
    fs::write(
        root.join("pdf_thumbnail.ts"),
        "export function renderPdfThumbnail() {\n\
         // cache retention of rendered pages\n\
         return 0\n\
         }\n",
    )
    .unwrap();
    fs::write(
        root.join("docs/adrs/ADR-001-trash.md"),
        "# ADR-001 Trash\n\n## Decision\n\nKeep trash for TRASH_RETENTION_DAYS then purge.\n",
    )
    .unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    root
}

fn has_path(pkg: &engram::types::ContextPackage, suffix: &str) -> bool {
    pkg.items.iter().any(|i| i.path.ends_with(suffix))
}

fn has_hit_path(hits: &[engram::store::FtsHit], suffix: &str) -> bool {
    hits.iter().any(|h| h.path.ends_with(suffix))
}

#[test]
fn fts_promotes_trash_item_row_not_only_file_snippet() {
    let root = trash_fixture();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "trash retention", 3000).unwrap();
    assert!(
        pkg.items.iter().any(|i| {
            i.path.ends_with("services/trash.ts")
                && (i.symbol.as_deref() == Some("TrashItemRow")
                    || i.symbol.as_deref() == Some("purgeTrash"))
        }),
        "expected TrashItemRow or purgeTrash, got {:?}",
        pkg.items
            .iter()
            .map(|i| (i.path.clone(), i.symbol.clone()))
            .collect::<Vec<_>>()
    );
    assert!(pkg.used_tokens <= 3000);
    assert!(pkg
        .items
        .iter()
        .all(|i| i.kind.as_deref() != Some("palace")));
}

#[test]
fn golden_trash_soft_delete_includes_trash_ts() {
    let root = trash_fixture();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "how does trash soft-delete work?", 3000).unwrap();
    assert!(
        has_path(&pkg, "services/trash.ts"),
        "items: {:?}",
        pkg.items.iter().map(|i| i.path.clone()).collect::<Vec<_>>()
    );
    assert!(pkg.used_tokens <= 3000);
    assert!(pkg
        .items
        .iter()
        .all(|i| i.kind.as_deref() != Some("palace")));
}

#[test]
fn golden_trash_retention_includes_trash_ts() {
    let root = trash_fixture();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "trash retention", 3000).unwrap();
    assert!(has_path(&pkg, "services/trash.ts"));
    assert!(pkg.used_tokens <= 3000);
}

#[test]
fn golden_trash_retention_days_includes_trash_ts() {
    let root = trash_fixture();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "TRASH_RETENTION_DAYS", 3000).unwrap();
    assert!(has_path(&pkg, "services/trash.ts"));
    assert!(
        has_path(&pkg, "config.ts")
            || pkg
                .items
                .iter()
                .any(|i| i.symbol.as_deref() == Some("TRASH_RETENTION_DAYS")),
        "config.ts or TRASH_RETENTION_DAYS should also appear"
    );
}

#[test]
fn fts_hyphen_underscore_or_fallback_includes_trash_ts() {
    let root = trash_fixture();
    index_repo(&root, true).unwrap();
    let hyphen = search_code(&root, "does trash soft-delete work", 30).unwrap();
    assert!(
        has_hit_path(&hyphen, "services/trash.ts"),
        "hyphen OR fallback hits: {:?}",
        hyphen.iter().map(|h| h.path.clone()).collect::<Vec<_>>()
    );
    assert!(hyphen.len() <= 30);
    let under = search_code(&root, "TRASH_RETENTION_DAYS", 30).unwrap();
    assert!(
        has_hit_path(&under, "services/trash.ts"),
        "underscore OR fallback hits: {:?}",
        under.iter().map(|h| h.path.clone()).collect::<Vec<_>>()
    );
    assert!(under.len() <= 30);
}
