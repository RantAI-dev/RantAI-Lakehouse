# BI-17 part A: chart formatting (with AI-8)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-3` (worktree `/home/hv/lakehouse-phase3-build`), after
the BI-18 part C review fixes are committed. Feature page:
`docs/core/features/chart-formatting.md` (decisions 1 to 15, planner
defaults the owner confirms at the phase 3 QA). Spec: `docs/core/specs/bi-17.md`
part A, `docs/core/specs/ai-8.md`.

Anchors were read on 2026-10-15 from the working tree at `82171ae7` while
another developer was editing `store.rs`, `dashboard.rs`, `versions.rs`,
`layout.rs`, `dashboard-page.tsx` and `dashboard-tabs.ts`; line numbers in
those files will have moved. Find each anchor by the symbol named with it.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions:

- **D1 One style block.** Everything this feature adds to a chart lives in
  one new flattened struct on `ChartInput` (`store.rs`, `ChartInput`, which
  already flattens `TableFields` and `ShapeFields`): `StyleFields`, in a
  new module `lakehouse-bi/src/style.rs`. Stored in the existing
  `spec_json` envelope, so no ClickHouse DDL. Fields (camelCase on the
  wire):
  - `format: Option<ValueFormat>` (the chart's default) and
    `valueFormats: BTreeMap<String, ValueFormat>` (per measure alias or
    column name).
  - `ValueFormat { style: auto|number|percent|currency|scientific|compact,
    decimals: Option<u8> (0..=10), thousands: bool (default true),
    currency: Option<String> (ISO 4217, upper case), currencyDisplay:
    symbol|code, prefix: Option<String>, suffix: Option<String> (each at
    most 16 chars, one line) }`.
  - `dateFormat: Option<DateFormat { style: auto|short|medium|long|iso|monthYear,
    hour12: Option<bool>, abbreviate: bool }>`.
  - `colors: Option<ChartColors { palette: Option<String> (a built-in id
    or "company"), series: BTreeMap<String, String> (series name → #rrggbb),
    byValue: Option<Vec<String>> (2 or 3 #rrggbb) }>`.
  - `axes: Option<Axes { x: AxisStyle, y: AxisStyle, y2: AxisStyle,
    rightValues: Vec<String> }>`, `AxisStyle { title: Option<String> (≤ 60),
    show: bool (default true), min: Option<f64>, max: Option<f64>, log: bool,
    rotate: Option<u16> (0, 45 or 90) }`.
  - `labels: Option<DataLabels { show: bool }>`.
  - `annotations: Vec<Annotation { x: String, series: Option<String>, text: String (≤ 140) }>` (≤ 20).
  - `tooltip: Option<TooltipFields { fields: Vec<String> }>` (aliases the chart already returns).
  - `conditional: Vec<CondRule>` (≤ 20) for `table`, raw table, `pivot`
    and `kpi` only: `CondRule { column: String, rule: Rule, color: String,
    target: cell|text }` where `Rule = Compare { op: lt|le|gt|ge|eq|ne,
    value: f64 } | Between { from: f64, to: f64 } | Contains { text: String } |
    Range { colors: Vec<String> (2 or 3) }`.
  Validation in one `check_style(kind, &StyleFields, &Columns)` called where
  `check_chart_shape` (`params.rs`) is called; every refusal is a plain
  sentence (400). A field that does not apply to the kind is refused the way
  `tables.rs` `only_for` refuses `compare` on a non-KPI.
- **D2 Reference lines extend the existing ones.** `ReferenceLine`
  (`params.rs`, `value: LineValue(Fixed|Param)`, `label`, max
  `MAX_REFERENCE_LINES = 5`) gains `kind: value|average|median|min|max|trend|band`
  (default `value`, so stored charts read as today), `to: Option<LineValue>`
  for a band, `color: Option<String>` and `measure: Option<String>` (which
  value a computed line describes; default the first). Computed lines are
  computed in the console from the rows the chart shows; when the tile is
  cut off, the label ends with "of the N shown" (feature decision 9). The
  existing `withReferenceLines` (`chart-option.ts`) is the one place that
  draws them.
- **D3 One formatter.** `src/lib/value-format.ts`: `formatValue(v, fmt, locale)`,
  `formatAxisValue`, `formatDateValue(v, dateFmt, locale)`. Compact uses
  K, M, B, T in every locale (feature decision 3); currency via
  `Intl.NumberFormat` with the chosen code and display; percent multiplies
  by 100, as Metabase's "Percent" style does (0.125 → 12.5 %; a column that
  already holds 12.5 uses `number` with suffix "%"). Every number on a chart (axis, tooltip, label, KPI, table cell, reference
  line label) goes through it. `chart-axis.ts` `formatNumber` /
  `formatCompactNumber` / `decimalsFor` stay as the `auto` style's
  implementation, called from the new module. `cell-format.ts` currency
  stops being fixed to IDR.
- **D4 The saved format is honoured.** The hard-coded
  `format: Some(NumFmt::Int)` in `store.rs` (five sites, grep
  `NumFmt::Int`) stays for stored data, but the renderer stops ignoring the
  spec: a chart with no `StyleFields.format` shows what it shows today
  (`auto`); a measure that is a metric (`MetricDef.format`, `metrics.rs`)
  uses the metric's format as its default.
- **D5 Settings.** Admin > Settings (`settings.rs` `get_reporting` /
  `put_reporting`, store `lakehouse-store/src/reporting_settings.rs`, page
  `src/features/admin/reporting-settings-page.tsx`) gains: `numberLocale`
  (`en-US` | `id-ID`, default `en-US`), `companyPalette` (0 or 2..=12
  #rrggbb), `fonts` (subset of the shipped ids, default all). Postgres
  migration `rust/migrations/0072_reporting_formatting.sql` (why-header;
  `ALTER TABLE reporting_settings ADD COLUMN … DEFAULT …`). They reach every
  view through the dashboard payload's `reporting` object (as BI-9 did:
  `routes/dashboard.rs` and `routes/embed.rs` add `"reporting"`;
  `reporting-context.tsx` reads it), so public links and embeds use them.
  `settings:write` to change, as today. If 0072 is taken on `main` when the
  phase PR is opened, the planner renumbers.
- **D6 Theme and font per dashboard.** `Board` gains `theme: auto|light|dark`
  (default auto) and `font: Option<String>` (must be one of the allowed
  fonts at save). Added the way `grain` was (`ensure_board_state_columns`,
  `BOARD_COLS`, `row_to_board`, `upsert_board`, `save_board_patch` /
  `BoardPatch`, `BoardFields`, `restore_board_fields`, versions
  `BoardState` and its diff "Theme"), payload field, `BoardEditBody`.
  The dashboard container applies its own theme over `next-themes`
  (`resolvedTheme`) for the tiles and the ECharts `dark` prop in
  `dashboard-page.tsx`, `public-dashboard.tsx`, `embed-view.tsx`.
  Fonts are self-hosted with `next/font` (Inter, IBM Plex Sans, Source
  Serif 4, JetBrains Mono) as CSS variables, plus `system`; nothing loads
  from a font CDN at run time. `bun run build` must pass (next/font fetches
  at build time; if the build cannot reach the network, stop and report).
- **D7 Assistant (AI-8).** New tool `format_chart { chart, ...StyleFields,
  referenceLines? }`: a partial merge (a field given replaces that field;
  `null` clears it; a field not given is kept), the same `check_style`
  validation, the same write path and confirmation behaviour as
  `update_chart` (read `routes/ai/tools/dashboards.rs` `update_chart`;
  if it uses `needs_confirmation`, so does this). New tool
  `set_dashboard_theme { board, theme?, font? }`. `create_chart` and
  `update_chart` schemas accept the style fields too (`registry.rs`
  `with_shape_properties` pattern). Both tools named in
  `src/features/copilot/capabilities.ts`; the prompt line for charts
  (`routes/ai/prompt.rs` SYSTEM_BUILD_SUFFIX) says "to format a chart,
  call format_chart with only what changes". Regenerate the fixture with
  the ignored writer test (`write_tool_schema_fixture`). Add the AI-8-AC1
  request to the standard request set (`AI-15`; grep its file).

## 2. Anchors (2026-10-15, find by symbol)

- Chart definition: `lakehouse-bi/src/store.rs` `ChartInput`, `StoredChartSpec`,
  `StoredEnvelope`, `insert_chart`; `specs.rs` `ChartKind`, `NumFmt`,
  `ChartSpec`, `ChartRenderSpec`; `tables.rs` `ColumnSetting`, `COLUMN_FORMATS`,
  `only_for`, `Compare`; `shape.rs` `ShapeFields`; `params.rs`
  `ReferenceLine`, `LineValue`, `MAX_REFERENCE_LINES`, `takes_reference_lines`,
  `check_chart_shape`; `metrics.rs` `MetricDef.format`.
  Literal `ChartInput { … }` constructions outside tests (each needs
  `style: StyleFields::default()` or `..Default::default()`): `store.rs`
  `empty_chart_input` and the KPI/text/chart/tables/groupings constructions,
  `layers.rs`, `metrics.rs` (grep `ChartInput {`).
- TS mirrors: `src/lib/dashboard-specs.ts` (`ChartSpec`, `ChartRenderSpec`),
  `src/services/clients/bi-store.ts` (`ChartInput`, `Board`),
  `src/lib/table-types.ts` (`ColumnSetting`, `TableDefFields`).
- Renderer: ECharts 6. `tile-body.tsx` (kind switch), `echart.tsx`
  (`echarts.init`, `dark` prop), `chart-option.ts` (`PALETTE`, `buildOption`,
  `valAxis`, category `rotate`, combo `yAxisIndex`, boxplot log axis, gauge,
  heatmap/calendar `visualMap`, `withReferenceLines`, `fmtInt`),
  `combined-option.ts` (imports `PALETTE`), `geo-option.ts`,
  `src/lib/chart-axis.ts`, `src/lib/cell-format.ts` (`cellContent`,
  currency fixed to IDR), `table-cell.tsx`, `raw-table.tsx`,
  `pivot-table.tsx`, `kpi-card.tsx`, `src/lib/kpi-compare.ts`.
- Builder: `chart-builder.tsx` sections ("More options" `FoldSection` holds
  `ReferenceLinesEditor`, `KpiCompareSection`, `TableSection part="format"`,
  `ClickSetting`), payload assembly and prefill; `builder-sections.tsx`
  (`AccordionSection`, `FoldSection`); `src/lib/builder-rules.ts`
  (`moreOptionsUsed`); `reference-lines-editor.tsx`, `table-settings.tsx`.
- Settings: `routes/settings.rs`, `routes/mod.rs` (routes),
  `lakehouse-store/src/reporting_settings.rs`, `rust/migrations/0070_reporting_settings.sql`,
  `src/features/admin/reporting-settings-page.tsx`,
  `src/services/clients/settings.ts`, `src/services/contracts/settings.ts`,
  `reporting-context.tsx`.
- Board: `store.rs` `Board`, `ensure_board_state_columns`, `BOARD_COLS`,
  `row_to_board`, `upsert_board`, `save_board_patch`, `BoardPatch`,
  `update_board_grain` (pattern), `BoardFields`, `restore_board_fields`;
  `versions.rs` `BoardState`, `describe_change`; `routes/dashboard.rs`
  `BoardEditBody`, `changes_content`, payload builder.
- Assistant: `routes/ai/registry.rs` (`chart_kind_enum`,
  `with_shape_properties`, `create_chart_schema`, `update_chart_schema`,
  `TOOLS`, the tool-count test, `write_tool_schema_fixture`),
  `routes/ai/tools/dashboards.rs`, `routes/ai/prompt.rs`,
  `src/features/copilot/capabilities.ts`, `chart-draft-card.tsx`.
- Policy: `rust/crates/lakehouse-api/src/policy.rs` `POLICY_TABLE`.

## 3. Tasks (one commit each, in this order; Rust together first)

**T1 Settings and migration (D5).** Migration 0072, store fields with
defaults, `put_reporting` validation (locale in the two values, palette
2..=12 valid hex or empty, fonts subset of the shipped ids), payload
`reporting` carries them, settings page sections "Numbers", "Company
palette" (swatches with add/remove, hex input), "Fonts" (checkboxes).
*Accept:* store round-trip test; route tests for each refusal and for
`settings:write` (403 without); `route_auth` unchanged or extended if a
route is added; page component test.

**T2 Style model and validation (D1, D2, D4).** `style.rs`, `check_style`,
ISO 4217 list as a constant (all active codes), hex check, limits,
kind applicability; `ReferenceLine` extension with backwards-compatible
serde defaults; `ColumnSetting.decimals` widened to 0..=10 and a
`currency` code on table columns; the TS mirrors.
*Accept:* unit tests per rule and limit (decimals 11 refused, unknown
currency refused, 13 palette colours refused, conditional rule on a bar
chart refused, log axis allowed only on a value axis, an old stored chart
JSON without any style field round-trips unchanged, an old reference line
without `kind` reads as `value`).

**T3 Board theme and font (D6), Rust side.** Columns, patch, versions,
payload, body. *Accept:* store test; route test (font not in the allowed
list refused; `dashboard:write` needed); versions diff names "Theme";
revert restores theme and font.

**T4 Formatter (D3).** `value-format.ts` and its use in `chart-axis.ts` and
`cell-format.ts`. *Accept:* tests: `1234567.891` → `Rp 1,23 M` (`id-ID`,
IDR, compact, 2 decimals) and `$1.23 M` (`en-US`, USD, symbol); percent
`0.125` → `12.5%` with 1 decimal; scientific; prefix/suffix; thousands
off; `auto` gives today's output for the existing fixtures (no change to
current snapshots).

**T5 Renderer.** `chart-option.ts`, `combined-option.ts`, `geo-option.ts`,
`kpi-card.tsx`, table/raw/pivot cells: formats on axes, tooltips, labels;
palettes (built-in ids and "company"), per-series colours, colour by
value (`visualMap` continuous over the shown min/max for the single-series
kinds of feature decision 7); axes (title, show, min/max, log with the
honest message of decision 8, rotate, `rightValues` on a second Y axis for
bar/line/area/combo); data labels with `labelLayout: { hideOverlap: true }`;
reference lines per D2 (trend = least squares on the shown points; band =
`markArea`); annotations as `markPoint` with the note in its tooltip, and
the "N notes not shown" line under the chart; tooltip fields; conditional
formatting on table, raw table, pivot cells and the KPI value.
*Accept:* `chart-option` unit tests per feature (option JSON asserts),
conditional-format tests for each tile kind, a log-axis-with-zero test
that returns the message instead of an option.

**T6 Theme and font in the console (D6).** Container theme override,
`next/font` families, dashboard ⋯ menu "Theme and font…" dialog.
*Accept:* component test that a dark-theme dashboard passes `dark` to its
tiles while the app theme is light; public and embed views too;
`bun run build` passes.

**T7 Builder.** A new section 5 "Format" (a `FoldSection` after "More
options") with groups Values, Dates, Colours, Axes, Labels, Reference lines
(the existing editor moves here, extended with the new kinds), Notes,
Tooltip; conditional formatting goes in the table/KPI section; groups that
do not apply to the kind are not shown (`builder-rules.ts`); "Add note"
in the chart click menu (`chart-click.ts`) in edit mode for a user with
`dashboard:write`. Prefill and payload round-trip every field.
*Accept:* builder tests: a saved chart with every field reopens with the
same values and saves the same JSON; a bar chart shows Colours, Axes,
Labels, Reference lines but not conditional formatting; a table shows
conditional formatting.

**T8 Assistant (D7).** *Accept:* tool tests: `format_chart` with only
`colors` keeps the chart's existing format and reference lines; `null`
clears; invalid currency refused with the console's sentence; a user
without `dashboard:write` is refused; `set_dashboard_theme`; tool-count
pin updated; fixture regenerated and the snapshot test green;
`capabilities.test.ts` green.

**T9 Docs.** CHANGELOG entry; the feature page's Status line stays (planner
updates it after QA). No change to `PRODUCT.md`/`BACKLOG.md` (planner).

## 4. Out of scope

Forecasting and clustering (part B), fonts from the internet, extra
tooltip fields the chart does not query, rich text in notes, the built-in
"Main" dashboard's fixed tiles, a global Content-Security-Policy.

## 5. Must verify, not assume

- Every reader of `ChartInput` and `ReferenceLine` still compiles and reads
  stored JSON written before this branch (add a fixture of today's JSON).
- `withReferenceLines` is used by every chart kind that `takes_reference_lines`
  allows; if not, say which.
- How `update_chart` confirms (needs_confirmation or not) before copying it.
- Whether an `AI-15` standard request file exists and where.

## 6. Checks

Per commit, scoped (AGENTS.md "Keep build time down"):
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`
on every cargo command, one at a time, `--test-threads=4`, check `free -g`
and `df -h /` first; never `cargo build -p lakehouse-api`; no
`rust/target` in the worktree; no prettier. Rust: `cargo fmt --check`,
`cargo clippy -p <crates touched> --all-targets -- -D warnings`,
`cargo test -p lakehouse-bi`, `-p lakehouse-store` (with
`DATABASE_URL` as the crate's tests expect), the `lakehouse-api` lib
filters for `settings`, `dashboard`, `ai::`, and the integration tests
`route_auth`, `sec11_guard`. TS: `bun run typecheck`, `bun run lint`, and
the tests of the files touched (`src/lib`, `src/features/dashboards`,
`src/features/admin`, `src/features/copilot`). Before handoff: the same
scoped set once more plus `bun run build`; the full workspace run is CI's
(write *not verified* for it).

## 7. Handoff (developer appends)

### BI-17 part A and AI-8, slice 1 (T1 to T9)

Branch `feat/bi-phase-3`, from `ea3e034e`. Developer: Claude Sonnet. Nothing pushed, no PR.

**Commits** (one per task, each body cites its task):

| Task | Commit | What it became |
| --- | --- | --- |
| T1 | `db19f436` | Migration `0072_reporting_formatting.sql` (`number_locale`, `company_palette`, `fonts`), `lakehouse_bi::formatting` (closed lists, `is_hex_color`), store `ReportingSettings` with the three fields, `put_reporting` with optional new fields (a field not sent keeps the stored value) and its sentences, `reporting` object of the dashboard, embed and public payloads carries them, Admin > Settings sections Numbers, Company palette, Fonts. |
| T2 | `770b35f7` | `lakehouse_bi::style` (`StyleFields` flattened in `ChartInput`, `check_style`, `check_style_names`, ISO 4217 list), `ReferenceLine` with `kind`/`to`/`color`/`measure` (`value` is now optional, absent on a computed line), `check_reference_lines` now runs on every chart, `ColumnSetting.decimals` 0..=10 and `currency`, TS mirrors (`chart-style.ts`, reference line draft helpers). |
| T3 | `0da344e0` | `Board.theme`/`font` (ClickHouse columns, upsert, patch, trash keep list, duplicate, version state and its "Theme" diff, revert), `PUT /api/dashboard/boards` takes `theme`/`font` (font checked against the administrator's list), dashboard payload (top-level `theme`/`font`) and embed/public payload (`board.theme`/`board.font`). |
| T4 | `03c57859` | `src/lib/value-format.ts` (`formatValue`, `formatAxisValue`, `formatCompactCurrency`, `formatDateValue`, `formatFromHint`, `withMetricFormats`); `chart-axis` and `cell-format` take a locale; a currency code with the compact style is a compact amount. |
| T5 | `828b9fa2` | Renderer: `buildOption` formats and palette, `style-option.ts` post-pass (axes, second axis, colours per series and by value, labels, notes, tooltip fields, log-axis message), reference lines per D2, conditional colours on grouped/raw tables, pivots and KPIs, layered charts and maps take formats/palette, `metricFormats` on the tile payload (D4). |
| T6 | `797883c0` | `dashboardLook`, `next/font` families, `theme-light` tokens, dashboard container/public/embed apply theme and font, "Theme and font..." dialog. |
| T7 | `a8f4ff51` | Builder section 5 "Format" (`format-section.tsx`, `style-draft.ts`), reference lines editor moved there and extended, conditional editor, "Add note..." in the click menu. |
| T8 | `7600cd33` | `format_chart`, `set_dashboard_theme` (`tools/look.rs`), `style_properties` shared by the three chart tools, prompt, domain words, gate summaries, fixture regenerated, tool count 93, `capabilities.ts`. |
| T9 | `59e0ad4f` | CHANGELOG entry. |

**Checks run** (all in the foreground, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`, on the final code before the T9 and handoff commits, which touch only Markdown):

- `cd rust && cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-bi -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings`: no warnings.
- `cargo test -p lakehouse-bi -- --test-threads=4`: 490 passed, 3 ignored, 0 failed (lib), plus `specs_match_typescript` 1 passed.
- `cargo test -p lakehouse-store -- --test-threads=4`: every test binary ok, 0 failed (`reporting_settings` 5 passed). The store tests start their own Postgres through `lakehouse-test-support` (testcontainers), so no `DATABASE_URL` was set or needed.
- `cargo test -p lakehouse-api --lib -- --test-threads=4 settings dashboard ai:: embed`: 522 passed, 4 ignored, 0 failed.
- `cargo test -p lakehouse-api --test route_auth -- --test-threads=2`: 38 passed. `--test sec11_guard`: 4 passed. `--test reporting_settings_route`: 8 passed. `--test ai_tool`: 9 passed. (Each integration file on its own.)
- `write_tool_schema_fixture` (ignored writer test) was run once; `tests/fixtures/tool_schemas.json` regenerated (+1327 lines, 18 lines replaced: the reference line schema).
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings (the same 6 as on `ea3e034e`). `bun run test`: 1913 pass, 1 skip, 0 fail (198 files).
- `bun --bun next build --webpack`: compiled successfully, including `next/font` downloading the four fonts at build time.

**Not verified**

- `bun run build` as written (Turbopack) fails here before compiling anything: `Symlink node_modules is invalid, it points out of the filesystem root`, because the worktree's `node_modules` is a symlink to `/home/hv/lakehouse-uiux/node_modules`. It is the environment, not this diff; `next build --webpack` passes on the same tree. CI, which has a real `node_modules`, is the check for the Turbopack build.
- The full workspace run (`cargo clippy --workspace`, `cargo test --workspace`), the other integration test files of `lakehouse-api` and the other lib filters: CI. `route_auth` with `--test-threads=4` failed 9 tests once (transient, same tests pass alone and at 2 threads); at 2 threads it passed.
- A browser: nothing was rendered in a real browser. The ECharts options are asserted as JSON, the tiles and the builder under happy-dom. Not seen with eyes: the palettes' look, the `theme-light` tokens inside a dark console (the `dark:` variant override in `globals.css`), fonts actually loading, the log axis, the markArea band, the pin notes, the second Y axis, the Format section's layout.
- The assistant through a model (`AI-8-AC1`): the request is tested as a `format_chart` argument set (`the_request_of_ai_8_ac1_applies_all_three_changes`), not as a conversation.
- Python, compose and the Dagster tests: not touched, not run.
- Migration `0072` against the dev database: only through the `sqlx::test` databases.

**Where the plan was wrong against the code (or I deviated)**

1. `check_chart_shape` (`params.rs`) is called only on the parameter path (`spec_from_param_chart`), not for every chart, so reference lines of an ordinary chart were never validated. I extracted `check_reference_lines` and call it from `spec_before_relation` (every chart) as well as from `check_chart_shape`; `check_style` is called there too. Names are checked in `spec_from_resolved`'s chart path (`check_style_names`) unless the chart uses a parameter (its columns change). So the signature is `check_style(&ChartInput)` plus `check_style_names(&ChartInput, &cols)`, not `check_style(kind, &StyleFields, &Columns)`.
2. Feature decision 6 says all five palettes pass the contrast check in both themes. The current default palette (kept, so no saved chart changes colour) does not: `#0ea5e9`, `#f59e0b`, `#84cc16`, `#f97316` and others are under 3:1 on the white card (sky is 2.77). The four new palettes pass 3:1 on `#ffffff` and `#18181b` (`chart-palettes.test.ts`); the test says the default is excluded and why. The owner decides whether the default should change.
3. `AI-15` has no request-set file: `docs/core/specs/ai-15.md` says "None today". I added no file; `AI-8-AC1` is pinned as a Rust test (`look.rs`) and the planner decides where the standard set lives.
4. D5 says "migration 0072 ... If 0072 is taken on main when the PR is opened, the planner renumbers": 0072 is free here (last is 0071).
5. The theme `light` inside a dark console is not possible with the existing `dark` class alone. I added `.theme-light` to the selector list of the light tokens in `design-system/tokens/colors.css` (one line, a comment says it is ours) and a `@custom-variant dark` override in `src/app/globals.css` that stops `dark:` matching under `.theme-light`. The design system folder is a copy in this repo; if it is meant to stay identical to its upstream, move the selector into `globals.css` instead.
6. D4 says a metric's `MetricDef.format` is the default of its value. That field is a free word (<= 20 alphanumerics) with no list behind it; only `number`, `percent`, `compact` and `scientific` are read (`formatFromHint`), any other hint is ignored, not guessed (`currency` is not honoured: it has no code). The tile payload carries `metricFormats` for this. Metric entries now store the hint in the catalog entry body.
7. Feature decision 15 says the two tools do not "wait for approval". They are `WriteLow` like `update_chart` (the plan says copy it), so they wait for the chat's confirmation (`needs_confirmation`) and need no approval.
8. `ReferenceLine.value` became `Option<LineValue>` (absent on a computed line); stored JSON of plain lines is unchanged (tested with a fixture, numbers stay `f64` on both sides as before).
9. Existing builder tests that expected reference lines in "More options" now expect them in Format (the plan moves the editor); `moreOptionsUsed` lost its `referenceLines` field; `SectionNumber` stays 1 to 4. The "four sections" test is "five sections". `PreviewInputs` gained `style`.
10. The dashboard payload carries `theme`/`font` at the top level, because its `board` field is the id string; the embed and public payloads carry them in `board`.
11. `format_chart` keeps the chart's `source` and author (it does not relabel a manual chart as `ai`); a field given replaces the whole field (including `colors`, so a `colors` with only `series` drops a stored `palette`); it reads nothing from the call outside the ten fields.
12. Tooltip fields are limited by the server to bar, hbar, line, area, stacked and combo (the kinds with an axis tooltip); on the other kinds a tooltip selection would be ignored, so it is refused. Colour by value on maps applies to `geomap` and to `pointmap` symbols.

**Smaller things worth a look in review**

- `Add note...` writes the clicked point's label as the note's `x`; for a grained axis that is the label ("Mar 2026"), which is what the axis data holds.
- Layered charts (`BI-13`): per-value formats and second-axis choices there are keyed by the series' names, not by result columns (`StyleInput.columns`).
- The per-value `Format of` select, the series-colour name field and the note fields are plain inputs with a datalist; no visual design pass was done.
- Hidden notes ("N notes not shown") are shown under the tile for charts and layered charts, not for a chart in the expanded dialog (it uses the same `TileBody`, so it shows there too).
- `cargo test -p lakehouse-api --lib` at the filters above was run once more after the last Rust edit; the full lib suite was not run.

## 8. Review (planner appends)

### Planner's review of slice 1 (T1 to T9), 2026-10-15

Deviations in the handoff, accepted:

1. `check_reference_lines` and `check_style` now run for every chart, not
   only on the parameter path: the plan assumed `check_chart_shape` already
   ran everywhere and it did not. Better than planned.
2. **Owner question (record at QA):** the current default palette does not
   pass 3:1 contrast (sky 2.77:1 on white; amber, lime, orange also short).
   Kept so saved charts do not change colour; the four new palettes pass.
   Feature decision 6 is corrected at QA either way: keep the default as it
   is, or change it (every existing chart's colours change).
3. No `AI-15` request file exists; `AI-8-AC1` is pinned as a Rust test.
4. The light-theme selector added to `design-system/tokens/colors.css` (one
   commented line): the folder is ours and has been edited before.
5. A metric's free-text format word is read only for number, percent,
   compact and scientific; anything else is ignored, not guessed.
6. `format_chart` and `set_dashboard_theme` are WriteLow like
   `update_chart`: they wait for the chat's confirmation, not an approval;
   consistent with feature decision 15.
7. to 10. Accepted as written (optional `ReferenceLine.value` with a
   fixture of old JSON; tests moved with the editor; tooltip fields refused
   on kinds without an axis tooltip; `colors` given replaces the whole
   field).

Not verified by anyone yet: every visual result (owner QA), `bun run build`
under Turbopack in this worktree (the `node_modules` symlink; webpack build
passes; CI builds from a real install), the full workspace run (CI). The
reviewer's own re-run was skipped at the owner's request.

### Migration renumbered at the merge of main (planner, 2026-10-17)

`main` took `0072` and `0073` (connector run record, `SRC-11`), so this
branch's `0072_reporting_formatting.sql` is now `0085_reporting_formatting.sql`
and BI-16 B's `0073_reporting_content_sites.sql` is
`0086_reporting_content_sites.sql` (`0078` to `0084` are used on other
branches and `0090` to `0094` are reserved by Data phase 3). The file
contents did not change. A database that applied them as 72 and 73 (only
the dev stack) needs its two `_sqlx_migrations` rows fixed by hand.

**Renumbered again (round M, decision M4, 2026-10-21).** `main` took `0085`
for `0085_quality_incident_stale.sql` (`DATA-14`), so the two files are now
`0086_reporting_formatting.sql` and `0087_reporting_content_sites.sql`, with
their contents unchanged. A database that applied them as 85 and 86 (only the
dev stack) needs its two `_sqlx_migrations` rows fixed by hand.

### Planner review of PR #199 before merge, 2026-10-21 (round M)

One review covers all three items of PR #199: see `docs/superpowers/plans/2026-10-14-bi-18c-layout.md`, section 8, "Planner review of PR #199 before merge, 2026-10-21 (round M)".
