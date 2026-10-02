# ADR 0014 — Files uploaded from the console

- **Status:** Accepted (product owner, 2026-10-02); implementation in progress
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

The file is removed when the upload is deleted. Nothing else expires it.

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

An upload may load into a table that does not exist, or into one an upload
of the same tenant created. It may never load into a table a connector
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

- A new table, `file_upload`, and six routes under `/api/uploads`.
- A new dependency: axum's `multipart` feature. The dependency checks are
  red on `main` (`SEC-8`); a pull request with this change cannot merge
  until they are green (`AGENTS.md`, step 6).
- The warehouse bucket grows by the size of every file kept.
- An uploaded table stops at Bronze until a pipeline can read a raw table.
  `authored_factory.py` reads the analytics engine only. That limit is the
  pipeline builder's, and is stated on the feature page.
- The unregistered transformation modules that came with the unfinished
  code (`silver_transform.py`, `gold_transform.py`, `sap_models.py`) leave
  the branch. `ch_models.py` stays: `connector_catalog.py` uses it.

## Verification

None yet. This ADR is written with the plan, before the code. What is run,
and what is not, is recorded in the plan's Handoff and Review sections.
