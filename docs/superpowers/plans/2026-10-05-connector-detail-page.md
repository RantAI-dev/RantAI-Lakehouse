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

## 9. Review (planner)
