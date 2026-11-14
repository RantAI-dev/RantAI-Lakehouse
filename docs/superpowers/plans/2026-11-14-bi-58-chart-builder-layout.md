# BI-58: the chart builder's layout (canvas), plan

| | |
| --- | --- |
| Backlog | `BI-58` |
| Spec | `docs/core/specs/bi-58.md` |
| Feature page | `docs/core/features/chart-builder-layout.md` |
| Base | `main` at `240d2e62` (anchors below verified there) |
| Branch | `feat/bi-58-chart-builder-revamp` |
| Planner and reviewer | Claude Opus (hub session) |
| Developer | an Opus developer agent (console design work, AGENTS.md exception) |

## 1. Decisions already made

- **Layout (owner, 2026-11-14): mockup A, Canvas.** The reference is commit `6ede5307` on the
  local branch `design/chart-builder-mockups`, files `src/features/mockups/chart-builder/`
  (`mockup-a.tsx`, `shared.tsx`). The worktree `/home/hv/mtharreq/lakehouse-mockups` shares this
  repository's objects, so `git show 6ede5307:<path>` works here. **Copy its layout, not its
  data layer:** the mockup computes in the browser (`engine.ts`, `fixture.ts`), and the product
  keeps the builder's real state, services and preview.
- **Chart-type gallery (owner, 2026-11-14): from mockup B.** Each type is a small picture of
  the chosen data, drawn from the rows the live preview already fetched. There is no extra
  query; at most 200 rows are drawn.
- **Same spec.** Nothing changes in what is saved (`StoredChartSpec`), the API, or the
  assistant's tools. Assistant parity: no tool or schema change, and the page context is
  unchanged. Name that in the PR.
- **New chart and Edit chart** both use the new layout. They are one component today
  (`chart-builder-page.tsx` → `ChartBuilder`).
- **Responsive.** Below 1280 px wide, the data panel folds into a "Pick data" button that
  opens it as a sheet. At phone width everything stacks: header, canvas, inspector. There is
  no horizontal scroll.
- **The assistant bar** (the floating "Ask anything" input) never covers a control. Reserve
  room at the bottom of the scroll areas, or lift the bar on this page, whichever reads
  better.
- **Out of scope:**
  - mockup C (the sentence builder);
  - the chart palette;
  - new chart features;
  - the dashboard page and its dialogs (the Parameters dialog checkbox label goes to the
    small-fix PR, §4).

## 2. Anchors (at `240d2e62`)

- **Page wrapper:** `src/features/dashboards/chart-builder-page.tsx`; routes
  `src/app/(overview)/dashboards/[id]/charts/new/page.tsx` and
  `.../charts/[chartId]/edit/page.tsx`.
- **The builder:** `src/features/dashboards/chart-builder.tsx` (1,996 lines).
  - `KIND_GROUPS` at `:153`, `KIND_DESCRIPTIONS` at `:260`, `ChartBuilder` at `:360`.
  - The page description at `:1844` glues two sentences:
    `` `${KIND_DESCRIPTIONS[kind]} Review live data before adding it to the dashboard.` ``.
- **Sections:**
  - `builder-sections.tsx` (the accordion shells);
  - `chart-shape-section.tsx` (X, values, the preview Rows table);
  - `layers-section.tsx` (`BI-13` "More questions", the second source);
  - `table-settings.tsx` (tables; `KpiCompareSection`, whose default note at `:273` is
    `` `${metric.timeColumn}, ${metric.grain ?? "month"} (from ${metric.label})` ``);
  - `format-section.tsx` (Style).
- **The preview:** `dashboard-preview.tsx`.
- **Tests to keep green, updating selectors where the layout moves things, never weakening an
  assertion:** `chart-builder.test.tsx`, `chart-builder-page.test.tsx`,
  `chart-shape-section.test.tsx`, `format-section.test.tsx`, `gantt-builder.test.tsx`,
  `type-b-builder.test.tsx`.

## 3. Tasks (one commit each, in order)

**T1, page frame.**
- **Build:**
  - a header row with: back link and breadcrumb (dashboard / New chart or the chart's
    name); "Add to" dashboard select; the width segmented control; Cancel; Save;
  - below the header, three zones: the data panel (left, about 260 px), the canvas
    (centre, flexible), and the inspector (right, about 320 px).
- **Remove:** the old title, Dashboard and Width row (where the labels overlap), and the
  long chart-type column.
- **Check:** at 1366×900 the canvas is at least half the page's width (spec T1). At 1190×800
  nothing overlaps.

**T2, data panel.**
- **Build:**
  - the source picker (mart, SQL source or write SQL, as today) at the top;
  - the source's columns grouped as Dates, Text and Numbers, each with its type icon, the
    column name, and the label if there is one;
  - a small chip on the column in use: "X axis", "Value" or "Split".
- **Clicks:** a click on an unused column sets it where it fits. A date or text column
  becomes X when X is empty; a number column becomes the value with the default aggregate
  when the value is empty. Otherwise a small menu offers "Use as X axis / value / split /
  filter".
- **Joins:** a joined source's columns appear under its own heading (`BI-7`).
- **Check:** with a source picked, one click on a date column and one on a number column
  give a preview (spec T4).

**T3, canvas and chart-type picker.**
- **Canvas:** the title and description are edited in place above the preview. The preview
  fills the canvas, with today's empty, loading and error states styled to fit. Below the
  preview sit the "N groups from M rows" note and a "Data" toggle for the Rows table.
- **Toolbar:** a row of icons for the eight most used types, plus **More**. More opens a
  gallery grouped as today (`KIND_GROUPS`).
  - Each card draws the chosen data in that type from the preview's rows.
  - A type that cannot draw this data is greyed out, with one line saying why. Reuse the
    rules the builder already has for which kinds need what; do not invent new ones.
  - The gallery has a search box (today's "Search chart types").
- **Check:**
  - every kind in `KIND_GROUPS` is reachable;
  - picking one updates the preview as today;
  - the gallery makes no extra request (a test spies on the services).

**T4, inspector tabs.** Every control of the old five sections moves to one of three tabs,
nothing dropped:

| Tab | Controls |
| --- | --- |
| **Data** | X axis, date grouping (`BI-9`), bins (`BI-10`), value(s) and aggregate, split into series, second source (`BI-13` layers), parameters (`BI-11`), KPI compare (`BI-16`), Gantt and table settings where the kind needs them |
| **Sort & filter** | sort, top N and "Other", filters, steps (`BI-6`) |
| **Style** | `format-section.tsx` as it is |

- Kind-specific panels appear only for the kinds that use them, as today.
- **Check:** a test that opens a saved chart of each family (bar, line, KPI with compare,
  table, Gantt, combined), saves it unchanged, and gets the same spec back (spec T5).

**T5, responsive and the assistant bar.** Make the 1280 px fold, the phone stack, and the
assistant bar never covering a control work as decided in §1.
- **Check:** screenshots at 1190×800, 1366×900 and 390×844, in light and dark, saved under
  `docs/plans/bi-58-screens/`. Check them yourself for overlap.

**T6, words and the builder findings from the 2026-11-14 QA.** These live in the same files,
so they ride along (feature page L5):
1. **Limit:** the input can be emptied and typed into
   (`chart-builder.tsx:1668`, `Number(e.target.value) || 20` snaps back to 20). Keep the
   number while typing and validate on blur.
2. **Number format:** numbers shown to people use the browser's locale with thousand
   separators. Today the builder mixes `toLocaleString("id-ID")` (`:1377`) and `"en-US"`
   (`:1613`). This covers the preview Rows table (`chart-shape-section.tsx`). One helper,
   reused.
3. **Description:** one sentence per kind, with no glued second sentence (`:1844`).
4. **The `BI-11` parameter:** do not offer it as the limit on a chart that has steps or
   groupings (the server refuses that shape: `params.rs` `check_chart_shape`). Hide the
   option and say why in one line, instead of letting the preview disappear with a raw
   message.
5. **KPI compare:** the Date column note (`table-settings.tsx:273`) must not be cut off.
   Wrap it, or shorten it to "start_date, by month".
6. **Plain words:**
   - "Lined up on" → "Matched on";
   - "More questions" → "Second source";
   - "Measure" → "Value" where the label is the user's;
   - raw lowercase error texts get a sentence.

   Keep the names the assistant and docs use in code identifiers.

**T7, handoff.** Append "Handoff, BI-58" below.
- **Light checks only:** `bun run typecheck`, `bun run lint`, `bun run test` (console).
- **No Rust:** no Rust compiles on this VM (owner, 2026-11-14), and this change touches no
  Rust.
- **Dev server for the owner's preview:** start it from this worktree on port 28620 after
  stopping the mockup server by its PID (`/home/hv/mtharreq/lakehouse-mockups`, PIDs in its
  log), with the API proxy on `localhost:28604` (`RUST_API_URL`). Report the PID.

## 4. Not in this PR (the small-fix PR, Sonnet)

- The Parameters dialog checkbox has no visible label (`param-dialog.tsx:103`, `aria-label`
  only).
- "Could not load values." shows before a column is picked
  (`filters/use-filter-values.ts:101`).
- `BI-13`: a year present only in the second source is appended after the last year even
  when sorted A to Z. The preview Rows table shows only the first source. The default
  "highest first" reverses a year axis.

## 5. Review

(planner)

## 6. Handoff

(developer)
