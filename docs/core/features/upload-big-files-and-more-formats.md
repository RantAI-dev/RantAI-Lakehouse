# Upload big files, several at once, in more formats

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-17` |
| Kind | Feature |
| Spec | `docs/core/specs/data-17.md` (no *(proposed)* number; the 2 GB figure ships only once measured, decision 3) |
| Status | Building (Excel and Parquet merged in #76; the rest in #186) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md` (Phase 3 master plan, slices 1, 2, 4, 5 and 6) |
| Started | 2026-10-07 (Excel); also in `BACKLOG.md` Dates |
| Shipped | |
| Evidence | #76 (Excel and Parquet) |

## Problem

Upload takes one file of up to 50 MB per request. The whole file is held in
memory by the console's proxy, the API and the load job, so the limit cannot
simply be raised, and the proxy cuts a larger request instead of refusing it.
JSON, JSON Lines, Avro and compressed files are refused, a drop of several
files keeps only the first, and a dropped connection means starting again.
Databricks takes 10 files up to 2 GB in total; Snowflake 250 files of 250 MB.

## What the user can do when this is done

1. Upload CSV, TSV, JSON, JSON Lines, Avro, Parquet or Excel (one chosen sheet).
2. Upload a gzipped text or JSON file, or a zip holding up to 10 files of one format.
3. Pick or drop up to 10 files at once and watch a progress bar per file.
4. Keep uploading after the connection drops: the upload continues by itself when the connection comes back.
5. Upload up to 2 GB in one go (once the measurement in decision 3 allows it).
6. Load several files with the same columns into one table; files whose columns differ are listed before loading.
7. See a clear message, before any byte is sent, when the files are too big.

## Not included

- Files arriving on a schedule (a bucket or SFTP connector).
- Resuming after the page is reloaded or the browser is closed (decision 7).
- Several sheets of one workbook in one load; `.xlsm`, `.xlsb`, `.ods`.
- ORC and XML (Snowflake has them; the spec does not).
- Nested JSON or Avro values spread into child tables (decision 11).
- Uploading straight from the browser to storage (the files go through the API, as today).

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is the AI
team's (`AI-16`).

## Decisions

Signed on 2026-10-16: the product owner chose every default ("jalankan default").
Each default was the planner's recommendation.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | What "GB" means | GiB, as the product's "50 MB" already means 50 MiB: the cap is 2,147,483,648 bytes. The acceptance rows give file sizes in bytes so no reader has to guess | 2026-10-16, product owner: the default |
| 2 | What the 2 GB counts | The total of one load (all its files), and the uncompressed size for gzip and zip, so a small archive cannot unpack into more than was measured | 2026-10-16, product owner: the default |
| 3 | When the 2 GB number ships | Only at the value measured in `docs/plans/DATA-17-RESULT.md` (peak memory of the console's proxy, the API and the load job, for a 2 GiB load). If memory is not flat in file size, a lower cap ships and the shortfall is reported, never hidden | 2026-10-16, product owner: the default |
| 4 | Several files into one table | One load = one table and one format. Columns are matched by name after cleaning; their order may differ. A file whose columns differ is listed with its missing and extra columns, and Load stays off until it is removed (as Databricks' upload does) | 2026-10-16, product owner: the default |
| 5 | More than 10 files picked | The first 10 are kept and the page says so (as today's "one file at a time: the first one was taken") | 2026-10-16, product owner: the default |
| 6 | What "in parallel" means | All files progress together, with up to 4 pieces in flight per user: the `SEC-17` per-user limit (`UPLOAD_MAX_CONCURRENT_PER_USER`, 4) stays as signed | 2026-10-16, product owner: the default |
| 7 | How far resuming goes | Automatic while the page stays open: a dropped connection or an API restart pauses the file and it continues on its own. After a page reload the file is uploaded again | 2026-10-16, product owner: the default |
| 8 | Unfinished uploads | Removed with their pieces after 24 hours *(proposed)* | 2026-10-16, product owner: the default |
| 9 | Excel size | Stays at 50 MB per workbook. A workbook is read whole to list its sheets, and `SEC-22` bounds that reading; Databricks' upload does not take Excel at all | 2026-10-16, product owner: the default |
| 10 | The 2,000,000-row cap | Removed once the measurement shows memory flat in row count; otherwise it stays and the reason is written down | 2026-10-16, product owner: the default |
| 11 | JSON shapes | A top-level array of objects (`.json`) or one object per line (`.jsonl`, `.ndjson`). Object keys become columns; a key missing from a row is empty (null). A nested object or array is kept as JSON text in one column; no child tables | 2026-10-16, product owner: the default |
| 12 | Avro | Object container files (`.avro`). Records, arrays and maps are kept as JSON text; a union with null is a column that may be empty | 2026-10-16, product owner: the default |
| 13 | Compressed files | gzip: one file of CSV, TSV, text, JSON or JSON Lines (`.csv.gz` and so on). zip: up to 10 files of one supported format other than Excel; each file in it counts toward the 10; folders are ignored; an archive inside an archive is refused | 2026-10-16, product owner: the default |
| 14 | The single-request upload route | `POST /api/uploads` stays for API clients and scripts, at 50 MB | 2026-10-16, product owner: the default |
| 15 | Excel and Parquet, already built | Sign decisions 10 and 11 of `upload-file.md` (built in #76, never signed) | 2026-10-16, product owner: the default |

## Limits to tell a customer

- Up to 10 files and 2 GB per upload, once measured (decision 3); Excel workbooks up to 50 MB.
- An upload continues after a dropped connection only while its page stays open.
- Several files load into one table only when their columns match.
- Nested JSON and Avro values are kept as JSON text, not spread into columns.
- An unfinished upload is removed after 24 hours.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-17-AC1` | Upload 10 Parquet files with the same columns, 2,040,000,000 bytes in total (1.9 GiB), into one new table | All 10 show progress at once; the load finishes; the table's row count is the sum of the files' row counts | Not run |
| `DATA-17-AC2` | Upload an `.xlsx` with three sheets | The Check step asks which sheet, and loads the chosen one | Not run |
| `DATA-17-AC3` | Start a 500 MB upload, turn the network off halfway for a minute, turn it back on | The file shows it is waiting for the connection, then continues from where it was and loads; the row count is right | Not run |
| `DATA-17-AC4` | Pick files totalling 2,255,000,000 bytes (2.1 GiB) | Refused with a message naming the 2 GB limit, before any byte is sent | Not run |
| `DATA-17-AC5` | Sign in as a user without the connector-manage permission and open the upload page; then upload a file that cannot be read | The upload is refused for the permission; the unreadable file shows the reason, not a bare "failed" | Not run |
| `DATA-17-AC6` | Upload one JSON (array), one JSON Lines and one Avro file, each into its own table | Each loads; a nested value shows as JSON text in its column | Not run |
| `DATA-17-AC7` | Upload a `.csv.gz`, and a zip holding three CSVs with the same columns | The first loads like the plain CSV; the zip's three files load into one table | Not run |
| `DATA-17-AC8` | Pick two CSVs where the second has one extra column | Before loading, the second file is listed with its extra column, and Load is off until it is removed | Not run |
| `DATA-17-AC9` | Pick 11 files | The first 10 are kept and the page says the 11th was not taken | Not run |
| `DATA-17-AC10` | Leave an upload unfinished (close the tab halfway) and come back after 24 hours | It no longer appears in the upload list (operator: its pieces are gone from storage) | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
