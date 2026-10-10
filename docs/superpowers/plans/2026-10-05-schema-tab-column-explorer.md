# Schema tab: columns as a column explorer — Implementation Plan

**Status:** asked for by the product owner on 2026-10-05 ("try it"), not
started. Written by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md` on `main`. The planner writes no product code.

**Base:** `feat/connectors`, in `/home/hv/lakehouse`.

**Goal, in the user's words:** the columns table on the Schema tab should
take the shape recommended from the comparison with other lakehouse
consoles: the column explorer of MotherDuck. One compact row per column
with a small picture of its values, and the detail of a column on demand.

Console only. No API change, no contract change.

---

## 1. What is wrong today

Seen on an 18-column Silver table at 1440 px:

- Rows are of uneven height: the "Top values" chips wrap to a second line
  for some columns and not others, so the eye cannot run down a column of
  the table.
- Five chips of raw text per row is the loudest thing on the page and says
  the least: which value is common, and how common, has to be read chip by
  chip.
- "Nullable" repeats what the type already says for an engine table
  (`Nullable(String)`).
- There is no way to look at one column closely.
- The filter box only appears past 25 columns.
- At phone width the table scrolls sideways.

What is already right and stays: the statistics themselves (nulls,
distinct, range, top values), the masked / classification / partition
marks, descriptions, the filter and the page sizes for wide tables, the
note under the table saying what was profiled.

## 2. What the user sees when this is done

1. **One line per column**, every row the same height: a type glyph, the
   name with its marks, the type, a **value bar**, the null share, the
   distinct count, and at wide widths the range.
2. **The value bar** is the row's picture of the column: one segment per
   most-frequent value, as wide as its share of the rows profiled; what is
   left is the plain track. Beside it, the commonest value and its share
   ("8250 · 100%"). A column with no listed values reads "Mostly unique",
   as today.
3. **Pressing a row opens the column** in place, under the row: every
   listed value with its own bar, count and share, then "other values" and
   "null" as their own lines; and the column's facts (type, whether it can
   be null, nulls, distinct, range, description, what it is a key of).
   Pressing again closes it. Several can be open.
4. **Keys are marked**: a column the table is sorted by carries a "sort
   key" mark, as a partition column already carries its own.
5. **The filter box** is there from 11 columns up, not 26.
6. **No sideways scroll at phone width**: the row sheds what the opened
   column still shows (range, then distinct, then the label beside the
   bar; the type moves under the name).

Not included: a histogram for numbers and dates, or a time line for
timestamps (the profile route gives a range and the most frequent values,
not a distribution; that is an API change of its own); sorting the list;
editing a description here; any change to the Sample tab, the "System
columns" card or the "Schema versions" card.

## 3. Decisions

1. **Shares are of the rows profiled** (`AssetProfile.rowsProfiled`), the
   number the note under the table already names. A top value's share is
   `count / rowsProfiled`; the null share is `nullFraction`; "other" is
   what is left, never below zero. With no rows profiled there is no bar.
2. **Only what the profile states is drawn.** The route lists at most five
   values and only those whose count is exact
   (`catalog_profile.rs::exact_top_values`). The bar draws those and
   nothing else: no guessed segment for "other", which is the bare track.
3. **One hue, stepped.** Segments are one colour at stepped strengths,
   commonest strongest, on the muted track. A segment's identity is its
   tooltip and its line in the opened column, which uses the same step.
   No palette of five colours: a colour would read as a category that
   means something across columns.
4. **The type is shown as the engine states it.** The separate "Nullable"
   column goes; whether the column can be null (yes / no / not known, as
   `schemaRows` already works out) is a fact in the opened column. The
   null share stays in the row: it is what the data does.
5. **Opening is in place**, not a side sheet: the row stays in its list,
   it works at phone width, and two columns can be compared.
6. **A sort key is marked only when the key names the column**: the
   storage card's `sortingKey` split on commas, each part trimmed, and a
   part that is exactly a column's name marks it. An expression
   (`toYYYYMM(d)`) marks nothing; the Storage card on Overview still
   shows the whole key.

## 4. Anchors (verified at `c5f9873` plus the uncommitted work)

- `src/features/catalog/asset-schema.tsx`: `AssetSchema` (the tab),
  `schemaRows`, `visibleColumns`, `useProfile` / `ProfileState`,
  `ProfileNote`, `NullMeter`, `StatCells` and `ColumnCells` (the cells to
  replace), `COLUMN_PAGE_SIZES`, the "System columns" card (keeps its
  three plain columns), `SchemaVersions` (**not yours: it carries another
  change waiting for the product owner; do not edit it**).
- `src/services/contracts/assets.ts`: `AssetColumn`, `ColumnProfile`
  (`nullCount`, `nullFraction`, `distinctCount`, `min`, `max`,
  `topValues`), `AssetProfile` (`rowsProfiled`, `sampled`), and
  `AssetDetail.storage.sortingKey` (~243).
- `src/features/catalog/asset-detail-tabs.test.tsx`: `describe("Schema
  tab")` (~369) and `describe("Schema tab: a wide table")` (~1127).
- Patterns to reuse before writing one: `Pill`, `ClassificationBadge`
  (`components/patterns/status-badge`), `SectionCard`, `MetadataList`
  (`components/patterns/metadata-list`), `Tooltip` (`components/ui`),
  `formatPercent`, `formatNumber`, `formatCompactNumber` (`lib/format`).

## 5. Tasks

### C1 — The arithmetic and the type families (`src/lib`)

- `src/lib/column-profile.ts`, pure:
  - `valueShares(column, rowsProfiled)` gives the listed values in the
    profile's order, each `{ value, count, share }`, then `otherShare` and
    `nullShare`. Shares are 0 to 1, `otherShare` never negative, and the
    whole never above 1 (a count above the rows profiled is clamped, not
    drawn past the track). No rows profiled, or a column not profiled,
    gives `null`.
  - `sortKeyColumns(sortingKey, columnNames)` gives the set of column
    names the key names exactly (decision 6).
- `src/lib/column-type.ts`, pure: `typeFamily(dataType)` gives `"text" |
  "number" | "time" | "boolean" | "nested" | "other"` for both spellings
  the catalog shows: the engine's (`Nullable(...)`, `LowCardinality(...)`
  unwrapped; `String`, `FixedString(n)`, `UUID`, `Enum8(...)`, `Int*`,
  `UInt*`, `Float*`, `Decimal(...)`, `Date`, `Date32`, `DateTime`,
  `DateTime64(...)`, `Bool`, `Array(...)`, `Map(...)`, `Tuple(...)`,
  `JSON`) and Iceberg's (`string`, `uuid`, `int`, `long`, `float`,
  `double`, `decimal(p, s)`, `date`, `time`, `timestamp`, `timestamptz`,
  `boolean`, `list<...>`, `map<...>`, `struct<...>`, `binary`). Anything
  else is `"other"`, never a guess.
- **Accept:** tests beside each file, in the style of the neighbours in
  `src/lib`: shares that add up, the clamp, zero rows, a not-profiled
  column, an empty value; a key of one column, of two, with spaces, an
  expression, an empty and a `null` key; each family in both spellings,
  the wrappers, an unknown type.

### C2 — The rows and the opened column (console)

- New `src/features/catalog/asset-columns.tsx` holds the "Columns" card:
  the list, a row, the opened column. `asset-schema.tsx` keeps the tab,
  the profile hook, "System columns" and "Schema versions", and renders
  the new card. Move, do not copy: `NullMeter`, `ProfileNote` and the
  filter / page-size controls go with the card; `StatCells` and the
  statistic half of `ColumnCells` are replaced.
- **A row**, one line, the same height whatever the column:
  type glyph (by `typeFamily`, with the family as its accessible name) ·
  name (mono) with the marks it has today plus "sort key" · type (mono,
  muted, truncated with the whole type as its title) · value bar with the
  commonest value and its share beside it · null share (the meter and the
  number, as today) · distinct (`≈`, as today) · range (truncated, whole
  range as its title) · a chevron that turns when open.
  The description no longer sits under the name (it made rows uneven); it
  is in the opened column, and the filter still matches it.
- **The value bar**: decision 1 to 3. Each segment's tooltip is `value ·
  N rows (P%)`, the empty string shown as "(empty)" as today. No listed
  values: "Mostly unique" in place of the bar, as today. Not profiled:
  "Not profiled (type not supported)" across the statistic cells, as
  today. Loading: a skeleton of the row's height in those cells.
- **Opening**: the whole row is the control (a real button for the name,
  `aria-expanded`, `aria-controls`; Enter and Space work; the row's hover
  shows it can be pressed). The opened column sits in a row of its own
  right under, spanning the table:
  - **Values**: one line per listed value: the value (mono, truncated,
    whole value as title), a bar as wide as its share at the same step as
    its segment, the count, the share. Then "Other values" and "Null" as
    their own lines when above zero, in the plain tone. Under it, when the
    profile was cut (`sampled`), the words the note already uses.
  - **Facts**: type (whole), can be null (Yes / No / Not known), nulls
    (count and share), distinct (`≈ N`), range (when there is one),
    description (when there is one), and "Partitioned by" / "Sorted by"
    when the column is one. Use `MetadataList` if it fits; do not write a
    second definition list.
  - Without a profile (no permission, unsupported, failed) the row still
    opens and shows the facts that need none.
- **Widths.** At `xl` and up everything shows. Below `xl` the range
  leaves the row; below `md` the distinct count and the label beside the
  bar leave it; below `sm` the type moves under the name in small muted
  type and the row is name, bar, null share. Nothing the row sheds is
  lost: the opened column has it. **No sideways scroll of the page or of
  the card at 390 px.**
- **The filter box** shows when the table has more than 10 columns; the
  page-size toggle keeps its rule (more than 25).
- Statistics columns do not show at all when there is no profile to show
  (as today's `showStats`).
- **Accept (component tests, in `asset-detail-tabs.test.tsx` or a new
  `asset-columns.test.tsx` beside the component):**
  - a row shows name, type, the commonest value with its share, the null
    share and the distinct count, and no chip list;
  - the bar has one segment per listed value, each as wide as its share
    (assert the width style), and none for "other";
  - pressing a row opens it: every listed value with count and share,
    "Other values", "Null", and the facts; pressing again closes it; two
    rows can be open; the button carries `aria-expanded`;
  - a description is in the opened column and still found by the filter;
  - a column named by the sort key carries the mark, one inside an
    expression does not;
  - "Mostly unique", "Not profiled", the loading skeleton, and the three
    no-profile states each still read as they did;
  - the filter box is there at 11 columns and not at 10;
  - the existing wide-table tests (25 first, sizes, filter across the
    whole table) still hold. A test that asserted the chip list is
    rewritten to assert the same values and counts where they now are;
    say which in the handoff. No assertion is dropped without its
    replacement.

### C3 — Documents

- `CHANGELOG.md` `[Unreleased]`: one entry.
- `docs/FEATURE_COVERAGE.md`: the asset detail row, if it describes the
  Schema tab's columns.
- **Accept:** each sentence names only what exists.

## 6. Working on this machine

- Work in `/home/hv/lakehouse` on `feat/connectors`. It is the product
  owner's checkout: their console dev server (port 3000) reloads every
  saved file under `src/`. Never kill or restart it, never run `next
  build` or `bun install` here, and do not leave `src/` in a state that
  does not compile.
- **The tree already holds another uncommitted change** (schema versions
  for Silver and Gold, waiting for the product owner): Rust files, the
  `SchemaVersions` card, `asset-activity.tsx`, `asset-overview.tsx`,
  `asset-about.tsx`, the contract, the mock, tests under `describe("Schema
  versions of a Silver or Gold table")`, three documents. Leave every one
  of those hunks exactly as it is. You will add hunks to
  `asset-schema.tsx`, the test file, `CHANGELOG.md` and
  `docs/FEATURE_COVERAGE.md` beside them.
- **Leave the work uncommitted.** No `git add`, commit, stash, reset,
  checkout or branch switch. Never push.
- Untracked and not yours: `docs/plans/FEAT-CONNECTORS-REPORT.md`,
  `lark-import/`, `ops/g3/bronze_catalog.py`. Never read or print `.env`.
- No Rust, no `docker`. No prettier: match the file's style by hand.
- Before the handoff, once, in the foreground: `bun run typecheck && bun
  run lint && bun run test`.

## 7. Acceptance checklist (product owner)

On a Silver table with many columns (Data Explorer, Schema tab):

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Look down the list | Every row one line, the same height; no chips | |
| 2 | Read one row | Glyph, name, type, a bar with the commonest value and its share, null share, distinct, range | |
| 3 | Hover a segment of a bar | The value, its rows and its share | |
| 4 | Press a row | It opens under the row: each value with bar, count and share; "Other values"; "Null"; the facts | |
| 5 | Press it again; open two | It closes; two stay open together | |
| 6 | Find the column the table is sorted by | It carries "sort key" | |
| 7 | Type in the filter box | The list narrows; a word from a description finds its column | |
| 8 | Narrow the window to a phone's width | No sideways scroll; rows keep name, bar and null share | |
| 9 | Open a raw table's Schema tab | The same rows; "System columns" and "Schema versions" as before | |

## 8. Handoff (developer)

Written by the developer agent (Claude Sonnet) on 2026-10-05. Everything is
uncommitted. **Not looked at in a browser**: the developer agent has none, so
nothing below says how the page looks; layout is reasoned from the CSS and
needs the reviewer's screenshots (390, 768, 1024, 1440; light and dark).

### Files

Added:

- `src/lib/column-type.ts`, `column-type.test.ts`: `typeFamily` (C1).
- `src/lib/column-profile.ts`, `column-profile.test.ts`: `valueShares`,
  `formatShare`, `sortKeyColumns` (C1); `noValuesLabel` (round 2, R4).
- `src/features/catalog/asset-columns.tsx`: the "Columns" card (C2): the
  list, a row, the opened column, `NullMeter`, `ProfileNote`, the filter and
  page-size controls; also now owns `visibleColumns`, `SchemaRow` and
  `ProfileState`.
- `src/features/catalog/asset-columns.test.tsx`: 44 component tests (37
  in round 1, 7 in round 2) that drive `ColumnsCard` with a hand-built
  profile (C2 accept list).

Changed:

- `src/features/catalog/asset-schema.tsx`: renders `ColumnsCard`; keeps
  `schemaRows`, `useProfile`, `ColumnCells` (for "System columns"),
  `AssetSchema`. Removed what moved (`COLUMN_PAGE_SIZES`, `visibleColumns`,
  `NullMeter`, the two types, `ProfileNote`, `StatCells`) and the imports
  they used. `SchemaVersions` is untouched (its hunks in `git diff` are the
  other change's, as before); the only shared line is the `@/lib/format`
  import, which lost `formatCompactNumber`, `formatNumber` and
  `formatPercent` and keeps the other change's three.
- `src/features/catalog/asset-detail-tabs.test.tsx`: `stubApi` takes an
  optional `profile` (default `PROFILE`, so every other test is unchanged);
  one test rewritten; one new `describe("Schema tab: the column explorer")`
  of four tests after the wide-table block. Nothing in or near
  `describe("Schema versions of a Silver or Gold table")` was touched.
- `CHANGELOG.md`: one new bullet at the top of `[Unreleased]` / Added,
  directly above the schema-versions bullet, which is unchanged.
- This plan, section 8.

Not changed, on purpose: `docs/FEATURE_COVERAGE.md`. C3 says "if it
describes the Schema tab's columns"; its asset detail row does not, so there
was nothing true to add.

### Commands (foreground, from `/home/hv/lakehouse`)

Round 1 (superseded by the round 2 run at the end of this section: 637
pass, 77 files):

- `bun run typecheck`: exit 0.
- `bun run lint`: exit 0; 0 errors, 5 warnings, the same five as the
  baseline (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx`,
  `use-data-table.ts`, `dashboard-specs.ts`), none in a file of this change.
- `bun run test`: exit 0; 637 pass, 0 fail, 77 files (baseline 563 in 74;
  +74 = 37 card tests, 33 `src/lib` tests, 4 new tab tests; the rewritten
  test is not an addition). Nothing was skipped.
- Not run: `next build`, `bun install`, prettier, any browser or `docker`
  command, any Rust or Python check (no such file changed).

### Departures from the plan, and what the plan had wrong

1. **Container queries, not viewport breakpoints.** The plan says the row
   sheds things below `xl`, `md`, `sm`. The sidebar takes 16rem from a
   viewport of `md` up, so at a 768 px viewport the card is roughly 450 to
   500 px wide (768 less the sidebar, the page and the card padding) and a
   viewport breakpoint would keep columns it has no room for. The
   list is an `@container`; the thresholds are the list's own width:
   `@lg` (32rem): the type gets a track, below it the type sits under the
   name; `@2xl` (42rem): label beside the bar, and the distinct count;
   `@4xl` (56rem): the range. Same order of shedding as the plan; the
   numbers differ. At 390 px the list is about 320 px wide.
2. **The filter and the page-size toggle are in the card body, above the
   list, not in the card header's `action` slot.** `SectionCard` wraps
   `action` in a `shrink-0` div, so a 176 px box beside a page-size toggle
   cannot fit a phone's card (from the CSS, for a table of more than 25
   columns it did not before either; not seen in a browser), and the filter
   now appears from 11 columns. The controls still go with the card, with no
   change to the shared pattern.
3. **Shares read "60.0%", not "60%"**: `formatPercent`, one decimal, like
   the null share beside it. A share under 0.1% reads "<0.1%"
   (`formatShare`), never "0.0%", which would say the value is absent.
4. **`valueShares` states less than the plan's shape in one case.** When the
   profile gives neither `nullFraction` nor `nullCount`, `nullShare` and
   `otherShare` are `null` (not 0), since "other" would then include nulls
   it cannot tell apart; the opened column then draws neither line. It also
   returns `otherCount` and `nullCount` for the lines' count column. A hair
   of float noise from adding fractions (`0.1 + 0.2 + 0.7`) is not turned
   into a sliver of "Other values".
5. **`sortKeyColumns` splits at top-level commas only** (outside parentheses,
   brackets and quotes) and reads a name in backticks or double quotes as the
   name. The plan's plain split on commas would mark `b` as a sort key for
   `cityHash64(a, b, c)`; a test pins that. Every case the plan names behaves
   as the plan says.
6. **Two facts the plan did not list: "Masked" and "Classification"**, only
   when the column has them. The row's marks can be cut short at narrow
   widths (they never push the row wider), and "nothing the row sheds is
   lost" needs them in the opened column.
7. **`visibleColumns`, `SchemaRow`, `ProfileState` live in
   `asset-columns.tsx`**, not `asset-schema.tsx`, so that
   `asset-schema.tsx -> asset-columns.tsx` is one direction (no import
   cycle). Same names, same behaviour. No test or other file imported any
   of them (grepped), so nothing needed to follow. `schemaRows` and
   `useProfile` stayed.
8. **Segments' tooltips are the native `title`**, not the `Tooltip`
   component: 25 rows of up to five segments would be 125 popup roots. The
   text is the plan's `value · N rows (P%)`. Not available by touch; the
   bar's `aria-label` and the opened column carry the same.
9. **No rows profiled gives "—" in the bar cell**, not a label (an empty
   table has no values to describe). The label itself changed in round 2
   (R4, below): an empty list no longer reads "Mostly unique" by default.
10. The plan said tests that asserted the chip list were to be rewritten. **No
    test asserted the chips**: every `PROFILE` fixture in the tabs test has
    `topValues: []`. What broke was the `td`-based row-name selector and the
    "Yes" in the old Nullable cell (below).
11. The plan said `schemaRows`, `visibleColumns`, `useProfile` are "exported
    or used by tests". `schemaRows` and `visibleColumns` were exported;
    `useProfile` and `ProfileState` were not; no test imported any.

### Existing tests rewritten or removed

One test rewritten, none removed, no assertion dropped:

- `describe("Schema tab")` > "lists columns in the table's order with their
  statistics, and system columns apart":
  - the row names read from `r.querySelector("td")` now read from the
    button in each `role="row"` (same expected `["id", "amount"]`);
  - "`amount` is optional in Iceberg" asserted `Yes` in the row's Nullable
    cell; it now presses the `amount` row and asserts "Can be null" is
    `Yes` in the opened column, and also `No` for the required `id`;
  - the `25.0%` wait, the "System columns" assertions and the "Schema
    versions" assertions are unchanged.
- `describe("Schema tab: a wide table")`: unchanged and passing (they count
  `role="row"`, so the header row is still the `- 1`; 25 / 50 / All; filter
  by name or description across the whole table; neither control for two
  columns).

New in round 1: `asset-columns.test.tsx` (37), and in the tabs file "draws each
column's most frequent values from the profile route, and opens the column",
"marks the column the storage card's sorting key names...", "marks no sort
key for a table that has no storage card", "asks for no statistics, and says
why, without query:read, yet still opens a column".

### How the row is built, and the 390 px rule

The list is `div`s with the ARIA table roles (`table`, `rowgroup`, `row`,
`columnheader`, `cell`), not the shared `Table`, whose `overflow-x-auto`
wrapper plus a seven-cell row is what scrolled sideways. Every row (and the
header) is a CSS grid whose tracks are fixed `rem` or `minmax(0, …fr)`, so a
cell can only truncate; the list is a container and sheds cells by its own
width (departure 1). At phone width the row is glyph, name (with the type on
a second grid line under it, then the marks after the type: round 2, R3),
bar, null share, chevron; the page's and the card's sideways scroll was
confirmed gone by the reviewer's screenshots at 390 px. What to look at in
round 2: 390 px (a column with a mark still shows its whole name, the type
truncates, rows stay equal), 768 and 1024 px with the sidebar open (range
gone at 1024 unless the card is 56rem wide), a bar's length at 1440 px
against rows with a short and a long commonest value, and an opened column
at each width.

### Unsure

- Everything visual, as above; in particular `line-clamp-2` on the "Not
  profiled" message as a grid cell.
- The dark theme: the ladder is `chart-3` at 100, 80, 60, 45, 30 percent on
  the muted track; the faintest step may be hard to see on the dark track.
  `chart-5` at full strength was rejected as too close to it.
- "Commonest" is the first listed value; that relies on the route's
  `approx_top_k` answering most frequent first (ClickHouse documents it;
  nothing in this repository states it, and `exact_top_values` only
  filters). If the order were ever different, the label and the steps would
  be wrong; a sort in `valueShares` would fix it at the cost of "in the
  profile's order".
- Enter and Space on the name button are the browser's own button
  behaviour; happy-dom does not turn a key press into a click, so the tests
  press with `click`. The row's own `onClick` serves a pointer and is not
  focusable; the button is the keyboard and screen-reader path.

### Round 2: the planner's review (R1 to R4)

Files touched: `src/features/catalog/asset-columns.tsx`,
`asset-columns.test.tsx`, `asset-detail-tabs.test.tsx` (one assertion),
`src/lib/column-profile.ts`, `column-profile.test.ts`, `CHANGELOG.md` (the
same bullet, three sentences), this section. Nothing else.

- **R1, one track for a bar on every row.** The bar and its label are now
  two tracks of a grid, `grid-cols-1` below `@2xl` (the bar takes the whole
  cell, as before) and `minmax(0,1fr) minmax(0,1fr)` from it. The bar is
  `w-full` with no floor and no `flex-1`; the label is its own track, the
  value left and truncating, the share at the right edge in tabular
  numbers. To leave the label room, the values track is `2fr` at `@2xl` and
  `@4xl` (it was `1.6fr` and `1.8fr`); the other tracks are unchanged. The
  "no listed value" label is not in the grid: it spans the cell.
- **R2, one spelling for the null share.** `NullMeter`'s number and the
  "Nulls" fact now use `formatShare`, like the opened column's "Null" line:
  under 0.1% reads "<0.1%" in all three, exactly zero stays "0.0%".
  `formatPercent` is no longer used in the card.
- **R3, the name wins over its marks below `@lg`.** The marks are rendered
  twice, by a `Marks` component: beside the name (`hidden @lg:flex`) and on
  the type's line after the type (`flex @lg:hidden`). CSS shows one; the
  other is `display: none`, so nothing is read twice. I chose two copies
  because the type and the marks must share a line below `@lg` but sit in
  different tracks from `@lg`, which one element cannot do. The type's line
  is `h-5` like the name's, so a marked row is as tall as an unmarked one
  (the row's floor is now `min-h-13`, 52 px, from `min-h-12`); the type
  truncates first, the marks are `shrink-0`. Consequence for tests: a marked
  column has each mark twice in the DOM (happy-dom shows no CSS), so mark
  assertions name the place.
- **R4, an empty list is not "unique".** `noValuesLabel(column, rows)` in
  `column-profile.ts`: all null (rows minus nulls is zero) reads "All null";
  a distinct count of at least 90% of the non-null rows reads "Mostly
  unique"; otherwise, or with no distinct count, "Many distinct values".
  `null` for a column not profiled or no rows. Without a null count the
  non-null rows are taken as all rows, which can only make "Mostly unique"
  harder to reach. The row shows the label where the bar would be, with the
  `title` "No value is listed: the profile states a value's count only where
  it is exact."; the opened column's sentence is the same constant.

Tests changed in round 2 (nothing dropped):

- Replaced: "says a column with no listed values is mostly unique, as the
  chips' empty case did" (it used `AMOUNT`, 40 distinct of 1,500 non-null
  rows, where "Mostly unique" is false) by four tests: mostly unique (1,990
  distinct, no nulls), many distinct values (`AMOUNT`), many distinct values
  with no distinct count, all null (including its opened column).
- Edited, same assertion with a wider regex: the three places that asserted
  `queryByText("Mostly unique")` is absent (no rows profiled, the three
  no-profile states, loading) now assert none of the three labels is there.
- Edited, same assertions: the two mark tests ("marks masked, classified
  and partitioned columns as before..." and "marks a column the sorting key
  names...") use `expectMark`, which asserts each mark in both places; in
  the tabs file "marks the column the storage card's sorting key names..."
  expects `sort key` twice in the marked row (and still none in the other).
- Added: bar track independent of the label (short and long value); null
  share under 0.1% in the row, list and facts; null share of exactly zero;
  marks off the name's line below `@lg`; and 9 `noValuesLabel` tests (all
  null, from the fraction, the 90% line and just below it, about 500
  distinct in 2,000 rows, the line against non-null rows, no distinct
  count, no null count, nothing to say), plus one `formatShare` case.

- **R5, the range's own track from `@4xl`** (superseded in width and in the
  name's track by R6, below; the fixed 11.5rem range track stands). After the values track went to
  `2fr` a date range read "2019-01-20 – 2025-09-…" at 1440 px. The `@4xl`
  template (header and rows, one string) now gives the range a fixed
  `11.5rem` track (it was `minmax(0,1.1fr)`; two ISO dates and the dash are
  23 characters of mono `text-xs`, about 10.4rem) and the name `1.4fr` (it
  was `1.2fr`); the type (`1fr`) and values (`2fr`) tracks are unchanged, so
  the room comes from the flexible tracks and the bar and the label stay two
  equal tracks. Arithmetic at the narrowest container that shows the range,
  56rem: 54.4rem inside the border and padding, 30rem for the fixed tracks
  and the gaps, 24.4rem shared 1.4 : 1 : 2, so name 7.8rem (what it had
  before this change), type 5.5rem (was 6.5rem), bar and label 5.3rem each
  (were 6.3rem); nothing is squeezed to nothing, so the width at which the
  range appears stays `@4xl`. At 1440 px (a list about 1,070 px wide by the
  reviewer's 142 px bar) that is name about 188 px, type 134 px, bar and
  label about 130 px each, range 184 px. A longer range (two timestamps) may
  still truncate; its title and the opened column carry it. New test: "gives
  the range, from @4xl, a fixed track that holds two ISO dates and the dash
  between them" (the template is the same in the header and the rows, the
  range's track is a fixed `rem` size of at least 23 x 0.6 x 0.75).

- **R6, the type before the range.** At a 1280 px viewport (the list about
  58.5rem, just past `@4xl`) R5's range track left every type cut to
  "Nullable(Str…" and the label to "2023-0…", while the range was "–" on
  most rows. The type matters more on this tab, so the range waits. The
  range now appears from `@5xl` (64rem) instead of `@4xl` (header, rows, the
  skeleton cell and the "Not profiled" `col-span` all follow), and the type
  has a fixed `9rem` track from `@3xl` (48rem): 20 characters of mono
  `text-xs`, `Nullable(Float64)` being 17; below `@3xl` it stays `1fr`. The
  name is `1.4fr` and the values `2fr` from `@3xl`, so the bar and the label
  remain two equal tracks and every bar track the same length. The
  templates: `@3xl:grid-cols-[1.25rem_minmax(0,1.4fr)_9rem_minmax(0,2fr)_6.5rem_4.5rem_1rem]`
  and `@5xl:grid-cols-[1.25rem_minmax(0,1.4fr)_9rem_minmax(0,2fr)_6.5rem_4.5rem_11.5rem_1rem]`,
  replacing R5's `@4xl` template. Arithmetic (a row is the container less
  1.625rem of border and padding, gaps 0.75rem, the rest shared 1.4 : 2),
  agreeing with the planner's: at `@3xl` 48rem 19.6rem free, name 8.1rem,
  values 11.5rem; at 58.5rem (1280 px, no range) name 12.4rem, values
  17.7rem; at `@5xl` 64rem 23.4rem free, name 9.6rem, values 13.8rem; at
  68.5rem (1440 px) name 11.5rem, values 16.4rem. Outcome: `Nullable(Float64)`
  whole at 1280 and 1440 px; the range, wherever it shows, whole
  ("2019-01-20 – 2025-09-23", 10.4rem in 11.5rem); no range column at 1280
  px. The R5 test is now "holds the type whole from @3xl and shows the
  range, whole, only from @5xl" (templates equal in header and rows at
  `@2xl`, `@3xl` and `@5xl`; the type a fraction at `@2xl` and a fixed size
  of at least 17 characters from `@3xl`; the range a fixed size of at least
  23 characters, from `@5xl`; its header and cell `hidden @5xl:block`; no
  `@4xl` left in the card). Not looked at in a browser.

Commands, round 2, once, in the foreground:
`cd /home/hv/lakehouse && bun run typecheck && bun run lint && bun run test`:
exit 0; typecheck clean; lint 0 errors and the same 5 warnings; 653 pass, 0
fail, 77 files (637 before this round: +7 card tests, +9 `noValuesLabel`
tests). Still **not looked at in a browser**.

After R5 the same command again: exit 0; typecheck clean; lint 0 errors and
the same 5 warnings; 654 pass, 0 fail, 77 files (+1, the R5 test).

After R6 the same command again: exit 0; typecheck clean; lint 0 errors and
the same 5 warnings; 654 pass, 0 fail, 77 files (the R5 test was rewritten,
not added to).

## 9. Review (planner)

### 2026-10-05 — C1 to C3, four rounds

Read: `asset-columns.tsx`, `src/lib/column-profile.ts`,
`src/lib/column-type.ts`, the diffs of the shared files, the CHANGELOG
entry. Ran myself, from this tree, after the last round:

| Command | Result |
| --- | --- |
| `bun run typecheck` | exit 0 |
| `bun run lint` | exit 0: 0 errors, 5 warnings, none in a changed file |
| `bun run test` | exit 0: 654 passed, 0 failed, 77 files |

Not run: `next build` (the product owner's dev server holds this tree).

**Findings, all closed.** Each was seen on a screenshot, which is where
this kind of work fails; the tests were green throughout.

- **R1 (SHOULD-FIX).** A bar's track was as wide as the label beside it
  left it, so at 1440 px a 97.7% bar was drawn 213 px long and a 100% bar
  192 px. Closed: bar and label on two tracks of fixed proportion.
  Measured after: every track 127 px at 1440, 138 px at 1280, 79 px at
  390.
- **R2 (SHOULD-FIX).** One null share read "<0.1%" in the opened column's
  list and "0.1%" in its facts and in the row. Closed: one spelling.
- **R3 (SHOULD-FIX).** At 390 px a mark squeezed its column's name to two
  letters ("pl…"). Closed: below the first threshold the marks sit on the
  type's line.
- **R4 (SHOULD-FIX, the plan's error).** Section 5 told the developer to
  keep "Mostly unique" for a column with no listed value. An empty list
  means more distinct values than the route counts exactly (100), not
  uniqueness: a column of about 500 distinct values in 2,000 rows read
  "Mostly unique" beside its own "Distinct ≈ 500". Closed: "All null",
  "Mostly unique" (distinct at least 90% of the non-null rows) or "Many
  distinct values", from the numbers.
- **R5, R6 (widths).** After R1 a date range was cut at 1440 px; giving
  the range a fixed track (R5) then cut every type at 1280 px. Closed
  (R6): the type has a fixed track that holds `Nullable(Float64)` from
  the third threshold, and the range shows only from the widest one.
  Measured after: no cell cut at 1280 or 1440; the range column is absent
  at 1280 and whole at 1440.

**Accepted departures from this plan:** container queries on the list's
width instead of viewport breakpoints (the sidebar takes 16rem of the
viewport); the filter above the list instead of in the card's header
slot, which cannot shrink; shares to one decimal; the sort key split
only at top-level commas; Masked and Classification among the opened
column's facts; the marks rendered twice, one copy per placement, the
hidden one `display: none`.

**In a browser** (headless; 1440 and 1280 dark, 1440 and 390 light, 900
dark; a Silver table of 18 columns and a raw table of 4): rows of one
height at every width; no sideways scroll at 390 px; a pressed row opens
under itself with its values, "Other values", "Null" and its facts; the
two columns the table is sorted by carry "sort key"; the raw table's
"System columns" and "Schema versions" cards are as they were.

**Not checked by anyone:** a screen reader; keyboard use in a real
browser (the control is a native button, the tests press it by click);
a table past 25 columns on the dev stack (none exists; the unit tests
cover the page sizes); a masked or classified column on the dev stack.

**Verdict:** ready for the product owner's look. Nothing is committed.
