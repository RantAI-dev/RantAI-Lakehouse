# BI-2: reusable metrics — Implementation Plan

**Status:** planner defaults, not started. Written by the planner (Claude
Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/reusable-metrics.md`. Specs:
`docs/core/specs/bi-2.md`, `docs/core/specs/ai-7.md`. `DATA-12`
(certification) does not exist; the "certified" part of the spec's
Governance row is out of this plan and reported as not met.

**Base:** `feat/bi-phase-2`, after `BI-13` is committed. Read first: every
phase 2 plan before this one and its handoff; `lakehouse-bi` `formula/`
and `fields.rs` (a metric is an aggregate formula with a home and filters),
`filters.rs`, `shape.rs`, the KPI comparison of `BI-16` part A
(`compare`), `staged.rs` (period functions); `routes/alerts.rs` and the
`lakehouse-alerts` crate (what an alert rule can watch today);
`routes/ai/registry.rs`, `routes/ai/prompt.rs`; the policy module doc on
introducing a permission string (`settings:write` in `BI-9` is the recent
example) and its migration.

**Verification:** as for `BI-6`. Section 4 is not reduced.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Metric | A new `console.bi_metric` table (created like the other BI tables): id, `name` (an identifier, unique in the deployment), `label`, `description` (at most 500 characters), source (a mart or a SQL source), `formula` (formula language, must check as aggregate level over that source; table calculations and parameters with no default are refused), `filters` (a list of `FilterDef` and segment references on that source), optional `timeColumn` (a date or timestamp column of the source) and default `grain`, optional `format` hint, created by, updated at. |
| Segment | A new `console.bi_segment` table: id, `name` (identifier, unique per source), `label`, source, `filters` (a list of `FilterDef`). Usable wherever a chart filter is (`BI-6`) as `{ "segment": <name> }`, and inside a metric's filters. |
| Routes and permission | `/api/dashboard/metrics` and `/api/dashboard/segments`: `GET` with `dashboard:read`; `POST`, `PUT`, `DELETE` with the new `metric:write`, introduced as `settings:write` was (policy module doc, seed migration with the next free number checked against `origin/main` and open PRs, no seed grant beyond what `*:*` already gives unless the policy doc says otherwise). `GET /api/dashboard/metrics/{id}/usage` lists the charts, KPIs, alerts and dashboards that use it, by title. A validate route checks a metric definition without saving. All in `POLICY_TABLE` and `route_auth`. Delete in use is 409 with the usage list. `name` and source are immutable. |
| A metric's filters | Applied to the metric's number only: each aggregate in the compiled formula takes the filter as its condition (the engine's conditional form of that aggregate), so a filtered metric and an unfiltered measure can share one statement and one grouping. A formula whose aggregate has no conditional form is refused at save with a positioned message. |
| Use in a chart | A measure may be `{ "metric": <name> }` on a chart whose source is the metric's source (with `BI-7` joins: the base table). It behaves as an aggregate-level field: no chart aggregation, allowed where aggregate fields are. Charts store the name, not a copy, and are rebuilt at read time, so a changed metric reaches every chart at once. A missing metric is the tile's plain error, never a silent fallback. |
| KPI and comparisons | A `kpi` may take `{ "metric": <name> }` as its measure. For a metric with a `timeColumn`, the KPI's `compare` defaults to that column and grain, and gains `{ "kind": "last_year" }` beside `previous` (same period one year earlier, by calendar shift as `SamePeriodLastYear` does). |
| Alerts | An alert rule can watch a metric by name with the comparison operators it already has, evaluated through the same builder path as a KPI tile, with the rule owner's permissions as today's rules do. If today's alert rules cannot run a builder statement without a larger redesign of the alerts crate, stop and report the options instead of building around it. |
| Built-in cards | Unchanged. |
| Assistant (`AI-7`) | Tools: `list_metrics` (names, labels, descriptions, sources), `describe_metric`, `query_metric` (the metric's value, optionally grouped by one column or a time grain and filtered, through the same builder path and the caller's permissions), `create_metric` / `update_metric` (low-risk writes, `metric:write`), `delete_metric` (approval). The system prompt's dashboard section tells the model to look for a matching metric before writing SQL, to name the metric it used, and to say when none matched. All tools named in `src/features/copilot/capabilities.ts`; `tool_schemas.json` regenerated, tool count assertion updated. |
| Export | `chart_def_fields` lists the `{ "metric": … }` form. |

## 2. Tasks

- **T1 — `lakehouse-bi`:** metric and segment types, stores, validation; compiling a formula under a filter condition; `{ "metric": … }` and `{ "segment": … }` resolution in the shape and builder; `last_year` comparison.
- **T2 — API:** routes, the permission and its migration, policy and auth tests; usage; validate; delete-in-use; preview, dashboard, records, export, embed and public paths.
- **T3 — Alerts:** a rule on a metric (or the stop-and-report).
- **T4 — Assistant:** tools, prompt, schemas, fixture, capabilities.
- **T5 — Console:** a "Metrics" list reachable from the Dashboards area (name, description, source, used by), the metric editor (formula box of `BI-8`, filters and segments, time column and grain, a live value), the segment editor; "change shows usage" confirmation.
- **T6 — Console:** metrics in the measure picker with their own mark and description on hover; KPI on a metric with the two comparisons; segments in the chart filter editor.
- **T7 — Docs:** `CHANGELOG.md` `[Unreleased]`, including that certification waits for `DATA-12`.

## 3. Out of scope (do not build)

Certification; metrics over several tables; goals and approval flows; replacing the built-in cards.

## 4. Things the developer must verify, not assume (not reduced)

Against the dev `ClickHouse` (26.8) and the real role rewrite, statements and results quoted:

- `BI-2-AC1` and `AC2` on `serving.mart_demo_map_points`: a metric `Sum([visitors])` equals 278,955 in a chart, a KPI and `query_metric`; the same metric with a filter on `category`, beside the unfiltered measure, gives two numbers that each match a direct query.
- Every aggregate in the formula catalog either compiles under a condition and runs, or is refused at save: one generated statement per aggregate.
- `last_year` against a direct query on `visit_date` (data runs Sep 2025 to Nov 2026).
- The role rewrite: a metric over a masked column and with a row filter behaves as a plain chart does; `query_metric` cannot be used to read a source the caller cannot read; a metric defined by an administrator does not lend its author's access to a viewer.
- Filter and segment values with quotes, backslashes and non-ASCII appear only inside escaped literals (extend the property test).
- A missing or broken metric fails its tile only; the board and an embed still render.
- Charts saved before this change produce the same SQL.

## 5. Handoff (developer appends)

## 6. Review (planner appends)
