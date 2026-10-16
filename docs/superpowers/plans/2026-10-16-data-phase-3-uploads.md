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
| State | Decisions signed 2026-10-16: the product owner chose every default on both pages, and `SEC-22` as its own PR to `main` first (slice 0). Ready to build |

## 1. Scope, and what is already done

| Item | What | State at `17867e52` | In this phase |
| --- | --- | --- | --- |
| `DATA-19` | Uploaded tables feed pipelines | **Merged in #165** (2026-10-15), In Acceptance. AC1–AC7 on `uploads-feed-pipelines.md` not run; that page still says "Planned" | No code. The planner corrects the page's status; the product owner runs its checklist |
| `DATA-17` | More formats, more files, bigger files | Building: Excel and Parquet merged in #76 (2026-10-08). The rest is missing (section 2) | Slices 1, 2, 4, 5, 6 |
| `DATA-18` | Column types on upload | Planned, nothing built | Slice 3 |
| `SEC-22` | Workbook and Parquet conversion can exhaust the API's memory (P0, Todo) | Open at the code (delivery review, 2026-10-12) | **Prerequisite**, section 6, slice 0 |

`DATA-19` needs nothing from this branch, so Phase 3 is `DATA-17` + `DATA-18`,
with `SEC-22` in front: it is the same code path, and a 2 GB upload must not
inherit an open P0 memory hole.

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
- **AD2. The job is the only reader of a whole file, and it streams. The API reads only bounded samples.** The preview and the type guess read a bounded prefix with range reads: the Parquet footer plus the first row group, the first Avro blocks, the first lines of JSON Lines, a gzip stream's first bytes, a zip's central directory. The job reads every format natively, in batches, and hands Arrow record batches to the sink. **Parquet stops being converted in the API.** Excel keeps its conversion in the API, at 50 MiB, bounded by `SEC-22`: a workbook must be opened whole to list its sheets, so a streamed path would not help.
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
| 0 | `SEC-22` | Bounded workbook and Parquet reading in the API (own plan) | none expected |
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

**Slice 0, `SEC-22`: its own small PR to `main` first (the product owner's choice, 2026-10-16).** It is P0 and already public in `BACKLOG.md`, and this phase will take weeks. Its fix (workbook reading that cannot end the API process, bounded footer sizes, a limit on conversions at once, no full re-conversion per preview) is the base slices 2 and 4 build on. It gets its own plan. If the product owner prefers it inside the phase PR, it becomes that PR's first commits, and nothing else in this plan changes.

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
  - In the API, `ingest` launches with `format: "parquet"` and the original key, and writes no `.converted.csv` for Parquet. The preview reads the footer and the first row group by range, with the size checks `SEC-22` adds (or adds here, citing `SEC-22`, if slice 0 has not merged).
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

## 12. Review

(Planner, per slice: findings tagged `BLOCKER` or `SHOULD-FIX`.)
