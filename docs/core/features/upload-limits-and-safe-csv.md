# Upload limits and CSV files that cannot run formulas

| | |
| --- | --- |
| Module | Sources (file upload), Query Studio and every table's "Export CSV" |
| Backlog | `SEC-17` |
| Spec | `docs/core/specs/sec-17.md` (target numbers; every *(proposed)* one is signed under Decisions) |
| Status | Draft. Decisions not signed; the defaults below are being built |
| Plan | `docs/superpowers/plans/2026-10-08-sec-17-upload-hardening.md` |

## Problem

Three gaps, each confirmed in the code on 2026-10-08:

- A file's header row decides how many columns its table gets, and nothing
  bounds it. A 50 MB file whose header is nothing but commas asks the load
  job for about fifty million columns.
- Nothing bounds how many files are being received, or how many loads are
  running, at one time. Each file being received is held in the API's memory
  (up to 50 MB), and each load is an orchestrator run.
- A CSV file the console writes carries cells as they are. A cell that starts
  with `=`, `+`, `-` or `@` is a formula to a spreadsheet, so data one person
  loaded can run a formula on the machine of the person who opens the export.

## What the user can do when this is done

1. Upload and load a file with up to 1,000 columns, as today.
2. See "The file has more than 1,000 columns." on the Check step for a file
   with more, and still change the delimiter or the header row there (a wrong
   delimiter guess must not lock a good file out).
3. Get the same sentence, and no load, if the load is asked for directly.
4. See the same sentence as the load's failure reason if a load of such a
   file is ever started some other way.
5. Have four of their own files being received at once, and four of their
   own files being loaded at once; a fifth is refused with "Too many uploads
   are in progress. Try again in a moment." and can be sent again.
6. Across the installation, sixteen files being received and sixteen being
   loaded at once; the seventeenth gets the same sentence.
7. Open any CSV file the console wrote (a table's "Export CSV", a dashboard
   tile's download, Query Studio's "Download CSV") in a spreadsheet and see a
   cell such as `=HYPERLINK(...)` as text, with a leading apostrophe.
8. Still see numbers as numbers in those files: `-5` and `+3.2e4` are not
   touched.

## Not included

- A limit on rows (already 2,000,000) or on file size (already 50 MB).
- A queue. A refused upload is sent again by the person; nothing waits.
- Parquet downloads: a Parquet file has typed columns and runs no formula.
- Limits that hold across several API processes for files being received
  (see Limits).
- Excel and Parquet uploads, which are not on `main` yet. Their branch must
  apply the column cap to its own preview when it merges (the load job's cap
  already covers every format, because every format reaches the job as
  delimited text).

## Asking the assistant

No. The assistant does not upload files or write CSV files.

## Decisions

Until signed, the default is used.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | Columns per file | 1,000 *(proposed in the spec)* | |
| D2 | Uploads in progress per user | 4 *(proposed in the spec)*; `UPLOAD_MAX_CONCURRENT_PER_USER` | |
| D3 | Uploads in progress per installation | 16 *(proposed in the spec)*; `UPLOAD_MAX_CONCURRENT` | |
| D4 | What "in progress" counts | Two things, each against the same two numbers: files being received by the API, and loads running | |
| D5 | Over the limit | Refused with 429 and `Retry-After: 5`; nothing waits | |
| D6 | Whose load it is | The person who uploaded the file (the only person the upload records), not the person who pressed Load | |
| D7 | Which cells get the apostrophe | A cell whose first character is `=`, `+`, `-`, `@`, a tab or a carriage return, unless the whole cell is a plain number | |
| D8 | Header cells | Treated like any other cell | |

## Limits

- The count of files being received is kept in the API process. With several
  API processes each has its own count. The count of loads is in the
  database and holds across processes.
- A CSV file with the apostrophe no longer reads back as the same bytes: a
  program that reads the export gets `'=A1`, not `=A1`. That is the price of
  the file being safe to open; Parquet is the format for exact data.
- A number exempted by D7 is decided by its text alone. A cell such as
  `-1+1` is not a number and gets the apostrophe.

## Acceptance checklist

Run by the product owner on a running console. A step not performed is never
a pass.

- [ ] A CSV file with 1,000 columns previews and loads
- [ ] A CSV file with 1,001 columns shows the sentence on the Check step; changing the delimiter to one that gives fewer columns shows the preview again
- [ ] A file whose header is 50 MB of commas is accepted as a file and refused at the Check step within a few seconds; no load starts
- [ ] `POST /api/uploads/{id}/ingest` for that file answers 400 with the sentence
- [ ] With four files of one user loading, that user's fifth load answers 429 with `Retry-After`; another user's load starts
- [ ] Five simultaneous `POST /api/uploads` by one user: four are accepted, one answers 429
- [ ] A table holding `=HYPERLINK("http://example.invalid","x")` exported from Query Studio (both the table menu's "Export CSV" and "Download CSV") opens in a spreadsheet as text
- [ ] The same export shows `-5` as the number -5
- [ ] A user without `connector:manage` is refused on every upload route, as before
