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

## 9. Review (planner)
