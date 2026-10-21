# Data roadmap Phase 3: file upload in depth (master plan)

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-16 |
| Roadmap | "Data Build Roadmap", phase 3 "File upload in depth": `DATA-17`, `DATA-18`, `DATA-19` |
| Base | `origin/main` at `17867e52`. Every anchor below was read there |
| Branch | `feat/data-phase-3-uploads`, worktree `/home/hv/lakehouse-phase3-uploads`. One branch, one PR for the phase, opened as a draft on the first push |
| Specs | `docs/core/specs/data-17.md`, `data-18.md` ("Today" corrected in this commit), `data-19.md` |
| Feature pages | `docs/core/features/upload-big-files-and-more-formats.md` (`DATA-17`), `upload-column-types.md` (`DATA-18`): decisions signed 2026-10-16 (every default) |
| State | Decisions signed 2026-10-16: the product owner chose every default on both pages. 2026-10-19: the product owner limited this work to the roadmap's Phase 3 items; `SEC-22` belongs to another team and is not part of this plan. Ready to build |

## 1. Scope, and what is already done

| Item | What | State at `17867e52` | In this phase |
| --- | --- | --- | --- |
| `DATA-19` | Uploaded tables feed pipelines | **Merged in #165** (2026-10-15), In Acceptance. AC1–AC7 on `uploads-feed-pipelines.md` not run; that page still says "Planned" | No code. The planner corrects the page's status; the product owner runs its checklist |
| `DATA-17` | More formats, more files, bigger files | Building: Excel and Parquet merged in #76 (2026-10-08). The rest is missing (section 2) | Slices 1, 2, 4, 5, 6 |
| `DATA-18` | Column types on upload | Planned, nothing built | Slice 3 |
| `SEC-22` | Workbook and Parquet conversion can exhaust the API's memory (P0) | Another team's item | **Not in this plan** (product owner, 2026-10-19). This plan adds no workbook reading and does not depend on it |

`DATA-19` needs nothing from this branch, so Phase 3 is `DATA-17` + `DATA-18`,
and nothing else. `SEC-22` (workbook and Parquet conversion in the API) is another
team's; this plan leaves the workbook path as it is and takes Parquet out of
the API by its own design (AD2), without waiting for it.

## 2. Spec rows against the code

Read at `17867e52`, not run. `R` = `rust/crates/lakehouse-api/src/routes/uploads.rs`,
`J` = `dagster/dispar_orchestrate/file_ingest.py`.

### `DATA-17`

| Spec row | Target | What the code does | Gap |
| --- | --- | --- | --- |
| Formats | CSV, TSV, JSON, JSON Lines, Avro, Parquet, Excel (one sheet) | Delimited text: comma, tab, semicolon, pipe; UTF-8/UTF-16 (`upload_parse.rs:69`, `:171-184`). Excel `.xls`/`.xlsx` with a sheet picker (`upload_workbook.rs`, `upload-check-step.tsx:165-181`). Flat Parquet (`upload_parquet.rs`). Both are **converted to CSV inside the API** (`R:1660` `convert_upload`) and the job reads only delimited text (`J:130-131`). A file is classed by magic bytes plus its name (`upload_parse.rs:101-121`, `R:653-673`) | JSON, JSON Lines, Avro: none in the API, the job or the console. The Excel and Parquet decisions (10, 11 on `upload-file.md`) were never signed |
| Files per upload | Up to 10, in parallel | One `file` part per request (`R:612-634`); the input has no `multiple` (`upload-file-step.tsx:131`); a drop keeps `files[0]` (`:83-86`) | All of it |
| Size | Up to 2 GB, memory measured in a RESULT doc | `MAX_UPLOAD_BYTES` = 50 MiB (`R:135`), body limit 51 MiB on `POST /api/uploads` only (`routes/mod.rs:736`), proxy 51 MiB (`next.config.ts:19`). 2,000,000 rows checked only by the job (`J:100`). Also 1,000 columns (`upload_parse.rs:552`, `SEC-17`), 5,000,000 cells per sheet or Parquet file (`upload_workbook.rs:93`), 512 MiB of converted text (`upload_parquet.rs:120`). No RESULT doc on upload memory exists | The whole file is in memory at every hop (section 3). No container has a memory limit (`docker-compose.yml` has no `mem_limit`) |
| Upload method | Resumable chunks, a progress bar per file | One multipart request read whole (`field.bytes()`, `R:626`), one `put` (`upload_store.rs:176`), no progress, no retry of the upload. Above 51 MiB the proxy **cuts** the body rather than refusing it | All of it. `object_store` 0.14 has the needed low-level API: `MultipartStore` is implemented for `AmazonS3` (`object_store-0.14.1/src/aws/mod.rs:486`) |
| Compressed files | gzip and zip | gzip classed "other binary" and refused (`upload_parse.rs:114`); zip refused unless named `.xls`/`.xlsx`; the console refuses both by name (`src/lib/uploads.ts:231-256`) | All of it. `flate2` and `zip` are in `Cargo.lock` only as transitive dependencies |
| Several files into one table | Same columns into one table; mismatches listed before loading | Several uploads may append into one table, one at a time (`R:1773`, `TABLE_BUSY`). No notion of a group of files | All of it |

### `DATA-18`

| Spec row | Target | What the code does | Gap |
| --- | --- | --- | --- |
| Type detection | Integer, decimal, boolean, date, timestamp, text from a 10,000-row sample *(proposed)* | Every column is declared `text` to dlt (`J:463`); ADR 0014 decision 4 forbids inference. A Parquet file's types are shown as "was: …" (`upload-preview-grid.tsx:120-124`) and dropped (`upload_parquet.rs` converts every value to text). The preview reads 256 KiB and returns 20 rows (`R:146`, `R:149`) | All of it, plus an ADR amendment |
| Edit before loading | Name and type per column in the preview | Nothing is editable. Names are cleaned only by the job (`_column_names`, `J:299`), so the preview shows the raw header (`contracts/uploads.ts:166-171`) | All of it. The cleaning rules must move to (or be mirrored in) the API so the preview can show final names |
| Bad values | Counted, first 100 listed, choose null or stop | n/a: every value is text | All of it |
| Date formats | ISO, dd/mm/yyyy, mm/dd/yyyy; ask when ambiguous | The Excel conversion already writes dates as ISO text (`upload_workbook.rs:32-39`), a good input for detection | Detection and the question |

### `DATA-19` (built, for the record)

All three rows are met by #165: the "Source table" field suggests Bronze tables
(D4), `dependsOn: ["file_ingest_job"]` re-runs a pipeline only for its own table
(D1, `authored_factory.py:912`, `:1136-1160`), and lineage draws `upload:<id>` →
Bronze from `upload_table_claim` (D5). This phase must keep the run-config path
`ops.ingest_uploaded_file.config.bronze_table_name` unchanged, or the D1 sensor
stops firing.

### Rules already signed that this phase must respect or amend by name

1. ADR 0014 decision 2: the file goes through the API and is held in memory; 50 MB is a memory bound. **Amended** by AD1 and AD2.
2. ADR 0014 decision 3: a person confirms the parse; the Rust preview and the Python load are two readers of one dialect, tested against shared fixtures (`ops/fixtures/uploads/`). **Kept**, and extended to types (AD3).
3. ADR 0014 decision 4: every column text, no inference. **Replaced** by `DATA-18`'s decisions.
4. `SEC-17`: 4 uploads per user and 16 per installation, counting files being received and loads running; 429 with no queue; 1,000 columns. **Kept**; the console works inside them (`DATA-17` decision 6).
5. One `ingest_run` row per run, failure reasons from a fixed list (`FAILURE_REASONS`, `J:150`), no retry, one load per table at a time (`TABLE_BUSY`). **Kept.**
6. Storage keys only under `uploads/<tenant>/` from server-made parts; permission `connector:manage` on every upload route (`policy.rs:745-750`); tenant check on every `/api/uploads/{id}*` route (`routes/mod.rs:718-731`, `require_upload_in_tenants`). **Kept** for every new route.

## 3. Why the current design cannot reach 2 GB

Every hop holds the whole file:

1. The browser sends one multipart request.
2. Next.js's `/api` rewrite keeps a copy of the whole body in memory, and past `proxyClientMaxBodySize` it ends the stream without a 413.
3. The API reads the part whole (`R:626`), hashes it, and writes it with one `put`.
4. For Excel and Parquet the API reads the object back whole (`upload_store.rs:222`) and builds the CSV in memory (`R:1667-1694`).
5. The job reads the object whole (`J:432`), decodes it into one Python string (`J:551`) and parses it twice.

Raising the constants would put 2 GB, several times over, into three processes.
The design has to change at every hop, which is what AD1 and AD2 do.

## 4. Architecture decisions (planner)

- **AD1. Chunked, resumable upload through the API into S3 multipart.** The browser asks for an upload session, sends the file in 8 MiB pieces (`PUT /api/uploads/{id}/parts/{n}`), and completes it. The API writes each piece as an S3 multipart part through `object_store::multipart::MultipartStore` and records the multipart id and each part's ETag and SHA-256 in Postgres, so any API process can take the next piece and a restart loses nothing. Rejected: presigned browser-to-storage upload (ADR 0014's "next step"). The storage endpoint is not published to browsers, it needs CORS, and the API would no longer see each piece for the tenant and permission checks. Each 8 MiB piece passes the existing 51 MiB proxy limit untouched. 2 GiB is 256 pieces, far below S3's 10,000.
- **AD2. The job is the only reader of a whole file, and it streams. The API reads only bounded samples.** The preview and the type guess read a bounded prefix with range reads: the Parquet footer plus the first row group, the first Avro blocks, the first lines of JSON Lines, a gzip stream's first bytes, a zip's central directory. The job reads every format natively, in batches, and hands Arrow record batches to the sink. **Parquet stops being converted in the API.** Excel keeps its conversion in the API, at 50 MiB, unchanged by this plan (its memory bounds are `SEC-22`, another team's): a workbook must be opened whole to list its sheets, so a streamed path would not help.
- **AD3. One column-spec contract, detected in the API and applied in the job.** `columns: [{source, name, type, dateFormat?}]` plus `onBadValue: "null" | "stop"`. The API (Rust) guesses types on the sample and validates what the user sends; the job (Python) casts the whole file. Both follow the same rules, proven by one shared fixture set in `ops/fixtures/uploads/types/` that both test suites read (the ADR 0014 decision 3 pattern). The job casts with vectorised `pyarrow.compute`, and goes row by row only to collect the first 100 failures.
- **AD4. A load of several files is one ingest call, with no batch object.** `POST /api/uploads/{id}/ingest` names a primary upload and up to nine more (`alsoUploads`). That makes one Dagster run and one table. Each file stays its own `file_upload` row (listed, deleted and duplicate-checked as today) with `loaded_with` = the primary. The Settler settles them all from the one `ingest_run` row, which is keyed by the primary.
- **AD5. The run config grows; the `DATA-19` path does not move.** New fields: `format`, `storage_keys` (the primary first), `columns`, `on_bad_value`. `bronze_table_name` stays where `authored_factory.py:912` reads it. The config stays closed, with every field required, and the API always sends it whole.
- **AD6. The bad-value report is a small JSON object stored next to the upload** (`<storage_key>.bad-values.json`: count per column and the first 100 rows, each value cut to 200 characters). The API reads it when it settles the row. No ClickHouse schema change, so `EXPECTED_SCHEMAS` is untouched.
- **AD7. Nothing that works today breaks.** `POST /api/uploads` stays at 50 MiB for API clients and the gate. An ingest without `columns` loads every column as text. Tables uploaded before this phase are untouched.
- **AD8. New dependencies, named up front.** Python: `fastavro` (Avro) and `ijson` (a JSON array, streamed). Rust: `apache-avro`, `flate2` and `zip` become direct dependencies, at the versions already in `Cargo.lock` (0.21.0, 1.1.10, 8.6.0). The PR touches dependencies, so it must not merge with a dependency check red (`AGENTS.md` step 6).
- **AD9. The 2 GiB constant is written last.** Slices 1–5 keep the 50 MiB cap behind one constant per language. Slice 6 sets it to the value measured in `docs/plans/DATA-17-RESULT.md`, and the constant cites that document (principle 5).

## 5. Decisions for the product owner

They are on the two feature pages, each with the planner's default. In short:

- **`DATA-17`:**
  - "GB" means GiB, and the cap counts all of a load's files.
  - 2 GB ships only once measured; a lower measured value ships honestly.
  - Several files go into one table and one format; mismatches are listed and must be removed.
  - The first 10 files of a larger pick are kept.
  - Up to 4 pieces are in flight per user.
  - Resuming is automatic while the page stays open.
  - Unfinished uploads are removed after 24 h *(proposed)*.
  - Excel stays at 50 MB.
  - The row cap goes once memory is measured flat.
  - The JSON, Avro, gzip and zip rules are on the page.
  - The old route stays.
  - Decisions 10 and 11 of `upload-file.md` (Excel, Parquet) get signed.
- **`DATA-18`:**
  - The guess reads 10,000 rows or 16 MB *(proposed)*.
  - Six types; Decimal is exact, with 9 digits after the point.
  - Leading zeros stay text; `1`/`0` are integers.
  - Ambiguous dates are asked about.
  - Timestamp zones follow the values.
  - "Stop" is pre-selected for values that do not fit.
  - Names are pre-filled with today's rules.
  - Appending keeps the table's types.
  - Files carrying their own types are the starting guess.
  - API callers with no column list get text.
  - ADR 0014 decision 4 is replaced.

## 6. Order and slices (one branch, one PR)

| Slice | Item | Content | Migration |
| --- | --- | --- | --- |
| 1 | `DATA-17` | Chunked, resumable transport; 10 files picked at once; progress per file | `0090` |
| 2 | `DATA-17` | The job streams; Parquet read natively by the job; the large-file harness | none |
| 3 | `DATA-18` | Column types end to end | `0091` |
| 4 | `DATA-17` | JSON, JSON Lines, Avro, gzip, zip | none |
| 5 | `DATA-17` | Several files into one table | `0092` |
| 6 | `DATA-17` | Measure, set the cap, docs | none |

Why this order:

- **Transport first.** Nothing big can arrive without it, and it changes no load behaviour.
- **Streaming second.** A 2 GB file is useless if the job loads it whole.
- **Types before the new formats.** JSON, Avro and Parquet carry their own types. Built after the column-spec contract exists, they arrive typed; built before it, they would be built as text first and then rebuilt.
- **Several files after formats.** The mismatch check compares columns and types, which slices 3 and 4 define.
- **The measurement last.** It must measure the code that ships.

**No slice 0.** `SEC-22` is another team's item (product owner, 2026-10-19). Slices 2 and 4 do not wait for it: slice 2 takes Parquet reading out of the API altogether, and nothing here touches the workbook conversion.

## 7. Tasks

One task per commit, in this order. Within a slice the Rust tasks sit together,
and a migration lands with the code that uses it. Every check below is the
scoped one ("Keep build time down"); the full block runs once per slice before
its handoff.

### Slice 1: `DATA-17` transport

- **T1.1 Migration `0090_upload_parts.sql` and the store.**
  - `file_upload.status` also allows `receiving`.
  - New `file_upload` columns: `declared_size BIGINT`, `multipart_id TEXT` (server-only, never serialised), `parts_digest BYTEA`.
  - New table `file_upload_part (upload_id → file_upload ON DELETE CASCADE, part_number 1..10000, size_bytes, etag, sha256, received_at, PRIMARY KEY (upload_id, part_number))`.
  - A why-header citing `DATA-17` and AD1.
  - Store functions in `lakehouse-store/src/uploads.rs`, with tests in `lakehouse-store/tests/uploads.rs`: record a part, replace it, list parts, find stale `receiving` rows.
  - Check: `cargo test -p lakehouse-store --test uploads`.
- **T1.2 `UploadStore` multipart.**
  - `create_multipart`, `put_part`, `complete_multipart`, `abort_multipart` through `MultipartStore`, plus a bounded `get_range`, in `upload_store.rs`.
  - Unit tests against `object_store::memory::InMemory`, which implements `MultipartStore` (`memory.rs:422`).
  - Before anything relies on it, prove on the dev stack's RustFS that create, part, complete and abort work and that a part number sent twice keeps the last copy (V1). Quote what was run.
- **T1.3 Routes.** All `connector:manage`, the `{id}` routes behind `require_upload_in_tenants`, and every route in `POLICY_TABLE` and asserted both ways in `tests/route_auth.rs`.
  - `POST /api/uploads/sessions` `{filename, sizeBytes}` → 201 `{upload, partSize, partCount}`.
    - Refused when `sizeBytes` is 0 or above the cap constant, with the existing size sentence.
  - `PUT /api/uploads/{id}/parts/{n}`, raw body.
    - Its own `DefaultBodyLimit` of part size plus 64 KiB.
    - `n` within `1..=partCount`. Every part except the last must be exactly `partSize`.
    - The upload must be `receiving`.
    - The `SEC-17` `ReceiveLimiter` slot is held while the part is received.
    - A part sent again replaces the earlier copy.
  - `GET /api/uploads/{id}/parts` → `{received, partSize, partCount}`.
  - `POST /api/uploads/{id}/complete`.
    - All parts present and their sizes summing to `declared_size`, else 409 listing the missing numbers.
    - Then the S3 complete call, a sniff of the first 64 KiB by range read, and today's `check_file` rules by kind (same sentences; a refused file is aborted and its row removed).
    - `parts_digest` = SHA-256 over the parts' SHA-256s in order. The duplicate check compares it with other chunked uploads.
    - The response has the same shape as `POST /api/uploads` (workbook and Parquet facts included).
  - `DELETE /api/uploads/{id}` on a `receiving` upload aborts the multipart upload.
  - A `receiving` upload cannot be previewed or loaded: 409.
  - An hourly sweep (the `main.rs:848` interval pattern) aborts and removes `receiving` uploads older than `UPLOAD_SESSION_TTL_HOURS`. The default is 24, per `DATA-17` decision 8.
  - No upstream text in any response (principle 4).
  - Tests in `tests/upload_routes.rs`:
    - parts sent out of order;
    - a part sent again;
    - a missing part answered with 409 and the list;
    - a wrong part size;
    - a session over the cap;
    - another tenant's upload answered with 404;
    - no permission answered with 403;
    - preview and ingest of a `receiving` upload refused;
    - the sweep.
  - Check: `cargo clippy -p lakehouse-api --all-targets -- -D warnings`, then `cargo test -p lakehouse-api --test upload_routes --test route_auth`.
- **T1.4 Console uploader.**
  - Client methods `createSession`, `putPart`, `listParts`, `complete` in `src/services/clients/uploads.ts`, through `apiFetch`, typed in `contracts/uploads.ts`.
  - A pure queue in `src/lib/upload-queue.ts` that:
    - cuts files into pieces;
    - runs at most 4 pieces at once across all files (`DATA-17` decision 6);
    - retries a failed piece with backoff (1 s doubling to 30 s);
    - honours `Retry-After` on 429;
    - pauses while `navigator.onLine` is false;
    - re-reads `GET …/parts` before resuming;
    - reports progress per file as acknowledged bytes over size.
  - File step:
    - `multiple`, and drop of up to 10 files (decision 5).
    - One row per file: name, size, bar, state (waiting, sending, waiting for the connection, received, refused with the reason), and remove.
    - The size check before sending counts the total (decision 2).
  - The Check, Table and Review steps carry on with the first file until slice 5. The other received files are loadable from the uploads panel.
  - Fix the stale empty-state sentence at `uploads-panel.tsx:66`.
  - Tests:
    - queue unit tests (concurrency cap, backoff, 429, offline then online, resync skips received pieces);
    - page tests (several files, the 11th refused, a bar moving, a refusal shown).
  - Check: `bun run typecheck && bun run lint`, then `bun test` on `src/lib/upload-queue.test.ts`, `src/services/clients/uploads.test.ts` and `src/features/connectors/` only. One whole `bun test src` process is OOM-killed on this machine.
- **T1.5 Docs at the code.** Module docs of `uploads.rs` and `upload_store.rs` state the protocol and AD1; add a `CHANGELOG.md` entry. The planner writes the ADR 0014 amendment (AD1, AD2) in the same slice.

### Slice 2: the job streams; Parquet natively

- **T2.1 Streamed delimited text in the job.**
  - `J` reads with `fs.open` (8 MiB blocks) through an incremental decoder, not `cat_file` (`J:432`). Python's `csv.reader` stays (ADR 0014 decision 3).
  - Rows are counted as they stream, and Arrow record batches of 50,000 rows go to the sink.
  - The row cap fails the run without writing anything (V4 proves this with dlt).
  - The run config gains `format` and `storage_keys`; `bronze_table_name` stays in place (AD5).
  - Unit tests on an fsspec memory filesystem, with no network:
    - UTF-8 and UTF-16 split across a block boundary;
    - each delimiter;
    - the row cap writing nothing;
    - peak memory independent of file size: a 200 MB generated file stays under a fixed bound, measured with `tracemalloc`.
  - Check: `pytest dispar_orchestrate/test_file_ingest.py`, plus the two `ops/lint` scripts.
- **T2.2 Parquet read by the job.**
  - `pyarrow.parquet.ParquetFile(...).iter_batches()`. Until slice 3, every column is cast to the same text the API's converter wrote today (a shared fixture proves the two agree), so behaviour does not change.
  - Binary and nested columns are refused as today.
  - In the API, `ingest` launches with `format: "parquet"` and the original key, and writes no `.converted.csv` for Parquet. The preview reads the footer and the first row group by range, with its own bounds on what the footer declares (footer length and row-group sizes no larger than the file).
  - Check: the Rust and Python scoped checks for the touched crate and files.
- **T2.3 The large-file harness.**
  - `ops/g9/large_upload.py` generates files at run time: a 2 GiB CSV, 10 Parquet files totalling 1.9 GiB, and a gzipped CSV once slice 4 exists.
  - It sends them through the chunked routes, loads them, and samples peak memory of `lakehouse-api`, the console and the Dagster run (the containers' cgroup `memory.peak`) and the wall time. It prints one table.
  - A `workflow_dispatch` CI job `g9-upload-large` runs it. The job is not in `ci-required`, and it frees runner disk first.
  - Check: one run on a small size (200 MB) in CI or on this VM, output quoted.

### Slice 3: `DATA-18` column types

- **T3.1 The shared rules and fixtures.**
  - `ops/fixtures/uploads/types/`: cases of sample values → guessed type, and of value + type + date format → cast value or "does not fit". The cases cover every rule in `DATA-18` decisions 2–5 and 7, including `007`, `1,234`, `1e3`, ` 12 `, `01/02/2026`, mixed time zones, and a decimal with 10 fractional digits.
  - The planner adds the rules to the ADR amendment.
- **T3.2 API: guess, check, validate, store** (Rust). The developer may split this into two commits.
  - New `upload_types.rs`:
    - guesses from up to 10,000 rows or 16 MiB (decision 1);
    - mirrors `_column_names` (`J:299`), proven by a shared fixture;
    - validates a column list (count, names per decision 8, types).
  - The preview response gains `columnSpecs [{source, name, type, guessedType, dateFormat?, askDateOrder}]` and `sampledRows`.
  - New `POST /api/uploads/{id}/check` `{parse options, columns}` → `{badCount per column, badRows ≤ 100}` on the sample. It goes in `POLICY_TABLE` and `route_auth.rs`.
  - `ingest` accepts `columns` and `onBadValue` (required with `columns`).
  - Migration `0091_upload_column_spec.sql`: `file_upload.column_spec JSONB`, `on_bad_value TEXT`.
  - Append into an existing table:
    - the API reads the table's schema through the catalog;
    - the preview pre-fills names and types from it;
    - `ingest` refuses a list that differs, with a 400 listing the columns (decision 9).
  - Parquet and Excel types are the starting guess (decision 10).
  - No column list means text everywhere (decision 11).
  - Tests: `upload_types` unit tests on the fixtures, plus route tests (guess, check, validate, append refusal, no list means text).
- **T3.3 The job casts** (Python).
  - Vectorised casts per the column list.
  - Arrow types:
    - Integer → `int64`;
    - Decimal → `decimal128(38, 9)`;
    - Boolean → `bool`;
    - Date → `date32`;
    - Timestamp → `timestamp("us")` or `timestamp("us", tz="UTC")`;
    - Text → `string`.
  - dlt hints match: `precision`, `scale`, `timezone` (dlt 1.30 has all three).
  - Bad values are counted per column and the first 100 collected. With `stop`, the run fails before anything is written: a new fixed reason, "Some values do not fit their column's type". With `null`, they load as null. The report object (AD6) is written whenever the count is above 0.
  - Prove V4, V5 and V6 before relying on them.
  - Tests: the shared fixtures, stop writing nothing, null and its report, replace with a changed type.
- **T3.4 API reads the report.**
  - The Settler reads the report object.
  - The upload response gains `badValues {count, byColumn, rows}`, bounded, with no storage key in it.
  - The new failure reason joins the fixed list.
- **T3.5 Console.**
  - Check step: under each column header, a name field and a type select, and a date-order select for Date and Timestamp columns.
  - "Day first or month first?" blocks Next while unanswered (decision 4).
  - The page says "Types guessed from the first N rows".
  - Edits call `check`, debounced. A failing-rows panel shows up to 100 rows, with the null-or-stop choice; "stop" is pre-selected (decision 6).
  - Review lists the names and types. The run view shows the report.
  - Remove every "each column as text" sentence: `upload-table-step.tsx:109`, `upload-review-step.tsx:97`, `:103-104`, `upload-check-step.tsx:158-161`, `:246-250`.
  - Tests next to each file.
- **T3.6 The gate.**
  - `ops/g9/upload_test.py`: a CSV with date, integer, decimal and boolean columns loads typed (read the schema back through the catalog route); a bad value with `stop` leaves the table unchanged; with `null` it loads, with the report.
  - Step 4's "every column text" assertion changes deliberately; the commit says why (rule 2).
  - Run the `g9-upload` sequence and quote its time. If `timeout-minutes` must rise, raise it by the measured increase plus 5.

### Slice 4: `DATA-17` formats and compression

- **T4.1 API preview readers** (Rust).
  - Readers for each format:
    - JSON Lines: the first lines of the sample.
    - JSON array: elements parsed from the bounded prefix.
    - Avro: header and first blocks with `apache-avro`.
    - gzip: a streamed prefix with `flate2`; the inner file is classed after decoding.
    - zip: the central directory by range read with `zip`; entries listed and capped; an archive inside refused.
  - New magic-plus-name rules in `sniff` and `check_file`. The uncompressed size counts toward the cap (decision 2).
  - Object keys become columns in first-seen order. Nested values become JSON text (decisions 11–12).
  - Fixtures go in `ops/fixtures/uploads/`.
- **T4.2 Job readers** (Python).
  - Readers for each format:
    - JSON Lines: a streamed line reader.
    - JSON array: `ijson` items.
    - Avro: `fastavro`.
    - gzip: `gzip.open` over the stream.
    - zip: `zipfile` over the seekable `s3fs` file, each entry streamed.
  - All of them feed the slice-3 caster.
  - An uncompressed-byte counter fails a bomb and never truncates.
  - Add `fastavro` and `ijson` to `dagster/pyproject.toml`, pinned, each with a comment saying why (AD8).
- **T4.3 Console.** Update the accept list and the refusal lists (`src/lib/uploads.ts:231-256`, `upload-file-step.tsx:15-25`). A zip's files show as rows of the file list.
- **T4.4 The gate.** One small fixture per format loads and reads back: a JSON array, JSON Lines, Avro, `.csv.gz`, and a zip of three CSVs. A gzip bomb is refused.

### Slice 5: several files into one table

- **T5.1 API.**
  - Migration `0092_upload_loaded_with.sql`: `file_upload.loaded_with UUID`.
  - New `POST /api/uploads/{id}/compare` `{alsoUploads}` → `{mismatches: [{uploadId, file, missing, extra, typeConflicts}]}`. It goes in `POLICY_TABLE` and `route_auth.rs`.
  - `ingest` accepts `alsoUploads` (≤ 9, same tenant, same format, not `receiving`) and re-checks, refusing with the same list (decision 4).
  - The run config's `storage_keys` lists every file. The Settler settles every row with `loaded_with` = the primary.
  - The lineage label (`routes/lineage.rs`, D5 of `DATA-19`) reads "<first file> and N more". This is a `DATA-19` code path: the reviewer checks its tenant rule again (`SEC-35`).
- **T5.2 Job.** Read `storage_keys` in order through the same caster; columns are mapped by name; the row count is the sum.
- **T5.3 Console.**
  - The Check step shows the first file's preview, and a file list with a mark or the mismatches for each file.
  - Load stays off until mismatched files are removed. Review says "N files into `bronze.<table>`"; one run view.
  - A zip's files go through the same path.
- **T5.4 The gate.** Three CSVs, one with its columns in another order, load into one table with every row; a fourth with an extra column is refused with the list.

### Slice 6: measure, set the cap, docs

- **T6.1 Measure.** Run `g9-upload-large` at full size: a 2 GiB CSV, 10 Parquet files totalling 1.9 GiB, a 2 GiB JSON Lines file, a gzipped CSV. The planner writes `docs/plans/DATA-17-RESULT.md`: peak memory per process, wall time, runner size, and anything that fell short.
- **T6.2 Set the cap.**
  - Set `MAX_UPLOAD_BYTES` (Rust, for sessions) and `src/lib/uploads.ts` to the measured cap, citing the RESULT doc. `POST /api/uploads` keeps 50 MiB.
  - Remove `MAX_ROWS` (`J:100`) if memory was flat in rows; otherwise keep it and write the reason down (decision 10).
- **T6.3 Docs.**
  - Developer: `CHANGELOG.md` and module docs.
  - Planner: the feature pages' limits, `PRODUCT.md`, `BACKLOG.md` statuses, the ADR's verification section, `upload-file.md`'s stale lines (`:187-189`), and `uploads-feed-pipelines.md`'s status.
- Then the full verification block (`AGENTS.md`), and the handoff.

## 8. Verify, do not assume

Each one is proven before the task that relies on it, and quoted in the handoff.
A "no" stops the task and is reported to the planner.

1. RustFS accepts S3 multipart create, part, complete, abort and list through `object_store` 0.14, with 8 MiB parts, and a part number sent twice keeps the last copy.
   **Answered for RustFS by the planner, 2026-10-16, with `botocore` (not yet through `object_store`).** The probe ran against a throwaway `rustfs/rustfs:1.0.0-rc.4` container (the compose image) with run-time credentials; the container was removed afterwards. Results:
   - Parts sent out of order (2, then 1, then 1 again) completed. The object held the **last** copy of part 1 (SHA-256 compared), and `list_parts` showed `(1, 8388608), (2, 3145745)`.
   - A ranged read of the first 64 KiB of the completed object matched.
   - A non-last part of 1 MiB was refused at completion with `EntityTooSmall`, so pieces must be at least 5 MiB, and 8 MiB is right.
   - After `abort`, `list_parts` answered `NoSuchUpload`, the object was absent, and the upload no longer appeared in `list_multipart_uploads`.
   - Completing with a part number never sent was refused with `InvalidPart`.

   What is left for T1.2 is the same sequence through `object_store`'s `MultipartStore`.
2. The Next.js rewrite passes an 8 MiB `PUT` with an `application/octet-stream` body unchanged (`next dev` against the API).
3. Peak memory of the console's proxy while 4 pieces are in flight. The proxy keeps a copy of each body, by reading: expect about 4 × 8 MiB per user, and measure it.
4. dlt 1.30: a generator that raises during extract leaves the table untouched, and the next run starts clean (`_abort_pending`, `adapters/sink.py`).
5. Replace with a changed column type creates no variant column (`x__v_bigint`, the dlt behaviour behind `SRC-15`). If it does, stop. The options to report are an atomic pyiceberg schema change plus overwrite in one transaction, or a refused type change.
6. dlt writes Arrow `decimal128(38, 9)`, `date32` and both timestamp kinds to Iceberg as declared. Read the schema back from the catalog.
7. Trino and ClickHouse read those columns through the query route.
8. The masking rewrite works on a typed upload column, run with a real policy and a restricted login (the Phase 2 lesson: not a hand-written query of the same shape).
9. The `DATA-19` sensor still fires after a typed load and after a load of several files.
10. The vectorised `pyarrow` casts give the shared fixtures' answers exactly.
11. Free disk in `dagster-code-location` covers dlt's working files for a 2 GiB load.

## 9. Out of scope

- Files on a schedule (spec "Not included").
- Presigned direct-to-storage upload.
- Resuming after a page reload (`DATA-17` decision 7).
- Several sheets in one load.
- ORC, XML, `.xlsm`, `.xlsb`, `.ods`.
- Nested values spread into child tables.
- Changing a column's type after loading.
- The assistant (`AI-16`; both specs say so).
- `BLD-4` (connector upstreams firing on every load), and `DATA-19`'s acceptance.

## 10. Rules for the whole phase

- **One branch, one PR.** The planner opens the draft PR on the first push, which needs the user's go-ahead, and keeps it current. Merging is not this session's job.
- **Merge `main` before each slice** and whenever it moves meaningfully. Resolve conflicts hunk by hunk, and run the scoped compile check after a merge that touched code.
- **Migrations `0090`–`0094` are reserved** for this phase. Taken elsewhere: `0076` and `0080` on `main`, `0077` (`BLD-2`), `0078`, `0079`, `0082`, `0083` (Phase 2, #106), and `0081` (`DATA-14`). After each merge, `ls rust/migrations | cut -c1-4 | sort | uniq -d` must print nothing.
- **One cargo job at a time on this VM** (11 GB RAM, shared). Check `ps` for cargo first. Use a shared `CARGO_TARGET_DIR` with `CARGO_INCREMENTAL=0`; touch the crate's sources before clippy when several worktrees share the target.
- **Before each slice, check other sessions** (`git log`, `git worktree list`, `ps`) for upload work started elsewhere.
- **Browser QA by the product owner once at the end**, on a QA instance, then the acceptance checklists.

## 11. Handoff

(Developer, per slice: commits, the exact commands with their counts, and what
was not verified, with the reason.)

### Slice 1

Branch `feat/data-phase-3-uploads`, five task commits on top of `7a2fd421`, in
plan order. Nothing pushed.

| Task | Commit |
| --- | --- |
| T1.1 migration `0090` and the store | `fc0e79eb` |
| T1.2 `UploadStore` multipart and ranged reads | `a8be24be` |
| T1.3 routes, sweep, session lifetime | `3cd0bd28` |
| T1.4 console uploader | `eef458df` |
| T1.5 docs at the code | `193d9d81` |

Environment: `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target`,
`CARGO_INCREMENTAL=0`, `-j 4`, one cargo job at a time. `ls rust/migrations | cut -c1-4 | sort | uniq -d` printed nothing.

**What was run (all in the foreground, from this branch's sources; the sources were `touch`ed before the clippy runs because the target dir is shared):**

- `cargo fmt --check` (workspace): clean.
- `cargo clippy -j 4 --workspace --all-targets --all-features -- -D warnings`: clean (`Finished` after a `touch` of every `crates/*/src/**.rs`).
- T1.1: `cargo test -p lakehouse-store --test uploads`: **51 passed**, 0 failed (14 are new: the session row and its wire form, the zero-byte refusal, pieces out of order and again, only a `receiving` upload takes a piece, finishing, deleting, the sweep query, the duplicate digest, `0090` over an earlier upload). The `0058` migration test now stops at 0058 (`migrations_up_to(58)`), because it pins what `0058` alone adds to `file_upload`.
- T1.2: `cargo test -p lakehouse-api --lib upload_store`: 17 passed, 1 ignored. With a live store: `UPLOAD_PROOF_ENDPOINT=… UPLOAD_PROOF_ACCESS=… UPLOAD_PROOF_SECRET=… cargo test -p lakehouse-api --lib upload_store -- --include-ignored`: **18 passed**.
- T1.3: `cargo test -p lakehouse-api --test upload_routes -- --test-threads=2`: **108 passed** (15 new, listed in the commit); `--test route_auth -- --test-threads=2`: **44 passed** (the upload test now loops the four new routes: an engineer is never refused at the gate, an analyst always is); `--lib -- sessions upload_ routes::tests config::tests::the_upload`: 144 passed, 1 ignored; `--lib -- timeout upload_session config::`: 99 passed. `cargo test -p lakehouse-store --test uploads` again after the `finish_receiving` change: 51 passed.
- T1.4: `bun run typecheck`: clean. `bun run lint`: 0 errors, 8 warnings, none in a file this slice touched (all were there before). `bun test src/lib/upload-queue.test.ts src/lib/uploads.test.ts src/services/clients/uploads.test.ts src/features/connectors/`: **404 passed**, 0 failed, 19 files (`bun test v1.4.0`). 14 of them are the queue's own (concurrency cap and turn-taking, progress, backoff 1 s to 30 s, re-read after a failure, 429 and `Retry-After`, offline and back, a hung request ended when the connection returns, fatal refusal, completion 409, a lost completion answer, remove, snapshot, dispose); the page tests were rewritten for the new flow (10 files, the 11th left out, a bar that moves, a refusal in its row, 413 and 500 from a proxy, 429 waited out, the others still sending).

**The `object_store` / RustFS proof (section 8, item 1), through `UploadStore`:** a throwaway `rustfs/rustfs:1.0.0-rc.4` container, run-time credentials in a file under `/tmp` (deleted afterwards), bucket made with `boto3`, then the ignored test `rustfs_multipart_keeps_the_last_copy_of_a_piece_and_aborts_cleanly` in `upload_store.rs`: create; part 2 (3 MiB), part 1 (8 MiB, wrong bytes), part 1 again (right bytes); complete with the last ETags; the object is 11 MiB, the first 8 MiB are the **last** copy of part 1, and a 4-byte ranged read across the join is right. Create, part 1 (6 MiB), abort, abort again: both succeed (`NotFound` is mapped to success) and the object is absent. Parts of 1 MiB and 1 MiB: completion is **refused** (the 5 MiB minimum). The container was removed afterwards (`docker rm -f`); the shared dev stack was not touched. The test keeps the credentials out of the repo (it reads three environment variables).

**Section 8, item 2 (the Next.js rewrite passes an 8 MiB `PUT` unchanged):** `next dev --webpack` (Turbopack refuses the symlinked `node_modules` this worktree uses) with `RUST_API_URL` pointing at a stdlib echo server that hashes what it receives. An 8,388,608-byte random file, `Content-Type: application/octet-stream`, 200; the server saw the same length and the same SHA-256 and the same content type, and no `Transfer-Encoding`.

**Section 8, item 3 (memory of the console's proxy with 4 pieces in flight):** the same `next dev` process, resident set before 345,296 KB, peak 418,984 KB over three rounds of four simultaneous 8 MiB `PUT`s (about +72 MB). It is a development server, one run, sampled every 0.1 s: **not verified for the production (`standalone`) build**, and it is not the slice 6 measurement.

**Not verified, with the reason:**

- `cargo test --workspace` (the full block's test line): **not run to the end.** The root disk reached 100% (1.5 GB free of 293 GB; the shared target holds 24 GB of test binaries of 750 to 930 MB each) and `cc` crashed while linking `agents_run`, `agents_approval` and `connector_discovery_launch`. I stopped it rather than fill the disk another session shares. What did run is listed above; the other crates' tests were not run by me. Someone should free disk before the reviewer's run.
- `bun run test` (the whole suite) and a production `next build`: not run (a whole `bun test src` is OOM-killed on this machine; the plan names the scoped files).
- Python lints and `pytest`, `docker compose config`: no Python, compose or ops file was touched.
- Browser behaviour of the new file step (a real drag and drop, a real connection drop, a real 8 MiB piece through the proxy to the API): not seen in a browser. The routes were exercised through the Rust route tests against a fake bucket that speaks S3 multipart, and the page through happy-dom with a fake `fetch`.
- One combined run, `cargo test -p lakehouse-api --test upload_routes --test route_auth`, showed 6 failures in `upload_routes` (the same binary alone passed 108 of 108, three times, including after the last code change). I did not find the cause; the failing tests were the timing-sensitive and large-body ones while another session's `cargo` was compiling on the same machine. Treat as unproven flakiness under load and look at it in the review run.

**Where the code differs from, or adds to, the plan (none changes a decision):**

1. `finish_receiving` (T1.1) no longer sets `sha256`: a file never held whole has no hash of its bytes, and writing the digest of the pieces into `sha256` would be a different quantity under the same name. The duplicate check reads `parts_digest` (`find_by_parts_digest`). The T1.1 commit has the function with a `sha256` argument; T1.3 removes it (the commit says so).
2. The sweep is a module of its own, `upload_sweep.rs` (in `lib.rs` and `main.rs`), not code in `main.rs`, so a route test can call it; `main.rs` only spawns it (`spawn_upload_session_sweep`, hourly, first tick at start). It also deletes the object at the session's key, tolerating its absence, for a process that died between S3's "complete" and the row's move to `uploaded`.
3. The handlers live in a child module, `routes/uploads/sessions.rs`, to keep `uploads.rs` from growing by 600 lines. `classify_head` is `check_file`'s kind rules split out so both paths give the same sentences.
4. `PUT /api/uploads/{id}/parts/{n}` takes the request, not a `Bytes`, so the `SEC-17` place is taken before the body is read; it has the 300 s request timeout the single upload has (the default 60 s would cut a piece on a link under about 1 Mbit/s). `is_upload_part_path` in `routes/mod.rs`, with a test.
5. `POST .../complete` reads a workbook or a Parquet file **whole** (as `POST /api/uploads` does today, under a `SEC-17` place) and refuses one above `MAX_UPLOAD_BYTES`; that is where decision 9 (Excel stays at 50 MB) holds when slice 6 raises the session cap, and the Parquet read is what slice 2 removes. A text file is classed from its first 64 KiB.
6. `MAX_SESSION_BYTES` is its own constant (50 MiB) so slice 6 changes one line without touching `POST /api/uploads`.
7. `UPLOAD_SESSION_TTL_HOURS` (default 24, config error if not a whole number of 1 or more) is read from the process environment but **is not passed through in `docker-compose.yml` or `.env.example`**: rule 8 asks a real `docker compose up` for a compose edit, which I did not run. A container runs with 24 until that is added.
8. The queue and the page start sending when files are picked, not when Next is pressed; Next opens when the first file (the first not refused) has been received. The others keep sending in the background and the page says so and where to load them.

**Things I saw and did not change (for the planner):**

- A user can open any number of sessions (each is a row and an S3 multipart upload) before the 24 h sweep removes them; the plan sets no per-user bound.
- If S3's "complete" succeeds and the database write that follows fails, the retry of `complete` meets `NoSuchUpload` (503) until the sweep removes the session; the queue recovers the case where only the *answer* was lost (it reads the upload and takes it if it is `uploaded`).
- `connector-create-page.tsx` still says "Have a file instead? Upload a CSV or TSV" (the plan asks only for the sentence at `uploads-panel.tsx:66`).
- Running `next dev` regenerated `AGENTS.md` in the worktree (Next's agent rules). I reverted it with `git checkout`; the file is not in any commit. The worktree has a `node_modules` symlink to `/home/hv/lakehouse-phase3-build/node_modules` (git-ignored), used read-only; it is removed with this handoff.

### Slice 2

Review SHOULD-FIX items 1 to 4 of slice 1 (round 1), then T2.1 to T2.3, one commit each, on top of `1ee184f9`. Nothing pushed. Environment as slice 1; `df -h /` before every build (it stayed between 9.8 and 6.5 GB; once below 8 GB I ran the prescribed `find … -size +50M -delete` on our target's test executables and nothing else).

| Item | Commit |
| --- | --- |
| Review SHOULD-FIX 1: at most 20 open sessions per user (429, fixed sentence, `Retry-After: 30`) | `3c58526f` |
| Review SHOULD-FIX 2: `complete` recovers when S3 assembled the file but the row was not updated | `e73dc744` |
| Review SHOULD-FIX 3: the connector page's "Upload CSV, TSV, Excel or Parquet files" | `33876a3b` |
| Review SHOULD-FIX 4: `UPLOAD_SESSION_TTL_HOURS` passthrough in compose and `.env.example` | `7d835871` |
| T2.1 the job streams delimited text | `dae916db` |
| T2.2 Parquet read by the job; footer and first row group by range in the API | `89954d60` |
| T2.3 the large-file harness and its manual workflow | `3ed7b448` |

**What was run (foreground, this branch's sources):**

- Rust: `cargo fmt --check` clean; `cargo clippy -j 4 --workspace --all-targets --all-features -- -D warnings` clean (after touching every `crates/*/src/**.rs`); `cargo test -p lakehouse-store --test uploads` **52 passed**; `cargo test -p lakehouse-api --test upload_routes -- --test-threads=2` **110 passed** (run alone: an earlier run of the same binary had 5 failures that were the harness Postgres container running out of shared memory, `No space left on device` in `dsm_impl`, and the rerun passed); `cargo test -p lakehouse-api --lib -- upload_ routes:: sessions` **1502 passed**, 15 ignored; `--lib -- upload_parquet` 34 passed (7 new in `upload_parquet.rs`, 4 in `upload_parquet_range.rs`); `--lib -- run_config converted_object` 21 passed.
- Python (Dagster venv): `pytest dispar_orchestrate/test_file_ingest.py dispar_orchestrate/test_parquet_ingest.py` **237 passed**, 1 skipped (the 200 MB test, opt-in); with `FILE_INGEST_LARGE=1 … -k 200_mb`: **1 passed** (93 s). The whole `pytest dispar_orchestrate` was started once: **1357 passed, 1 skipped, no failure**, then my 590 s timeout stopped it (the suite is longer than that), so it is *not* a complete run. `python3 ops/lint/check_intra_package_imports.py` and `check_bare_iceberg_count.py`: both OK. `pytest ops/g9/test_large_upload.py`: 5 passed.
- Console: `bun test src/features/connectors/connector-create-page.test.tsx` 7 passed; `bun run typecheck` clean (with the borrowed `node_modules` symlink, removed afterwards).

**T2.1 measurements.** With dlt 1.30.0 into a local filesystem destination (a unit test, `test_a_failure_while_dlt_extracts_leaves_the_destination_untouched_and_the_next_run_starts_clean`): a generator that raises inside extract (the row cap) leaves the destination's files exactly as they were, and the next run replaces cleanly (V4 answered for dlt itself; **the Iceberg path through `load_via_sink`, with `_abort_pending`, was not run here**). Arrow tables with `text` hints and the sink's stamp (`_ingested_at` as `timestamp[us, UTC]`) load through dlt. `tracemalloc`, production sizes, a generated 200 MB CSV (1,940,172 rows): peak **62.3 MB**, 76 s under tracemalloc; the always-on test shows a 24 MB file peaking no higher than a 6 MB one (it uses 1 MiB blocks and 5,000-row batches to reach the plateau quickly). A 200 MB CSV and a 200 MB Parquet file generated by the harness have 1,747,626 and 1,400,000 rows, under the cap.

**T2.2 proof of what the API asks of the store.** A recording `ChunkReader` over a four-row-group file: `inspect` asks only for the magic and the end of the file; a 20-row preview asks for nothing between the end of the first row group and the footer (`the_footer_alone_is_read_to_inspect_a_file_and_the_first_row_group_to_preview_it`). Forged inputs: a footer length of `u32::MAX` or the file's own length, files under 12 bytes, and column-chunk offsets shifted past the data (100 bytes dropped from the front) are `Unreadable`, no panic. The text of every value is pinned by `ops/fixtures/parquet/types.expected.json`, which a Rust test (`convert` writes it) and a Python test (the job's reader writes it) both read; orders and the legacy `INT96` file are pinned in the Python test as in the Rust one.

**T2.3.** `ops/g9/large_upload.py` and `.github/workflows/g9-upload-large.yml` (separate workflow, `workflow_dispatch` only, not in `ci-required`; inputs default to 200 MiB so the first dispatch is the 200 MB run; the full size is 2048 and 1945).

**Not verified, with the reason:**

- **The harness has not been run against a stack, at 200 MB or any size.** This machine has the shared dev stack and about 10 GB of free disk; bringing up Lakekeeper, RustFS, ClickHouse and Dagster here would touch what other sessions use, and I cannot dispatch a GitHub workflow without pushing. What ran: its generators (200 MB CSV in 7.6 s, 200 MB Parquet in 7.8 s), its piece sender against a loopback stub of the routes, its report, and the job's readers on the generated files. The first dispatch is the real check of the harness, of `docker exec … cat /sys/fs/cgroup/memory.peak` on the runner, and of the compose passthrough of `UPLOAD_SESSION_TTL_HOURS` (the workflow asserts the API holds the value it was given; `docker compose config` here needs secrets this shell does not have).
- **A Parquet or CSV load into Iceberg through the real sink was not run** (needs the stack). The sink change (`_stamp_ingested_at` accepting Arrow) is covered by unit tests and by the dlt run above, not by an Iceberg write; `ops/g9/upload_test.py` is the gate that will show it.
- The whole `pytest dispar_orchestrate` (see above) and `cargo test --workspace` (slice 1: the disk) were not completed.

**Where the code differs from, or adds to, the plan:**

1. `run_config` and `FileIngestParams` replace `storage_key` with `storage_keys` (a list; the job refuses more than one key until slice 5) and add `format` (`delimited`, `parquet`), as AD5; `bronze_table_name` has not moved. `test_authored_factory.py`'s two copies of the config were updated.
2. `pyarrow==25.0.1` is a new pin in `dagster/pyproject.toml` (already installed transitively; the job imports it). Not in AD8's list; named so the planner can veto.
3. Sink: `_stamp_ingested_at` handles Arrow tables and record batches (T2.1 needs it; the plan did not name it). It replaces a column of the same name.
4. Parquet text is written in `parquet_ingest.py` (a new module) rather than in `file_ingest.py`; the float rule is a port of `float_text` (`0.1` for a 32-bit float is found by trying 1 to 9 digits).
5. `MAX_CELLS` (5,000,000) still applies at `inspect` for Parquet, and `convert` stays in `upload_parquet.rs` (the preview's writer and the fixture's oracle). Nothing needs the cap for memory any more; the plan did not say to remove it, so I left it for slice 6 to judge.
6. A preview whose first row group is over 64 MiB shows the columns and no rows (`MAX_PREVIEW_GROUP_BYTES`); the footer is bounded at 16 MiB (`MAX_FOOTER_BYTES`). Both are mine; the plan asks only that the footer's declarations are no larger than the file.
7. `ingest` for Parquet now checks the footer (opens, has rows, at most 1,000 columns) before the claim, as the conversion did.
8. The harness workflow is its own file, not a job of `ci.yml`: on a manual run `ci.yml`'s change detector has nothing to compare and every other job would start.
9. Session cap: counted before the multipart upload is opened, not atomic with the insert (two simultaneous requests can overshoot by one); the comment at the constant says so.

### Slice 3

Developer (Claude Sonnet 5.5), branch `feat/data-phase-3-uploads`, `DATA-18` T3.1 to T3.6. The run was
stopped once by a rate limit and resumed; another session had touched the worktree meanwhile.

**Commits.** `e70723b7` T3.1 shared fixtures; `657a9ce5` T3.2 guess, check route, column list on ingest
(migration 0091); `b9592280` T3.3 job casts, counts, writes the report (and the retype in the sink);
`383f1046` T3.4 the settled load carries the report; `89a22b7f` T3.5 wizard; `c6bc8f8c` T3.6 gate; plus this entry.

**Kept from the other session's work.** The T3.1 fixtures, migration 0091, `columns.rs`, and the staged type
rules and tests, after reading every hunk. Changed: its SQL bound the spec and on-bad-value to `$6,$7`
(jsonb against text type error), now `$5,$6`, covered by a store test; the call to a removed
`record_column_spec` is replaced by the atomic claim fields (`columns::claim_fields`) so the column list is
stored in the same statement that claims the upload; clippy fixes (`assigning_clones`, `too_many_lines`).

**Section 8 items proved locally** (local filesystem destination, as for V4; the real Iceberg write against
Lakekeeper is CI-only, *not verified* here). Item 5: dlt declares the types as given and there is no variant
column, but pyiceberg refuses a replace that changes a column type, so (after review BLOCKER 1, below) the sink
writes such a replace as one pyiceberg transaction (`_replace_changing_types`) and never drops the table. Items 6 and 10:
covered by `test_typed_load.py` and the Parquet/Excel starting-type tests (decision 10).

**Commands run, with counts.** `cargo test -p lakehouse-store --test uploads` 53 passed;
`cargo test -p lakehouse-api --test upload_routes` 118 passed; `--test route_auth` 44 passed;
`cargo test -p lakehouse-api --lib upload_` and `columns` 201 passed; `cargo clippy -p lakehouse-api -p
lakehouse-store --all-targets -- -D warnings` clean; pytest `test_file_ingest.py`, the Parquet, typed-load
and type tests about 330 passed; bun on the queue, lib, client and connectors test files 419 passed;
`bun run typecheck` clean; both `ops/lint` scripts OK; `py_compile` on `ops/g9/upload_test.py`.

**Additions beyond the plan text.** `file_upload.bad_values` column (report kept with the upload); `float`
wire type (from Parquet); a `timezone` flag on timestamp columns; `?table=` on the preview (columns of the
table for append, decision 9); stricter name rule `^[a-z][a-z0-9]*(_[a-z0-9]+)*$`, at most 128, not `_dlt...`;
the ninth failure reason "Some values do not fit their column's type."; the choices on trimming and leading
zeros (text is never trimmed or coerced, so `007` stays text).

**T3.6 gate.** The "every column text" assertion is changed on purpose (see the commit): the load now sends the
guessed list and each column is checked against its type; a new step covers typed load, `stop`, `null`, and
the replace back to text.

**Slice 3, round 1 fix (Phase 3 slice 3 review BLOCKER 1).** The purge is gone. A `replace` whose column
types differ from the table's is written by `_replace_changing_types` (`adapters/sink.py`) as one pyiceberg
transaction: per changed column `update_schema` delete and add (the type is parsed from Iceberg's own name),
a second `update_schema` moves each back to its place (`move_first`/`move_after`, so the file's column order
is kept), then the batches are written inside the same transaction, the first group with `overwrite`, the next
with `append`. Groups are gathered up to 64 MiB (`RETYPE_FLUSH_BYTES`), so the file is never held whole; a
`replace` with the same types, and every other mode, still goes through dlt. Nothing is dropped or purged
(a test asserts `purge_table` and `drop_table` are never called). A failure inside the transaction, the
stream's own stop included, commits nothing; data files already written stay in the bucket unreferenced, for
the orphan clean-up. The table's dlt schema in the bucket keeps the old types until the next dlt load, which
always declares its types and those win (section 8 item 5: no variant column). The result carries
`columns=()` for this path (nothing guessed). New tests in `test_typed_load.py`: one commit with the new
schema, rows and column order and all earlier snapshots kept; a failure in the stream or in the cast leaves
the schema, rows, current snapshot and snapshot list equal; `load_via_sink` takes the transaction path for a
type change and dlt for the same types. Commands: `pytest` file-ingest, Parquet, typed-load, upload-types,
authored-factory, sink, sink-history, sink-refresh, sink-empty-table 520 passed, 1 skipped; both `ops/lint`
scripts OK. *Not verified*: the transaction against Lakekeeper's REST catalog (the g9 "replace back to text"
step exercises it in CI; if the REST catalog refuses it, stop and report, do not fall back to a drop).
Snapshots: one `overwrite` (a delete and an append at most) per write group.

**Not verified.** The gate and the large-file harness against a real stack; the Lakekeeper Iceberg write;
`cargo test --workspace` and the full pytest and `bun test` (whole-suite runs are not possible on this machine);
the `g9-upload` job `timeout-minutes: 25` (may need a measured raise); browser QA of the wizard.

## 12. Review

### Slice 1, round 1 (planner/reviewer, Claude Opus), at `159f8dea`, 2026-10-19

**Re-run by the reviewer** (`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target`, crates touched first):

| Command | Result |
| --- | --- |
| `cargo fmt --check` | exit 0 |
| `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -j 4 -- -D warnings` | exit 0 |
| `cargo test -p lakehouse-store --test uploads` | 51 passed |
| `cargo test -p lakehouse-api --lib -- upload_ config::` | 223 passed, 1 ignored (the live-RustFS test) |
| `cargo test -p lakehouse-api --test upload_routes -- --test-threads=2` | 108 passed |
| `cargo test -p lakehouse-api --test route_auth` | 44 passed |
| `bun test` on the queue, `src/lib/uploads`, the uploads client and `src/features/connectors/` | 404 passed |
| `bun run typecheck` | **not verified by the reviewer.** 8 errors, all in `src/features/landing/scene/*`: `three` is missing from the borrowed `node_modules` (a stale install of the main checkout), which is not this diff. The developer reports a clean run; CI decides |

**Reading:**

- The `{id}` routes sit in `per_upload`, behind `require_upload_in_tenants`.
- All four routes are in `POLICY_TABLE` with `connector:manage` and are asserted in `route_auth.rs`.
- The piece route has its own 8 MiB + 64 KiB body limit and takes the `SEC-17` place before it reads the body.
- `multipart_id` is never serialised.
- Parts complete in `part_number` order.
- The migration's why-header covers the CHECK swap and the cascade.
- The console goes through `apiFetch`, with at most 4 pieces in flight; it honours `Retry-After` and resumes on `online`.
- The plan/code differences 1–3 and 5–6 in the handoff are accepted: no file hash for a file never held whole, the sweep in its own module, the 300 s timeout per piece, files start when picked, and `complete` opens a workbook or Parquet file whole up to 50 MiB until slice 2.

No `BLOCKER`.

**`SHOULD-FIX`, to land with slice 2:**

1. **Cap open sessions per user.** A user can open any number of `receiving` sessions before the 24 h sweep. Refuse a new session past 20 open ones per user with a 429 and a fixed sentence; decision 5 needs 10.
2. **`complete` must recover from its own half-success.** When S3's complete succeeded but the database write after it failed, a retry answers 503 until the sweep removes the file. On retry, when the multipart upload is gone and the object exists with the declared size, carry on from the sniff.
3. **The stale "Upload a CSV or TSV" text in `connector-create-page.tsx`** (the developer's note).
4. **`UPLOAD_SESSION_TTL_HOURS` passthrough** in `docker-compose.yml` (`${UPLOAD_SESSION_TTL_HOURS:-}`, with a comment) and `.env.example` (no value). The g9 CI job's `compose up` proves it (rule 8).

**Noted:** the one combined run with 6 failures did not recur in the reviewer's runs. Watch for it in CI.

### Slice 2, round 1 (planner/reviewer, Claude Opus), at `4d4431ba`, 2026-10-19

**Re-run by the reviewer:**

| Command | Result |
| --- | --- |
| `cargo fmt --check` | exit 0 |
| `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -j 4 -- -D warnings` | exit 0 |
| `cargo test -p lakehouse-store --test uploads` | 52 passed |
| `cargo test -p lakehouse-api --lib -- upload_` | 144 passed, 1 ignored |
| `cargo test -p lakehouse-api --test upload_routes -- --test-threads=2` | 110 passed |
| `cargo test -p lakehouse-api --test route_auth` | 44 passed |
| `pytest test_file_ingest.py test_parquet_ingest.py test_authored_factory.py` (the `DATA-19` sensor included) | 342 passed, 1 skipped |
| `ops/lint` (both scripts) | OK |
| `pytest ops/g9/test_large_upload.py` | 5 passed |

**Reading:**

- The four slice-1 `SHOULD-FIX` items are done, each cited at its fix site.
- The run config replaces `storage_key` with `storage_keys` and adds `format`. `bronze_table_name` stays where the `DATA-19` sensor reads it, and its tests pass.
- The Parquet preview bounds the footer length, the chunk offsets and the row-group sizes before it reads them.
- The `pyarrow==25.0.1` pin is accepted. The job now imports it directly, and the comment says why. AD8 is amended in spirit: one more named dependency. The PR's dependency checks must stay green.
- The separate `g9-upload-large.yml` workflow is accepted, with the developer's reason (on a manual run `ci.yml`'s change detector has nothing to compare).

No `BLOCKER` and no `SHOULD-FIX`.

**Not verified, carried to CI and slice 6:**

- A real Iceberg write through the streamed and Parquet paths (the `g9-upload` job, after the push).
- The 200 MB harness run against a stack (the first manual dispatch of `g9-upload-large`).
- The full workspace `cargo test` and the whole Dagster suite.

**Noted:** the session cap can overshoot by one under two simultaneous requests. This is accepted: the cap guards against abuse, not exact accounting.

### Slice 3, round 1 (planner/reviewer, Claude Opus), at `3ed97ae2`, 2026-10-21

**Re-run by the reviewer:**

| Command | Result |
| --- | --- |
| `cargo fmt --check` | exit 0 |
| `cargo clippy -p lakehouse-api -p lakehouse-store --all-targets -j 4 -- -D warnings` | exit 0 |
| `cargo test -p lakehouse-store --test uploads` | 53 passed |
| `cargo test -p lakehouse-api --lib -- upload_ columns` | 201 passed, 1 ignored |
| `cargo test -p lakehouse-api --test upload_routes -- --test-threads=2` | 118 passed |
| `cargo test -p lakehouse-api --test route_auth` | 44 passed |
| `pytest` file ingest, Parquet, typed load, upload types, authored factory | 444 passed, 1 skipped |
| `ops/lint` (both) | OK |
| `bun run typecheck` (with a `node_modules` that has `three`) | exit 0 |
| `bun test` queue, uploads lib, client, `src/features/connectors/` | 419 passed |

**`BLOCKER` 1: a replace that changes a column type purges the table before the new rows are written.**

`_drop_table_if_types_differ` (`adapters/sink.py`) calls `catalog.purge_table` between dlt's normalize and load. If the load then fails, the table, its data files and every snapshot are gone, with no way back, and `DATA-19` pipelines that read the table break. Time travel (`DATA-16`) loses the table's history even when the load succeeds. Section 8 item 5 told the developer to stop and report at exactly this point. Choosing a destructive path is not the developer's call.

**Planner's decision.** Make the type change and the new rows one Iceberg commit.

The planner proved on 2026-10-21 that pyiceberg 0.12.0 does this against a local SQL catalog:

- One `table.transaction()` held `update_schema().delete_column("a")`, then `update_schema().add_column("a", LongType())`, then `overwrite(<new Arrow table>)`.
- It committed atomically. The table read back with `a: long` and the new rows, and all 3 snapshots were kept.
- The column moved to the end of the schema. Use `move_before`/`move_after`, or rebuild the order, so the file's column order is kept.

What the fix must do:

1. **Write type-changing replaces through pyiceberg.** For a `replace` whose column types differ from the table's, write through that kind of pyiceberg transaction (the precedent is `SRC-10`'s own history transaction), not through dlt's load. Keep dlt for every other load. Read the Arrow batches the job already builds, and keep memory bounded: append batches inside the one transaction, or stage them, then overwrite.
2. **Prove it against Lakekeeper's REST catalog.** The g9 step's "replace back to text" already exercises it. In the unit tests, a load that fails inside the transaction leaves the table exactly as it was: same schema, same rows, same current snapshot.
3. **Never purge or drop the table.** If the atomic path proves impossible against Lakekeeper, stop and report. Do not fall back to a drop.
4. **Update the handoff** for section 8 item 5, and the feature page's limit line if anything about history changes (the planner edits `docs/core/`).

**Accepted additions** (listed in the handoff):

- The `file_upload.bad_values` column in `0091`.
- The `float` wire type, for decision 10 ("Floating point (from the file)").
- The `timezone` flag on timestamp columns, for decision 5.
- `?table=` on the preview, for decision 9's pre-fill from an existing table.
- The stricter column-name rule (it matches decision 8).
- The ninth failure reason.
- The fixes to the other session's work: the SQL placeholder numbers and the removed `record_column_spec`.

**Not verified, carried to CI:**

- The g9 typed step against a real stack, and whether `g9-upload`'s 25-minute timeout still holds.
- The real Iceberg write through Lakekeeper.

The branch is pushed for that CI run, with this `BLOCKER` open. It must not merge until the fix is reviewed.

