# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project intends to adhere to [Semantic Versioning](https://semver.org/)
once a first release is tagged.

## [Unreleased]

### Added

- Excel workbooks (`.xls`, `.xlsx`) in the file upload (`DATA-9`; plan `docs/superpowers/plans/2026-10-07-upload-excel.md`; ADR 0014, amendment of 2026-10-07). On Sources, Upload file accepts a workbook of up to 50 MB. The Check step lists its sheets (a hidden one is marked and can still be chosen; the first visible one is chosen to begin with), hides the encoding and delimiter, and keeps the header-row control; one sheet is loaded per upload. The API stores the workbook as it arrived, converts the chosen sheet to UTF-8 comma-separated text in memory for the preview, and for the load stores that text beside the original (`<key>.converted.csv`, removed with the upload) and launches the unchanged `file_ingest_job` on it. Every column is text, written like this: a number as the cell holds it, never its display format (`1234.5`, not `1,234.50`; `0.25` for a cell shown as `25%`), a whole number without a decimal point, other numbers in the shortest form that reads back the same (`1e21` and `1.5e-7` in exponent form from `1e21` and below `1e-6`); a date as `2025-09-24`, a date with a time as `2025-09-24 13:30:05`, a time of day as `13:30:00`, a duration as `36:00:00`; a boolean as `true` or `false`; an error cell as its own text (`#DIV/0!`); a formula as the result the file stored (a formula with no stored result is empty); a merged range as its top-left value with the rest empty. Whether a number is a date is decided by the cell's number format. A sheet's rows start at its first filled cell, so the header-row number counts from there. A sheet whose used range is over 5,000,000 cells is refused (a cap, not a measurement). `.xlsm`, `.xlsb`, `.ods`, other zip files, a damaged or password-protected workbook are refused with the reason; a text file named `.xls`/`.xlsx` is still read as text. New in the API: `sheet` on the preview and the load, `workbook` (`sheets`, `defaultSheet`, and `sheet` on the preview) in the answers; the sheet is recorded in the upload's `parseOptions` and in the `upload.ingest` audit event. No migration. New dependency `calamine` 0.36 (MIT) with seven transitive crates, all MIT or Apache-2.0. Not included: loading several sheets at once, typed columns, calculating formulas, `.xlsm`/`.xlsb`/`.ods`.
- `PUT /api/identity/users/{id}/tenants/{tenant_id}` adds an existing user to a tenant (`identity:write`; no body; answers the updated user). A user could be given a tenant only when invited, so an account created without one, such as the bootstrap admin, could not get one through the API. The caller must hold `*:*` or be a member of that tenant; anyone else gets the same 404 as for a tenant that does not exist. Adding a member again succeeds and changes nothing; an add that changes something writes an `identity.user.tenant.add` audit event. The user's next request sees the tenant, with no new login. Removing a membership is not included.
- Connector types show their product's own mark on the New connector page (plan `docs/superpowers/plans/2026-10-05-connector-type-brand-icons.md`): PostgreSQL, MySQL, MariaDB, MongoDB, Apache Kafka, Google Sheets, SAP and MQTT, on the type tiles and the selected-type strip (a CDC type shows its database's mark). Seven marks are copied in from Simple Icons release 16.34.0 (CC0-1.0), and MySQL's dolphin from the `mysql-icon` of SVG Logos by Gil Barbara (`@iconify-json/logos` 1.2.15, CC0-1.0) because Simple Icons' MySQL drawing is the wordmark and unreadable at tile size; no new dependency. A mark is shown only where Simple Icons publishes it today, so Oracle and SQL Server, which Simple Icons removed at the owners' request, keep the generic database icon, as do object storage, SFTP and REST API. Each mark is a trademark of its owner, shown only to name the product. Kafka, MariaDB and MQTT are drawn in the foreground colour in the dark theme, where their brand colour is below 3:1 on the tile chip, and so is MySQL's dolphin, a thin line drawing too faint at 3.16:1. Console only.
- The Schema tab's "Columns" card as a column explorer (plan `docs/superpowers/plans/2026-10-05-schema-tab-column-explorer.md`). One line of the same height per column replaces the seven-column table: a glyph for the family of the type (text, number, date or time, boolean, nested, other), the name with its masked, classification and partition marks and a new "sort key" mark, the type as the engine states it, a bar of the column's most frequent values with the commonest value and its share beside it, the null share, the distinct count and the range. The bar draws only what the profile states: shares are of the rows profiled, there is one segment per listed value (at most five, and only where the count is exact, as before), in one hue at stepped strengths with the commonest strongest, and what no listed value takes is the bare track. The bar and the label beside it sit on two tracks of fixed proportions, so a bar is as long on every row whatever the commonest value is. A column with no listed value reads "All null", "Mostly unique" (its distinct count is at least 90% of its non-null rows) or "Many distinct values" where the bar would be, since an empty list means more distinct values than the profile counts exactly, not that they are unique. A null share under 0.1% reads "<0.1%" in the row, the opened column's list and its facts alike. Pressing a line opens the column under it, in place: every listed value with its own bar, count and share, "Other values" and "Null" when above zero, and the column's facts (type, whether it can be null, nulls, distinct, range, partition and sort key, masking, classification, description). Several columns can be open, and without `query:read` or a profile a column opens on the facts that need none. "sort key" marks only a column that the storage card's sorting key names exactly, never one that is only inside an expression such as `toYYYYMM(d)`. The separate "Nullable" column and the description under the name are gone (both are in the opened column; the filter still matches the description); the filter box shows from 11 columns instead of 26 and sits above the list instead of in the card header, whose slot cannot shrink; the page sizes still start past 25 columns. The list is a grid with table roles instead of a `<table>`, so it has no sideways scroller of its own, and as the card narrows it sheds the range, then the distinct count and the label beside the bar, then moves the type under the name, with the marks after it so the name keeps its whole track. Console only; no API or contract change. Not included: a histogram or time line, sorting the list, and any change to the Sample tab, "System columns" or "Schema versions".
- Schema versions for Silver and Gold tables (ADR 0015; plan `docs/superpowers/plans/2026-10-05-schema-versions-silver-gold.md`): a raw table shows its schema versions from its Iceberg metadata, while a Silver or Gold table, whose engine keeps only the current columns, showed none. The console now records them itself. A pass reads the ordered `(column name, type)` list of every table in `silver` and `serving` (the two databases the catalog serves; not `GOLD_SOURCE_SCHEMA`, which only the export routes read) and, for each table whose list differs from the last one recorded, adds a version to `console.table_schema_version`, a `ClickHouse` table the API creates on first use (no migration, no new route). A pass runs when the API starts, on `POST /api/alerts/run` (the answer gains `schemaPassStarted`; not on a single-rule `?id=` run) and when the orchestrator reports a finished run (`POST /api/pipelines/events/run-finished`); one runs at a time, and a page view only reads. `schemaVersions` on the asset detail of such a table, always `[]` before, now lists its versions newest first as `{ version, at, change, current }`, `change` a sentence per change (`Added <name> (<type>)`, `Dropped <name> (<type>)`, `Changed <name> from <type> to <type>`, `Reordered columns`, and `First recorded with 4 columns` for the first); a failed read leaves it `[]`. The Schema tab lists them with a `current` pill and says since when they are recorded, the Overview's Schema row says "recorded", and the Activity tab's change history includes each one. Limits, stated on the page and in `docs/OPERATIONS.md`: versions start the day the console first looks (the first is dated by that look, not by the table's creation), a time is when the console saw the change, two changes between two looks show as one, a renamed column shows as one dropped and one added, and only the definition is kept, not the rows. Raw tables are unchanged. Verified on the dev stack on 2026-10-05 (plan section 9): the first pass at start-up recorded one version for each of the seven tables it found; on a demo table, an added column, a retyped plus a dropped column, and a moved column each gave the next version with its sentence, a look with nothing changed recorded nothing, and a column added before the orchestrator's 15-minute schedule was recorded at that schedule with no other trigger.
- The Sample tab as a data preview (plan `docs/superpowers/plans/2026-10-05-sample-tab-data-preview.md`). It opens on 25 rows (50 and 100 on request) instead of five; the five the detail body carries show until the answer arrives. A header carries the type's glyph, the column name and its type, taken from the asset's schema (a column the schema does not list shows its name alone and is left-aligned). Numbers are right-aligned in tabular figures and every value is shown as stored, so an id like `0250161` is not reformatted; `NULL` reads `NULL` and an empty text `(empty)`, both in a quiet italic; a long value is cut at a fixed width with the whole value as the cell's title. A row-number gutter and the first column stay put on sideways scroll (the first column only where the grid is 32rem wide or more) and the header stays put on downward scroll, inside a frame of the grid's own. A sort control in each header orders the rows shown, ascending, descending, then the table's order; numbers compare as numbers only in a number column, `NULL` is last either way, and the card says the sort is of the rows shown. Pressing a cell opens an inspector beside the grid (under it below `xl`) with its row, column, whole value and a Copy button (offered for a value only, not for `NULL` or an empty text); pressing a header's name, or opening a cell, shows the same opened column as the Schema tab from the table's profile, which is requested the first time the inspector opens and not when the tab opens. The row count and "Open in Query Studio" sit in the card's body so the title keeps its width on a phone, and if the larger sample fails the rows the page already has stay, with a line above them and Retry. The grid is one tab stop with the arrow keys, Home and End moving the cell and Escape closing the inspector. API: a `NULL` cell of `GET /api/catalog/{id}/sample` and of the detail body's `sample` is now JSON `null` where it was `""`, so no value and an empty text can be told apart; every other cell is the string it was, and masking is unchanged. The console's Sample tab is the only reader. Not included: number formatting, filtering, resizing or hiding columns, download, anything past 100 rows.
- A page for each connector, `/connectors/<id>` (FC-35; plan `docs/superpowers/plans/2026-10-05-connector-detail-page.md`): on Sources, pressing a connector's name, "View details" or anywhere on its row opens the page the side sheet used to be, so it can be bookmarked, reloaded and sent to someone. It carries what the sheet showed (health, direction and environment; Test connection, Edit, Create pipeline, Audit, Delete; the Overview, Ingest and Connection tests tabs, the open one in `?tab=`) under a header with a "Sources" link back; deleting the connector returns to Sources, and an unknown or other tenant's connector shows the API's message with a way back. The edit page's Cancel and "Back to connector", and a connector node in an asset's lineage map, now go to this page instead of the list or the edit form; a created connector gains "Open connector". Console only; no API change.
- Upload a file into a raw table (`DATA-9`, ADR 0014): on Sources, "Upload file" and an "Uploaded files" tab; the flow is `/connectors/upload` (File, Check, Table, Review, then the result of the load), also reachable from the first step of "New Connector". A delimited text file (UTF-8 or UTF-16; comma, semicolon, tab or pipe) of up to 50 MB and 2,000,000 data rows is stored under `uploads/` in the warehouse bucket, previewed with the encoding, delimiter and header row the API detected (each can be changed and the preview reads again), and loaded as a raw Iceberg table by the new `file_ingest_job`, one Dagster run per file. Every column is text. A load may target a table that does not exist or one an earlier upload of the same tenant created (replace its rows, the default, or add to them); never a connector's table, and `PUT /api/connectors/{id}/ingest-spec` refuses a table name reserved for uploads (409). A file over either limit is refused with the reason, never cut short. Six routes, all `connector:manage`, tenant-scoped (another tenant's upload answers 404), audited: `GET`/`POST /api/uploads`, `GET`/`DELETE /api/uploads/{id}`, `GET /api/uploads/{id}/preview`, `POST /api/uploads/{id}/ingest`. The load's outcome is the one row the job records in `lake.bronze_meta.ingest_run`; a failure shows one of seven fixed reasons, never exception text. Migrations `0057` and `0058` (the upload table and the per-tenant claim on a table name). Not included: workbooks, JSON, Parquet, files over 50 MB, type inference, an assistant tool. See `docs/core/features/upload-file.md`.
- `AI_DEFAULT_REPLY_LANGUAGE` sets the language the copilot answers in when
  a chat message is too short to detect its language, such as a one-word
  follow-up. Accepted values are `id` (Indonesian) and `en` (English); unset
  or empty keeps the previous behavior. A language detected in the message
  always wins. Any other value stops the API at start with a config error.
- Login throttling and session cleanup (backlog `SEC-2`/`SEC-5`).
  `POST /api/auth/login` now throttles failed password attempts per email
  (SHA-256 of the trimmed lower-cased address, stored in a new
  `login_throttle` table, migration `0055`): after `LOGIN_MAX_FAILURES`
  (default 5) failures within `LOGIN_FAILURE_WINDOW_SECS` (default
  900 s), the email is locked out for `LOGIN_LOCKOUT_SECS` (default
  300 s) and every attempt gets an identical `429` with a `Retry-After`
  header — same response whether or not the account exists. A successful
  login clears the counter, and a failure after an expired lock starts
  the count fresh. There is no off switch: `LOGIN_MAX_FAILURES=0`,
  negative, and invalid values all fall back to the defaults instead of
  disabling throttling or failing boot. The
  console's login page surfaces the lockout with the wait time. A new
  hourly background job purges expired/revoked sessions, revoked
  credentials, and unlocked throttle rows whose failure window has
  passed (throttle rows are not gated by `AUTH_RETENTION_DAYS`; sessions
  and credentials are, default 30 days), logging purge counts; it is
  best-effort and skips a tick if the previous run has not finished. This
  replaces the previously-documented "no login rate limiting beyond
  logging" and "no session cleanup job" limitations (README, SECURITY.md).

- Gold publish as a per-mart option (backlog `DATA-1`). Publishing is off by
  default for every mart. A Platform Admin switches it on from the mart's
  asset detail page in Data; once on, the mart is published automatically
  after every authored pipeline run and by the nightly 04:00 schedule.
  The console shows whether each mart is up to date, plus last published
  time, snapshot ID, and the last five publish runs. The Gold Exports page
  (linked from the card) shows an Enabled column across all marts. Under
  the hood: a new `run_status_sensor` triggers `gold_export_job` on
  authored-pipeline SUCCESS, the schedule authenticates via a
  `gold-export-scheduler` service identity bootstrapped from
  `GOLD_EXPORT_RUN_TOKEN`, `POST /api/gold/export/{mart}?ifChanged=true`
  skips unchanged marts (strictly before, equal timestamps export),
  and `GOLD_EXPORT_MARTS` defaults to empty — the console now owns the
  list through `GET /api/gold/publications`. See
  `docs/OPERATIONS.md` for append-only growth and background-merges note.

- Run-config schema and validated trigger (plan R4 2c): `GET /api/pipelines/{id}/config-schema` returns the job's default config YAML (`pipeline:read`), `defaultConfig: null` alongside `defaultConfigYaml` since the workspace has no YAML parser dep and Dagster emits the default as a string; `pl-…` ids return `hasConfig: false` without contacting Dagster. `POST /api/pipelines/{id}/trigger` now accepts an optional `{"runConfig": <object>}` body — when supplied, the route validates against the job's schema (`isPipelineConfigValid`) and, on `RunConfigValidationInvalid`, returns a structured 400 `{ errors: [{ path: string[], reason: <EvaluationErrorReason> }] }` built only from `path` and `reason` (Dagster's free-form `message` is never forwarded, AGENTS.md principle 4); on success the config goes through `launchRun(runConfigData:)`. The audit row for `pipeline.trigger` now carries `args.configKeys` — the TOP-LEVEL keys of the supplied config, never the values. The copilot's `trigger_pipeline` tool gained the same optional `runConfig`.
- Definition version history for authored pipelines (plan R4 2b): a new `pipeline_definition_version` row per `create`/`update`/`delete` (migration `0053`), written inside the same transaction so a failed write leaves no orphan row. The list endpoint (`GET /api/pipelines/{id}/versions`) returns metadata only, newest-first, so the list scales to many versions without shipping every prior snapshot; the get endpoint (`GET /api/pipelines/{id}/versions/{version}`) returns the editable state captured at that version; restore (`POST /api/pipelines/{id}/versions/{version}/restore`) replays a snapshot back into the live row through the existing update path, leaving `name` and `status` alone. Reads are `pipeline:read`; restore is `pipeline:write`. 404 for non-`pl-` ids and for unknown `(id, version)` pairs.
- Pipeline duration SLA: a new `pipeline_sla` row per pipeline (`maxDurationSeconds`, `lateAfterSeconds`) edited through `GET`/`PUT /api/pipelines/{id}/sla` (plan 1f, migration `0050`). When the SLA exists, the pipeline row's payload gains `slaOk` (true while the latest run's `durationSeconds` is at or under `maxDurationSeconds`, `null` while a run is in flight or no SLA is set) and each run gains `overDuration` (true when the run finished and was over `maxDurationSeconds`).
- Late pipeline alerts: `late` on the pipeline row is `true` when no run has succeeded in `lateAfterSeconds` and a threshold is configured, and a `pipeline_late` kind of `alert_rule` fires for matching pipelines via the `/api/alerts/run` op. With no threshold, both stay `null`/`unsupported` rather than zero-firing.
- Volume drop alerts: each `authored__<id>` job reports its row count to the API through Dagster's new `run_status_sensor(SUCCESS)` (mirrors the existing `run_failure_sensor`), and a `pipeline_volume_drop` kind of `alert_rule` fires when the latest run wrote under half the median of the previous 5+ completed runs. The same hook computes `pipeline_slow` (using `overDuration`). With fewer than 5 prior samples, the rule is recorded as `skipped` rather than firing or staying silent.
- Dagster `pipeline_run_finished_sensor` (plan 1f) — mirror of the existing failure sensor, posting one `POST /api/pipelines/events/run-finished` per `SUCCESS` run in this code location. Same posture: degraded-honest when `PIPELINE_RUN_TOKEN` is unset, `default_status=RUNNING`, retries at the sensor level, deduped on the API side by `(run_id, kind)`.
- Pipeline-failure alerts: a new `pipeline_failure` kind of `alert_rule`, scoped to one pipeline or `*`, that fires when a Dagster run ends in `FAILURE`. Dagster's `run_failure_sensor` (in `dagster/dispar_orchestrate/pipeline_events.py`) posts each failure to a new service-only `POST /api/pipelines/events/run-failed`; the API dedupes by `(run_id, kind)` (new `pipeline_run_event` table, migration `0049`) so a sensor retry never double-alerts, and the alert body names the pipeline, the run id, and a relative `/pipelines/<id>?run=<runId>` link — never the orchestrator's error text.
- Console-authored pipelines now run: `authored_factory.py` builds one `authored__<id>` job, and a cron schedule when the pipeline has one, per `ready` or `paused` pipeline from the new service-only `GET /api/pipelines/runnable`. The API asks the orchestrator to reload after every change, which is why the code location now runs `dagster code-server start`.
- Edit (`PUT /api/pipelines/{id}`, `/pipelines/{id}/edit`) and delete (`DELETE /api/pipelines/{id}`) for authored pipelines, with audit events.
- `GET /api/pipelines/{id}/schedule-ticks` and a Schedule history on the pipeline page: each time a schedule was due and whether it launched, skipped or failed. The orchestrator's error text is not sent.
- Authored pipelines now declare upstream wiring (`dependsOn` / `depends_on`, max 10): migration `0052_pipeline_depends_on.sql` adds the column, `POST`/`PUT /api/pipelines` validate ids and cycles against the existing authored graph + the live Dagster job list, and `authored_factory.py` builds one `run_status_sensor(SUCCESS)` per downstream — `authored__<safe_id>_after`, default `RUNNING`, run_key = upstream run id, ALL semantics ("every upstream has a SUCCESS run that finished after the downstream's most-recent start"). Dagster unreachable on a write degrades to "no Dagster upstreams accepted" rather than refusing the write. The detail payload gains `upstream` (with each upstream's `lastSuccessAt`) and `downstream`. The schedule-ticks endpoint now also merges in the dependency sensor's ticks, tagged `kind:"sensor"` so the UI can render the upstream-staleness skip reason alongside the schedule's outcomes.
- Rows per run: gold export and ingest report the rows they wrote, shown as "Rows written" in the run inspector.
- `scripts/compose.sh`: `docker compose` with `GIT_SHA` set from the checkout, which the pipeline page's source view needs.
- Dashboard SQL sources: a saved read-only `SELECT` (e.g. a join across
  several `serving` marts) that a chart can use instead of one mart.
  Authored from Query Studio ("Save as SQL source") with the new
  `dashboard:sql` permission (only `*:*` holds it today); restricted to
  one `SELECT`/`WITH` over `serving.*` tables, run through the policy
  rewrite, capped at 2,000 rows and 30 s. Charts on a source follow its
  current SQL, dashboard filters apply, and the Copilot can list sources
  and build charts on them (`list_sql_sources`, `sqlSource`).
- Dashboard folders (stage 1): nested up to four levels, holding
  dashboards and SQL sources; only empty folders can be deleted. Managed
  from the dashboard title menu; "Move to folder…" in the ⋯ menu.
- Page-aware Copilot: the dashboard's tiles with their first rows and the
  active filters, or Query Studio's SQL and last result, are sent as page
  context (bounded; the API cap is now 6,000 characters).
- Chart builder: chart types in a sidebar next to the form and live
  preview, replacing the separate gallery step.
- Four chart kinds: sankey and sunburst (flow / two-level hierarchy over
  a dimension and a required breakdown), box plot (min, quartiles, max per
  category via `quantilesExact`), and calendar heatmap (daily values, up
  to the last year of data). Available in the builder, to the Copilot, and
  on SQL sources.
- Maps: the map chart is no longer Jakarta only. A `map` id (bundled maps:
  `dki-jakarta`, `id-provinces`, `id-regencies`; the console owns the list,
  the API checks only the id's shape) picks the outline; region names are
  matched case-insensitively with `Kabupaten`/`Kab.`/`Kota Administrasi`
  and province aliases handled, and rows that match no region are counted
  under the map instead of dropped silently. Two new kinds draw rows with a
  latitude and a longitude column (`lat`, `lon`) on an outline: `pointmap`
  (symbols sized and coloured by the first measure) and `geoheat` (density
  heatmap), each capped at the top 5,000 rows by value (2,000 on a SQL
  source). All three maps pan by dragging and zoom 1x-20x with buttons on
  the tile; there is no wheel zoom. Stored specs need no
  migration: `map`/`lat`/`lon` are optional and absent on every chart saved
  before. Boundary files and their licence: `public/geo/README.md`.

- Gold Exports console page: per-mart last export (`snapshotId`/
  `exportedAt`, read straight off the Iceberg table's own snapshot), an
  "Export now" action, export history from a new `console.gold_export_run`
  table, and a `GET /api/gold/export/{mart}/consumers` route that reports
  an honest `supported: false` until Trino query-history correlation is
  written (the `lakehouse-trino` client crate exists but this route does
  not yet call it) (WS6).
- `POST`/`GET /api/gold/export/{mart}`'s `check_export_token` now also
  accepts a session holding the `gold:export` permission, in addition to
  the pre-existing shared-token/service-identity paths — a deployment
  that sets `GOLD_EXPORT_RUN_TOKEN` for the Dagster schedule no longer
  locks the console's "Export now" button out for every human operator
  (WS6).
- The scheduled `gold_export_schedule` (daily 04:00) is restored. Known
  gap: unlike `agent_run_schedule`/`alerts_run_schedule`, no service
  identity is provisioned for it yet, so its nightly run currently gets
  `401` at `Policy::RequiresAuth` before `check_export_token` ever runs —
  shipped anyway per AGENTS.md rule 2 (a schedule that visibly fails is
  more honest than one withheld to hide the gap) (WS6).
- Per-tenant built-in "Main" dashboard tile catalog loaded from a JSON
  file via `BUILTIN_DASHBOARD_SPEC`, replacing the removed
  `BUILTIN_DASHBOARD_ENABLED` boolean flag (WS6).
- Dashboard list page at `/dashboards/browse`, as a gallery of cards or a
  table with search, filters and sorting. Each dashboard shows its tile
  count, owner, last-updated time and whether it is shared publicly or
  embeddable; rows offer rename, duplicate and delete. Reachable from the
  board switcher ("Browse all dashboards"), the command palette, and the
  navbar. Dashboards can now carry a one-line description, set when they
  are created or renamed and shown in the list.
- `GET /api/dashboard/boards` now returns `chartCount`, `builtin`,
  `description`, `createdBy` and `updatedAt` per board. `chartCount` is
  derived from `console.bi_chart` on each request rather than stored, so it
  cannot go stale when a chart moves between boards. `updatedAt` exposes the
  same column as the existing `createdAt` under an honest name: the table is
  a `ReplacingMergeTree(created_at)`, so that column is the version column
  and every save rewrites it.
- `POST /api/dashboard/boards` accepts `description`, and records the
  signed-in caller's display name in `created_by`.
  `PUT /api/dashboard/boards` accepts `description`.
- Pipeline recovery API (R2): `POST /api/pipelines/runs/{runId}/retry`
  now accepts `{"strategy":"selected","stepKeys":[...]}` to re-run a chosen
  subset of steps, with the step keys validated against the parent run's
  step history and the first unknown key named in the 400 response; the
  Copilot `retry_pipeline_run` tool gained a matching `stepKeys` array
  argument. A `selected` request against a run that `Dagster` does not
  know returns 404 (the run is the missing resource). The per-run steps
  view (`GET /api/pipelines/{id}/runs/{runId}/steps`) reports the
  step's attempt count (`attempts`); the runs × steps matrix
  (`GET /api/pipelines/{id}/runs/steps`) returns one row per recent
  run and per-step `stepKey`, `status`, and `durationMs` (and no
  attempt count — `attempts` is a per-step, per-run field, not a
  matrix field). On a `Dagster` transport failure the matrix route
  reports `unavailable: "<reason>"`, never a 5xx, and never an
  `available` field. The matrix-vs-`run_steps` resolution is
  handled by axum's `matchit` router (static segment ahead of
  parameter), not by the registration order in
  `routes::pipelines_router`. (R2)
- `max_retries` per authored pipeline: `POST /api/pipelines` and
  `PUT /api/pipelines/{id}` accept `maxRetries` (0–5, rejected with a 400
  outside the band; defaults to 2) and persist it in `lakehouse-store` via
  the new migration `0051_pipeline_max_retries.sql`. The Dagster
  `authored__<id>` job reads `definition["maxRetries"]` and overrides only
  the attempt count on the shared `DEFAULT_RETRY_POLICY`; delay, backoff
  and jitter come from the shared policy. (R2)

### Changed

- `gold_export_job` is rebuilt as `list_gold_marts` (fan-out, one `DynamicOutput` per configured mart) -> `export_gold_mart[<key>]` (mapped, one step per Gold mart) -> `summarize_gold_export` (collect), per PART A of `parts/1b-dagster-one-op-per-unit-and-retries.md`. One mart's HTTP error fails only that mapped step (the others still record their `maintenance_run` success rows); the failure row for the failed mart is written before the bare re-raise, so the governance surface sees the failure immediately, not only after the retry policy's attempts have all come up empty. The console's pipeline detail log parsing is unchanged: the log shape (`Execution of step "<key>" failed.`) is unchanged, and the console fixture now references the mapped-step names (`export_gold_mart[<key>]`) instead of the old single `run_gold_export`.
- `ingest_job` is rebuilt as `run_ingest` (fan-out, batch connectors only — `cdc` and `kafka`/stream connectors yield no mapped steps) -> `ingest_source_object[<key>]` (mapped per `sourceObjects` entry) -> collect, per PART B of `parts/1b-dagster-one-op-per-unit-and-retries.md`. The mapped step's input carries the connector with its `secretRef` STRING REFERENCES only — the secrets are resolved INSIDE the mapped op, never on the `DynamicOutput` payload, because Dagster's IO manager persists op outputs and a resolved secret on one would leak. `SecretRefRejected`, `UnknownAdapter`, `ssrf_guard.SsrfBlocked`, and `UnsupportedColumnType` are wrapped as `Failure(allow_retries=False)` at the mapped op's boundary so a config-shaped failure is never retried.
- `bronze_maintenance_job` is rebuilt as `list_bronze_tables` (fan-out, runs catalog-database creation and the policy-list fetch once per run, not once per table) -> `maintain_bronze_table[<key>]` (mapped, per-table body) -> `summarize_bronze_maintenance` (collect), per PART C of `parts/1b-dagster-one-op-per-unit-and-retries.md`. A failed table's mapped step fails without taking the others down, and a fresh catalog with zero tables yields a still-successful run (`summarize_bronze_maintenance` collects an empty list). Mapping-key collisions (two tables whose names sanitize to the same Dagster step key) raise `Failure(allow_retries=False)` naming both.
- A shared `DEFAULT_RETRY_POLICY` (max_retries=2, delay=30s, exponential backoff with ±jitter) is wired on every op in the Dagster code location, and every `Failure` raised on a config, auth, SSRF, secret-ref, or column-type problem sets `allow_retries=False`, per PART D of `parts/1b-dagster-one-op-per-unit-and-retries.md`. Gold export HTTP errors are NOT wrapped in `Failure(allow_retries=False)` — they are bare re-raised so the default policy retries a transient 5xx (busy ClickHouse on the `lakehouse-api` side, the canonical case the plan names), and a persistent 4xx/5xx fails after the policy's two attempts. The sanitize helpers (`gold_export._sanitize_mapping_key`, `ingest_factory._sanitize_target`, `maintenance._sanitize_mapping_key`) now use the exact ASCII `[A-Za-z0-9_]` rule via a precompiled `re`, replacing the previous `str.isalnum()`-based rule that would silently accept non-ASCII letters as Dagster mapping keys.
- `/dashboards` no longer renders a page. It resolves: to the dashboard you
  last had open, or — when you have not created one yet — to the built-in
  "Main" board, and only otherwise to the list at `/dashboards/browse`.
  Returning to Dashboards from another section therefore reopens the
  dashboard you were working on instead of making you pick it again. The
  single-dashboard canvas moved to `/dashboards/[id]`. The last-opened
  dashboard is remembered per browser and validated against the server on
  every resolve, so a deleted board never leaves you on an empty canvas.
- **Relicensed the project from Apache-2.0 to AGPL-3.0-or-later.**
  `v0.1.0` was released and remains distributed under Apache-2.0 — that
  historical release is unaffected. All source as of this change is
  licensed AGPL-3.0-or-later: `LICENSE` now carries the full AGPLv3 text
  (copyright RantAI), `rust/Cargo.toml`'s `[workspace.package] license`
  (inherited by all 12 `lakehouse-*` crates) is `AGPL-3.0-or-later`,
  `package.json` now declares `"license": "AGPL-3.0-or-later"`,
  `rust/deny.toml` carries per-crate exceptions so `cargo deny check
  licenses` accepts the first-party AGPL declaration without loosening the
  permissive-only allowlist for third-party dependencies, and `NOTICE` /
  `README.md` reflect the new license. A dependency re-audit
  (`cargo deny check licenses`) found no dependency license incompatible
  with distributing the combined work under AGPL-3.0.
- **Console honesty pass.** Every value the API could not actually measure
  now reports as not measured instead of a plausible-looking number: overview
  KPI tiles, service health, catalog asset size/freshness/usage, pipeline run
  counters and cost, digital-employee metrics, query transparency fields
  (cache hit, pushdowns, query plan), operations health/version/latency, and
  workload start times are all `null`/"unknown" until something real
  computes them, instead of hardcoded zeros or constants.
- Audit-trail deep links (query runs, approvals, connectors) now carry a real
  event id only when a real audit event exists, instead of a synthesized id
  that named no event and 404ed on click.
- Governance lineage is reported as unsupported rather than rendering a
  fixed source→bronze→silver template for every dataset; data-quality rule
  results report as not evaluated rather than defaulting to "warning".
- The connector creation wizard now runs the real connection test against
  the newly created connector and shows the actual outcome, instead of a
  local "Test passed" step that contacted nothing; connector health reports
  unknown, with no last-test time, until a probe actually runs.
- Identity records no longer serve activity timestamps that were never
  written; rotation status now matches what authentication itself enforces
  (expired once the credential's expiry has passed).
- The app shell now shows every real page unconditionally instead of hiding
  them behind a "preview" flag, and drops the always-on notification and
  presence indicators that nothing measured.
- User-facing messages, error text, and code comments in the console and API
  were translated from Indonesian to English; copilot prompt text sent to
  the model is intentionally left unchanged.
- The alerts table now distinguishes an API failure from having no alerts,
  instead of showing an empty state for both.

### Fixed

- Upload file: a file over about 10 MB could not be uploaded from the console, CSV included, and the browser showed a bare "Internal Server Error". The console proxies `/api/*` to the API with a Next.js rewrite, and Next.js 16 cuts a proxied request body at `experimental.proxyClientMaxBodySize` (default 10 MB; the dev server logs "Request body exceeded 10MB"), so the API received a broken multipart form. `next.config.ts` now sets the limit to 51 MiB, the API's `MAX_REQUEST_BODY_BYTES` (50 MiB of file plus 1 MiB of form framing). The page also refuses, before sending anything, a file whose name says it is an Excel workbook, archive, Parquet file or other binary (`.xls`, `.xlsx`, `.zip`, `.parquet`, ...) with the API's reason and "save the sheet as CSV first"; unknown or missing extensions are still the API's to judge by content. The first step states the accepted kinds and the 50 MB limit, the file picker suggests CSV, TSV and text, and an answer that is not the API's JSON error (a proxy's plain 500 or 413) reads "The upload did not reach the service" and names the limit. Verified through a dev server against a stand-in upstream that counts bytes: 24 MB and 49 MB bodies arrived whole (500 before). Not verified: a signed-in 49 MB load against the real API, and the standalone Docker image (no `next build` was run).

- Cycle detection now fires on the row being edited (PR #57 review F1.7): the cycle walk's closing-edge check moved ahead of the "not an authored pipeline" skip, so a 2-cycle like `A → B; PUT B with dependsOn:[A]` (or any cycle that closes through the edited row) is refused with a 400 that names the edited pipeline — pre-fix the edited row was excluded from `others` by `collect_authored_depends_on(..., Some(&id))`, the skip treated it as unknown, and the cycle silently landed. The walk is also bounded by a visited set so a fan-in (diamond) graph doesn't blow up exponentially.
- The create route mints the pipeline's id before running the `dependsOn` validator (PR #57 review F1.7): the validator's self-reference rule now compares against the real id (`pl-<slug>-<base36 millis>`), not `body.name`, which only equals the slug for snake_case names.
- Editing an authored pipeline no longer wipes its `dependsOn` chain (PR #57 review F1.8): the field is `Option<Vec<String>>` joined by `COALESCE($N, depends_on)` in the write, so `None` keeps the stored chain (a console save that did not touch the field), `Some(vec)` replaces it, and an explicit `Some(vec![])` clears it.
- `dependsOn` refuses entries in the orchestrator's reserved namespace (PR #57 review F1.9): a list starting with `authored__` is rejected with a 400 that names the offending entry, and duplicate entries in the same submission are rejected with a 400 that names the duplicated value.
- `POST /api/pipelines/runs/{runId}/retry` with `selected` step keys returns 404 when the run does not exist (PR #57 review F1.10), not 400 — the precondition `pipeline_run_status` lookup distinguishes "this run exists but has no steps yet" (a 400 with the first unknown key) from "this run does not exist" (a 404).
- The false "the matrix route is registered before `/runs/{runId}/steps`, order matters" comments in `routes/mod.rs` and `policy.rs` are removed (PR #57 review F1.10): axum's `matchit` router resolves static segments ahead of path parameters regardless of registration order, and the routing is pinned by an integration test in `tests/pipeline_routing.rs` that drives the real `routes::router` for both paths and asserts each reaches the correct handler.
- Wiremock mocks in the runs × steps matrix tests (lakehouse-dagster, `list_runs_with_steps_for_job` at limit 30) and in the `selected_retry_without_step_keys_returns_400_before_calling_dagster` mock now carry `.expect(1)` / `.expect(0)` (PR #57 review F1.10), so a regression that adds a second round trip (or leaks the parser) surfaces as a mock-mismatch rather than a silent "well, the response looked right".

- A pipeline with `dependsOn` no longer takes down the whole Dagster code location (PR #57 review F1.1): the dependency sensor re-fetched the runnable list and rebuilt the job, and Dagster refuses two job definitions with the same name at load time. One fetch now feeds both the jobs and the sensors.
- Pausing a chained pipeline now also stops its dependency sensor, and resuming starts it again (PR #57 review F1.2): Dagster keeps a sensor's stored RUNNING state across code-location reloads, so removing the sensor alone left the chain firing; a paused pipeline also builds no sensor at all.
- The dependency chain now holds "every upstream succeeded" (PR #57 review F1.3): the downstream launches only once every upstream has a SUCCESS run newer than the downstream's latest run, a queued or in-progress downstream never double-fires, and one round of upstream successes requests one downstream run instead of one per upstream tick.
- An authored pipeline's write now replaces its target table atomically via a staging table and `EXCHANGE TABLES` (PR #56 review F1.4): the write was a plain `INSERT INTO … SELECT` into a `MergeTree ORDER BY tuple()` table, so a retry after the insert committed — or an ordinary second run — appended the whole result again. A run now always leaves exactly its SELECT's rows; append-per-run behaviour is gone.
- A pipeline created in the console now belongs to its creator's tenant and appears on the Pipelines list; it used to be stored without a tenant and was invisible to every list. A Platform Admin (`*:*`) with no tenant now sees every tenant's authored pipelines there, the rule that already showed them the Dagster jobs; before, they saw none. An authored pipeline's `authored__<id>` Dagster job is not listed a second time.
- `PIPELINE_RUN_TOKEN` is now passed to `lakehouse-api` and `dagster-code-location`; it was passed to neither, so no authored pipeline could become a job.
- The authored-pipeline schedule field offers cron presets and a validated custom cron; its old free-text default ("Every hour") never fired.
- The Pipelines list shows "Never run" instead of an empty badge for a job that has never run, and schedules in words.
- Dashboard drill-down, filter values (`/api/dashboard/values`), alert
  values and digests now go through the policy rewrite like dashboard
  tiles, so masking and row filters apply there too (alerts and digests as
  the least-privileged "Dashboard Viewer" role, like embeds).
- LLM failures no longer show the provider's raw response text in the
  Copilot or the text-to-SQL agent.
- `enforce` passes no permissions to the statement classifier (it passed
  role names); sensitive `system.*` tables stay unreadable from governed
  SQL surfaces, now stated explicitly.
- A new dashboard no longer shows the previous board's charts (a stale
  load response won the race); closed selects in the chart builder show
  labels instead of raw ids; Copilot chart drafts on a SQL source preview
  correctly.
- Tests: `lakehouse-test-support` reuses one labelled Postgres container
  instead of leaking one per test binary; the connector secret allowlist
  test no longer depends on a developer `.env` (loaded by `sqlx::test`).
- `debezium-server` re-pinned to `:1.1.1.Final@sha256:2ad14b1…` so the
  `Rust · G4 Debezium CDC into Bronze` job pulls again — the prior
  digest-only pin (`…5281e2bd…`, what `:latest` resolved to on 2026-09-24)
  now 404s (`manifest unknown`, CI run 36680491360), and a bare digest
  pin dies silently whenever upstream overwrites the untagged image.
- The Dagster dependency sensor (`authored__<id>_after`) is now actually built. The Rust `RunnablePipeline` serializes `depends_on` as `dependsOn` (`#[serde(rename_all = "camelCase")]` in `lakehouse-store/src/pipelines.rs`), but the factory read `pipeline.get("depends_on")` which always returned `None`, so every chain was silently dead. The factory now reads `dependsOn`, matching the wire format, and the test fixture mirrors the live JSON. The existing pre-fix tests for `schedule`/`status`/`definition`/`sourceZone`/... already proved the rest of the factory's reads were on the camelCase wire.
- `DELETE /api/pipelines/{id}` refuses to delete an authored pipeline that is still listed as an upstream by another authored pipeline's `depends_on`, naming every downstream id verbatim in a 400. Without this, deleting the upstream left a dangling reference the validator refuses the next time the downstream is edited, AND a sensor that watched a job that no longer existed. Pure helper `referencing_downstreams(pairs, target_id)` extracted from the route so the reverse walk is unit-tested directly (the route handler cannot be tested at this seam without a real pool).

### Removed

- The unregistered transformation modules that came with the first sketch of file upload (`silver_transform.py`, `gold_transform.py`, `sap_models.py`, `ch_models.py`): a model written for one customer's export does not belong in the product. They stay in history at `f9793cd`. The three helpers of `ch_models.py` that other code still used moved into `connector_catalog.py`.
- Pages and dialogs with no backend behind them: storage tiering (including
  the "Restore to Hot" dialog, which hardcoded the asset it restored),
  knowledge sources/vector jobs/semantic search, agent workflows/tools
  registry/agentic builder, governance residency/workspace settings, the
  policy-impact preview, and the query-collaboration surface. Their API
  routes remain registered for later workstreams that will rebuild them on
  real data.
- The pipeline detail view no longer synthesizes an op graph, description,
  or config summary on the client; it shows the pipeline's real run history
  with an honestly empty graph tab.
- The query-transparency estimate panel, which rendered plan stages as
  "completed" before they had run.
- Seeded pipeline and alert rows that were indistinguishable from real
  activity are pruned by migration.

### Fixed

- Sorting a table column now actually reorders the rows. `useDataTable`
  built its column whitelist from `column.id`, but TanStack derives that id
  from `accessorKey` inside the table rather than on the definition object,
  so almost every column was missing from the whitelist and the URL parser
  discarded the sort it had just written. Restoring a filter from table
  memory failed the same way. Affects every table page, not only the
  dashboard list.

## [0.1.0] - 2026-08-30

First tagged release. Everything below reflects the commit history on
`main..feat/rust-backend` (84 commits, merged via #1) — a full backend port
from the original TypeScript/Next.js API routes to a Rust/axum service, plus
the CI/security work done to prepare the repository for its first release.

### Added

- Rust workspace scaffold (`rust/`, 11 `lakehouse-*` crates) alongside the
  existing Next.js frontend, as the target of a full backend port from
  TypeScript to Rust/axum.
- `lakehouse-core`: shared error type (`ApiError`), SQL-injection-safe
  identifier newtypes (`Ident`, `SqlLiteral`), and status enums.
- `lakehouse-clickhouse`: HTTP client for ClickHouse's plain HTTP interface,
  porting `src/services/clients/clickhouse.ts`.
- axum HTTP chassis for `lakehouse-api`: config resolution, error bridge,
  shared state, health check, per-route request timeouts matching each
  original route's `maxDuration`.
- Route ports to axum: catalog, storage, overview, ops, governance
  (including lineage), query (`run`/`estimate`), pipelines (list, runs,
  trigger), dashboard (all 8 sub-routes), embed (`/api/embed/data`,
  `/api/public/dashboard/{token}`), agent (`ask`/`query`/`text-to-sql`), AI
  chat/sessions/build-status, alerts (`/api/alerts`, `/api/alerts/run`).
- New crates added during the port: `lakehouse-dagster` (Dagster GraphQL
  client), `lakehouse-embed` (signed embedding, HS256 JWT), `lakehouse-llm`
  (OpenAI-compatible chat completions client), `lakehouse-notify` (webhook +
  SMTP delivery), `lakehouse-bi` (dashboard specs + SQL builders +
  ClickHouse-backed board/chart store), `lakehouse-alerts` (threshold
  alerts + scheduled digests).
- `lakehouse-store`: Postgres-backed OLTP foundation for Phase 2
  (`console`-schema mutation state that ClickHouse is a poor fit for), with
  lazy, non-fatal connection handling.
- Phase 2 domains backed by real Postgres storage and routes: identity
  (`/api/identity/*`), governance policies/authored rules, saved
  queries/history/collaboration, pipelines (authored definitions + real
  Dagster mutations), storage/ops/overview (real `KILL QUERY`, alert
  instances), connectors, knowledge (sources + vector jobs), and digital
  employees (agents, tools, workflows, runs, approvals).
- `lakehouse-auth`: the authentication core — `Principal`, the
  `Authenticator` trait/seam, and local-password, session, and
  service-token authenticators, all reading/writing a single
  `auth_identity` table designed to hold any future identity provider as
  rows, not schema changes.
- Authentication wired into the axum router (`crate::auth`,
  `crate::policy`'s deny-by-default `POLICY_TABLE`), plus a route-policy
  completeness test that hard-fails on any route missing a policy entry.
- OIDC identity-provider support (Task 3.5): `OidcAuthenticator` as a
  resource server verifying bearer `id_token`s against a configured
  provider's JWKS, with JIT provisioning and configurable IdP-group-to-
  local-role mapping (union, not IdP-authoritative — see
  `rust/crates/lakehouse-auth/README.md`).
- Frontend: login flow, session-aware app shell, and centralized 401
  handling routed through `apiFetch`.
- Full cutover: TypeScript Next.js API routes deleted; the Rust
  `lakehouse-api` service is now the sole backend, reached via the
  `next.config.ts` `/api/*` rewrite.
- Corpus parity harness + TS/Rust spec drift guard, used throughout the
  port to verify each new Rust route's responses against golden output
  captured from the original TypeScript backend.
- Dashboards: click-to-cross-filter and drill-down records (Metabase-style),
  PDF export via print (no added dependency), a self-contained Jakarta
  choropleth geomap (no external tile dependency).
- Threshold alerts and scheduled digests (webhook + email delivery).
- Self-contained `docker compose` backend stack (Postgres, ClickHouse,
  `lakehouse-api`) with migrations run at container boot via the entrypoint,
  plus operations docs.
- `lakehouse-test-support` crate; Postgres integration tests de-ignored;
  named regression tests for four specific security properties; HTTP-level
  authorization contract tests exercising the real router end to end.
- CI restructured into fast-feedback and heavy workflows: `cargo audit`,
  `cargo deny check`, `gitleaks` (working tree and full-history scan),
  `cargo llvm-cov` coverage + CycloneDX SBOM generation, and a Docker
  build/smoke-test job — see `docs/CI.md`.
- Release-prep hygiene for open-sourcing: license, docs, and CI templates.

### Changed

- BI chart aggregate typed as an enum, closing a raw-string SQL path.
- `ChartInput` made lenient so text/kpi charts are reachable via AI chat.
- `ensure_bi_table`'s DDL bootstrap cached once per process instead of
  re-issued per request.
- Six ad-hoc SQL escapers replaced with `SqlLiteral` instead of
  quote-stripping.
- XML tool-call argument parsing made order-preserving; `buildRunId`
  omission and `<think>` tag stripping made case-insensitive in the AI
  chat path.
- Governance `GET /api/governance/{kind}` now unions authored rules into
  the response.
- Infra endpoint URLs and credentials made env-only — no internal defaults
  baked into source.
- Remaining admin routes gated on the new permissions model; ad-hoc
  `/api/*` fetch calls in the frontend routed through the central
  `apiFetch` (and its 401 handling) instead of calling `fetch` directly.

### Fixed

- Every JSON response now emits `application/json;charset=utf-8`
  consistently.
- ClickHouse and Dagster clients no longer leak internal endpoint URLs on
  transport failure.
- Invalid `SMTP_PORT` degrades gracefully (falls back to `587`) instead of
  failing boot; `SMTP_SECURE`'s effective value now folds in the
  `port === 465` rule from the original TypeScript, not just the raw env
  var.
- Request-timeout responses render as a proper JSON error envelope instead
  of a bare timeout.
- Stale `#[allow(dead_code)]` on `ApiRejection`/`ErrorBody` removed once no
  longer needed.
- BI: stopped dropping every live stored chart on the new envelope shape.
- Stale `pub mod tenant;` dropped from `lib.rs` (the tenant module is not
  wired into any route — see the Security section of the release notes for
  what this does and does not mean for tenant isolation).
- `docker compose`: `lakehouse-api`'s runtime base image matched to the
  builder's OS; built from a pinned Rust 1.96.1 `Dockerfile`.
- Nested `if let` chains collapsed for clippy on current stable.
- `must_change_password` now enforced server-side, not just as a UI hint.
- `gitleaks`'s full-history job fixed to actually detect the key it was
  missing, then to stop flagging its own scanner config and docs as new
  matches.

### Security

- **Unauthenticated API surface** — before the `lakehouse-auth` core was
  wired into the router, every route (including writes to Postgres-backed
  storage) was open. Fixed by introducing `Principal`/`Authenticator` and
  gating the router on `crate::policy`'s deny-by-default `POLICY_TABLE`.
- **`/api/identity/*` privilege escalation** — identity routes were
  auth-gated but not permission-gated, allowing any authenticated caller to
  reach admin-only identity operations. Fixed by permission-gating those
  routes (D1).
- **Embed signing secret returned over HTTP** — a dashboard-embed response
  was returning the HMAC signing secret used to sign embed tokens. Fixed by
  no longer including it in the response (D2).
- **`ai/chat` executing write tools in read-only mode** — the write-tool
  block was previously enforced only at advertisement time (tools were
  hidden from the model) but not at dispatch time, so a crafted tool call
  could still execute a write. Fixed by enforcing the block at dispatch
  (D3).
- `/api/alerts/run` now fails closed (401) when `ALERTS_RUN_TOKEN` is unset,
  instead of allowing unauthenticated calls (D4).
- `Config`'s `Debug` implementation hand-written (not derived) so secret
  fields (`ch_password`, `llm_key`, `embed_secret`, `alerts_run_token`,
  `smtp_pass`, `database_url`) can never leak into a `{:?}`-formatted log
  line; enforced further by a `check-no-secrets.sh` CI script after a
  secret was leaked once during development.

[Unreleased]: https://github.com/RantAI-dev/RantAI-Lakehouse/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/RantAI-dev/RantAI-Lakehouse/releases/tag/v0.1.0
