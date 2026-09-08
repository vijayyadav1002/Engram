# Engram Palace CLI Compat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make opt-in `get_context` attach MemPalace drawers against live 3.3.5 CLI stdout, and give slow searches 8s instead of 2.5s, without changing default Core behavior.

**Architecture:** Keep `CliPalaceSearch` spawning `mempalace search --results <n> <query>`. Change only `parse_search_output` so a hit body starts at the first non-metadata line after `[N] wing / room` (legacy `→` still stripped). Commit a 3.3.5 stdout fixture; CI never spawns a real palace. Bump `PALACE_TIMEOUT_MS` to 8000.

**Tech Stack:** Existing Engram crate (Rust, clap, serde_json). No new dependencies. No HTTP. No live MemPalace in default tests.

**Spec:** `docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md` (patch on `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md`)

## Global Constraints

- Default **off**. `get_context` without opt-in must match Core (no `palace` key in JSON).
- Palace failure or absence never fails `get_context`.
- Code items first; palace fills remaining budget only.
- Caps: `PALACE_MAX_HITS=3`, `PALACE_ITEM_MAX_CHARS=1200`, `PALACE_MIN_REMAINING=200`, `PALACE_TIMEOUT_MS=8000`, query argv max **250** characters.
- Drawers are verbatim (truncate with `…`, never summarize). No LLM, no network, no nested MCP client, no palace writes.
- Inject `PalaceSearch` in tests; do not call a real `mempalace` in default CI.
- Work in `engram/`. Run `cd engram && cargo test …`.
- TDD on every task.
- Do not add Git, embeddings, conversation mining, a Rust/Go extractor, a watcher, or MCP `index`.

## File map

| File | Responsibility |
|---|---|
| `engram/testdata/mempalace-search-3.3.5.txt` | Captured 3.3.5 CLI stdout (no `→`) |
| `engram/src/palace.rs` | `parse_search_output`, `PALACE_TIMEOUT_MS` |
| `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md` | Parse rules + timeout table |
| `docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md` | This slice’s spec (already written) |
| `README.md` | One sentence: 3.3.x CLI text is the parse target |

Do not change `compile.rs` `attach_palace` unless a test proves it is required. The compiler already maps empty parse → `unparseable`.

---

### Task 1: Parse MemPalace 3.3.5 stdout (keep legacy `→`)

**Files:**
- Create: `engram/testdata/mempalace-search-3.3.5.txt`
- Modify: `engram/src/palace.rs` (`parse_search_output`, tests module)

**Interfaces:**
- Consumes: existing `parse_search_output(stdout: &str) -> Vec<PalaceDrawer>`, `PalaceDrawer { wing, room, source, text }`, `parse_hit_header`, `is_rule_line`
- Produces: same signature; 3.3.5 fixture yields ≥2 drawers with non-empty `text`; legacy `parse_two_cli_hits` still passes

- [ ] **Step 1: Write the fixture**

Create `engram/testdata/mempalace-search-3.3.5.txt` with this exact content (live 3.3.5 shape: headers, `Source:`, `Match:`, indented body, **no** `→`, rule separators):

```text
============================================================
  Results for: "Engram get_context"
============================================================

  [1] sessions / technical
      Source: summary.json
      Match:  cosine=0.686  bm25=1.644

      "last_recap": "We added the README note that Engram does not store conversations."
      }

  ────────────────────────────────────────────────────────
  [2] sessions / technical
      Source: segment_000.md
      Match:  cosine=0.488  bm25=0.266

      ript / TSX extractor ## Task Description Read your task brief first.

  ────────────────────────────────────────────────────────
```

- [ ] **Step 2: Write the failing tests** in `engram/src/palace.rs` tests module, next to `parse_two_cli_hits`.

Do **not** change `parse_two_cli_hits`. Add:

```rust
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
    }

    #[test]
    fn parse_skips_match_metadata_line() {
        let out = "  [1] w / r\n      Match:  cosine=0.1  bm25=0.0\n      body only\n";
        let hits = parse_search_output(out);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "body only");
        assert!(!hits[0].text.contains("cosine"));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run:

```bash
cd engram && cargo test --lib parse_mempalace_3_3_5_fixture_without_arrow parse_skips_match_metadata_line -- --exact
```

Expected: FAIL because `parse_search_output` returns 0 hits (no `→`).

- [ ] **Step 4: Implement `parse_search_output`**

Replace `parse_search_output` in `engram/src/palace.rs` with:

```rust
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
                });
            }
            continue;
        }
        i += 1;
    }
    hits
}
```

Leave `parse_hit_header` and `is_rule_line` unchanged.

- [ ] **Step 5: Run tests to verify they pass**

Run:

```bash
cd engram && cargo test --lib parse_mempalace_3_3_5_fixture_without_arrow parse_skips_match_metadata_line parse_two_cli_hits parse_empty_is_empty_vec -- --exact
```

Expected: PASS (3.3.5 fixture, metadata skip, legacy arrow, empty garbage).

Then:

```bash
cd engram && cargo test --lib
```

Expected: PASS (whole lib, including `cli_empty_parse_is_unparseable`).

- [ ] **Step 6: Commit**

```bash
git add engram/testdata/mempalace-search-3.3.5.txt engram/src/palace.rs
git commit -m "$(cat <<'EOF'
fix: parse MemPalace 3.3.5 search stdout without arrow markers

The live CLI prints an indented body after Source/Match. Requiring `→`
made every opt-in get_context return unparseable. Keep the legacy arrow
fixture working.
EOF
)"
```

---

### Task 2: Raise palace search timeout to 8000ms

**Files:**
- Modify: `engram/src/palace.rs` (`PALACE_TIMEOUT_MS` and one assertion)

**Interfaces:**
- Consumes: `pub const PALACE_TIMEOUT_MS: u64` used by `compile.rs` when building `CliPalaceSearch { timeout_ms: PALACE_TIMEOUT_MS }`
- Produces: `PALACE_TIMEOUT_MS == 8000`; child-kill test still uses an explicit 200ms and must keep passing

- [ ] **Step 1: Write the failing test** in `engram/src/palace.rs` tests:

```rust
    #[test]
    fn palace_timeout_ms_is_eight_seconds() {
        assert_eq!(PALACE_TIMEOUT_MS, 8000);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cd engram && cargo test --lib palace_timeout_ms_is_eight_seconds -- --exact
```

Expected: FAIL with `left: 2500, right: 8000`.

- [ ] **Step 3: Change the constant**

In `engram/src/palace.rs`:

```rust
pub const PALACE_TIMEOUT_MS: u64 = 8000;
```

Do not change `cli_timeout_kills_child` (`timeout_ms: 200`).

- [ ] **Step 4: Run tests**

Run:

```bash
cd engram && cargo test --lib palace_timeout_ms_is_eight_seconds cli_timeout_kills_child -- --exact
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engram/src/palace.rs
git commit -m "$(cat <<'EOF'
fix: allow 8s for mempalace search before timeout

2.5s raced a ~2s local search and dropped drawers. Child-kill tests
still use an explicit short timeout.
EOF
)"
```

---

### Task 3: Align specs and README with the live CLI

**Files:**
- Modify: `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md`
- Modify: `README.md` (palace subsection)
- Modify: `docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md` (status line only)
- Modify: `docs/superpowers/specs/2026-09-07-engram-core-design.md` (status line only)

**Interfaces:**
- Consumes: parser rules and timeout from the compat spec
- Produces: bridge spec §7 matches 3.3.x text; README tells operators what CLI shape Engram parses

- [ ] **Step 1: Patch the bridge spec parse + timeout**

In `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md`:

1. Change the header `Status:` from `draft, pending user review` to `implemented (parser follow-up in 2026-09-08-engram-palace-cli-compat-design.md)`.
2. Replace the **Parse:** paragraph in §7 with:

```markdown
**Parse:** MemPalace 3.3.x CLI is human text, not JSON. The adapter extracts each `[N] wing / room` block. `Source:` sets `source`. `Match:` is skipped. The body is every following non-empty line until the next header or a `─` rule line. A leading `→` on a body line is stripped if present (legacy). If parse yields nothing, treat as `Unparseable` and skip (do not fail `get_context`).

If a future MemPalace `--json` flag exists, prefer that in a follow-up; do not block on it.
```

3. In the caps table, set `PALACE_TIMEOUT_MS` to `8000`.

- [ ] **Step 2: Patch Core spec status**

In `docs/superpowers/specs/2026-09-07-engram-core-design.md`, change:

```markdown
Status: draft, pending user review
```

to:

```markdown
Status: implemented on main
```

Do not rewrite Core non-goals.

- [ ] **Step 3: Patch the compat spec status**

In `docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md`, after Tasks 1–2 land, set:

```markdown
Status: implemented on main
```

- [ ] **Step 4: README palace note**

In `README.md`, in the “Optional: attach drawers on `get_context`” section, after the sentence about palace absence/timeout/parse failure, add:

```markdown
Engram parses MemPalace **3.3.x** `search` CLI text (`[N] wing / room`, then `Source:` / `Match:`, then the indented body). There is no `search --json` flag in that release. If `stats.palace.status` is `unparseable`, the CLI format changed — file a fixture, do not grep the tree as a substitute for `get_context`.
```

- [ ] **Step 5: Run the suite (docs-only change must not break tests)**

Run:

```bash
cd engram && cargo test
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add \
  docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md \
  docs/superpowers/specs/2026-09-08-engram-palace-cli-compat-design.md \
  docs/superpowers/specs/2026-09-07-engram-core-design.md \
  README.md
git commit -m "$(cat <<'EOF'
docs: record 3.3.x palace CLI parse rules and 8s timeout

Align the bridge spec and README with the live MemPalace search
format. Mark Core as implemented on main.
EOF
)"
```

---

## Out of this plan

Do not implement these here. They are real follow-ons, not part of the broken tool:

| Item | Why later |
|---|---|
| Rust / Go symbol graph | New extractors; Core language policy. Own spec. |
| Watcher / MCP `index` | Core: MCP is read-only; no daemon. |
| Git + structured decisions | Named Core follow-on slice. |
| Embeddings / rerank | Named Core follow-on slice. |
| Token-savings A/B vs grep | Harness measurement, not a compiler bug. |
| Nested MemPalace MCP | Bridge non-goal; CLI spawn stays. |

## Self-review

1. **Spec coverage:** Parser rules §5 → Task 1. Timeout §6 → Task 2. Fixture §7 → Task 1 Step 1. Success §8 docs → Task 3. Non-goals §4 → Out of this plan.
2. **Placeholders:** none. Tests and replacement function are inlined.
3. **Types:** `PalaceDrawer`, `parse_search_output`, `PALACE_TIMEOUT_MS` match existing crate names.
