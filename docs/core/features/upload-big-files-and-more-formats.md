# Upload big files, several at once, in more formats

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-17` |
| Kind | Feature |
| Spec | `docs/core/specs/data-17.md` (no *(proposed)* number; the 2 GB figure ships only once measured, decision 3) |
| Status | Building (Excel and Parquet merged in #76; the rest in #186, not merged). Product owner's browser check passed on 2026-10-23 on a QA stack built from #186's CI images (`aa9f5fac`): several files at once, a resumed upload, column types, one table from several files and the refusal of a file that differs, the Excel 50 MB limit, a load into Bronze seen in Data Explorer (10 files, 589K rows, typed columns), a reload replacing the rows, a value that does not fit stopping the load, and the JSON, Parquet, gzip and zip formats; the acceptance checklist runs after merge |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md` (Phase 3 master plan, slices 1, 2, 4, 5 and 6) |
| Started | 2026-10-07 (Excel); also in `BACKLOG.md` Dates |
| Shipped | |
| Evidence | #76 (Excel and Parquet); #186 (the rest, open); `docs/plans/DATA-17-RESULT.md` (the measurement the 2 GB cap is set from) |

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
| 9 | Excel size | Stays at 50 MB per workbook. A workbook is read whole to list its sheets, as today; Databricks' upload does not take Excel at all | 2026-10-16, product owner: the default |
| 10 | The 2,000,000-row cap | Removed once the measurement shows memory flat in row count; otherwise it stays and the reason is written down | 2026-10-16, product owner: the default |
| 11 | JSON shapes | A top-level array of objects (`.json`) or one object per line (`.jsonl`, `.ndjson`). Object keys become columns; a key missing from a row is empty (null). A nested object or array is kept as JSON text in one column; no child tables | 2026-10-16, product owner: the default |
| 12 | Avro | Object container files (`.avro`). Records, arrays and maps are kept as JSON text; a union with null is a column that may be empty | 2026-10-16, product owner: the default |
| 13 | Compressed files | gzip: one file of CSV, TSV, text, JSON or JSON Lines (`.csv.gz` and so on). zip: up to 10 files of one supported format other than Excel and Parquet; each file in it counts toward the 10; folders are ignored; an archive inside an archive is refused | 2026-10-16, product owner: the default. Amended 2026-10-21, product owner: a Parquet footer cannot be read by range inside a deflated zip entry, so Parquet stays out of a zip (built later as its own slice, `DATA-28`) |
| 14 | The single-request upload route | `POST /api/uploads` stays for API clients and scripts, at 50 MB | 2026-10-16, product owner: the default |
| 15 | Excel and Parquet, already built | Sign decisions 10 and 11 of `upload-file.md` (built in #76, never signed) | 2026-10-16, product owner: the default |

## Limits to tell a customer

- Up to 10 files and 2 GB (GiB: 2,147,483,648 bytes) per upload, counting all the files of one load together. A gzip or zip counts its unpacked size. Excel workbooks stay at 50 MB each. There is no row cap any more: the old 2,000,000-row limit was removed because the job's memory was measured not to follow the size of the file (`docs/plans/DATA-17-RESULT.md`).
- The 2 GB figure is measured for **one** load at a time, on one 16 GB GitHub-hosted runner: a 2 GiB load of CSV, JSON Lines, a gzipped CSV or ten Parquet files in one load grew the load job by 0.9 to 2.3 GiB, and finished in under two minutes (CSV 107 s). Those four are the formats the measurement covered. Not measured, and so not promised:
  - a JSON array, an Avro file, a zip, or a single Parquet file at large sizes (they were measured before the streaming fix, not after it);
  - two 2 GB loads at the same time (the installation allows up to 16 loads at once, `SEC-17`);
  - a Parquet file whose single row group is larger than memory (the reader decodes one row group whole);
  - any machine other than that runner.
- A zip may not hold a Parquet file (decision 13; `DATA-28`).
- An Avro file must be **uncompressed** (the null codec). A compressed container is refused, on both sides, with "This Avro file is compressed. Save it with no compression.": the Avro readers inflate a whole block with no limit on the output (apache-avro 0.21 in the API, fastavro 1.13.1 in the job), so a few MB of a compressed file of zeros would ask for gigabytes of memory. Uncompressed Avro files are the norm for an upload (`PR #186 review BLOCKER 1`).
- An upload continues after a dropped connection only while its page stays open.
- Several files load into one table only when their columns match.
- Nested JSON and Avro values are kept as JSON text, not spread into columns.
- An unfinished upload is removed after 24 hours. A person can have at most 20 uploads being received at once.
- A workbook is still read whole by the API (its memory bounds are `SEC-22`, not this feature).

## Spec targets, row by row

Against `docs/core/specs/data-17.md`, as built in #186. "Met" means built and covered by tests and the `g9-upload` gate on the pull request; the acceptance checklist below is not run.

| Spec row | Target | Result |
| --- | --- | --- |
| Formats | CSV, TSV, JSON, JSON Lines, Avro, Parquet, Excel (one sheet) | Met. JSON is a top-level array of objects (decision 11); Avro is an object container file whose top level is a record and which is uncompressed (the null codec: a compressed container is refused, see the limits above) |
| Files per upload | Up to 10, in parallel | Met. Up to 10; up to 4 pieces in flight per user (decision 6) |
| Size | Up to 2 GB, memory measured in a RESULT document | Met for one load at a time, see the limits above and `docs/plans/DATA-17-RESULT.md`. Time was not a target; it grew by 30 to 60 percent with the streaming fix |
| Upload method | Resumable chunks, a progress bar per file | Met, with the reach of decision 7: resuming works while the page stays open |
| Compressed files | gzip and zip | Met, except a Parquet file inside a zip (decision 13, amended; `DATA-28`) |
| Several files into one table | Same columns into one table; mismatches listed before loading | Met |

No spec target is left unmet, so there is no "Spec targets this feature does not meet" section. The part of the size target that was not measured is in the limits above and in `BACKLOG.md` (`DATA-33`, `DATA-34`).

## What changed in the build

- **Decision 3 (when the 2 GB ships).** The first full-size measurement (`DATA-17-RESULT.md`, run 39903340840) showed the load job's memory growing with the file, 3 to 5 times its size (CSV 2 GiB: +9.75 GiB), so the planned streaming of the job did not hold (`PR #186 review BLOCKER 6`). The product owner decided on 2026-10-23 to fix the streaming first and set the cap from a second measurement rather than ship a smaller cap. The write step of the Iceberg load was the cause; it now writes one transaction in groups of up to 32 MiB (`adapters/iceberg_stream.py`). The second measurement (run 39916318666) is the one the cap is set from, and it met the target, so 2 GiB ships.
- **Decision 10 (the row cap).** Removed. The measured growth followed bytes, not rows, and is now flat in both. The failure reason `TOO_MANY_ROWS` stays in the fixed list so that an older failed upload keeps its message; it is no longer raised.
- **Decision 13 (compressed files).** Parquet inside a zip is out, signed 2026-10-21 (`DATA-28`), because a footer cannot be read by range inside a deflated entry.
- **Decision 6 (parallel).** `PR #186 slice 1 review SHOULD-FIX 1`: a user may hold at most 20 open upload sessions (429 past that), so a client cannot open sessions without end before the 24 hour sweep.
- **The old route.** `POST /api/uploads` keeps its 50 MB cap (decision 14). The 2 GB applies to the chunked sessions; the unpacked-size check has its own 2 GiB constant so the two do not disagree.

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
