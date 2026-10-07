# ADR 0014 — Files uploaded from the console

- **Status:** Accepted (product owner, 2026-10-02); implementation in progress. Amended 2026-10-07: Excel workbooks (see "Amendment" below)
- **Phase:** `DATA-9`, plan `docs/superpowers/plans/2026-10-02-upload-file.md`
- **Date:** 2026-10-02

## Context

Every way data enters Bronze today starts from a connector row: a database,
a bucket, an API, a topic (`ADR 0013`). A file a person holds has no row to
start from. Commit `f9793cd` on `feat/connectors` sketched an upload path
and left it unwired; reading it for this ADR found the questions it had
answered implicitly, and a few it had answered wrongly (the plan's section 2
lists them). This ADR records the answers the build must keep.

The file that motivated the work was named `.xls`, was UTF-16 tab-separated
text, and carried six report lines above its real header. Its extension, its
declared content type and its first line were all misleading.

## Decision 1 — the original file is kept, in the warehouse bucket

The bytes are written to the same S3-compatible warehouse bucket Bronze
uses, under `uploads/<tenant id>/<upload id>.<ext>`. The key is built only
from server-generated parts; the user's file name contributes a sanitised
extension and is otherwise kept for display.

Kept, because a parse that turns out wrong (delimiter, header row, encoding)
can then be redone without asking for the file again. In the warehouse
bucket, because Postgres holds the console's state and not customer data,
and because one bucket means one set of credentials and one thing to back
up. Under its own prefix, so nothing an upload writes can sit next to an
Iceberg table's files.

The file is removed when the upload is deleted, and so is its row. Nothing
else expires it.

## Decision 2 — the upload passes through the API, with a cap

`POST /api/uploads` takes one multipart part, buffers it, hashes it and
stores it. The cap is 50 MB. It is a memory bound as much as a product
limit: ten uploads at the cap at once is 500 MB of API process.

## Decision 3 — the parse is confirmed by a person, never guessed silently

The API detects encoding, delimiter and header row and shows what they
produce. The load uses what the user confirmed and records it. A detection
that silently overrides a person produces a table that looks successful and
is wrong, which is the failure this product treats as the most serious.

Preview (Rust) and load (Python) are two readers of one dialect: fields
separated by one delimiter character, `"` as the quote, `""` as an escaped
quote, delimiters and line breaks allowed inside quotes. Both are tested
against the same fixture files, so they cannot drift apart unnoticed.

## Decision 4 — only delimited text, and every column as text

A file whose first bytes say it is a workbook, a Parquet file or another
binary format is refused by name. Reading a workbook is its own feature
(sheets, merged cells, dates stored as numbers), not a variant of this one.

No type is inferred. A material number `0250161` is not the integer 250161,
and `07.02.2025` is not the same date in every locale. Bronze keeps what the
source said; typing belongs to the step after it, where the rule is visible
and can be re-run.

## Decision 5 — the load is an ordinary Bronze write

`file_ingest_job` reads the stored object, parses it and hands the rows to
the shared sink (`adapters/sink.py::load_via_sink`), with a `replace` or
`append` plan. It writes no Iceberg of its own. The table is registered in
the catalog the way a connector's table is, with its total counted under a
`WHERE` (risk R11).

The job reports its outcome the way `ingest_job` does, in
`lake.bronze_meta.ingest_run`, under the id `upload:<upload id>`. It does
not write to Postgres: the job holds credentials for the source database of
the demo ingest, not for the console's database, and the two are the same
only on the compose stack.

An upload may load into a table that does not exist, or into one its
tenant has claimed. A claim is a row of `upload_table_claim`, keyed by the
table name, made when a load is first requested and never released: one
name, one tenant, decided by the database. It is a record of its own
because an upload's row cannot carry it: the row names only the table of
its last load, it goes when the upload is deleted, and nothing stops two
tenants' rows naming the same table. The cost is that a name a tenant's
upload once asked for stays that tenant's, even if the load never wrote. It may never load into a table a connector
loads or that anything else created. The API checks this before launching,
and refuses when it cannot check.

## Decision 6 — an upload belongs to a tenant

The row carries the uploader's active tenant. Listing is scoped to it, and
every route that names an upload answers 404 to a caller outside that
tenant, the same rule connectors follow (`routes::connectors::
ensure_connector_in_tenants`). The permission is the existing
`connector:manage`.

The catalog itself is not per tenant (`docs/OPERATIONS.md`, "set
`CATALOG_TENANT_ID`"). The table an upload creates is therefore visible to
whoever may see the shared catalog. This ADR does not change that.

## Alternatives considered

**The browser writes to object storage through a presigned URL.** It
removes the cap and the memory cost. Rejected for this version: it needs the
object store reachable from the browser, with CORS and a public endpoint,
which no install has today. It is the natural next step when 50 MB is too
small.

**A hidden `files` connector pointed at the uploads prefix.** It would reuse
`adapters/files.py` unchanged. Rejected: that adapter reads UTF-8,
comma-separated files whose first line is the header, and a connector is a
standing thing with a schedule and a credential, which an uploaded file is
not. The two share the sink, which is the part worth sharing.

**Load straight into the analytics engine.** The table would be usable by a
pipeline at once. Rejected: raw data is Iceberg in this product, and a
second kind of raw table would have to be explained on every page that
shows layers.

**Infer types.** Rejected for the reason in decision 4.

## Consequences

- Two new tables, `file_upload` and `upload_table_claim`, and six routes
  under `/api/uploads`.
- A table name an upload has asked for stays reserved for that tenant's
  uploads. A connector cannot take it.
- A new dependency: axum's `multipart` feature. The dependency checks are
  red on `main` (`SEC-8`); a pull request with this change cannot merge
  until they are green (`AGENTS.md`, step 6).
- The warehouse bucket grows by the size of every file kept.
- An uploaded table stops at Bronze until a pipeline can read a raw table.
  `authored_factory.py` reads the analytics engine only. That limit is the
  pipeline builder's, and is stated on the feature page.
- The unregistered transformation modules that came with the unfinished
  code (`silver_transform.py`, `gold_transform.py`, `sap_models.py`) leave
  the branch, and `ch_models.py` with them: the three helpers
  `connector_catalog.py` used moved into it.

## Amendment, 2026-10-07 — Excel workbooks

The product owner asked for `.xls` and `.xlsx` (plan
`docs/superpowers/plans/2026-10-07-upload-excel.md`). Decision 4's first
paragraph no longer holds for those two kinds: a workbook is accepted by
converting ONE sheet to delimited text in the API.

- The workbook is stored as it arrived (decision 1). Preview converts the
  chosen sheet in memory and runs the existing preview on it. A load converts
  it, stores the text beside the original (`<key>.converted.csv`, deleted with
  the upload) and launches the unchanged `file_ingest_job` on that object with
  a fixed dialect, UTF-8 and comma. The job does not change.
- Decision 3's invariant (two readers, one dialect) is unchanged, because the
  load job still reads delimited text and nothing else. The conversion is a
  third component, a writer of that dialect, and its output is pinned by
  `ops/fixtures/uploads/converted_sheet.csv`, which both readers' fixture
  tests read, and by a test that converts the `Quirks` sheet of the workbooks
  in `ops/fixtures/workbooks/` to exactly those bytes.
- Decision 4's second paragraph holds: every column is text. A date is
  written ISO 8601 and a number as the cell holds it, never through its display
  format; the full rules are in `upload_workbook.rs` and on the feature page.
- A workbook is recognised by its first bytes AND an `.xls` or `.xlsx` name:
  the name alone cannot decide, because the file that motivated this ADR is
  UTF-16 text called `.xls`. `.xlsm`, `.xlsb`, `.ods` and other zip files stay
  refused, with the reason.
- A sheet over 5,000,000 cells (used range) is refused: a workbook is
  compressed, and the API converts it in memory. A cap, not a measurement.
- The chosen sheet is not in the job's run configuration (its schema is
  closed and the job does not change); it is recorded in the upload's
  `parse_options` and in the `upload.ingest` audit event. No migration.
- New dependency: `calamine` (MIT), with seven transitive crates, all MIT or
  Apache-2.0 and in `deny.toml`'s allow list.

## Verification

None yet. This ADR is written with the plan, before the code. What is run,
and what is not, is recorded in the plan's Handoff and Review sections.
