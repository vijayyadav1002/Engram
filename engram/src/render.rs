use crate::types::ContextPackage;

/// Extractive text digest of a context package (path, line range, fenced source).
pub fn render_digest(pkg: &ContextPackage) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&pkg.query);
    out.push('\n');
    out.push_str(&format!(
        "budget {} used {}\n",
        pkg.budget_tokens, pkg.used_tokens
    ));

    for item in &pkg.items {
        out.push('\n');
        out.push_str("## ");
        out.push_str(&item.path);
        out.push(':');
        out.push_str(&item.start_line.to_string());
        out.push('-');
        out.push_str(&item.end_line.to_string());
        if let Some(sym) = &item.symbol {
            out.push_str(" (");
            out.push_str(sym);
            out.push(')');
        }
        if !item.why.is_empty() {
            out.push_str(" [");
            out.push_str(&item.why.join(", "));
            out.push(']');
        }
        out.push('\n');
        out.push_str("```\n");
        out.push_str(&item.text);
        if !item.text.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::types::{ContextItem, ContextPackage, ContextStats};

    fn sample_package() -> ContextPackage {
        ContextPackage {
            query: "q".into(),
            budget_tokens: 3000,
            used_tokens: 12,
            items: vec![ContextItem {
                path: "a.ts".into(),
                start_line: 1,
                end_line: 2,
                symbol: Some("foo".into()),
                kind: Some("function".into()),
                text: "fn foo() {}".into(),
                why: vec!["exact_symbol".into()],
            }],
            edges: vec![],
            stats: ContextStats {
                files_considered: 1,
                symbols_considered: 1,
                dropped_for_budget: 0,
                stale_omitted: 0,
                stale_index: false,
                truncated: false,
            },
        }
    }

    #[test]
    fn digest_is_extractive() {
        let pkg = sample_package();
        let t = crate::render::render_digest(&pkg);
        assert!(t.contains("a.ts"));
        assert!(t.contains("```"));
        assert!(t.contains("fn foo"));
    }
}
