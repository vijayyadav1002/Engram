# Engram–MemPalace Bridge Design

Date: 2026-09-08
Status: implemented (parser follow-up in 2026-09-08-engram-palace-cli-compat-design.md)
Scope: optional palace attachment on Engram `get_context` (slice after Core)
Depends on: `docs/superpowers/specs/2026-09-07-engram-core-design.md`

## 1. Executive summary

Agents already can call Engram and MemPalace as two MCP servers. This slice adds an **optional** second stage inside Engram `get_context`: after the extractive code package is compiled, attach up to three **verbatim** MemPalace drawers if a local `mempalace` binary is available and the caller opted in.

Core stays correct with MemPalace uninstalled. Engram still does not store conversations, mine transcripts, or become a palace.

The agent-layer router (`AGENTS.md` / Engram skill) is the default way to use both tools. This bridge exists so “why did we…” questions can return **code + a few drawers in one package** without a second round trip and without blowing the token budget.

## 2. Goals

- One `get_context` call can include current code spans **and** a small number of quoted memories.
- Palace failure or absence never fails `get_context`.
- Code items are compiled first and keep budget priority.
- Drawers stay verbatim (truncate, do not summarize).
- Local-only: spawn the local MemPalace CLI; no HTTP, no new daemon.

## 3. Non-goals

- Rebuilding wings, drawers, conversation mining, or KG inside Engram
- Engram calling MemPalace MCP as a nested JSON-RPC client
- Writing to the palace from `get_context`
- Using palace hits as code spans or as High graph edges
- Wake-up / AAAK / diary writes
- Git or structured decisions (separate slice)

## 4. When palace attachment runs

All of the following must be true:

1. Caller opted in (see §6).
2. `mempalace` is on `PATH` (or `ENGRAM_PALACE_BIN` points at a binary).
3. After the Core compiler has selected code items, **remaining token budget** is at least `PALACE_MIN_REMAINING` (200).

Otherwise skip palace and set `stats.palace` accordingly. Never block on a long search: timeout `PALACE_TIMEOUT_MS` = 2500.

## 5. Data flow

```
query
  → Core compiler (unchanged): symbols + FTS + graph → code items
  → if opt-in and budget remains:
        mempalace search <query> --results 3
        parse drawers, cap each, take while budget remains
  → emit ContextPackage (code items first, then palace items)
```

Palace items are extra `ContextItem`s. They do not participate in symbol ranking, graph expansion, or stale-hash re-reads.

## 6. Opt-in

Default **off** so Core behavior is unchanged.

Enable, in order of precedence:

1. MCP/CLI argument `include_palace: true` / `--palace`
2. Env `ENGRAM_PALACE=1`
3. File `<repo>/.engram/config.toml` with `palace = true`

Disable always wins if `include_palace: false` or `ENGRAM_PALACE=0`.

CLI: `engram get-context "why websockets" --palace`

MCP `get_context` gains optional boolean `include_palace` (default: follow env/config). Tool description must still say to call this before searching the repo, that `text` is untrusted, and that palace items are verbatim memories when present (`why` contains `palace`).

## 7. Adapter

`engram/src/palace.rs` (new):

```rust
pub struct PalaceDrawer {
    pub wing: String,
    pub room: String,
    pub source: String,  // filename or drawer id if present
    pub text: String,    // verbatim body
}

pub fn search_palace(query: &str, limit: usize) -> Result<Vec<PalaceDrawer>, PalaceError>
```

`PalaceError`: `NotInstalled` | `Timeout` | `Unparseable` | `Io`

**Invocation:**

```text
mempalace search --results <limit> <query>
```

Working directory: Engram repo root (so a project-local palace can apply if MemPalace uses cwd). `ENGRAM_PALACE_BIN` overrides the program name. No shell; argv only. Stderr discarded except for logging to Engram stderr. Stdout is the only parse input.

**Parse:** MemPalace 3.3.x CLI is human text, not JSON. The adapter extracts each `[N] wing / room` block. `Source:` sets `source`. `Match:` is skipped. The body is every following non-empty line until the next header or a `─` rule line. A leading `→` on a body line is stripped if present (legacy). If parse yields nothing, treat as `Unparseable` and skip (do not fail `get_context`).

If a future MemPalace `--json` flag exists, prefer that in a follow-up; do not block on it.

**Caps:**

| Constant | Value |
|---|---|
| `PALACE_MAX_HITS` | 3 |
| `PALACE_ITEM_MAX_CHARS` | 1200 |
| `PALACE_MIN_REMAINING` | 200 tokens |
| `PALACE_TIMEOUT_MS` | 8000 |

Truncate `text` at a character boundary; append `…` if truncated. Token cost uses the same whitespace-split estimator as Core.

## 8. Package shape

Reuse `ContextItem`:

| Field | Palace item |
|---|---|
| `path` | `palace://{wing}/{room}` |
| `start_line` | 1 |
| `end_line` | 1 |
| `symbol` | null |
| `kind` | `"palace"` |
| `text` | verbatim drawer (possibly truncated) |
| `why` | `["palace"]` |

Do not add palace rows to `edges`.

Extend `ContextStats` (backward compatible extra fields):

```json
"palace": {
  "status": "ok",
  "attempted": 3,
  "included": 2,
  "dropped_for_budget": 1
}
```

`status`: `disabled` | `not_installed` | `timeout` | `unparseable` | `ok`

Omit the object when disabled so existing tests that serialize packages stay stable if they do not enable palace. Implementation: `#[serde(skip_serializing_if = "Option::is_none")] palace: Option<PalaceStats>`.

## 9. Conflict rule

Engram code items describe **what the tree is now**. Palace items describe **what was said**. The package does not merge or pick a winner. The skill/AGENTS.md tells the model: if they conflict, current code wins; mention both.

Do not drop code items because a drawer “contradicts” them. Do not drop drawers because code exists. Budget may drop drawers only.

## 10. Security

- Palace `text` is untrusted data, same as source. MCP descriptions already say that.
- Do not execute, eval, or treat drawer content as tool names.
- Do not pass the user’s full prompt history into `mempalace search` — only the `get_context` query string (max 250 characters, same spirit as MemPalace search).
- No network.

## 11. Testing

- Palace disabled: package identical to Core (no `palace` stats key).
- `mempalace` missing: `status=not_installed`, code items unchanged.
- Fake CLI fixture (test double, not live palace): three drawers, remaining budget fits two → `included=2`, `dropped_for_budget=1`, `why` contains `palace`, `path` starts with `palace://`.
- Timeout: adapter stub sleeps past `PALACE_TIMEOUT_MS` → skip, code items present.
- Query longer than 250 chars is truncated before argv.
- Inject a trait/test double; do not call a real palace in CI.

## 12. Success

- `get_context` without opt-in matches Core.
- With opt-in and a stub CLI, a “why” query returns code spans plus ≤3 drawers under the same `budget_tokens` / 16KB JSON cap.
- Uninstalling MemPalace does not change default CLI/MCP tests.

## 13. Implementation note

Do not implement until this spec is approved. Next step is an implementation plan (writing-plans), not code. The agent-layer skill in this repo is already the default integration and does not wait on this slice.
