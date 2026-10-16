# `DATA-13` Column-level lineage — Implementation Plan

| | |
| --- | --- |
| Item | `DATA-13`, Data roadmap phase 2 (master plan `2026-10-14-data-phase-2.md`) |
| Branch / PR | `feat/data-12-certification-governed-tags`, PR #106; built on the sub-branch `feat/data-13-column-lineage` in worktree `/home/hv/lakehouse-data13`, merged back by the planner |
| Base verified at | `57de99d7` (phase branch, `origin/main` `53d05f00` merged in) |
| Scope | **The spec's Target rows and `DATA-13-AC1`–`AC3`** (`docs/core/specs/data-13.md`); the product owner decided on 2026-10-16 that phase 2 follows the specs |
| Written | 2026-10-16 by the planner (Claude Opus) for a developer agent, under `AGENTS.md` |

## 1. The spec rows and what this plan builds

| Spec row | Target | This plan |
| --- | --- | --- |
| Column mappings | Recorded automatically for every pipeline, Gold build and saved SQL source | Built (tasks 1–5): rows written when a pipeline run succeeds, when a Silver or Gold table is built by SQL outside a pipeline (read from the ClickHouse query log), and when a dashboard SQL source is saved. What the parser cannot map is stored as **partial** with the reason |
| View | Column lineage on the asset's Lineage tab, upstream and downstream | Built (task 7) |
| Impact | From a source column, list every downstream column, chart and dashboard | Built (tasks 6–7). Charts and dashboards are listed only to a caller with `dashboard:read`, as the table-level Dependents card does today |
| Retention | 1 year | Built (task 6): rows older than 365 days are removed; the answer states how far back rows go |

## 2. What exists today (anchors at `57de99d7`)

`A` is `rust/crates/lakehouse-api/src`, `R` is `A/routes`.

| # | Code | Verdict |
| --- | --- | --- |
| F1 | `R/lineage.rs:509`: `json!({ "focus": focus, "columnMappings": [], … })`. The table graph is rebuilt per request from records (`:12-20`); no lineage table exists. `GET /api/governance/lineage` needs `lineage:read` (`A/policy.rs:266`). Contract `columnMappings: { source; target; transform }[]` (`src/services/contracts/governance.ts:83`); rendered only when non-empty (`src/features/catalog/asset-lineage.tsx:286-322`, `src/features/governance/lineage-page.tsx:132-143`). Fixtures pin `[]` (`rust/tests/parity/corpus/gov-lineage*.json`, `asset-lineage-map.test.ts:20`, `asset-detail-tabs.test.tsx`). | Nothing is recorded. |
| F2 | Pipelines: the executor composes the SQL in Python (`dagster/dispar_orchestrate/authored_factory.py` `_build_select_sql`, `:487-532`); rename and cast apply only to columns listed in a `select`, otherwise the SQL is `SELECT *`. Before writing, the op posts the exact SQL to `POST /api/pipelines/runnable/{id}/enforce` (`R/authored_pipelines.rs:1041` `enforce_pipeline_sql`). The run's success arrives at `run_finished_event` (`R/pipelines.rs:2679`), which already refuses anything not `SUCCESS` (`:2726`). | The enforce call is the one place the API sees a run's real SQL; the success event is where it becomes true. |
| F3 | Gold and Silver built outside pipelines: `R/lineage/builds.rs` reads `system.query_log` (`BUILDS_SQL` `:28-40`) for `CREATE … AS SELECT` / `INSERT … SELECT` into `silver`/`serving`, table names only, the query text not selected. | The SQL text is in the log; nothing parses it. |
| F4 | Dashboard SQL sources: `R/dashboard_sources.rs:209` `save()` normalizes the SQL (`check_sql_source`, only `serving.*`), rewrites it, and learns its output columns with `probe_columns` (`:160`). | Output names are known at save. |
| F5 | No helper maps output columns to source columns: `sql_rewrite::referenced_tables` (`A/sql_rewrite.rs:118`) gives tables only; `referenced_identifiers` (`:1905`) one expression only. `sqlparser` is 0.63 (`rust/Cargo.toml`). | A projection-lineage helper is new. |
| F6 | Charts: `console.bi_chart.spec_json` (`lakehouse-bi/src/store.rs:238` `StoredChartSpec`): `ChartInput` names the columns (`dimension`, `measures`, `breakdown`, `lat`, `lon`, `tables.columns`); the relation is a mart or a SQL source (`resolve_relation`, `:2555`). Table-level dependents (`R/catalog_governance.rs:195-278`) list dashboards only for `dashboard:read`. | Charts can be matched to columns at request time. |
| F7 | The gate: `ops/g3a/g3a_test.py:393` `RECORDED_LINEAGE_EDGE_KINDS` lacks `build` (`builds.rs:43`) and `upload` (`R/lineage.rs:291`); the check at `:437-439` fails on any other kind. | Fixed here. |
| F8 | Retention precedent: `lakehouse-auth/src/cleanup.rs` `purge`, run hourly; the 15-minute alerts tick (`R/alerts.rs:907-922`) already starts background passes. | Purge on the tick. |

Migration number: **`0079`** (reserved for `DATA-13`). Check `ls rust/migrations | cut -c1-4 | sort | uniq -d` prints nothing.

## 3. Decisions already made by the planner

- **One table.** `0079_column_lineage.sql`: `column_lineage (id BIGSERIAL,
  producer_kind TEXT CHECK IN ('pipeline','build','sql_source'),
  producer_ref TEXT, tenant_id UUID NULL, from_table TEXT, from_column TEXT
  NULL, to_table TEXT, to_column TEXT, transform TEXT, partial BOOLEAN,
  reason TEXT NULL, recorded_at TIMESTAMPTZ DEFAULT now())`, indexes on
  `(to_table, to_column)` and `(from_table, from_column)` and
  `(producer_kind, producer_ref)`. Table keys are written the way the table
  graph names them (`silver.x`, `serving.x`, `bronze.x`, `sql_source:<id>`).
  A producer's rows are replaced as a set on each write (delete by
  `producer_kind, producer_ref`, insert), in one transaction. A partial row
  has `from_column` null when nothing could be resolved, and a `reason`.
- **One helper, pure.** `A/sql_lineage.rs`: `map_select(sql, dialect,
  known_columns: &HashMap<table, Vec<column>>) -> Lineage { mappings,
  partial: Vec<(output, reason)> }`. For each output column: its name (alias
  or column name), its source columns (identifiers resolved through the
  FROM/JOIN aliases; an unqualified identifier resolves when exactly one
  table in scope has that column in `known_columns`), and `transform`:
  `copy` (a bare column, possibly renamed), `cast` (a `CAST` of one column),
  or `expression`. `*` and `t.*` expand from `known_columns`, or are partial
  ("columns of <table> not known"). A CTE, a subquery in FROM, a `UNION`, a
  table function, or an identifier that resolves to no or several tables
  gives a partial row with a fixed reason; never a guess. `INSERT INTO t
  SELECT …` without a column list maps by position against `t`'s columns.
  Unit tests cover each case.
- **Pipelines (F2).** `enforce_pipeline_sql` stores the SQL it approved for
  the pipeline in Postgres, not in memory (a restart between enforce and
  success would lose it): `column_lineage_pending (pipeline_id
  PRIMARY KEY, sql TEXT, enforced_at)` in the same migration. On a
  `SUCCESS` run-finished for that pipeline, the API maps the pending SQL
  (known columns from `system.columns` for ClickHouse sources and the
  Iceberg schema for Bronze sources), writes the rows (producer
  `pipeline`, ref = pipeline id, the pipeline's tenant), and clears the
  pending row. A failed run writes nothing (the event is refused before).
- **Gold and Silver built outside a pipeline (F3).** On the 15-minute alerts
  tick, a pass reads the log entries newer than the last one processed
  (kept in a one-row state table in the same migration), with the query
  text, maps each with the helper (target columns from `system.columns`),
  and writes rows (producer `build`, ref = target table, shared, no
  tenant). A build whose target is also a pipeline's target is skipped, as
  `builds::drop_repeats` does for edges.
- **SQL sources (F4).** In `dashboard_sources::save`, after the probe, map
  the normalized SQL with the probed output names and write rows (producer
  `sql_source`, ref = source id, `to_table = sql_source:<id>`). A delete of
  the source deletes its rows.
- **Reading.** `GET /api/governance/lineage/columns?table=&column=&direction=up|down`
  (`lineage:read`, same tenant rule as the table route): rows reached by
  following mappings up to 10 hops, each `{ from, to, transform, partial,
  reason, producer: { kind, ref }, recordedAt, hop }`, plus `retention: {
  days: 365, oldestRecordedAt }`, plus for `down`: `charts` — every chart
  whose mart or SQL source is reached and whose `ChartInput` names the
  column, `{ chartId, chartTitle, dashboardId, dashboardTitle }` — present
  only for a caller with `dashboard:read`, otherwise `chartsHidden: true`.
  The existing table route's `columnMappings` becomes the mappings that
  touch the focus table, in the new row shape; contract, parity corpus and
  fixtures updated.
- **Retention (F8).** The same tick deletes rows with `recorded_at` older
  than 365 days.
- **Console.** The asset's Lineage tab gains a column picker; for the
  chosen column: Upstream and Downstream lists (table.column, transform, a
  "partial" pill with the reason as its title, producer), then Charts and
  dashboards (or "Listing dashboards needs dashboard access"), and the
  retention line ("Column lineage kept for 365 days; oldest record
  <date>").
- **Gate (F7).** Add `build` and `upload` to `RECORDED_LINEAGE_EDGE_KINDS`,
  and a gate step that, after the gate's pipeline run, asks the columns
  route for one of the pipeline target's columns and asserts at least one
  upstream row from the source table (`DATA-13-AC1` on CI).
- **Not changed.** The table graph, the copilot's `get_lineage` (AI team,
  `AI-16`), Gold export (a copy, not a build).

## 4. Tasks

One task per commit, in order. Cite `DATA-13` and the finding at each fix
site and in the commit body.

1. **Migration and store.** `0079`, store module with replace-a-producer,
   read-up/down (recursive CTE, 10 hops), purge, pending and state rows.
   `sqlx::test`s for each.
2. **The helper (F5).** `A/sql_lineage.rs` and its unit tests (copy, rename,
   cast, expression, join with aliases, unqualified resolution, `*` with and
   without known columns, CTE/subquery/UNION/table function partial,
   positional INSERT).
3. **Pipelines (F2).** Pending SQL at enforce; mapping at success; tests at
   handler level for success, failure (nothing written) and a restart
   between the two (the pending row survives).
4. **Builds (F3).** The tick pass, its state row, skip pipeline targets;
   tests with a mocked ClickHouse.
5. **SQL sources (F4).** Write on save, delete on delete; tests.
6. **Read route, impact, retention.** Route with `POLICY_TABLE` row,
   `policy.rs` test and `route_auth` cases; charts matching; purge on the
   tick; table route's `columnMappings` in the new shape; parity corpus.
7. **Console.** Contract, client, the Lineage tab's column view; tests.
8. **Gate (F7).** `g3a` kinds and the new step.
9. **Handoff.**

## 5. Build limits on this machine

As `2026-10-16-data-15-asset-page-depth.md` section 5 (shared target dir,
two jobs, `CARGO_INCREMENTAL=0`, no `cargo test`, `touch` before a quoted
clippy, the TypeScript and Python lines; no docker `up`). Another developer
builds `DATA-15` at the same time with the same target dir: if cargo waits
on the lock, wait.

## 6. Out of scope

- Lineage from systems outside the lakehouse (`GOV-3`), the copilot tool,
  Gold export mappings, Query Studio saved queries (not a "saved SQL
  source").
- Column lineage across connectors (source system to Bronze).

## Handoff

*(developer)*

## Review

*(planner)*
