# DATA-23: the Catalog page links and previews by namespace id

Backlog `DATA-23` (Task, Data, P2). Reassigned to the planner lane on 2026-11-01. Base: `main` at
`a33e3d81`. Every `file:line` below was read there. Planner: Claude Opus. Developer: another agent
(`AGENTS.md`). If the code disagrees, stop and report it.

## What is wrong (confirmed)

- **Two fields, two values.** `build_namespaces` (`rust/crates/lakehouse-api/src/routes/catalog.rs:1381-1413`)
  sets `id` to the raw namespace (`primer`, `silver`, …) and `name` to its display label
  (`tenant.rs:92-114`). Assets carry the raw id (`catalog.rs:1186,1314,1348`), and the explorer
  filter compares to it (`catalog_query.rs:295`).
- **The console uses `.name` in three places,** so each one matches nothing for a namespace that
  has a label:
  - `src/features/catalog/catalog-page.tsx:197`: "Browse in Data Explorer" and "Browse all N
    assets" (`namespaceAssetsHref(selected.name)`);
  - `src/features/catalog/catalog-namespace-columns.tsx:186`: the row menu's "Browse assets";
  - `src/features/catalog/catalog-page.tsx:43-56`: the drawer preview filters
    `namespace eq selectedName`, so it says "No assets in this namespace yet." Not in the backlog
    row, found while planning.

## Decisions

- **D1.** Use `.id` in all three places.
- **D2.** "Copy namespace" (`catalog-namespace-columns.tsx:197`) copies the **id**, because that
  is what a query or a filter needs. The display name stays on screen.
- **D3.** Console only. No API change.

## Task (one commit)

- **T1.** Make D1 and D2. Cite `DATA-23` at the sites.
- Add tests:
  - in a new `catalog-page.test.tsx`, the drawer's preview filter and both buttons use the id, for
    a namespace whose name differs from its id;
  - in `catalog-namespace-columns.test.tsx`, the row menu href and the copied value are the id.
- *Accept:* `bun run typecheck`, `bun run lint`, and `bun test src/features/catalog src/lib`.

## Verification before handoff

`bun run typecheck && bun run lint && bun test src/features/catalog src/lib`, then `git status`.

## Handoff (developer appends below)

## Review (planner appends below)
