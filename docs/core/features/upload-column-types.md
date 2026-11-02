# Column types on upload

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-18` |
| Kind | Feature |
| Spec | `docs/core/specs/data-18.md` (one *(proposed)* number: the 10,000-row sample, decision 1) |
| Status | Building (#186, not merged). Product owner's browser check passed on 2026-10-23 on a QA stack built from #186's CI images (`aa9f5fac`): several files at once, a resumed upload, column types, one table from several files and the refusal of a file that differs, the Excel 50 MB limit, a load into Bronze seen in Data Explorer (10 files, 589K rows, typed columns), a reload replacing the rows, a value that does not fit stopping the load, and the JSON, Parquet, gzip and zip formats; the acceptance checklist runs after merge |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md` (Phase 3 master plan, slice 3) |
| Started | 2026-10-16; also in `BACKLOG.md` Dates |
| Shipped | |
| Evidence | #186 (open) |

## Problem

Every uploaded column is stored as text (ADR 0014 decision 4), so numbers and
dates cannot be summed, compared or filtered as such without a pipeline that
casts them. A Parquet file's own types are shown in the preview and then
dropped. Column names are cleaned only when the file is loaded, so the user
never sees the final names first. Databricks and Snowflake both guess the
types and let the user change names and types before loading.

## What the user can do when this is done

1. See each column's guessed type (integer, decimal, boolean, date, timestamp or text) in the preview.
2. Change any column's name and type before loading.
3. Answer "day first or month first?" when a date column could be read either way.
4. See the rows whose values do not fit the chosen type (up to 100) before loading.
5. Choose what happens to such values: load them as empty, or stop the load.
6. Find the loaded table's columns with those types in the Catalog and in queries.

## Not included

- Changing a column's type after the table is loaded.
- Types other than the six named (no time-of-day, interval or binary type).
- Spreading a nested JSON or Avro value into columns (`DATA-17` decision 11).

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is the AI
team's (`AI-16`).

## Decisions

Signed on 2026-10-16: the product owner chose every default ("jalankan default").
Each default was the planner's recommendation.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | How much of the file the guess reads | The first 10,000 rows *(proposed, from the spec)*, or the first 16 MB if that comes first. The preview says how many rows it looked at | 2026-10-16, product owner: the default |
| 2 | The types offered | Text, Integer (whole numbers, 64-bit), Decimal (exact: 38 digits, 9 of them after the point), Boolean, Date, Timestamp | 2026-10-16, product owner: the default |
| 3 | What counts as a number or a yes/no | Integer: an optional sign and digits, without a leading zero, so `007` and postcodes stay text. Decimal: digits with one `.`; `1,234` stays text. Boolean: `true`/`false`, `yes`/`no`, `y`/`n`, `t`/`f` in any case; `1`/`0` are integers. Empty cells do not count toward the guess | 2026-10-16, product owner: the default |
| 4 | Dates | ISO `2026-10-16`; `16/10/2026` and `10/16/2026`, with `/`, `-` or `.`. When every value could be either day first or month first, the preview asks before Load can be pressed. A column with values that contradict each other stays text | 2026-10-16, product owner: the default |
| 5 | Timestamps | ISO with `T` or a space, with or without seconds and fractions, and the day-first and month-first forms followed by a time. When every value has a time zone (`Z` or `+07:00`) it is stored as a moment in UTC; when none has, as a timestamp without zone; a mix stays text | 2026-10-16, product owner: the default |
| 6 | Values that do not fit | The preview checks the rows it read at once and lists up to 100 failing rows. Before Load the user picks "load them as empty" or "stop the load"; **stop** is pre-selected. The load counts over the whole file: "stop" writes nothing and shows the count and the first 100; "empty" loads and shows the same report | 2026-10-16, product owner: the default |
| 7 | Empty cells | Empty (null) in every column except text, which keeps an empty text as today | 2026-10-16, product owner: the default |
| 8 | Column names | Pre-filled with today's cleaning rules, so nothing changes for a user who edits nothing. A name the user types must be lowercase letters, digits and `_`, start with a letter, be at most 128 characters, be unique, and not start with `_dlt` | 2026-10-16, product owner: the default |
| 9 | Adding to a table that already exists | The names and types come from the table and cannot be changed; a file whose columns or values do not fit is listed before Load. Replacing a table may change its types | 2026-10-16, product owner: the default |
| 10 | Files that carry their own types | Excel number, date and yes/no cells, and Parquet, Avro and JSON types, are the starting guess. *Beyond the spec:* Parquet and Avro floating-point columns keep their floating-point type, shown as "Floating point (from the file)"; the user may change them to Decimal or Text | 2026-10-16, product owner: the default |
| 11 | API callers that send no column list | Every column text, as today, so scripts and gates keep working | 2026-10-16, product owner: the default |
| 12 | ADR 0014 decision 4 ("no type inference", signed 2026-10-02) | Replaced by decisions 1–11; the ADR gets an amendment | 2026-10-16, product owner: the default |

## Limits to tell a customer

- The guess reads the first 10,000 rows (or 16 MB); a bad value further down is caught by the load, which follows the "empty or stop" choice.
- A decimal keeps 9 digits after the point; a value with more does not fit.
- Replacing an uploaded table with different types can break a pipeline that reads it; its next run fails and says why.
- Adding to an existing table keeps that table's names and types.
- The size of a typed load is the same as for any upload: 2 GB, one load at a time measured (`docs/plans/DATA-17-RESULT.md`); the harness sends no column list, so no typed load was measured at full size.
- A query through the console shows a NULL as an empty string (`DATA-29`); use `IS NULL` to tell them apart.

## Spec targets, row by row

Against `docs/core/specs/data-18.md`, as built in #186. "Met" means built and covered by tests and the `g9-upload` gate on the pull request; the acceptance checklist below is not run.

| Spec row | Target | Result |
| --- | --- | --- |
| Type detection | Integer, decimal, boolean, date, timestamp, text from a sample of 10,000 rows *(proposed)* | Met (decision 1; the sample is also cut at 16 MB) |
| Edit before loading | Name and type per column in the preview | Met (decision 8) |
| Bad values | Counted, first 100 listed, choose null or stop | Met for the load, workbooks included since `DATA-31` |
| Date formats | ISO, dd/mm/yyyy, mm/dd/yyyy; ask when ambiguous | Met (decision 4) |

No spec target is left unmet, so there is no "Spec targets this feature does not meet" section.

## What changed in the build

- **Decision 12 (ADR 0014 decision 4).** Replaced as signed; the amendment is in `docs/adr/0014-file-upload.md`.
- **Decision 9 (replacing a table).** `PR #186 slice 3 review BLOCKER 1`: a replace that changes a column's type first purged the table, so a failure in between lost its data. The type change and the new rows are now one Iceberg commit; a failed load changes nothing.
- **Decision 10 (files that carry their own types).** Floating-point columns of Parquet and Avro keep their type, shown as "Floating point (from the file)" (beyond the spec, as signed).
- **Decision 6 (values that do not fit).** The comparison of several files reports a type conflict only for a column the first file's own sample fits, so two identical files never conflict (`PR #186 slice 5 review`, `BLOCKER 5`).

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-18-AC1` | Upload a CSV with an ISO date column and load it | The preview guesses Date; the Catalog shows the column as a date; a query can filter it by date range | Not run |
| `DATA-18-AC2` | In the preview, change a column holding some words to Integer | The failing rows are listed (up to 100) before loading | Not run |
| `DATA-18-AC3` | Sign in as a user without the connector-manage permission and try to upload; then make a load fail | The upload is refused for the permission; the failed load shows an honest reason | Not run |
| `DATA-18-AC4` | Upload a file whose dates are all like `01/02/2026` | The preview asks "day first or month first?" and Load stays off until answered | Not run |
| `DATA-18-AC5` | Upload a file whose row 20,000 has `abc` in an integer column; keep "stop the load" | The load stops, the table is unchanged, and the count and row are shown | Not run |
| `DATA-18-AC6` | The same file with "load them as empty" | The table loads with that cell empty, and the report shows one value made empty | Not run |
| `DATA-18-AC7` | Rename a column in the preview and load | The table has the new name | Not run |
| `DATA-18-AC8` | Add a file to an existing table where one column holds text the table stores as integers | The mismatch is listed before loading | Not run |
| `DATA-18-AC9` | Upload a file with `007` in one column and `1`/`0` in another | The first is guessed Text, the second Integer | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
