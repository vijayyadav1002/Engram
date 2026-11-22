use engram::compile::get_context;
use engram::index::index_repo;
use engram::store::Store;
use std::fs;

#[test]
fn authentication_question_finds_verify_token() {
    let root = std::env::temp_dir().join(format!(
        "engram-alias-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src/auth")).unwrap();
    fs::create_dir_all(root.join(".engram")).unwrap();
    fs::write(
        root.join("src/auth/session.ts"),
        "export type Session = { guestId: string };\n\
         export function createSession(guestId: string) { return guestId }\n\
         export function verifyToken(token: string): Session | null { return token ? { guestId: token } : null }\n",
    )
    .unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "where is authentication handled?", 3000).unwrap();
    assert!(
        pkg.items.iter().any(|item| {
            item.path.ends_with("src/auth/session.ts")
                && matches!(
                    item.symbol.as_deref(),
                    Some("verifyToken") | Some("createSession") | Some("Session")
                )
        }),
        "expected a session symbol, got {:?}",
        pkg.items
            .iter()
            .map(|item| (item.path.clone(), item.symbol.clone(), item.why.clone()))
            .collect::<Vec<_>>()
    );
}
