# BI-16 part C: Gantt, world and country maps, uploaded boundaries, spatial analysis, custom charts (with AI-5)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-6` (to be cut; one branch and one draft PR for the
phase), base `feat/bi-phase-4` until phase 4 merges, then `main`. Feature
page: `docs/core/features/gantt-world-maps-spatial.md` (decisions 9 and 10
wait for the owner, Q3 and Q4; the rest planner defaults confirmed at the
phase 6 QA). Spec: `docs/core/specs/bi-16.md` part C; assistant
`docs/core/specs/ai-5.md`. Not blocked: `BI-10` (the spec's blocker, for
histogram bins in part B) is merged (#160, In Acceptance).

Anchors were read on 2026-10-23 on `feat/bi-phase-4` at `150a081a`. Find
each by the symbol named.

## 0. Parts, order and what runs in parallel

The spec's part C is size L. It is cut so the most used and least risky
part ships first, and each part can be reviewed and QA'd alone (one
commit series per part on the phase branch; the handoff and review below
are per part).

| Part | What | Size | Mostly | Risk | Depends on |
| --- | --- | --- | --- | --- | --- |
| C1 | Gantt chart | S | Rust kind + builder SQL, then TS rendering | Low: one new kind on known rails (the part B kinds) | — |
| C2a | World and country maps (bundled) | S | TS + a one-off build script + files | Low: the map catalogue is console-owned, the API stores only an id | — |
| C2b | Uploaded GeoJSON boundaries | M | Rust (route, store, validation, simplification) + TS admin page | Medium: a 50 MB upload, a new dependency, a migration | C2a (the catalogue gains a second source) |
| C3 | Spatial functions: `Distance`, `RegionOf` (spatial join) | M | Rust (formula catalog, compiler, a ClickHouse polygon dictionary per map) | Highest: engine dictionaries, the policy rewrite meeting `dictGet` | C2b (the boundaries on the engine) |
| C4 | Custom charts from a declarative template | S–M | TS + a small Rust validator | Security if done wrong | **Owner decides whether at all** (Q3) |

Parallel: C2a is TypeScript and data files; it runs beside C1's Rust
build. C1's TS (rendering) runs beside C2b's Rust build. C3 waits for
C2b. C4 waits for the owner and touches neither maps nor Gantt.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions.

### C1 Gantt

- **D1 Kind.** `ChartKind::Gantt` (`lakehouse-bi/src/specs.rs`, serde
  `"gantt"`), not static. Fields: `x` = task column (the dimension),
  new optional `start`, `end`, `progress` columns on the chart shape
  (beside `lat`/`lon`, rendered by `render_stored_spec` like them), and
  `series` = group. `is_grouped_kind` excludes it (its own statement, like
  `Histogram`), `kind_check` requires `start` and `end` of a date or
  date-time kind and `progress` numeric; `params.rs` and `fields.rs` kind
  lists include it where the part B kinds are.
- **D2 Statement.** `gantt_sql`: `SELECT x AS task, series AS group,
  min(start) AS start, max(end) AS end[, avg(progress) AS progress] FROM
  ... WHERE <filters> AND end >= start GROUP BY task, group ORDER BY start
  LIMIT 1001`, plus a second count of rows with `end < start` for the
  note (one statement with `countIf` in a `WITH` or a sibling query
  through `try_run_spec_sql`, the developer's choice, either through the
  role rewrite). 1001 to tell "more than 1,000". Dashboard filters and
  parameters apply as for other kinds.
- **D3 Rendering.** ECharts `custom` series (`renderItem` drawing a rect
  from start to end per category row), y axis = tasks, x axis = time,
  colour by group with the chart's palette (`BI-17` A colours), a
  `markLine` at today in the report time zone, the progress shading, a
  tooltip with task, group, start, end, duration. Drill on a bar as for
  other kinds. The XSS rules of `chart-option.ts` (escape labels) apply.

### C2 Maps

- **D4 Bundled maps (C2a).** Natural Earth 1:50m admin-0 (world by
  country, with `ISO_A2`/`ISO_A3` kept as properties) and 1:10m admin-1
  split per country into `public/geo/admin1/<ISO_A3>.geojson`, produced by
  a script in `ops/geo/` (Python, `from __future__ import annotations`,
  reads a downloaded archive path given on the command line; it never
  downloads in tests) using mapshaper or `shapely` simplification with the
  settings written in `public/geo/README.md` (source, version, licence:
  public domain, simplification). `MAP_CATALOGUE` gains `world` and one
  entry per country file (label "<Country> — states/provinces"). Indonesia
  keeps its own provinces file as the default for `IDN`.
  `src/lib/geo-regions.ts` matches by `name` and, for `world`, by ISO code.
- **D5 Uploaded maps (C2b).** `POST /api/maps` (multipart, `settings:write`,
  50 MB like `uploads::MAX_UPLOAD_BYTES`, its own route timeout entry),
  `GET /api/maps` (`RequiresAuth`), `GET /api/maps/{id}/geojson`
  (`RequiresAuth`, `Cache-Control: private, max-age=3600`, ETag),
  `PATCH /api/maps/{id}` (rename, `settings:write`), `DELETE /api/maps/{id}`
  (`settings:write`; refused with the list of charts using it). All in
  `POLICY_TABLE`, both ways in `route_auth`. Validation in a new
  `lakehouse-bi/src/geo.rs` (pure, unit-tested): FeatureCollection of
  `Polygon`/`MultiPolygon`, longitudes in -180..180 and latitudes in
  -90..90, at most 20,000 features, the chosen name property present and
  non-empty on every feature (duplicates refused with the names), rings
  closed. Simplification with the `geo` crate (`SimplifyVwPreserve`,
  tolerance stepped until the output is ≤ 5 MB or a floor is reached;
  the floor reached → refused with a sentence). New dependency `geo` (and
  `geojson`), pinned; `cargo deny check` and `cargo audit` must pass.
  Storage: Postgres `geo_map` (migration: next free number at build time;
  check every branch; `0098`+ are being taken by `BI-19`) with id, name,
  name property, region count, simplified GeoJSON (`bytea`, gzip),
  created by/at; the full polygons go to ClickHouse
  `console.geo_region (map_id String, name String, polygon
  MultiPolygon)` (created by the store's `CREATE TABLE IF NOT EXISTS`
  pattern) for C3. Map ids are server-made (`m_` + 12 hex) so they are
  safe identifiers. The console's `ensureMap` loads an uploaded map from
  `/api/maps/{id}/geojson` through `apiFetch` (not the static `fetch` the
  bundled files use; comment at the site).
- **D6 Grid maps.** The existing `grid` on `pointmap`/`geoheat`
  (`shape.rs` grid check, `bin.rs` `GridSpec`) works on any map id; no new
  grid code. Only a test that a pointmap with grid on an uploaded map id
  saves and renders.

### C3 Spatial

- **D7 Functions.** In `lakehouse_bi::formula::catalog::CATALOG`:
  `Distance(lat1, lon1, lat2, lon2[, 'km'|'m'])` compiled to
  `geoDistance(lon1, lat1, lon2, lat2)` (metres, WGS 84) divided as asked;
  `RegionOf('<map name or id>', lat, lon)` compiled to
  `dictGetOrDefault('console.geo_dict_<id>', 'name', (toFloat64(lon),
  toFloat64(lat)), '')`. The map argument must be a literal resolved at
  compile time to a map id the caller can see; the identifier is built
  only from that server-made id (a constant identifier, the one allowed
  `format!` use). Both are row-level (not aggregates).
- **D8 Dictionary.** When a map is uploaded (C2b), the API also creates
  `CREATE DICTIONARY IF NOT EXISTS console.geo_dict_<id> (key
  Array(Array(Array(Array(Float64)))), name String) PRIMARY KEY key
  SOURCE(CLICKHOUSE(QUERY 'SELECT polygon AS key, name FROM
  console.geo_region WHERE map_id = ''<id>''')) LAYOUT(POLYGON())
  LIFETIME(0)` (exact DDL per the must-verify below), and drops it on
  delete. The spatial join is then a calculated field grouped or filtered
  like any column.
- **D8a RegionOf and data policies (signed by the owner, 2026-10-24).** The
  policy rewrite refuses every `dictGet*` call of a caller who has any
  authored obligation (`sql_rewrite::DictOrJoinFunctionDenied`), because a
  dictionary's source cannot be seen from the query text. **Signed by the
  owner, 2026-10-24: C3 `RegionOf` option B** (works for callers without
  obligations; a caller with a data policy gets a clear English refusal naming
  `RegionOf`); the narrow exemption for the server-made `console.geo_dict_*`
  dictionaries (option A) is a backlog follow-up for the security team. No
  change to `sql_rewrite.rs` or `policy_engine.rs` in this part.

### C4 Custom charts (only after the owner's yes)

- **D9** Not planned in tasks until Q3 is answered. If yes, the planner
  writes its tasks into this file as T11+ (a `custom` kind whose
  definition is an ECharts option template with `{{column}}` placeholders,
  validated in Rust to contain no functions, no `formatter` strings with
  HTML, no URLs; rendered with the chart's rows). If no, the backlog row
  records the owner's decision and the spec's line moves to "Not
  included".

### Assistant (AI-5), per part

- **D10** C1: the chart tools' kind enum and properties (`start`, `end`,
  `progress`) in `routes/ai/registry.rs` beside the part B block. C2: a
  read tool `list_maps` (built-in and uploaded, ids and labels), the map
  property's description names it. C3: nothing new: the functions reach
  the assistant through `list_formula_functions` (the catalog); one
  prompt line on `RegionOf`. Each part regenerates
  `tests/fixtures/tool_schemas.json` with the ignored writer test, moves
  the count pin, updates `capabilities.ts`, and adds one case to
  `ops/ai_eval/dashboard_cases.json`.

## 2. Anchors (verified at `150a081a`, find by symbol)

- Kinds: `lakehouse-bi/src/specs.rs` `ChartKind` (`Geomap`, `Pointmap`,
  `Geoheat`, the part B kinds, `is_static`); `shape.rs` `is_grouped_kind`,
  `check_kind_fields`, the `grid` check; `fields.rs` and `params.rs` kind
  lists (`ChartKind::Histogram`, `ChartKind::Bullet` arms); `builder.rs`
  `histogram_sql`, `build_points_sql`, `point_limit`, `POINT_LIMIT`;
  `bin.rs` `GridSpec`; `routes/support.rs` `render_stored_spec` (`map`,
  `lat`, `lon` fields).
- Console kinds: `src/lib/dashboard-specs.ts` `ChartKind`;
  `src/features/dashboards/chart-option.ts` (scatter branch, XSS escaping),
  `tile-body.tsx`, `chart-builder.tsx`.
- Maps: `src/features/dashboards/echarts-maps.ts` `MAP_CATALOGUE`,
  `ensureMap`, `mapCredit`, `DEFAULT_CHOROPLETH_MAP`; `geo-chart.tsx`,
  `geo-option.ts`; `src/lib/geo-regions.ts`; `public/geo/*.geojson`,
  `public/geo/README.md` (the provenance table to extend).
- Uploads: `routes/uploads.rs` `MAX_UPLOAD_BYTES`,
  `MAX_REQUEST_BODY_BYTES`, `check_file`; `routes/mod.rs` `route_timeout`
  (`UPLOAD_REQUEST_TIMEOUT`).
- Formula: `lakehouse_bi::formula::catalog::CATALOG`;
  `routes/ai/tools/calc_fields.rs` `list_formula_functions`.
- Store pattern: `lakehouse-bi/src/store.rs` `CREATE TABLE IF NOT EXISTS
  console.bi_chart` and the `ALTER ... ADD COLUMN IF NOT EXISTS` lines.
- Assistant: `routes/ai/registry.rs` (the `BI-16` part B properties block
  at the `"histogram"` schema), `capabilities.ts`,
  `ops/ai_eval/dashboard_cases.json`.

## 3. Tasks (one commit each, in order within a part; Rust first)

### C1 Gantt

**T1 Kind and statement (D1, D2).** *Accept:* `kind_check` tests (missing
start/end, wrong kinds, progress not numeric); `gantt_sql` golden tests
(filters, parameters, SQL source with its settings, limit 1001, the
reversed-row count); route test through the role rewrite (a masked group
column stays masked); `render_stored_spec` round-trip.

**T2 Assistant for Gantt (D10 C1).** *Accept:* schema fixture, count pin,
`ai_tool` test that `create_chart` with `kind: gantt` saves.

**T3 Console Gantt (D3).** *Accept:* option tests (bars from start to end,
groups coloured, today line in the report zone, progress shading, more
than 1,000 → note, reversed rows note, labels escaped); builder fields
round-trip; drill on a bar.

### C2a Bundled maps (runs beside T1)

**T4 Natural Earth files (D4).** Script, generated files, README rows,
`MAP_CATALOGUE`, ISO matching. *Accept:* the script's unit tests (on a
three-feature fixture, no network); the total size of `public/geo` after
the change quoted in the commit (stop and report if over 25 MB);
`geo-regions` tests for ISO-2/ISO-3 and names; a world geomap renders in
a component test with a fixture.

### C2b Uploaded maps

**T5 Validation and simplification (D5, `geo.rs`).** *Accept:* unit tests:
valid FC accepted; point features, bad coordinates, open rings, missing
or duplicate names, over 20,000 features refused with sentences; a 30 MB
fixture generated in the test (not committed) simplifies under 5 MB;
`cargo deny check`, `cargo audit` green with `geo`/`geojson`.

**T6 Routes and storage (D5).** Migration, `geo_map` store, ClickHouse
`geo_region`, the five routes. *Accept:* store round-trip; `route_auth`
both ways; upload over 50 MB refused before parsing; delete of a map in
use refused with chart names; the GeoJSON route answers the simplified
file with ETag; SEC-11 guard green.

**T7 Assistant `list_maps` (D10 C2).** *Accept:* tool test lists built-in
and uploaded; fixture; count pin.

**T8 Console: Admin > Maps and the map picker.** Upload with name
property choice (read from the file's first feature in the browser),
rename, delete; uploaded maps in the chart builder's map select;
`ensureMap` for uploaded ids. *Accept:* component tests; typecheck, lint,
tests; webpack build.

### C3 Spatial

**T9 Functions and dictionaries (D7, D8).** *Accept:* formula compile
tests (`Distance` units, `RegionOf` by name and id, unknown map refused,
a map id never taken from user text unresolved); dictionary DDL created
on upload and dropped on delete (mock engine); a live test, ignored by
default, that `RegionOf` on three known points of the Indonesia provinces
file names the right provinces and `Distance(Jakarta, Bandung)` is within
1 % of a published figure (quoted with its source in the test); the role
rewrite accepts a statement with `dictGetOrDefault` on a `console.geo_dict_*`
name and the masks still apply to the point columns.

**T10 Assistant line and AI-15 case (D10 C3).** *Accept:* prompt test;
`list_formula_functions` output contains both; AI-15 case added.

### C4 Custom charts

**T11+** Written after the owner's answer to Q3 (D9).

## 4. Out of scope

Map tiles or basemaps from outside, routing and travel time, editing
boundaries, admin-2 for countries other than Indonesia, projections other
than ECharts' default, plugins that run code (unless the owner reverses
Q3's recommendation, in which case the planner writes a separate plan
with a security review first).

## 5. Must verify, not assume

- Natural Earth's licence statement and the exact release used; the total
  size of the per-country admin-1 files after simplification.
- That ECharts' `custom` series with `renderItem` works in the PDF render
  view (animation off) and in the phone layout.
- The exact `CREATE DICTIONARY ... LAYOUT(POLYGON())` syntax and key type
  on ClickHouse 26.8, `dictGetOrDefault` with a `(lon, lat)` tuple, and
  its memory for a 20,000-region map; record in a RESULT note in the
  handoff.
- How `policy_engine::rewrite_sql_for_roles` and `sql_rewrite` treat a
  `dictGet*` call (refused as a table-like access, or passed through); if
  refused, stop and report: the planner decides with the security owner.
- Whether the API's ClickHouse user may `CREATE DICTIONARY` in `console`;
  if not, stop and report (grants are owned elsewhere).
- How the chart builder reads a column's kind for `start`/`end` (dates
  versus date-times versus strings) so the Gantt check matches the
  builder's own column kinds.

## 6. Checks

As in the phase 4 plans (same cargo environment:
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0`, one cargo command at a time, `--test-threads=4`,
`route_auth` at 2; check `free -g` / `df -h /` first; never `cargo build -p
lakehouse-api`; no `rust/target`; no prettier; touch the crate sources
before clippy when several worktrees share the target dir). Per task the
crates touched (`lakehouse-bi`, `lakehouse-store`, `lakehouse-api`), the
`lakehouse-api` lib filters for the modules touched (`maps`, `support`,
`ai`, `calc_fields`), the integration files `route_auth`, `route_policy`,
`sec11_guard`, `ai_tool`; `cargo deny check` and `cargo audit` after T5.
Python: `ops/geo` script tests with `pytest -q` on its folder. Once,
before each part's handoff: `cargo fmt --check` and whole-workspace
`cargo clippy --workspace --all-targets --all-features -- -D warnings` with
the environment above (no full workspace test run on this machine; CI runs
it on the PR). TS: typecheck, lint, tests of the files touched; the
webpack build after T3, T4 and T8.

## 7. Handoff (developer appends, one entry per part)

### C2a Bundled maps (developer, branch `feat/bi-phase-6`)

Commit: `840cf5e0` feat(maps): BI-16 C2a world and country maps from Natural Earth (T4). TypeScript, Python and static data only; no cargo command run, no Rust change (the API only checks the shape of a map id, `store.rs` `map_id_is_well_formed`, which `world` and `adm1-usa` satisfy).

Data (Natural Earth 5.1.1, public domain; the archives were downloaded into a scratch directory outside the repo, read with `zipfile` only, and deleted afterwards): `public/geo/world.geojson` 743,310 bytes (242 countries); `public/geo/admin1/` 3,838,691 bytes in 250 files (largest `RUS` 200,785); `public/geo` in all 5,724,669 bytes (255 files), against the 15 MB budget and the plan's 25 MB stop line.

Deviations and findings, for review:
- **No mapshaper/shapely.** Neither is installed and installing was not allowed; the script (`ops/geo/build_natural_earth.py`) parses the shapefile with `struct` and simplifies with Douglas-Peucker (settings in `public/geo/README.md`). D4 names them as options, not requirements.
- **Per-country files are keyed by Natural Earth `adm0_a3`**, which differs from ISO for a few places (for example `KOS`). Catalogue ids are `adm1-<a3 lower>`; Indonesia has none (D4).
- **ISO codes:** `ISO_A2_EH`/`ISO_A3_EH` repair the -99 codes of France and Norway. Somaliland, N. Cyprus and the Siachen Glacier have no ISO code; the properties are empty rather than filled with Natural Earth's own codes. Kosovo carries `XK`/`KOS`, not ISO.
- **Name matching:** `plain()` now also ignores accents, for every level (the Indonesian files have none, so nothing changes there; pinned by tests). New levels `country` and `subdivision`; aliases (ISO codes, `name_long`, `iso_3166_2`) are read from the map's own properties by `ensureMap` and never override a feature name; a short table covers UK, Burma, Ivory Coast and a few more.
- The country picker in the chart builder now lists 251 maps in one select (sorted by country name after the three Indonesian ones). No search box was added; that is a UI decision for the planner.
- **Webpack build not run** (plan section 6 asks for it after T4): not verified, to keep the VM's disk and memory free; CI runs it.
- A world `geomap` rendered in a real canvas/browser was not checked; the component test hands the option to a mocked `EChart`.

Checks (all from `/home/hv/lakehouse-phase6-build`):
- `bun run typecheck`: exit 0.
- `bun run lint`: 0 errors, 9 warnings, none in a file of this commit.
- `bun test src/lib/geo-regions.test.ts src/features/dashboards/natural-earth-maps.test.tsx`: 24 pass, 0 fail.
- `bun test src/features/dashboards src/lib`: 1317 pass, 1 skip, 0 fail (151 files).
- `bun run test`: 2444 pass, 1 skip, 0 fail (251 files).
- `python3 -I -m unittest discover -s ops/geo -p 'test_*.py'`: 11 tests OK (pytest is not installed here; the tests are plain `unittest`, which pytest also runs).
- Existing behaviour: `natural-earth-maps.test.tsx` pins the three Indonesian/Jakarta catalogue entries, the two defaults, the credit text, and draws Jakarta (no map set) and `id-provinces` through `GeoChart` with the old name rules.


### C1 Gantt, console half (developer, branch `feat/bi-phase-6`)

Commit: `75a9cf46` feat(dashboards): BI-16 C1 Gantt chart in the console (T3). TypeScript only; no cargo command run, no Rust change. **T1 (kind and statement) and T2 (assistant) are not built: the server rejects the `gantt` kind on save and on preview until T1 lands**, so the builder can fill the fields and the tile can draw, but a real chart cannot be saved or read yet.

What it does (plan D1, D3): `gantt` in `ChartKind` and the builder's type list (group "Schedule"); `features/dashboards/gantt-option.ts` (the option builder: ECharts `custom` series, one per group, colours from the chart's palette; time axis in UTC over naive wall times; line at today from `wallClockMs` in the report zone, drawn only inside the tasks' range; progress shading; first 1,000 tasks by start with "Showing the first 1,000 tasks by start"; reversed-row and undated-row notes; HTML tooltip with escaped task and group); `GanttBody` in `tile-body.tsx`; `GanttFields` (start, end, progress) in `type-b-fields.tsx`. The task is the `dimension`, the group the `breakdown`; `start`, `end`, `progress` are flat on the chart in `TypeDefFields` (so the preview key, the edit round trip and the draft preview carry them like `lat`/`lon`). A Gantt has no measure, aggregate, sort or limit in the builder. Drill: a bar reports its task as the hit name, so the default `drillTarget` filters on the task column.

Assumptions to check against T1 (the plan leaves them open):
- Result columns are D2's aliases `task`, `group`, `start`, `end`, `progress`; the tile reads exactly these names, not `spec.x`.
- The count of rows with `end < start` (D2: "a second count ... for the note") is read from an optional `reversedRows` on the tile result (`Rows.reversedRows`); absent means no note. T1 must return it under that name, or this changes.
- Start and end arrive as `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS` strings in the report zone; the zone conversion is the server's.
- Not built because the plan's D1 to D3 do not define a field for it: the switch that turns the today line off (feature decision 2 says "can be switched off"). The line is always on while today is in range. A field (`today`) would need T1 first.
- The line at today is a `markLine` on an empty `line` series, because a `custom` series has no mark lines of its own to rely on. Not seen in a real browser (see below).

Checks (from `/home/hv/lakehouse-phase6-build`):
- `bun run typecheck`: exit 0.
- `bun run lint`: 0 errors, 10 warnings, none in a file of this commit.
- `bun test src/features/dashboards/gantt`: 17 pass, 0 fail (option: bars start to end and axis order, date-time reading, groups coloured with a legend, today in the report zone and out of range, progress shading, 1,000 cut with note, reversed note, empty message, escaped tooltip; tile: notes, no data, drill target; builder: type offered, fields round trip with no measure/order/limit sent, start and end required).
- `bun run test`: 2536 pass, 1 skip, 0 fail (260 files).
- Not verified: the webpack build; a Gantt drawn in a real canvas (the tests hand the option to a mocked `EChart`, and the geometry of a bar is tested through `ganttBarItem` with a stub API); the `markLine` on the empty line series.


### C1 Gantt today-line switch, and C2b Admin > Maps (console halves; developer, branch `feat/bi-phase-6`)

Commits: `b410aa54` feat(dashboards): BI-16 C1 Gantt "Today line" switch (todayLine style field); the fix `Used by` guard and the C2b commit `feat(maps): BI-16 C2b Admin > Maps` follow it in the log. TypeScript only; no cargo command, no Rust change.

**Gantt switch.** `todayLine?: boolean` on `StyleFields`; `FormatRules.todayLine` (Gantt only); a "Today line" switch in the format section, on by default; only `todayLine: false` is stored or sent (`stylePayload`, `styleFromDef`); `ganttOption` leaves the line out when it is false. Tests in `format-section.test.tsx` and `gantt-option.test.tsx`. T1 validates the field; until then the server rejects it with the gantt kind.

**C2b console (T8), contract for T5/T6 (the plan names routes and permissions, not JSON; these shapes are the console's choice).** Types are in `src/services/contracts/maps.ts`, the client in `src/services/clients/maps.ts`.
- `GET /api/maps` answers `{ maps: [{ id, name, nameProperty, regionCount, createdAt, createdBy? }] }`; `id` is `m_` and 12 lower-case hex digits (the console draws any id of that form without the list).
- `POST /api/maps`: multipart with `name`, `nameProperty` and `file`; answers the map. A refused file answers 400 or 413 with `{ error }` as a plain sentence the console shows as is.
- `PATCH /api/maps/{id}` `{ name }` answers the map. `DELETE /api/maps/{id}` answers `{ ok: true }`, or 409 with `{ error, charts: [{ chartId, chartTitle, dashboardId, dashboardTitle }] }` when a chart uses the map.
- `GET /api/maps/{id}/geojson` must answer the simplified file with the chosen name property **copied to `properties.name` on every feature**, so `ensureMap` reads it like a built-in map (it reads `properties.name`). The console fetches it through `apiFetch`, comment at the site.
- Owner decision Q4: the form refuses a file over 50 MB before reading it ("This file is over 50 MB. Simplify it or split it, then upload again."), and says the file is simplified to 5 MB or less for drawing.

Built: Admin > Maps (`features/admin/maps-page.tsx`, route `/admin/maps`, sidebar entry): upload with the name property chosen from the first feature's properties (only the first 1 MB is read, `lib/geojson-head.ts`), list, rename, delete (a refused delete lists the charts); read-only text without `settings:write`. Uploaded maps in the chart builder's map select under "Uploaded by your administrator" (`use-map-catalogue.ts`, `map-picker.tsx`); `MapEntry.uploaded`, region level `uploaded` (names matched as typed, ignoring case, accents and spacing). The sidebar entry is shown to everyone who sees Administration; the server refuses what `settings:write` does not allow.

Server pieces still needed: T5 (validation), T6 (the five routes, storage), T7 (`list_maps`, assistant), T1 and T2 for Gantt. A pin or grid map on an uploaded id has no console test beyond the catalogue (D6 asks one; it needs T6's save to be meaningful).

Checks (from `/home/hv/lakehouse-phase6-build`): `bun run typecheck` exit 0; `bun run lint` 0 errors, 9 warnings, none in these files; `bun test src` (summary only): 2570 pass, 1 skip, 0 fail (265 files). New tests: `maps-page.test.tsx` 8, `geojson-head.test.ts` 8, `uploaded-maps.test.tsx` 4, plus the Gantt switch tests. Not verified: the webpack build, an upload against a real server, a map drawn from an uploaded file in a browser.


### C1 Gantt, server half (T1 and T2; developer, branch `feat/bi-phase-6`)

Commits: `75997369` (T1), `0f34504e` (T2, with the demo seed). Rust; verified with `cargo check` and `cargo clippy` only where noted (disk was the constraint; no `lakehouse-api` tests were run locally, CI on the draft PR does).

**T1.** `ChartKind::Gantt`; `start`, `end`, `progress` on `TypeFields` (flat, refused on any other kind, absent on every old chart); `todayLine` on `StyleFields` (Gantt only, `check_kind_bound_fields`). The task is the `dimension`, the group the `breakdown`; a Gantt chart takes no measure, aggregate, limit, order or grain (`types_b::check_gantt`). Save-time column checks in `store.rs` (`check_gantt_columns`): task, group, start, end, progress exist; start and end are date or date-time; progress is a number. Statement (`builder::gantt_sql`): an inner select renames the columns to `__t`, `__g`, `__s`, `__e`, `__p` (so a source column called `start` cannot shadow the result), the dashboard's predicates sit inside it on the raw rows, the outer select keeps rows with `__e >= __s`, groups by task and group, orders by first start and keeps 1,001. Result names are the console's: `task`, `group`, `start`, `end`, `progress`. Timestamps are converted to the report zone in SQL. `TableRead::Reversed { count_sql }` carries the count of rows that end before they start; `annotate_table` runs it through the same role rewrite and puts `reversedRows` on the tile when it is above zero. A Gantt chart is always rebuilt on read (the count is not stored).
- Contract points of the console half, all met: result column names; `reversedRows`; start and end as `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS`; `todayLine` validated.
- Deviations: the "undated rows" note of the console cannot occur from this statement (rows with a NULL start or end fail `__e >= __s` and are dropped without being counted); say so if the owner wants them counted.
- Tests (`lakehouse-bi`): field checks, wire round trip, golden statement with a dashboard filter and the reversed count; `type_b_rewrite.rs` role-rewrite test for the statement (not run locally). Not tested: a masked group column (the existing `only_jakarta` policy was used).
- Checks run: `cargo test -p lakehouse-bi gantt` 3 passed; `cargo clippy -p lakehouse-bi --all-targets -- -D warnings` clean; `cargo check -p lakehouse-api --tests` clean. Not run: the whole `lakehouse-bi` suite, any `lakehouse-api` test.

**T2.** `create_chart` and `update_chart` take kind `gantt` and `start`, `end`, `progress` (`with_type_properties`); `format_chart` takes none. Fixture regenerated by editing the JSON (it round-tripped byte for byte through Python before the edit; only the additions differ). Test `the_request_to_show_the_project_plan_creates_a_gantt_chart` (mock mart; not run locally). `ai_tool` integration (create via the gate) not added: the unit test above covers the save.

**Demo data for the Gantt QA:** `ops/demo/mart_demo_projects.sql` (15 tasks, three teams, two reversed rows). The repo has no seed for `mart_demo_map_points`, so this one is a hand-run script; it writes to ClickHouse only when someone runs it (dev stack only, `DROP TABLE IF EXISTS` then create).

### C2b Uploaded maps, server half (T5, T6, T7; developer, branch `feat/bi-phase-6`)

Commits: T5 `a109bf98`, T6 `fb096312`, T7 (next commit, `list_maps`). Rust plus one line of TypeScript (`capabilities.ts`). Checked with `cargo test -p lakehouse-bi geo::` (9 passed) and `cargo clippy ... --all-targets -- -D warnings` on `lakehouse-bi`, `lakehouse-store` and `lakehouse-api` (clean); the store and route integration tests and the `lakehouse-api` lib tests were **not run locally** (disk): CI is the check.

**Deviations from D5, for the reviewer**
- **No `geo`/`geojson` crates.** `lakehouse_bi::geo` has its own Douglas-Peucker pass (rings stay closed and never drop below a triangle, positions rounded to five decimals, tolerance doubled from about a metre up to about 5 km until the written file is at most 5 MB, else a sentence). Reasons: the disk this was built on, and no change to `cargo deny` / `cargo audit`. The contract (a closed ring, at most 5 MB) is the same; it does not preserve topology between rings (neighbouring borders may show hairline gaps, which the feature page already says). A 30 MB fixture generated in the test (1.3 M positions) simplifies under 5 MB.
- **Migration `0100_geo_map.sql`** (the plan says "next free above 0098"; `0099` is on main, `0098_public_links` on `feat/bi-phase-4b`).
- **No gzip column.** The file is stored as `bytea` (at most 5 MB, CHECK-ed); Postgres compresses large values itself.
- **`console.geo_region` (ClickHouse) is not created here.** Its only reader is part C3 (`RegionOf`, T9); creating an empty table now would be dead code. T9 owns it.
- **No tenant column**, per the plan ("visible to everyone who can make charts"). In a multi-tenant installation every tenant sees every uploaded map; say so if that is not wanted.
- No audit event is written for upload, rename or delete (the plan does not ask for one); easy to add if the owner wants it.

**Contract with the console (met):** `GET /api/maps` -> `{ maps: [{ id, name, nameProperty, regionCount, createdAt, createdBy }] }`; `POST` multipart `name`, `nameProperty`, `file` -> 201 and the map, 400 with the sentence (413 does not exist in `ApiError`, so the over-50-MB refusal is a 400 with the console's own sentence, read before parsing); `PATCH` `{ name }` -> the map; `DELETE` -> `{ ok: true }` or 409 `{ error, charts: [{ chartId, chartTitle, dashboardId, dashboardTitle }] }` (only charts the caller may see are named; a delete the chart store cannot check is refused); `GET /api/maps/{id}/geojson` -> `properties.name` on every feature, `ETag` (Postgres `md5`), 304 on `If-None-Match`, `Cache-Control: private, max-age=3600`. All five routes are in `POLICY_TABLE` (GET reads `RequiresAuth`; the rest `settings:write`), so the table-driven `route_auth` / `route_policy` tests cover them both ways; `/api/maps` has the upload timeout.

**T7.** `list_maps` (Read, `dashboard:read`): the four fixed built-in maps by id and label, the rule for the per-country ids (`adm1-<iso3 lower>`), and the uploaded maps from Postgres. The tool count is 107 (pin updated), the fixture has the tool appended, the tool is in the dashboard domain of the prompt (`list_maps` beside `list_charts`; no new keyword, so it is offered when the conversation already is about charts) and in `capabilities.ts`. Not done: a tool test that lists an uploaded map (needs Postgres in the lib test state), the `AI-15` case.

### Review fix S1 (developer)

SHOULD-FIX S1 fixed: upload, rename and delete of a map each record an audit event (`map_upload`, `map_rename`, `map_delete`; resource kind `map`, the map id, args `{ "name": ... }`, never the file), best effort like the upload events (a failed audit write never fails the write). Extended `maps_routes.rs` (reads `audit_event` after an upload and a rename). Clippy clean; test not run locally (CI).

### C3 Spatial (T9, T10; developer, branch `feat/bi-phase-6`)

Commits: `3149945f` (functions), `7ff6e0c2` (lifecycle), T10 (next). **Owner decision, 2026-10-24: RegionOf option B** (see D8a): works for callers without a data policy; a caller with one gets an English refusal naming `RegionOf`; the narrow exemption is a backlog follow-up for the security team. `sql_rewrite.rs` and `policy_engine.rs` are untouched.

- **Functions.** `Distance(lat1, lon1, lat2, lon2[, 'm'|'km'])` -> `geoDistance(lon1, lat1, lon2, lat2)` (metres; `/ 1000` for km). `RegionOf(map, lat, lon)` -> `dictGetOrDefault('console.geo_dict_<id>', 'name', (lon, lat), '')`; the map literal is resolved against the maps the `TimeContext` carries (`with_maps`, filled by `settings::time_context`); the dictionary name comes only from the server-made id (`geo::dictionary_name`); unknown map, a map whose lookup is not set up, and a non-literal map are refused with sentences. Catalog now has 64 functions (category `spatial`).
- **Refusal for governed callers.** `support::enforcement_message_for` swaps the fixed (Indonesian) `DictOrJoinFunctionDenied` message for an English sentence naming RegionOf when the refused statement names a `console.geo_dict_m_` dictionary (tiles and previews via `try_run_spec_sql`, chart downloads). Other dictionary refusals keep their message.
- **Lifecycle.** After an upload is saved: `console.geo_region` (`CREATE TABLE IF NOT EXISTS`, full boundaries as `Array(Array(Array(Tuple(Float64, Float64))))`), the polygon dictionary, `SYSTEM RELOAD DICTIONARY`; delete drops the dictionary and the rows. Migration **0102** `geo_map.region_lookup` (0101 is `query_limits` on another branch), set only after the dictionary loaded. Failure at any step: logged under a reference id, undone, flag off; the map still draws.
- **NOT VERIFIED on a running engine** (no write access to a ClickHouse here): the dictionary DDL (key type, `LAYOUT(POLYGON())`, `SOURCE(CLICKHOUSE(QUERY ...))`, no `USER`/`PASSWORD` in the source, so it reads as the `default` user), `SYSTEM RELOAD DICTIONARY`, `mutations_sync`, the API user's right to `CREATE DICTIONARY` in `console`, the memory of a 20,000-region dictionary, and the live test of the plan (three known points of the Indonesia provinces; `Distance(Jakarta, Bandung)` against a published figure) which is **not written**: it needs a running engine with the dictionary. If the engine refuses, the flag stays off and `RegionOf` says so.
- **T10.** The build prompt names `Distance`, `RegionOf`, `list_maps`, "never geoDistance or SQL" and the data-policy refusal (test `the_build_prompt_names_distance_and_region_of`); `list_formula_functions` lists both (test `list_formula_functions_lists_distance_and_region_of`); AI-15 case `calc_distance_to_bandung` in `ops/ai_eval/dashboard_cases.json`. No tool schema changed, so the fixture is untouched.
- Checks: `cargo test -p lakehouse-bi` (569 + new, all passed before this part; `geo::` 11 passed), `cargo clippy -p lakehouse-bi -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` clean, `cargo test -p lakehouse-api --lib routes::ai` 424 passed. Not run: store/route integration tests (CI).

### Review fixes S1–S8 (C3)

Commits: `cd3033ae` (S8), `7a49eff5` (S6), `532ecac7` (S1, S2, S3, S4, S5, S7), docs commit (BI-16C-RESULT, feature page limits). No `sql_rewrite.rs` / `policy_engine.rs` change (D8a). No new setting, no compose change.

- **S1 credentials.** Measured on a throwaway `ClickHouse` 26.8.9.10 with a password on `default` (`docs/plans/BI-16C-RESULT.md`): the old DDL creates but cannot load (`516 Authentication failed`). `dictionary_ddl(id, user, password)` now writes `USER`/`PASSWORD` from the API's own `CH_USER`/`CH_PASSWORD` as escaped literals; loads with a password, with an empty one and with `a'b\c`. The password is `[HIDDEN]` in `SHOW CREATE`, `system.tables`, `system.dictionaries` and `query_log`, but **in clear in the engine's metadata file** (written on the feature page; a rotation needs a re-upload). A dedicated read-only engine user would be tighter but needs `CREATE USER` and a compose change: the planner's call.
- **S2 key.** Key is now `Array(Array(Array(Tuple(Float64, Float64))))`; the old form fails with `53 TYPE_MISMATCH` (measured). Test pins both.
- **S3.** Map delete refuses (409, body `fields`) while a calculated field's `RegionOf` names the map by id or name (`compile::region_maps_in`, syntax only); fails closed. Feature page limits: rename breaks label-named fields (use the id). Rename itself is not blocked.
- **S4.** `geo_map::set_region_lookup` returns `bool`; `false` removes the lookup and answers with the flag off. Store test added. Note: the 201 body does not carry `regionLookup` (`MapView` has no such field), so the route tests read the flag from `geo_map`.
- **S5.** (a) Three route tests over a mock engine in `tests/maps_routes.rs` (statement order CREATE TABLE, INSERT, CREATE DICTIONARY, RELOAD; delete drops; engine failure gives 201 with flag false and no upstream text; field-blocked delete). (b) `#[ignore]` live test `formula::compile::tests::live_region_of_names_three_provinces_and_distance_matches_a_published_figure`: Dki Jakarta, Jawa Barat, Bali, open sea empty; Distance CGK to BDO 133.07 km against the published 133 km (airmilescalculator.com, Vincenty), tolerance 1 %; out-of-range gives NULL.
- **S6.** `run_records` and `values_for_roles` use `support::enforcement_message_for`.
- **S7.** `Distance` is `if(abs(lat) <= 90 AND ... , geoDistance(...), NULL)`; out-of-range literals are refused with a sentence.
- **S8.** Doc comment back on `check_and_simplify`.
- **Commands (from `rust/`, shared target, under `flock`):** `cargo fmt --check` clean; `cargo clippy -p lakehouse-bi -p lakehouse-store -p lakehouse-api --all-targets --all-features -- -D warnings` clean (after touching every `lib.rs`); `cargo test -p lakehouse-bi --lib` 574 passed, 0 failed, 4 ignored; `cargo test -p lakehouse-api --lib -- routes::maps routes::support routes::calc_fields routes::dashboard` 205 passed, 0 failed, 3 ignored; the live test with `--include-ignored formula::compile geo::` against the throwaway engine: 52 passed, 0 failed (the live test among them). The throwaway containers were stopped and are gone.
- **Not verified:** the new `maps_routes.rs` and `tests/geo_map.rs` tests were compiled by clippy but not run (need Postgres; CI runs them); `cargo test --workspace`; the 20,000-region dictionary memory and load time (34 provinces: 17.5 MB, 0.33 s); a restricted API engine user's right to `CREATE DICTIONARY`; the AI-15 run.

### QA fix Q1 (BI-16 C2b, owner QA BLOCKER): a region chart on an uploaded map could not be previewed or saved

Cause: `geo_fields` (`lakehouse-bi/src/store.rs`) used `map_id_is_well_formed`, the bundled-map shape (lowercase, digits, `-`), which refuses the underscore of a server-made id `m_` + 12 hex. Fix: accept that shape OR `geo::is_map_id` (reused, no second guard); the message names both. Places checked: the assistant's `create_chart`/`update_chart` go through the same `geo_fields`, so they are fixed with it (their schema text still lists only the bundled maps; unchanged, so `tool_schemas.json` is untouched, and the assistant has no list of uploaded maps yet). The console already took uploaded ids (`mapEntry` with `UPLOADED_ID`, `ensureMap` via `apiFetch` on `/api/maps/{id}/geojson`, region names from `properties.name`, matched by `matchRegionRows`). One console gap fixed: a deleted or unreadable uploaded map showed "Add public/api/maps/... to enable this map"; it now says the uploaded map no longer exists or cannot be read (`geo-chart.tsx`). No server existence check was added: the map delete already refuses (409) while a chart uses the map, and the console is the one place that resolves the map, so there is a single check.

Tests: `a_geomap_on_an_uploaded_map_passes_and_a_malformed_uploaded_id_is_refused` (store.rs; `m_0123456789ab` passes, `m_XYZ`, `M_...`, upper-case hex, wrong length refused, Bar still refuses a map); `a_region_chart_on_an_uploaded_map_is_not_refused_for_its_id` (tests/maps_routes.rs, needs Postgres: runs in CI, compiled here under clippy `--all-targets`, *not run locally*). Commands: `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings` clean; `cargo test -p lakehouse-bi --lib` 575 passed, 0 failed, 4 ignored; `bun run typecheck` clean; `bun test src/features/dashboards/uploaded-maps.test.tsx` 4 pass. No console render test was added (the renderer had no built-in-only assumption; only a message changed).

### Handoff (PR #246 review BLOCKER fixes, 2026-10-26)

Developer (branch `feat/bi-phase-6`). The two review BLOCKERs of #246, one
commit each. No scope change; no new dependency; no compose change.

**Commits**

- `562f11ce` fix(maps): a public link and an embed read a map boundary by
  their own token (PR #246 review BLOCKER 1).
- `e37bbaa8` fix(maps): the region dictionary reads as a password-less engine
  user (PR #246 review BLOCKER 2).

**BLOCKER 1 — the anonymous views read a map boundary by their own token**

The console's `ensureMap` fetched an uploaded map through `apiFetch`, whose
401 calls `redirectToLogin()`; a public link and an embed have no account to
log into, so a map tile sent the viewer to the login page.

- `GET /api/public/dashboard/{token}/maps/{id}/geojson` (new): the share
  token opens the board as `GET /api/public/dashboard/{token}` does, and the
  file is served only when a chart on that board draws on that map; a 404
  otherwise (the same answer for a map the board does not use, for a map that
  does not exist and for a map id that is not server-made, so the route is no
  oracle for what a board holds).
- `POST /api/embed/maps/{id}/geojson` (new): the body is `{ jwt }`, exactly
  as `POST /api/embed/data` takes it, and it opens the same board. The token
  check is now one helper (`routes::embed::embed_grant`) that both routes
  use, so they cannot disagree about what a token opens.
- Both routes are `Policy::Public` (the token is the credential, as for the
  board's data) and are asserted both ways in `tests/route_auth.rs`; the
  public-route count in `policy.rs` moves from nine to eleven.
- In the console `ensureMap(id, load)` takes the loader from its caller: the
  public link and the embed pass a token-based loader, the signed-in console
  keeps `apiFetch`. A boundary that cannot be fetched shows a fixed sentence
  on the tile ("This map's boundary file could not be loaded."), never a
  redirect.
- Tests: `tests/maps_routes.rs` (a map the board uses → 200 and the file; a
  map it does not use → 404; a token that is not a board → the same 404 the
  public dashboard route gives; a bad JWT → the same fixed 401
  `POST /api/embed/data` gives), `tests/route_auth.rs` (the two new routes
  are not refused by the auth gate), and
  `src/features/dashboards/public-map-boundary.test.tsx` (the public link and
  the embed read the boundary from their own route and never reach
  `apiFetch`; a boundary that cannot be read shows the sentence).

**BLOCKER 2 — the dictionary reads as a password-less engine user**

The API's `ClickHouse` password was written into every map dictionary's DDL
(in clear in the engine's own metadata files). Per the planner's decision the
dictionary now reads through `lakehouse_geo`, which has NO password, may log
in from the local host only and may only `SELECT console.geo_region`:

- `lakehouse_bi::geo`: `GEO_ENGINE_USER`, `GEO_USER_DDL`
  (`CREATE USER IF NOT EXISTS lakehouse_geo NOT IDENTIFIED HOST LOCAL`) and
  `GEO_GRANT_DDL` (`GRANT SELECT ON console.geo_region TO lakehouse_geo`),
  next to `REGION_TABLE_DDL`; `dictionary_ddl(id)` writes
  `SOURCE(CLICKHOUSE(USER 'lakehouse_geo' QUERY ...))` with no `PASSWORD`
  and no API credential (the user and password parameters and the now-unused
  `string_literal` are gone).
- `routes::maps::enable_region_lookup` runs the two statements idempotently,
  right where `REGION_TABLE_DDL` is run (the first upload that needs the
  lookup), before the inserts and the dictionary.
- Measured on a throwaway `ClickHouse` 26.8.30.8 (`clickhouse/clickhouse-server:26.8`,
  the tag `docker-compose.yml` runs): `NOT IDENTIFIED` and `HOST LOCAL` are
  accepted; the dictionary loads and `dictGet` answers; from another
  container on the same docker network the engine user is refused (516); the
  engine's metadata holds no password (the stored DDL reads
  `SOURCE(CLICKHOUSE(USER 'lakehouse_geo' QUERY ...)`); the user may not read
  another table and may not write (497). The commands and their outputs are
  in `docs/plans/BI-16C-RESULT.md`.
- The ignored live test passes against that engine with the new DDL:
  `LIVE_CH_URL=http://172.17.0.5:8123 LIVE_CH_USER=admin LIVE_CH_PASSWORD=adminpw
  cargo test -p lakehouse-bi --lib live_region_of -- --ignored` → 1 passed,
  0 failed.
- The cost, measured and written on the feature page: the API's engine login
  needs `CREATE USER` and `GRANT`. Without them the upload still answers 201
  and the map still draws, but `RegionOf` has no lookup for it (the
  lifecycle's fail-closed path).
- Docs: `docs/plans/BI-16C-RESULT.md` (the measurement, and the S1 section
  marked superseded) and the feature page's limit line
  (`docs/core/features/gantt-world-maps-spatial.md`).

**Commands and counts (final tree, 2026-10-26, in the foreground)**

- `cd rust && cargo fmt --check` → clean, no output.
- `/home/shiro/rantai/bin/cargo-locked clippy -j 2 --workspace --all-targets --all-features -- -D warnings` → Finished, 0 warnings.
- `/home/shiro/rantai/bin/cargo-locked test -j 2 -p lakehouse-bi` → 575 passed, 0 failed, 4 ignored (lib) + 1 passed (`specs_match_typescript`).
- `/home/shiro/rantai/bin/cargo-locked test -j 2 -p lakehouse-api --test route_auth` → 60 passed, 0 failed.
- `/home/shiro/rantai/bin/cargo-locked test -j 2 -p lakehouse-api --test maps_routes` → 11 passed, 0 failed.
- `/home/shiro/rantai/bin/cargo-locked test -j 2 -p lakehouse-api --test embed_tokens` → 24 passed, 0 failed.
- `/home/shiro/rantai/bin/cargo-locked test -j 2 -p lakehouse-api --lib -- maps` → 22 passed, 0 failed, 2564 filtered out.
- `flock /home/shiro/rantai/.heavy.lock bun run typecheck` → exit 0.
- `flock /home/shiro/rantai/.heavy.lock bun run lint` → 0 errors, 9 warnings, none in a file of these commits.
- `flock /home/shiro/rantai/.heavy.lock bun test src/features/dashboards/public-map-boundary.test.tsx src/features/dashboards/uploaded-maps.test.tsx src/features/dashboards/natural-earth-maps.test.tsx` → 17 pass, 0 fail (3 files).
- `flock /home/shiro/rantai/.heavy.lock bun run test` → 2718 pass, 1 skip, 0 fail (280 files).

The repository has no `vitest`: the console's test runner is `bun test`
(`bunfig.toml` preloads happy-dom for the DOM). The new console test was
therefore written for `bun:test`, like every other test file under `src/`,
and run with the repo's runner.

**Not verified**

- `cargo test --workspace` was not run here (the crates above were); CI runs
  the whole workspace on the PR.
- The webpack build was not run (CI runs it).
- The map was never drawn in a real canvas: the component tests hand the
  option to a mocked `EChart` (as the earlier parts of this plan did).
- The PDF print view (`src/features/dashboards/render-view.tsx`, BI-1's
  surface) still reads an uploaded map through `apiFetch`, so the same
  redirect BLOCKER 1 describes is still open there. It is reported, not
  fixed: that surface is not this PR's, and the fix would need a third
  token-authenticated route.
- The engine-user change on a deployment whose API engine login may not
  create users was measured only as the engine's refusal (`497`), not on a
  running stack; the 20,000-region dictionary's memory was not measured.
- The throwaway engine container (`pr246-ch-geo`, `pr246-ch-client`) and the
  throwaway docker network were removed at the end of the measurement.

## 8. Review (planner appends, one entry per part)

Planner, 2026-10-24: the console's Gantt assumptions are the contract for T1 (result columns `task`, `group`, `start`, `end`, `progress`; optional `reversedRows`; dates as report-zone strings). Feature decision 2's "today line can be switched off" becomes a style field `todayLine: false` (planner default). The console adds the switch, and T1 validates the field.

### Review of C2b T5–T7 (server, `a109bf98`, `fb096312`, `e06018c0`), planner, 2026-10-24

- **Accepted, no `geo`/`geojson` crates:** our own Douglas-Peucker in `lakehouse_bi::geo` keeps the contract (closed rings, at most 5 MB) and leaves `cargo deny`/`cargo audit` unchanged, which also keeps #246 a PR that touches no dependencies while `SEC-51` is red. Hairline gaps between neighbouring borders are already a stated limit.
- **SHOULD-FIX S1, audit the map writes.** Upload, rename and delete of a map change what every tenant's charts can draw; other administrator writes in `routes/` record an audit event. Record one per write (map id and name, never the file), the same way the existing admin routes do.
- **Accepted:** no tenant column (as planned), no gzip (Postgres compresses), `console.geo_region` left to C3 T9; migration `0100` (0099 is on `main`, 0098 on `feat/bi-phase-4b`).

### Review of C3 (T9, T10), planner, 2026-10-25

Commits 3149945f, 7ff6e0c2, 487f78fa, 17a11db5, read with `git show`. CI on #246 is green at 17a11db5; the reviewer did not re-run cargo locally.

No BLOCKER on security: the map id is validated (`is_map_id`, `m_` + 12 lower hex) before every SQL use, region names go in as `JSONEachRow` (serde-escaped), and the governed-caller swap only changes the refusal text (D8a: `sql_rewrite.rs`/`policy_engine.rs` untouched). The feature is unproven on a real engine, so S1 and S2 must be settled before the item can move to In Acceptance.

- **SHOULD-FIX S1, dictionary source credentials.** `geo.rs` `dictionary_ddl` has `SOURCE(CLICKHOUSE(QUERY …))` with no `USER`/`PASSWORD`. Compose sets a password on `default` whenever `CH_PASSWORD` is set, so on any passworded stack the dictionary cannot load and `RegionOf` is dead (fail closed, not a hole). Check on a passworded ClickHouse 26.8 how a local `CLICKHOUSE` source authenticates. Use credentials that work (the API's own user, or a dedicated read-only user for `console.geo_region`). If a password must appear in the DDL, confirm the engine hides it in `SHOW CREATE`, `system.dictionaries` and `query_log`. Record the result in `docs/plans/BI-16C-RESULT.md`.
- **SHOULD-FIX S2, key type.** The table's `polygon` is `Array(Array(Array(Tuple(Float64, Float64))))`, but the dictionary key is `Array(Array(Array(Array(Float64))))`. Make them the same type and prove it loads (with S1).
- **SHOULD-FIX S3, deleting or renaming a map breaks `RegionOf` fields silently.** Delete's 409 in-use check covers chart map specs only. A calc field naming the map falls back to its stored SQL and fails with the generic database error. Extend the check to calc fields whose formula's `RegionOf` names the map (by id or name) and list them. Add to the feature page's limits that a rename breaks fields that name the map by label (recommend the id).
- **SHOULD-FIX S4, an upload racing a delete.** `geo_map::set_region_lookup` ignores `rows_affected`. If the map was deleted mid-upload, the dictionary and rows stay behind, and the 201 says `region_lookup: true`. Return whether a row was updated. On false, remove the lookup and answer with the flag off. Add a store test.
- **SHOULD-FIX S5, T9 acceptance tests missing.** (a) A route test with a mock engine: upload sends CREATE TABLE/INSERT/CREATE DICTIONARY/RELOAD, delete drops, and an engine failure still gives 201 with the flag off. (b) The `#[ignore]` live test: three known Indonesian province points, and `Distance(Jakarta, Bandung)` within 1 % of a published figure with its source quoted. It can be written now.
- **SHOULD-FIX S6, two governed paths keep the generic Indonesian text.** `routes/dashboard.rs` `run_records` (~l.2799) and `values_for_roles` (~l.3811) do not call `support::enforcement_message_for`. Call it there too. This is a text change only.
- **SHOULD-FIX S7, out-of-range coordinates give a meaningless distance.** Measured by the planner on the dev ClickHouse 26.8.9.10: `geoDistance(200, 100, 0, 0)` returns `8963269` and does not throw. Wrap so any latitude outside ±90 or longitude outside ±180 gives NULL, and refuse out-of-range literals at compile time with a sentence (rule 14: unsupported, honestly, over a wrong number).
- **SHOULD-FIX S8, a doc comment moved.** `routes/maps.rs` ~l.213: the comment for `check_and_simplify` now sits on `struct Processed`. Move it back.
- **Accepted:** injection (check 1), the governed-caller swap (only text), the other-tenants question (maps hold boundaries only, governed callers are refused every `dictGet*`), the fail-closed lifecycle and SEC-11, migration 0102, `Distance` unit literals, assistant parity (prompt, catalog, AI-15 case; no schema change, so the fixture is untouched; the AI-15 run is *not verified*), the Rust non-negotiables, and 17a11db5 as a test-only fix that keeps both sides of the check.
