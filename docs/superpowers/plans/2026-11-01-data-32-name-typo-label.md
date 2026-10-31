# DATA-32: a typo that matches only a table's name says "approximate match"

Backlog `DATA-32` (Task, Data, P3). Reassigned to the planner lane on 2026-11-01. Base: `main` at
`a33e3d81`. Planner: Claude Opus. Developer: another agent (`AGENTS.md`). If the code disagrees, stop
and report it.

## What is wrong (confirmed)

- **The server drops the flag.** `Field::reported`
  (`rust/crates/lakehouse-api/src/routes/catalog_search.rs:106`) returns `None` for `Name`.
  Reason selection (`:345`) skips such hits, and the emitter (`:361-372`) writes no `matchedOn`,
  so a name hit's `approximate` flag (`:280-305`) is lost.
- **The console says nothing.** `matchedOnLabel` (`src/lib/palette-search.ts:32-50`) shows nothing
  without `matchedOn`.

## Decisions

- **D1.** When no reported field explains the hit, but some query word matched the name only
  approximately, emit `matchedOn: { field: "name", value: "", approximate: true }`.
  - An exact name match still emits no `matchedOn`, as today. Keep those tests
    (`catalog_search.rs:690-701`, `:783-790`).
- **D2.** `matchedOnLabel` gets a `"name"` case that returns "approximate match". Update the
  contract comment (`src/services/contracts/assets.ts:100-110`) and the module doc
  (`catalog_search.rs:32-37`).
- **D3.** Update `docs/core/features/catalog-search.md`, acceptance row 5 (`:94`) and the note at
  `:117`.

## Task (one commit)

- **T1.** Make D1 to D3. Cite `DATA-32`.
- Tests:
  - `revnue` finds `Revenue` with `matchedOn.field == "name"` and `approximate`;
  - an exact name match still has no `matchedOn`;
  - the console label, in `palette-search.test.ts`, `command-palette.test.tsx` and
    `data-explorer-columns.test.tsx`.
- *Accept:* `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets -- -D warnings`;
  `cargo test -p lakehouse-api catalog_search`; `bun run typecheck`; `bun run lint`;
  `bun test src/lib src/components/command-palette.test.tsx src/features/catalog`.

## Handoff (developer appends below)

## Review (planner appends below)
