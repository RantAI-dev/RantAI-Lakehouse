# Joins, aggregations and a SQL step in pipelines

| | |
| --- | --- |
| Module | Build |
| Backlog | `BLD-1` |
| Kind | Feature |
| Spec | `docs/core/specs/bld-1.md` (every *(proposed)* number is signed under Decisions) |
| Status | Planned |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | Not written yet. **Waits for `BLD-3`**. |
| Started | |
| Shipped | |
| Evidence | |

## Problem

A pipeline built in the console reads one table and has five steps (dedupe,
filter, rename, cast, select). Joining tables or summing by a column needs
SQL written outside the product, so most Silver and Gold tables cannot be
built in the Build module or by the assistant.

## What the user can do when this is done

1. Add up to 5 input tables to a pipeline, from Bronze, Silver or Gold.
2. Join them: inner, left, right or full outer, on one or more pairs of columns.
3. Group by columns and add `count`, `count distinct`, `sum`, `avg`, `min`, `max`.
4. Filter on an aggregated column.
5. Add a SQL step: one `SELECT` over the inputs and earlier steps.
6. Preview the first 100 rows of any step.
7. See a clear refusal when a SQL step is not a `SELECT`, or reads a table they may not read.
8. Ask the assistant to build the same pipeline in plain language.

## Not included

- Window functions, pivot, unpivot, unions; joins on anything but equality.

## Asking the assistant

Yes, in the same PR. "Join orders and customers on customer_id, total
amount per country, write it to silver.sales_by_country." The assistant's
`create_pipeline` gets the joins, aggregations and SQL step. The AI team
reviews that part.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Scope | Joins, basic aggregations and a free SQL step; no window functions | Product owner, 2026-10-14 |
| 2 | Who may use the SQL step | Anyone with `pipeline:write` | Product owner, 2026-10-14 |
| 3 | Who the pipeline reads as | Its author: every read goes through the author's masking and row filters, when saved and when run | Product owner, 2026-10-14 |
| 4 | Inputs per pipeline *(proposed)* | 5 | Product owner, 2026-10-14 |
| 5 | Preview rows *(proposed)* | 100 | Product owner, 2026-10-14 |
| 6 | Assistant | Same PR | Product owner, 2026-10-14 |

## Limits to tell a customer

- At most 5 input tables per pipeline.
- The SQL step takes one `SELECT`; it cannot change data or tables.
- A pipeline reads as its author: a column masked for the author is masked in the output table too.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `BLD-1-AC1` | Join two Bronze tables on one column, group by a column with `sum`, write a Silver table | Row count and totals equal the same query in Query Studio | Not run |
| `BLD-1-AC2` | Use a left join where some rows have no match | Unmatched rows are kept, with empty columns | Not run |
| `BLD-1-AC3` | Preview a step | First 100 rows, or fewer, shown | Not run |
| `BLD-1-AC4` | Save a SQL step containing `DROP TABLE` or `INSERT` | Refused when saved, with a clear message | Not run |
| `BLD-1-AC5` | As an author for whom column C of table T is masked, run a pipeline that copies C | C is masked in the output table | Not run |
| `BLD-1-AC6` | Save a pipeline reading T, then add a masking policy on a column it copies, then run it | The new run's output has the column masked | Not run |
| `BLD-1-AC7` | Ask the assistant for the pipeline of AC1 | The same pipeline is created as a draft | Not run |
| `BLD-1-AC8` | As a user without `pipeline:write`, try to create a pipeline | Refused with an honest message | Not run |
| `BLD-1-AC9` | Pause the pipeline | It stops running; the output table stays | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
