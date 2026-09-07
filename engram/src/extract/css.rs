use crate::types::{ExtractedSymbol, Extraction, ParseStatus, SymbolKind};
use regex::Regex;
use std::sync::OnceLock;

fn class_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.([A-Za-z_][\w-]*)").expect("css class regex"))
}

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"#([A-Za-z_][\w-]*)").expect("css id regex"))
}

fn custom_prop_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"--([A-Za-z_][\w-]*)").expect("css custom prop regex"))
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)url\([^)]*\)").expect("css url regex"))
}

pub fn extract(source: &str) -> Extraction {
    let mut symbols = Vec::new();
    let url_spans: Vec<(usize, usize)> = url_re()
        .find_iter(source)
        .map(|m| (m.start(), m.end()))
        .collect();

    let in_url = |start: usize, end: usize| -> bool {
        url_spans
            .iter()
            .any(|&(us, ue)| start >= us && end <= ue)
    };

    let line_of = |byte: usize| -> u32 {
        (source[..byte].bytes().filter(|&b| b == b'\n').count() + 1) as u32
    };

    let mut push = |name: String, start: usize, end: usize| {
        if in_url(start, end) {
            return;
        }
        let line = line_of(start);
        symbols.push(ExtractedSymbol {
            name,
            kind: SymbolKind::Selector,
            start_line: line,
            end_line: line,
            start_byte: start as u32,
            end_byte: end as u32,
            signature: None,
        });
    };

    for m in class_re().find_iter(source) {
        push(m.as_str().to_string(), m.start(), m.end());
    }
    for m in id_re().find_iter(source) {
        push(m.as_str().to_string(), m.start(), m.end());
    }
    for m in custom_prop_re().find_iter(source) {
        push(m.as_str().to_string(), m.start(), m.end());
    }

    Extraction {
        status: ParseStatus::Outline,
        symbols,
        edges: vec![],
    }
}

#[cfg(test)]
mod tests {
    use crate::types::ParseStatus;

    #[test]
    fn css_class_id_custom_prop() {
        let src = ".auth-panel { color: red; }\n#root { }\n:root { --brand: #00f; }\n";
        let ext = crate::extract::css::extract(src);
        assert_eq!(ext.status, ParseStatus::Outline);
        assert!(ext.edges.is_empty());
        let names: Vec<_> = ext.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&".auth-panel") || names.contains(&"auth-panel"));
        assert!(names.contains(&"#root") || names.contains(&"root"));
        assert!(names.iter().any(|n| n.contains("--brand") || *n == "brand"));
    }
}
