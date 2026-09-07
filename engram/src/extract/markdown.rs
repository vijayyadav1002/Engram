use crate::types::{ExtractedSymbol, Extraction, ParseStatus, SymbolKind};
use regex::Regex;
use std::sync::OnceLock;

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").expect("heading regex"))
}

pub fn extract(source: &str) -> Extraction {
    let re = heading_re();
    let mut symbols = Vec::new();
    let mut offset = 0usize;

    for (i, line) in source.split('\n').enumerate() {
        let line_no = (i + 1) as u32;
        let line_start = offset;
        // Account for the '\n' that split consumed, except after the last segment.
        offset += line.len() + 1;

        let Some(caps) = re.captures(line) else {
            continue;
        };
        let name = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let end_byte = line_start + line.len();
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

    Extraction {
        status: ParseStatus::Outline,
        symbols,
        edges: vec![],
    }
}

#[cfg(test)]
mod tests {
    use crate::types::{ParseStatus, SymbolKind};

    #[test]
    fn markdown_atx_headings() {
        let src = "# Title\n\n## WebSockets\n\ntext\n";
        let ext = crate::extract::markdown::extract(src);
        assert_eq!(ext.status, ParseStatus::Outline);
        assert!(ext.edges.is_empty());
        assert!(ext
            .symbols
            .iter()
            .any(|s| s.name == "WebSockets" && s.kind == SymbolKind::Heading));
    }
}
