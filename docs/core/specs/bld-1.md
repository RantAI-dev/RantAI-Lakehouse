# `BLD-1` Pipelines built from steps: SQL, joins and aggregations

| | |
| --- | --- |
| Backlog | `BLD-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks), in three parts: 1 the step editor and SQL, 2 joins and aggregations as forms, 3 reuse and output options |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open |

## Why

A pipeline built in the console reads one table and has five operations:
dedupe, filter, rename, cast, select. Joining two tables or summing by a
column means writing SQL outside the product, so a Silver or Gold table that
needs either cannot be built in the Build module or by the assistant.

The product owner's direction (2026-11-12): give the person building a
pipeline the freedom to write it.

## What users get

A pipeline is inputs, an ordered list of named steps, and one output table.
A step is one `SELECT` or a form. The assistant builds the same pipelines
from a request in plain language. The decisions, signed and proposed, are on
the [feature page](../features/pipeline-joins-and-sql.md).

## Target specs

"Today" is `main` at `657c2502`, read from the code, not tested. A target is
either a competitor's documented behaviour (named under Benchmark), a
decision on the feature page, or marked *(proposed)*: the planner's number,
which the product owner confirms or changes on the feature page.

| Capability | Today | Target | Part |
| --- | --- | --- | --- |
| Inputs per pipeline | 1 table | Up to 5 tables from Bronze, Silver or Gold, each with a name | 1 |
| Steps | A list of up to five kinds of operation on the one table | An ordered list of up to 20 *(proposed)* named steps; a step reads inputs and earlier steps by name | 1 |
| SQL step | None | One `SELECT` (with `WITH`) of up to 20,000 characters *(proposed)*. Anything else (DDL, `INSERT`, `SYSTEM`, table functions, `SETTINGS`, `FORMAT`, a second statement) is refused when saved. Window functions are allowed *(proposed)* | 1 |
| What a step may read | The one source table | Only the pipeline's inputs and earlier steps. A table that is not an input is refused, naming the step | 1 |
| Check when saved | The five operations are parsed. Access is not checked | The whole pipeline is compiled and put through the access rewrite for the person it reads as. A refusal fails the save and names the step | 1 |
| Read as the author | When run, the statement is rewritten for the author's roles (PR #173) | The same, for the compiled statement of all steps | 1 |
| Where the statement is built | In the orchestrator (Python), from the five operations | In the API only. The orchestrator asks the API for the statement and runs it | 1 |
| Preview | None | First 100 rows of any input or step, as the person looking, within 30 seconds *(proposed)* | 1 |
| Existing pipelines | One table and five operations | Open in the editor as one input and one form step; run as before | 1 |
| Target lag with several inputs | Follows the one source table | Follows every input | 1 |
| The pipeline as text | The stored definition is shown as JSON, read-only | A text form that can be copied and used to make a pipeline | 1 |
| Assistant | Five operations | Inputs, steps and SQL steps through `create_pipeline`; tool schema fixture regenerated | 1 |
| Join types as a form | None | Inner, left, right, full outer (Metabase's four); one or more column pairs, equality only | 2 |
| Aggregation as a form | None | Group by any columns; `count`, `count distinct`, `sum`, `avg`, `min`, `max`; filter on an aggregated column | 2 |
| Filter as a form | One comparison | Several conditions, all or any | 2 |
| A form step as SQL | None | Shown as SQL; can be switched to a SQL step, one way | 2 |
| A step as its own table | None | A step can be published as a table other pipelines read | 3 |
| Output options | The output is rebuilt and takes the last step's columns | A choice of what a run does when the columns change; quality rules before the output is published | 3 |

## Security rules for the SQL step

These are conditions of the feature, not options:

1. Parsed with the SQL parser the API already uses, not only by string
   matching. One statement; a `SELECT` (with `WITH`) only.
2. A step reads only the pipeline's inputs and earlier steps. The check is on
   the parsed tables of the compiled statement, not on the text.
3. The whole compiled statement goes through the tenant gate and the access
   rewrite (`policy_engine::rewrite_sql_for_roles`) for the person the
   pipeline reads as, at save (to refuse early) and at every run (policies
   change). A refused query fails with a fixed message.
4. The run still uses the orchestrator's ClickHouse user; rule 3 is what
   makes it read only what the author may see, masked as the author sees it.
5. Masking therefore carries into the output (`GOV-4`, decided 2026-10-14).
6. A preview runs as the person looking, through the same gate and rewrite
   as Query Studio.
7. Error text from ClickHouse never reaches the console from a run; the run
   shows a fixed message and the detail goes to the server log. A preview
   reports an error the way Query Studio reports one for the same SQL.

## Benchmark

- Snowflake Dynamic Tables: a table is one `SELECT` and a target lag;
  Snowflake orders the refreshes by which table reads which, and chooses
  between an incremental and a full refresh. No builder in the product;
  transformations are code (SQL, Snowpark, dbt projects).
- Databricks Lakeflow: declarative pipelines in SQL or Python, and Lakeflow
  Designer, a visual builder whose flows run as declarative pipelines.
- dbt: one `SELECT` per model, models read each other by name, tests per
  model.
- Metabase notebook editor: joins (left, right, inner, full), several join
  conditions, summarize with count, sum, average, distinct, min, max, then
  filter on the result. Tableau Prep: join, aggregate and a custom SQL input.

Where we stay behind after all three parts: Python and notebooks (`DEC-1`),
dbt projects (`DEC-2`), refreshing only changed rows (`BLD-11`), pipelines
kept in a Git repository.

## Acceptance checklist

On the [feature page](../features/pipeline-joins-and-sql.md), `BLD-1-AC1` to
`BLD-1-AC14` for part 1. Parts 2 and 3 add their own when they are planned.

## Asking the assistant

The assistant change is part of this item (decision 6). The AI team reviews
that part of the plan and the diff.

## Before anyone builds it

The feature page holds the decisions; every *(proposed)* one is signed by
the product owner before the part that needs it is built (`AGENTS.md`).
