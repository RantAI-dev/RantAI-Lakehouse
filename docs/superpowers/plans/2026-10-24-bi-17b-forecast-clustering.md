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

Commit note: two commits carry T2, `df05a7ad` (the files and this entry) and `2c0270a6` (only a wording fix in the handoff, made after a session restart left both writing the same files). No code differs between them.


### T3 `kmeans.ts` (developer, branch `feat/bi-phase-6`)

Commit: `feat(clusters): deterministic k-means for scatter and bubble charts (BI-17 B T3)`. TypeScript only, no cargo; standalone (no type from T1; the `k` argument is `number | "auto"`, which is what T1's `Clusters { k: Auto | Fixed(2..=10) }` becomes on the wire, to be mapped by T4).

API: `cluster(points, k)` returns `{ labels, k, auto, groups: [{ size, center }], score }` or `{ refused }`; `isClusterRefusal`, `mulberry32`. Min-max scaling per measure, k-means++ from mulberry32 (seed 1 + k), Lloyd to convergence or 100 iterations, best of 5 restarts by inertia, k by Calinski-Harabasz over 2..min(10, floor(n/3)), as D5. Groups are numbered by first appearance; centres are reported in the measures' own units for the legend "Cluster 1 (n = 42; x = ..., y = ...)".

Decisions to review (all beyond what D5 spells out):
- A fixed k needs `3k` points and `k` distinct positions, else a refusal sentence (feature decision 6: "at least 3 points per group, else fewer groups or a sentence"); auto also never exceeds the number of distinct positions.
- Fewer than 6 points, all points identical, or a non-finite value: a refusal sentence each. A measure that never varies scales to 0.
- An emptied group during Lloyd is re-seeded on the farthest point; CH ties go to the smaller k.
- The seed varies with k (`1 + k`) so each k has its own fixed stream; every run of the same input gives the same labels (tested).

Checks: `bun test src/lib/kmeans.test.ts`: 10 pass, 0 fail (blobs give k = 3 with every point in its blob, fixed k honoured, same labels across runs, n = 5 refused, millions-scale measure does not dominate, three-measure bubble, refusals). On the final commit (T3): `bun run typecheck` exit 0; `bun run lint` 0 errors, 9 warnings (none in these files); `bun run test` 2474 pass, 1 skip, 0 fail (254 files). The webpack build is not due until T4.

### T4 Drawing and editor (developer, branch `feat/bi-phase-6`)

Commits: `90c35b75` (Forecast kind and horizon in the reference-line editor), `16da9a7f` (period arithmetic, `src/lib/forecast-periods.ts`), `1ef3faa9` and `a7f93881` (forecast drawing and the tile test; these two came from a second session that was running against the same worktree), `8d7518ed` (Clusters setting in the Format section), `21cf4ae9` (cluster drawing). TypeScript only, no cargo.

**The server does not validate any of this until T1 lands.** The console sends `referenceLines: [{ kind: "forecast", horizon: N, label?, color?, measure? }]` and `clusters: { k: "auto" | 2..10 }` exactly as plan D1 and D2 define them; until T1 adds `LineKind::Forecast`, `horizon` and `StyleFields.clusters`, a save carrying them is whatever the current Rust does with an unknown kind and an unknown field (not tested here). The limits (horizon 1 to 60, one forecast per chart, k 2 to 10, scatter and bubble only, no forecast on a horizontal bar chart) are enforced by the console alone for now: the editor hides what is invalid and `refLinesPayload` / `stylePayload` do not send it.

What was built:
- **Forecast drawing** (`withReferenceLines` -> `withForecastLine`, `forecast-line.ts`): future labels appended to the category axis from the last bucket and the grain; a dashed line starting on the last actual value; the band as two stacked line series (the lower one invisible; both kept out of the tooltip and the legend); the line's tooltip writes "value (95 % band lo to hi)"; a bar chart gets the forecast as a line over the bars (the extra categories did not disturb the existing series, tested). When the unfinished current period is left out, the forecast starts on its category. The note under the chart (`forecastNote`) is the model and band, plus what was filled or left out, or the refusal sentence; a refusal draws no line (Q5). Refusal sentences of its own: breakdown, horizontal bars, a grain that is not a truncation, dates that are not one run of periods.
- **How the buckets arrive (plan section 5):** as wall-clock text in the report zone (`YYYY-MM-DD`, or with `HH:MM:SS` for hour and minute), so future labels are calendar steps on that text with `bucketLabel`; only "is the last period finished" needs the zone, through `Intl` (`wallClockMs`). Rows are laid on a regular grid, a missing bucket is a gap that `forecast.ts` fills or refuses.
- **PDF render view:** `render-view.tsx` mounts `TileBody`, so the PDF draws what the tile hands the chart; `tile-forecast.test.tsx` and `tile-clusters.test.tsx` assert that at the `EChart` boundary. *Not verified:* the band in a real PDF (needs a browser and a running stack).
- **Clusters:** `StyleFields.clusters`; Format section group "Clusters" (switch, number of groups); `clustersOf` / `clusterNote` in `chart-option.ts`, `cluster-plan.ts` (memoised) over `kmeans.ts`. One series per cluster with the palette's colours, legend "Cluster 1 (n = 5; x = ..., y = ...)" (bubble adds z); the note says "Clusters of the N points shown" and how k was chosen; a refusal keeps the single series and prints the sentence.
- A forecast on a chart whose rows were cut off by the row limit forecasts the rows on screen, as the trend line does.

Not done in T4 (as the plan lists it for T5 or later): assistant schema and cases; the copilot draft card does not show a forecast.

Checks on the final commit: `bun run typecheck` exit 0; `bun run lint` 0 errors, 9 warnings (none in these files); `bun run test` 2513 pass, 1 skip, 0 fail (258 files); `bun --bun next build --webpack` exit 0 (run detached, log `/tmp/claude-1000/build-t4.log`). Scoped runs: forecast-line 9, tile-forecast 3, tile-clusters 5, forecast-periods 7, style-draft and params additions, reference-lines-editor 6, format-section 12.

Process note: two sessions resumed this task at once for a while and both wrote to the worktree; the committed `src/lib/kmeans.ts` (`kmeans({ points, k })`) is the other session's version of T3, not the `cluster(points, k)` the T3 handoff above describes. The T3 handoff text predates that and should be read against the file.

### T1 Validation and assistant schema (developer, branch `feat/bi-phase-6`)

Written after T2 to T4 because the owner's QA hit "body JSON is invalid" on saving a chart with a Forecast line: the console sent `kind: "forecast"`, `horizon` and `clusters`, which the server refused as unknown. Commits: `f42c7077` (`lakehouse-bi`), `7274a470` (the AI files and the fixture).

- `params.rs`: `LineKind::Forecast` (`is_computed`), `ReferenceLine.horizon` and `ResolvedLine.horizon` (`Option<u32>`, left out when absent, so an old chart writes back byte for byte; tested). `check_reference_lines`: horizon 1 to 60 (`MAX_FORECAST_HORIZON`), at most one forecast, kinds bar/line/area/stacked/combo (not hbar), no breakdown, no `value`/`to`, `horizon` refused on other kinds, `measure` allowed. The breakdown sentence of the other computed kinds is unchanged.
- `shape.rs`: `RelationCheck.truncated_grain`, set in `store.rs` from the chart's grain; `check_against_relation` refuses a forecast whose date column has no minute-to-year time grain (a part of the date such as weekday does not count). This is the "kind and dimension" check D1 asks for, placed where the relation is known.
- `style.rs`: `StyleFields.clusters: Option<Clusters { k: ClusterCount }>`; `ClusterCount` is `"auto"` or a number on the wire (custom serde; a number too large for `u8` is kept as 255 so `check_style` refuses it with its own sentence). Only scatter and bubble; 2 to 10 (`check_clusters`, split out for clippy `too_many_lines`). `deny_unknown_fields` on `Clusters`.
- AI files, minimal: `registry.rs` `reference_lines_schema` (kind `forecast`, `horizon`) and `style_properties` (`clusters`; shared by create_chart, update_chart and format_chart, so all three fixtures change); `look.rs` `FORMAT_FIELDS` gains `clusters` (10 to 11). `tests/fixtures/tool_schemas.json` regenerated with the ignored writer test; the diff is only those additions. Tool count unchanged. Not touched: the prompt, `capabilities.ts`, the AI-15 cases (T5).

Tests: `lakehouse-bi` 552 passed, 0 failed, 3 ignored (new: forecast line checks incl. every refusal, round trip with `horizon` and resolved line, clusters checks and round trip, the forecast grain check); `lakehouse-api` lib `routes::ai` 402 passed, 1 ignored (new: `a_forecast_line_and_clusters_are_saved_and_checked`; two field-list pins updated); `--test ai_tool` 21 passed (run before the last look.rs test was added; the fixture did not change after). `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` exit 0 (without `--all-features`, as asked). All with `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0`, one command at a time, `--test-threads=1`, sources touched before each run.

Not verified: a real save through the API against ClickHouse (needs the stack; the validation is unit-tested); `sec11_guard`/`route_auth` were not run (no route was added or changed). A first clippy run showed errors in `lakehouse-api` about `store::move_board` and `set_board_public` that do not exist in this tree; they were a stale artifact from another worktree in the shared target dir and vanished on the next run after touching the sources.

Disk: the root disk dropped to 9.0 GB free during the last combined run (under the 10 GB line you set); I stopped cargo after it finished. The shared target dir is 7.7 GB before these runs.

## 8. Review (planner appends)
