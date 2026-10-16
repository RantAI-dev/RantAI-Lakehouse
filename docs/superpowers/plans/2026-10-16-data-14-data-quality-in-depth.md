# `DATA-14` Data quality in depth — Implementation Plan

| | |
| --- | --- |
| Item | `DATA-14`, Data roadmap phase 2 (master plan `2026-10-14-data-phase-2.md`) |
| Branch / PR | `feat/data-12-certification-governed-tags`, PR #106; built on the sub-branch `feat/data-14-quality` in worktree `/home/hv/lakehouse-data14`, merged back by the planner |
| Base verified at | `57de99d7` (phase branch, `origin/main` `53d05f00` merged in) |
| Scope | **The spec's Target rows and `DATA-14-AC1`–`AC4`** (`docs/core/specs/data-14.md`); the product owner decided on 2026-10-16 that phase 2 follows the specs. Where the feature page's decisions differ from a spec row (two threshold levels), the spec row wins |
| Written | 2026-10-16 by the planner (Claude Opus) for a developer agent, under `AGENTS.md` |

## 1. The spec rows and what this plan builds

| Spec row | Target | This plan |
| --- | --- | --- |
| Ready checks | Null count and percent, duplicate count, row count, freshness, accepted values, min/max/average, unique count | Built (task 1): the threshold grammar gains the missing checks; every existing rule keeps working |
| Expectations | Each check takes a threshold; breaking it opens an incident | Built (task 2): one threshold per check, as today; a failed run opens an incident, kept open until a person resolves it |
| Schedules | Per table: on every load, or every 1 hour to every day | Built (tasks 3–4). On every load: authored pipelines, uploads and connector runs (a connector run's success covers every table its ingest spec loads) |
| Anomaly detection | Volume and freshness anomalies learned from at least 14 days of history | Built (task 5): the product records each table's loads from now on; detection starts once a table has 14 days and 5 loads of history, and says "learning" before that |
| Overview | A quality page listing open incidents by table, with history | Built (task 6) |
| Alerts | Through SRC-7's channels | Built (task 7): a new alert kind for an opened incident, delivered by webhook or email like SRC-7's |

## 2. What exists today (anchors at `57de99d7`)

`A` is `rust/crates/lakehouse-api/src`, `R` is `A/routes`.

| # | Code | Verdict |
| --- | --- | --- |
| F1 | `R/quality.rs`: `enum Check` (`:106-119`) has `RowCount`, `NotNull{min_percent}`, `Unique`, `Range`; `parse_threshold` (`:174-223`); `check_sql` (`:253-282`) one aggregate per check with `max_execution_time = 30`; verdict passed/failed, `warning` only for "no rows to check" (`:304-309`). `GRAMMAR_HINT` (`:57`). | Accepted values, duplicate count, null count, average, distinct count and freshness are missing. |
| F2 | Runs: `console.quality_run` in ClickHouse, created by `ensure_table` (`:351-371`), no threshold column, no TTL; `latest_runs` (`:405`); `forget_runs` (`:583-591`) deletes a rule's runs when its asset or threshold is edited (`:651-654`) and on delete (`:714`). No incident exists anywhere; `alert_instance` cannot serve (resolve records no actor, needs only a login). | History is erased on edit; incidents are new. |
| F3 | Schedule: one global rule: a rule is due when its verdict is over 60 minutes old (`SCHEDULED_EVERY_MINUTES` `:732`), 25 per tick (`SCHEDULED_BATCH` `:736`), started by the 15-minute alerts tick (`R/alerts.rs:907-908`; Dagster `alerts_run.py:142`). `spawn_scheduled_pass` (`:868`) returns `false` while a pass runs. | No per-table schedule; 25 per tick caps hourly rules at about 100. |
| F4 | Loads: `run_finished_event` (`R/pipelines.rs:2679`) already starts background passes (`:2693`, `:2696`); a connector run (`ingest_job`) reaches `connector_run_event(.., Success)` (`:2701-2706`), which finds the connector from the run config (`R/load_alerts.rs:176`); an authored pipeline's target is known (`lakehouse-store/src/pipelines.rs:141`); an upload's success (`file_ingest_job`) falls into "unknown jobName" (`:2707-2712`). | The on-load hook is this event; uploads need routing. |
| F5 | History for anomalies: Iceberg snapshots are kept 7 days (OPS-1); `lake.bronze_meta.ingest_run` keys on the source object, no run id; pipeline volume uses the last 30 Dagster runs per pipeline (`R/pipelines.rs:2634`). No per-table load history of 14 days exists. | The product records its own. |
| F6 | Alerts (SRC-7): `AlertKind` (`lakehouse-alerts/src/lib.rs:128-200`), channels webhook and email (`:258`), rules in ClickHouse `console.alert_rule` (`ensure` `:834-879`), connector-scoped delivery guarded by `is_connector_scoped` (`:206`); console kinds in `src/services/contracts/alerts.ts:26-31`. | A table-scoped kind is new. |
| F7 | Console: `src/features/governance/data-quality-page.tsx` (flat list, add sheet with the grammar hint `:339-345`, drawer `:195-267`), `quality-rule-edit-dialog.tsx:146` ("results … are cleared"), `src/features/catalog/asset-quality.tsx` (`KINDS` `:49-54`, `ruleThreshold` `:77-100`). Contracts `src/services/contracts/governance.ts:39-131`. | Pickers, incidents, history and schedule controls are new. |
| F8 | The assistant's `draft_quality_rule` (`registry.rs:739`, `tools/governance.rs:151`) sends a threshold string; its schema is pinned in `tests/fixtures/tool_schemas.json`. It is a Data tool owned by the AI team (`AI-16`). | Unchanged: the threshold stays a string, so its schema does not change. |

Migration number: **`0081`** (reserved for `DATA-14`; `0080` is taken on another branch). Check `ls rust/migrations | cut -c1-4 | sort | uniq -d` prints nothing.

## 3. Decisions already made by the planner

- **Grammar additions (task 1)**, each one aggregate and one comparison,
  following today's style (`col` is a column name, `N` a number, `D` a
  duration `30m`/`6h`/`2d`):
  - null count `col nulls <= N`, null percent `col nulls <= N%`
    (today's `col not null [>= P%]` stays);
  - duplicate count `col duplicates <= N` (today's `col unique` stays and
    means `duplicates <= 0`);
  - row count `rows <cmp> N` (exists);
  - freshness `col fresh within D`: the newest value of a time column is no
    older than `D`;
  - accepted values `col in ('a', 'b', …)`: no value outside the list
    (nulls are not counted; say so in the hint);
  - min / max / average `min(col) <cmp> x`, `max(col) <cmp> x`,
    `avg(col) <cmp> x`, `avg(col) between a and b` (today's `col between a
    and b` stays);
  - unique count `col distinct <cmp> N`.
  Literals in `in (…)` are bound safely the way today's range literals are;
  read how `check_sql` builds SQL and keep to it. The two console pickers
  offer every check. `GRAMMAR_HINT` lists them.
- **Incidents (task 2).** Migration `0081_quality_incidents.sql`:
  `quality_incident (id UUID, asset TEXT, rule_id TEXT NULL, kind TEXT CHECK
  IN ('check','volume_anomaly','freshness_anomaly'), opened_at, last_seen_at,
  detail TEXT, status TEXT CHECK IN ('open','resolved'), resolved_at,
  resolved_by_user_id UUID, resolution TEXT)`, one open incident per
  `(asset, rule_id or kind)` (partial unique index). A failed run opens one
  or, if one is open, moves `last_seen_at`. A passing run does not close it.
  `POST /api/governance/quality/incidents/{id}/resolve` `{ resolution }`
  (`governance:write`, reason required, audited `quality.incident_resolve`).
  `GET /api/governance/quality/incidents?status=` with the same permission as
  the quality list. Deleting a rule resolves its open incidents with the
  resolution "rule deleted" and the deleting user.
- **History kept (task 2).** `forget_runs` is no longer called on edit; it
  stays on delete. `quality_run` gains a `threshold` column (`ALTER TABLE …
  ADD COLUMN IF NOT EXISTS` in `ensure_table`; old rows stay empty) so a kept
  run says what it checked. The edit dialog's sentence changes accordingly.
- **Schedules (task 3).** In `0081`: `quality_schedule (asset PRIMARY KEY,
  mode TEXT CHECK IN ('on_load','interval'), interval_minutes INT CHECK
  BETWEEN 60 AND 1440, updated_by, updated_at)`. No row: interval 60, today's
  behaviour. `PUT /api/governance/quality/schedules/{asset}`
  (`governance:write`). The tick runs the rules that are due by their
  table's interval, oldest due first, up to 100 per tick (was 25), and says
  in the pass's log how many it left for the next tick.
- **On every load (task 4).** From `run_finished_event`, after its existing
  checks, a background pass for the loaded tables whose schedule is
  `on_load`: an authored pipeline's target; an upload's table (route
  `file_ingest_job` success to the upload, as its failure is routed today);
  a connector run's target tables from the connector's ingest spec. The
  pass runs after the success is confirmed and never fails the event. A
  failed run triggers nothing (tables a failed connector run committed are
  covered by their interval).
- **Load history and anomalies (task 5).** In `0081`: `table_load (asset,
  loaded_at, rows_after BIGINT, PRIMARY KEY (asset, loaded_at))`, written by
  the same on-load hook for every loaded table whatever its schedule
  (`rows_after` from a `count()` with a `WHERE`, per the R11 lint), kept 90
  days. A table is **learned** when its history spans 14 days and holds 5
  loads. Volume: the load's size is `rows_after` minus the previous load's
  when the table only ever grew, otherwise `rows_after`; an anomaly when it
  is under half the median of the previous 30 loads (a load of 10 % is
  flagged: `DATA-14-AC2`). Freshness, on the tick: an anomaly when the time
  since the last load exceeds twice the median gap between loads and at
  least one hour. Each opens an incident of its kind. An unlearned table
  shows "learning: N days and M loads so far".
- **Overview (task 6).** The Data Quality page leads with Open incidents
  grouped by table (count, oldest, each incident's detail and Resolve), then
  the rules list; a rule's drawer shows its last 50 runs with the threshold
  each ran with. The asset's Quality tab shows its open incidents and its
  schedule control.
- **Alerts (task 7).** A new kind `quality_incident`, scoped to one table or
  all (`*`), delivered when an incident opens (not when it is seen again),
  through the existing webhook and email delivery. `console.alert_rule`
  gains an `asset` column (`ADD COLUMN IF NOT EXISTS` in `ensure`). The
  console's alert form lists the kind.
- **Permissions (AC4).** Unchanged for viewing; writing rules, schedules and
  resolving need `governance:write`; every new route in `POLICY_TABLE` with
  `route_auth` cases.
- **Not changed.** The assistant's tool (F8); the legacy observed rows from
  `_silver_meta.quality` (they have no stable id and open no incident; say
  so in a comment).

## 4. Tasks

One task per commit, in order. Cite `DATA-14` and the finding at each fix
site and in the commit body.

1. **Checks (F1, F7).** Grammar, SQL, verdicts, hint, both pickers. Tests per
   check: parse, SQL shape, pass and fail.
2. **Incidents and kept history (F2).** Migration `0081` (all three tables
   of this plan), store, open/seen/resolve, delete resolves, `forget_runs`
   only on delete, `threshold` on runs, the two routes. Tests: a failing
   `not null` run opens one incident (`AC1`), a second failure moves
   `last_seen_at`, a pass leaves it open, resolve needs a reason and the
   permission, delete resolves.
3. **Schedules (F3).** Store, route, the tick's due selection and cap.
   Tests: interval bounds, due order, the default.
4. **On load (F4).** The hook for the three load kinds; upload success
   routing. Tests at handler level for each kind, and that a failure
   triggers nothing.
5. **Load history and anomalies (F5).** Writer, purge after 90 days,
   learned state, volume on load, freshness on the tick. Tests: unlearned
   says so; a 10 % load opens a volume incident; a missed load opens a
   freshness incident; a growing table measures increments.
6. **Overview (F7).** Console pages and contracts; tests.
7. **Alerts (F6).** Kind, scope, delivery on open, console form; tests with
   the existing delivery test doubles.
8. **Handoff.**

## 5. Build limits on this machine

As `2026-10-16-data-15-asset-page-depth.md` section 5. Another developer
may be building with the same target dir: if cargo waits on the lock, wait.

## 6. Out of scope

- AI-suggested checks (`AI-16`), the assistant's quality tool.
- Two threshold levels per check (not in the spec).
- Per-table on-load checks for change-capture (CDC) tables, which load
  outside Dagster; their interval covers them.

## Handoff

*(developer)*

## Review

*(planner)*
