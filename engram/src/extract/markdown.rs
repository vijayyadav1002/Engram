use crate::types::{
    Confidence, EdgeKind, ExtractedEdge, ExtractedSymbol, Extraction, ParseStatus, SymbolKind,
};
use regex::Regex;
use std::sync::OnceLock;

const MAX_DECISION_LINES: u32 = 80;

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").expect("heading regex"))
}

fn adr_filename_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^adr[-_ ]?[0-9]+").expect("adr filename regex"))
}

fn decision_heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^decision\s*$").expect("decision heading regex"))
}

fn supersedes_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)supersedes\s+ADR-?\s*([0-9]+)").expect("supersedes regex"))
}

fn md_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[^\]]+\]\(([^)]+)\)").expect("markdown link regex"))
}

fn posix_norm(rel_posix: &str) -> String {
    let mut s = rel_posix.replace('\\', "/");
    while s.starts_with("./") {
        s.replace_range(..2, "");
    }
    s
}

pub fn is_adr_path(rel_posix: &str) -> bool {
    let s = posix_norm(rel_posix).to_ascii_lowercase();
    if s.split('/')
        .any(|seg| matches!(seg, "adr" | "adrs" | "decisions"))
    {
        return true;
    }
    let filename = s.rsplit('/').next().unwrap_or(s.as_str());
    adr_filename_re().is_match(filename)
}

fn filename_stem(rel_posix: &str) -> String {
    let s = posix_norm(rel_posix);
    let last = s.rsplit('/').next().unwrap_or(s.as_str());
    match last.rfind('.') {
        Some(i) if i > 0 => last[..i].to_string(),
        _ => last.to_string(),
    }
}

struct Heading {
    level: usize,
    line: u32,
    name: String,
}

struct LineSpan {
    start_byte: usize,
    end_byte: usize,
}

pub fn extract(source: &str, rel_posix: &str) -> Extraction {
    let re = heading_re();
    let mut symbols = Vec::new();
    let mut headings = Vec::new();
    let mut lines = Vec::new();
    let mut offset = 0usize;

    for (i, line) in source.split('\n').enumerate() {
        let line_no = (i + 1) as u32;
        let line_start = offset;
        // Account for the '\n' that split consumed, except after the last segment.
        offset += line.len() + 1;
        let end_byte = line_start + line.len();
        lines.push(LineSpan {
            start_byte: line_start,
            end_byte,
        });

        let Some(caps) = re.captures(line) else {
            continue;
        };
        let name = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let level = caps.get(1).map(|m| m.as_str().len()).unwrap_or(1);
        headings.push(Heading {
            level,
            line: line_no,
            name: name.to_string(),
        });
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            kind: SymbolKind::Heading,
            start_line: line_no,
            end_line: line_no,
            start_byte: line_start as u32,
            end_byte: end_byte as u32,
            signature: None,
        });
    }

    let mut edges = Vec::new();
    if is_adr_path(rel_posix) {
        if let Some(decision) = decision_symbol(source, rel_posix, &headings, &lines) {
            edges.extend(supersedes_edges(source, &decision.name));
            symbols.push(decision);
        }
    }

    Extraction {
        status: ParseStatus::Outline,
        symbols,
        edges,
    }
}

fn decision_symbol(
    source: &str,
    rel_posix: &str,
    headings: &[Heading],
    lines: &[LineSpan],
) -> Option<ExtractedSymbol> {
    if source.is_empty() || lines.is_empty() {
        return None;
    }
    let nlines = lines.len() as u32;
    let name = headings
        .iter()
        .find(|h| h.level == 1)
        .map(|h| h.name.clone())
        .unwrap_or_else(|| filename_stem(rel_posix));
    if name.is_empty() {
        return None;
    }
    let (start_line, end_line) = decision_span(headings, nlines);
    if start_line < 1 || end_line < start_line || start_line > nlines {
        return None;
    }
    let end_line = end_line.min(nlines);
    let start_idx = (start_line as usize).saturating_sub(1);
    let end_idx = (end_line as usize).saturating_sub(1);
    Some(ExtractedSymbol {
        name,
        kind: SymbolKind::Decision,
        start_line,
        end_line,
        start_byte: lines[start_idx].start_byte as u32,
        end_byte: lines[end_idx].end_byte as u32,
        signature: None,
    })
}

fn decision_span(headings: &[Heading], nlines: u32) -> (u32, u32) {
    if let Some(d) = headings
        .iter()
        .find(|h| decision_heading_re().is_match(&h.name))
    {
        let end = headings
            .iter()
            .find(|h| h.line > d.line && h.level <= d.level)
            .map(|h| h.line - 1)
            .unwrap_or(nlines);
        return (d.line, end);
    }
    if let Some(h1) = headings.iter().find(|h| h.level == 1) {
        let before_next = headings
            .iter()
            .find(|h| h.line > h1.line && h.level == 1)
            .map(|h| h.line - 1)
            .unwrap_or(nlines);
        let capped = h1.line.saturating_add(MAX_DECISION_LINES - 1);
        return (h1.line, before_next.min(capped).min(nlines));
    }
    (1, nlines.min(MAX_DECISION_LINES))
}

fn link_target_path(raw: &str) -> String {
    let t = raw.trim();
    let t = t
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(t)
        .trim();
    let path = if let Some(rest) = t.strip_prefix('"') {
        rest.split_once('"').map(|(p, _)| p).unwrap_or(rest)
    } else if let Some(rest) = t.strip_prefix('\'') {
        rest.split_once('\'').map(|(p, _)| p).unwrap_or(rest)
    } else {
        t.split_whitespace().next().unwrap_or("")
    };
    path.trim().to_string()
}

fn supersedes_edges(source: &str, src_name: &str) -> Vec<ExtractedEdge> {
    let mut edges = Vec::new();
    for caps in supersedes_re().captures_iter(source) {
        let Some(num) = caps.get(1).map(|m| m.as_str()) else {
            continue;
        };
        if num.is_empty() {
            continue;
        }
        edges.push(ExtractedEdge {
            src_name: src_name.to_string(),
            dst_name: format!("ADR-{num}"),
            kind: EdgeKind::Supersedes,
            confidence: Confidence::Low,
        });
    }
    for caps in md_link_re().captures_iter(source) {
        let raw = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let target = link_target_path(raw);
        if target.is_empty() || !is_adr_path(&target) {
            continue;
        }
        let dst_name = filename_stem(&target);
        if dst_name.is_empty() {
            continue;
        }
        edges.push(ExtractedEdge {
            src_name: src_name.to_string(),
            dst_name,
            kind: EdgeKind::Supersedes,
            confidence: Confidence::Low,
        });
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, ParseStatus, SymbolKind};

    #[test]
    fn markdown_atx_headings() {
        let src = "# Title\n\n## WebSockets\n\ntext\n";
        let ext = extract(src, "README.md");
        assert_eq!(ext.status, ParseStatus::Outline);
        assert!(ext.edges.is_empty());
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "WebSockets" && s.kind == SymbolKind::Heading));
    }

    #[test]
    fn adr_path_detection() {
        assert!(is_adr_path("docs/adr/007-websockets.md"));
        assert!(is_adr_path("docs/decisions/use-ws.md"));
        assert!(is_adr_path("adr-007-foo.md"));
        assert!(!is_adr_path("README.md"));
        assert!(!is_adr_path("docs/superpowers/specs/foo.md"));
    }

    #[test]
    fn decision_section_span_and_supersedes() {
        let src = "# Use WebSockets\n\n## Context\n\npolling\n\n## Decision\n\nUse WS.\n\n## Consequences\n\nok\n\nSupersedes ADR-003\n";
        let ext = extract(src, "docs/adr/007-websockets.md");
        let d = ext
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Decision)
            .unwrap();
        assert_eq!(d.name, "Use WebSockets");
        assert_eq!(d.start_line, 7); // "## Decision"
        assert!(d.end_line >= d.start_line);
        assert!(ext
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Supersedes && e.dst_name == "ADR-003"));
    }

    #[test]
    fn readme_is_not_a_decision() {
        let ext = extract("# Title\n\n## WebSockets\n", "README.md");
        assert!(ext.symbols.iter().all(|s| s.kind != SymbolKind::Decision));
        assert!(ext.edges.is_empty());
    }
}
