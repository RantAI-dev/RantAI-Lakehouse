# Pipelines built from steps: SQL, joins and aggregations

| | |
| --- | --- |
| Module | Build |
| Backlog | `BLD-1` |
| Kind | Feature |
| Spec | `docs/core/specs/bld-1.md` |
| Status | Building |
| Priority | In `BACKLOG.md`, the one place it is kept |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | Reads as its author: `docs/superpowers/plans/2026-10-15-bld-1-run-as-author.md` (merged). Part 1: `docs/superpowers/plans/2026-11-12-bld-1-part-1-steps-and-sql.md`. The text form, the last slice of part 1: `docs/superpowers/plans/2026-11-13-bld-1-part-1-pipeline-as-text.md` (planned, not started). Parts 2 and 3 are not written yet. |
| Started | 2026-10-15 |
| Shipped | |
| Evidence | PR #173 (a pipeline reads as its author); PR #481 (part 1, S1: inputs and steps, the compiler, the check at save, the run route, the assistant); PR #500 (part 1, S2: the preview of a step); PR #499 (part 1, S3: the editor in the console) |

## Problem

A pipeline built in the console reads one table and has five operations
(dedupe, filter, rename, cast, select). A filter is one comparison. There is
no join, no aggregation, no calculated column and no SQL, so most Silver and
Gold tables cannot be built in the Build module or by the assistant.

The product owner's direction (2026-11-12): the person building a pipeline
gets the freedom to write it. The page becomes an editor of steps, and SQL
comes first.

## How it works

A pipeline is a list of **inputs**, an ordered list of **steps**, and one
**output table**.

- An input is a Bronze, Silver or Gold table, given a short name.
- A step is a named table. It reads inputs and earlier steps by name. A step
  is written as SQL (one `SELECT`) or with the form (the visual operations).
- The output table is what the last step returns. Every run rebuilds it.

This is the unit Snowflake Dynamic Tables and dbt use (one `SELECT` makes one
table, tables read each other by name), with the form on top, as Databricks
Lakeflow Designer does.

## What the user can do when this is done

### Part 1: the step editor and SQL

1. Add up to 5 input tables to a pipeline, from Bronze, Silver or Gold.
2. Add steps, name them, reorder and remove them.
3. Write a step as SQL: one `SELECT` over the inputs and earlier steps, with
   joins, aggregations, `CASE`, subqueries and window functions.
4. Keep using the five existing operations as a form step.
5. Preview the first 100 rows of any input or step.
6. See a clear refusal, on the step at fault, when a step is not a single
   `SELECT`, reads a table that is not an input, or reads a table they may
   not read.
7. Ask the assistant to build the same pipeline in plain language.
8. See the pipeline as one text file, copy it, and make a pipeline from one.
9. Set a target lag on a pipeline with several inputs: it refreshes when any
   of them changes, without naming which pipelines to wait for.

### Part 2: joins and aggregations without SQL

10. Join two inputs or steps in a form: inner, left, right or full outer, on
    one or more pairs of columns.
11. Group by columns and add `count`, `count distinct`, `sum`, `avg`, `min`,
    `max`; filter on an aggregated column.
12. Filter with several conditions, joined by all or any.
13. See any form step as SQL, and switch it to a SQL step (one way).

### Part 3: reuse and output options

14. Publish a step as its own table, so other pipelines can read it.
15. Choose what a run does when the output's columns change, and run quality
    rules before the output is published.

## Not included

- Python steps and notebooks. Not decided (`DEC-1`); needs a runner that is
  isolated from the orchestrator's secrets first.
- dbt projects (`DEC-2`).
- Refreshing only the changed rows. Every run rebuilds the output (`BLD-12`).
- More than one statement in a step, and statements that change data or
  tables.
- Storing a pipeline in a Git repository.

## Asking the assistant

Yes, in part 1. "Join orders and customers on customer_id, total amount per
country, write it to silver.sales_by_country." The assistant's
`create_pipeline` takes inputs and steps, and writes SQL steps. The AI team
reviews that part.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Scope | Joins, basic aggregations and a free SQL step | Product owner, 2026-10-14 |
| 2 | Who may use the SQL step | Anyone with `pipeline:write` | Product owner, 2026-10-14 |
| 3 | Who the pipeline reads as | Its author: every read goes through the author's masking and row filters, when saved and when run | Product owner, 2026-10-14 |
| 4 | Inputs per pipeline | 5 | Product owner, 2026-10-14 |
| 5 | Preview rows | 100 | Product owner, 2026-10-14 |
| 6 | Assistant | Same part as the feature it covers | Product owner, 2026-10-14 |
| 7 | Shape of the page | An editor of named steps; a step is SQL or a form; SQL is built first | Product owner, 2026-11-12 |
| 8 | Refresh | The user does not choose how the output is refreshed; no Replace, Append or Merge choice on the page | Product owner, 2026-11-12 |
| 9 | Order between pipelines | Derived from the inputs; the user does not name pipelines to wait for | Product owner, 2026-11-12 |
| 10 | The pipeline as text | A text form of the whole pipeline, in part 1 | Product owner, 2026-11-12 |
| 11 | Publishing a step as a table | Later (part 3) | Product owner, 2026-11-12 |
| 12 | Window functions in a SQL step | Allowed. It is still one `SELECT` through the same access rewrite. Replaces "no window functions" of 2026-10-14 | Product owner, 2026-11-12 |
| 13 | Who a preview reads as | The person looking, never the author, so an editor sees no row the author can see and they cannot | Product owner, 2026-11-12 |
| 14 | Who the pipeline reads as after someone else edits it | When someone other than the author changes the inputs or the steps, the pipeline reads as that person from then on. Closes `SEC-71` | Product owner, 2026-11-12 |
| 15 | Steps per pipeline | 20 | Product owner, 2026-11-12 |
| 16 | Length of a SQL step | 20,000 characters, the limit a dashboard's SQL source has | Product owner, 2026-11-12 |
| 17 | Time limit of a preview | 30 seconds | Product owner, 2026-11-12 |
| 18 | Kind and incremental column | Removed from the page. Nothing reads them today; every run rebuilds the output (`BLD-12`) | Product owner, 2026-11-12 |

## Limits to tell a customer

- At most 5 input tables and 20 steps per pipeline.
- A SQL step is one `SELECT`. It cannot change data or tables, and it reads
  only the pipeline's inputs and earlier steps.
- Every run rebuilds the output table and gives it the columns of the last
  step. A dashboard that reads a column the pipeline no longer returns
  breaks.
- A pipeline reads as its author: a column masked for the author is masked
  in the output table too.
- A preview shows what the person looking may see. It can differ from what
  the pipeline writes when that person is not the author.
- Changing the inputs or steps of someone else's pipeline makes it read as you from then on. An administrator who does this makes the output unmasked; the editor says so before saving.
- A pipeline with a SQL step, several inputs or more than one step has no column lineage yet (`DATA-57`). Table lineage shows every input.
- The editor has not been checked in a browser by anyone yet, nor against a running API; the acceptance checklist below is that check.
- The text form of a pipeline (capability 8) is not built yet.
- No Python.

## Acceptance checklist

Part 1. Run on a running console by the product owner. A step not performed
is never a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `BLD-1-AC1` | Add two Bronze inputs, write a SQL step that joins them and sums by a column, write a Silver table, run it | Row count and totals equal the same query in Query Studio | Not run |
| `BLD-1-AC2` | In a SQL step, use a left join where some rows have no match | Unmatched rows are kept, with empty columns | Not run |
| `BLD-1-AC3` | Preview an input, then a step | First 100 rows, or fewer, with column names and types | Not run |
| `BLD-1-AC4` | Save a SQL step containing `DROP TABLE` or `INSERT` | Refused when saved, on that step, with a clear message | Not run |
| `BLD-1-AC5` | Save a SQL step that reads a table that is not an input | Refused, on that step, saying to add the table as an input | Not run |
| `BLD-1-AC6` | As an author for whom column C of table T is masked, run a pipeline that copies C | C is masked in the output table | Not run |
| `BLD-1-AC7` | Save a pipeline reading T, then add a masking policy on a column it copies, then run it | The new run's output has the column masked | Not run |
| `BLD-1-AC8` | As a user who may not read table T, add T as an input and save | Refused with an honest message | Not run |
| `BLD-1-AC9` | Open a pipeline made before this change | It opens in the editor as one input and one form step, and runs as before | Not run |
| `BLD-1-AC10` | Ask the assistant for the pipeline of AC1 | The same pipeline is created as a draft | Not run |
| `BLD-1-AC11` | As a user without `pipeline:write`, try to create a pipeline and to preview a step | Both refused with an honest message | Not run |
| `BLD-1-AC12` | Give a pipeline with two inputs a target lag; change the second input | The pipeline runs again within the lag | Not run |
| `BLD-1-AC13` | Copy a pipeline's text form, make a new pipeline from it under another name | The new pipeline has the same inputs, steps and output settings | Not run |
| `BLD-1-AC14` | Pause the pipeline | It stops running; the output table stays | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
