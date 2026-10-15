# `BLD-1` Joins, aggregations and a SQL step in the pipeline builder

| | |
| --- | --- |
| Backlog | `BLD-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks), in two parts: A joins and aggregations, B the SQL step |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Waits for | `BLD-3` (a pipeline must read Bronze before it can join Bronze tables) |

## Why

A pipeline built in the console has five steps: dedupe, filter, rename, cast,
select. It reads one table. Joining two tables or summing by a column means
writing SQL outside the product, so a Silver or Gold table that needs either
cannot be built in the Build module or by the assistant.

## What users get

A pipeline that joins tables, groups and aggregates, and, where the visual
steps are not enough, runs a SQL step. The assistant builds the same
pipelines from a request in plain language.

## Decisions (product owner, 2026-10-14)

- Scope: joins and basic aggregations, **plus a free SQL step**. Window
  functions are not included.
- Who may use the SQL step: anyone who may create a pipeline
  (`pipeline:write`). No separate permission.
- The assistant gets the same steps in the same PR (`create_pipeline`).
- **A pipeline reads as its author** (2026-10-14): its SQL goes through the
  same masking and row-filter rewrite (`sql_rewrite::enforce`) as the
  author's own queries in Query Studio, for the author's roles. A column
  masked for the author is masked in the output table. This also closes
  `GOV-4` for pipelines.

## Target specs

"Today" is `main` at `99b42812`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Inputs per pipeline | 1 table | Up to 5 tables *(proposed)* from Bronze, Silver or Gold |
| Join types | None | Inner, left, right, full outer (Metabase's four) |
| Join conditions | None | One or more column pairs per join, equality only |
| Aggregation | None | Group by any columns; `count`, `count distinct`, `sum`, `avg`, `min`, `max` |
| Filter after aggregation | None | A filter step on an aggregated column |
| SQL step | None | One `SELECT` statement over the pipeline's inputs and earlier steps; anything else (DDL, `INSERT`, `SYSTEM`, table functions, `SETTINGS`) refused when saved |
| Read as the author | Runs as the orchestrator, unmasked | Every read, through any step, is rewritten for the author's roles exactly as Query Studio rewrites the author's own query (masking and row filters). A table the rewrite refuses fails the save, and fails the run with a fixed message if it is refused later |
| Preview | None | First 100 rows *(proposed)* of any step, with the same permission check |
| Assistant | Five steps | The same steps, joins, aggregations and SQL step through `create_pipeline`; tool schema fixture regenerated |

## Security rules for the SQL step

These are conditions of the feature, not options:

1. Parsed with the SQL parser the API already uses, never by string
   matching. One statement; a `SELECT` (with `WITH`) only.
2. The whole pipeline query, SQL step included, goes through
   `sql_rewrite::enforce` for the author's roles, at save (to refuse early)
   and at every run (policies change). A refused query fails with a fixed
   message.
3. The run still uses the orchestrator's ClickHouse user; rule 2 is what
   makes it read only what the author may see, masked as the author sees it.
4. Masking therefore carries into the output (`GOV-4`, decided 2026-10-14).
5. Error text from ClickHouse never reaches the console; the run shows a
   fixed message and the detail goes to the server log.

## Benchmark

Metabase notebook editor: joins (left, right, inner, full), multiple join
conditions, summarize with count, sum, average, distinct, min, max, then
filter on the result. Tableau Prep: join, aggregate and a custom SQL input.
Snowflake and Databricks: full SQL. Databricks Lakeflow pipelines also take
plain SQL.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BLD-1-AC1` Join two Bronze tables on one column, aggregate by a column, write a Silver table; row counts match the same query in Query Studio
- `BLD-1-AC2` A left join keeps rows with no match
- `BLD-1-AC3` A SQL step with a `DROP` or `INSERT` is refused when saved, with a clear message
- `BLD-1-AC4` A column masked for the author is masked in the output table, and a masking policy added later applies from the next run
- `BLD-1-AC5` The assistant builds the AC1 pipeline from a plain-language request
- `BLD-1-AC6` A user without `pipeline:write` is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Window functions, pivot and unpivot, unions.
- Joins on anything but equality.

## Asking the assistant

The assistant change is part of this item (decision above). The AI team
reviews that part of the plan and the diff.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
