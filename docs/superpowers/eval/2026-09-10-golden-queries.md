# Golden queries — Engram + MemPalace

Date: 2026-09-10
Spec: `docs/superpowers/specs/2026-09-10-engram-mempalace-optimization-design.md`

Contamination (session blobs, `palace://sessions/*`, other-project specs) is a
hard fail even when the right file is also present.

Git+decisions is already on main (`engram/src/git.rs`). Do not re-implement it.
After MDA `engram index`, stop remining those ADR files into the palace.

## CI fixture (always)

Implemented by `engram/tests/ranking_golden.rs` and `engram/tests/palace_bridge.rs`.

Tiny repo files:

- `services/trash.ts` — `TrashItemRow`, `purgeTrash`, soft-delete
- `config.ts` — `TRASH_RETENTION_DAYS`
- `RemoveTagsDialog.tsx` — delete-tags UI (competes on FTS)
- `pdf_thumbnail.ts` — unrelated + “retention” of rendered pages
- `docs/adrs/ADR-001-trash.md` — Decision heading for retention days

| Query | Must include | Must not include |
|---|---|---|
| `how does trash soft-delete work?` | `services/trash.ts` | `kind=palace` when palace off |
| `trash retention` | `services/trash.ts` | `kind=palace` when palace off |
| `TRASH_RETENTION_DAYS` | `services/trash.ts` (may also include `config.ts`) | `kind=palace` when palace off |

Palace-bridge (fake searcher):

- Palace off: no `kind=palace`; `stats.palace` omitted
- Cosine 0.34: `status = below_threshold`, no palace items
- Cosine 0.72 with `palace_wing` set: attached, `path` = `palace://{wing}/{room}`
- Opt-in, empty wing: searcher not called; `unscoped_disabled`

Pass if `used_tokens ≤ 3000` on the ranking queries.

## Local MDA eval (not CI)

Repo: `/Users/vijay/Projects/mda`. Default budget 3000.
Archive: ~/.mempalace/palace.archive-20260910

| Query | Must include | Must not include |
|---|---|---|
| how does trash soft-delete / TRASH_RETENTION_DAYS work? | `apps/backend/src/services/trash.ts` (or current path) | `palace://sessions/*`, other-project specs |
| same query, palace off | same file | any `kind=palace` |
| same query, `include_palace` after attach ships | same file; palace items only `palace://mda/...` at similarity ≥ 0.6 if any | session blobs, `github` frontend specs |

Run after ranking (Task 5) and again after hygiene (Task 9) + attach (Task 8).
