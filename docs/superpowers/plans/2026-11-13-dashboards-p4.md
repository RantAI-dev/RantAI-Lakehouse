# QA round, Dashboards (P4): a year column's bins, an alert's average, a sum's decimals

Backlog task `BI-57` (P2, Dashboards), which is part 2 of `BI-55`. QA case:
`LH-BI-2-02` (F1). For context only: `LH-BI-6-04` (the average; fixed in part 1, `#503`).
No other QA case is fixed here.

Base: `main` at `65cf0358b` (every `file:line` below was read there). If `main` moves before
the build starts, **re-find an anchor by the text quoted with it, not by its line number**.

History, read before anything else: `docs/superpowers/plans/2026-11-12-qa-round-dashboards-p3.md`
(part 1). Its T1/D4 shipped (an average, a minimum and a maximum keep six decimals through
`Aggregate::rounded`, `rust/crates/lakehouse-bi/src/specs.rs:190-210`; a sum stays whole). Its
T2/D5 (bins of a whole-number column never narrower than 1, labelled "2020") was **wrong and
was undone** in review round 1 (R1, R2): with a width of 1 over 2020 to 2024 the last bin is
closed and holds the maximum, so the bin labelled "2023" held 2023 and 2024. Round 1 R7 named
the alert statement that still rounds an average. This plan fixes all three, the bin first.

What QA saw (`LH-BI-2-02` F1, Indonesian in the case, in English): a year column picked as
the dimension starts on "Automatic (about 10 bins)" and its bars read "2,020 – 2,020.5": a
thousands separator and a decimal on a year. The five years 2020 to 2024 should read 2020,
2021, 2022, 2023, 2024.

Slices (two pull requests):

- **S1** = parts 1 and 2: the year bins (T1, T2), alerts round like charts (T3), the
  documents (T4). Branch `fix/dashboards-p4`. One draft pull request, opened by the planner
  as soon as the first commit is pushed.
- **S2** = part 3: a sum keeps six decimals (T5). **Waits for pull request #504**
  (`fix/qa-round-dashboards-p2`): #504 adds a test that asserts `round(sum(visitors)) AS
  visitors` (`store.rs`, `a_radar_with_two_measures_is_saved_with_both_columns`), which S2
  changes by meaning. S2 starts only after the planner says #504 and S1 are merged, on
  `fix/dashboards-p4-sums` from the `main` of that moment. If the owner says no to S2 (open
  question 5), T5 is not built and the follow-up line in section 4 stays.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer writes
product code and tests and edits documents only where a task says so. If an anchor, a
signature or a quoted text below does not match the code, stop and report the mismatch in
the handoff; do not improvise. If the engine answers a different number than a test here
expects, stop and report the statement and the answer; do not change the expected value
(rule 2).

## 1. Decisions

### Process

- **D1. One task per commit, in the order of section 2** (Rust first: T1 `lakehouse-bi`,
  T2 `lakehouse-api` with its console half, T3 `lakehouse-alerts`; then T4 the documents).
  Each commit body cites `BI-57` and, for T1 and T2, `LH-BI-2-02`. Each fix site carries a
  short comment naming the case (rule 13), e.g. `// QA LH-BI-2-02 (BI-57, D3): …`; T3's
  site names `BI-57` and `#503 review R7`. Push after every commit.
- **D2. No new route, no migration, no new permission, no change to a stored shape.**
  `POLICY_TABLE` and `tests/route_auth.rs` do not change. The records route keeps its query
  (`binWidth` only, `routes/dashboard.rs:3315-3347` is not edited). The only new field on the
  wire is additive: `whole` inside a tile's `bins` entry (D6).
- **D2a. Assistant (assistant parity).** No tool, schema or prompt change, and
  `rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json` is not regenerated. Reason: no
  tool input changes. The assistant sends the same `bin` (`auto`, `count`, `width`) and the
  same `aggregate` words; the server decides the width, the bins and the rounding. A tile's
  `bins` entry gains `whole`, which a tool that hands a tile to the model passes on as data.
  The page context (`src/lib/page-context-summary.ts:111`) calls `binLabel` with two
  arguments and keeps saying "tahun in bins of 1 (a value is the bin's lower edge)", which
  stays true; that file is not edited (it is also in #504). `ops/ai_eval/dashboard_cases.json`
  was read at `65cf0358b`: no case sets a bin, asserts a bin's label, an average's value, a
  table-column alert's value or a decimal sum (its one number check, line 389, reads
  `jumlah_event`, a whole number), so no line of it changes, in S1 or S2. The AI team
  reviews this paragraph.

### Part 1: a year column's bins (`LH-BI-2-02`)

- **D3. The rule.** Over a column whose values (over the rows the bins are cut from) are all
  whole numbers, **a bin of width exactly 1 holds one value, and the maximum has a bin of its
  own**. Everywhere else the rule of `BI-10` is unchanged: a bin includes its lower edge and
  excludes its upper edge, and the last bin also holds the maximum.
  - Why: with whole numbers and width 1, the bin `[2023, 2024)` holds exactly 2023, so every
    bin is one value and can be named by it. The last-bin clamp exists so that the maximum
    does not sit alone in a bin of its own; for one value per bin, alone is right.
  - "Whole" is read from the data, not from the column's name or type, in the same
    statement that reads the minimum and maximum: `min(toFloat64(c) = floor(toFloat64(c)))`
    over the same filtered rows (a fourth scalar, `__bwh_<c>`, beside `__bmn_<c>`, `__bmx_<c>`,
    `__bw_<c>`). The builder cannot know it otherwise: its column kinds are `Number`, `Date`,
    `DateTime`, `Text` (`rust/crates/lakehouse-bi/src/filters.rs:469-479`), and `Number`
    covers `Int*`, `UInt*`, `Float*` and `Decimal*`.
  - **One rule for every bin mode** (automatic, counted, a typed width, and the histogram's
    modes). Reason: the records list of a clicked bin (`records_sql`) and the dashboard
    filter of a clicked bin (`isLastBin`) know only the bin's width, never its mode
    (`routes/dashboard.rs:3323-3346` reads `binWidth` only), so a rule that depended on the
    mode would list or filter rows that the bar does not hold. A typed width of 1 over whole
    numbers therefore also gives the maximum its own bin, and so does a histogram whose
    range is 1 wide (one more range than today, within the "can cost one range over `n`" the
    module doc already states for histograms).
  - The exception is written in exactly three places, each the mirror of the others:
    `Range::bin_expr` (the bin), `bin_records_predicate` (the records of a clicked bin) and
    `isLastBin` (the dashboard filter of a clicked bin). With it, **no bin is closed above**:
    the closed-above clause and the "last" flag apply only when the exception does not.
- **D4. The narrow-range floor, automatic and counted bins only.** Automatic and counted bins
  of a whole-number column are never narrower than 1, and they use one bar per value only
  when that many bars fit the count: if one bin per value from the minimum to the maximum
  would be more than `n` bins (`mx - mn + 1 > n`), the width is 2. In the `Nice` statement,
  after today's choice (named `w0`):
  `multiIf(wh = 1 AND round(w0, 10) <= 1 AND mx - mn < n, 1, wh = 1 AND round(w0, 10) <= 1, 2, w0)`.
  - Why the second branch can only be reached at `w0 = 1`: a nice width below 1 is at most
    0.5, and if it fits `n` bins then `mx - mn <= n / 2 < n`. So width 2 appears only when
    today's choice was 1 and `mx - mn = n` (eleven years under "Automatic"); width 2 then
    gives at most `n / 2 + 1` bins.
  - `round(w0, 10)` because `10 * 0.1` and `100 * 0.01` are floating-point products; the
    width the statement then uses is the literal `1`, so `bin_expr`'s exact `= 1` holds.
  - A typed width is honoured as typed (0.5 stays 0.5). The histogram's two width rules
    (`Exact`, `Nearest`) get no floor (section 4).
  - What it gives with "Automatic" (10 bins), all on the engine in T1's test:

    | Years | Today | After |
    | --- | --- | --- |
    | 2020–2024 (5) | width 0.5, "2,020 – 2,020.5" | width 1, five bars 2020 … 2024 |
    | 2019–2024 (6) | width 0.5 | width 1, six bars |
    | 2015–2024 (10) | width 1, nine bars, the last holds 2023 and 2024 | width 1, ten bars |
    | 2014–2024 (11) | width 1, ten bars, the last holds two | width 2, five bars, the last ("2,022 – 2,024") holds 2022, 2023, 2024 |

- **D5. The label.** A bin reads as its one value only when it holds one value: the tile
  says the column is whole (`whole`, D6), the width is exactly 1 and the lower edge is a whole
  number. It reads `String(lower)`: "2020", with no separator, the text an unbinned number
  category shows (`categoryLabel`, `src/lib/chart-transforms.ts:23`, is `String(v)` for a
  number). Every other bin keeps its range label ("2,014 – 2,016"), including a whole column
  in bins of 2, a SQL source's preview (which reads no range, so the console does not know
  the column is whole) and a histogram (`type-b-option.ts:69` calls `binLabel` with two
  arguments and is not edited). Fail honestly: if `whole` is missing, the label stays a range.
- **D6. `whole` on the wire.** `bins: { <column>: { mode, width, min, max, whole? } }`.
  `whole` is the engine's `wh` (`1` or `0`), present whenever the read returned a non-null
  value (every mode now reads it); absent from an older server and from a read with no rows.
  The console reads it as true only when it is `true` or a finite `1`.

### Part 2: alerts round as charts do

- **D7. Reuse `Aggregate::rounded`.** `lakehouse-alerts` already depends on `lakehouse-bi`
  (`rust/crates/lakehouse-alerts/Cargo.toml`, `lakehouse-bi = { path = "../lakehouse-bi" }`;
  `lakehouse-bi` depends only on `lakehouse-core` and `lakehouse-clickhouse`, so there is no
  cycle). `mart_statement` (`lakehouse-alerts/src/lib.rs:1811-1825`) calls
  `lakehouse_bi::specs::Aggregate::from_str_lossy(agg).rounded(measure_ident.as_str())`
  instead of writing `round({agg}(x))`. `from_str_lossy` is safe here because `agg` was
  checked against `AGGS` two lines above; the `count` branch stays as it is. No new crate, no
  moved function (rule 4).
- **D8. What changes for an existing alert.** Only a **table-column** threshold rule (mart,
  measure, aggregate) on an **average, a minimum or a maximum** changes: it now compares the
  value with six decimals, as the chart shows it, instead of the whole number. Example: an
  average of 1466.18 with "above 1466" did not fire (1466 is not above 1466) and now fires;
  with "below 1466.5" it fired (1466) and now does not. A rule near its line may therefore
  fire or recover at a different check than before, once, without anyone editing it. A
  `sum` or `count` rule does not change in S1. A rule on a chart or a saved metric already
  read six decimals since part 1 (it uses the chart's or the metric's own statement,
  `metric_statement`, `lib.rs:1889`). The alert's message still shows a whole number
  (`fmt_id_id`, `lib.rs:2031`), so a message can read "= 1.466 > 1.466 (threshold breached)":
  named on the feature page and in section 4, not changed here. Written in the changelog and
  on `docs/core/features/sharing-and-delivery.md` as *(proposed, not yet signed)*.

### Part 3: a sum of a decimal column (S2)

- **D9. The builder does not know the column's type, and does not need to.** It knows a
  column only as `ColumnKind::Number` (`filters.rs:469-479`; `RelationColumns` is a map of
  name to kind, `builder.rs:1477-1479`), so "integer or decimal" is not known at build time,
  and threading the type through every caller of `RelationColumns` is out of proportion.
  It is not needed: `round(x, 6)` of an integer in `ClickHouse` is the integer, of the same
  type (the same fact D4 of part 1 relied on for a minimum and a maximum). So S2 writes
  `round(sum(x), 6)` for every sum: a sum of whole numbers stays the same whole number (and
  the same 64-bit type the engine quotes in JSON), a sum of a `Float*` or `Decimal*` column
  keeps six decimals. T5's engine test proves both before any pinned text is trusted.
  - Cost, and why it is a slice of its own: 44 pinned assertions hold `round(sum(` (listed in
    T5), plus the test #504 adds. The built-in dashboards' statements
    (`rust/crates/lakehouse-bi/specs/builtin-default.json`, `src/lib/dashboard-specs.ts`) are
    hand-written, do not go through `Aggregate::rounded` and sum whole-number columns; they
    are **not** changed, so the drift guard `specs_match_typescript` is untouched.
  - A plain chart saved before S2 is read with its stored statement when no dashboard filter
    applies (`builder.rs:1710-1724`), so a sum of a decimal column there stays whole until the
    chart is saved again, and shows decimals when a filter applies: the same limit part 1
    accepted for averages (its open question 4). With "As it is now" a large sum shows about
    four significant digits either way; the difference is seen with a format that sets
    decimals.
  - Alerts follow without an edit of their own: after T3 they call the same function.

### Words on screen (English, as the console). Not yet seen by the product owner.

- A bin of one value over whole numbers: the value, written as it is stored ("2020").
- No other new text.

### Shared files with pull request #504 (`fix/qa-round-dashboards-p2`, open)

Read from `gh pr diff 504` on 2026-11-13. Line numbers are `main`'s at `65cf0358b`.

| File | This plan | #504 | Risk |
| --- | --- | --- | --- |
| `rust/crates/lakehouse-api/src/routes/support.rs` | T2: doc of `annotate_bins` `:1045-1051`, the object in `add_bins` `:1087-1094` | a new function after `:994` | Different functions, about 50 lines apart |
| `src/features/dashboards/dashboard-page.tsx` | T2: one argument at `:1017` | the import at `:16-23` (it keeps `isLastBin`), `:129`, `:222`, `:325`, `:353`, `:473`, `:1086`, `:1153` | Nearest pair 70 lines apart. Do not edit the import line |
| `rust/crates/lakehouse-bi/src/store.rs` | S2 only (T5): five assertions | a new test after `:7983` asserting `round(sum(visitors))` and `round(avg(stay))` | **By meaning.** S2 waits for #504; T5 then edits #504's test too |
| `docs/core/features/chart-builder-questions.md` | S2 only (T5): line 57 | a new last row of Decisions, a new last line of Limits | Different lines |

Not shared: `src/lib/chart-bins.ts`, `src/lib/chart-bins.test.ts`, `bin.rs`, `builder.rs`,
`binned.rs`, `lakehouse-alerts`, the two feature pages of T4. Not edited by this plan though
#504 edits them: `page-context-summary.ts`, `tile-body.tsx`, `routes/dashboard.rs`,
`ops/ai_eval/dashboard_cases.json`.

## 2. Tasks

S1: T1 to T4. S2: T5.

### T1. In bins of 1 over whole numbers every value is its own bin, and automatic bins of such a column are never narrower than 1 (LH-BI-2-02; BI-57, D3, D4)

- Anchors, `rust/crates/lakehouse-bi/src/bin.rs`: module doc `:13-16` ("the maximum is never
  alone in a bin of its own (`BI-10-AC1`)"), `:31-34` ("one row `(width, min, max)`"),
  `:36-43` ("How the width is chosen"); `Range::mn`/`mx`/`w` `:342-352`; `stats_sql`
  `:354-395` (`l1` at `:358`, `Fixed` `:360`, `Exact` `:361-369`, `Nearest` `:370-379`, `Nice`
  `:380-393`); `scalars_sql` `:397-408`; `read_sql` `:410-418`; `bin_expr` `:420-429`;
  `bin_records_predicate` `:432-459`; `max_scalar_sql` `:465-469`; tests `:670-679`,
  `:681-688`, `:690-698`, `:711-721`, `:817-835`.
  `rust/crates/lakehouse-bi/src/builder.rs`: `Relation::Ranged` doc `:252-259`; `records_sql`,
  the `RecordsValue::Bin` branch `:2049-2066`; tests `:5483-5520`, `:5593-5616`.
  `rust/crates/lakehouse-bi/src/builder/binned.rs`: `BinRead::sql` doc `:37`, `reads` doc
  `:195-196`; tests `:226-249` (must pass unchanged).
- Change, `bin.rs`:
  1. A private helper `fn whole_test(c: &str) -> String` returning
     `toFloat64({c}) = floor(toFloat64({c}))`, and `fn wh(&self) -> String` on `Range`
     returning `__bwh_{column}` (beside `mn`, `mx`, `w`).
  2. `stats_sql`: `l1` becomes
     `SELECT min(toFloat64({c})) AS mn, max(toFloat64({c})) AS mx, min({whole_test}) AS wh FROM {base}`.
     Every level of every branch carries `wh` right after `mx`: `Fixed` →
     `SELECT mn, mx, wh, {w} AS w FROM ({l1})`; `Exact` →
     `SELECT mn, mx, wh, if(…as today…) AS w FROM (SELECT mn, mx, wh, (mx - mn) / {n} AS raw FROM ({l1}))`;
     `Nearest` → `SELECT mn, mx, wh, if(…as today…) AS w FROM (SELECT mn, mx, wh, if(…as today…) AS p FROM ({l1}))`.
     `Nice`: `l2` selects `mn, mx, wh, if(…as today…) AS p`; the `multiIf` of today, branches
     unchanged, becomes `SELECT mn, mx, wh, multiIf(<branches>) AS w0 FROM ({l2})`; the
     branch returns
     `SELECT mn, mx, wh, multiIf(wh = 1 AND round(w0, 10) <= 1 AND mx - mn < {n}, 1, wh = 1 AND round(w0, 10) <= 1, 2, w0) AS w FROM (<that>)`.
     Comment at the `Nice` site: QA `LH-BI-2-02` (`BI-57`, D4): a whole-number column is never
     cut narrower than 1, and one bin per value only when that many fit `n`. Update the doc
     comment to `SELECT mn, mx, wh, w`.
  3. `scalars_sql`: `SELECT w AS {w}, mn AS {mn}, mx AS {mx}, wh AS {wh} FROM (…)`. Doc:
     four scalars.
  4. `read_sql`: `SELECT w AS width, mn AS min, mx AS max, wh AS whole FROM (…)`. Doc: one row
     `(width, min, max, whole)`.
  5. `bin_expr`: the condition of the clamp becomes
     `{mx} > {mn} AND NOT ({wh} = 1 AND {w} = 1)`; nothing else in the expression changes, so
     it still starts `round(least(floor(toFloat64({c}) / {w}), if(`. Doc: the D3 exception.
  6. A new `#[must_use] pub fn whole_scalar_sql(column: &Ident, base: &str) -> String`
     returning `SELECT min({whole_test}) FROM {base}`, beside `max_scalar_sql`, doc: 1 when
     every value of `column` over `base` is a whole number (`BI-57`).
  7. `bin_records_predicate` gains a fifth parameter `whole_sql: Option<&str>`. With
     `max_sql` `Some(max)` and `whole_sql` `Some(whole)` the closed clause is
     `(toFloat64(c) = {upper} AND toFloat64(c) = ({max}) AND ({whole}) = 0)`; with
     `whole_sql` `None` it is today's text, byte for byte. Doc: the clause is left out by the
     engine when the column is whole and the width is 1 (D3); the caller passes `whole_sql`
     only for a width of 1.
  8. Module doc: in `:13-16` add "except over a column of whole numbers in bins of width 1,
     where each value, the maximum too, is a bin of its own (`BI-57`, QA `LH-BI-2-02`)"; in
     `:31-34` the read is `(width, min, max, whole)`; in "How the width is chosen" add two
     sentences: the floor and the width 2 of D4, and that the tile carries `whole`.
- Change, `builder.rs`:
  1. `records_sql`, `RecordsValue::Bin` branch: compute the base text once,
     `let base = format!("{from_sql}{others}");`, keep `max` from it, add
     `let whole = ((width - 1.0).abs() < f64::EPSILON).then(|| crate::bin::whole_scalar_sql(column, &base));`
     (comment: `QA LH-BI-2-02 (BI-57, D3)`; the width is compared with a tolerance because
     `clippy::float_cmp` refuses `==`; the console sends `1` exactly) and pass
     `whole.as_deref()` as the fifth argument.
  2. The `Relation::Ranged` doc: `__bmn_<c>`, `__bmx_<c>`, `__bw_<c>` and `__bwh_<c>`.
- Change, `binned.rs`: the `BinRead::sql` doc reads `SELECT width, min, max, whole`; the
  `reads` doc names the whole flag. No code change.
- Pinned statements: **four assertions change, each to the exact new statement below,
  because the rule changed (rule 2; say so in the commit body)**:
  1. `bin.rs` `the_range_statement_reads_the_column_once_from_the_filtered_base` (`:674-677`):
     `SELECT w AS width, mn AS min, mx AS max, wh AS whole FROM (SELECT mn, mx, wh, 500 AS w FROM (SELECT min(toFloat64(visitors)) AS mn, max(toFloat64(visitors)) AS mx, min(toFloat64(visitors) = floor(toFloat64(visitors))) AS wh FROM serving.t))`
     (its `matches("serving.t").count() == 1` stays).
  2. `bin.rs` `the_bin_is_the_lower_edge_and_the_maximum_stays_in_the_last_bin` (`:694-697`):
     `round(least(floor(toFloat64(visitors) / __bw_visitors), if(__bmx_visitors > __bmn_visitors AND NOT (__bwh_visitors = 1 AND __bw_visitors = 1), ceil(__bmx_visitors / __bw_visitors) - 1, floor(__bmx_visitors / __bw_visitors))) * __bw_visitors, 10)`.
  3. `builder.rs` `a_binned_dimension_reads_the_range_and_the_filters_under_the_bin` (`:5502`):
     `FROM (SELECT * FROM (SELECT * FROM serving.mart_x WHERE stay >= 10) AS flt CROSS JOIN (SELECT w AS __bw_stay, mn AS __bmn_stay, mx AS __bmx_stay, wh AS __bwh_stay FROM (SELECT mn, mx, wh, 500 AS w FROM (SELECT min(toFloat64(stay)) AS mn, max(toFloat64(stay)) AS mx, min(toFloat64(stay) = floor(toFloat64(stay))) AS wh FROM (SELECT * FROM serving.mart_x WHERE stay >= 10) AS flt))) AS rng) AS ranged) AS bkt GROUP BY stay ORDER BY stay`.
  4. the same test (`:5512`): `.starts_with("SELECT w AS width, mn AS min, mx AS max, wh AS whole FROM")`.
  Two calls change their arguments and not their assertions: `bin.rs`
  `a_clicked_bin_is_closed_above_only_when_it_ends_at_the_maximum` (`:714`, `:719`) passes
  `None` as the new fifth argument. Must pass **unchanged**: `builder.rs:5494-5500` (the
  `starts_with` of the bin), `builder.rs:5611-5615` (a clicked bin of width 500: no whole
  read), `binned.rs:226-276`, `bin.rs:681-688`, `bin.rs:817-835`. If any of these fails, stop
  and report.
- Check, unit (`lakehouse-bi`, all fail on today's code):
  - `bin.rs` **`automatic_bins_read_whether_the_column_is_whole_and_never_go_below_one_for_it`**:
    `Range::new(&col("tahun"), BinMode::Auto).read_sql("serving.t")` equals exactly
    `SELECT w AS width, mn AS min, mx AS max, wh AS whole FROM (SELECT mn, mx, wh, multiIf(wh = 1 AND round(w0, 10) <= 1 AND mx - mn < 10, 1, wh = 1 AND round(w0, 10) <= 1, 2, w0) AS w FROM (SELECT mn, mx, wh, multiIf(ceil(mx / (1 * p)) - floor(mn / (1 * p)) <= 10, 1 * p, ceil(mx / (2 * p)) - floor(mn / (2 * p)) <= 10, 2 * p, ceil(mx / (5 * p)) - floor(mn / (5 * p)) <= 10, 5 * p, ceil(mx / (10 * p)) - floor(mn / (10 * p)) <= 10, 10 * p, ceil(mx / (20 * p)) - floor(mn / (20 * p)) <= 10, 20 * p, ceil(mx / (50 * p)) - floor(mn / (50 * p)) <= 10, 50 * p, ceil(mx / (100 * p)) - floor(mn / (100 * p)) <= 10, 100 * p, 1000 * p) AS w0 FROM (SELECT mn, mx, wh, if(mx > mn, pow(10, floor(log10((mx - mn) / 10))), 1) AS p FROM (SELECT min(toFloat64(tahun)) AS mn, max(toFloat64(tahun)) AS mx, min(toFloat64(tahun) = floor(toFloat64(tahun))) AS wh FROM serving.t))))`
    and names `serving.t` once; `Range::new(&col("tahun"), BinMode::Count(50)).read_sql("serving.t")`
    contains `AND mx - mn < 50, 1,`; `scalars_sql("serving.t")` of the automatic range starts
    with `SELECT w AS __bw_tahun, mn AS __bmn_tahun, mx AS __bmx_tahun, wh AS __bwh_tahun FROM (`.
  - `bin.rs` **`a_clicked_bin_of_one_over_whole_numbers_is_not_closed_above`**:
    `bin_records_predicate(&col("tahun"), 2023.0, 1.0, Some("SELECT 9"), Some("SELECT 1"))` is
    exactly
    `((toFloat64(tahun) >= 2023 AND toFloat64(tahun) < 2024) OR (toFloat64(tahun) = 2024 AND toFloat64(tahun) = (SELECT 9) AND (SELECT 1) = 0))`;
    `whole_scalar_sql(&col("tahun"), "serving.t")` is
    `SELECT min(toFloat64(tahun) = floor(toFloat64(tahun))) FROM serving.t`.
  - `builder.rs` **`a_clicked_year_in_bins_of_one_asks_whether_the_column_is_whole_and_a_wider_bin_does_not`**,
    beside `:5593`, with the same relation, columns and filter as that test and
    `RecordsValue::Bin { column: &v, lower: 2023.0, width: 1.0 }`: `got.rows` is exactly
    `SELECT * FROM serving.t WHERE ((toFloat64(v) >= 2023 AND toFloat64(v) < 2024) OR (toFloat64(v) = 2024 AND toFloat64(v) = (SELECT max(toFloat64(v)) FROM serving.t WHERE g IN ('a')) AND (SELECT min(toFloat64(v) = floor(toFloat64(v))) FROM serving.t WHERE g IN ('a')) = 0)) AND g IN ('a') ORDER BY ALL LIMIT 50 OFFSET 0`;
    `got.count` holds `SELECT max` once and `SELECT min(` once. With `width: 2.0`, `got.rows`
    contains no `floor(toFloat64(v)))`.
- Check, engine (`rust/crates/lakehouse-api/tests/chart_numbers.rs`, the file of part 1; add
  to its `seed` and keep T1's table and test as they are). Seed
  `default.bi57_years (tahun UInt16) ENGINE = MergeTree ORDER BY tahun` with the eleven years
  2014 to 2024, one row each, and `default.bi57_stay (stay Float64) ENGINE = MergeTree ORDER BY stay`
  with `1.5, 2.0, 2.5, 3.0, 4.5`. A helper builds, for a column `c`, a mode and a base text
  `b`, the statement the chart runs:
  `SELECT {bin_expr} AS b, count() AS n FROM (SELECT * FROM {b} CROSS JOIN ({scalars_sql(b)}) AS rng) GROUP BY b ORDER BY b`
  (the text `Relation::Ranged` renders, `builder.rs:368-374`), and returns `(b, n)` pairs as
  `f64` through `number`. The base for "the last N years" is
  `(SELECT * FROM default.bi57_years WHERE tahun >= {first}) AS flt`. Module doc: add
  `BI-57` (QA `LH-BI-2-02`) and why the engine decides it.
  - **`five_years_under_automatic_bins_are_five_bars_one_year_each`**: base from 2020:
    `read_sql` answers `width` 1.0, `min` 2020.0, `max` 2024.0, `whole` 1.0; the bins are
    exactly `[(2020, 1), (2021, 1), (2022, 1), (2023, 1), (2024, 1)]`.
  - **`six_and_ten_years_are_one_bar_per_year_and_eleven_years_are_bins_of_two`**: from 2019:
    width 1.0 and six pairs 2019 … 2024 each 1; from 2015: width 1.0 and ten pairs 2015 …
    2024 each 1; from 2014 (all rows): width 2.0 and exactly
    `[(2014, 2), (2016, 2), (2018, 2), (2020, 2), (2022, 3)]`. The same three with
    `BinMode::Count(10)` give the same widths.
  - **`a_fractional_column_keeps_its_width_and_its_last_bin_holds_the_maximum`**: `stay`, base
    `default.bi57_stay`, automatic: `width` 0.5, `whole` 0.0; bins exactly
    `[(1.5, 1), (2.0, 1), (2.5, 1), (3.0, 1), (4.0, 1)]` (4.5 sits in the last bin, as today).
  - **`a_typed_width_of_one_gives_the_last_year_its_own_bin_too`**: from 2020,
    `BinMode::Width(1.0)`: five pairs 2020 … 2024 each 1.
  - **`a_clicked_year_lists_that_year_and_the_last_year_lists_itself`**: base from 2020; for
    `lower` 2023 then 2024, width 1:
    `SELECT tahun FROM {b} WHERE {bin_records_predicate(tahun, lower, 1.0, Some(max_scalar_sql(tahun, b)), Some(whole_scalar_sql(tahun, b)))} ORDER BY tahun`
    answers exactly `[2023]`, then `[2024]`. With `stay`, lower 4.0, width 0.5 and
    `whole_sql` `None`, it answers `[4.5]` (the closed last bin is unchanged).
  Each assertion message quotes the statement. All of these fail on today's code (2020 to
  2024 is width 0.5 today; the functions and the `whole` column do not exist).

### T2. The tile says the column is whole; a bin of one value reads as the value and is never closed above (LH-BI-2-02; BI-57, D3, D5, D6)

- Anchors: `rust/crates/lakehouse-api/src/routes/support.rs:1045-1051` (doc of
  `annotate_bins`), `:1083-1096` (`add_bins`, the object at `:1087-1094`);
  `src/lib/chart-bins.ts:110-114` (`BinMeta`, `BinInfo`), `:121-149` (`binsOfTile`),
  `:151-156` (`binLabel`), `:164-178` (`relabelBins`), `:180-185` (`isLastBin`);
  `src/features/dashboards/dashboard-page.tsx:1017`; `src/lib/chart-bins.test.ts:34-68`
  (existing tests that must pass unchanged); `src/lib/chart-click.ts:66-69`, `:96-97`
  (carry `BinInfo` as it is, not edited).
- Change, Rust (`support.rs`): build the entry as today, then, only when
  `!row["whole"].is_null()`, insert `"whole": row["whole"]` into it. Comment: QA `LH-BI-2-02`
  (`BI-57`, D6). Doc of `annotate_bins`: `{ mode, width, min, max, whole? }`. A read that
  answers no `whole` (an older statement, a mock) gives no key, so no existing assertion on
  `bins` changes.
- Change, console (`src/lib/chart-bins.ts`):
  1. `BinMeta` gains `whole?: unknown`; `BinInfo` gains `whole?: boolean` (comment: every
     value of the column is a whole number, said by the server, `BI-57`).
  2. `binsOfTile`: when `m.whole === true || finite(m.whole) === 1`, the entry also has
     `whole: true`; otherwise no such key (existing `deepEqual` assertions pass unchanged).
     The "fixed width" fallback (`:138-147`) adds no `whole`.
  3. `binLabel(lower: number, width: number, whole = false)`: first line
     `if (whole && width === 1 && Number.isInteger(lower)) return String(lower)` with the
     comment `// QA LH-BI-2-02 (BI-57, D5): a bin of one whole value reads as that value.`
     Update the doc comment.
  4. `relabelBins` passes `bins[column].whole` as the third argument.
  5. `isLastBin(lower, width, max, whole = false)`: first line
     `if (whole && width === 1) return false` (comment: D3, the maximum has a bin of its own,
     so no bin is closed above). Update the doc comment.
  6. `dashboard-page.tsx:1017`: pass `target.bin.whole` as the fourth argument of
     `isLastBin`. Nothing else in that file.
- Check (`src/lib/chart-bins.test.ts`, new tests after the test at `:59-68`; all fail on
  today's code):
  - **"a bin of one over whole numbers reads as its value, with no separator"**:
    `binLabel(2020, 1, true)` is `"2020"`; `binLabel(-3, 1, true)` is `"-3"`;
    `binLabel(2020, 1)` is `"2,020 – 2,021"`; `binLabel(2014, 2, true)` is `"2,014 – 2,016"`;
    `binLabel(0.5, 1, true)` is `"0.5 – 1.5"`.
  - **"the tile's bins say whole only when the server said so"**:
    `binsOfTile({ tahun: { mode: "auto", width: 1, min: 2020, max: 2024, whole: 1 } }, undefined)`
    deep-equals `{ tahun: { width: 1, max: 2024, whole: true } }`; with `whole: true` the
    same; with `whole: 0`, and with no `whole`, it is `{ tahun: { width: 1, max: 2024 } }`.
  - **"a year in a bin of one reads as the year"**:
    `relabelBins([{ tahun: 2020, v: 1 }, { tahun: 2024, v: 2 }], { tahun: { width: 1, max: 2024, whole: true } })`
    deep-equals `[{ tahun: "2020", v: 1 }, { tahun: "2024", v: 2 }]`.
  - **"in bins of one over whole numbers no bin is closed above"**:
    `isLastBin(2023, 1, 2024, true)` and `isLastBin(2024, 1, 2024, true)` are `false`;
    `isLastBin(2023, 1, 2024)` is `true` (unchanged); `isLastBin(2022, 2, 2024, true)` is
    `true`; and `binRangeFilter("tahun", 2023, 1, isLastBin(2023, 1, 2024, true))` deep-equals
    `{ column: "tahun", values: [], op: "between", min: "2023", max: "2024", maxExclusive: true }`.
  - No test drives `add_bins` (it needs the engine, the gate and the policy store together;
    none exists). Say so in the handoff; T1's engine test proves the `whole` column.

### T3. A table-column alert rounds as the chart does (BI-57, D7, D8; #503 review R7)

- Anchors: `rust/crates/lakehouse-alerts/src/lib.rs:37-43` (`AGGS`, `aggregate_allowed`),
  `:1795-1825` (`mart_statement` and its doc), `:3602-3623` (the model test,
  `an_owned_rules_statement_reaches_the_gate_with_its_owner`), `:6569-6593` (`legacy_rule`,
  `engine_answering`); `rust/crates/lakehouse-bi/src/specs.rs:190-210` (`rounded`).
- Change: in `mart_statement`, the `else` branch becomes
  `lakehouse_bi::specs::Aggregate::from_str_lossy(agg).rounded(measure_ident.as_str())`, with
  the comment `// BI-57 (#503 review R7): the one rounding rule of charts (BI-55 D4); agg was
  checked against AGGS above, so the lossy parse never falls back.` The `count` branch and
  the validation stay. Add one sentence to the doc comment: the aggregate is rounded by
  `Aggregate::rounded`, as a chart's.
- Check (`lakehouse-alerts` tests, fail on today's code):
  - **`a_table_rule_rounds_an_average_a_minimum_and_a_maximum_to_six_decimals_as_a_chart_does`**:
    `mart_statement("mart_x", "amount", "avg")` is `Ok("SELECT round(avg(amount), 6) AS v FROM serving.mart_x")`;
    `max` → `round(max(amount), 6)`; `min` → `round(min(amount), 6)`; `sum` →
    `SELECT round(sum(amount)) AS v FROM serving.mart_x`; `count` →
    `SELECT count() AS v FROM serving.mart_x`; `"median"` is an `Err`.
  - **`an_average_rule_sends_the_six_decimal_statement_to_the_gate`**: as the model test at
    `:3602`, with `rule.agg = "avg".to_owned()`: the gate saw
    `SELECT round(avg(amount), 6) AS v FROM serving.mart_x`.
  - The model test at `:3621` (`round(sum(amount))`) passes unchanged in S1.

### T4. The documents say the rules (BI-57)

Every added line ends with *(proposed, not yet signed; `BI-57`)*.

- `docs/core/features/bins-groups-hierarchies.md`:
  - Decisions table, a new row after row 6 (`:42`):
    `| 7 | Over a column whose values are all whole numbers (a year), a bin of width 1 holds one value and the maximum has a bin of its own; such a bin reads as its value ("2020"). Automatic and counted bins of such a column are never narrower than 1, and are 2 wide when one bar per value would pass the count | Planner default (QA LH-BI-2-02) | *(proposed, not yet signed; BI-57)* |`
  - Limits, a new last line after `:48`: "A whole-number column over more values than the
    bin count (more than ten years under Automatic) is cut into ranges, which read with a
    thousands separator ("2,014 – 2,016"). A width of 1 typed over whole numbers, and a
    histogram ranged by 1, also give the maximum its own bin."
- `docs/core/features/sharing-and-delivery.md`, Limits, a new line after `:116` ("An alert
  looks at one number every 15 minutes…"): "An alert on the average, minimum or maximum of a
  table column compares the value with up to six decimals, as the chart shows it; before
  `BI-57` it compared the whole number, so an alert close to its threshold can fire or
  recover at a different check than before, once, without being edited. The alert's message
  still shows whole numbers."
- `CHANGELOG.md`, `## [Unreleased]` > `### Fixed`, a new bullet directly after the `BI-55`
  bullet (`:62`): "A year column in automatic bins reads one bar per year, "2020" to "2024",
  where it read "2,020 – 2,020.5" (backlog `BI-57`, QA `LH-BI-2-02`): over a column of whole
  numbers a bin of width 1 holds one value and the maximum has a bin of its own, and
  automatic and counted bins of such a column are never narrower than 1. A table-column
  alert on an average, a minimum or a maximum compares the value with up to six decimals, as
  the chart shows it, so an alert near its threshold may fire or recover at a different
  check than before. *(proposed, not yet signed)*"

### T5 (S2). A sum keeps six decimals; a sum of whole numbers is unchanged (BI-57, D9)

**Starts only after the planner says #504 and S1 are merged.** Branch
`fix/dashboards-p4-sums` from that `main`. Re-find every anchor by its text.

- Anchors: `rust/crates/lakehouse-bi/src/specs.rs:190-210` (`rounded` and its doc).
- Change:
  1. `Self::Sum => format!("round(sum({operand}), {})", Self::DECIMAL_SCALE)`; merge it into
     the `Avg | Max | Min` arm if that reads simpler. The doc: every aggregate but `count`
     keeps six decimals; `round(x, 6)` of an integer is the integer in `ClickHouse`, so a sum
     of whole numbers is unchanged; QA `LH-BI-6-04` (`BI-55`) and `BI-57`. Update the doc of
     `DECIMAL_SCALE` ("an average, a minimum and a maximum" → "every aggregate but count").
  2. Doc comments that quote the old text, changed to `round(sum(v), 6)`:
     `builder.rs:979`, `builder.rs:1146` (two occurrences), `builder/staged.rs:94`,
     `builder/ranked.rs:30`, and the comment at `lakehouse-api/src/routes/alerts.rs:2249`.
  3. The pinned assertions, **44 lines, each `round(sum(X))` → `round(sum(X), 6)` and
     nothing else on the line, because the rule changed (rule 2; the commit body says so)**.
     Line numbers at `65cf0358b`, with the test:
     - `lakehouse-bi/src/specs.rs:532` (`a_sum_is_a_whole_number_and_an_average_…`; also
       rename the test to `every_aggregate_but_a_count_keeps_six_decimals`);
     - `lakehouse-bi/src/bin.rs:755` (`a_grid_statement_groups_by_cell_and_keeps_the_cap`);
     - `lakehouse-bi/src/builder.rs` (24): `:3372` `kpi_kind_routes_to_kpi_builder`; `:3574`
       `a_kpi_over_a_sql_source_is_capped_too`; `:3675`
       `a_point_map_selects_coordinates_label_and_value_largest_first`; `:3705`
       `a_stored_point_map_is_rebuilt_with_the_year_filter_and_its_coordinates`; `:4168`
       `a_filter_on_an_aliased_measure_is_applied_inside_the_relation`; `:4189`
       `a_filter_on_another_column_keeps_the_plain_where`; `:4377`
       `a_grained_chart_buckets_the_dimension_in_an_inner_relation_and_keeps_the_latest`;
       `:4429` `a_dashboard_filter_on_the_grained_column_reads_the_raw_column`; `:4461`
       `a_breakdown_beside_a_time_grain_keeps_every_bucket_and_the_top_breakdown_values`;
       `:4662` `a_heatmap_a_sankey_and_a_sunburst_keep_their_statements_…`; `:4712`
       `a_value_order_is_the_editors_choice_and_keeps_the_top_buckets_not_the_latest`;
       `:5139`, `:5156` `charts_saved_before_bi_6_build_the_statement_they_always_did`;
       `:5192` `two_aggregations_of_one_column_are_two_result_columns`; `:5209`
       `a_measure_beside_another_aggregation_of_its_column_is_renamed_outside_the_aggregate`
       (also its comment at `:5199`); `:5230`
       `a_chart_filter_and_a_dashboard_filter_on_one_column_both_apply`; `:5342`
       `a_grained_chart_sorted_by_its_dimension_descending_keeps_the_latest_buckets_newest_first`;
       `:5358` `a_breakdown_sorted_by_a_measure_keeps_and_shows_the_dimensions_in_that_order`;
       `:5380` `other_is_aggregated_over_the_rows_outside_the_kept_groups_and_flagged`;
       `:5434`, `:5448` `a_table_grouped_by_several_columns_lists_them_in_order_with_a_grain_on_a_date`;
       `:5464`, `:5475` `steps_are_nested_statements_and_the_sort_and_limit_belong_to_the_last_one`;
       `:5496` `a_binned_dimension_reads_the_range_and_the_filters_under_the_bin`;
     - `lakehouse-bi/src/fields.rs` (3): `:1315`, `:1344`, `:1766`;
     - `lakehouse-bi/src/metrics.rs:1223`;
     - `lakehouse-bi/src/tables.rs:1294`;
     - `lakehouse-bi/src/types_b.rs` (6 lines, 7 occurrences): `:933`, `:948`, `:949`,
       `:975`, `:1011`, `:1023` (two on the line);
     - `lakehouse-bi/src/store.rs` (5): `:7296`, `:7906`, `:7973`, `:8027`, `:9064`; and the
       test #504 adds (`a_radar_with_two_measures_is_saved_with_both_columns`);
     - `lakehouse-api/src/routes/ai/tools/dashboards.rs:1737`
       (`the_request_for_a_progress_bar_against_a_goal_of_a_million_creates_one`);
     - `lakehouse-alerts/src/lib.rs:3621` (T3's model test) and T3's new `sum` line.
  4. **Not changed** (stored rows or hand-written statements, not built by
     `Aggregate::rounded`): `store.rs:6110`, `store.rs:6382`,
     `lakehouse-api/src/routes/dashboard.rs:6801`, `routes/layers_rewrite.rs:323`, `:334`,
     `lakehouse-api/tests/dashboard_tenant_scope.rs:214`, `specs/builtin-default.json` (five
     lines), `src/lib/dashboard-specs.ts` (five lines).
     Afterwards `grep -rn "round(sum(" rust/crates src ops` shows only the six-decimal form and
     the list just above. If one of the 44 still passes unchanged after step 1, or a test not
     listed fails, stop and report: the classification is wrong, not the test.
  5. Documents: `docs/core/features/chart-builder-questions.md:57`, replace "A sum is a whole
     number. An average, a minimum and a maximum keep up to six decimals;" with "A sum, an
     average, a minimum and a maximum keep up to six decimals, and a sum of whole numbers is a
     whole number;", and replace "with an average, a minimum or a maximum for the whole chart"
     with "with a sum, an average, a minimum or a maximum for the whole chart"; the line keeps
     its mark and gains `BI-57`. `CHANGELOG.md`, `### Fixed`, a bullet after T4's: "A sum of
     a decimal column keeps up to six decimals; a sum of whole numbers is unchanged (backlog
     `BI-57`). A chart saved before keeps a whole sum until it is saved again, unless a
     dashboard filter applies. *(proposed, not yet signed)*"
- Check (`rust/crates/lakehouse-api/tests/chart_numbers.rs`; fails on S1's code):
  seed `default.bi57_sums (kab String, n UInt32, f Float64, d Decimal(18, 2))` with
  `('a', 1, 1.25, 1.25), ('a', 2, 2.5, 2.50), ('b', 7, 0.1, 0.10), ('b', 0, 0.2, 0.20)`.
  **`a_sum_of_decimals_keeps_them_and_a_sum_of_whole_numbers_is_unchanged_on_the_engine`**:
  build through `spec_from_inline_sql` as T1 of part 1 does, source
  `SELECT kab, n, f, d FROM default.bi57_sums`, columns `kab String`, `n UInt32`,
  `f Float64`, `d Decimal(18, 2)`, input
  `{ "title": "BI-57 sums", "kind": "bar", "dimension": "kab", "measures": ["n", "f", "d"] }`.
  The saved statement contains `round(sum(n), 6)`, `round(sum(f), 6)`, `round(sum(d), 6)`.
  Run it: row `a`: `n` 3.0, `f` 3.75, `d` 3.75; row `b`: `n` 7.0, `f` 0.3, `d` 0.3. Part 1's
  test in the same file (its sum of `v` is 5.0) passes unchanged.

## 3. Verification

Per commit, only for what the commit touched (AGENTS.md, "Keep build time down"); section 3a
replaces the common brief's machine-specific commands. A handoff quotes each command with
its counts and writes *not verified* with the reason for anything not run (rule 7).

| Task | Rust unit (`--lib`) | Integration (`--test`) | Console (`bun test`) |
| --- | --- | --- | --- |
| T1 | `lakehouse-bi`: the whole `--lib` once (four pinned statements moved) | `chart_numbers` | none |
| T2 | `lakehouse-api --lib bin_rewrite` (must pass unchanged) | `chart_numbers`, `sec11_guard`, `route_auth` | `src/lib/chart-bins.test.ts`, `src/lib/chart-click.test.ts` |
| T3 | `lakehouse-alerts`: the whole `--lib` | none | none |
| T4 | none | none | none |
| T5 | `lakehouse-bi`, `lakehouse-alerts`: the whole `--lib`; `lakehouse-api --lib ai::tools::dashboards` | `chart_numbers`, `sec11_guard`, `route_auth` | none |

Always, per commit:

```bash
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status    # nothing generated staged
```

Nothing here is proven in a browser. The owner runs `LH-BI-2-02` on staging after the merge
(a year column under "Automatic": five bars 2020 to 2024; a click on 2023 lists 2023 only and
filters `2023 ≤ tahun < 2024`; a click on 2024 lists 2024).

## 3a. Change of machine

- The developer works on the ARM64 build machine, in the checkout
  `/home/rismana/rantai/lakehouse-agents-4`, and only under `/home/rismana/rantai/`.
- Before the first cargo command of a shell:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-4 CARGO_BUILD_JOBS=6`.
  Plain `cargo`.
- Per Rust commit:

  ```bash
  cd rust && cargo fmt --check
  cargo clippy -p <crate> --all-targets -- -D warnings      # each crate the commit touched
  cargo test -p <crate> --lib                                # the touched crate's unit tests (a name filter while iterating)
  cargo test -p lakehouse-api --test <file>                  # each touched integration test file (Docker runs there)
  cargo test -p lakehouse-api --test sec11_guard             # ALWAYS when lakehouse-api/src is touched (T2, T5)
  cargo test -p lakehouse-api --test route_auth              # ALWAYS when lakehouse-api/src is touched (T2, T5)
  ```

  T1 touches `lakehouse-bi` and a test file of `lakehouse-api`: clippy both crates. T3
  touches `lakehouse-alerts` only. `lakehouse-api` unit tests run only as
  `cargo test -p lakehouse-api --lib <filter>`.
- Never `cargo test --workspace`, never `cargo clean`.
- `chart_numbers` does not include `tests/common/mod.rs`, so it reports exactly its own
  tests (part 1's handoff measured this).
- TypeScript: `bun install --frozen-lockfile` before the first check. Per console commit
  `bun run typecheck && bun run lint && bun test <the test files of the table>`.
  `bun run test` (the whole console suite) once, before the handoff.
- Start no container you do not remove; stop no process you did not start (the test
  containers of `chart_numbers` are the harness's own).
- Before the handoff of S1, once, in the foreground, on the final commit:

  ```bash
  cd rust && cargo fmt --check
  cargo clippy -p lakehouse-bi --all-targets -- -D warnings
  cargo clippy -p lakehouse-api --all-targets -- -D warnings
  cargo clippy -p lakehouse-alerts --all-targets -- -D warnings
  cargo test -p lakehouse-bi --lib
  cargo test -p lakehouse-alerts --lib
  cargo test -p lakehouse-api --lib bin_rewrite
  cargo test -p lakehouse-api --test chart_numbers
  cargo test -p lakehouse-api --test sec11_guard
  cargo test -p lakehouse-api --test route_auth
  cd .. && bun run typecheck && bun run lint && bun run test
  python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
  git status
  ```

  S2's block is the same without `bin_rewrite` and the console lines, plus
  `cargo test -p lakehouse-api --lib ai::tools::dashboards`.
- The repository's lints run on every commit (above). CI on the pull request (x86_64) is the
  final word.

## 4. Out of scope

- **A rule from a column's name** (a column called `tahun` starting with "No bins"), and
  changing the builder's default bin (`chart-builder.tsx` is not edited).
- **Labels of wider bins of whole numbers**: "2,014 – 2,016" keeps its separator and reads as
  a range with its upper edge, not as "2014–2015" (D5).
- **A floor for a typed width and for a histogram's own width rules**: a typed 0.5 over years
  is honoured; a histogram of a narrow whole-number range can still be cut below 1 (D4).
- **The alert's message** shows whole numbers (`fmt_id_id`); a message can read "= 1.466 >
  1.466" for an average that fired at 1466.18 (D8).
- **Charts saved before** keep their stored statement when no dashboard filter applies (D9;
  part 1's open question 4).
- **Integer vs decimal column types in the builder** (D9).

Follow-up lines for the planner to add to `docs/core/BACKLOG.md` (the developer does not edit
it):

- `BI-<next>` Task: an alert's message shows the value with the decimals the rule compared
  (today `fmt_id_id` rounds to a whole number). Under `BI-19`.
- `BI-<next>` Task: a chart saved before `BI-55`/`BI-57` shows the same decimals with and
  without a dashboard filter. Under `BI-6`. (Part 1 proposed it; it is not in the backlog at
  `65cf0358b`.)
- If S2 is held (open question 5): `BI-<next>` Task: a sum of a decimal column keeps its
  decimals, from T5 of this plan.

## Open questions for the owner

None blocks S1; the plan builds the answer named first. None of the wording or the decisions
here has been seen by the owner.

1. **The maximum gets a bin of its own over whole numbers in bins of 1** (D3). It is an
   exception to decision 2 of the bins page ("the last bin includes the maximum", confirmed
   at QA-1, 2026-11-02): the last bin still includes the maximum, but holds nothing else.
   Recommendation: accept; without it a year chart shows a bar "2023" that holds two years.
   The other choice: keep "No bins" as the answer for years and close `LH-BI-2-02` as a label
   finding only (labels stay ranges).
2. **Eleven or more values under "Automatic" become bins of 2** (D4), not eleven bars.
   Recommendation: as planned ("about ten" and "at most 10/50/100" stay true). The other
   choice: allow one bar more than the count for whole numbers.
3. **A bin of one value reads as the value with no separator** ("2020", and "1000" for a
   column of counts in bins of 1), as an unbinned category does (D5). Recommendation: as
   planned.
4. **Table-column alerts on an average, a minimum or a maximum compare six decimals** (D8);
   such an alert may fire or recover once at a different check after the deploy, and its
   message still shows a whole number. Recommendation: accept, with the follow-up for the
   message.
5. **S2: a sum keeps six decimals** (D9), after #504 merges; a chart saved before keeps a
   whole sum until saved again unless a filter applies. Recommendation: build S2; integer
   sums do not change, and a revenue column in `Decimal` stops losing its cents. The other
   choice: hold S2 until the "charts saved before" follow-up, so no chart switches between
   the two while filtered.

## 5. Handoff

## 6. Review
