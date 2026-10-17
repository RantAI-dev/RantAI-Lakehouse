# BI-16 part B: progress, histogram, bullet, image and web cards (with AI-5)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-3` (worktree `/home/hv/lakehouse-phase3-build`), after
BI-17 part A (`docs/superpowers/plans/2026-10-15-bi-17a-chart-formatting.md`)
is committed: the new kinds use its value formats and its settings page.
Feature page: `docs/core/features/progress-histogram-bullet-cards.md`
(decisions 1 to 10, planner defaults the owner confirms at the phase 3 QA).
Spec: `docs/core/specs/bi-16.md` part B; assistant `docs/core/specs/ai-5.md`.

Anchors were read on 2026-10-15 at `82171ae7` (working tree, other edits in
progress). Find each by the symbol named.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus:

- **D1 Five kinds.** `ChartKind` (`specs.rs`) gains `progress`, `histogram`,
  `bullet`, `image`, `web`. `image` and `web` are static
  (`ChartKind::is_static`, TS `isStaticKind` in `src/lib/dashboard-tabs.ts`):
  no statement, no filter, like `text` and `heading`. The other three are
  data charts that take the dashboard's filters and BI-17 A's value formats.
- **D2 Fields** live in a new flattened struct on `ChartInput`,
  `TypeFields` in a new module `lakehouse-bi/src/types_b.rs` (name free,
  keep it one module), refused on other kinds the way `tables.rs` `only_for`
  refuses:
  - `goal: Option<Goal>` where `Goal = Fixed(f64) | Value(String)` (a
    second measure alias of the same source): progress and bullet.
  - `bands: Vec<f64>` (1 to 3 ascending limits): bullet.
  - `histogram: Option<HistogramBins { width: Option<f64>, count: Option<u16> }>`
    (at most one of the two; count 1..=100; both absent = automatic).
  - `url: Option<String>`, `alt: Option<String>` (≤ 200): image; `url`: web.
- **D3 Histogram SQL reuses BI-10.** `lakehouse-bi/src/bin.rs` `BinSpec`
  (`auto|count|width`), `Range::stats_sql` (aims at about 10 nice bins
  today), `bin_expr`, the `builder/binned.rs` path. Add a target-count
  parameter to the automatic width so a histogram aims at about 20 (BI-10's
  own default of about 10 does not change). The query counts rows per bin
  (`count()`); the console fills empty bins between the first and the last
  with zero (feature decision 2) from the bin width, and labels them with
  the BI-10 labels (`routes/support.rs` `annotate_bins`, TS
  `src/lib/chart-bins.ts`).
- **D4 Content sites.** Admin > Settings gains `contentSites` (0..=50,
  `https://host` or `https://host:port`, no wildcards). Reuse
  `embed_access::normalize_origin` / the shape of `validate_origins`
  (`lakehouse-bi/src/embed_access.rs`) with its own max of 50, not a copy
  (rule 4). Migration `rust/migrations/0073_reporting_content_sites.sql`
  (or the next free number; why-header). The list reaches every view
  through the dashboard payload's `reporting` object as in BI-17 A.
  - At save (console and assistant): an `image`/`web` url must be `https://`,
    parse as a URL, and its origin must be in the list, else 400 "Images
    and web pages may only come from the sites an administrator allows
    (Admin > Settings)."
  - At render: the console checks the origin against the current list from
    the payload; a site removed since shows "This site is no longer allowed"
    and loads nothing.
  - There is no Content-Security-Policy in the app today (no `headers()` in
    `next.config.ts`; `src/proxy.ts` sets only `frame-ancestors` for
    `/embed`). This part does not add one; the list is enforced at save and
    render. The feature page's limits say so; a CSP is a separate backlog
    item the planner files.
- **D5 Rendering.**
  - progress: a horizontal bar (plain HTML/CSS, not ECharts) with the value,
    the goal and the percentage in the value's format; above 100 % the bar
    is full and the label shows the real percentage.
  - histogram: ECharts bars with no gap (`barCategoryGap: 0`), x labels
    from the bin labels.
  - bullet: ECharts horizontal bar for the value, `markLine` for the
    target, `markArea` bands in three greys (light to dark), one row per
    category, at most 20 and "N more not shown".
  - image: `<img src alt referrerPolicy="no-referrer" loading="lazy">`,
    `object-fit: contain`; an image that fails to load shows "The image
    could not be loaded" (no URL echoed in the message).
  - web: `<iframe sandbox="allow-scripts allow-popups" referrerPolicy="no-referrer" loading="lazy" title={title}>`
    and an "Open in a new tab" link (`target=_blank rel="noopener noreferrer"`).
    No `allow-same-origin`, no `allow-top-navigation`.
- **D6 Assistant (AI-5).** `chart_kind_enum` (`registry.rs`) gains the five
  kinds; the create/update schemas gain `goal`, `bands`, `histogram`, `url`,
  `alt`; the same validation and sentences as the console; prompt line
  "progress and bullet take goal; histogram takes one number column; image
  and web need an address on an allowed site". Fixture regenerated with the
  ignored writer test. `capabilities.ts` unchanged unless a new tool is added
  (none is). The two requests of the feature page join the standard request
  set.

## 2. Anchors (2026-10-15, find by symbol)

- Kinds, Rust: `specs.rs` `ChartKind`, `is_static`; `store.rs` `KINDS`,
  `derive_common_fields`, `breakdown_allowed`, `breakdown_required`,
  `spec_from_resolved` (KPI/gauge branch); kind matches in `fields.rs`
  `takes_aggregate_measure`, `shape.rs` `is_grouped_kind` and `takes_bin`,
  `params.rs` `takes_reference_lines` and the next match, `grain.rs`
  `kind_takes`, `builder.rs` `result_aliases` and the KPI SQL path,
  `routes/dashboard.rs` `chart_def_fields` (YAML export),
  `lakehouse-alerts/src/lib.rs` (kind match), `tables.rs` `only_for`.
  Tests listing kinds: `store.rs` grain-refusal list, `builder.rs` kind
  list.
- Kinds, TS: `src/lib/dashboard-specs.ts` `ChartKind`; `dashboard-tabs.ts`
  `STATIC_KINDS`; `chart-builder.tsx` `KIND_GROUPS`, `KIND_DESCRIPTIONS`
  (a `Record<ChartKind, …>`, the compiler forces entries), `KIND_KEYWORDS`,
  `kindIcon`; `builder-rules.ts` `LINE_KINDS`, `SINGLE_KINDS`;
  `chart-click.ts`, `chart-shape.ts`, `chart-title.ts`, `section-summary.ts`,
  `chart-transforms.ts`, `tile-body.tsx` (switch), `tile-card.tsx`; tests
  `builder-rules.test.ts`, `chart-click.test.ts`, `chart-title.test.ts`,
  `dashboard-tabs.test.ts`.
- Gauge target and KPI goal to mirror: `store.rs` `ChartSpec.target` (kept
  only for gauge), `chart-option.ts` gauge branch, `tables.rs` `Compare`
  goal, `src/lib/kpi-compare.ts`.
- Bins: `bin.rs` `BinSpec`, `mode`, `Range::stats_sql`, `bin_expr`;
  `builder/binned.rs`; `shape.rs` `ShapeFields.bin`; `routes/support.rs`
  `annotate_bins`; `src/lib/chart-bins.ts`, `bin-picker.tsx`.
- URL helpers: `embed_access.rs` `validate_origins`, `normalize_origin`;
  `click.rs` `validate_click_url`; TS `safeHttpUrl`, `safeImageUrl`
  (`src/lib/cell-format.ts`), `table-cell.tsx` image cell.
- Settings: as in the BI-17 A plan (`routes/settings.rs`,
  `reporting_settings.rs`, `reporting-settings-page.tsx`,
  `reporting-context.tsx`).
- Assistant: `registry.rs` `chart_kind_enum`, schemas, count test,
  `write_tool_schema_fixture`; `routes/ai/tools/dashboards.rs`;
  `routes/ai/prompt.rs`.

## 3. Tasks (one commit each, in this order)

**T1 Content sites setting (D4).** Migration, store, validation (reusing
the origin helper), payload, settings page section "Allowed content
sites". *Accept:* store round-trip; route refusals (http, wildcard, path,
51 sites); `settings:write` needed; page test.

**T2 Kinds and fields, Rust (D1, D2, D3).** Every match site in section 2;
validation and sentences; histogram SQL (count per bin, target ~20);
progress/bullet SQL (value and optional goal measure through the KPI path
or the grouped path for bullet with a category); image/web validation
against the settings list (the store path gets the list from the route).
*Accept:* SQL snapshot tests for histogram (auto, width, count) and bullet
(with and without category, goal as a value); refusal tests (goal on a bar
chart, 4 bands, bands not ascending, count 101, width and count together,
http image, image on a site not listed); `sql_rewrite::enforce` applies to
the new statement shapes (test as BI-10 did); a static `image` tile takes
no filter (test like the text tile's); the YAML export includes the new
fields.

**T3 Renderer and builder, TS (D5).** Kind registration everywhere in
section 2; the five tiles; builder fields (goal fixed or value, bands,
histogram width/count, url + alt with the allowed sites shown under the
input); builder rules (histogram: one number column, no breakdown).
*Accept:* tile tests (progress over 100 %, goal zero message; histogram
zero-filled gap; bullet "N more not shown"; image failure message; web
iframe has the sandbox and no `allow-same-origin`; a site removed from
the list renders the message and no `img`/`iframe`); builder round-trip
per kind.

**T4 Coverage check.** The feature page's Metabase table as a TS test
(`src/lib/chart-coverage.test.ts`): each Metabase type maps to a `ChartKind`
that exists or to a named owner decision; correct the feature page's table
if the code says otherwise and say so in the handoff.

**T5 Assistant (D6).** *Accept:* tool tests: create a progress bar with a
fixed goal; a histogram with count 10; an image on a non-allowed site is
refused with the console's sentence; the tool-count pin is unchanged (no
new tool); fixture regenerated; snapshot test green.

**T6 Docs.** CHANGELOG entry.

## 4. Out of scope

Part C (Gantt, world maps, uploads, spatial, plugins); uploading image
files; a Content-Security-Policy; detecting whether a site refuses framing.

## 5. Must verify, not assume

- Every `match` on `ChartKind` compiles with the new variants (no `_ =>`
  that silently treats a new kind as another; say where one exists).
- The alerts crate's kind match: whether an alert on a progress or bullet
  chart makes sense; if not, refuse it with a sentence and say so.
- Embed and public views render the five kinds (they share `tile-body.tsx`;
  confirm).

## 6. Checks

As in the BI-17 A plan, section 6 (same cargo environment and rules), plus
`cargo test -p lakehouse-alerts` if its match changes.

## 7. Handoff (developer appends)

### BI-16 part B and AI-5, slice 1 (T1 to T6)

Branch `feat/bi-phase-3`, from `27c51dfe`. Developer: Claude Sonnet. Nothing pushed, no PR.

**Commits** (one per task, each body cites its task):

| Task | Commit | What it became |
| --- | --- | --- |
| T1 | `48485be1` | Migration `0073_reporting_content_sites.sql` (`content_sites TEXT[]`, empty by default, CHECK for shape and the 50 limit), store field, `embed_access::validate_content_sites` (calls the same `normalize_origin` as `validate_origins`, https only, max 50) and `https_origin_of`, the settings route's optional `contentSites` (kept when not sent), the `reporting` object of the dashboard, embed and public payloads, Admin > Settings "Allowed content sites". |
| T2 | `16619d2c` | Five `ChartKind`s (`image` and `web` static); `lakehouse_bi::types_b` (`TypeFields`, `Goal`, `HistogramBins`, `check_fields`, `check_content_url`, `CONTENT_SITE_REFUSED`); the content-site check in `spec_before_relation`; progress and single-bullet statement (`build_progress_sql`), histogram statement (`histogram_sql` over the BI-10 binned relation), per-category bullet through the grouped statement; every kind match site of section 2; YAML export; `routes/type_b_rewrite.rs` (real `enforce`). |
| T3 | `29469e8d` | `lib/type-b.ts`, `progress-card.tsx`, `content-cards.tsx` (image, web), `type-b-option.ts` (histogram, bullet), `type-b-fields.tsx`, `TileBody` branches, builder wiring, kind registration in every TS rule. |
| T4 | `8215c335` | `src/lib/chart-coverage.test.ts` reading the feature page's table; four rows of the page corrected (below). |
| T5 | `6626e70d` | `chart_kind_enum` + `with_type_properties` on `create_chart` / `update_chart`, prompt sentence, fixture regenerated (ignored writer test), tool-argument tests for the two standard requests and the refusals. |
| T6 | `f01f0741` | CHANGELOG entry. |
| fix | `22968c9a` | A histogram result as long as the server's cap (100 ranges) says it may have been cut (found while writing the handoff: see item 5 below). |

**Checks run** (foreground, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`, on the final Rust code; the last commit touches TS only):

- `cd rust && cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-bi -p lakehouse-store -p lakehouse-api -p lakehouse-alerts --all-targets -- -D warnings`: no warnings.
- `cargo test -p lakehouse-bi -- --test-threads=4`: 511 passed, 3 ignored, 0 failed (lib), `specs_match_typescript` 1 passed.
- `cargo test -p lakehouse-alerts -- --test-threads=4`: 90 passed, 1 ignored, 0 failed.
- `cargo test -p lakehouse-store -- --test-threads=4`: 199 lib tests and every integration binary ok, 0 failed (`reporting_settings` 6 passed, among them the new CHECK test; own Postgres through testcontainers).
- `cargo test -p lakehouse-api --lib -- --test-threads=4 settings dashboard ai:: embed organise rewrite`: 615 passed, 17 ignored, 0 failed (includes `type_b_rewrite` 4, the new tool tests, the tool schema snapshot).
- `cargo test -p lakehouse-api --test route_auth -- --test-threads=2`: 38 passed. `--test sec11_guard`: 4 passed. `--test reporting_settings_route`: 10 passed. `--test ai_tool`: 9 passed. `--test dashboard_export`: 6 passed. (Each file on its own.)
- `write_tool_schema_fixture` (ignored writer test) run once; `tests/fixtures/tool_schemas.json` +120 lines (the five kinds, `goal`, `bands`, `histogram`, `url`, `alt` on the two tools). `tool_schemas().len()` is still 93.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, the same 6 warnings as before. `bun run test`: 1961 pass, 1 skip, 0 fail (202 files) before the last commit; 1962 pass after it (the new test).
- `bun --bun next build --webpack`: compiled successfully (`bun run build` / Turbopack fails in this worktree because `node_modules` is a symlink; the webpack build is the check here). It ran before the last, small commit.

**Not verified**

- Nothing was rendered in a browser: the ECharts options and the tiles are asserted as JSON and under happy-dom. Not seen with eyes: the bullet bands and target marks, the histogram's touching bars, the progress bar, an image or a frame actually loading, the builder layout of the new controls, "Open in a new tab".
- The new statements were never run on a real ClickHouse. They are asserted as text and through the real `sql_rewrite::enforce` (row filter and mask: every read of the table becomes its governed projection), not executed. The engine behaviour of the nested `__m0`/`__m1` select and of `count()` under the bins is the same shape as BI-10's and the KPI's, but unmeasured here. Set `BIN_STATEMENTS_OUT`/`BIN_REWRITE_OUT` to dump `type_b_rewrite`'s statements, as for BI-10, to run them by hand.
- The assistant through a model: `AI-5` is pinned as tool-argument tests, not as a conversation.
- Migration `0073` against the dev database: only through the `sqlx::test` databases.
- `bun run build` under Turbopack (see above), the full workspace run and the other integration test files of `lakehouse-api`: CI.
- Python, compose and Dagster: not touched, not run.
- The copilot's chart draft card (`features/copilot/chart-draft-card.tsx`) renders `TileBody` outside a `ReportingProvider`, so an image or web draft would show "This site is no longer allowed" in the card (default list empty); saving it goes through the server's check. Not changed, not tested.

**Where the plan was wrong against the code, or I deviated**

1. **D4 "the store path gets the list from the route".** `spec_from_input` is called from nine places (console create and update, preview, version revert, the assistant's `create_chart` / `update_chart` / `format_chart`, inline SQL). Adding an argument to each would leave any one that was missed saving a card around the list. I put the list in `TimeContext` (`with_content_sites` / `content_sites()`, loaded by `settings::time_context`), which every one of them already receives, and check in `spec_before_relation`. `TimeContext::default()` carries no site, so a context without a list refuses every card (fail closed). It is documented on the struct, next to BI-11's parameter values that ride there for the same reason. The check function is `lakehouse_bi::types_b::check_content_url`, not in the API crate; the console route, the assistant tools and a version revert reach it through `spec_from_input`, and both are tested (route test, tool tests). A consequence to decide on: reverting a dashboard version that holds an image card whose site has since left the list is refused with the sentence ("no longer works").
2. **D2 `Goal::Value(String)` "a second measure alias".** It is the second measure's **column name**, which must be the second entry of `measures` (`goal: {"measure": "<column>"}`, `measures: [value, column]`); the result alias is read from `y` (`["v", "g"]` on a single row, the measure names on a grouped bullet). A fixed goal needs exactly one measure; a progress bar needs a goal, a bullet does not.
3. **D3 "add a target-count parameter to the automatic width".** Not needed: `BinMode::Count(n)` already takes any `n`; the 10/50/100 list lives in `BinSpec::mode()`, so `bin.rs` is untouched. A histogram uses `BinMode::Count(20)` (`AUTO_HISTOGRAM_BINS`) when no width or count is given, `Count(n)` for 1 to 100, `Width(w)` otherwise. It does not go through `shape.bin`; `binned::chart_layers` returns the layer for kind `histogram` from `types.histogram`, so `FilteredSql.bins` and `annotate_bins` work unchanged.
4. **Histogram shape.** The definition has the number column as `dimension` and no measures; the statement is `histogram_sql` (the BI-10 binned relation, `count() AS __count`, ordered by range, `LIMIT 100`); `limit`, `order`, `aggregate` other than `count`, a breakdown and a grain are refused with a sentence (beyond the plan, because they would be stored and ignored). A SQL source takes a width only (as for BI-10 bins).
5. **At most 100 ranges are returned.** A width that gives more than 100 non-empty ranges is cut to the first 100 (in range order) by the statement's limit. The tile says so when the result is exactly the cap (`22968c9a`); the plan did not mention it. A width that would need more than 500 bars between the first and last range is refused by the tile with a sentence instead of filling zeros.
6. **Bullet.** With no category it is one row (the progress statement); with one it is the grouped statement with the goal as its second measure, `order` desc and a `limit` of 1 to 100 (default 20). The tile draws the first 20 and says "N more not shown" where N counts the rows the limit returned. A fixed target is a `markLine`; a target that is a second value is one scatter tick per row (a mark line cannot differ per row). No sort control, no click, no "Other", no steps (`is_grouped_kind` is false for the three data kinds; the server refuses `sort`, own aggregates and so on with the existing sentences).
7. **Progress / histogram / bullet / image / web are in `NO_VALUE_KINDS`** (no click setting, no drill menu). A click on a histogram bar to filter by its range is not built; the plan did not ask for it.
8. **`ChartInput.types` is a `Box<TypeFields>`.** A bare copy pushed the assistant loop's future past clippy's `large_futures` limit (16 384 bytes; `agents.rs` `run_headless_loop` came out at 16 416). Serde, `Default` and `Clone` are unaffected.
9. **Alerts.** The only kind match is the board digest, which lists KPI and gauge tiles. I left progress and bullet out on purpose (a digest line is one number; alerts are rules on metrics, not tiles) and said so in a comment at the match; no sentence is needed because no alert can be put on a tile.
10. **sec11_guard.** `format!("contentSites: {why}")` was flagged, correctly; the route answers a fixed sentence instead of the validator's text (which names the first bad entry).
11. **Feature page, section "Coverage against Metabase", four rows corrected** after checking the code: Detail is not a tile kind (it is the one-record view with Previous / Next in the record list, `drill.tsx`), Metabase's row chart is our `hbar`, pie and donut are one kind (`pie`, drawn as a donut), Maps names the three kinds. The page now says the TS test reads the table.
12. **`AI-15`** still has no request-set file; the two requests are tool-argument tests next to the tool tests (`the_request_for_a_progress_bar_against_a_goal_of_a_million_creates_one`, `the_request_to_show_how_prices_are_spread_creates_a_histogram`). The planner decides where the standard set lives.
13. **The builder** previews an image or a web card locally (nothing to ask the server), builds the goal's second measure into `measures` at save and reads it back out of the measure rows (`measureRowsOf`), and labels the missing fields "Goal number", "Goal column", "Target number", "Band limits, rising", "Number of ranges", "Range width", "Address", "Description" (not "Goal": `missingSection("Goal")` is the KPI comparison's, in More options).
14. **Tests that log.** The web-card tests switch happy-dom's iframe page loading off so no test touches the network; happy-dom logs "Iframe page loading is disabled" for each frame. The tests pass.

**Smaller things worth a look in review**

- `https_origin_of` (Rust) and `httpsOrigin` (TS) are two implementations of one rule; the TS test uses the Rust test's cases (and more). Whether to add a shared corpus is the planner's call.
- `typePayload` sends `image` / `alt` trimmed; the server stores them trimmed (`stored_card_fields`).
- The Rust bullet check allows a `goal` of any finite number, zero and negative included (the tile says "No goal to compare with" for a progress bar; a bullet just draws no target at or below zero's edge).
- `HistogramBins` accepts a number or a numeric string for `width` and `count` (the assistant's model sometimes sends text); a fraction or a negative count is kept out of range so the sentence "a histogram has 1 to 100 ranges." answers it.

## 8. Review (planner appends)

### Planner's review (T1 to T6), 2026-10-16

Accepted deviations: the site list rides in `TimeContext`, filled by the one
`routes::settings::time_context` every route already calls, so no save path
can skip it, and `TimeContext::default()` carries no site (fail closed);
`Goal::Value` names the second measure's column; no change to `bin.rs`
(`BinMode::Count(20)`); the histogram's refusals and the 100-range cut-off
note; `ChartInput.types` boxed for clippy's `large_futures`; progress and
bullet left out of the board digest on purpose; four rows of the feature
page's coverage table corrected from the code.

Consequence to tell the owner at QA: going back to a version that holds an
image or web card whose site has since been removed from the allowed list
is refused (the card would not show anyway).

- **R1 SHOULD-FIX (open)** — the assistant's draft card
  (`chart-draft-card.tsx`) renders `TileBody` outside a `ReportingProvider`,
  so an image or web draft would read "This site is no longer allowed" in
  the chat. Fixed after the owner's QA together with whatever the QA finds.

Not verified by anyone yet: the new statements on a real ClickHouse (the
dev API rebuild and the owner's QA run them), the browser, the assistant
through a model. The reviewer's own re-run was skipped at the owner's
request.
