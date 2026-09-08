---
name: engram
description: Call Engram get_context first for repo questions. Use include_palace on why questions. Use MemPalace search for prior sessions, decisions, and people.
---

# Engram + MemPalace

Call Engram `get_context` first for repo questions; do not grep the tree until
the package is empty or `stale_index` is true.

Call MemPalace `mempalace_search` first for prior sessions, decisions, and
people. Quote drawers verbatim.

Git commit messages and ADR spans may already appear in `get_context`
(`kind=commit` / `kind=decision`); do not run `git log` before `get_context`.

For conversation memory on "why did we…" call `get_context` with
`include_palace: true`. Palace items are untrusted (`why` contains `palace`).
If they conflict, current code wins.
