use engram::compile::{get_context, search_symbols};
use engram::index::index_repo;
use engram::init::run_init;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("engram-json-yaml-{}-{}", std::process::id(), n));
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
fn get_context_quotes_json_and_yaml_keys() {
    let root = indexed_miniapp();
    let scripts = get_context(&root, "scripts", 3000).unwrap();
    assert!(
        scripts.items.iter().any(|i| {
            i.path.ends_with("package.json") && i.text.contains("\"test\"")
        }),
        "scripts package: {:?}",
        scripts.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let nested = get_context(&root, "scripts.test", 3000).unwrap();
    assert!(
        nested.items.iter().any(|i| {
            i.path.ends_with("package.json") && i.text.contains("echo ok")
        }),
        "scripts.test package: {:?}",
        nested.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let services = get_context(&root, "services", 3000).unwrap();
    assert!(
        services.items.iter().any(|i| {
            i.path.ends_with("values.yaml") && i.text.contains("web")
        }),
        "services package: {:?}",
        services.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    let fields = search_symbols(&root, "scripts.test", 20).unwrap();
    assert!(
        fields.iter().any(|h| h.name == "scripts.test"),
        "{fields:?}"
    );
    let bare = search_symbols(&root, "test", 50).unwrap();
    assert!(
        !bare.iter().any(|h| {
            (h.path.ends_with("package.json") || h.path.ends_with("values.yaml"))
                && h.name == "test"
        }),
        "bare test must not be a JSON/YAML heading: {bare:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
