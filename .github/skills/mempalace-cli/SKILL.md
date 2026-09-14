---
name: mempalace-cli
description: Use when Copilot MCP is unavailable or blocked and the question is what was decided, what happened last session, who someone is, or anything that may already be filed in MemPalace.
---

# MemPalace CLI recall

MCP is not available. Search with the `mempalace` CLI before answering from model memory.

## When

Past work, decisions, people, last session, "remember when". Not greenfield edits (rename, typo, new file with no history). Not "where is this implemented?" — that is Engram CLI.

## Search

Wing is required. Read `palace_wing` from `.engram/config.toml`. If it is missing or empty, ask. Never search without `--wing`.

```bash
mempalace search --wing <palace_wing> --results 5 "<short query>"
```

Add `--room <palace_room>` only when `palace_room` is set and non-empty.

Query: keywords or a short question. Not the conversation.

## Quote

CLI `cosine=` is similarity (higher is better). Floor is `palace_min_cosine` from config, default 0.6. Quote the drawer verbatim only at or above the floor. Missing cosine, below floor, no hits, timeout, or `mempalace` not on PATH: palace has nothing. Do not paraphrase. Do not invent.

## After

Do not mine the repo into the palace as a substitute for `engram index`. Do not dump long palace listings into the prompt.
