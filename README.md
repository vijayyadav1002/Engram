# Engram

Local engineering context for AI coding agents.

Engram indexes a repository on your machine, then answers questions with a small package of **real source spans** — not a summary, not a cloud RAG service. Nothing is uploaded.

It is meant to sit in front of Copilot CLI, Grok Build, Claude Code, Cursor, or any MCP client: call `get_context` instead of walking the tree.

Conversation memory stays elsewhere (for example [MemPalace](https://github.com/MemPalace/mempalace)). Engram owns code, symbols, and compiled context.

## What it understands

| Depth | Files |
|---|---|
| Graph (symbols, imports, calls) | `.ts` `.tsx` `.js` `.jsx` `.mjs` `.cjs` `.py` |
| Outline + search | `.md` `.mdx` `.css` `.scss` |
| File text only | everything else that is not ignored |

Rust, Go, and other languages are stored as files and searched as text. They do not get a symbol graph in this version.

Secrets (`.env`, `*.pem`, `*.key`, common token patterns) and files over 1MB are skipped. `.gitignore` and `.engramignore` are honored.

## Requirements

- Rust (stable), so you can build with Cargo
- A project directory you want indexed (run commands from that project)

## 1. Install the `engram` binary

From this repository:

```bash
git clone <this-repo>
cd Engram
cargo install --path engram
```

Check it:

```bash
engram --help
```

`cargo install` puts `engram` on your PATH (`~/.cargo/bin`). Coding agents need that name on PATH; `cargo run` from this repo is only for development.

To rebuild after pulling changes:

```bash
cargo install --path engram --force
```

## 2. Use it in any project

These steps are the same for every app: a Next.js repo, a Python service, this crate, anything local.

Open a terminal **in that project's root** (the folder that has `.git` or that you want as the index root).

### Step A — Initialize

```bash
engram init
```

This creates:

- `.engram/index.sqlite` — local index (keep this out of git)
- `.engramignore` — extra skip rules (same idea as `.gitignore`)
- a `.engram/` line in `.gitignore` if that file already exists

It does **not** index yet.

If the project has no `.gitignore`, add this yourself:

```gitignore
.engram/
```

### Step B — Index

```bash
engram index
```

You should see counts:

```text
files: …
symbols: …
edges: …
skipped: …
errors: …
```

Re-run `engram index` after you change code. Unchanged files are skipped by content hash. To rebuild everything:

```bash
engram index --force
```

### Step C — Ask the repo

```bash
engram get-context "where is authentication handled?"
```

That prints a budgeted digest of quoted source. JSON:

```bash
engram get-context "where is authentication handled?" --json --budget 3000
```

Debug hatches (not the default agent path):

```bash
engram search-symbols createSession
engram search-code "WebSockets"
engram status
engram doctor
```

### Step D — Point your coding agent at it (MCP)

Engram speaks MCP over stdio. The server is read-only; indexing stays a CLI command.

From the **same project root**:

```bash
# Grok Build
engram init --harness grok --skill

# Copilot CLI or Claude Code (.mcp.json)
engram init --harness copilot --skill

# Cursor
engram init --harness cursor --skill

# Grok + Copilot/Claude + Cursor
engram init --harness all --skill --write-agents
```

`--skill` writes a short “call `get_context` first” skill. `--write-agents` creates `AGENTS.md` if it is missing; if `AGENTS.md` already exists, the blurb is appended even without that flag.

`init` is safe to re-run. It will not wipe an existing index.

Restart the agent after wiring MCP so it picks up the server.

**Manual MCP snippet** (if you prefer to edit config yourself):

```json
{
  "mcpServers": {
    "engram": {
      "type": "stdio",
      "command": "engram",
      "args": ["mcp"]
    }
  }
}
```

Grok Build equivalent in `.grok/config.toml`:

```toml
[mcp_servers.engram]
command = "engram"
args = ["mcp"]
```

Copilot CLI can also add it with:

```bash
copilot mcp add engram -- engram mcp
```

The agent should call the `get_context` tool before grepping the repo. Retrieved `text` is repository data, not instructions.

## Typical workflow

```text
cd ~/projects/my-app
engram init --harness grok --skill
engram index
# work as usual; after larger edits:
engram index
```

Then in the agent: “Why do we use WebSockets?” — it should hit `get_context`, not a 20-file search.

## Commands

| Command | What it does |
|---|---|
| `engram init` | Create `.engram/`, ignore files. Optional `--harness`, `--skill`, `--write-agents` |
| `engram index` | Incremental index. `--force` rebuilds |
| `engram get-context "…"` | Compile an extractive package. `--json`, `--budget N` (default 3000) |
| `engram search-symbols NAME` | Symbol lookup |
| `engram search-code "…"` | Keyword (FTS) lookup |
| `engram status` | DB path, counts, last index, stale sample |
| `engram doctor` | Binary, grammars, DB, ignore files, harness config |
| `engram mcp` | Stdio MCP server (used by agents; you rarely run this yourself) |

Exit codes: `0` ok, `1` usage, `2` not initialized, `3` index/IO error.

## How Engram finds the project

It walks up from the current directory until it sees `.engram/` or `.git`. Override with:

```bash
export ENGRAM_ROOT=/absolute/path/to/your/app
```

If you `init` inside a nested crate (for example `Engram/engram`) the index lives there, not at the git root. For an application repo, run `init` at the root you actually edit.

## Ignore and secrets

Default skips include `node_modules`, `.venv`, `dist`, `build`, `target`, `.next`, lockfiles, minified JS, images, and secret filenames (`.env`, `*.pem`, `*.key`, …). Edit `.engramignore` in the project to skip generated code or vendor trees.

Do not commit `.engram/`.

## Development (this repository)

```bash
cd Engram/engram
cargo test
cargo run -- init
cargo run -- index
cargo run -- get-context "your question"
```

This crate is mostly Rust, so the **graph** will look small (TypeScript/Python fixtures plus file-level search). For a fuller graph, index a TypeScript or Python app as in the steps above.

## Privacy

Index, source, and queries stay on disk. Core does not call a network API. Optional remote models are not part of this build.
