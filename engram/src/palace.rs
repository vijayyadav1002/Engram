use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub const PALACE_MAX_HITS: usize = 3;
pub const PALACE_ITEM_MAX_CHARS: usize = 1200;
pub const PALACE_MIN_REMAINING: u32 = 200;
pub const PALACE_TIMEOUT_MS: u64 = 8000;
pub const PALACE_QUERY_MAX_CHARS: usize = 250;
pub const PALACE_MIN_COSINE_DEFAULT: f64 = 0.6;

#[derive(Debug, Clone, PartialEq)]
pub struct PalaceDrawer {
    pub wing: String,
    pub room: String,
    pub source: String,
    pub text: String,
    pub cosine: Option<f64>,
}

/// First `PALACE_QUERY_MAX_CHARS` characters of `query`.
pub fn truncate_query(query: &str) -> String {
    query.chars().take(PALACE_QUERY_MAX_CHARS).collect()
}

/// At most `PALACE_ITEM_MAX_CHARS` characters; appends `…` when cut.
pub fn truncate_drawer_text(text: &str) -> String {
    let count = text.chars().count();
    if count <= PALACE_ITEM_MAX_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(PALACE_ITEM_MAX_CHARS).collect();
    out.push('…');
    out
}

/// Parse `mempalace search` CLI stdout into drawers.
///
/// Accepts MemPalace 3.3.x (indented body after Source/Match, no arrow)
/// and the legacy synthetic format (body line starts with `→`).
pub fn parse_search_output(stdout: &str) -> Vec<PalaceDrawer> {
    let mut hits = Vec::new();
    let lines: Vec<&str> = stdout.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some((wing, room)) = parse_hit_header(trimmed) {
            i += 1;
            let mut source = String::new();
            let mut body = String::new();
            let mut cosine = None;
            let mut in_body = false;
            while i < lines.len() {
                let next = lines[i];
                let nt = next.trim_start();
                if nt.starts_with('[') && parse_hit_header(nt).is_some() {
                    break;
                }
                if is_rule_line(nt) {
                    break;
                }
                if !in_body {
                    if let Some(rest) = nt.strip_prefix("Source:") {
                        source = rest.trim().to_string();
                        i += 1;
                        continue;
                    }
                    if nt.starts_with("Match:") {
                        cosine = parse_cosine_from_match(nt);
                        i += 1;
                        continue;
                    }
                    if nt.is_empty() {
                        i += 1;
                        continue;
                    }
                    in_body = true;
                    let start = nt
                        .strip_prefix('→')
                        .map(str::trim_start)
                        .unwrap_or(nt);
                    body.push_str(start);
                    i += 1;
                    continue;
                }
                if nt.is_empty() {
                    i += 1;
                    continue;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                let piece = nt
                    .strip_prefix('→')
                    .map(str::trim_start)
                    .unwrap_or(nt);
                body.push_str(piece);
                i += 1;
            }
            if !body.is_empty() {
                hits.push(PalaceDrawer {
                    wing,
                    room,
                    source,
                    text: body,
                    cosine,
                });
            }
            continue;
        }
        i += 1;
    }
    hits
}

fn parse_hit_header(trimmed: &str) -> Option<(String, String)> {
    // [N] <wing> / <room>
    if !trimmed.starts_with('[') {
        return None;
    }
    let after_bracket = trimmed.strip_prefix('[')?;
    let close = after_bracket.find(']')?;
    let idx = &after_bracket[..close];
    if idx.is_empty() || !idx.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = after_bracket[close + 1..].trim_start();
    let (wing, room) = rest.split_once(" / ")?;
    let wing = wing.trim();
    let room = room.trim();
    if wing.is_empty() || room.is_empty() {
        return None;
    }
    Some((wing.to_string(), room.to_string()))
}

fn is_rule_line(trimmed: &str) -> bool {
    trimmed.starts_with('─') || trimmed.starts_with("──")
}

fn parse_cosine_from_match(trimmed: &str) -> Option<f64> {
    let rest = trimmed.strip_prefix("Match:")?;
    for part in rest.split_whitespace() {
        if let Some(v) = part.strip_prefix("cosine=") {
            return v.parse().ok();
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PalaceError {
    NotInstalled,
    Timeout,
    Unparseable,
    Io(String),
}

pub trait PalaceSearch: Send + Sync {
    fn search(&self, query: &str, limit: usize) -> Result<Vec<PalaceDrawer>, PalaceError>;
}

#[derive(Debug, Clone)]
pub struct CliPalaceSearch {
    pub bin: PathBuf,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct FakePalaceSearch {
    pub drawers: Vec<PalaceDrawer>,
    pub error: Option<PalaceError>,
}

pub fn default_bin() -> PathBuf {
    match std::env::var_os("ENGRAM_PALACE_BIN") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("mempalace"),
    }
}

pub fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout_ms: u64,
) -> Result<std::process::Output, PalaceError> {
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(timeout_ms));
        let _ = tx.send(());
    });

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(PalaceError::Timeout);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            },
            Err(e) => return Err(PalaceError::Io(e.to_string())),
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

impl PalaceSearch for FakePalaceSearch {
    fn search(&self, _query: &str, _limit: usize) -> Result<Vec<PalaceDrawer>, PalaceError> {
        match &self.error {
            Some(err) => Err(err.clone()),
            None => Ok(self.drawers.clone()),
        }
    }
}

impl PalaceSearch for CliPalaceSearch {
    fn search(&self, query: &str, limit: usize) -> Result<Vec<PalaceDrawer>, PalaceError> {
        let limit = limit.min(PALACE_MAX_HITS);
        let mut child = match Command::new(&self.bin)
            .args([
                "search",
                "--results",
                &limit.to_string(),
                &truncate_query(query),
            ])
            .current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(PalaceError::NotInstalled);
            }
            Err(e) => return Err(PalaceError::Io(e.to_string())),
        };
        let output = wait_with_timeout(&mut child, self.timeout_ms)?;
        if !output.stderr.is_empty() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let hits = parse_search_output(&stdout);
        if hits.is_empty() {
            Err(PalaceError::Unparseable)
        } else {
            Ok(hits)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalaceOpt {
    Enable,
    Disable,
    Unspecified,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PalaceFileConfig {
    pub opt: PalaceOpt,
    pub wing: Option<String>,
    pub room: Option<String>,
    pub min_cosine: f64,
}

/// Line-oriented palace file keys; last assignment wins; `#` comments ignored.
pub fn parse_palace_file_config(text: &str) -> PalaceFileConfig {
    let mut cfg = PalaceFileConfig {
        opt: PalaceOpt::Unspecified,
        wing: None,
        room: None,
        min_cosine: PALACE_MIN_COSINE_DEFAULT,
    };
    for line in text.lines() {
        let line = strip_toml_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "palace" => match value {
                "true" => cfg.opt = PalaceOpt::Enable,
                "false" => cfg.opt = PalaceOpt::Disable,
                _ => {}
            },
            "palace_wing" => cfg.wing = nonempty_unquoted(value),
            "palace_room" => cfg.room = nonempty_unquoted(value),
            "palace_min_cosine" => {
                if let Ok(v) = unquote_toml(value).parse::<f64>() {
                    if (0.0..=2.0).contains(&v) {
                        cfg.min_cosine = v;
                    }
                }
            }
            _ => {}
        }
    }
    cfg
}

/// Last `palace = true` / `palace = false` line wins; `#` comments ignored.
pub fn parse_palace_config_toml(text: &str) -> PalaceOpt {
    parse_palace_file_config(text).opt
}

fn unquote_toml(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

fn nonempty_unquoted(value: &str) -> Option<String> {
    let s = unquote_toml(value).trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn strip_toml_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn env_disables(env_palace: &str) -> bool {
    matches!(
        env_palace.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no"
    )
}

fn env_enables(env_palace: &str) -> bool {
    matches!(
        env_palace.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

/// Resolve palace opt-in. Disable always wins (`explicit false` or env `0`/`false`/`no`).
pub fn resolve_opt_in(
    explicit: Option<bool>,
    env_palace: Option<&str>,
    config_text: Option<&str>,
) -> bool {
    if explicit == Some(false) {
        return false;
    }
    if env_palace.is_some_and(env_disables) {
        return false;
    }
    if explicit == Some(true) {
        return true;
    }
    if env_palace.is_some_and(env_enables) {
        return true;
    }
    matches!(
        parse_palace_config_toml(config_text.unwrap_or("")),
        PalaceOpt::Enable
    )
}

pub fn read_config_text(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join(".engram/config.toml")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn palace_timeout_ms_is_eight_seconds() {
        assert_eq!(PALACE_TIMEOUT_MS, 8000);
    }

    #[test]
    fn truncate_query_caps_at_250() {
        let q = "a".repeat(300);
        let t = truncate_query(&q);
        assert_eq!(t.len(), 250);
    }

    #[test]
    fn truncate_drawer_appends_ellipsis() {
        let t = truncate_drawer_text(&"x".repeat(1201));
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 1201); // 1200 + ellipsis
    }

    #[test]
    fn parse_two_cli_hits() {
        let out = r#"============================================================
  Results for: "websockets"
============================================================

  [1] sessions / architecture
      Source: abc.jsonl
      Match:  cosine=0.4  bm25=0.0

      → We switched to WebSockets instead of polling.

  ────────────────────────────────────────────────────────

  [2] myapp / decisions
      Source: def.jsonl
      Match:  cosine=0.3  bm25=1.0

      → Keep polling for now.
"#;
        let hits = parse_search_output(out);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].wing, "sessions");
        assert_eq!(hits[0].room, "architecture");
        assert_eq!(hits[0].source, "abc.jsonl");
        assert!(hits[0].text.contains("WebSockets"));
        assert_eq!(hits[1].wing, "myapp");
        assert!(hits[1].text.contains("polling"));
    }

    #[test]
    fn parse_mempalace_3_3_5_fixture_without_arrow() {
        let out = include_str!("../testdata/mempalace-search-3.3.5.txt");
        assert!(
            !out.contains('→'),
            "fixture must be live CLI shape, not the synthetic arrow format"
        );
        let hits = parse_search_output(out);
        assert!(hits.len() >= 2, "got {} hits", hits.len());
        assert_eq!(hits[0].wing, "sessions");
        assert_eq!(hits[0].room, "technical");
        assert_eq!(hits[0].source, "summary.json");
        assert!(
            hits[0].text.contains("does not store conversations"),
            "body was {:?}",
            hits[0].text
        );
        assert!(hits[0].text.contains('}'), "JSON fragment brace is drawer text");
        assert_eq!(hits[1].source, "segment_000.md");
        assert!(hits[1].text.contains("TSX extractor"));
        assert_eq!(hits[0].cosine, Some(0.686));
        assert_eq!(hits[1].cosine, Some(0.488));
    }

    #[test]
    fn parse_file_config_reads_wing_room_floor() {
        let cfg = parse_palace_file_config(
            "palace = false\n\
             palace_wing = \"engram\"\n\
             palace_room = \"decisions\"\n\
             palace_min_cosine = 0.6\n",
        );
        assert_eq!(cfg.opt, PalaceOpt::Disable);
        assert_eq!(cfg.wing.as_deref(), Some("engram"));
        assert_eq!(cfg.room.as_deref(), Some("decisions"));
        assert!((cfg.min_cosine - 0.6).abs() < 1e-9);
    }

    #[test]
    fn parse_file_config_empty_wing_is_none() {
        let cfg = parse_palace_file_config("palace_wing = \"\"\n");
        assert!(cfg.wing.is_none());
        assert!((cfg.min_cosine - PALACE_MIN_COSINE_DEFAULT).abs() < 1e-9);
    }

    #[test]
    fn parse_file_config_bad_cosine_keeps_default() {
        let cfg = parse_palace_file_config("palace_min_cosine = 9\n");
        assert!((cfg.min_cosine - PALACE_MIN_COSINE_DEFAULT).abs() < 1e-9);
    }

    #[test]
    fn parse_cosine_from_match_line() {
        let out = "  [1] sessions / technical\n      Source: summary.json\n      Match:  cosine=0.686  bm25=1.644\n\n      body\n";
        let hits = parse_search_output(out);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "body");
        assert_eq!(hits[0].cosine, Some(0.686));
    }

    #[test]
    fn parse_missing_cosine_is_none() {
        let out = "  [1] w / r\n      Source: s\n      hello\n";
        let hits = parse_search_output(out);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].cosine, None);
    }

    #[test]
    fn parse_skips_match_metadata_line() {
        let out = "  [1] w / r\n      Match:  cosine=0.1  bm25=0.0\n      body only\n";
        let hits = parse_search_output(out);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "body only");
        assert!(!hits[0].text.contains("cosine"));
    }

    #[test]
    fn parse_empty_is_empty_vec() {
        assert!(parse_search_output("no results\n").is_empty());
    }

    #[test]
    fn fake_search_returns_drawers() {
        let fake = FakePalaceSearch {
            drawers: vec![PalaceDrawer {
                wing: "w".into(),
                room: "r".into(),
                source: "s".into(),
                text: "hello".into(),
                cosine: None,
            }],
            error: None,
        };
        let hits = fake.search("q", 3).unwrap();
        assert_eq!(hits[0].text, "hello");
    }

    #[test]
    fn fake_search_propagates_not_installed() {
        let fake = FakePalaceSearch {
            drawers: vec![],
            error: Some(PalaceError::NotInstalled),
        };
        assert!(matches!(
            fake.search("q", 3),
            Err(PalaceError::NotInstalled)
        ));
    }

    #[test]
    fn cli_missing_bin_is_not_installed() {
        let cli = CliPalaceSearch {
            bin: PathBuf::from("/this/binary/does/not/exist-engram-test"),
            cwd: std::env::temp_dir(),
            timeout_ms: 500,
        };
        assert!(matches!(cli.search("q", 3), Err(PalaceError::NotInstalled)));
    }

    #[test]
    fn default_bin_uses_env_or_mempalace() {
        match std::env::var_os("ENGRAM_PALACE_BIN") {
            Some(p) => assert_eq!(default_bin(), PathBuf::from(p)),
            None => assert_eq!(default_bin(), PathBuf::from("mempalace")),
        }
    }

    #[cfg(unix)]
    #[test]
    fn cli_timeout_kills_child() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("engram-palace-timeout-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("sleep_search");
        fs::write(&bin, "#!/bin/sh\nsleep 5\n").unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        let cli = CliPalaceSearch {
            bin,
            cwd: std::env::temp_dir(),
            timeout_ms: 200,
        };
        let started = std::time::Instant::now();
        assert!(matches!(cli.search("q", 3), Err(PalaceError::Timeout)));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "timeout should kill the child instead of waiting out sleep 5"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_empty_parse_is_unparseable() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("engram-palace-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("empty_search");
        fs::write(&bin, "#!/bin/sh\necho no results\n").unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        let cli = CliPalaceSearch {
            bin,
            cwd: std::env::temp_dir(),
            timeout_ms: 500,
        };
        assert!(matches!(cli.search("q", 3), Err(PalaceError::Unparseable)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_off() {
        assert!(!resolve_opt_in(None, None, None));
    }

    #[test]
    fn env_zero_disables_even_if_config_true() {
        assert!(!resolve_opt_in(None, Some("0"), Some("palace = true\n")));
    }

    #[test]
    fn explicit_false_wins() {
        assert!(!resolve_opt_in(Some(false), Some("1"), Some("palace = true\n")));
    }

    #[test]
    fn explicit_true_enables() {
        assert!(resolve_opt_in(Some(true), None, None));
    }

    #[test]
    fn config_true_enables() {
        assert!(resolve_opt_in(None, None, Some("# hi\npalace = true\n")));
    }

    #[test]
    fn env_one_enables() {
        assert!(resolve_opt_in(None, Some("1"), None));
    }
}
