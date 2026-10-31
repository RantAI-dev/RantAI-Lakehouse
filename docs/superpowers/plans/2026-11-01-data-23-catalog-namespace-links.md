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

### Handoff (developer, 2026-10-31 UTC)

Branch `fix/data-23-catalog-namespace-links`, base `main` at `a33e3d81`, the plan commit
`254df171` on top. Console only, as decided (D3): no Rust, no cargo, no API change.

**Commits**

- `a7c07f00` — fix(catalog): the Catalog page links and previews by namespace id (DATA-23 T1).
  D1: the drawer's preview filter (`catalog-page.tsx`), the drawer's button (`catalog-page.tsx`)
  and the row menu's "Browse assets" link (`catalog-namespace-columns.tsx`) now carry the
  namespace **id**. D2: "Copy namespace" copies the id. D3: nothing else changed — the API
  already sends both `id` and `name` (`rust/crates/lakehouse-api/src/routes/catalog.rs:1381-1413`,
  read, not touched).
  Tests: `catalog-page.test.tsx` (new) and `catalog-namespace-columns.test.tsx` (new), 6 tests.
- The commits after `a7c07f00` add only this handoff to the plan file; the branch head is the
  last of them.

**Commands run, with counts** (all in the foreground)

- `bun install --frozen-lockfile` — `node_modules` was missing in the worktree; 739 packages
  installed.
- `bun run typecheck` — clean (no output).
- `bun run lint` — 0 errors, 10 warnings. Every warning is in a file this branch does not touch
  (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx`, `login-page.tsx`, `use-copilot.ts`,
  `use-data-table.ts`, `dashboard-specs.ts`, `uploads.test.ts`, `http.ts`).
- `bun test src/features/catalog src/lib` (the plan's accept check) — **1431 pass, 1 skip,
  0 fail**, 2404 `expect()` calls, 132 files (15.9 s).
- `bun run test` (the whole `src` tree) — **3058 pass, 1 skip, 0 fail**, 8329 `expect()` calls,
  312 files (94.1 s).
- `git status` — clean; nothing generated staged.

The plan's own block (`bun run typecheck && bun run lint && bun test src/features/catalog src/lib`,
then `git status`) was **re-run on this branch's head**, which adds only this plan file to the
code commit above: typecheck clean, lint 0 errors / 10 warnings (the same ones), **1431 pass,
1 skip, 0 fail**, 2405 `expect()` calls, 132 files (15.2 s), `git status` clean. The same tests
and the same results as above; the `expect()` count differs by one between the two runs and no
test file changed.

**The new tests fail without the fix.** With the two source files stashed (`git stash push
src/features/catalog/catalog-page.tsx src/features/catalog/catalog-namespace-columns.tsx`) and only
the new test files in place, all 6 new tests fail (0 pass, 6 fail); with the fix they pass. The
page test's stub answers the preview with the assets of the namespace the request names, so the
drawer's preview comes back empty when the request asks by the display name — the bug as a user
saw it.

**Not verified / skipped**

- No `cargo` command ran and no Rust file changed (console only), so the Rust half of the
  verification block is **not verified** here.
- `python3 ops/lint/check_intra_package_imports.py`, `python3 ops/lint/check_bare_iceberg_count.py`,
  `(cd dagster && python -m pytest dispar_orchestrate -q)` and
  `docker compose --profile '*' config --quiet` were **not run**: this branch touches no Python,
  no Dagster and no compose file (`AGENTS.md`, "Only build what changed").
- `bun run build` was **not run**: it is not in the plan's accept check, and `next build` writes
  `.next/` into the worktree.

**For the reviewer**

- The namespace column's own copy button (`catalog-namespace-columns.tsx:43`, the `Copyable`
  around the displayed name) still copies the **display name**, because that is the value it
  shows. D2 named only the row menu's "Copy namespace", so it was left as it was — say whether
  the two should agree.
- The row menu opens on a pointer press, so the test fires `pointerDown` on the trigger; a bare
  `fireEvent.click` does not open it here (the menu is a radix one; the connectors page's menu,
  whose trigger is a base-ui `Button`, does open on a click).
- The drawer's button is an `<a>` with `role="button"` (base-ui's `Button` with
  `render={<Link … />}`), so the tests find it by role `button` and read its `href`.

## Review (planner appends below)
