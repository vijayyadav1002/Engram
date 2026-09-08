# Engram–MemPalace Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optionally attach up to three verbatim MemPalace drawers onto an Engram `get_context` package without changing default Core behavior.

**Architecture:** After the existing extractive compiler finishes, if the caller opted in and budget remains, a `PalaceSearch` adapter runs `mempalace search --results 3 <query>` (argv only, timeout 2500ms), parses CLI text into drawers, and appends `ContextItem`s with `why=["palace"]`. Missing binary, timeout, or parse failure never fails `get_context`. Tests inject a fake searcher; CI does not call a live palace.

**Tech Stack:** Existing Engram crate (Rust, clap, serde_json). No new HTTP client. No `toml` crate — parse `.engram/config.toml` for a `palace = true/false` line only.

**Spec:** `docs/superpowers/specs/2026-09-08-engram-mempalace-bridge-design.md`

## Global Constraints

- Default **off**. `get_context` without opt-in must match Core (no `palace` key in JSON).
- Palace failure or absence never fails `get_context`.
- Code items first; palace fills remaining budget only.
- Caps: `PALACE_MAX_HITS=3`, `PALACE_ITEM_MAX_CHARS=1200`, `PALACE_MIN_REMAINING=200`, `PALACE_TIMEOUT_MS=2500`, query argv max **250** characters.
- Drawers are verbatim (truncate with `…`, never summarize). No LLM, no network, no nested MCP client, no palace writes.
- `path` = `palace://{wing}/{room}`, `kind` = `"palace"`, `why` = `["palace"]`. No palace rows in `edges`.
- Inject `PalaceSearch` in tests; do not call a real `mempalace` in CI.
- Keep `pub fn get_context(root, query, budget_tokens)` working; add `get_context_with` for opts.
- Work in `engram/`. Run `cd engram && cargo test …`.
- TDD on every task.

## File map

| File | Responsibility |
|---|---|
| `engram/src/types.rs` | `PalaceStats`, optional `ContextStats.palace` |
| `engram/src/palace.rs` | parse CLI, `PalaceSearch` trait, CLI adapter, opt-in, constants |
| `engram/src/compile.rs` | `GetContextOpts`, `get_context_with`, attach palace items |
| `engram/src/lib.rs` | `pub mod palace` |
| `engram/src/main.rs` | `--palace` flag |
| `engram/src/mcp.rs` | `include_palace` arg + tool description |
| `engram/tests/palace_bridge.rs` | integration with fake searcher |

Do not add Git, embeddings, or conversation mining.

---

### Task 1: Palace stats types

**Files:**
- Modify: `engram/src/types.rs`
- Modify: `engram/src/lib.rs` (re-export if needed)

**Interfaces:**
- Consumes: existing `ContextStats`, `ContextPackage`
- Produces:
  - `pub struct PalaceStats { pub status: String, pub attempted: u32, pub included: u32, pub dropped_for_budget: u32 }`
  - `ContextStats.palace: Option<PalaceStats>` with `#[serde(skip_serializing_if = "Option::is_none")]`
  - Status strings exactly: `disabled` | `not_installed` | `timeout` | `unparseable` | `ok`

- [ ] **Step 1: Write the failing test** in `types.rs` tests:

```rust
#[test]
fn palace_stats_omitted_when_none() {
    let pkg = ContextPackage {
        query: "q".into(),
        budget_tokens: 3000,
        used_tokens: 0,
        items: vec![],
        edges: vec![],
        stats: ContextStats {
            files_considered: 0,
            symbols_considered: 0,
            dropped_for_budget: 0,
            stale_omitted: 0,
            stale_index: false,
            truncated: false,
            palace: None,
        },
    };
    let v = serde_json::to_value(&pkg).unwrap();
    assert!(v["stats"].get("palace").is_none());
}

#[test]
fn palace_stats_serialized_when_present() {
    let pkg = ContextPackage {
        query: "q".into(),
        budget_tokens: 3000,
        used_tokens: 0,
        items: vec![],
        edges: vec![],
        stats: ContextStats {
            files_considered: 0,
            symbols_considered: 0,
            dropped_for_budget: 0,
            stale_omitted: 0,
            stale_index: false,
            truncated: false,
            palace: Some(PalaceStats {
                status: "ok".into(),
                attempted: 3,
                included: 2,
                dropped_for_budget: 1,
            }),
        },
    };
    let v = serde_json::to_value(&pkg).unwrap();
    assert_eq!(v["stats"]["palace"]["status"], "ok");
    assert_eq!(v["stats"]["palace"]["included"], 2);
}
```

Existing `context_package_json_field_names` must set `palace: None`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd engram && cargo test --lib types::`
Expected: FAIL/compile error — `palace` field missing on `ContextStats`.

- [ ] **Step 3: Write minimal implementation**

Add `PalaceStats` next to `ContextStats`. Add `palace: Option<PalaceStats>` with skip_serializing_if none. Update every `ContextStats { ... }` in the crate to include `palace: None` so it compiles (search with `ContextStats {`).

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib types:: && cargo test --lib`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/types.rs engram/src
git commit -m "feat: add optional palace stats on ContextPackage"
```

---

### Task 2: Parse MemPalace CLI text and truncate queries

**Files:**
- Create: `engram/src/palace.rs`
- Modify: `engram/src/lib.rs` (`pub mod palace`)

**Interfaces:**
- Consumes: nothing from Task 1 except maybe unused
- Produces:
  - `pub const PALACE_MAX_HITS: usize = 3`
  - `pub const PALACE_ITEM_MAX_CHARS: usize = 1200`
  - `pub const PALACE_MIN_REMAINING: u32 = 200`
  - `pub const PALACE_TIMEOUT_MS: u64 = 2500`
  - `pub const PALACE_QUERY_MAX_CHARS: usize = 250`
  - `pub struct PalaceDrawer { pub wing: String, pub room: String, pub source: String, pub text: String }`
  - `pub fn truncate_query(query: &str) -> String` — first 250 chars
  - `pub fn truncate_drawer_text(text: &str) -> String` — at most 1200 chars, append `…` if cut
  - `pub fn parse_search_output(stdout: &str) -> Vec<PalaceDrawer>`

Parse rules (from current `mempalace search` text):

- A hit starts with a line matching `[N] <wing> / <room>` (spaces around `/`).
- Optional `Source: <file>` line → `source`; else `source` is empty.
- The body is the line starting with `→` (strip the arrow and leading space) plus following indented lines until the next `[N]` header or a `──` rule or end.
- Ignore cosine/bm25 lines.

Fixture (put in the test):

```text
============================================================
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
```

- [ ] **Step 1: Write failing tests** in `palace.rs`:

```rust
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
    let out = /* fixture above */;
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib palace::`
Expected: FAIL — module missing

- [ ] **Step 3: Implement parser + truncators** in `palace.rs`. Do not spawn processes yet.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib palace::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/palace.rs engram/src/lib.rs
git commit -m "feat: parse mempalace search CLI output"
```

---

### Task 3: `PalaceSearch` trait and CLI adapter

**Files:**
- Modify: `engram/src/palace.rs`

**Interfaces:**
- Consumes: `parse_search_output`, `truncate_query`, `PALACE_*` constants, `PalaceDrawer`
- Produces:
  - `pub enum PalaceError { NotInstalled, Timeout, Unparseable, Io(String) }`
  - `pub trait PalaceSearch: Send + Sync { fn search(&self, query: &str, limit: usize) -> Result<Vec<PalaceDrawer>, PalaceError>; }`
  - `pub struct CliPalaceSearch { pub bin: PathBuf, pub cwd: PathBuf, pub timeout_ms: u64 }`
  - `impl PalaceSearch for CliPalaceSearch`
  - `pub struct FakePalaceSearch { pub drawers: Vec<PalaceDrawer>, pub error: Option<PalaceError> }` for tests (`Clone` ok)
  - `pub fn default_bin() -> PathBuf` — `ENGRAM_PALACE_BIN` if set, else `"mempalace"`

CLI adapter:

- `Command::new(&self.bin).args(["search", "--results", &limit.to_string(), &truncate_query(query)]).current_dir(&self.cwd).stdout(Stdio::piped()).stderr(Stdio::piped())`
- No shell.
- Wait with timeout: spawn, `std::thread` + `mpsc::recv_timeout(Duration::from_millis(self.timeout_ms))`; on timeout `child.kill()` → `PalaceError::Timeout`.
- `ErrorKind::NotFound` → `NotInstalled`.
- Non-zero exit with empty stdout → `Unparseable` if parse is empty, else return parsed hits.
- Empty parse → `Err(Unparseable)` so `get_context` can set status (Task 5 maps errors to stats).
- Limit passed to CLI is `min(limit, PALACE_MAX_HITS)`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn fake_search_returns_drawers() {
    let fake = FakePalaceSearch {
        drawers: vec![PalaceDrawer {
            wing: "w".into(),
            room: "r".into(),
            source: "s".into(),
            text: "hello".into(),
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
    assert!(matches!(fake.search("q", 3), Err(PalaceError::NotInstalled)));
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
fn cli_timeout_kills_child() {
    // Use a portable sleep: `perl -e "select(undef,undef,undef,3)"` may be missing.
    // Use the current engram test helper: Command that is `sleep` on unix.
    #[cfg(unix)]
    {
        let cli = CliPalaceSearch {
            bin: PathBuf::from("sleep"),
            cwd: std::env::temp_dir(),
            timeout_ms: 200,
        };
        // sleep 10 — adapter always prepends `search --results`; sleep will fail fast
        // instead use /bin/sh -c is forbidden (no shell).
        // Implement timeout test against a tiny wrapper is overkill.
        // Test Timeout by making CliPalaceSearch wait on `cat` with no stdin close:
        // skip if too environment-specific; instead unit-test the wait helper.
    }
}
```

For timeout without a custom binary, extract:

```rust
pub fn wait_with_timeout(child: &mut std::process::Child, timeout_ms: u64) -> Result<std::process::Output, PalaceError>
```

Test it with a function that you can call — or skip live timeout in this task and cover Timeout via `FakePalaceSearch` plus a unit test that `recv_timeout` path returns `Timeout` using a `Command::new("sleep").arg("5")` **only if** you change the adapter to allow extra args for tests.

**Ruling for implementer:** `CliPalaceSearch` always runs `bin search --results N query`. Timeout test: set `bin` to an executable script written into tempdir:

```text
#!/bin/sh
sleep 5
```

`chmod +x`, `timeout_ms=200` → `Timeout`. Unix-only `#[cfg(unix)]`. On other OS, skip.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --lib palace::`
Expected: FAIL — trait missing

- [ ] **Step 3: Implement trait, Fake, CliPalaceSearch, wait_with_timeout**

Log CLI stderr with `eprintln!` only when non-empty (Engram logs go to stderr).

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib palace::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/palace.rs
git commit -m "feat: add PalaceSearch trait and CLI adapter"
```

---

### Task 4: Opt-in resolution

**Files:**
- Modify: `engram/src/palace.rs`

**Interfaces:**
- Consumes: none
- Produces:
  - `pub enum PalaceOpt { Enable, Disable, Unspecified }`
  - `pub fn parse_palace_config_toml(text: &str) -> PalaceOpt` — last `palace = true` or `palace = false` line (trim, ignore comments `#`). Other keys ignored.
  - `pub fn resolve_opt_in(explicit: Option<bool>, env_palace: Option<&str>, config_text: Option<&str>) -> bool`
  - Precedence: `explicit == Some(false)` or env `0`/`false`/`no` → **false**. Then `explicit == Some(true)` → true. Then env `1`/`true`/`yes` → true. Then config `palace = true` → true. Else false.
  - Disable always wins: if env is `0` even with `explicit true`? Spec: “Disable always wins if `include_palace: false` or `ENGRAM_PALACE=0`.” Explicit false **or** env 0 disables. Explicit true with env 0 → **false**. Explicit true with env unset → true.
  - `pub fn read_config_text(root: &Path) -> Option<String>` reads `root.join(".engram/config.toml")` if present.

- [ ] **Step 1: Write failing tests**

```rust
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
```

- [ ] **Step 2: Run to verify fail**

Run: `cd engram && cargo test --lib palace::resolve_opt_in`
Expected: FAIL

- [ ] **Step 3: Implement** `parse_palace_config_toml` and `resolve_opt_in` as specified.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --lib palace::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/palace.rs
git commit -m "feat: resolve palace opt-in from flag, env, and config"
```

---

### Task 5: Attach palace items in `get_context_with`

**Files:**
- Modify: `engram/src/compile.rs`
- Modify: `engram/src/lib.rs` if exports needed
- Create: `engram/tests/palace_bridge.rs`

**Interfaces:**
- Consumes: `get_context`, `token_cost` (make `pub(crate)`), `PalaceSearch`, `FakePalaceSearch`, `resolve_opt_in`, `read_config_text`, `truncate_drawer_text`, `PALACE_*`, `PalaceStats`, `ContextItem`
- Produces:
  - `pub struct GetContextOpts { pub include_palace: Option<bool>, pub palace_search: Option<Arc<dyn PalaceSearch>> }`
  - `impl Default for GetContextOpts`
  - `pub fn get_context_with(root: &Path, query: &str, budget_tokens: u32, opts: GetContextOpts) -> Result<ContextPackage, Error>`
  - `pub fn get_context(...)` becomes `get_context_with(..., GetContextOpts::default())`

Attach algorithm (after existing package `items`/`used_tokens`/`truncated` are computed, **before return**):

1. `env = std::env::var("ENGRAM_PALACE").ok()`
2. `cfg = read_config_text(root)`
3. If `!resolve_opt_in(opts.include_palace, env.as_deref(), cfg.as_deref())` → leave `stats.palace = None`, return (Core identical).
4. Remaining = `budget_tokens.saturating_sub(used_tokens)`. If remaining < `PALACE_MIN_REMAINING` → `stats.palace = Some(PalaceStats { status: "ok", attempted: 0, included: 0, dropped_for_budget: 0 })` — actually spec says skip if remaining < 200. Use status `"ok"` with attempted 0, or treat as skip without calling adapter. **Do not call searcher.** `palace = Some(PalaceStats { status: "ok", attempted: 0, included: 0, dropped_for_budget: 0 })` is noisy. Spec: “Otherwise skip palace and set stats.palace accordingly.” Use `status: "ok", attempted: 0, included: 0, dropped_for_budget: 0` only if we searched. If we skip for budget before search: `status: "ok", attempted: 0, included: 0, dropped_for_budget: 0` is fine **or** omit. **Ruling:** if opted in but remaining < 200, set `palace = Some(PalaceStats { status: "ok", attempted: 0, included: 0, dropped_for_budget: 0 })` and do not spawn.
5. `searcher = opts.palace_search.clone().unwrap_or_else(|| Arc::new(CliPalaceSearch { bin: default_bin(), cwd: root.to_path_buf(), timeout_ms: PALACE_TIMEOUT_MS }))`
6. `match searcher.search(query, PALACE_MAX_HITS)`:
   - `Err(NotInstalled)` → `status: "not_installed"`, items unchanged
   - `Err(Timeout)` → `status: "timeout"`
   - `Err(Unparseable)` | `Io(_)` → `status: "unparseable"`
   - `Ok(drawers)` → for each drawer, `text = truncate_drawer_text(&d.text)`, item:

```rust
ContextItem {
    path: format!("palace://{}/{}", d.wing, d.room),
    start_line: 1,
    end_line: 1,
    symbol: None,
    kind: Some("palace".into()),
    text,
    why: vec!["palace".into()],
}
```

   Token cost = `token_cost(&item.text)`. If `used + cost > budget_tokens` or JSON would exceed 16KB, `dropped_for_budget++` and stop. Else push item, add cost.
7. Re-run JSON 16KB pop-until-fits **after** palace append (may drop palace items first if they are last).
8. Never add palace to `edges`.

- [ ] **Step 1: Write failing tests** in `engram/tests/palace_bridge.rs` (use the same temp repo helper pattern as `compiler_pkg.rs`: write `session.ts`, `Store::create`, `index_repo`, then `get_context_with`).

```rust
#[test]
fn disabled_omits_palace_key() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let pkg = get_context(&root, "createSession", 3000).unwrap();
    let v = serde_json::to_value(&pkg).unwrap();
    assert!(v["stats"].get("palace").is_none());
    assert!(pkg.items.iter().all(|i| i.kind.as_deref() != Some("palace")));
}

#[test]
fn missing_searcher_sets_not_installed() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let fake = Arc::new(FakePalaceSearch {
        drawers: vec![],
        error: Some(PalaceError::NotInstalled),
    });
    let pkg = get_context_with(
        &root,
        "createSession",
        3000,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    assert_eq!(pkg.stats.palace.as_ref().unwrap().status, "not_installed");
    assert!(pkg.items.iter().any(|i| i.symbol.as_deref() == Some("createSession")));
}

#[test]
fn fake_three_drawers_budget_keeps_two() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let drawers: Vec<PalaceDrawer> = (0..3)
        .map(|i| PalaceDrawer {
            wing: "w".into(),
            room: format!("r{i}"),
            source: "s".into(),
            text: "word ".repeat(80), // plenty of tokens
        })
        .collect();
    let fake = Arc::new(FakePalaceSearch {
        drawers,
        error: None,
    });
    // Small remaining budget after code: use a tight budget that still fits createSession
    // plus ~2 drawers. Tune repeats so included==2, dropped_for_budget==1.
    let pkg = get_context_with(
        &root,
        "createSession",
        /* pick budget after seeing used_tokens of code-only run in a first assert */,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    let p = pkg.stats.palace.unwrap();
    assert_eq!(p.attempted, 3);
    assert_eq!(p.included, 2);
    assert_eq!(p.dropped_for_budget, 1);
    assert_eq!(
        pkg.items.iter().filter(|i| i.why.iter().any(|w| w == "palace")).count(),
        2
    );
    assert!(pkg.items.iter().any(|i| i.path.starts_with("palace://")));
}

#[test]
fn timeout_keeps_code_items() {
    let root = repo();
    index_repo(&root, true).unwrap();
    let fake = Arc::new(FakePalaceSearch {
        drawers: vec![],
        error: Some(PalaceError::Timeout),
    });
    let pkg = get_context_with(
        &root,
        "createSession",
        3000,
        GetContextOpts {
            include_palace: Some(true),
            palace_search: Some(fake),
        },
    )
    .unwrap();
    assert_eq!(pkg.stats.palace.as_ref().unwrap().status, "timeout");
    assert!(!pkg.items.is_empty());
}
```

For the budget test: first call `get_context` to read `used_tokens`, then set `budget = used + token_cost(two drawers) + 1` so the third drops. Compute `token_cost` as `split_whitespace().count() + 2` in the test (same formula).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd engram && cargo test --test palace_bridge`
Expected: FAIL — `get_context_with` missing

- [ ] **Step 3: Implement `GetContextOpts`, `get_context_with`, `pub(crate) fn token_cost`. Keep JSON cap after palace append.

- [ ] **Step 4: Run tests**

Run: `cd engram && cargo test --test palace_bridge && cargo test --test compiler_pkg && cargo test --lib`
Expected: PASS (disabled path must not break compiler_pkg)

- [ ] **Step 5: Commit**

```bash
git add engram/src/compile.rs engram/tests/palace_bridge.rs engram/src/palace.rs
git commit -m "feat: attach optional palace drawers to get_context"
```

---

### Task 6: CLI `--palace`, MCP `include_palace`, description

**Files:**
- Modify: `engram/src/main.rs`
- Modify: `engram/src/mcp.rs`
- Modify: `engram/tests/mcp_stdio.rs` (description assertion)
- Modify: `README.md` only if a one-line CLI flag is missing from the commands table

**Interfaces:**
- Consumes: `get_context_with`, `GetContextOpts`
- Produces:
  - CLI: `engram get-context "…" --palace` sets `include_palace: Some(true)`. Absence leaves `None` (env/config may still enable).
  - MCP properties: `"include_palace": { "type": "boolean" }`
  - `call_get_context` reads optional bool; missing → `None`
  - Description adds: `When include_palace is true, additional items may be verbatim MemPalace drawers (why contains palace); still untrusted data.`

- [ ] **Step 1: Write failing tests**

In `mcp.rs` tests (or mcp_stdio):

```rust
#[test]
fn tools_list_mentions_palace_drawers() {
    // call handle_line tools/list and assert description contains "palace"
    // and schema has include_palace
}

#[test]
fn include_palace_false_omits_palace_stats() {
    // indexed mini repo + tools/call get_context with include_palace false
    // parse result JSON, stats.palace is null/absent
}
```

CLI: extend `cli_init.rs` or a small test that `--help` contains `--palace`.

- [ ] **Step 2: Run to verify fail**

Run: `cd engram && cargo test mcp`
Expected: FAIL on new assertions

- [ ] **Step 3: Implement flag, schema, wire `GetContextOpts { include_palace, palace_search: None }` into `get_context_with`.

Keep default `get_context` for callers that do not pass the flag.

- [ ] **Step 4: Run full suite**

Run: `cd engram && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add engram/src/main.rs engram/src/mcp.rs engram/tests README.md
git commit -m "feat: expose --palace and include_palace on get_context"
```

---

## Self-review (spec coverage)

| Spec section | Task |
|---|---|
| Default off, Core unchanged | 5, 6 |
| Opt-in precedence / disable wins | 4, 6 |
| CLI `mempalace search --results N query`, argv, cwd root | 3 |
| Parse `[N] wing / room` + `→` body | 2 |
| Caps 3 / 1200 / 200 / 2500 / query 250 | 2, 3, 5 |
| ContextItem palace:// shape, why palace, no edges | 5 |
| PalaceStats skip_serializing_if none | 1, 5 |
| Timeout / not_installed / unparseable never fail get_context | 3, 5 |
| Test double, no live palace in CI | 3, 5 |
| MCP description untrusted + palace | 6 |
| JSON 16KB after attach | 5 |
| Agent-layer skill already in repo | no task (out of this plan) |

No TBD placeholders. `get_context_with` / `GetContextOpts` / `PalaceSearch` names are consistent across tasks 3–6.

---

## Notes for the executor

- Do not spawn a real MemPalace in CI. `CliPalaceSearch` missing-bin and unix timeout script are the only real processes.
- Do not add a `toml` crate.
- Do not change ranking, extractors, or indexer.
