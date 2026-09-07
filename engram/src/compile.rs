use crate::error::Error;
use crate::hash::blake3_file;
use crate::store::{FtsHit, NeighborHit, Store, SymbolHit};
use crate::types::{
    Confidence, ContextEdge, ContextItem, ContextPackage, ContextStats, EdgeKind, SymbolKind,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

pub const CAP_SYMBOLS: usize = 50;
pub const CAP_FTS: usize = 30;
pub const CAP_NEIGHBORS: usize = 40;
pub const DEFAULT_BUDGET: u32 = 3000;
pub const MAX_JSON_BYTES: usize = 16_384;

const FTS_SPAN_LINES: u32 = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlan {
    pub symbol_terms: Vec<String>,
    pub fts_query: String,
    pub path_hints: Vec<String>,
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "in", "for", "of", "where", "how", "what",
];

/// Deterministic query plan: quoted strings and CamelCase/`snake_case` tokens
/// become symbol terms; path-like tokens become hints; the rest is FTS.
pub fn plan_query(query: &str) -> QueryPlan {
    let mut symbol_terms = Vec::new();
    let mut path_hints = Vec::new();
    let mut fts_parts = Vec::new();

    let (quoted, rest) = extract_quoted(query);
    for q in quoted {
        push_unique(&mut symbol_terms, q);
    }

    for raw in rest.split_whitespace() {
        let token = trim_token(raw);
        if token.is_empty() {
            continue;
        }
        if is_path_hint(&token) {
            push_unique(&mut path_hints, token);
        } else if is_symbol_term(&token) {
            push_unique(&mut symbol_terms, token);
        } else if !is_stopword(&token) {
            fts_parts.push(token);
        }
    }

    QueryPlan {
        symbol_terms,
        fts_query: fts_parts.join(" "),
        path_hints,
    }
}

pub fn search_symbols(root: &Path, name: &str, limit: usize) -> Result<Vec<SymbolHit>, Error> {
    let store = Store::open_read(&root.join(".engram/index.sqlite"))?;
    Ok(collect_symbols_tagged(&store, &[name.to_string()], limit)?
        .into_iter()
        .map(|(h, _)| h)
        .collect())
}

pub fn search_code(root: &Path, query: &str, limit: usize) -> Result<Vec<FtsHit>, Error> {
    let store = Store::open_read(&root.join(".engram/index.sqlite"))?;
    fts_try(&store, query, limit)
}

pub fn get_context(root: &Path, query: &str, budget_tokens: u32) -> Result<ContextPackage, Error> {
    let store = Store::open_read(&root.join(".engram/index.sqlite"))?;
    let plan = plan_query(query);

    let tagged = collect_symbols_tagged(&store, &plan.symbol_terms, CAP_SYMBOLS)?;
    let symbol_hits: Vec<SymbolHit> = tagged.iter().map(|(h, _)| h.clone()).collect();
    let mut by_id: HashMap<i64, SymbolHit> = HashMap::new();
    for h in &symbol_hits {
        by_id.insert(h.id, h.clone());
    }
    let accepted_ids: HashSet<i64> = by_id.keys().copied().collect();

    let fts_hits = collect_fts(&store, &plan)?;
    let (neighbor_cands, neighbor_edges) = collect_neighbors(&store, &symbol_hits, &mut by_id)?;

    let mut terms_for_heading = plan.symbol_terms.clone();
    for w in plan.fts_query.split_whitespace() {
        push_unique(&mut terms_for_heading, w.to_string());
    }

    let mut spans: Vec<SpanCand> = Vec::new();
    for (h, exact) in &tagged {
        spans.push(span_from_symbol(h, why_for_symbol(h, *exact)));
    }
    for (h, n) in &neighbor_cands {
        let mut why = BTreeSet::new();
        why.insert(neighbor_why(n.kind).to_string());
        if h.kind == SymbolKind::Heading {
            why.insert("heading".into());
        }
        if h.kind == SymbolKind::Selector {
            why.insert("selector".into());
        }
        let mut span = span_from_symbol(h, why);
        span.neighbor_high = n.confidence == Confidence::High;
        span.neighbor_low = n.confidence == Confidence::Low;
        spans.push(span);
    }

    for (idx, hit) in fts_hits.iter().enumerate() {
        let fts_norm = 1.0 / (1.0 + idx as f64);
        if let Some(heading) = heading_in_file(&store, &hit.path, &terms_for_heading, &symbol_hits)?
        {
            let mut why = BTreeSet::new();
            why.insert("heading".into());
            why.insert("fts".into());
            let mut span = span_from_symbol(&heading, why);
            span.fts_norm = fts_norm;
            spans.push(span);
        } else {
            let mut why = BTreeSet::new();
            why.insert("fts".into());
            spans.push(SpanCand {
                path: hit.path.clone(),
                start_line: 1,
                end_line: FTS_SPAN_LINES,
                symbol: None,
                kind: None,
                why,
                fts_norm,
                neighbor_high: false,
                neighbor_low: false,
                score: 0.0,
            });
        }
    }

    let mut fused = fuse_spans(spans);
    let top_files: HashSet<String> = fused
        .iter()
        .filter(|s| s.why.contains("exact_symbol"))
        .map(|s| s.path.clone())
        .collect();
    for s in &mut fused {
        rescore(s, &plan.path_hints, &top_files);
    }

    let deduped = dedupe_spans(fused);
    let mut scored = deduped;
    for s in &mut scored {
        rescore(s, &plan.path_hints, &top_files);
    }
    scored.sort_by(|a, b| cmp_score_desc(a, b));

    let files_considered = scored
        .iter()
        .map(|s| s.path.as_str())
        .collect::<HashSet<_>>()
        .len() as u32;
    let symbols_considered = accepted_ids.len() as u32 + neighbor_cands.len() as u32;

    let mut first_pass = Vec::new();
    let mut overflow = Vec::new();
    let mut per_path: HashMap<String, usize> = HashMap::new();
    for s in scored {
        let n = per_path.entry(s.path.clone()).or_insert(0);
        if *n < 2 {
            *n += 1;
            first_pass.push(s);
        } else {
            overflow.push(s);
        }
    }

    let mut items = Vec::new();
    let mut used_tokens = 0u32;
    let mut dropped_for_budget = 0u32;
    let mut stale_omitted = 0u32;
    let mut fresh_cache: HashMap<String, bool> = HashMap::new();

    let stopped = fill_items(
        &first_pass,
        root,
        &store,
        budget_tokens,
        &mut items,
        &mut used_tokens,
        &mut dropped_for_budget,
        &mut stale_omitted,
        &mut fresh_cache,
    )?;
    if !stopped && used_tokens < budget_tokens {
        fill_items(
            &overflow,
            root,
            &store,
            budget_tokens,
            &mut items,
            &mut used_tokens,
            &mut dropped_for_budget,
            &mut stale_omitted,
            &mut fresh_cache,
        )?;
    }

    let mut pkg = ContextPackage {
        query: query.to_string(),
        budget_tokens,
        used_tokens,
        items,
        edges: Vec::new(),
        stats: ContextStats {
            files_considered,
            symbols_considered,
            dropped_for_budget,
            stale_omitted,
            stale_index: false,
            truncated: false,
        },
    };
    pkg.edges = package_edges(&pkg.items, &neighbor_edges);
    pkg.stats.stale_index = stale_index_flag(pkg.stats.stale_omitted, pkg.items.len());

    enforce_json_cap(&mut pkg, &neighbor_edges);
    Ok(pkg)
}

#[derive(Debug, Clone)]
struct SpanCand {
    path: String,
    start_line: u32,
    end_line: u32,
    symbol: Option<String>,
    kind: Option<String>,
    why: BTreeSet<String>,
    fts_norm: f64,
    neighbor_high: bool,
    neighbor_low: bool,
    score: f64,
}

fn extract_quoted(query: &str) -> (Vec<String>, String) {
    let mut quoted = Vec::new();
    let mut rest = String::with_capacity(query.len());
    let mut chars = query.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut inner = String::new();
            let mut closed = false;
            while let Some(ch) = chars.next() {
                if ch == '"' {
                    closed = true;
                    break;
                }
                inner.push(ch);
            }
            if closed {
                let t = inner.trim();
                if !t.is_empty() {
                    quoted.push(t.to_string());
                }
                rest.push(' ');
            } else {
                rest.push('"');
                rest.push_str(&inner);
            }
        } else {
            rest.push(c);
        }
    }
    (quoted, rest)
}

fn trim_token(raw: &str) -> String {
    raw.trim_matches(|c: char| {
        matches!(
            c,
            '?' | '!' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"'
        )
    })
    .to_string()
}

fn is_path_hint(token: &str) -> bool {
    if token.contains('/') {
        return true;
    }
    let mut chars = token.chars();
    matches!(chars.next(), Some('.')) && matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
}

fn is_symbol_term(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let dotted: Vec<&str> = token.split('.').collect();
    if dotted.len() > 1 && dotted.iter().all(|p| is_ident(p)) {
        return true;
    }
    is_ident(token) && token.chars().any(|c| c.is_ascii_uppercase() || c == '_')
}

fn is_ident(token: &str) -> bool {
    let mut chars = token.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn is_stopword(token: &str) -> bool {
    STOPWORDS.iter().any(|w| token.eq_ignore_ascii_case(w))
}

fn push_unique(out: &mut Vec<String>, item: String) {
    if !out.iter().any(|e| e == &item) {
        out.push(item);
    }
}

fn collect_fts(store: &Store, plan: &QueryPlan) -> Result<Vec<FtsHit>, Error> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if !plan.fts_query.is_empty() {
        for h in fts_try(store, &plan.fts_query, CAP_FTS)? {
            if seen.insert(h.path.clone()) {
                out.push(h);
                if out.len() >= CAP_FTS {
                    return Ok(out);
                }
            }
        }
    }
    for term in &plan.symbol_terms {
        if out.len() >= CAP_FTS {
            break;
        }
        let remain = CAP_FTS - out.len();
        for h in fts_try(store, term, remain)? {
            if seen.insert(h.path.clone()) {
                out.push(h);
                if out.len() >= CAP_FTS {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

fn fts_try(store: &Store, query: &str, limit: usize) -> Result<Vec<FtsHit>, Error> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    match store.fts_search(query, limit) {
        Ok(hits) => Ok(hits),
        Err(_) => {
            let quoted = format!("\"{}\"", query.replace('"', ""));
            match store.fts_search(&quoted, limit) {
                Ok(hits) => Ok(hits),
                Err(_) => {
                    let tokens: Vec<&str> = query
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .filter(|t| !t.is_empty())
                        .collect();
                    if tokens.is_empty() {
                        Ok(vec![])
                    } else {
                        match store.fts_search(&tokens.join(" "), limit) {
                            Ok(hits) => Ok(hits),
                            Err(_) => Ok(vec![]),
                        }
                    }
                }
            }
        }
    }
}

fn collect_neighbors(
    store: &Store,
    accepted: &[SymbolHit],
    by_id: &mut HashMap<i64, SymbolHit>,
) -> Result<(Vec<(SymbolHit, NeighborHit)>, Vec<NeighborHit>), Error> {
    let mut edges = Vec::new();
    let mut high: Vec<NeighborHit> = Vec::new();
    let mut low: Vec<NeighborHit> = Vec::new();
    let accepted_ids: HashSet<i64> = accepted.iter().map(|h| h.id).collect();

    for h in accepted {
        for n in store.neighbors(h.id, CAP_NEIGHBORS)? {
            edges.push(n.clone());
            match n.confidence {
                Confidence::High => high.push(n),
                Confidence::Low => low.push(n),
            }
        }
    }

    let mut cands = Vec::new();
    let mut taken = HashSet::new();
    for n in high.into_iter().chain(low) {
        if taken.len() >= CAP_NEIGHBORS {
            break;
        }
        let other_id = if accepted_ids.contains(&n.src_id) && !accepted_ids.contains(&n.dst_id) {
            n.dst_id
        } else if accepted_ids.contains(&n.dst_id) && !accepted_ids.contains(&n.src_id) {
            n.src_id
        } else {
            continue;
        };
        if !taken.insert(other_id) {
            continue;
        }
        let hit = match resolve_neighbor(store, by_id, other_id, &n)? {
            Some(h) => h,
            None => {
                taken.remove(&other_id);
                continue;
            }
        };
        by_id.insert(hit.id, hit.clone());
        cands.push((hit, n));
    }
    Ok((cands, edges))
}

fn resolve_neighbor(
    store: &Store,
    by_id: &HashMap<i64, SymbolHit>,
    other_id: i64,
    n: &NeighborHit,
) -> Result<Option<SymbolHit>, Error> {
    if let Some(h) = by_id.get(&other_id) {
        return Ok(Some(h.clone()));
    }
    let (name, path) = if n.src_id == other_id {
        (n.src_name.as_str(), n.src_path.as_str())
    } else {
        (n.dst_name.as_str(), n.dst_path.as_str())
    };
    Ok(store
        .lookup_symbols_exact(name, CAP_SYMBOLS)?
        .into_iter()
        .find(|h| h.id == other_id && h.path == path))
}

fn neighbor_why(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Import => "import_neighbor",
        EdgeKind::Call => "call_neighbor",
    }
}

fn why_for_symbol(h: &SymbolHit, exact_first: bool) -> BTreeSet<String> {
    let mut why = BTreeSet::new();
    if exact_first {
        why.insert("exact_symbol".into());
    } else {
        why.insert("prefix_symbol".into());
    }
    if h.kind == SymbolKind::Heading {
        why.insert("heading".into());
    }
    if h.kind == SymbolKind::Selector {
        why.insert("selector".into());
    }
    why
}

fn span_from_symbol(h: &SymbolHit, why: BTreeSet<String>) -> SpanCand {
    SpanCand {
        path: h.path.clone(),
        start_line: h.start_line,
        end_line: h.end_line.max(h.start_line),
        symbol: Some(h.name.clone()),
        kind: Some(h.kind.as_str().to_string()),
        why,
        fts_norm: 0.0,
        neighbor_high: false,
        neighbor_low: false,
        score: 0.0,
    }
}

fn heading_in_file(
    store: &Store,
    path: &str,
    terms: &[String],
    known: &[SymbolHit],
) -> Result<Option<SymbolHit>, Error> {
    for h in known {
        if h.path == path && h.kind == SymbolKind::Heading && name_matches(&h.name, terms) {
            return Ok(Some(h.clone()));
        }
    }
    for term in terms {
        for h in store.lookup_symbols_exact(term, CAP_SYMBOLS)? {
            if h.path == path && h.kind == SymbolKind::Heading {
                return Ok(Some(h));
            }
        }
    }
    Ok(None)
}

fn name_matches(name: &str, terms: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    terms
        .iter()
        .any(|t| name.eq_ignore_ascii_case(t) || lower.contains(&t.to_ascii_lowercase()))
}

fn fuse_spans(spans: Vec<SpanCand>) -> Vec<SpanCand> {
    let mut map: HashMap<(String, u32, u32), SpanCand> = HashMap::new();
    for s in spans {
        let key = (s.path.clone(), s.start_line, s.end_line);
        if let Some(dst) = map.get_mut(&key) {
            fuse_into(dst, s);
        } else {
            map.insert(key, s);
        }
    }
    map.into_values().collect()
}

fn fuse_into(dst: &mut SpanCand, src: SpanCand) {
    dst.why.extend(src.why);
    if dst.symbol.is_none() {
        dst.symbol = src.symbol;
        dst.kind = src.kind;
    }
    dst.fts_norm = dst.fts_norm.max(src.fts_norm);
    dst.neighbor_high |= src.neighbor_high;
    dst.neighbor_low |= src.neighbor_low;
}

fn rescore(span: &mut SpanCand, path_hints: &[String], top_files: &HashSet<String>) {
    if path_hints.iter().any(|h| span.path.contains(h.as_str())) {
        span.why.insert("path_hint".into());
    }
    let mut score = 0.0;
    if span.why.contains("exact_symbol") {
        score += 5.0;
    }
    if span.why.contains("prefix_symbol") {
        score += 3.0;
    }
    if span.why.contains("fts") {
        score += 2.0 * span.fts_norm;
    }
    if span.why.contains("heading") || span.why.contains("selector") {
        score += 2.0;
    }
    if span.neighbor_high {
        score += 1.5;
    } else if span.neighbor_low {
        score += 0.5;
    }
    if path_hints.iter().any(|h| span.path.contains(h.as_str())) {
        score += 1.0;
    }
    if top_files.contains(&span.path) {
        score += 1.0;
    }
    span.score = score;
}

fn dedupe_spans(spans: Vec<SpanCand>) -> Vec<SpanCand> {
    let mut by_path: HashMap<String, Vec<SpanCand>> = HashMap::new();
    for s in spans {
        by_path.entry(s.path.clone()).or_default().push(s);
    }
    let mut out = Vec::new();
    for (_, group) in by_path {
        out.extend(merge_path_spans(group));
    }
    out
}

fn merge_path_spans(mut spans: Vec<SpanCand>) -> Vec<SpanCand> {
    let symbols: Vec<SpanCand> = spans
        .iter()
        .filter(|s| s.symbol.is_some())
        .cloned()
        .collect();
    spans.retain(|s| {
        if s.symbol.is_some() || !s.why.contains("fts") {
            return true;
        }
        !symbols
            .iter()
            .any(|sym| sym.start_line >= s.start_line && sym.end_line <= s.end_line)
    });
    spans.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then(a.end_line.cmp(&b.end_line))
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    let mut out: Vec<SpanCand> = Vec::new();
    for s in spans {
        if let Some(last) = out.last_mut() {
            if s.start_line <= last.end_line {
                last.start_line = last.start_line.min(s.start_line);
                last.end_line = last.end_line.max(s.end_line);
                fuse_into(last, s);
                continue;
            }
        }
        out.push(s);
    }
    out
}

fn cmp_score_desc(a: &SpanCand, b: &SpanCand) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.path.cmp(&b.path))
        .then_with(|| a.start_line.cmp(&b.start_line))
}

fn fill_items(
    spans: &[SpanCand],
    root: &Path,
    store: &Store,
    budget_tokens: u32,
    items: &mut Vec<ContextItem>,
    used_tokens: &mut u32,
    dropped_for_budget: &mut u32,
    stale_omitted: &mut u32,
    fresh_cache: &mut HashMap<String, bool>,
) -> Result<bool, Error> {
    for s in spans {
        if !file_is_fresh(store, root, &s.path, fresh_cache)? {
            *stale_omitted += 1;
            continue;
        }
        let abs = root.join(&s.path);
        let text = match read_span(&abs, s.start_line, s.end_line) {
            Ok(t) if !t.is_empty() => t,
            Ok(_) => continue,
            Err(_) => {
                *stale_omitted += 1;
                continue;
            }
        };
        let cost = token_cost(&text);
        if *used_tokens + cost > budget_tokens {
            *dropped_for_budget += 1;
            return Ok(true);
        }
        *used_tokens += cost;
        items.push(ContextItem {
            path: s.path.clone(),
            start_line: s.start_line,
            end_line: s
                .end_line
                .min(s.start_line.max(1) + text.lines().count() as u32 - 1),
            symbol: s.symbol.clone(),
            kind: s.kind.clone(),
            text,
            why: s.why.iter().cloned().collect(),
        });
    }
    Ok(false)
}

fn file_is_fresh(
    store: &Store,
    root: &Path,
    path: &str,
    cache: &mut HashMap<String, bool>,
) -> Result<bool, Error> {
    if let Some(&fresh) = cache.get(path) {
        return Ok(fresh);
    }
    let Some(row) = store.get_file(path)? else {
        cache.insert(path.to_string(), false);
        return Ok(false);
    };
    let disk = match blake3_file(&root.join(path)) {
        Ok(h) => h,
        Err(_) => {
            cache.insert(path.to_string(), false);
            return Ok(false);
        }
    };
    let fresh = disk == row.hash;
    cache.insert(path.to_string(), fresh);
    Ok(fresh)
}

fn read_span(path: &Path, start_line: u32, end_line: u32) -> Result<String, Error> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Ok(String::new());
    }
    let start = (start_line.max(1) as usize - 1).min(lines.len());
    let end = (end_line as usize).clamp(start, lines.len());
    Ok(lines[start..end].join("\n"))
}

fn token_cost(text: &str) -> u32 {
    text.split_whitespace().count() as u32 + 2
}

fn package_edges(items: &[ContextItem], neighbors: &[NeighborHit]) -> Vec<ContextEdge> {
    let present: HashSet<String> = items
        .iter()
        .filter_map(|i| i.symbol.as_ref().map(|s| format!("{}:{}", i.path, s)))
        .collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for n in neighbors {
        let from = format!("{}:{}", n.src_path, n.src_name);
        let to = format!("{}:{}", n.dst_path, n.dst_name);
        if !present.contains(&from) || !present.contains(&to) {
            continue;
        }
        let key = (from.clone(), to.clone(), n.kind.as_str().to_string());
        if !seen.insert(key) {
            continue;
        }
        out.push(ContextEdge {
            from,
            to,
            kind: n.kind.as_str().to_string(),
            confidence: n.confidence.as_str().to_string(),
        });
    }
    out
}

fn stale_index_flag(stale_omitted: u32, items_len: usize) -> bool {
    stale_omitted * 2 > (items_len as u32 + stale_omitted)
}

fn enforce_json_cap(pkg: &mut ContextPackage, neighbors: &[NeighborHit]) {
    loop {
        let Ok(bytes) = serde_json::to_vec(pkg) else {
            break;
        };
        if bytes.len() <= MAX_JSON_BYTES {
            break;
        }
        if pkg.items.pop().is_none() {
            pkg.stats.truncated = true;
            break;
        }
        pkg.stats.truncated = true;
        pkg.used_tokens = pkg.items.iter().map(|i| token_cost(&i.text)).sum();
        pkg.edges = package_edges(&pkg.items, neighbors);
        pkg.stats.stale_index = stale_index_flag(pkg.stats.stale_omitted, pkg.items.len());
    }
}

fn collect_symbols_tagged(
    store: &Store,
    terms: &[String],
    cap: usize,
) -> Result<Vec<(SymbolHit, bool)>, Error> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if cap == 0 {
        return Ok(out);
    }
    for term in terms {
        if out.len() >= cap {
            break;
        }
        let remain = cap - out.len();
        for h in store.lookup_symbols_exact(term, remain)? {
            if seen.insert(h.id) {
                out.push((h, true));
                if out.len() >= cap {
                    return Ok(out);
                }
            }
        }
        let remain = cap - out.len();
        for h in store.lookup_symbols_prefix(term, remain)? {
            if seen.insert(h.id) {
                out.push((h, false));
                if out.len() >= cap {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

#[test]
fn plan_extracts_quotes_camel_paths() {
    let p = plan_query(r#"where is "createSession" in src/auth for LoginBanner?"#);
    assert!(p.symbol_terms.iter().any(|t| t == "createSession"));
    assert!(p.symbol_terms.iter().any(|t| t == "LoginBanner"));
    assert!(p.path_hints.iter().any(|h| h.contains("src/auth")));
}
