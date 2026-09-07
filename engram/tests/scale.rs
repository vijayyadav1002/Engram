use engram::compile::get_context;
use engram::index::index_repo;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn tempfile_dir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "engram-scale-{}-{}-{}",
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

fn many_symbol_repo(n: usize) -> PathBuf {
    let root = tempfile_dir();
    for i in 0..n {
        std::fs::write(
            root.join(format!("f{i}.py")),
            format!("def name_{i}():\n    return {i}\n"),
        )
        .unwrap();
    }
    engram::init::run_init(&root).unwrap();
    root
}

#[test]
fn json_cap_sets_truncated() {
    // index many unique symbols then get_context with huge budget but compiler must still cap JSON
    let root = many_symbol_repo(200); // 200 tiny .py files each with unique def name_i
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "name", 100_000).unwrap();
    let bytes = serde_json::to_vec(&pkg).unwrap();
    assert!(bytes.len() <= 16_384);
    assert!(pkg.stats.truncated || bytes.len() < 16_384);
}

#[test]
fn scale_p95_optional() {
    if std::env::var("ENGRAM_SCALE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    let root = many_symbol_repo(10_000);
    index_repo(&root, true).unwrap();
    let start = std::time::Instant::now();
    let mut times = vec![];
    for i in 0..20 {
        let q = format!("name_{}", i * 10);
        let t0 = std::time::Instant::now();
        let _ = get_context(&root, &q, 3000).unwrap();
        times.push(t0.elapsed());
    }
    times.sort();
    let p95 = times[(times.len() * 95) / 100];
    assert!(
        p95.as_millis() <= 200,
        "p95 {:?} too slow (index {:?})",
        p95,
        start.elapsed()
    );
}

#[test]
fn doctor_reports_db() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    let out = engram::doctor::run_doctor(&root).unwrap();
    assert!(out.contains("schema_version") || out.contains("schema"));
    assert!(out.contains("index.sqlite"));
}

#[test]
fn doctor_unreadable_db() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    std::fs::remove_file(root.join(".engram/index.sqlite")).unwrap();
    let err = engram::doctor::run_doctor(&root).unwrap_err();
    assert_eq!(err.exit_code(), 3);
}

#[test]
fn doctor_missing_harness_warns_exit_zero() {
    let root = tempfile_dir();
    engram::init::run_init(&root).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_engram"))
        .args(["doctor"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("absent"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warn"),
        "expected harness-missing warn on stderr, got {stderr:?}"
    );
}
