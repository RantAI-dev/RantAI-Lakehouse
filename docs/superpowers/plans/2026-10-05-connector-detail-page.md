# A page for one connector — Implementation Plan

**Status:** asked for by the product owner on 2026-10-05, not started.
Written by the planner (Claude Opus) for a developer agent, under the role
split in `AGENTS.md` on `main`. The planner writes no product code.

**Base:** `dc63925` on `feat/upload-file`. **Branch:**
`feat/connector-detail-page`, stacked on it, in the worktree
`/home/hv/lakehouse-upload`. Stacked because `connectors-page.tsx` was
reshaped there (the "Connectors" and "Uploaded files" tabs), and so the
product owner sees this on the trial console without a second one.

**Goal, in the user's words:** on Sources, when a user presses one of the
connectors in the list, open a new page for its detail view instead of a
sheet at the side.

**Backlog:** no ID yet. `docs/plans/FEAT-CONNECTORS-REPORT.md` lists it as
`FC-35`; `docs/FEATURE_COVERAGE.md` has had "Dedicated connector detail
route: [PARTIAL] (drawer sufficient for preview)" since the mock period.
This file carries what a feature page would (what the user can do, the
checklist), because the change is one screen and no API.

Console only. No route, store, migration, orchestrator or compose change.

---

## 1. What the user can do when this is done

1. Press a connector in the Sources list and land on its own page,
   `/connectors/<id>`, with an address that can be bookmarked, reloaded and
   sent to someone.
2. See there what the side sheet showed: status, direction and environment;
   the actions Test connection, Edit, Create pipeline, Audit, Delete; and
   the tabs Overview, Ingest and Connection tests.
3. Open a tab directly by address (`?tab=ingest`, `?tab=tests`), and keep
   the tab on reload.
4. Go back to Sources from the page.
5. After deleting the connector, be taken back to Sources.
6. Reach the page from the places that point at a connector: the row menu's
   "View details", the edit page (Cancel, and after saving), the page shown
   after creating a connector, and a connector node in a lineage map.

Not included: any change to what the three tabs contain or do; any new
action; the "Uploaded files" tab.

## 2. Decisions (made by the planner; small enough not to ask)

| Decision | Choice |
| --- | --- |
| Address | `/connectors/[id]`, beside the existing `/connectors/[id]/edit` |
| The side sheet | Removed from the Sources list. The `DetailDrawer` pattern itself stays; other pages use it |
| What opens the page | The connector's name is a real link (keyboard, open in new tab). A press anywhere on the row opens it too, the way the Data Explorer list opens an asset (`onRowClick`), except on the row's own controls |
| Tabs | Overview, Ingest, Connection tests, in the address as `?tab=`; Overview is the default and leaves the address clean. Kept the way `asset-detail-tabs.tsx` keeps its tab |
| Header | `EntityHeader`: eyebrow link "Sources" back to `/connectors`; the connector's name; its type as the description; health, direction and environment beside the name; the actions on the right |
| Width | The Overview's four sections may sit in two columns at large widths. Nothing else in the tabs is laid out again |
| Unknown or another tenant's connector | The existing error state with the API's sentence, and a link back to Sources. Never a blank page |
| Lineage | A connector node links to the detail page, not to the edit page |

## 3. Anchors (verified at `dc63925`)

Line numbers drift; re-find by symbol.

- `src/features/connectors/connectors-page.tsx`: `ConnectorDetail` (~42,
  the sheet's body: badges, actions, test result, the three tabs),
  `ConnectorsTab` (~169, the list with `selected` state and
  `<DetailDrawer>` ~240), `ConnectorsPage` (~271, the two page tabs).
- `src/features/connectors/connectors-columns.tsx`: `getConnectorColumns({
  onSelect })`; the name cell is a `<button>` (~55); the row menu's "View
  details" calls `onSelect` (~175).
- `src/features/connectors/connector-overview.tsx`,
  `connector-ingest-panel.tsx`, `connector-probe-history-panel.tsx`,
  `connector-delete-dialog.tsx`: the parts the page reuses unchanged.
- `src/features/connectors/connector-edit-page.tsx`: links to
  `/connectors` at ~308 (after saving) and ~526 (Cancel);
  `onDeleted={() => router.push("/connectors")}` ~514.
- `src/features/connectors/connector-create-page.tsx`: "View connectors"
  ~220 and ~266.
- `src/features/governance/lineage-graph.tsx`: `nodeHref`, `case
  "connector"` ~59, returns `/connectors/<id>/edit`. Pinned by
  `src/features/catalog/asset-detail-tabs.test.tsx` ~299.
- Patterns to follow: `EntityHeader` (`src/components/patterns/
  page-header.tsx` ~52); the tab-in-address code of
  `src/features/catalog/asset-detail-tabs.tsx` (~122-168); `onRowClick` in
  `src/features/catalog/data-explorer-page.tsx` (~279); a thin route file
  such as `src/app/(data)/connectors/[id]/edit/page.tsx`.
- Tests today: `src/features/connectors/connectors-page.test.tsx`.

## 4. Tasks

One task per commit. Console checks only (`bun run typecheck`, `bun run
lint`, the tests touched); the full console block (`bun run typecheck && bun run lint && bun run
test`) once before the handoff.

### T1 — The page

- `src/features/connectors/connector-detail-page.tsx`
  (`ConnectorDetailPage`, `"use client"` first, imports from `@/services`
  only): move `ConnectorDetail`'s body here and give it the header and the
  tabs of section 2. Keep every behaviour it has: the test action reloads
  the detail and refreshes the overview and the test history; Delete is
  disabled with its reason while pipelines use the connector; the Ingest
  tab stays mounted so unsaved table picks survive a look at another tab.
- `src/app/(data)/connectors/[id]/page.tsx`: a thin default export.
- Deleting from the page goes to `/connectors`.
- `connector-overview.tsx`: the two-column layout at large widths, and its
  `onOpenTab` now changes the page's tab.
- Tests (`bun:test`, as the other connector component tests): loading;
  the error state with the way back for a refused read; the three tabs and
  `?tab=`; the test action's result line; Delete disabled while in use;
  delete goes to `/connectors`; the back link.
- **Accept:** the tests; `bun run typecheck` passes with the new route.

### T2 — The list opens the page; the sheet goes

- `connectors-columns.tsx`: the name is a `Link` to `/connectors/<id>`;
  "View details" is a link too; `onSelect` goes.
- `connectors-page.tsx`: remove `selected`, `ConnectorDetail` and the
  `DetailDrawer`; a press on a row opens the page, as Data Explorer does.
  The page tabs and the "Uploaded files" tab are untouched.
- Update `connectors-page.test.tsx`: no sheet; the name's `href`; the row
  press.
- **Accept:** the tests; nothing in `src/features/connectors/` imports
  `DetailDrawer`.

### T3 — What points at a connector points at its page

- `connector-edit-page.tsx`: Cancel and the link after saving go to
  `/connectors/<id>`; deleting from the edit page still goes to
  `/connectors`.
- `connector-create-page.tsx`: after creation, add "Open connector" to
  `/connectors/<id>` beside "View connectors".
- `lineage-graph.tsx` `nodeHref`: a connector goes to `/connectors/<id>`.
  Update the test that pins the old address.
- `docs/FEATURE_COVERAGE.md`: the "Dedicated connector detail route" row
  and the Connectors row. `CHANGELOG.md` `[Unreleased]`: one entry.
- **Accept:** the tests touched; `grep -rn "connectors/.*edit" src` shows
  only links that mean "edit".

## 5. Out of scope

- The tabs' contents and actions.
- The Sources page title ("Connectors") and its Sink / Bidirectional
  wording (`FC-33`).
- Any API change.

## 6. Working on this machine

- Work only in `/home/hv/lakehouse-upload`, on `feat/connector-detail-page`.
  Never edit or run anything in `/home/hv/lakehouse` or
  `/home/hv/lakehouse-uiux`. Never push.
- A console dev server from this worktree is running on port 3200 for the
  product owner's QA (`next dev`). Leave it running; do not start another
  and do not kill it. Do not run `next build` in this worktree while it
  runs (both write under `.next/`); write the build down as *not verified*
  and the reviewer decides how to check it.
- No prettier; match each file's style by hand. No Rust, no Python.
- Commits: one per task, Conventional Commits, English, the task id in the
  subject, ending with the developer model's own `Co-Authored-By` line.

## 7. Acceptance checklist (product owner, on the trial console)

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Open Sources, press a connector's name | Its page opens at `/connectors/<id>`; no side sheet | |
| 2 | Press elsewhere on another row | The same | |
| 3 | Reload the page | The same connector, the same tab | |
| 4 | Open the Ingest tab, then Connection tests | The address gains `?tab=ingest`, `?tab=tests`; the content is what the sheet showed | |
| 5 | Press "Test connection" | A result line; the Connection tests tab lists it | |
| 6 | Press "Edit", then Cancel | Back on the connector's page | |
| 7 | Press "Sources" above the name | Back on the list | |
| 8 | Open `/connectors/does-not-exist` | A message that it was not found, and a way back | |
| 9 | From an asset's Lineage map, press a connector node | The connector's page, not the edit form | |
| 10 | Delete a connector nothing uses, from its page | Back on Sources; the connector is gone | |

## 8. Handoff (developer)

Written by the developer agent (Claude Sonnet 5.5), 2026-10-05. Base
`e452e47`; the handoff itself is the commit after these three.

### Commits

| Task | Commit | Subject |
| --- | --- | --- |
| T1 | `733a5ae` | `feat(connectors): T1 a page for one connector at /connectors/<id>` |
| T2 | `fad7b77` | `feat(connectors): T2 the Sources list opens the connector's page, not a sheet` |
| T3 | `9cfcee0` | `feat(connectors): T3 what points at a connector points at its page` |

### The page as built

Address `/connectors/<id>`; the route file
`src/app/(data)/connectors/[id]/page.tsx` reads `id` with `useParams` the
way the edit route does and renders `ConnectorDetailPage`
(`src/features/connectors/connector-detail-page.tsx`).

- **Loading:** the link "Sources" (to `/connectors`) above the shared
  loading skeleton.
- **Error** (unknown id, another tenant's, or any refused read): the link
  "Sources" above the shared `ErrorState`, which shows its title (for a 404
  "Not found"), the API's own sentence plus the shared hint, and "Retry".
  There are no tabs and no actions. The link sits above the state, not in
  it, because `ErrorState` has no slot but Retry.
- **Loaded:** `EntityHeader`. Eyebrow: "Sources" (link to `/connectors`).
  Title: the connector's name. Beside it: the health badge, a direction pill
  (Source, Sink, Bidirectional) and, when set, an environment pill.
  Description: the connector's type. Actions on the right: "Test
  connection" ("Testing…" while it runs), "Edit" (to
  `/connectors/<id>/edit`), "Create pipeline" (to
  `/pipelines/create?connectorId=<id>`), "Audit" (only when the connector has
  an audit event) and "Delete" (disabled with the title "Used by N
  pipeline(s) (see Used by); those must be deleted or moved first" while
  pipelines use it; otherwise it opens the delete dialog, and a delete goes
  to `/connectors`).
- **Under the header:** the test's result line (the message and the latency,
  or "Not testable · <message>"), then the tabs "Overview", "Ingest" and
  "Connection tests". The open tab is in `?tab=ingest` / `?tab=tests`;
  Overview is the default and leaves the address clean; an unknown `?tab=`
  falls back to Overview. The Ingest tab stays mounted. The Overview's
  "Manage", "Test history" and "Runs" buttons change the page's tab.
- **Overview layout:** one column, and from the `xl` breakpoint (1280 px) the
  four sections (Details, Connection, Ingest, Used by) in two columns;
  "Needs attention", when shown, spans both.
- **List (T2):** the name is a link to `/connectors/<id>`; the row menu has
  "View details" as a link and "Create pipeline"; a press anywhere else on a
  row opens the page; there is no side sheet.
- **T3:** the edit page's "Cancel" (every state) and the link after saving go
  to `/connectors/<id>`, the latter now labelled "Back to connector"; deleting
  from the edit page goes to `/connectors`; the create page's result card has
  "Open connector" beside "View connectors"; a connector node in a lineage
  map opens `/connectors/<id>`.

### Commands run (all from `/home/hv/lakehouse-upload`, foreground)

On the final commit (`9cfcee0`):

- `bun run typecheck`: passes, no output.
- `bun run lint`: 0 errors, 5 warnings, all in files this work does not touch
  (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx`, `use-data-table.ts`,
  `dashboard-specs.ts`), the same five as the base.
- `bun run test`: 520 pass, 0 fail, 73 files (before this work: 501 in 72).
  The 19 added: 14 in `connector-detail-page.test.tsx`, 4 in
  `connectors-page.test.tsx` (the list), 1 in `connector-edit-page.test.tsx`
  (Cancel). New assertions went into existing tests of the edit page (the
  link after saving, the push after a forced delete), the create page ("Open
  connector") and `asset-detail-tabs.test.tsx` (the lineage address).
- Per commit: `bun run typecheck`, `bun run lint` (same 5 warnings) and the
  test files touched, each passing (T1: 14; T2: `src/features/connectors`,
  100; T3: edit, create and asset-detail-tabs, 79).
- Mutation checks, each restored afterwards: without `keepMounted` the
  "Ingest tab mounted" test fails; with `keepDataOnReload: false` the
  "reloads the detail" test fails; removing any one of the three
  `stopPropagation` calls in `connectors-columns.tsx` fails its list test.
- `curl` on the dev server on port 3200: `/connectors/some-id`,
  `/connectors` and `/connectors/some-id/edit` each answer 200, and the
  shell has no build-error text. That is the page shell only.
- `git status` is clean; nothing generated is staged.

### Not verified, or skipped

- **`next build`: not verified**, as the plan says: it would write under
  `.next/` while the dev server runs.
- **Not seen in a browser, signed in.** The page, its layout (the two
  columns at `xl`, the header wrapping on a narrow screen) and every
  acceptance row in section 7 were checked only through the tests above,
  against stubbed responses. The checklist's Result column is untouched.
- What the API answers for another tenant's connector (404 or 403) was not
  read in the Rust code (no Rust here). The connector client turns a 404 into
  "Not found" and any other 4xx into "Request could not be processed" with the
  API's sentence; the page shows either with the way back.
- Browser behaviours the tests cannot show: a ctrl-click on the name opening a
  new tab without also navigating this one (the `stopPropagation` keeps the
  row from also pushing; the browser does the rest), and the real History API
  keeping `?tab=` across a reload (the tests stub `replaceState`).

### Where the plan and the code differed, and choices made

1. **The error state has no slot for a link.** The plan says "the existing
   error state ... and a link back to Sources". `ErrorState` accepts only
   Retry, so the link is rendered above it.
2. **`keepDataOnReload` on the detail read (not in the plan).** The sheet
   reloaded the detail after a test and showed a skeleton for a moment. On a
   whole page that would blank the header and, because the skeleton replaces
   everything, unmount the Ingest tab and drop the table picks it is kept
   mounted to protect. The page reloads with `keepDataOnReload`; the test
   action still reloads the detail and refreshes the overview and the history.
3. **"Open connector" is beside the card's "View connectors", not the
   header's.** The create page shows "View connectors" twice (header and
   card); the plan did not say which. The header keeps its one button.
4. **The edit page's post-save link was labelled "Back to connectors"**; going
   to the connector's page made that wrong, so it reads "Back to connector".
5. **`FEATURE_COVERAGE.md`: the "Dedicated connector detail route" row is
   removed** from the "Intentionally missing / still mocked" table (it is no
   longer missing or partial) and the Connectors row names the page and its
   route. The row's own `testConnection` backend note was left as it was.
6. **The name's focus style** changed from none (`focus:outline-none`) to an
   underline on keyboard focus, since the name is now the keyboard route to
   the page.
7. **The two-column breakpoint is `xl`**, not `lg`: with the sidebar open the
   content is about 1000 px wide at a 1280 px window, and two columns of label
   and value pairs would be cramped below that.
8. **The plan's `onOpenTab` line:** the Overview's `onOpenTab` was already a
   callback, so the only change in `connector-overview.tsx` besides the grid
   is its doc comment; the page passes its tab setter.

### Seen and not changed

- The "Connection test passed" toast (`withNotify` around `testConnection`)
  fires for any answer with an HTTP success, including a probe that returned
  `ok: false`; the result line under the header is the accurate one. This is
  how the sheet behaved; the plan keeps the actions as they are.
- `Button render={<Link />}` renders an anchor with `role="button"`, so the
  header's Edit, Create pipeline and Audit are announced as buttons although
  they navigate. Shared component behaviour, as before.
- `ErrorState`'s shared hint for a 404 reads "It may already have been
  deleted. Reload the list.", written for lists; on this page it is slightly
  off. It lives in `src/lib/notify-message.ts` and is used everywhere.

## 9. Review (planner)
