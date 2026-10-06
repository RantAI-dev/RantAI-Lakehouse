# Sample tab: a data preview with an inspector — Implementation Plan

**Status:** asked for by the product owner on 2026-10-05, not started.
Written by the planner (Claude Opus) for a developer agent, under the role
split in `AGENTS.md` on `main`. The planner writes no product code.

**Base:** `feat/connectors`, in `/home/hv/lakehouse`.

**Goal, in the user's words:** give the Sample tab the shape recommended
from the comparison with other lakehouse consoles: the data preview of
Snowflake's Snowsight. A grid that reads like data (types, alignment,
`NULL` said out loud, row numbers), that can be sorted, and a panel beside
it that opens on a cell or a column.

One small API change (a `NULL` cell stays `null`), then console.

---

## 1. What is wrong today

Seen on an 18-column Silver table at 1440 px:

- Five rows by default on a page with room for forty.
- A header is a bare name: nothing says what kind of data is under it.
- A `NULL` cell and an empty text look the same: both are blank. The API
  turns `NULL` into `""` before the console sees it
  (`routes/catalog.rs::governed_sample`, `js_string`), so the console
  could not tell them apart if it tried.
- Numbers are left-aligned, so their sizes cannot be compared down a
  column.
- A long value pushes every column after it off the screen; scrolling
  right loses which row is which.
- Nothing can be done with a cell or a column: no full value, no copy, no
  sorting, no "what is in this column".

What is already right and stays: the rows are the reader's own (masked and
row-filtered, through the same rewrite as a query); the row-count choice;
"Open in Query Studio"; the notice for a stand-in table; the empty,
restricted and failed states.

## 2. What the user sees when this is done

1. **Twenty-five rows on opening**, 50 or 100 on request (the API's cap).
   The five the page already carries show at once while the rest load.
2. **A header that says what the column is**: a type glyph, the name, and
   the type in small print under it.
3. **Cells that read like data**: numbers right-aligned in tabular
   figures; `NULL` written as `NULL` in a quiet tone; an empty text
   written as `(empty)` in the same tone; a long value cut at a fixed
   width with the whole value one press away. A masked value shows as it
   comes (`***`).
4. **Row numbers** in a gutter that stays put when the grid scrolls
   sideways, and from tablet width up the first column stays with it. The
   header stays put when the grid scrolls down.
5. **Sorting**: a control in each header sorts the rows shown by that
   column, ascending, then descending, then back to the table's own
   order. The card says the sort is of the rows shown, not of the table.
6. **An inspector** beside the grid (under it on a narrow screen), closed
   until something is picked:
   - **a cell**: which row and column, the whole value (wrapped, with
     `NULL` or `(empty)` explained in a sentence), and a Copy button;
   - **its column**, or a column picked by its header: the same detail the
     Schema tab opens for a column (most frequent values with bars,
     "Other values", "Null", the facts), read from the table's profile.
     The profile is asked for the first time a column is shown, not when
     the tab opens.
   A close button puts the grid back to full width.
7. **Keyboard**: the grid is one tab stop; arrow keys move the picked
   cell; Escape closes the inspector.

Not included: number formatting (thousands separators, decimals): the
preview shows values as stored, and an id like `0250161` must not be
reformatted; filtering or searching the rows; resizing, hiding or
reordering columns; statistics of a selection of cells; downloading;
anything past 100 rows (Query Studio).

## 3. Decisions

1. **`NULL` is `null` on the wire.** `governed_sample` sends JSON `null`
   for a `NULL` cell and a string for everything else, in both places the
   sample is served (the detail body's five rows and
   `GET /api/catalog/{id}/sample`). The contract becomes `Record<string,
   string | null>[]`. The console is the only reader. Masking is
   unchanged: a masked cell is whatever the rewrite makes it.
2. **Type and alignment come from the asset's schema**
   (`AssetDetail.schema`, by column name) through `typeFamily`
   (`src/lib/column-type.ts`). A column the schema does not list has no
   glyph and no type, and is left-aligned: no guessing from the values.
3. **Sorting is of the rows shown and says so.** Numbers compare as
   numbers when the column's family is a number and both values parse;
   everything else compares as text; `NULL` sorts last in both
   directions. It never asks the API for anything.
4. **The inspector's column statistics are the table's, from the
   profile**, not computed from the rows shown: twenty-five rows are not
   a column. It is the same component the Schema tab opens, so the two
   tabs cannot disagree. The profile needs the same permission as the
   sample and has the same failure states; the inspector shows them in
   words, as the Schema tab's note does.
5. **Selection follows focus.** One cell is the grid's tab stop; arrow
   keys, Home and End move it; the inspector shows whatever is picked.
   A grid of 1,800 buttons in the tab order is not acceptable.
6. **Twenty-five is the default** because it is the smallest size that
   fills the page; the request is one `LIMIT 25`.

## 4. Anchors (verified at `56aae7e` plus the uncommitted work)

- `src/features/catalog/asset-sample.tsx`: the whole tab (117 lines):
  `SIZES`, the `useService` call that keeps the body's rows on screen
  while more load, the card's description, the states.
- `rust/crates/lakehouse-api/src/routes/catalog.rs`: `governed_sample`
  (~1619) builds each row with `Value::String(js_string(row.get(..)))`;
  `js_string` (`routes/support.rs` ~15) turns `Value::Null` into `""`.
  Tests that pin the sample's shape: ~3964 (`{"id": "1", "email":
  "***"}`) and ~4009.
- `src/services/contracts/assets.ts`: `AssetDetail.sample` (~101),
  `getAssetSample` (~401); client `src/services/clients/assets.ts` (~101).
- From the column explorer (uncommitted, in this tree):
  `src/lib/column-type.ts` (`typeFamily`), `src/lib/column-profile.ts`,
  and in `src/features/catalog/asset-columns.tsx` the glyph table
  (`FAMILY`), `ColumnDetail`, `ProfileNote`, `SchemaRow`, `ProfileState`;
  in `asset-schema.tsx`, `schemaRows` and `useProfile`.
- Where the tab is rendered: `asset-detail-tabs.tsx` (~144-148) passes
  `asset` to `AssetSample`, and `asset` plus `iceberg` to `AssetSchema`.
- Tests: `asset-detail-tabs.test.tsx` has the Sample tab's tests (search
  for "Sample").
- Patterns to reuse before writing one: `SectionCard`, `EmptyState`,
  `ErrorState`, `CountToggle`, `Button`, `Tooltip`; how other pages copy
  to the clipboard (`navigator.clipboard.writeText`, e.g.
  `features/governance/policy-columns.tsx`) and report it
  (`src/lib/notify.ts`).

## 5. Tasks

### P1 — `NULL` stays `null` (API, contract, client)

- `governed_sample`: a `NULL` cell is `Value::Null`; every other cell is
  the string it is today. Do not change `js_string` itself: other routes
  rely on it. Say at the site why the sample differs (a reader must be
  able to tell "no value" from "empty text").
- Contract and client: `Record<string, string | null>[]` for
  `AssetDetail.sample` and `getAssetSample`. The mock fixture follows the
  type and nothing more.
- **Accept:** a route test where one cell is `NULL` and one is `""`: the
  body carries `null` and `""`; the masked test still passes unchanged;
  `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets -- -D
  warnings`, `cargo test -p lakehouse-api catalog`.

### P2 — The arithmetic (`src/lib`)

- `src/lib/sample-grid.ts`, pure:
  - `cellKind(value)`: `"null" | "empty" | "value"`.
  - `sortRows(rows, column, direction, family)`: a new array, decision 3;
    stable for equal values, so the table's order breaks ties.
  - `nextSort(current, column)`: none → ascending → descending → none; a
    different column starts at ascending.
- **Accept:** tests beside it: numbers as numbers (`"10"` after `"9"`),
  text as text, a number column with a value that does not parse, `NULL`
  last both ways, ties keeping the table's order, the input not mutated,
  the three-step cycle and the change of column.

### P3 — The grid and the inspector (console)

- `asset-sample.tsx` keeps the tab (sizes, loading, states, the card) and
  renders a new `src/features/catalog/sample-grid.tsx` (the grid) and
  `sample-inspector.tsx` (the panel). Split further if a file passes
  about 300 lines.
- **Sizes**: 25, 50, 100; 25 on opening. The body's rows show until the
  first answer; if the deployment cannot fetch more
  (`getAssetSample` absent), the body's rows are the sample and the size
  control does not show.
- **Grid**: a real `<table>` with `role="grid"` in a scroller of its own
  (both ways, a height cap so the page keeps its header and tabs in
  view), the header row sticky at the top, the row-number gutter sticky
  at the left at every width, the first data column sticky beside it from
  `md` up. Header cell: glyph, name (mono), type under it, and the sort
  control (`aria-sort` on the header cell; the control names what
  pressing it will do). Body cell: decision 2 and section 2 item 3; cut
  at a fixed maximum width with the whole value as the title.
- **Picking**: decision 5. A pressed or focused cell is the picked cell
  and is marked; pressing a header's name picks the column. The picked
  column's cells carry a faint tint so the eye can run down it.
- **Inspector**: section 2 item 6. Beside the grid from `lg` up (a fixed
  width; the grid takes the rest), under it below `lg`. For the column
  part, reuse the Schema tab's opened column: export what is needed from
  `asset-columns.tsx`, and move `useProfile` and `schemaRows` out of
  `asset-schema.tsx` into a module both tabs import if that is what it
  takes (move, do not copy; `AssetSample` then also takes the page's
  `iceberg` state so its facts are the Schema tab's facts). The profile
  is requested only once a column is shown in the inspector.
- **The card's description** says how many rows, that masking and row
  filters apply, and, when sorted, "sorted by <column>, among the rows
  shown".
- **Copy** uses the app's existing way to copy and to say it did.
- **Accept (component tests, a new `asset-sample.test.tsx` or the
  existing Sample block):**
  - 25 is asked for on opening and the body's rows show meanwhile; 50 and
    100 ask again; without `getAssetSample` there is no size control;
  - a header shows glyph, name and type; a column the schema does not
    list shows the name only;
  - `null` reads `NULL`, `""` reads `(empty)`, a number cell is
    right-aligned, a masked `***` is shown as is;
  - row numbers start at 1 and follow the sort (they number the rows as
    shown);
  - pressing the sort control sorts ascending, then descending, then
    restores the order; `aria-sort` follows; the description says so;
  - pressing a cell opens the inspector with row, column, whole value and
    Copy; a `NULL` and an empty cell are explained; Copy copies the value
    (and is not offered for `NULL`);
  - pressing a header's name opens the inspector on the column and asks
    for the profile then, not before; the profile's loading, restricted,
    unsupported and failed states each read in words;
  - arrow keys move the picked cell, the grid has one tab stop, Escape
    closes the inspector, the close button does too;
  - the restricted, empty and failed states of the sample read as they
    did. An existing Sample test that changes is listed in the handoff
    with what replaced each assertion.

### P4 — Documents

- `CHANGELOG.md` `[Unreleased]`: one entry (its own bullet).
- `docs/FEATURE_COVERAGE.md` if its asset-detail row describes the sample.
- **Accept:** each sentence names only what exists.

## 6. Working on this machine

- Work in `/home/hv/lakehouse` on `feat/connectors`. It is the product
  owner's checkout: their console dev server (port 3000) reloads every
  saved file under `src/`. Never kill or restart it, never run `next
  build` or `bun install` here, and do not leave `src/` in a state that
  does not compile.
- **The tree already holds two uncommitted changes** waiting for the
  product owner: schema versions for Silver and Gold (Rust files
  including hunks in `routes/catalog.rs`, the `SchemaVersions` card,
  `asset-activity.tsx`, `asset-overview.tsx`, `asset-about.tsx`, the
  `schemaVersions` contract field, the mock, tests, documents) and the
  Schema tab's column explorer (`asset-columns.tsx`, `src/lib/column-*`,
  hunks in `asset-schema.tsx` and the tests, a CHANGELOG bullet). Leave
  their hunks as they are, except the moves this plan names. Your
  CHANGELOG entry is a separate bullet.
- **Leave the work uncommitted.** No `git add`, commit, stash, reset,
  checkout, restore or branch switch. Never push.
- Untracked and not yours: `docs/plans/FEAT-CONNECTORS-REPORT.md`,
  `lark-import/`, `ops/g3/bronze_catalog.py`. Never read or print `.env`.
- Rust: `cd rust && export
  CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-catalog-target
  CARGO_BUILD_JOBS=4`. `df -h /` before a build; stop and report under
  15 GB free. No `cargo clean`, no `--release`, no rebuild loops.
  `rustfmt` only a file whose every hunk is yours (so not `catalog.rs`:
  format your lines by hand and confirm with `cargo fmt --check`).
- No `docker` commands. The reviewer builds and restarts the API. Until
  then the running API still sends `""` for `NULL`, so on port 3000 a
  `NULL` cell will read `(empty)`: expected, and not yours to work
  around.
- No prettier: match the file's style by hand.
- Before the handoff, once, in the foreground: `cargo fmt --check &&
  cargo clippy --workspace --all-targets --all-features -- -D warnings &&
  cargo test -p lakehouse-api`; `bun run typecheck && bun run lint && bun
  run test`.

## 7. Acceptance checklist (product owner)

On a Silver table with many columns (Data Explorer, Sample tab):

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Open the tab | 25 rows; headers with glyph, name, type; row numbers | |
| 2 | Look at a column with gaps | `NULL` written out, quiet; an empty text reads `(empty)` | |
| 3 | Look at a number column | Right-aligned, figures in line | |
| 4 | Scroll the grid right and down | Row numbers and the first column stay; the header stays | |
| 5 | Press a header's sort control three times | Ascending, descending, the table's order; the card says the sort is of the rows shown | |
| 6 | Press a cell with a long value | The inspector opens beside the grid: row, column, whole value, Copy | |
| 7 | Press Copy | The value is on the clipboard and the page says so | |
| 8 | Press a header's name | The inspector shows the column: frequent values with bars, nulls, distinct, range | |
| 9 | Use the arrow keys, then Escape | The picked cell moves; Escape closes the inspector | |
| 10 | Choose 100 rows | 100 rows, the grid scrolls inside its own frame | |
| 11 | Narrow the window to a phone's width | The grid scrolls inside its frame, the page does not; the inspector opens under the grid | |

## 8. Handoff (developer)

Uncommitted. The first part (P1, P2, the grid, the inspector, the moves, the tests in `asset-detail-tabs.test.tsx`) was written before a session interruption; the CHANGELOG bullet, the lint fix in the tests and every verification run below were done after it. I could not look at a browser: layout is reasoned from the CSS, not seen.

**Files.** Rust: `routes/catalog.rs` (`governed_sample`: `NULL` is `Value::Null`; test `sample_keeps_a_null_cell_null_and_an_empty_text_empty`). Console: `src/lib/sample-grid.ts` + `.test.ts` (new); `src/features/catalog/sample-grid.tsx`, `sample-inspector.tsx`, `asset-column-data.ts` (new: `schemaRows` and `useProfile`, moved out of `asset-schema.tsx`); `asset-sample.tsx` (rewritten), `asset-detail-tabs.tsx` (passes `iceberg`), `asset-schema.tsx` (the move only), `asset-columns.tsx` (exports `FAMILY`, `EMPTY_VALUE`, `ProfileNote`, `ColumnDetail`); contract and client `string | null`; `CHANGELOG.md` (own bullet); tests in `asset-detail-tabs.test.tsx`. The mock fixture needed no change (it already satisfies the type). `docs/FEATURE_COVERAGE.md` does not mention the sample, so it is untouched.

**Checks.** `cargo fmt --check` 0; `cargo clippy --workspace --all-targets --all-features -- -D warnings` 0; `cargo test -p lakehouse-api` at default parallelism: 1116 passed, 69 failed, all "failed to connect to setup test database: ConnectionReset" (the shared test Postgres), twice; with `-- --test-threads=4`: lib 1185 passed (second lib target 1230), 0 failed, 1 ignored, all other binaries 0 failed. `bun run typecheck` 0; `bun run lint` 0 errors, 5 warnings (none in my files; one of mine in the test was fixed); `bun run test` 716 pass, 0 fail, 717 tests in 80 files (run before a final test-only lint fix, after which typecheck, eslint on that file and the affected test were re-run: pass).

**Departures.** (1) The inspector sits beside the grid from `xl` (1280), not `lg`: at 1024 the card body is about 42.6rem, and a 22rem panel would leave about 20rem of grid. (2) The first data column is sticky by the grid's own width (`@lg`, 32rem container query), not the viewport's `md`: at 768 the card body is about 27rem. (3) Focus alone does not open the inspector: a press, Enter or Space picks; once something is picked, moving by keyboard follows. Tabbing through the page therefore neither shifts the layout nor requests the profile. (4) In a number column values that do not parse sort after the numbers, as text, in both directions mirrored (pairwise "number if both parse, else text" is not transitive: 9 < 10 < 1e < 9). Integers compare exactly (BigInt) so 64-bit ids keep their order. (5) The header's controls are a roving tab stop too (the active column's name and sort control). (6) A cell missing from a row, like `NULL`, is `null` on the wire. (7) `ColumnDetail`'s facts are forced to one column inside the 22rem panel by a CSS override at the call site; `asset-columns.tsx` itself got only `export`s. Plan inaccuracy: none found beyond the `lg` width.

**Existing tests changed.** "shows the rows the asset carries, and asks for more only when asked" became four tests: sizes `["5","25","50","100"]` with 5 pressed -> `["25","50","100"]` with 25 pressed; "2 rows, no /sample call" -> body's row shown with "Loading 25 rows" and `limit=25` asked on opening; 26 rows after pressing 25 -> 26 rows after the answer; the `limit=25` URL check kept; 50 and 100 added. "offers no row count..." kept and gained "no sample request". New: no `getAssetSample` means no size control; empty and failed sample states. The two `findByText("Sample rows")` waits are unchanged. `stubApi` gained an optional `sample` answer.

**Layout.** One scroller (`max-h-[70vh]`, both ways) inside a frame with `@container`; a bare `<table role="grid">` because the shared `Table` wrapper has no height, so a sticky header would not stick to the grid. Sticky: header row (top, z-20), gutter (3rem, left, z-10, corner z-30), first column from `@lg` (left 3rem). Every cell paints an opaque background from `--card`/`--muted`; tints are `color-mix` over them; `border-separate` so sticky borders travel. Cells cut at 16rem (first column 12rem), whole value as title above 24 characters, and in the inspector; a 60-character value shows about 35 characters then an ellipsis. Card body widths: 1440 about 68.5rem, 1280 about 58.5rem, 1024 about 42.6rem, 900 about 35rem, 390 about 21rem. Inspector open: grid about 733px at 1440, about 573px at 1280 (22rem panel plus 0.75rem gap); at 1024 and below the panel is under the grid at full width and scrolls into view on opening. Look at: 1280 with the inspector open, the sticky first column over a narrow frame, tint contrast in dark, the header at two lines, ring colour on a picked cell.

**Review round 1 (after screenshots).** S1: the grid frame is `isolate`, so its sticky z-indexes stay under the page's tab bar (the inspector has no z-index). S2: the row-count control and "Open in Query Studio" moved from the card's header action into the first line of the card body at every width (wrapping), so the title and description keep the card's width. S3: if the larger sample fails and rows exist, the grid stays with a `role=alert` line above it ("The larger sample could not be loaded. Showing the N rows the page already has.", Retry; no upstream text); only with no rows does `ErrorState` stand alone. S4: Copy is offered for a value only (not `NULL`, not an empty text). This corrects the earlier Copy-for-empty and error-replaces-grid statements. No Rust changed; console typecheck 0, lint 0 errors/5 warnings, `bun run test` 719 pass, 0 fail (720 tests, 80 files).

**Unsure.** Row hover is the opaque `--muted`, slightly stronger than other tables.

## 9. Review (planner)
