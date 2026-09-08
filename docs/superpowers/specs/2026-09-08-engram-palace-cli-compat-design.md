# Engram Palace CLI Compat Design

Date: 2026-09-08
Status: draft, pending user review
Scope: make the existing MemPalace bridge parse live CLI output and survive slow searches
Depends on: `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md`

## 1. Executive summary

The MemPalace bridge is shipped: `get_context` can attach up to three drawers when the caller opts in. Against **MemPalace 3.3.5** that path is dead. The parser requires a `→` body marker that the live CLI does not print, so `stats.palace.status` is `unparseable` and no drawers are attached. The 2500ms timeout is also tight (a local search here took ~1.9s).

This slice fixes the parser against a committed 3.3.5 fixture, keeps the old `→` fixture working, and raises the timeout. It does not add a Rust/Go graph, a watcher, Git intelligence, embeddings, or a nested MemPalace MCP client.

## 2. Problem

`parse_search_output` in `engram/src/palace.rs` only emits a drawer when it sees a line whose trim starts with `→`. The unit test `parse_two_cli_hits` invents that marker.

Live `mempalace search --results 3 "<query>"` (3.3.5) looks like:

```text
============================================================
  Results for: "Engram get_context"
============================================================

  [1] sessions / technical
      Source: summary.json
      Match:  cosine=0.686  bm25=1.644

      "last_recap": "We added the README note…"
      }

  ────────────────────────────────────────────────────────
  [2] sessions / technical
      Source: segment_000.md
      Match:  cosine=0.488  bm25=0.266

      ript / TSX extractor ## Task Description …
```

Headers `[N] wing / room` already parse. `Source:` already parses. The body is indented prose (or a JSON fragment) with **no** `→`. Result: zero hits → `PalaceError::Unparseable` → code package only.

## 3. Goals

- Opt-in `get_context` against MemPalace 3.3.5 attaches drawers (`status=ok`, `attempted>0`) when stdout matches the committed fixture.
- The existing `→` fixture still parses (do not break the tests that already pass).
- Empty / garbage stdout is still `unparseable`; missing binary is still `not_installed`; `get_context` still never fails because of palace.
- Slow searches have headroom: default timeout **8000ms**.
- CI still does not spawn a real `mempalace`.

## 4. Non-goals

- Rust, Go, or other new graph extractors
- File watcher or MCP `index`
- Git history / structured decisions
- Embeddings or LLM rerank
- `mempalace search --json` (the 3.3.5 CLI has no such flag)
- Nested MemPalace MCP client
- Measuring token-savings percentages vs a grepping agent
- Ticking historical plan checkboxes from Core / bridge implementation

Those stay later slices or docs-only follow-ups.

## 5. Parser rules

Keep `parse_hit_header` (`[N] <wing> / <room>`) and `is_rule_line` (lines starting with `─` or `──`).

After a header, walk lines until the next header or rule line:

| Line (after `trim_start`) | Action |
|---|---|
| `Source: …` | Set `source`, do not start body |
| `Match: …` | Skip |
| empty | Skip while body has not started; once body started, skip (do not insert blank lines) |
| starts with `→` | Start or continue body; strip one leading `→` and following whitespace from that line |
| anything else | Start or continue body; store `trim_start()` of the line |

Emit a `PalaceDrawer` only when `text` is non-empty after this walk. Truncation (`PALACE_ITEM_MAX_CHARS` + `…`) stays in `truncate_drawer_text` / `attach_palace`, not in the parser.

`text` is the body lines joined by `\n`. Do not summarize. Do not strip trailing `}` from JSON fragments — that is drawer content.

## 6. Timeout

| Constant | Old | New |
|---|---|---|
| `PALACE_TIMEOUT_MS` | 2500 | **8000** |

`CliPalaceSearch.timeout_ms` still comes from this constant. Tests that pass an explicit `timeout_ms: 200` (child-kill test) stay as they are.

No env override in this slice.

## 7. Fixture

Commit `engram/testdata/mempalace-search-3.3.5.txt` with stdout captured from MemPalace 3.3.5 (at least two hits, `Source:` + `Match:`, no `→`, rule separators). Tests read that file. Do not call a live palace in default CI.

Optional live smoke (not default CI): if `ENGRAM_PALACE_LIVE=1` and `mempalace` is on `PATH`, a `#[ignore]` or `cfg`-gated test may spawn it. Default `cargo test` must not.

## 8. Success

- `parse_search_output` on the 3.3.5 fixture returns ≥2 drawers; first hit has non-empty `wing`, `room`, `source`, and `text` that includes a substring of the captured body.
- `parse_two_cli_hits` (legacy `→`) still passes.
- `parse_search_output("no results\n")` is still empty.
- `PALACE_TIMEOUT_MS == 8000`.
- Bridge spec §7 parse paragraph and timeout table match this document.
- README palace section notes that MemPalace 3.3.x CLI text is the parse target.

## 9. Risks

- Future MemPalace CLI changes will break the parser again. Mitigation: fixture + one test named after the version; if `--json` appears later, prefer it in a new slice.
- Raising timeout to 8s can stall an opted-in `get_context` when the palace hangs until kill. Mitigation: still kill the child; default `get_context` does not opt in.
