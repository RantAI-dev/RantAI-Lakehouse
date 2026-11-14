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

### Review, round 1 (planner, Opus, 2026-11-14, with the owner's QA)

The planner re-ran, each under `systemd-run --user --scope -p MemoryMax=3G`:
- `bun run typecheck`: OK;
- `bun run lint`: 0 errors, the 11 existing warnings;
- `bun test chart-builder-layout.test.tsx chart-builder.test.tsx chart-builder-page.test.tsx`: 80 pass, 0 fail.

The code follows the plan. The owner's QA on the preview (port 28620) found these. The general
rules they come from are now in `/home/hv/mtharreq/DESIGN.md` §3, "Form dan panel pengaturan
rapi dan lega".

- **SHOULD-FIX 1, the inspector tabs are cramped.**
  - Data / Sort & filter / Style touch the panel's edges and each other, with no breathing
    room. The inactive "Style" also reads as disabled.
  - Give the tab row inner padding from the panel edge and real gaps.
  - Make inactive tabs readable (muted, not greyed out like a disabled control), with a clear
    active state.
- **SHOULD-FIX 2, controls in the inspector are full width and uniform.**
  - "On click" is a narrow select while "Freshness" is full width.
  - Every select and input in the inspector spans the panel's content width, unless two
    belong together on one row. Value plus aggregate stays a pair, as today, with matching
    widths.
- **SHOULD-FIX 3, the Style tab.** It still carries the old format section's look:
  - Bordered fieldsets ("Values", "Dates", "Colours", "Axes") make boxes inside the panel.
    Replace them with small group headings and spacing, no borders.
  - Fields are labelled only by placeholders, and they are cut off ("Decima…", "Prefix",
    "Suffix", "Series name", "X title"). Give every field a visible label above it, and no
    placeholder that is cut off.
  - The thousands toggle is labelled "1,000". Make it "Thousands separator".
  - The "Short names" and "Colour by value" toggles get a label that says what they do.
  - Uneven widths: "As it is now" and "Time as is" selects of different sizes. Make them
    full width per SHOULD-FIX 2.
  - Defaults read oddly ("As it is now", "Time as is"). Use "Automatic" where it means the
    chart's own default, consistently.
- **Open for the owner (L4):** at 1366 px the data panel is folded into a button. The owner has
  not answered yet whether it stays folded or shows inline. Leave it as built until they decide.

- **SHOULD-FIX 4 (owner, added during the round):** remove the header's Cancel. The back arrow
  and Esc leave, with a "Discard changes?" confirm after a change (DESIGN.md "Satu aksi, satu
  tombol").
- **SHOULD-FIX 5 (owner, added during the round):**
  - Merge "Add to" into the breadcrumb as a dashboard picker, so the header is only
    "← [dashboard ⌄] / New chart" and Save.
  - Move the width to the Style tab as "Size on dashboard": two pictures, "Half row" and
    "Full row". The owner did not understand "Half width / Full width".
  - The owner confirmed that SHOULD-FIX 2 means every inspector control is full width.

No `BLOCKER`. Fix SHOULD-FIX 1–3, cite "BI-58 review round 1" at the fix sites and in the
commit, re-take the affected screenshots (the Style tab and the inspector at 1366×900, light
and dark), and hand off again.

## 6. Handoff

### Handoff, BI-58 (Opus developer agent, 2026-11-14)

**Commits** on `feat/bi-58-chart-builder-revamp`, over `a986737f`, not pushed:

| Task | Commit | What |
| --- | --- | --- |
| T1 | `302e6566` | Page frame: header row, three zones (container queries), FormPage, type column and resizable split removed; the round-trip guard test |
| T2 | `11aa14f8` | Data panel: columns as Dates / Text / Numbers, joined tables under their own heading, chips, one-click use and the use menu |
| T3 | `197285a9` | Canvas (title, description, states, result line, Data toggle) and the type toolbar with its gallery drawn from the preview's rows |
| T4 | `5aee3a18` | Inspector tabs Data / Sort & filter / Style, nothing dropped |
| T5 | `6dab5315` | The 1280 px fold ("Pick data" sheet), the phone stack, the assistant bar |
| T6 | `2a390cc3` | Limit, number format, BI-11 parameter limit, KPI note, plain words, error sentences |
| T5 | `baf193a2` | Fixes found in the screenshots, and the screenshots |
| T7 | this entry | |

**Checks on `baf193a2`**, each run alone, under `nice -n 15` and a cgroup cap
(`systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0`):

- `bun test` on the 18 builder files (`chart-builder`, `chart-builder-layout`, `chart-builder-page`,
  `chart-shape-section`, `format-section`, `gantt-builder`, `type-b-builder`, `layers-section`,
  `joins-section`, `rows-table`, and `lib/` `builder-rules`, `section-summary`, `style-draft`,
  `format`, `kind-gallery`, `column-click`, `chart-coverage`, `chart-kind-search`):
  **242 pass, 0 fail, 1,202 expect() calls**.
- `bun run typecheck`: **0 errors**.
- `bun run lint`: **0 errors, 11 warnings** (the 11 that were there before).
- `bun run test` (the whole console suite): **not verified (CI)**. The planner stopped it on this
  VM (OOM, 2026-11-14); it runs on the draft PR.
- No Rust was touched or compiled.

**Tests the plan names:**
- The gallery makes no request: `chart-builder-layout.test.tsx`, "draws each type it can from the
  preview's rows, with no request of its own and at most 200 rows" (fetch and every method of
  `dashboardService`, `metricService` and `paramService` watched while it opens and draws).
- A saved chart of each family round-trips: the same file, "saves an unchanged … chart back as the
  same definition" for bar, line, KPI with compare, table, Gantt and combined. It was run against the
  builder at `240d2e62` first and passed there, so its definitions are the old builder's own output.
- One click on a date column and one on a number column give a preview: "gives a preview from a
  source, one click on a date column and one on a number column" (and the same through the sheet).
- The Limit can be emptied: "lets the Limit be emptied and typed into, and puts back the last number
  when left empty".

**Screenshots:** `docs/plans/bi-58-screens/`, taken on the QA data (dashboard `b_47cff752` "QA BI",
Edit chart on `jumlah by tahun`) with a dev server of this worktree started briefly on port 28622
and stopped by PID straight after: `new-` and `edit-` at `1190x800`, `1366x900` and `390x844`,
`light` and `dark` (12), plus `gallery-1366x900-light` and `sheet-1190x800-dark`. Measured in each:
no sideways scroll; at both laptop sizes the page height equals the window (the zones end above the
assistant bar); at 1366x900 the canvas is 726 px wide, more than half of the window.

**Choices the plan did not settle:**
1. *"Below 1280 px wide"* is read as the builder's own width (a container query), not the window's.
   With the window's, the data panel would be inline at 1366x900 and the canvas about 460 px, under
   half the page (spec T1). With the builder's (about 1,060 px at 1366 with the sidebar open) the
   panel is folded at 1190 and 1366 and inline from about 1,580 px windows. The planner and the
   owner may prefer otherwise.
2. The breadcrumb's heading is "New chart" / "Edit chart"; the chart's name is not repeated there,
   since it is the large title right below (DESIGN.md: say it once).
3. The description under the title is saved as `subtitle`, which `StoredChartSpec` already has and the
   dashboard already shows. Sent only when filled, so every saved chart without one round-trips
   unchanged; an edited chart that has one (the assistant writes it) now keeps it, where the old
   builder dropped it on save.
4. The result line says what is known: "5 groups", or "Showing 20 of 54 groups" when the limit cut,
   rather than "N groups from M rows": the preview does not return the number of source rows read.
5. Gallery rules (`lib/kind-gallery.ts`): a type draws a picture only from rows of the same shape
   (one row per X, or per X and series). Others show their icon until picked (KPI, maps, scatter…).
   A type is greyed out only for what the builder's own pickers require (a date column for Calendar
   and Gantt, two text or date columns for heatmap/Sankey/sunburst, enough number columns for
   `measureLimits`, a region column for the region map) and cannot be clicked then. Enter in the
   search picks the first match the source can make.
6. Words beyond the plan's list, to match the chips: "Dimension (X)" is "X axis", "Breakdown /
   series" is "Split into series", and "split" replaces "breakdown" in the limit labels and the
   split's grain and bins. Inside "Second source" each entry is still "Question n" (left as is).
7. On click and Freshness (from More options, not in the plan's table) are in the Data tab.
8. The toolbar's eight types are mockup A's (bar, horizontal bar, line, area, donut, treemap, KPI,
   table).
9. Removed as dead with the old layout: `lib/kind-list-pref.ts`, `AccordionSection`, `FoldSection`,
   `moreOptionsUsed`, `formatUsed`, with their tests. The point map's grid picker and the calendar
   note were rendered twice; they render once.
10. A single value row hides its move buttons (all disabled), and the move and remove buttons sit on
    the value's label line, so the column picker keeps its width in the narrow inspector.

**Not done or not verified:**
- `bun run test` in full: not verified (CI), as above.
- The owner's preview server: not started (coordinator's instruction); ready for one.
- Not checked in a real phone browser (iPhone Safari); the 390x844 shots are Chrome emulation.
- The SQL source paths (write SQL, Edit SQL, the held preview's Chart/Rows) were not exercised in
  the screenshots; their tests pass.
- `AGENTS.md` carries the block `next dev` writes on start (uncommitted). Restoring it with
  `git checkout` was refused by this session's permission check, so it is left for the planner.

### Handoff, BI-58 review round 1 (Opus developer agent, 2026-11-14)

**Commits:** `5bde2ebe` (SHOULD-FIX 1–5, tests, screenshots), and this entry's commit, which also
adds the round's citation at the two-column rows of `type-b-fields.tsx`.

**Checks on the final commit**, each run alone with `nice -n 15` under
`systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0`:
- `bun test` on the 20 builder files (round 0's 18 plus `reference-lines-editor` and
  `click-setting`): **256 pass, 0 fail, 1,294 expect() calls**.
- `bun run typecheck`: **0 errors**.
- `bun run lint`: **0 errors, 11 warnings** (the existing 11).
- `bun run test` in full: **not verified (CI)**.

**Fixes:**
- **SHOULD-FIX 1:** the tab row has 1rem of padding from the panel's edge and gaps between its tabs.
  An inactive tab is muted text that brightens on hover; the active one is foreground with its
  underline.
- **SHOULD-FIX 2 (owner: every control full width):** the two-column rows of On click, the goal and
  Gantt fields follow the inspector's own width (`@md`), so in the inspector each control spans the
  panel. The value and its aggregate are equal halves. A step's condition is its column across the
  row, then comparison and value as a pair. A step's value box keeps its `min-w-28` floor (phase 2
  QA R12, test unchanged).
- **SHOULD-FIX 3:**
  - The Style tab is groups under small headings, separated by space, with no bordered fieldsets.
  - Every field has a visible label, and no placeholder is cut off.
  - The toggles read as sentences: "Thousands separator", "Short month and day names (Oct, Mon)",
    "Shade each mark by its value", "Logarithmic scale", "Show this axis".
  - "Automatic" replaces "As it is now" and "Time as is".
  - Axes, notes, conditional rules and reference lines are short stacks of labelled fields under a
    sub-heading.
  - Steps, second-source questions and the open filter editor use a soft fill instead of a border.
  - Accessible names are unchanged, so the existing tests find the same controls.
- **SHOULD-FIX 4:**
  - Cancel is gone. The back arrow and Esc leave; Esc counts only when no popover, list, menu or
    dialog is open.
  - After a change, "Discard changes?" asks first (Keep editing / Discard).
  - In-app links (the sidebar) are held back the same way, and a reload or tab close asks through
    `beforeunload`.
  - "A change" means a field the chart is saved with changed after the person first touched the
    builder. Before that first touch, the baseline follows the builder's own loading and settling.
- **SHOULD-FIX 5:**
  - The header is "← [dashboard ⌄] / New chart" and Save. The breadcrumb's dashboard is the target
    picker ("Add to" before).
  - The width is "Size on dashboard" at the top of Style: two drawings of a dashboard row, "Half
    row" and "Full row". It is a radio group with arrow keys and keeps the same `span` values.
  - A note, an image or a web page shows the same group in its inspector.

**Tests added:**
- back with no change leaves at once;
- back after a change asks, Keep editing stays, Discard leaves;
- Esc after a change asks;
- picking another dashboard in the breadcrumb saves to it;
- the header holds only back, the picker and Save;
- the width pictures set the span, with radio semantics and arrow keys.

**Screenshots:** `docs/plans/bi-58-screens/review-data-`, `review-style-` and `review-style-axes-`
at `1366x900`, `light` and `dark`. They were taken on the planner's preview at port 28620 with a
headless Chrome started and stopped by PID. No server was started or stopped. The round-0
`new-*` and `edit-*` shots still show the old header (Add to, Width, Cancel); the `review-data-*`
shots show the new one.

**Choices:**
1. The picker keeps the accessible name "Add to dashboard"; its hover title says what it is.
2. The browser's own back button cannot be held back while Next's router owns the history; only
   links, the back arrow, Esc, reload and tab close ask.
3. Leaving with the confirm open: Esc or a click outside closes the confirm (keep editing).

**Not verified:** the full `bun run test` (CI); a real iPhone Safari check; the discard confirm on the
live preview (covered by the tests only). `AGENTS.md` still carries the uncommitted block `next dev`
writes, and `.preview-dev.log` (the planner's preview log) is untracked; neither is committed.

### Handoff, BI-58 review round 2 (Opus developer agent, 2026-11-14)

**Commits:**
- `1ab47ac4` SHOULD-FIX 6, the tab row: equal thirds, a full-width divider, the sliding underline, and the test.
- `712caad2` SHOULD-FIX 7's code: `ZONE_HEADER`, the shared 44 px height.
- `5bdab0bc` SHOULD-FIX 7's test and its measurement.
- `8f625135` the app shell's sidebar at 14rem, without the subtitle.
- `c4d87211` the composer, centred on the content area, with one shared bottom token.
- The commit of this entry adds the screenshots.

**History note (not rewritten):** `712caad2` holds SHOULD-FIX 7 (`ZONE_HEADER`, the shared 44 px height),
not 6 as its message says; `1ab47ac4` is SHOULD-FIX 6. A second session committed it while this one
was working. The final tree has each fix once, and tests cover both: equal tabs in the R3 tab test,
the shared height in the zones test.

**Checks on `c4d87211`**, each run alone with `nice -n 15` under `systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0`:
- `bun test` on 23 files (the builder's 20 plus the 3 app-shell tests): **273 pass, 0 fail, 1,370 expect() calls**;
- `bun run typecheck`: **0 errors**;
- `bun run lint`: **0 errors, 11 warnings** (the existing 11);
- `bun run test` in full: **not verified (CI)**.

**Measured on the preview at 28620** over CDP, using `getBoundingClientRect`:

| Check | 1366x900 | 1190x800 |
| --- | --- | --- |
| Divider bottoms: canvas toolbar / inspector tabs | 180 / 180 | 180 / 180 |
| Card tops / bottoms | 136 / 788 | 136 / 688 |
| Composer centre / content centre | 795 / 795 | 707 / 707 |
| Gap between the zones and the composer | 14 px | 14 px |
| Page scroll | none | none |

- Card tops and bottoms were measured before the composer change. After it, the zones end 14 px
  above the composer, at 824 (1366x900) and 724 (1190x800).
- With the sidebar collapsed to icons, the composer centre and the content centre are both 707.
- At 14rem (224 px) no sidebar label is cut off.

**Screenshots:** `docs/plans/bi-58-screens/review-data-`, `review-style-` and `review-style-axes-`
at `1366x900`, light and dark, and `review-sidebar-1366x900-dark.png`, all re-taken after the
sidebar and composer changes. They were taken on 28620 with one headless Chrome of my own (its own
profile folder), stopped by PID afterwards. No dev server was started or stopped.

**Not verified:** the full suite (CI); a real iPhone Safari check; the phone drawer and the composer
on a phone, beyond the code path (with no `--content-left` the composer is centred on the window as
before).

### Handoff, BI-58 review round 3 (Opus developer agent, 2026-11-14)

**SHOULD-FIX 8, the type gallery always shows pictures.** The fix is in this entry's commit.
- **Before any data,** every type is drawn from a fixed sample shaped for it (`src/lib/kind-sample.ts`):
  five categories, eight months, two or three values, a split, a box plot, six weeks of days, eight
  ranges, a four-task Gantt plan, a KPI, a gauge, a progress bar, bullets, a pivot and a note. The
  sample is never requested and never saved. A quiet line above the grid says "Pick a source and
  columns to see your own numbers."
- **With data,** types that can be drawn from the preview's rows use them. The others use the sample
  at 0.7 opacity. Types the source cannot make stay greyed out with their reason.
- **Drawings:** the region, point and density maps, the image and the web page need a file or an
  address to draw, so they get small inline SVG drawings in the theme's colours.
- **Cards:**
  - short titles with no leading icon (`KIND_SHORT`); the long labels stay for the toolbar, the
    tooltips and the search;
  - descriptions wrap without an ellipsis;
  - the picture sits flush on the card's own fill, with no box inside the card;
  - the selected card has a ring around the whole card;
  - the gallery is three columns of about 330 px at 1366 wide.

**Checks**, each run alone with `nice -n 15` under the 3G cap:
- `bun test` on 23 files: **274 pass, 0 fail, 1,578 expect() calls**;
- `bun run typecheck`: **0 errors**;
- `bun run lint`: **0 errors, 11 warnings** (the existing 11);
- the full `bun run test`: **not verified (CI)**.

**Tests:**
- With no data, all 33 cards draw something (a chart, a table, a drawing or the note), none says
  "No data", none is dimmed, each title is its short name, and no request is made.
- With data, a KPI card shows the sample at reduced emphasis while Donut and Line draw the
  person's rows.
- The earlier no-request and 200-row checks still pass.
- `chart-builder.test.tsx` now stubs ECharts (no canvas in the test DOM) and restores it after.

**Screenshots:** `docs/plans/bi-58-screens/review-gallery-1366x900-dark.png` (no data) and
`review-gallery-data-1366x900-dark.png` (Edit chart on "jumlah by tahun"). They were taken on the
28620 preview with one headless Chrome, stopped by PID afterwards. No server was started.

### Handoff, BI-58 review round 4 (Opus developer agent, 2026-11-14)

**SHOULD-FIX 9, the type gallery as a dialog with two views** (this entry's commit):
- **Dialog:**
  - "More" opens the console's `Dialog`: centred, a dim backdrop, the title "Chart type", a close
    button and Esc;
  - focus stays inside and returns to "More" on close;
  - about 960 px wide at 1366 and at most 80 vh tall. The title, the view switch and the search stay
    at the top while the types scroll. Without data, the "Pick a source…" line sits under the
    search as the dialog's description.
  - Picking a type closes the dialog and applies it.
- **Views:** a Cards / List switch (a radio group with icons and labels) at the top right. The choice
  is remembered per browser (`lh.chartBuilder.galleryView`, inside try/catch).
  - Cards: four to a row, with the picture on top, the short name, and a description of at most two
    lines. Seven long descriptions are shortened for the gallery (`kindBlurb`); the full ones stay as
    tooltips and search text.
  - List: compact rows with a 72×44 picture (drawn at 4× and scaled), the name, the description and
    the greyed reason where the type is unavailable.
  - Both views keep the groups and the search.
- **No orphan rows:** every card in a row has the same height and gap, so a group that ends with a
  single card adds no larger gap.

**Checks**, each run alone with `nice -n 15` under the 3G cap:
- `bun test` on 23 files: **276 pass, 0 fail, 1,658 expect() calls**;
- `bun run typecheck`: **0 errors**;
- `bun run lint`: **0 errors, 11 warnings** (the existing 11);
- the full `bun run test`: **not verified (CI)**.

**Tests:**
- "More" opens a modal dialog (the page behind is out of reach), Esc closes it, and focus returns to
  "More".
- The List view shows the same 33 types in the same order, each with its picture. The view is
  remembered across a remount.
- Picking Funnel in the list closes the dialog and sets the kind.
- The no-request tests still pass.
- Phase 2 QA R17's outside-press test now presses the dialog's backdrop, which is where an outside
  press lands on a modal.

**Screenshots:** `docs/plans/bi-58-screens/review-gallery-dialog-cards-1366x900-dark.png` and
`review-gallery-dialog-list-1366x900-dark.png`. Both are the Edit chart page on the 28620 preview;
the dialog measures 960 × 720 px. They were taken with one headless Chrome, stopped by PID
afterwards. No server was started.
