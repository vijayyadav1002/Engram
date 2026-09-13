use engram::compile::{get_context, search_symbols};
use engram::index::index_repo;
use engram::init::run_init;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("engram-graphql-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn indexed_miniapp() -> PathBuf {
    let dir = tmp();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/miniapp");
    copy_dir(&src, &dir);
    run_init(&dir).unwrap();
    index_repo(&dir, true).unwrap();
    dir
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn get_context_quotes_graphql_type_and_operation() {
    let root = indexed_miniapp();
    let types = get_context(&root, "Reservation", 3000).unwrap();
    assert!(
        types
            .items
            .iter()
            .any(|i| { i.path.ends_with("schema.graphql") && i.text.contains("type Reservation") }),
        "Reservation package: {:?}",
        types.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let ops = get_context(&root, "GetReservation", 3000).unwrap();
    assert!(
        ops.items
            .iter()
            .any(|i| { i.path.ends_with("schema.graphql") && i.text.contains("GetReservation") }),
        "GetReservation package: {:?}",
        ops.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let fields = search_symbols(&root, "Reservation.id", 20).unwrap();
    assert!(
        fields.iter().any(|h| h.name == "Reservation.id"),
        "{fields:?}"
    );
    let bare = search_symbols(&root, "id", 50).unwrap();
    assert!(
        !bare
            .iter()
            .any(|h| h.path.ends_with("schema.graphql") && h.name == "id"),
        "bare id should not be a GraphQL field symbol: {bare:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
