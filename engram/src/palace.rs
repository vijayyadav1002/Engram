pub const PALACE_MAX_HITS: usize = 3;
pub const PALACE_ITEM_MAX_CHARS: usize = 1200;
pub const PALACE_MIN_REMAINING: u32 = 200;
pub const PALACE_TIMEOUT_MS: u64 = 2500;
pub const PALACE_QUERY_MAX_CHARS: usize = 250;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalaceDrawer {
    pub wing: String,
    pub room: String,
    pub source: String,
    pub text: String,
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

/// Parse `mempalace search` CLI stdout into drawers (header + optional Source + `→` body).
pub fn parse_search_output(stdout: &str) -> Vec<PalaceDrawer> {
    let mut hits = Vec::new();
    let lines: Vec<&str> = stdout.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some((wing, room)) = parse_hit_header(trimmed) {
            i += 1;
            let mut source = String::new();
            while i < lines.len() {
                let t = lines[i].trim_start();
                if t.starts_with('[') && parse_hit_header(t).is_some() {
                    break;
                }
                if is_rule_line(t) {
                    break;
                }
                if let Some(rest) = t.strip_prefix("Source:") {
                    source = rest.trim().to_string();
                    i += 1;
                    continue;
                }
                if let Some(body_start) = t.strip_prefix('→') {
                    let mut body = String::new();
                    body.push_str(body_start.trim_start());
                    i += 1;
                    while i < lines.len() {
                        let next = lines[i];
                        let nt = next.trim_start();
                        if nt.starts_with('[') && parse_hit_header(nt).is_some() {
                            break;
                        }
                        if is_rule_line(nt) {
                            break;
                        }
                        if nt.starts_with('→') {
                            break;
                        }
                        // Indented continuation lines belong to the body.
                        if next.starts_with(' ') || next.starts_with('\t') {
                            if !body.is_empty() {
                                body.push('\n');
                            }
                            body.push_str(nt);
                            i += 1;
                            continue;
                        }
                        if nt.is_empty() {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    hits.push(PalaceDrawer {
                        wing,
                        room,
                        source,
                        text: body,
                    });
                    break;
                }
                i += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_empty_is_empty_vec() {
        assert!(parse_search_output("no results\n").is_empty());
    }
}
