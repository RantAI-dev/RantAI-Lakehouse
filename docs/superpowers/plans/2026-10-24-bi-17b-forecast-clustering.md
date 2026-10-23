# BI-17 part B: forecast and clustering (with AI-8, forecast half)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-6` (to be cut; shared with `BI-16` C and `BI-14`),
base `feat/bi-phase-4` until phase 4 merges, then `main`. Feature page:
`docs/core/features/forecast-and-clustering.md` (decision 1 waits for the
owner, Q5; the rest planner defaults confirmed at the phase 6 QA). Spec:
`docs/core/specs/bi-17.md` part B. Builds on `BI-17` part A (#199, merged:
`referenceLines` with computed kinds, the colours and style fields).

**Mostly TypeScript.** One short Rust task (validation and the assistant
schema); the forecast and k-means maths are TS modules computed from the
rows on screen, like `trendEnds`. This item can run beside another item's
Rust build once T1 is committed.

Anchors were read on 2026-10-23 on `feat/bi-phase-4` at `150a081a`. Find
each by the symbol named.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions.

- **D1 Forecast is a reference line.** `lakehouse_bi::params::LineKind`
  gains `Forecast` (computed, `is_computed` true); `ReferenceLine` gains
  `horizon: Option<u32>` (required for `Forecast`, 1..=60, refused on other
  kinds). At most one `Forecast` per chart. `check_reference_lines` gains
  the kind's checks, and a check with the chart's kind and dimension
  (`line`, `area`, `bar`, `stacked`, `combo` over a date or date-time
  dimension with a grain) at save in `shape.rs` `check_kind_fields`'s
  neighbourhood. Reusing `referenceLines` means the editor, the
  validation and `format_chart` (AI-8) already carry it.
- **D2 Clusters are a style field.** `StyleFields` gains `clusters:
  Option<Clusters { k: Auto | Fixed(2..=10) }>`, valid on `scatter` and
  `bubble` only.
- **D3 Forecast maths (`src/lib/forecast.ts`, pure).** Additive ETS
  models ANN, AAN, ANA, AAA (no damping, no multiplicative); parameters
  by minimising the sum of squared one-step errors with bounded
  Nelder–Mead over α, β, γ in (0.0001, 0.9999) (β ≤ α, γ ≤ 1 − α),
  initial states by the classical decomposition of the first two seasons
  (or the first points for non-seasonal); selection by AICc; point
  forecasts and the 95 % interval from the analytical variance of the
  additive class (Hyndman, Koehler, Ord, Snyder 2008, class 1;
  *Forecasting: Principles and Practice*, 3rd ed., section 8.7, table of
  variance expressions). Season length from the tile's `grain`
  (`month` 12, `quarter` 4, `week` 52, `day` 7, `hour` 24, `year` none,
  `minute` none). Thresholds and gap filling as feature decisions 2–4;
  every refusal returns a `{ refused: sentence }` the chart prints.
- **D4 Forecast drawing.** In `withReferenceLines` (`chart-option.ts`):
  future x labels made from the last bucket and the grain (in the report
  time zone, the same bucket labelling as the server's `grain`); a
  dashed line series for the point forecast and a stacked transparent +
  shaded area pair for the band; tooltip with the value and the band; a
  note line "Forecast: <model>, 95 % band" or the refusal sentence under
  the chart. For `bar`, the forecast is drawn as a line over the bars.
- **D5 k-means (`src/lib/kmeans.ts`, pure).** Min–max scaling per
  measure, k-means++ seeding from a fixed-seed PRNG (mulberry32, seed 1),
  Lloyd iterations to convergence or 100, 5 restarts keeping the lowest
  inertia, k chosen by the Calinski–Harabasz index over 2..=min(10,
  ⌊n/3⌋); n < 6 → refusal sentence. Points coloured by cluster with the
  chart's palette; the legend "Cluster 1 (n = 42; x̄ = …, ȳ = …)".
- **D6 Assistant (AI-8).** The `format_chart` schema in
  `routes/ai/registry.rs` gains `forecast` in the line-kind enum,
  `horizon`, and `clusters`; `look.rs` passes them (its list of style
  fields gains `clusters`); the prompt tells the assistant to read the
  chart's note for the model or the refusal; fixture regenerated with the
  ignored writer test; two `AI-15` cases.

## 2. Anchors (verified at `150a081a`, find by symbol)

- Lines: `lakehouse-bi/src/params.rs` `LineKind` (`Value`, `Average`,
  `Median`, `Min`, `Max`, `Trend`, `Band`), `LineKind::is_computed`,
  `ReferenceLine` (`value`, `kind`, `to`, `label`, `color`, `measure`),
  `MAX_REFERENCE_LINES` (5), `check_reference_lines`;
  `lakehouse-bi/src/shape.rs` `ShapeFields.reference_lines` (wire name
  `referenceLines`), `check_kind_fields`.
- Style: `lakehouse-bi/src/style.rs` `StyleFields` (format, colors, axes,
  labels, annotations, tooltip, conditional), `StyleFields::is_used`.
- Console: `src/features/dashboards/chart-option.ts` `withReferenceLines`,
  `trendEnds`, `LINE_NAME`, the scatter/bubble branch; `src/lib/params.ts`
  `LINE_KINDS`, `refLinesFromDef`, `refLinesPayload`; `src/lib/chart-style.ts`
  `ReferenceLineKind`; `src/features/dashboards/reference-lines-editor.tsx`;
  `style-option.ts`; `src/lib/style-draft.ts`.
- Grain on a tile: `routes/support.rs` `annotate_grain` (the tile's
  `grain`), payload `appliedGrain`, `reporting.timeZone`; console
  `src/lib/grain-switch.ts`.
- Assistant: `routes/ai/tools/look.rs` (the style field list including
  `referenceLines`, `check_reference_lines` call), `routes/ai/registry.rs`
  `format_chart` schema, `capabilities.ts`, `ops/ai_eval/dashboard_cases.json`.

## 3. Tasks (one commit each, in order; Rust first)

**T1 Validation and assistant schema (D1, D2, D6 Rust half).** *Accept:*
`params` tests: `forecast` needs `horizon` 1..=60, refused with a `to`
or `value`, at most one per chart; kind/dimension check (refused on a
pie, on a bar over a text dimension); `clusters` only on scatter/bubble,
k 2..=10 or "auto"; serde round-trip keeps old charts byte-identical
(no field written when absent); `look.rs` test: `format_chart` with a
forecast line and with clusters saves; fixture regenerated; count pin
unchanged (no new tool).

**T2 `forecast.ts` (D3).** *Accept:* unit tests against reference
numbers produced offline by `statsmodels` `ETSModel` (additive error,
the four models) on two public series committed as small fixtures
(monthly airline passengers on a log scale, 144 points; a quarterly
series with trend, no season): selected model equal, point forecasts
within 1 %, interval bounds within 3 %; the handoff quotes the Python
command, the `statsmodels` version and its output. Thresholds: 4 points
refused; 23 monthly points → no seasons (fewer than 2 full seasons) and
says so; 12 % gaps refused; 8 % gaps filled and counted; the unfinished
current period dropped (fake clock).

**T3 `kmeans.ts` (D5).** *Accept:* three well-separated blobs → k = 3
chosen and every point in its blob; fixed k honoured; same input → same
labels across runs; n = 5 refused; scaling makes a measure in millions
not dominate.

**T4 Drawing and editor (D4).** Forecast in `withReferenceLines`;
cluster colouring in the scatter/bubble branch; the editor's Forecast
kind with a horizon field; Clusters section. *Accept:* option tests
(future labels for month/quarter/week/day in a non-UTC report zone,
band series stacked under the line, refusal sentence in the note, bar
chart with forecast line); builder round-trip; the PDF render view shows
the band (component test of the render page with a fixture payload).

**T5 Assistant cases (D6 TS half).** `capabilities.ts` mention, prompt
line test, two `AI-15` cases. *Accept:* tests green; cases listed in the
handoff.

## 4. Out of scope

Multiplicative or damped models, ARIMA, forecast from other columns,
forecasts in downloads, clusters on more than three measures or saved to
a table, server-side computation.

## 5. Must verify, not assume

- How a tile's date buckets arrive in the rows (text in the report zone,
  or timestamps) for each grain, so the future labels are made the same
  way (`annotate_grain`, `grain-switch.ts`).
- That a bar chart's category axis accepts the extra future categories
  without breaking `BI-17` A's data labels and annotations.
- The variance expressions for the four additive models, checked against
  the reference implementation's intervals in T2 rather than copied.
- The scatter row cap in force for the shown points (the `BI-5` map/row
  limits if phase 5 has merged), for the note's N.

## 6. Checks

As in the phase 4 plans (same cargo environment:
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0`, one cargo command at a time, `--test-threads=4`,
`route_auth` at 2; check `free -g` / `df -h /` first; never `cargo build -p
lakehouse-api`; no `rust/target`; no prettier; touch the crate sources
before clippy when several worktrees share the target dir). T1 only: the
crates `lakehouse-bi` and `lakehouse-api` (lib filters `params`, `shape`,
`style`, `look`, `registry`; integration `ai_tool`). Once, before the
handoff: `cargo fmt --check` and whole-workspace `cargo clippy
--workspace --all-targets --all-features -- -D warnings` with the
environment above (no full workspace test run on this machine; CI runs
it on the PR). T2–T5 run no cargo command: TS typecheck, lint, tests of
the files touched; the webpack build after T4.

## 7. Handoff (developer appends)

## 8. Review (planner appends)
