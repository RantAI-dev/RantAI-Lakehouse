# Upload a file into a raw table

**Refreshed on 2026-11-02 for QA-1.** Pull request #186 (merged 2026-10-26)
changed what this page describes: column types are now guessed and can be
changed before loading (`DATA-18`), and files go up in pieces, up to 2 GB
(`DATA-17`). The claims and the checklist below were rewritten against the
code on `main` today.

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `DATA-9` |
| Status | In Acceptance. Merged: #71, #73 (CSV and text), #76 (Excel and Parquet) and #186 (more formats, several files at once, the size, column types; merged 2026-10-26). The checklist below has not been run. Decisions signed 2026-10-02; Excel added 2026-10-07 and Parquet 2026-10-08 (decisions 10 and 11 signed 2026-10-16); the formats, the size and the column types of #186 are signed on their own pages (decisions 12 to 15 below, `upload-big-files-and-more-formats.md` and `upload-column-types.md`) |
| Plan | `docs/superpowers/plans/2026-10-02-upload-file.md`; `docs/superpowers/plans/2026-10-16-data-phase-3-uploads.md` (the #186 slices) |

## Problem

Data reaches the lakehouse only through a registered source: a database, a
bucket, an API. A person who holds a file, exported from an ERP, a
spreadsheet or a vendor portal, has no way to bring it in from the console.
Before this feature, someone converted the file and loaded it into a database
by hand, on the server (`PRODUCT.md` section 2: "Upload a file from the
console: Missing").

The feature is built and merged. What follows describes what it does on
`main` today, not what it was planned to do.

## What the user can do when this is done

1. On Sources, press "Upload file". The same page is offered from the first
   step of "New Connector".
2. Choose up to ten files at once and send them: CSV, TSV, JSON, JSON Lines,
   Avro, Parquet, an Excel workbook (`.xls`, `.xlsx`), a `.gz` of one text
   or JSON file, or a `.zip` of up to ten files of one format. The page sends
   every file in pieces; up to 2 GB in all for one upload. An Excel workbook
   keeps its own limit of 50 MB, because it is read whole.
3. See what the file looks like before anything is loaded: the encoding,
   the delimiter and the header row the system detected, the column names,
   and the first 20 rows. For a workbook: the sheet (the first visible one
   to begin with), the header row, the column names and the first 20 rows.
   For a Parquet file: the column names and, under each, the type the file
   declares ("was: int64"), the number of rows the file states and the first
   20 rows; there is nothing to choose. For a JSON, Avro, gzip or zip file:
   the format it is read as, and, for a zip, the files it holds.
4. Correct any of the three and see the preview change. For a workbook,
   choose another sheet (a hidden one is marked and can be chosen) and see
   that sheet.
5. See every column's name and its type. The type is guessed from the first
   10,000 rows (or the first 16 MB of the file, whichever comes first), and
   both the name and the type can be changed before the file is loaded. A
   column of dates that could be read either way asks "Day first or month
   first?" and Load stays off until it is answered.
6. See the values that do not fit the chosen type (the count, and up to 100
   of them), and choose what happens to them: load them as empty, or stop
   the load. Stop is the pre-selected choice.
7. Name the raw table the file becomes.
8. Load it, and watch the load go from "Loading" to "Loaded" with the number
   of rows, or to "Failed" with a reason.
9. Open the new table in Data Explorer from the result.
10. Load a later file into a table an earlier upload created: replace its
    rows (the default), or add to them.
11. Load several files into one table when their columns match; a file whose
    columns differ is listed before the load and keeps Load switched off.
12. Be told when the same file was uploaded before.
13. See every upload of their tenant under "Uploaded files" on Sources, with
    its status, table, rows, who uploaded it and when.
14. Delete an upload. The file is removed; the table it became stays.

A user without the permission to manage connectors cannot upload, load or
delete, and does not see the tenant's uploads either. A user with it sees
every upload of their tenant, whoever made it.

## Not included

- `.xlsm`, `.xlsb`, `.ods` and other archives. They are refused with a
  message that says what to do, never half-read. (JSON, JSON Lines, Avro,
  gzip and zip are read, since #186.)
- Parquet columns of binary or nested type (the file is refused, naming the
  first such column).
- Loading several sheets of a workbook in one upload (load the workbook
  again with another sheet), calculating formulas, and password-protected
  workbooks (refused, with the reason).
- Files over 2 GB in one upload, and an Excel workbook over 50 MB. There is
  no row cap any more.
- Files that arrive on a schedule. A bucket or SFTP source does that.
- Changing a column's type after the table is loaded.
- Turning the table into Silver or Gold. That is the pipeline builder's job.
- Removing the raw table from the upload list.
- Scanning files for malware.

## Asking the assistant

Not in this version. A file cannot be attached to a chat, and loading an
uploaded file changes which data exists, so it is left for a follow-up with
its own approval rule. The assistant can already describe and query the
table once it is loaded, with the column types the load chose.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Which files are accepted | Delimited text: UTF-8 or UTF-16; comma, semicolon, tab or pipe | Product owner, 2026-10-02 |
| 10 | Excel workbooks | `.xls` and `.xlsx`, one sheet per load, converted to text in the API as described under "How a workbook is read" | Asked for by the product owner, 2026-10-07; the conversion rules signed 2026-10-16 by the product owner, as `DATA-17` decision 15 |
| 11 | Parquet files | `.parquet`, the file's one table, read as described under "How a Parquet file is read"; binary and nested columns refused | Asked for by the product owner, 2026-10-08; the conversion rules signed 2026-10-16 by the product owner, as `DATA-17` decision 15 |
| 2 | Size limits | 50 MB and 2,000,000 rows per file. Over either, the file is refused, not cut short | Product owner, 2026-10-02. **Amended 2026-10-16** by decision 14 (the row cap is gone) and decision 13 (the 50 MB stays for a workbook and for `POST /api/uploads`; the page's own path takes 2 GB) |
| 3 | Where it sits | A button and an "Uploaded files" tab on Sources; a link from "New Connector" | Product owner, 2026-10-02 |
| 4 | Who may upload | Holders of the existing `connector:manage` permission (Data Engineer, Platform Admin). No new permission | Product owner, 2026-10-02 |
| 5 | Loading into a table that exists | Only a table an upload of the same tenant created. Replace by default; adding is a choice. Never a connector's table | Product owner, 2026-10-02 |
| 6 | Column types | Text for every column | Product owner, 2026-10-02. **Replaced 2026-10-16** by decision 15 (`DATA-18`): the type of every column is guessed and can be changed before the load |
| 7 | The original file | Kept until the upload is deleted. Deleting an upload keeps its table | Product owner, 2026-10-02 |
| 8 | The assistant | No tool in this version | Product owner, 2026-10-02 |
| 9 | The transformation code that came with the unfinished upload code | Leaves the branch. A model written for one customer's export does not belong in the product | Product owner, 2026-10-02 |
| 12 | Which formats the page takes | CSV, TSV, JSON (an array of objects), JSON Lines (one object per line), Avro (an uncompressed container), Parquet, Excel (one sheet), a `.gz` of one text or JSON file, and a `.zip` of up to ten files of one format (Parquet stays out of a zip) | Product owner, 2026-10-16, as `DATA-17` decisions 1, 11, 12 and 13; `upload-big-files-and-more-formats.md` |
| 13 | How many files, how big, and how they are sent | Up to ten files at once, 2 GB (2,147,483,648 bytes) in all for one upload, counting what a gzip or a zip unpacks to. The page sends every file in pieces of 8 MiB. An Excel workbook stays at 50 MB (it is read whole), and `POST /api/uploads` stays at 50 MB for scripts | Product owner, 2026-10-16, as `DATA-17` decisions 1, 2, 5, 6, 9 and 14; `upload-big-files-and-more-formats.md` |
| 14 | The 2,000,000-row cap | Removed: the measurement showed the load's memory follows bytes and not rows (`docs/plans/DATA-17-RESULT.md`) | Product owner, 2026-10-16, as `DATA-17` decision 10 |
| 15 | Column types | The type of every column is guessed from the first 10,000 rows (or the first 16 MB) and the name and the type can be changed before the load. A load that sends no column list still stores every column as text | Product owner, 2026-10-16, as `DATA-18` decisions 1, 2, 8 and 11; `upload-column-types.md` |

## How a workbook is read

The API stores the workbook as it arrived. The chosen sheet is converted to
comma-separated UTF-8 text, and that text is stored beside the original; the
conversion runs once and the preview reads the stored text. The column's
type is then guessed from that text, like any other file's (`DATA-18`), so
the cell's kind is what the text shows: `1234.5` is a number, `2025-09-24` a
date, `true` a yes/no. A cell becomes text like this:

| Cell | Text |
| --- | --- |
| Empty | Empty |
| Text | As stored |
| Number | As the cell holds it, never the display format: `1234.5` (not `1,234.50`), `0.25` (not `25%`). A whole number has no decimal point; any other is the shortest form that reads back the same (`0.1`); exponent form (`1e21`, `1.5e-7`) from 1e21 and below 1e-6 |
| Date | `2025-09-24`; with a time, `2025-09-24 13:30:05` (`.mmm` only if the cell holds milliseconds). 1900 and 1904 workbooks alike |
| Time of day | `13:30:00` |
| Duration (`[h]:mm:ss`) | `36:00:00` |
| Boolean | `true` / `false` |
| Error | Its own text, `#DIV/0!` |
| Formula | The result the file stored (empty if it stored none) |
| Merged cells | The top-left value; the rest empty |

A number is a date only if its cell's number format says so; the display
format is not used for anything else. A value no date can be (negative, or past
the year 9990) is written as the number it is. A date cell with no date part
(a value below 1) is a time of day.

A sheet's rows start at its first filled cell, so "header row 1" is the first
row with something in it, which is not always the row number Excel shows. A
cell that is only formatted is not filled. Chart, dialog and macro sheets are
not listed.

A sheet whose used range (rows times columns, empty cells included) is over
5,000,000 cells is refused. That number is a cap that keeps the conversion's
memory bounded, not a measurement; a workbook can hold far more than its file
size suggests because it is compressed. The preview reads the first 256 KB of
the converted text and shows the first 20 rows of it, so a header row past
that shows no columns; the type guess reads the first 10,000 rows of the
text, or the first 16 MB of it.

## How a Parquet file is read

The API stores the file as it arrived. For the preview it reads the file's
footer for the column names, the declared types and the row count, and
decodes only the rows it shows (the first 2,000 at most) to write them as
comma-separated UTF-8 text; for the load the file stays as it arrived and the
load job reads it natively, in batches of rows, instead of reading a
converted copy (`DATA-17`). The first record is the file's own column names,
so there is no header row to choose. A column's type starts from the type the
file declares and can be changed before the load (`DATA-18`). A value becomes
text like this:

| Parquet value | Text |
| --- | --- |
| Null | Empty (so is an empty string) |
| String | As stored |
| Integer (signed or unsigned) | Its digits |
| `float`, `double` | The shortest form that reads back the same, never widened first: `0.1`; a whole number has no decimal point; exponent form (`1e21`, `1e-7`) from 1e21 and below 1e-6; `NaN`, `inf`, `-inf` |
| Decimal | Its exact digits with the scale applied, never through a float: `12.50` for `decimal(10,2)` |
| Boolean | `true` / `false` |
| Date | `2025-09-24` |
| Timestamp, adjusted to UTC | `2025-09-24T13:30:05.123456Z`: the fraction only when there is one, exact to the unit the file uses (milliseconds, microseconds, nanoseconds) |
| Timestamp, not adjusted | The same without the `Z`: a local date and time, never converted to a zone the file does not name |
| Legacy `INT96` timestamp (Spark, Impala) | A local time, without `Z` (the type carries no zone flag); read as microseconds, so a year outside 1677 to 2262 does not wrap, and a sub-microsecond part is dropped |
| Time of day | `13:30:00`, with the fraction only when there is one |
| Binary, list, map, struct | Not loaded: the file is refused, naming the first such column and its type |

A date, timestamp or time that no calendar date or day can hold (a year past
9999, a time outside one day) is written as the integer the file holds, not as
a wrong date. A column name is loaded the way the job names the columns of any
file (`_column_names` in `file_ingest.py`): lower case, only ASCII letters,
digits and `_`, a blank one `col_<position>`, a repeated one with `_2`, `_3`.
So the table's column can be spelled differently from the file's (`Unit Price`
becomes `unit_price`); the preview shows the file's own names, and no column is
renamed or refused by the API. A file is also
refused when it is encrypted, does not open, or would make more than 512 MB of
text. Those are caps that keep the conversion's memory bounded, not
measurements. The preview decodes at most the first 2,000 rows and shows 20.

## Limits to tell a customer

- Delimited text files are accepted in UTF-8 or UTF-16, Excel workbooks
  (`.xls`, `.xlsx`) one sheet at a time, Parquet files, JSON (an array of
  objects), JSON Lines (one object per line), Avro (an uncompressed
  container), a `.gz` of one text or JSON file and a `.zip` of up to ten
  files of one format; every other kind is refused with the reason. A
  macro-enabled `.xlsm`, an `.xlsb` or an `.ods` has to be saved as
  `.xlsx` or CSV first, and a Parquet file cannot be put in a zip.
- Up to ten files and 2 GB (GiB: 2,147,483,648 bytes) per upload, counting
  all the files of one load together and what a gzip or a zip unpacks to. The
  Upload file page sends every file in pieces, so a file of that size never
  arrives in one request. `POST /api/uploads`, the route for scripts, keeps
  its 50 MB limit, and so does an Excel workbook, which is read whole. There
  is no row cap any more: the old 2,000,000-row limit was removed because the
  job's memory was measured not to follow the size of the file
  (`docs/plans/DATA-17-RESULT.md`; `DATA-17`, #186, merged 2026-10-26;
  `upload-big-files-and-more-formats.md`). A workbook sheet of more than
  5,000,000 cells is refused.
- An Avro file must be uncompressed (the null codec). A compressed container
  is refused, on both sides, with "This Avro file is compressed. Save it with
  no compression."
- A workbook is stored more than once: the original, the list of its sheets,
  and the converted text of each sheet that was read. They are all removed
  with the upload.
- A Parquet file with a binary or nested column is refused: remove the column
  and save the file again. On the chunked path the API does not convert a
  Parquet file; the load job reads it (`DATA-17`, #186). The 5,000,000-cell
  cap applies only to a whole-file conversion in the API, which nothing
  calls.
- A file of more than 1,000 columns is refused at the Check step with "The
  file has more than 1,000 columns."; changing the delimiter or the header
  row there reads it again (`SEC-17`).
- Four uploads in progress per user and sixteen in all, at once: files being
  received and loads running are each counted against both numbers. The next
  is refused with "Too many uploads are in progress. Try again in a moment."
  and can be sent again; nothing waits. A load that has shown as running for
  more than 6 hours no longer counts (`SEC-17`, `UPLOAD_MAX_CONCURRENT_PER_USER`
  and `UPLOAD_MAX_CONCURRENT`; `docs/core/features/upload-limits-and-safe-csv.md`).
- Each large load runs in its own process: about 2.2 GiB of memory per 2 GB file when several run
  together, and about 4.9 GiB for a 2 GB Parquet file that is a single row group
  (`docs/plans/DATA-17-RESULT.md`, `DATA-33`, `DATA-34`). The installation allows 16 loads at
  once, so 16 large files at once can run out of memory on a 16 GB server. A separate limit for
  large loads is `DATA-37`.
- The type of every column is guessed from the first 10,000 rows (or the
  first 16 MB) and can be changed before the load; a date that could be read
  either way asks which reading it is, and Load stays off until it is
  answered. A value that does not fit the chosen type is counted, the first
  100 are listed, and the load either stops or writes them as empty, as the
  person chose (`DATA-18`, #186, merged 2026-10-26; `upload-column-types.md`).
  A load that sends no column list stores every column as text.
- The Upload file page shows a table's name twice: `bronze.<table>` as the Catalog lists it, and `iceberg.bronze.<table>`, the name Query Studio reads on the Trino engine (`DATA-53`).
- A file named like a format the upload reads that does not open as one is refused as a file that could not be opened, not as a format that is not read (`DATA-51`).
- Adding a file to a table that exists keeps that table's names and types.
  Replacing a table may change its types, and a pipeline that reads the table
  fails on its next run and says why.
- An uploaded table stays in the raw layer. A dashboard reads mart tables
  only, so an uploaded table reaches a dashboard through a pipeline (raw →
  Silver → Gold), which is built (`DATA-19`, `uploads-feed-pipelines.md`).
  Until that pipeline exists, the table can be explored and queried.
- The original file is kept until someone deletes the upload, so storage
  grows with every upload.
- Deleting an upload never drops its table. A table an upload made is dropped
  from the table's page in the Catalog (`DATA-55`, `asset-page-depth.md`), and
  while it can still be restored a file cannot be loaded into its name.
- Adding the same file to a table twice doubles its rows. The console warns
  about a repeated file; it does not stop it.
- Replacing a table with a file whose columns differ keeps the old
  columns, empty, beside the new ones.
- A table name starts with a lower-case letter and uses lower-case letters
  and digits joined by single underscores, with at most 128 characters.
- A table name an upload has asked for stays reserved for that tenant's
  uploads. Another tenant cannot upload into it and a connector cannot load
  into it, even if the first load never wrote anything.
- A table name that cannot be used is refused with one sentence, "That table
  name cannot be used. Choose another name.", whether another tenant holds
  it, another tenant's connector loads it, or it exists and nobody
  claimed it. Only a connector of the person's own tenant is named: "A
  connector loads that table, so a file cannot be loaded into it." (`SEC-16`.)
  A refused name still tells a person that someone has taken it; only a
  namespace per tenant would remove that.
- On the Upload file page a dropped connection pauses the upload and it
  continues by itself while the page stays open (`DATA-17`, #186). After a
  reload it starts again.
- A person may hold at most twenty open upload sessions; the next is refused
  with "Too many uploads are open. Finish or remove some of them, then try
  again."
- In an install with several tenants, the table an upload creates appears in
  the shared catalog, which only the tenant that owns the catalog sees.

## Acceptance checklist

Screen labels are as specified in the plan. If the built screen differs,
record it as a finding.

**Before starting:** the orchestrator is running; the storage credentials
are set (operator); you have a Data Engineer login and an Analyst login, and
a second tenant's Data Engineer login if the install has several tenants.
Prepare these files:

- **A**: a comma-separated UTF-8 file with a header and N data rows, one
  field of which holds a comma and a line break inside quotes. Write N down.
- **B**: a tab-separated UTF-16 export with title lines above its header.
- **C**: an `.xlsx` workbook with two sheets, a date column and a number
  column formatted with thousands separators.
- **E**: an `.xlsm` workbook (or a password-protected `.xlsx`), and an
  `.xls` (the old format) with any content.
- **D**: any file larger than 50 MB and smaller than 2 GB.
- **G**: any file larger than 2 GB.
- **H**: an `.xlsx` workbook larger than 50 MB.
- **F**: a `.parquet` file with a decimal, a date and a timestamp column, and a
  second one with a binary or list column.
- **I**: a CSV with a whole-number column, an ISO date column, a text column
  and one column of `01/02/2026`-shaped dates.
- **J**: a CSV with a word in a column of whole numbers.
- **K**: a `.json` file holding an array of objects, one of which has a
  nested object.
- **L**: a `.csv.gz` of a CSV with a header, and a `.zip` of three CSVs with
  the same columns.
- **M**: eleven small CSVs with the same columns.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | As Data Engineer, open Sources | An "Upload file" button and an "Uploaded files" tab | |
| 2 | Press "New Connector" | The first step offers a link to upload a file; it opens the same page as the button | |
| 3 | Upload file A | The preview says UTF-8, comma, header row 1; the quoted field is one cell | |
| 4 | Change the delimiter to semicolon, then back | The preview shows one column, then the real columns again | |
| 5 | Name the table `qa_upload_a` and load it | "Loading", then "Loaded" with N rows | |
| 6 | Open the table from the result | It is in Data Explorer, in the raw layer; the columns have the types the Check step showed; the sample shows the file's rows | |
| 7 | In Query Studio, count the table's rows with a `WHERE` clause | N | |
| 8 | Upload file A again | A notice that this file was uploaded before | |
| 9 | Load it into `qa_upload_a`, leaving "Replace" selected | Loaded; the table still has N rows | |
| 10 | Load it once more, choosing "Add to its rows" | Loaded; the table has 2 × N rows | |
| 11 | Upload file B | The preview says UTF-16, tab, and a header row below the title lines; the column names are the file's own | |
| 12 | Load file B into `qa_upload_b` | Loaded; the title lines are not rows | |
| 13 | Upload file C | Accepted: the Check step lists C's sheets and shows the first visible one. (Choose a sheet, load it into `qa_upload_c`: Loaded, with that sheet's rows, dates as `YYYY-MM-DD`) | |
| 14 | Upload file D (over 50 MB, under 2 GB) | Accepted. The file is sent in pieces with a progress bar, and it loads | |
| 14a | Upload file G (over 2 GB) | Refused in the browser before a byte is sent, naming the 2 GB limit. Nothing new under "Uploaded files" | |
| 14b | Upload file H (a workbook over 50 MB) | Refused, naming the 50 MB limit for an Excel workbook. Nothing new under "Uploaded files" | |
| 15 | Load file A into a table a connector of your own tenant loads | Refused, saying a connector loads it; that table is unchanged | |
| 15a | Load file A into a table another tenant's connector loads | Refused with "That table name cannot be used. Choose another name.", which does not mention a connector | |
| 16 | Load file A into a table named `Orders 2025` | Refused, with the naming rule | |
| 17 | (operator) Stop the orchestrator's code location; load file A into `qa_upload_fail`; start it again | The upload shows "Failed" with a reason, or the load is refused as unavailable. It is never shown as loaded | |
| 18 | Look at the rows of the failed upload | "Not measured", not 0 | |
| 19 | As Analyst, open Sources and then the "Uploaded files" tab | The list says the permission is missing | |
| 20 | (operator) As Analyst, call `POST /api/uploads` | 403 | |
| 20a | (operator) As Data Engineer, call `POST /api/uploads` with file D | 400, naming the 50 MB limit of that route | |
| 21 | As the second tenant's Data Engineer, open "Uploaded files", then the address of an upload from step 5 | The upload is not listed; its address says not found | |
| 22 | As Data Engineer, delete the upload from step 5 | It leaves the list; `qa_upload_a` and its rows remain in Data Explorer | |
| 23 | Upload file C; on the Check step pick the second sheet | The preview shows that sheet's rows; the first sheet was the one chosen to begin with | |
| 24 | Look at a date column and a number column of the preview | Dates as `YYYY-MM-DD`, numbers without thousands separators | |
| 25 | Load the sheet into `qa_upload_c`, then open it | Loaded with the sheet's rows; the columns have the types the Check step showed | |
| 26 | Upload an `.xls` of file E | The same as 23-25 | |
| 27 | Upload the `.xlsm` or the password-protected file of E | Refused with the reason; nothing new under "Uploaded files" | |
| 28 | Delete the upload from step 25 | It leaves the list; every stored object of that upload is removed (operator: nothing under its key in the bucket) | |
| 29 | Upload the first file of F | Accepted: the Check step has no sheet, encoding, delimiter or header-row control; each column shows "was: <type>" under its name; the line above the preview gives the file's row count | |
| 30 | Look at the decimal, date and timestamp columns of the preview | A decimal with its scale (`12.50`), a date as `YYYY-MM-DD`, a timestamp as `YYYY-MM-DDTHH:MM:SS`, with a trailing `Z` only for a column in UTC | |
| 31 | Load it into `qa_upload_f`, then open it | Loaded with the file's rows; the columns keep the types the file declared | |
| 32 | Upload the second file of F | Refused, naming the first binary or list column and its type; nothing new under "Uploaded files" | |
| 33 | Delete the upload from step 31 | It leaves the list; the stored objects are removed | |
| 34 | Upload file I | The Check step shows a type for every column: Integer for the whole numbers, Date for the ISO dates, Text for the words; the line above the columns says how many rows were read | |
| 35 | On the Check step of file I, change the text column's type to Integer | The select shows Integer, the line under it says "Guessed: Text", and the panel "Values that do not fit" lists the failing values | |
| 36 | On the Check step of file I, rename a column | The name field holds the new name; after the load the table has that name | |
| 37 | Upload the file of I whose dates are all `01/02/2026` | The "Read as" select asks "Day first or month first?" and Load stays off until it is answered | |
| 38 | Upload file J | The Check step counts the values that do not fit and lists up to 100 of them, and asks what should happen; "Stop the load." is the pre-selected choice | |
| 39 | Load file J with "Stop the load." | The load stops, the table is unchanged, and the count and the failing rows are shown | |
| 40 | Load file J with "Load them as empty, and show the same report." | The table loads with that cell empty, and the same report is shown | |
| 41 | Upload file K | The Check step has no encoding, delimiter or header-row control, because the keys are the columns; the Review step says "Format: JSON"; the nested value is shown as JSON text in one column | |
| 42 | Upload the `.csv.gz` of file L | It loads like the plain CSV | |
| 43 | Upload the `.zip` of file L | The Check step lists the three files; loading them puts their rows in one table | |
| 44 | Pick the eleven files of M | The first ten are kept and the page says the eleventh was left out | |
| 45 | Pick two CSVs where the second has one extra column | Before loading, the second file is listed with its extra column, and Load is off until it is removed | |
| 46 | Open the table of step 34 in Data Explorer, then its Schema tab | The columns are listed with their types (Integer, Date, Text), and the renamed column keeps its new name | |

**Accepted** when 1–16, 19, 22 and 23–46 pass, and 17, 18, 20, 20a and 21 pass or
have an agreed exception.

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 ("Upload a file from the console") and section 3 updated
- [ ] `BACKLOG.md`: `DATA-9` moved to Done; follow-ups added (loading several sheets, larger files, pipelines reading raw tables)
- [ ] `CHANGELOG.md` entry a customer can read

## QA-1 decisions (product owner, 2026-11-02)

- An empty file is refused with a message that the file is empty, and it is never shown as loaded.
- Column types are guessed and editable before loading; files go up to 2 GB in pieces; a workbook keeps the 50 MB limit.
- When a page states a rule but not its exact message, QA checks that the action is refused (or shown) and that the message names the rule. The wording itself is not tested. Defaults the planner wrote on a page count as decided unless the owner changes them.
