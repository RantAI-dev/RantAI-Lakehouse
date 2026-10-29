# Results you can use in Query Studio

| | |
| --- | --- |
| Module | Query Studio |
| Backlog | `QS-2` |
| Kind | Feature |
| Spec | `docs/core/specs/qs-2.md` |
| Status | Building |
| Priority | P2 |
| Owner | Held in the base. |
| Acceptor | Held in the base. |
| Started | 2026-10-28; also in `BACKLOG.md` Dates |
| Shipped | Not yet |
| Evidence | |
| Plan | Plan files are not kept in the repository. |

## Problem

`BI-5` (PR #245) already gave Query Studio an administrator-set row limit applied in the engine (default 2,000,
100 to 100,000, `lakehouse-store/src/query_limits.rs:30-37`), an administrator-set time limit (10 s to 30 min,
`routes/query_limits.rs:47-51`) and a Stop that kills the query on ClickHouse (`routes/query_cancel.rs`). What is
still missing:

- The result has no size cap in bytes, and the "of M rows" in the cut banner is not the engine's total
  (`query-results-section.tsx`; `rowCount` counts the rows that arrived).
- The grid sorts and scrolls (TanStack Table, `src/components/data-table/data-table.tsx`) but has no per-column
  filter, no column statistics, no cell inspector and no copy.
- The only download button is a browser CSV of the rows already shown (`query-results-section.tsx`).
  The server route `GET /api/query/run/{id}/download` exists with no button: CSV or Parquet, cut at 10,000 rows
  (`routes/query.rs`), buffered in memory (`ChClient::raw_bytes`), and it ignores the role download switch
  that dashboard exports honour (`routes/settings.rs:420`).
- Nothing shows how long a running query has taken.

## What the user can do when this is done

1. Run a query and see up to 64,000 rows or 10 MB of row data, whichever comes first, with a banner that says
   which limit cut the result.
2. Filter each column of the shown rows, see a column's statistics (empty cells, distinct values, min and max, a
   histogram), open a long or JSON value in an inspector, and copy the rows as TSV or one cell's value.
3. Download the full result from the server as CSV, TSV, Excel or Parquet, up to 5 GB, with the same masks and row
   filters the run had.
4. See the time a running query has taken so far.

## Not included

- Rows read while a query runs. It needs the run to stream its progress, which it does not today. Later part.
- Stop on Trino (as in `BI-5`), and a download of a Trino run.
- A download of more than 5 GB, and an Excel file with more than one sheet.

## Asking the assistant

No new tool. The chat's `run_sql` keeps its own limit. Downloads are a console action.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Query Studio's row limit default goes from 2,000 to 64,000 (Databricks). The administrator's range stays 100 to 100,000 (`BI-5`) | 64,000 | Owner, 2026-10-28 |
| 2 | A result is also cut at 10 MB of row data (Databricks), counted on the server before it is sent. The banner says which limit cut it | 10 MB | Owner, 2026-10-28 |
| 3 | The time limit stays `BI-5`'s 10 s to 30 min. The spec's 2 hours was a planner's number. A query over 30 minutes belongs in a pipeline, not the editor | 30 min max | Owner, 2026-10-28 |
| 4 | Filter, statistics and histogram work on the rows shown, not the whole result, and the panel says "of the N rows shown". Exact statistics over the full table stay on the catalog page (`asset-columns.tsx`) | none | Owner, 2026-10-28 |
| 5 | Download uses the existing route `GET /api/query/run/{id}/download` (the run in History), so there is no new route and no `POLICY_TABLE` change. It re-runs the SQL with the caller's current masks and row filters, as today | none | Owner, 2026-10-28 |
| 6 | Download formats: CSV, TSV, Excel, Parquet. CSV, TSV and Excel are written by the API from streamed rows, with the formula guard dashboard exports use (`csv_export::neutralise`). Parquet is written by ClickHouse (`FORMAT Parquet`), so no new dependency | none | Owner, 2026-10-28 |
| 7 | A download stops at 5 GB (Databricks). Excel stops at 1,048,575 data rows (1,048,576 with the header), the sheet's limit. Either cut is said in the file name and in a message, never silent | 5 GB, 1,048,575 data rows | Owner, 2026-10-28 |
| 8 | A download honours the role download switch (`DOWNLOADS_OFF`), as dashboard exports do, and the run's time limit. A download in progress can be stopped with Stop | none | Owner, 2026-10-28 |
| 9 | Elapsed time is counted in the browser from the moment Run is pressed | none | Owner, 2026-10-28 |

## Limits to tell a customer

- Statistics and filters in the grid cover the rows shown, up to 64,000, not the whole result. A numeric column's
  histogram has ten equal-width buckets, a text column's the ten most frequent values.
- A result is cut at 64,000 rows or at 10 MB of row data; the banner says which limit cut it. The product's "MB"
  is MiB: 10 MB is 10,485,760 bytes.
- A download re-runs the query. If the data changed since the run, the file shows the new data.
- Downloads stop at 5 GB, and Excel at 1,048,575 data rows (1,048,576 with the header).
- A comma or tab file over 5 GB is cut at the last whole row that fits, and the file's name says how many rows it
  holds. A Parquet result over 5 GB is refused whole, never sent half.
- The grid copies the rows it shows as TSV, and one cell's value from the inspector; a selection of several cells
  cannot be copied at once.
- Downloads run on ClickHouse only; the button is off for a Trino result.
- Stop and the time limit do not reach a Trino query.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `QS-2-AC1` | Run a query returning 100,000 rows | 64,000 rows shown, a banner says the result was cut at 64,000 rows | Not run |
| `QS-2-AC2` | Download that run as Parquet | The file holds all 100,000 rows | Not run |
| `QS-2-AC3` | Run a query with wide text columns that passes 10 MB | The banner says it was cut at 10 MB | Not run |
| `QS-2-AC4` | Filter a column, open its statistics, inspect a JSON cell, copy the rows as TSV | Each works on the shown rows, and the statistics say "of the N rows shown" | Not run |
| `QS-2-AC5` | Download as CSV, TSV and Excel a query on a masked column | Every file shows the masked value | Not run |
| `QS-2-AC6` | As a role with downloads switched off, download | Refused with a plain message | Not run |
| `QS-2-AC7` | Start a large download, press Stop | The query ends on the engine (`system.processes`) within 5 seconds | Not run |
| `QS-2-AC8` | A user without `query:read` downloads, and a download fails on the engine | Refused; the failure shows a fixed message, not the engine's text (principles 2 and 4) | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
