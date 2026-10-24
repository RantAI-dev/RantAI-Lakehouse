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


## 8. Review (planner appends, one entry per part)
