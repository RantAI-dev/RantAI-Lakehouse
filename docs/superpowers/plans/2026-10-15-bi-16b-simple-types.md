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

## 8. Review (planner appends)
