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

### T2 `forecast.ts` (developer, branch `feat/bi-phase-6`)

Files: `src/lib/forecast.ts`, `src/lib/forecast.test.ts`, `src/lib/forecast-reference.json`, `ops/forecast/make_reference.py`. TypeScript only; no cargo command run, and nothing depends on T1: the module has its own types (`ForecastInput`, `Forecast`, `ForecastRefusal`) and takes the grain as a string.

Reference numbers (statsmodels, run on this machine, packages installed with `pip --target` into a scratch directory, not into the repo or the user site):

    python3 -I ops/forecast/make_reference.py <dir with numpy and statsmodels> src/lib/forecast-reference.json

`statsmodels` 0.15.0, `numpy` 2.5.4. Series: `AirPassengers` (144 months, log scale, via `get_rdataset`) and the first 80 quarters of `macrodata` `realgdp` (log scale; trend, no season). Output of the script: `airline AAA {'ANN': -230.631, 'AAN': -146.013, 'ANA': -473.915, 'AAA': -495.581}` and `gdp AAN {'ANN': -450.485, 'AAN': -491.266, 'ANA': -439.725, 'AAA': -481.863}` (AICc per model). The TypeScript chooses the same model on both and reproduces every point forecast and every 95 % bound to better than 1e-6 relative (the tests allow the plan's 1 % and 3 %). Measured worst deviation: points 4e-8, bounds 9e-8, parameters equal to four digits.

Decisions the plan left open, for review:
- **Initial states** follow statsmodels' `heuristic` start (classical decomposition and a regression on ten points), not its estimated start, so both implementations optimise the same likelihood from the same states. D3 says the same.
- **AICc counts** the smoothing parameters, the initial states (seasonal ones as `m`) and the variance, the usual convention; statsmodels' own `aicc` leaves heuristic initial states out, so the script recomputes it from the log-likelihood and the test compares AICc to 0.1 %.
- **The search** is Nelder-Mead from four starts, with the parameters projected into 0 < alpha < 1, 0 < beta <= alpha, 0 < gamma <= 1 - alpha (a projection, not a penalty, so bound solutions such as alpha = 0.9999 are reached).
- **Seasons** are offered when there are at least `max(2m, 10 + 2 floor(m/2))` periods: two full seasons (the plan) and enough smoothed points for the regression (what the decomposition needs). Fewer gives a note saying which. For 52 weekly periods that is 124 points.
- **Gaps:** leading missing periods are skipped; a missing latest period is refused; more than 10 % missing is refused; up to 10 % filled in a straight line and counted in `notes`.
- **Current period:** the caller passes the exclusive end of the last period (`lastPeriodEnd`) and, in tests, `now`; the module has no zone logic, T4 owns the bucket arithmetic (plan section 5, first item, still to verify there).
- **Horizon** is clamped to 1..60 and to the history length, with a note.
- A constant series gives a flat forecast with a zero-width band (the SSE is floored at 1e-300).

Checks: `bun test src/lib/forecast.test.ts` 15 pass, 0 fail; `bun run typecheck` exit 0; eslint on the two files clean; full `bun run test` and `bun run lint` counts are in the T3 entry (run once after both modules).


### T2 `forecast.ts` (developer, branch `feat/bi-phase-6`)

Commit: the one titled `feat(forecast): additive exponential-smoothing forecast (BI-17 B T2)`. TypeScript only; no cargo command run. Standalone: it imports only the `Grain` type of `time-grain.ts`; nothing from T1.

API: `forecast({ values, grain, horizon, lastPeriodEnd?, now? })` returns a `Forecast` (model ANN/AAN/ANA/AAA, label, parameters, AICc, `point`/`lower`/`upper`, notes) or `{ refused: sentence }`; `isRefusal`, `seasonLength(grain)`. The caller (T4) passes one value per consecutive period (`null` for a missing bucket, so the bucket arithmetic stays with the drawing) and the end of the last bucket; `now` is a parameter, which is how the tests fake the clock.

Reference numbers (committed in `src/lib/forecast-reference.json`), produced offline by:

    pip install --target <dir> numpy scipy pandas statsmodels
    python3 -I ops/forecast/make_reference.py <dir> src/lib/forecast-reference.json

with `statsmodels` 0.15.0 and `numpy` 2.5.4, `ETSModel(error="add", trend/seasonal "add" or None, initialization_method="heuristic")`. Series: `AirPassengers` (144 monthly points, log scale; Rdatasets download) and the first 80 quarters of `statsmodels.datasets.macrodata` `realgdp` (log scale; trend, no season). Output: airline selects AAA (AICc ANN -230.6, AAN -146.0, ANA -473.9, AAA -495.6), gdp selects AAN (ANN -450.5, AAN -491.3, ANA -439.7, AAA -481.9). The TypeScript selects the same models, and its parameters, points and band agree with `statsmodels` to about 1e-7 relative (measured worst: point 4e-8, band 9e-8), against the plan's 1 % and 3 %.

Decisions to review:
- **Initial states = statsmodels' "heuristic" start**, not its default "estimated" one, so the comparison is against an independent fit of the same problem, not of a different one. The plan's D3 also says decomposition-based starts.
- **AICc counts the initial states and the variance as parameters** (Hyndman et al.), recomputed in the reference script from the log-likelihood because `statsmodels` leaves heuristic states out of its own `aicc`. A model with `n - k - 1 <= 0` is not a candidate (a short series falls back to ANN).
- **Seasons need `max(2m, 10 + 2*floor(m/2))` points**: 2 full seasons, as signed, plus what the decomposition needs to start (24 for months, 8 for quarters, 16 for days, 124 for weeks). Below 2 seasons the note says "fewer than 2 full seasons"; between the two limits, "too few to start them". Year and minute have no seasons.
- **Thresholds** as in the feature page: under 5 periods refused (counted after the unfinished period is dropped, and the sentence says so); more than 10 % periods missing refused, up to 10 % filled in a straight line and counted; a missing latest period is refused, leading gaps are trimmed; horizon held to 1..60 and to the history length, with a note.
- Ties in AICc go to the simpler model; a zero-error series is floored so the log stays finite (constant series: flat ANN forecast).
- The `statsmodels` closed-form variances were checked against the generic sum c_j = alpha + j beta + gamma [m | j] used here (they agree by the numbers above, all four models' shapes: AAN, ANA via airline/gdp alternatives are covered only for the selected models; ANN and the other unselected models are not compared one by one).

Checks (from `/home/hv/lakehouse-phase6-build`): `bun test src/lib/forecast.test.ts`: 15 pass, 0 fail; `bun run typecheck`: exit 0; eslint on the two files: clean.


## 8. Review (planner appends)
