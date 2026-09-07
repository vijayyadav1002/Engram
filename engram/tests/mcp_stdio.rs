use engram::index::index_repo;
use engram::store::Store;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

fn tempfile_dir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "engram-mcp-stdio-{}-{}-{}",
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mini_indexed_repo() -> PathBuf {
    let root = tempfile_dir();
    std::fs::create_dir_all(root.join("src/auth")).unwrap();
    std::fs::write(
        root.join("src/auth/session.ts"),
        "export function createSession() { return 1 }\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join(".engram")).unwrap();
    Store::create(&root.join(".engram/index.sqlite"), root.to_str().unwrap()).unwrap();
    index_repo(&root, true).unwrap();
    root
}

#[test]
fn tools_list_contains_get_context() {
    let root = mini_indexed_repo();
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    assert!(resp.contains("get_context"));
    assert!(resp.contains("search_symbols"));
    assert!(resp.contains("search_code"));
    assert!(resp.contains("index_status"));
    assert!(
        resp.contains("before searching the repo")
            || resp.contains("before grepping")
            || resp.contains("before searching")
    );
}

#[test]
fn get_context_tool_returns_package_under_16kb() {
    let root = mini_indexed_repo();
    let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_context","arguments":{"query":"createSession"}}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    assert!(resp.len() < 16_384);
    assert!(resp.contains("createSession") || resp.contains("items"));
}

#[test]
fn missing_index_is_not_initialized() {
    let root = tempfile_dir();
    let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"index_status","arguments":{}}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    assert!(resp.contains("not_initialized") || resp.contains("not initialized"));
}

#[test]
fn stdout_has_no_logs() {
    let root = mini_indexed_repo();
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    let resp = engram::mcp::handle_line(&root, req).unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
}

#[test]
fn mcp_stdio_stdout_is_jsonrpc_only() {
    let root = mini_indexed_repo();
    let mut child = Command::new(env!("CARGO_BIN_EXE_engram"))
        .arg("mcp")
        .current_dir(&root)
        .env_remove("ENGRAM_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"test","version":"0"}}}}}}"#
        )
        .unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
        )
        .unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
        )
        .unwrap();
        writeln!(stdin, "not-json").unwrap();
    }

    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 3, "stdout: {stdout:?}");
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line).expect("jsonrpc line");
        assert_eq!(v["jsonrpc"], "2.0");
    }
    assert!(lines[1].contains("get_context"));
    let parse_err: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert!(parse_err["id"].is_null());
    assert_eq!(parse_err["error"]["code"], -32700);
}
