# Upload a file into a raw table

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `DATA-9` |
| Status | In build. Decisions signed 2026-10-02 |
| Plan | `docs/superpowers/plans/2026-10-02-upload-file.md` |

## Problem

Data reaches the lakehouse only through a registered source: a database, a
bucket, an API. A person who holds a file, exported from an ERP, a
spreadsheet or a vendor portal, has no way to bring it in from the console.
Today someone converts the file and loads it into a database by hand, on the
server (`PRODUCT.md` section 2: "Upload a file from the console: Missing").

Unfinished code for this exists on `feat/connectors` (commit `f9793cd`). It
is not compiled into the API, not registered in the orchestrator, and has no
screen.

## What the user can do when this is done

1. On Sources, press "Upload file". The same page is offered from the first
   step of "New Connector".
2. Choose a delimited text file (CSV or TSV) of up to 50 MB and send it.
3. See what the file looks like before anything is loaded: the encoding,
   the delimiter and the header row the system detected, the column names,
   and the first 20 rows.
4. Correct any of the three and see the preview change.
5. Name the raw table the file becomes.
6. Load it, and watch the load go from "Loading" to "Loaded" with the number
   of rows, or to "Failed" with a reason.
7. Open the new table in Data Explorer from the result.
8. Load a later file into a table an earlier upload created: replace its
   rows (the default), or add to them.
9. Be told when the same file was uploaded before.
10. See every upload of their tenant under "Uploaded files" on Sources, with
    its status, table, rows, who uploaded it and when.
11. Delete an upload. The file is removed; the table it became stays.

A user without the permission to manage connectors cannot upload, load or
delete, and does not see other people's uploads.

## Not included

- Excel workbooks (`.xlsx`, `.xls`), JSON and Parquet. They are refused with
  a message that says what to do, never half-read.
- Files over 50 MB or over 2,000,000 rows.
- Files that arrive on a schedule. A bucket or SFTP source does that.
- Guessing column types. Every column is stored as text.
- Turning the table into Silver or Gold. That is the pipeline builder's job.
- Removing the raw table from the upload list.
- Scanning files for malware.

## Asking the assistant

Not in this version. A file cannot be attached to a chat, and loading an
uploaded file changes which data exists, so it is left for a follow-up with
its own approval rule. The assistant can already describe and query the
table once it is loaded.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Which files are accepted | Delimited text only: UTF-8 or UTF-16; comma, semicolon, tab or pipe | Product owner, 2026-10-02 |
| 2 | Size limits | 50 MB and 2,000,000 rows per file. Over either, the file is refused, not cut short | Product owner, 2026-10-02 |
| 3 | Where it sits | A button and an "Uploaded files" tab on Sources; a link from "New Connector" | Product owner, 2026-10-02 |
| 4 | Who may upload | Holders of the existing `connector:manage` permission (Data Engineer, Platform Admin). No new permission | Product owner, 2026-10-02 |
| 5 | Loading into a table that exists | Only a table an upload of the same tenant created. Replace by default; adding is a choice. Never a connector's table | Product owner, 2026-10-02 |
| 6 | Column types | Text for every column | Product owner, 2026-10-02 |
| 7 | The original file | Kept until the upload is deleted. Deleting an upload keeps its table | Product owner, 2026-10-02 |
| 8 | The assistant | No tool in this version | Product owner, 2026-10-02 |
| 9 | The transformation code that came with the unfinished upload code | Leaves the branch. A model written for one customer's export does not belong in the product | Product owner, 2026-10-02 |

## Limits to tell a customer

- Only delimited text files are accepted, in UTF-8 or UTF-16. A workbook has
  to be saved as CSV first.
- 50 MB and 2,000,000 rows per file.
- Every column is text. Numbers and dates have to be converted afterwards.
- An uploaded table stays in the raw layer. The pipeline builder cannot read
  raw tables yet, and a dashboard reads Gold tables only. So an uploaded
  table can be explored and queried, and cannot yet feed a dashboard.
- The original file is kept until someone deletes the upload, so storage
  grows with every upload.
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
- An upload is not resumable. If the connection drops, it starts again.
- In an install with several tenants, the table an upload creates appears in
  the shared catalog, which only the tenant that owns the catalog sees.

## Acceptance checklist

Screen labels are as specified in the plan. If the built screen differs,
record it as a finding.

**Before starting:** the orchestrator is running; the storage credentials
are set (operator); you have a Data Engineer login and an Analyst login, and
a second tenant's Data Engineer login if the install has several tenants.
Prepare four files:

- **A**: a comma-separated UTF-8 file with a header and N data rows, one
  field of which holds a comma and a line break inside quotes. Write N down.
- **B**: a tab-separated UTF-16 export with title lines above its header.
- **C**: an `.xlsx` workbook.
- **D**: any file larger than 50 MB.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | As Data Engineer, open Sources | An "Upload file" button and an "Uploaded files" tab | |
| 2 | Press "New Connector" | The first step offers a link to upload a file; it opens the same page as the button | |
| 3 | Upload file A | The preview says UTF-8, comma, header row 1; the quoted field is one cell | |
| 4 | Change the delimiter to semicolon, then back | The preview shows one column, then the real columns again | |
| 5 | Name the table `qa_upload_a` and load it | "Loading", then "Loaded" with N rows | |
| 6 | Open the table from the result | It is in Data Explorer, in the raw layer; every column is text; the sample shows the file's rows | |
| 7 | In Query Studio, count the table's rows with a `WHERE` clause | N | |
| 8 | Upload file A again | A notice that this file was uploaded before | |
| 9 | Load it into `qa_upload_a`, leaving "Replace" selected | Loaded; the table still has N rows | |
| 10 | Load it once more, choosing "Add to its rows" | Loaded; the table has 2 × N rows | |
| 11 | Upload file B | The preview says UTF-16, tab, and a header row below the title lines; the column names are the file's own | |
| 12 | Load file B into `qa_upload_b` | Loaded; the title lines are not rows | |
| 13 | Upload file C | Refused, with a message to save it as CSV. Nothing new under "Uploaded files" | |
| 14 | Upload file D | Refused, naming the 50 MB limit. Nothing new under "Uploaded files" | |
| 15 | Load file A into a table a connector of your own tenant loads | Refused, saying a connector loads it; that table is unchanged | |
| 15a | Load file A into a table another tenant's connector loads | Refused with "That table name cannot be used. Choose another name.", which does not mention a connector | |
| 16 | Load file A into a table named `Orders 2025` | Refused, with the naming rule | |
| 17 | (operator) Stop the orchestrator's code location; load file A into `qa_upload_fail`; start it again | The upload shows "Failed" with a reason, or the load is refused as unavailable. It is never shown as loaded | |
| 18 | Look at the rows of the failed upload | "Not measured", not 0 | |
| 19 | As Analyst, open Sources | Uploading is not available; the page says the permission is missing | |
| 20 | (operator) As Analyst, call `POST /api/uploads` | 403 | |
| 21 | As the second tenant's Data Engineer, open "Uploaded files", then the address of an upload from step 5 | The upload is not listed; its address says not found | |
| 22 | As Data Engineer, delete the upload from step 5 | It leaves the list; `qa_upload_a` and its rows remain in Data Explorer | |

**Accepted** when 1–16, 19 and 22 pass, and 17, 18, 20 and 21 pass or have
an agreed exception.

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 ("Upload a file from the console") and section 3 updated
- [ ] `BACKLOG.md`: `DATA-9` moved to Done; follow-ups added (workbooks, larger files, pipelines reading raw tables)
- [ ] `CHANGELOG.md` entry a customer can read
