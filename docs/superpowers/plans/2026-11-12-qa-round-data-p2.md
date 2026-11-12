# QA round, Data (P2): uploads — long table names, the upload page, people by name, types

Fixes for four backlog rows opened by the QA round on staging (2026-11-02 to 11-12):
`DATA-50` (under `DATA-9`, case `LH-DATA-9-37`), `DATA-51` (under `DATA-17`, `DATA-35`),
`DATA-52` (under `DATA-12`, `DATA-15`) and `DATA-53` (under `DATA-9`, cases `LH-DATA-9-32`,
`LH-DATA-9-07`).

Base: `main` after pull request #484 (`fix/qa-round-data-p1`) is merged. Every anchor below
was read at `d6d22a2c5`, the head of that branch, which already changed the upload page, the
uploads API and the Gold publish code. If a line has moved after the merge, find it by the
text quoted here; if the code is not what a task says, stop and report the mismatch in the
handoff. Branch: `fix/qa-round-data-p2`. One pull request for the whole slice, opened as a
draft by the planner as soon as T1 is pushed, so CI (x86_64) runs every test.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer pushes
after every commit and does not change a decision below.

The owning features exist and are signed (`docs/core/features/upload-file.md`,
`upload-big-files-and-more-formats.md`, `upload-column-types.md`, `upload-folders.md`,
`asset-page-depth.md`). The decisions below are the product owner's brief for this slice,
made exact by the planner against the code. Two findings of `DATA-53` are what a signed
decision says and are not changed: they are questions 1 and 2 at the end. T16 names every
feature-page line that changes.

What QA saw:

- `LH-DATA-9-37` (fail): a load into a table name of 52, 64, 100 or 128 characters fails with
  "Load failed · The load into the table failed."; 40 and 46 characters load. The page and
  the API accept 128.
- `LH-DATA-17-04` (fail): two files of 1,127,500,000 bytes each: the second is refused
  ("would bring the files to 2.1 GB, over the 2 GB limit for one upload"), but the first is
  already sent (a session is opened and parts are put).
- `LH-DATA-17-13`: "The file is larger than the upload limit once unpacked.", without the
  number. `LH-DATA-17-05`: a damaged Parquet file is answered "This is not a file this upload
  reads. Upload CSV, TSV, JSON, JSON Lines, Avro, Parquet or Excel…".
- `LH-DATA-35-10`: a folder with 105 files shows 100 and no note. `LH-DATA-35-07`: why "New
  subfolder" is off is only in a `title`. `LH-DATA-9-02`: the link on New Connector names
  fewer formats than the page accepts.
- `LH-DATA-12-01` F3, `LH-DATA-15-02`, `TC-1730`: an asset's Usage panel and "Frequent users"
  show a user's UUID. `LH-DATA-15-08`: a version written by an upload has the author
  "unknown".
- `LH-DATA-9-32`: a Parquet `uint64` and a nanosecond timestamp are guessed to a type that
  cannot hold the file's values; an all-null row stores `''` for a text column;
  `decimal(10,2)` and `decimal(20,4)` are stored as `Decimal(38, 9)`; `POST /api/query/run`
  shows a large decimal with other digits than are stored. `LH-DATA-9-07`: "Lands in the
  Catalog as `bronze.<table>`", but `SELECT … FROM bronze.<table>` fails in Query Studio.

## 1. Decisions

- **D1. One task per commit, in the order of section 2.** T1 is the reproduction of
  `DATA-50`. Each commit body cites its QA case IDs and its backlog ID. Each fix site carries
  a short comment naming the case (rule 13), for example
  `# QA LH-DATA-9-37 (DATA-50): …`. Three tasks (T5, T8, T9) change Rust and Python in one
  commit, because one shared file or one run configuration binds the two sides and a test on
  each side reads it: splitting them would leave a commit whose tests cannot pass.
- **D2. No new helper where one exists (rule 4).**
  - Names of people by id: `lakehouse_store::dashboard_access::user_names`
    (`rust/crates/lakehouse-store/src/dashboard_access.rs:257`), one read for a list of ids.
    The audit list does not look names up: it shows the `actor_label` stored with the event
    (`routes/governance.rs:438`), so there is no resolver to share with it.
  - A snapshot's author: `history.snapshot_properties`
    (`dagster/dispar_orchestrate/adapters/history.py:517`), `SinkConfig.author`
    (`adapters/sink.py:415`) and `write_one_snapshot`'s `snapshot_properties`
    (`adapters/iceberg_snapshot.py:108`). All three exist; only the callers change.
  - "This file could not be opened as …": `ParquetFileError::Unreadable`
    (`rust/crates/lakehouse-api/src/upload_parquet.rs:189`), `WorkbookError::Unreadable`
    (`upload_workbook.rs:124`), `Refusal::JsonInvalid` (`upload_formats.rs`), `NOT_AVRO` and
    `NOT_AN_ARCHIVE` (`routes/uploads.rs:278-280`). No new sentence for T4.
  - A Bronze table's name in Query Studio: `trinoBronzeName` (`src/lib/asset-query.ts:35`).
  - The local loader harness: the `lab` fixture and `Lab`
    (`dagster/dispar_orchestrate/test_load_modes_measurement.py:97-151`), imported the way
    `test_snapshot_author.py:21` imports it.
  - The gate's helpers: `_upload_one`, `_settle`, `_delete_upload`, `count_rows`, `_expect`
    (`ops/g9/upload_test.py:1096`, `:824`, `:968`, `:389`, `:281`).
- **D3. No route, permission or migration changes.** `POLICY_TABLE`,
  `tests/route_auth.rs` and `rust/migrations/` (highest `0136`) are not touched.

### DATA-50, a table name of up to 128 characters

- **D4. The cause** (read in the pinned `dlt` 1.30.0, `dagster/pyproject.toml:13`; the
  package is not in the repository, the planner read an installed copy of that version).
  `Pipeline._get_schema_or_create` (`dlt/pipeline/pipeline.py:1503-1509`) answers the
  pipeline's default schema, and when the pipeline has none it builds `Schema(pipeline_name)`.
  `Schema` refuses a name longer than 64 characters (`InvalidSchemaName`,
  `dlt/common/schema/utils.py:74-80`). The pipeline of a table is named
  `bronze_ingest_<table>` (`adapters/sink.py:686`), 14 characters more than the table, so the
  limit is a table name of 50 characters. Measured by the planner with a call that writes no
  file: `dlt.Schema("bronze_ingest_" + "a" * 50)` is accepted, 51, 52, 64, 100 and 128 raise
  `InvalidSchemaName`. That is where QA's 46 passes and 52 fails.
  The call is reached from one place, `history.open_table`
  (`adapters/history.py:399-411`, `pipeline.destination_client()` at `:408`), when the
  pipeline has no schema yet, that is on the first load of a table. An upload always gets
  there: `file_ingest._write` passes `column_types=stream.column_types()`
  (`file_ingest.py:1217-1222`), never `None`, so a `replace` (the page's default) opens the
  table before the load (`adapters/sink.py:1235-1240`). The exception leaves as
  `LoadFailure(LOAD_FAILED)` (`file_ingest.py:1223-1228`), the sentence QA saw. The same call
  fails for a first load in history mode and for clearing a long-named table that was never
  loaded.
- **D5. The fix is in `history.open_table`, and no name is shortened.** When the pipeline has
  no default schema (`pipeline.default_schema_name` is empty after the sync), `open_table`
  answers `None` without asking `dlt` for a client. That is the answer it gives today for
  every shorter name: the schema `dlt` would build knows no table, so `load_open_table`
  raises `DestinationUndefinedEntity` (`dlt/destinations/impl/filesystem/filesystem.py:
  1364-1370`), which `open_table` already turns into `None`. Nothing changes for a name of
  50 characters or fewer.
  The pipeline name `bronze_ingest_<table>` stays as it is. Reason: the cursor, the
  deduplication marker and the stored schema of a table are kept in the bucket under the
  pipeline's name. A connector's first load runs through `pipeline.run` only and does not
  reach the failing call, so a long-named connector table can exist today; renaming its
  pipeline would orphan that state. The name that did hit the limit is one `dlt` derives,
  and for the pipeline's real schema `dlt` already derives a stable shortened name with a
  hash tag (`dlt/extract/utils.py:329-335`; measured: 64 characters, and two 115-character
  pipeline names that differ in one character get different names), so nothing of ours is
  cut into a collision.
- **D6. The other derived names on the load path, as read.** The `dlt` dataset is the
  constant `bronze`. A table or column name in `dlt` may be 65,536 characters for this
  destination (`generic_capabilities`, `dlt/common/destination/capabilities.py:278-279`).
  The pipeline name is checked as a file name component only, and its length is not judged
  (`validate_file_name_component`). The upload launch sends no run tags
  (`routes/uploads.rs:2828-2832`, `launch_run_with_config`); the job, op and op tags are
  constants (`file_ingest.py:1403-1413`). There is no staging table. Object keys hold the
  table name once, far under S3's 1,024 bytes. `file_upload.bronze_table` is `TEXT`
  (`rust/migrations/0057_upload.sql:64`); the registry rows are strings. What the planner
  could not read: the catalog server's and the query engine's own limit on a table name
  (neither is in the repository). The gate case of T3 is the proof for those.
- **D7. If a second limit shows.** If T1's tests fail with another exception than
  `InvalidSchemaName`, or T3's gate case fails after T2, the developer writes down the
  exception or the upload's recorded reason and the Dagster run's log line, and stops that
  task. No limit is lowered and no name is cut to get green.

### DATA-51, the upload page

- **D8. The total of a selection is checked before any file of it is sent** (a). `planPick`
  (`src/lib/uploads.ts:309-327`) adds up the files already listed that were not refused and
  every file of the selection that has no problem of its own. When that passes 2 GiB, every
  such file of the selection is listed as refused with one sentence that names the total and
  the limit, so none is handed to the queue to send. Files listed before stay as they are.
  A selection of one file keeps today's sentence.
- **D9. The unpacked message names the limit** (b): "The file is larger than the 2 GB limit
  once unpacked." The API's own refusal builds the number from `MAX_UNPACKED_BYTES`
  (`routes/uploads.rs:199`). The job's recorded reason is a fixed sentence, as all its
  reasons are, because the API shows a recorded reason only when it equals one of the fixed
  list (`outcome_of`, `routes/uploads.rs:1354-1366`); a test pins it to the API's built
  sentence. The new sentence is the twelfth reason of
  `ops/fixtures/upload_load_failure_reasons.json`; the old one stays in the list, no longer
  raised, so a load an older job recorded keeps its message (as `TOO_MANY_ROWS` did).
- **D10. A file that cannot be read is not "a format this upload does not read"** (c). Bytes
  that are neither text nor a kind the API knows are answered by what the file's name says
  it is: `.parquet` "This file could not be opened as a Parquet file.", `.xls`/`.xlsx`
  "This file could not be opened as an Excel workbook (.xls or .xlsx).", `.json`, `.jsonl`,
  `.ndjson` "This is not a valid JSON file.", `.avro` "This file is named .avro but is not
  an Avro file.", `.gz`/`.zip` "This file is named like a .gz or .zip archive but is not
  one. Rename it to match what it is.". Any other name keeps "This is not a file this upload
  reads. …".
- **D11. A list cut at 100 says so, without a total** (d). `GET /api/uploads` answers a bare
  list (`routes/uploads.rs:1619-1663`); a total would change the answer's shape for every
  reader, which is not cheap, so the page says the sentence of D23 when exactly 100 came
  back (question 3).
- **D12. Why "New subfolder" is off is text under the buttons** (e), as the Delete folder
  reason is (`uploads-panel.tsx:237-241`). The `title` stays.
- **D13. One list of formats** (f): two constants in `src/lib/uploads.ts`, read by the link
  on New Connector, the empty state of Uploaded files, and the Upload file page. The phrase
  names JSON Lines, which decision 12 of `upload-file.md` lists and the page left out.

### DATA-52, people by name

- **D14. Usage names people at read time.** `query_history.user_name` keeps the user's id
  (it is also what `X-Trino-User` is derived from, `routes/query.rs:1694-1700`) and stored
  history is not rewritten. `usage` (`routes/catalog_governance.rs:645-683`) reads the names
  of the ids it found with one call of `user_names` and gives them to `summarize_usage`.
  Rules: the id is `owner_id`, else a `user_name` that parses as a UUID; an id with a name is
  shown by it; an id with no user (deleted) or with a blank name is **"Unknown user"**, and
  all such people share one row; a `user_name` that is not an id (a row older than the ids)
  is shown as stored. If the lookup fails, a warning is logged and everyone but the caller is
  "Unknown user". The UUID is never in `frequentUsers` or `recentQueries`. `usage.users`
  still counts distinct ids.
- **D15. An upload's versions name who started the load.** The API sends the load's author
  in the run configuration as `author`: the display name of the person who pressed Load (not
  of who sent the file, if they differ), followed by ` (file upload)`. The job puts it in
  `SinkConfig.author`, and the two writes an upload makes carry it as `rantai.author`: the
  streamed write (`adapters/iceberg_stream.py:150-190`) and the type-changing replace
  (`adapters/sink.py:915-972`). The console already shows `author` as text
  (`asset-activity.tsx:215-218`), so no read side changes.
  - The label is stored, not an id resolved later: `file_upload.uploaded_by` already stores
    the display name (`routes/uploads.rs:1270`), decision 5 of `asset-page-depth.md` says
    "the principal recorded when the write was committed", and the name survives the
    person's removal. It is the name the person had then.
  - `author` is a required field of the job's configuration, like every other
    (`test_file_ingest.py:983-985`); an empty string records no author.
  - The catalog's own `author` of an uploaded table stays `upload` (`file_ingest.py:146`):
    that catalog is shared across tenants.
  - Versions written before this change stay "unknown". There is no backfill.
  - A consequence, kept: the streamed write records the author of any load whose
    `SinkConfig` carries one, so a connector's replace and append versions now name their
    run (`connector <id>, run <id>`) too. A merge load (`incremental_dedup`) still goes
    through `dlt`'s own write and records none.

### DATA-53, types

- **D16. A Parquet `uint64` starts as Decimal** (a). Decimal (38 digits, 9 after the point)
  holds every `uint64`; Integer (64-bit signed) does not. The other integer types stay
  Integer.
- **D17. A nanosecond timestamp starts as Timestamp and loads** (a). The table keeps
  microseconds (Iceberg format version 2), the widest it can. A value fits a Timestamp
  column with a fraction of up to nine digits; the digits after the sixth are **cut, not
  rounded**. The guess from text is unchanged: it still wants at most six digits (a text
  column with seven stays Text), the way the Integer guess refuses a leading zero that a
  chosen Integer accepts (`upload_types.rs:21-23`). The Check step says so under a Parquet
  column whose file type is `timestamp (ns…)`.
- **D18. Not changed: an empty text cell** (b). Decision 7 of `upload-column-types.md`
  ("Empty (null) in every column except text, which keeps an empty text") and the table "How
  a Parquet file is read" of `upload-file.md` ("Null: Empty (so is an empty string)") are
  signed, and the code does what they say. Question 1.
- **D19. Not changed: a decimal's own precision and scale** (c). Decision 2 of
  `upload-column-types.md` offers one Decimal, 38 digits with 9 after the point, signed, and
  `declared_from_parquet` follows it (`upload_types.rs:527-539`). Question 2. What does
  change: the Check step says what a Decimal column is stored as.
- **D20. `POST /api/query/run` keeps a decimal's digits** (d). The engine writes a decimal
  as a JSON number and the API reads it as a 64-bit float (`ChResult`,
  `lakehouse-clickhouse/src/lib.rs:45-60`), which keeps about 15 digits. The query route
  answers every cell as text (`cell_value`, `routes/query.rs:2144-2152`), so for this route
  the engine is asked to write decimals as text (`output_format_json_quote_decimals = 1`,
  a setting of that one request). Dashboards' tiles and downloads are not changed.
- **D21. The Upload page also gives the name Query Studio reads** (e):
  `iceberg.bronze.<table>` on the Trino engine, beside the Catalog's `bronze.<table>`.

### Text, and the assistant

- **D22. The assistant.** No dashboard behaviour changes, so no tool or schema changes and
  `tests/fixtures/tool_schemas.json` is not regenerated. The assistant's SQL tool runs
  through the same `run_statement`, so it gets a decimal's exact digits after T10.
- **D23. Text on screen** (English, as the console).
  - Several files over the limit, on each of them: **"The N files chosen would bring the
    upload to X, over the 2 GB limit for one upload (all its files together). None of them
    was sent. Choose fewer or smaller files."** (N the files of the selection without a
    problem of their own, X the total as `formatBytes` writes it.)
  - **"The file is larger than the 2 GB limit once unpacked."**
  - Under the list: **"Showing the newest 100 files; search to find older ones."**; while a
    search is on: **"Showing the newest 100 files that match; narrow the search to find
    older ones."**
  - Under the folder buttons: **"Folders can be nested at most 3 levels deep."**
  - The formats: **"CSV, TSV, JSON, JSON Lines, Avro, Excel or Parquet"**.
  - **"Unknown user"**.
  - The author of an upload's version: **"<display name> (file upload)"**; **"file upload"**
    when the name is empty.
  - Under a Decimal column's type: **"Stored exactly, with up to 38 digits, 9 of them after
    the point."**
  - Under a Timestamp column from a nanosecond Parquet column: **"The file keeps
    nanoseconds. The table keeps microseconds, so digits after the sixth are cut."**
  - Under "Lands in the Catalog as": **"In Query Studio, on the Trino engine:"** and the
    name. On the Review step the label **"In Query Studio (Trino)"**. After a load: **"In
    Query Studio, on the Trino engine, the table is `iceberg.bronze.<table>`."**

## 2. Tasks

Python first (the reproduction), then the tasks that touch Rust, then the console, then the
documents.

### T1. Reproduce: a first load into a table name of 52 and of 128 characters (LH-DATA-9-37; DATA-50)

- Anchors: `dagster/dispar_orchestrate/adapters/sink.py:673-692` (`_bronze_pipeline`),
  `:1235-1240`; `adapters/history.py:399-411`; `test_snapshot_author.py:1-30` (how a test
  file uses `lab`); `test_load_modes_measurement.py:97-151`.
- Change: add `dagster/dispar_orchestrate/test_sink_long_table_name.py`: a narrative module
  docstring (what D4 says, in your own short words), `from __future__ import annotations`,
  `pytestmark = pytest.mark.loader`, the `lab` import. No product code changes in this
  commit. Table names are built as `"t" + "a" * (length - 1)`. Tests:
  1. `test_dlt_refuses_a_schema_named_after_a_pipeline_of_more_than_64_characters`:
     `dlt.Schema("bronze_ingest_" + "a" * 50)` is made; for 51, 52 and 128,
     `pytest.raises(InvalidSchemaName)` (`dlt.common.schema.exceptions`). Then
     `make_schema_with_default_name` (`dlt.extract.utils`) of `"bronze_ingest_" + "a" * 100
     + "b"` and of the same ending in `"c"`: both 64 characters, and different. This test
     passes before and after the fix; it pins the library's limit.
  2. `test_a_first_typed_replace_into_a_long_table_name_loads`, parametrised over 52 and
     128: `load_via_sink([{"id": 1, "v": "a"}], name, lab.config, LoadPlan(mode="replace"),
     column_types={"id": "long", "v": "string"})` answers `rows == 1` with no failed jobs;
     then `load_via_sink([{"id": 2, "v": "b"}], name, lab.config, LoadPlan(mode="append"))`
     answers `rows == 1`; `sorted(row["id"] for row in lab.rows(name)) == [1, 2]`.
  3. `test_clearing_a_long_named_table_that_was_never_loaded_says_there_is_nothing_to_empty`:
     `clear_table(name_of_128, lab.config).load_info_str == "table not loaded; nothing to
     empty"`.
  4. `test_a_first_load_without_a_column_list_into_a_128_character_name_loads`:
     `load_via_sink([{"id": 1}], name_of_128, lab.config, LoadPlan(mode="append"))` answers
     `rows == 1`. This one is expected to pass on today's code: it is the planner's reading
     that a load which does not open the table first is not affected (D5).
  Mark tests 2 and 3, and only those, with
  `@pytest.mark.xfail(strict=True, raises=InvalidSchemaName, reason="QA LH-DATA-9-37
  (DATA-50): fixed in the next commit")`.
- Check: run the file (section 3). Expected on this commit: tests 1 and 4 pass, tests 2 and
  3 are `xfailed`. Quote the summary line in the handoff. If test 2 or 3 fails with another
  exception (strict `raises`), or test 4 fails, the cause is not the one of D4: write the
  exception type and its message's first line in the handoff and stop T1 to T3 (D7).

### T2. A table `dlt` has no schema for is "not there", without asking for a client (LH-DATA-9-37; DATA-50)

- Anchor: `dagster/dispar_orchestrate/adapters/history.py:399-411` (`open_table`).
- Change: after the `if sync: pipeline.sync_destination()` lines and before the `try`, add
  ```python
  if not pipeline.default_schema_name:
      return None
  ```
  with a comment that says: QA `LH-DATA-9-37` (`DATA-50`); a pipeline with no schema knows
  no table, so the client `dlt` would build answers "not there" anyway; asking for it makes
  `dlt` build a schema named after the pipeline, which it refuses past 64 characters, that
  is for a table name past 50. Add one sentence to the docstring saying the same. Change
  nothing else: `_bronze_pipeline` and the pipeline name stay (D5).
  In `test_sink_long_table_name.py` remove the two `xfail` marks and the now unused import
  if any.
- Check: the file of T1 passes whole (five tests counting the parameters), and
  `test_sink_empty_table.py`, `test_sink_history.py`, `test_sink_history_guard.py`,
  `test_typed_load.py` and `test_snapshot_author.py` still pass (they open tables through
  the changed function).

### T3. The upload gate loads into a 128-character name (LH-DATA-9-37; DATA-50)

- Anchors: `ops/g9/upload_test.py:1603-1640` (`main`), `:1096-1101` (`_upload_one`),
  `:824-831` (`_settle`), `:389-393` (`count_rows`), `:968` (`_delete_upload`); the module
  docstring's list of steps at the top of the file.
- Change: add `step_long_table_name(api, state)` and call it in `main` after
  `step_several_files(...)`. The step:
  1. `table = (f"g9_long_{secrets.token_hex(4)}_" + "x" * 128)[:128]`; fail with `G9Failure`
     if `len(table) != 128`.
  2. `upload_id = _upload_one(api, "long_name.csv", "id,name\n1,a\n2,b\n", state)`.
  3. `POST /api/uploads/{upload_id}/ingest` with `{"bronzeTable": table, "mode": "replace",
     "encoding": "utf-8", "delimiter": ",", "headerRow": 0}`, expect 200; `_settle`; fail
     unless the status is `ingested` and `rows == 2`, quoting `error` when it is `failed`.
  4. Wait, as `step_load` does (`:486-496`), until `count_rows(api, table) == 2`.
  5. The same request with `"mode": "append"`; settle; wait until the count is 4.
  6. `_delete_upload(api, upload_id, state)`.
  Add the step to the module docstring's list, citing `DATA-50` and `LH-DATA-9-37`.
- Check: CI job "Rust · G9 file upload" on the pull request. It is not run per commit. To
  run it on the build machine only when that job fails and the log does not say why:
  ```bash
  cd /home/rismana/rantai/lakehouse-agents-1
  # a .env as the CI job writes it (.github/workflows/ci.yml, job g9-upload, "Write CI-only .env")
  docker compose -p g9data50 --profile dagster build lakehouse-api dagster-code-location oidc-mock
  docker compose -p g9data50 --profile dagster up -d --no-build lakehouse-api \
    dagster-code-location dagster-webserver dagster-daemon clickhouse-iceberg-init
  docker compose -p g9data50 --profile dagster run --rm g9-test-runner
  docker compose -p g9data50 down -v   # always, also after a failure
  ```
  If the 128-character load fails there or in CI, D7 applies.

### T4. A damaged file of an accepted format says it could not be opened (LH-DATA-17-05; DATA-51)

- Anchors: `rust/crates/lakehouse-api/src/routes/uploads.rs:1028-1048` (`classify_head`,
  `Kind::OtherBinary => OTHER_BINARY` at `:1045`), `:275-280`, `:557-565`
  (`has_workbook_name`, `has_parquet_name`), `:2301` (how `ParquetFileError::Unreadable` is
  already used); `upload_formats.rs:157-182` (`ByName`, `by_name`).
- Change: add a private function next to `classify_head`,
  `fn unreadable_as_named(filename: &str) -> String`, that answers, in this order:
  `.parquet` name → `upload_parquet::ParquetFileError::Unreadable.to_string()`; workbook name
  → `upload_workbook::WorkbookError::Unreadable.to_string()`; then by
  `upload_formats::by_name(filename)`: `ByName::Plain(Format::Avro)` → `NOT_AVRO`,
  `ByName::Plain(Format::Json | Format::JsonLines)` → `Refusal::JsonInvalid.to_string()`,
  `ByName::Gzip(_) | ByName::Zip` → `NOT_AN_ARCHIVE`, anything else → `OTHER_BINARY`. In
  `classify_head` the `Kind::OtherBinary` arm returns
  `Err(ApiError::BadRequest(unreadable_as_named(filename)))`; the other arms are untouched
  (rewrite the `match` only as far as the types need). Doc comment with `# Errors` is not
  needed (it returns no `Result`); say in its comment why (D10) and cite the case.
- Check: unit test in the `tests` module of `routes/uploads.rs`,
  `a_binary_file_named_like_an_accepted_format_is_told_it_could_not_be_opened`: with the
  bytes `b"a,b\0c,d"` (the bytes the test at `:3580` already uses for "not text"),
  `check_file` refuses `a.parquet`, `a.xlsx`,
  `a.avro`, `a.json`, `a.csv.gz`, `a.zip` each with the sentence of D10 (compare with the
  constants and `to_string()` calls, not with copied text), none of them contains "not a
  file this upload reads", and `a.bin` and `a.pdf` still get `OTHER_BINARY`. Integration
  test in `tests/upload_routes.rs`, next to
  `a_binary_that_is_not_a_supported_file_is_refused_at_completion_and_removed` (`:5600`):
  `a_damaged_parquet_file_is_refused_at_completion_as_a_file_that_could_not_be_opened`, the
  bytes of that neighbouring test sent as a session named `data.parquet`: 400, the Parquet
  sentence, nothing left in the bucket. If an existing test expects "not a file this upload
  reads" for a binary file under one of these names, its expectation follows D10; name each
  such test in the commit body (the rule changed, the test is not weakened).

### T5. The unpacked limit is named (LH-DATA-17-13; DATA-51)

One commit: Rust, the shared fixture, the job and the gate.

- Anchors: `rust/crates/lakehouse-api/src/upload_formats.rs:236-238`;
  `routes/uploads.rs:199`, `:342-384` (the reasons), `:4378-4400` (the fixture test);
  `tests/upload_routes.rs:6801`; `ops/fixtures/upload_load_failure_reasons.json`;
  `dagster/dispar_orchestrate/file_ingest.py:207-251`, `:789-790`, `:986-987`;
  `test_file_ingest.py:263-275`; `ops/g9/upload_test.py:984`.
- Change:
  1. `routes/uploads.rs:199`: `pub(super) const MAX_UNPACKED_BYTES` becomes `pub(crate)`.
  2. `upload_formats.rs:237`: the attribute becomes
     `#[error("The file is larger than the {} GB limit once unpacked.",
     crate::routes::uploads::MAX_UNPACKED_BYTES / (1024 * 1024 * 1024))]` (a constant
     expression as a format argument, the form `connector_discover.rs:158` uses; a
     constant of `routes::uploads` is already read from outside `routes` at
     `upload_convert.rs:498`). The variant stays a unit variant.
  3. `routes/uploads.rs:365`: rename the existing constant to
     `JOB_TOO_LARGE_UNPACKED_UNNAMED` (text unchanged, comment: no longer raised, kept so a
     load an older job recorded keeps its message) and add
     `const JOB_TOO_LARGE_UNPACKED: &str = "The file is larger than the 2 GB limit once
     unpacked.";`. `JOB_FAILURE_REASONS` becomes `[&str; 12]`: the first eleven in today's
     order with the renamed constant in its place, the new one last. Correct "eleven" in the
     comments at `:342-347` and `:371`.
  4. The fixture: append the new sentence as the twelfth line; the first eleven unchanged.
  5. `file_ingest.py`: the same on the job's side: `TOO_LARGE_UNPACKED_UNNAMED` (old text,
     the comment as above) stays tenth in `FAILURE_REASONS`; `TOO_LARGE_UNPACKED` is the new
     sentence, appended last. The two `raise LoadFailure(TOO_LARGE_UNPACKED)` sites then
     raise the new one with no edit.
  6. `tests/upload_routes.rs:6801` and `ops/g9/upload_test.py:984`: the new sentence.
- Check: in `routes/uploads.rs` tests, change
  `the_reasons_the_api_knows_are_the_reasons_in_the_shared_fixture` to twelve, and add
  `the_unpacked_sentences_name_the_unpacked_cap`: `MAX_UNPACKED_BYTES % (1024 * 1024 * 1024)
  == 0`, `JOB_TOO_LARGE_UNPACKED == Refusal::TooLargeUnpacked.to_string()`, and both contain
  `format!("the {} GB limit", MAX_UNPACKED_BYTES / (1024 * 1024 * 1024))`. In
  `test_file_ingest.py` the count test becomes twelve (rename it to say twelve, and add to
  its comment that `DATA-51` made it twelve), and add
  `test_the_old_unpacked_sentence_stays_in_the_list_and_is_no_longer_the_one_raised`
  (`TOO_LARGE_UNPACKED_UNNAMED in FAILURE_REASONS`, `TOO_LARGE_UNPACKED` contains "2 GB").
  The existing bomb test of `upload_routes.rs` (`:6783`) proves the API's sentence. The
  changed counts are a rule change, not a weakened test: say so in the commit body.

### T6. Usage shows people by name (LH-DATA-12-01 F3, LH-DATA-15-02, TC-1730; DATA-52)

- Anchors: `rust/crates/lakehouse-api/src/routes/catalog_governance.rs:346-367`
  (`person_key`, `frequent_users`), `:477-572` (`summarize_usage`; `"user": m.user_name` at
  `:564`), `:645-683` (`usage`), tests `:1569-1580`, `:1589-1626`, `:1733-1754`;
  `rust/crates/lakehouse-store/src/dashboard_access.rs:257-269`.
- Change, following D14:
  1. `const UNKNOWN_USER: &str = "Unknown user";`
  2. `fn person_id(m: &HistoryMention) -> Option<uuid::Uuid>`: `m.owner_id`, else
     `uuid::Uuid::parse_str(m.user_name.trim()).ok()`.
  3. `fn person(m, names: &HashMap<uuid::Uuid, String>) -> (String, String)`, the row's
     grouping key and the name shown: an id found in `names` → `(id.to_string(), name)`; an
     id not found → `(UNKNOWN_USER, UNKNOWN_USER)`; no id → `(m.user_name, m.user_name)`.
  4. `frequent_users(reads, names)` groups and names by `person`. `person_key` stays and
     still feeds the `users` count.
  5. `summarize_usage` gains a last parameter `names: &HashMap<uuid::Uuid, String>`;
     `"user"` of `recentQueries` becomes `person(m, names).1`.
  6. `async fn people_names(pg: &PgPool, mentions: &[HistoryMention], caller: &Principal)
     -> HashMap<uuid::Uuid, String>`: the distinct `person_id`s, one
     `lakehouse_store::dashboard_access::user_names(pg, &ids)`, names that are blank after
     `trim` left out; on `Err` a `tracing::warn!` and an empty map (the name is decoration:
     a failed lookup must not turn the page's usage into "not measured"); then the caller's
     own id is inserted with `caller.display_name` if absent. `usage` calls it after
     `history_mentioning` and passes the map on. The function returns no `Result`, so it has
     no `# Errors`; its comment says why the failure is swallowed.
- Check (unit tests in the same file's `tests` module; every existing `summarize_usage` call
  gets the new argument):
  - `frequent_users_rank_people_by_their_queries` is rewritten as
    `frequent_users_are_named_from_the_user_table_and_ranked_by_their_queries`: the names
    come from the map, the fixture's `user_name` is left as it is, the expected list is
    unchanged. (The rule changed: the history's own text was the name before.)
  - `a_person_who_no_longer_exists_is_unknown_user_and_never_an_id`: three readers, one in
    the map; the two others have `user_name` set to their own id as text. `frequentUsers`
    is one "Unknown user" row with their queries added and one row for the known name;
    `usage["users"] == 3`; the JSON text of `usage` and of `recent` contains neither id.
  - `a_row_without_an_owner_is_named_by_the_id_in_its_user_name_or_shown_as_stored`:
    `owner_id: None` with `user_name` an id in the map → the name; with `user_name`
    `"anonymous"` → `"anonymous"`.
  - In `usage_counts_everyone_but_lists_only_the_callers_queries`, pass the caller's name
    in the map and assert `recent[0]["user"]` is that name.
  `user_names` itself is covered by `lakehouse-store/tests/dashboard_access.rs:344-350`.

### T7. A load through the streamed write records its author (LH-DATA-15-08; DATA-52)

- Anchors: `dagster/dispar_orchestrate/adapters/iceberg_stream.py:150-190` (`_write`; the
  `write_one_snapshot` call at `:184-189`); `adapters/sink.py:411-415` (the comment on
  `SinkConfig.author`), `:915-917` and `:968-972` (`_replace_changing_types`), `:1243`,
  `:1251-1257` (`pipeline.run`); `adapters/iceberg_snapshot.py:121-126` (the docstring that
  says these paths carry none); `test_snapshot_author.py`; `test_typed_load.py:437-462`.
- Change:
  1. `iceberg_stream.py`: a module-level `_snapshot_properties: dict[str, dict[str, str]]
     = {}` and a context manager
     `snapshot_properties_for(table: str, properties: dict[str, str])` that stores a copy
     under `table` on entry and removes it in a `finally` on exit. `_write` passes
     `snapshot_properties=_snapshot_properties.get(table.name()[-1])` to
     `write_one_snapshot` (`table.name()` is the `pyiceberg` identifier tuple; its last part
     is the table's name). Add a paragraph to the module docstring: why a module-level
     table (`dlt` calls the write from its own load threads with no argument of ours), that
     it is keyed by table so two loads cannot read each other's, and the QA case.
  2. `sink.py`: wrap the `pipeline.run(...)` call of `load_via_sink` (`:1251-1257`) in
     `with iceberg_stream.snapshot_properties_for(bronze_table_name,
     history.snapshot_properties(config.author)):`. `_replace_changing_types` gains
     `author: str = ""` and passes
     `snapshot_properties=history.snapshot_properties(author)` to `write_one_snapshot`; its
     caller at `:1243` passes `author=config.author`.
  3. Correct the three comments that say a `dlt` load records no author: `sink.py:411-415`,
     `iceberg_snapshot.py:121-126`, the docstring of `test_snapshot_author.py:1-9`. New
     wording: a replace or an append records the author; a merge load, which `dlt` writes
     itself, records none.
  4. `test_typed_load.py:452` and `:462` pass `None` as the sink configuration, which the
     code now reads. Pass `sink_module.SinkConfig("u", "w", "e", "a", "s", "b", "",
     author="a person (file upload)")` instead (the form `Lab.config` uses) and say why in
     the commit body.
- Check, in `test_snapshot_author.py` (the `lab` harness):
  - `test_a_replace_and_an_append_record_the_author_of_the_load`: with
    `dataclasses.replace(lab.config, author="a person (file upload)")`, a `replace` then an
    `append` of one row each into `t`; `_authors(lab, "t") == ["a person (file upload)"] *
    2`. Fails on today's code (`[None, None]`).
  - `test_a_load_without_an_author_records_none_and_nothing_stays_registered`: a load with
    `lab.config` records `None`; after each load `iceberg_stream._snapshot_properties ==
    {}`, also after a load that raised (make the source a generator that raises).
  - In `test_typed_load.py`, in the changed test, after the type-changing replace:
    `reopened.current_snapshot().summary.additional_properties["rantai.author"] ==
    "a person (file upload)"`.
  The existing three author tests stay green unchanged.

### T8. The API tells the job who started the load (LH-DATA-15-08; DATA-52)

One commit: Rust, the job and the gate.

- Anchors: `rust/crates/lakehouse-api/src/routes/uploads.rs:262` (`MAX_STORED_TEXT_CHARS`),
  `:2712-2738` (`run_config`), `:2819-2832` (`launch`), `:3296` (the call in `ingest`),
  tests `:4133`, `:4159`; `tests/upload_routes.rs:2019-2036` and the two other places that
  compare the whole run configuration (search `"on_bad_value": "stop",` near `:4571` and
  `:5016`), `:1159` (the seeded caller's display name, as `uploadedBy`);
  `dagster/dispar_orchestrate/file_ingest.py:282-327` (`FileIngestParams`), `:331-354`
  (`CONFIG_SCHEMA`), `:1279-1297` (`_load`; `sink_config = load_config()` at `:1295`),
  `:117` (the `dataclasses` import); `test_file_ingest.py:105-120` (`_params`), `:157-215`
  (`_Harness`), the `RUN_CONFIG` of that file; `ops/g9/upload_test.py` (the step of T3).
- Change:
  1. `routes/uploads.rs`: `fn load_author(display_name: &str) -> String`, next to
     `run_config`: the name without control characters, trimmed, at most
     `MAX_STORED_TEXT_CHARS` characters; `format!("{name} (file upload)")`, or
     `"file upload"` when nothing is left. `run_config` gains a last parameter
     `author: &str` and the key `"author": author`. `launch` gains `author: &str` and passes
     it; `ingest` calls `launch(…, kind, &load_author(&principal.display_name))`. Update the
     doc comment of `run_config` (the config schema is closed: the job takes `author` from
     this change on).
  2. `file_ingest.py`: `CONFIG_SCHEMA` gains `"author": Field(str, description=…)`
     (required; say in the description that an empty string records no author);
     `FileIngestParams` gains `author: str` as its last field with a `#:` comment (D15).
     In `_load`, right after `sink_config = load_config()` succeeds:
     `sink_config = replace(sink_config, author=params.author)` (`from dataclasses import
     dataclass, field, replace`). `AUTHOR = "upload"` and `_register` do not change; extend
     the comment at `:145` with one sentence: the person is on the table's versions, not in
     the shared catalog.
  3. `ops/g9/upload_test.py`, in `step_long_table_name` after the second load:
     `GET /api/lakehouse/tables/bronze/{table}`, expect 200, and fail unless every entry of
     `snapshots` has an `author` that ends with `" (file upload)"`.
- Check:
  - Rust unit tests in `routes/uploads.rs`:
    `the_run_config_names_who_started_the_load` (`job["author"]` is what was passed) and
    `the_author_of_a_load_is_the_name_without_control_characters_or_file_upload_alone`
    (a plain name; a name with `\n` and `\u{7}` inside; a blank name → `"file upload"`; a
    name of 300 characters → 255 of them and the suffix). The two existing `run_config`
    calls get an author argument.
  - `tests/upload_routes.rs`: the three whole-configuration comparisons gain
    `"author": "<the seeded caller's display name> (file upload)"`, built from the same
    text `:1159` expects, not typed again.
  - `test_file_ingest.py`: `_params` and `RUN_CONFIG` gain `author`; `_Harness.load`
    records `config.author`; add
    `test_the_sink_is_told_the_author_the_run_was_launched_with` (the recorded author is the
    params' one, and the registered catalog author is still `"upload"`). The two schema
    tests at `:983-985` and `:1342-1347` pass unchanged: `author` is required like the rest.
  - The G9 job proves the name reaches a real table's versions.

### T9. A `uint64` starts as Decimal; a timestamp takes a fraction of up to nine digits (LH-DATA-9-32; DATA-53)

One commit: Rust, the shared fixture and the job.

- Anchors: `rust/crates/lakehouse-api/src/upload_types.rs:34-37` (the module's rule text),
  `:305-357` (`split_timestamp`; the fraction check at `:349-353`), `:423` and `:472` (its
  two callers), `:502-545` (`declared_from_parquet`), test `:964-984`;
  `ops/fixtures/uploads/types/cast.json` (the case with value
  `2026-10-16T13:30:05.1234567`); `ops/fixtures/uploads/types/guess.json` (the case "a
  fraction of seven digits is text": stays);
  `dagster/dispar_orchestrate/upload_types.py:30-32` (the rule text), `:148-151`
  (`_TIMESTAMP`), `:214-216` (the fraction).
- Change:
  1. `declared_from_parquet`: take `"uint64"` out of the integer arm and give it its own:
     `"uint64" => Declared::of(ColumnType::Decimal)`. Rewrite the sentence of the doc
     comment that says a `uint64` past 64 bits will not fit.
  2. `upload_types.rs`: two constants, `GUESS_FRACTION_DIGITS: usize = 6` (what the guess
     takes: what the table keeps) and `MAX_FRACTION_DIGITS: usize = 9` (what fits: a
     nanosecond file's digits; the job cuts after the sixth). `split_timestamp` gains
     `max_fraction: usize` and compares `fraction.len() <= max_fraction`. The guess (`:423`)
     passes `GUESS_FRACTION_DIGITS`, `fits` (`:472`) passes `MAX_FRACTION_DIGITS`. Update
     the rule text at `:34-37`.
  3. `upload_types.py`: in `_TIMESTAMP`, `(?P<f>[0-9]{1,6})` becomes `{1,9}`; the fraction
     becomes `pc.utf8_rpad(pc.utf8_slice_codeunits(pc.coalesce(optional("f"),
     pa.scalar("")), 0, 6), 6, "0")`. Update the rule text at `:30-32` (cut, not rounded).
  4. `cast.json`: the seven-digit case becomes `"ok": true, "cast":
     "2026-10-16T13:30:05.123456"`. Add three cases: `2026-10-16T13:30:05.123456789`
     (no zone) → `…05.123456`; `2026-10-16T13:30:05.999999999Z` (zone) →
     `2026-10-16T13:30:05.999999Z` (cut: the second does not roll over);
     `2026-10-16T13:30:05.1234567891` (ten digits) → `"ok": false, "cast": null`.
     `guess.json` is not edited.
- Check: `a_files_and_a_tables_own_types_become_the_starting_types` gains
  `("uint64", ColumnType::Decimal, None)` and `("timestamp (ns, UTC)",
  ColumnType::Timestamp, Some(true))`. Add
  `a_uint64_s_largest_value_fits_the_type_its_column_starts_with`:
  `fits(declared_from_parquet("uint64").column_type, None, false,
  "18446744073709551615")`. Add
  `the_guess_keeps_six_digits_of_a_fraction_and_a_chosen_timestamp_takes_nine`:
  `guess_column(&["2026-10-16T13:30:05.1234567"])` is Text, and `fits(Timestamp, Some(Iso),
  false, …)` is true for seven and nine digits and false for ten. The two fixture-driven
  tests (`a_value_fits_exactly_when_the_shared_fixture_says_so`,
  `the_guess_reaches_every_case_of_the_shared_fixture`) and `test_upload_types.py` read the
  changed file and must pass; they fail on today's code with the new cases. Changing the
  seven-digit case is a rule change (D17): say so in the commit body.

### T10. The query route keeps a decimal's digits (LH-DATA-9-32; DATA-53)

- Anchors: `rust/crates/lakehouse-clickhouse/src/lib.rs:145-161` (`QueryOptions`),
  `:342-377` (`query_with_options`), test `:1384-1421`;
  `rust/crates/lakehouse-api/src/routes/query.rs:1526` (`run.options(limits,
  limits.studio_rows)`), `:2144-2152` (`cell_value`);
  `rust/crates/lakehouse-api/src/query_limits.rs:381-387`;
  `rust/crates/lakehouse-api/tests/query_result_cap.rs:145` (`run_as_reader`), `:196-215`.
- Change:
  1. `QueryOptions` gains `pub quote_decimals: bool` (default `false`), documented: the
     engine writes a decimal as JSON text with every digit (`output_format_json_quote_
     decimals = 1`) instead of a number, which this client reads as a 64-bit float; read by
     `query_with_options` only. `query_with_options` pushes
     `("output_format_json_quote_decimals", "1")` when it is set.
  2. Every `QueryOptions { … }` literal that names all fields stops compiling: add
     `quote_decimals: false` (or `..QueryOptions::default()`) to each
     (`query_limits.rs:382`, `routes/query.rs:3459`, the tests of `lib.rs`). Find them with
     `grep -rn "QueryOptions {" rust/crates`.
  3. `routes/query.rs:1526`: `let options = lakehouse_clickhouse::QueryOptions {
     quote_decimals: true, ..run.options(limits, limits.studio_rows) };` with the comment
     (QA `LH-DATA-9-32`, `DATA-53`: every cell of this route is text already, so a decimal
     as text changes no cell's type and keeps its digits). `support.rs:413` (dashboards) is
     not changed.
- Check:
  - `lakehouse-clickhouse`, unit:
    `query_with_options_asks_for_quoted_decimals_only_when_told`: with the option, the
    request carries `output_format_json_quote_decimals=1`; without it, the request's query
    string does not contain that name (read `received_requests()`, as `:1376-1379` does).
  - `lakehouse-api`, integration, in `tests/query_result_cap.rs` (a real engine):
    `a_decimal_keeps_every_digit_the_engine_holds`: `run_as_reader(&app, "SELECT
    toDecimal128('123456789012345.678901234', 9) AS d")` answers 200 and the row's `d` is
    exactly `"123456789012345.678901234"`. Fails on today's code (the float's digits).
  - `a_null_cell_is_null_and_every_other_cell_keeps_its_text` (`query.rs:4319`) stays green
    unchanged.

### T11. A selection over the limit sends nothing (LH-DATA-17-04; DATA-51)

- Anchors: `src/lib/uploads.ts:296-327` (`planPick` and its comment);
  `src/lib/uploads.test.ts:411-420`; `src/features/connectors/use-upload-queue.ts:49-56`
  (unchanged: it hands `candidates` to the queue, which never sends one with a problem,
  `src/lib/upload-queue.ts:215-223`).
- Change, following D8: compute each taken file's own problem first; `listed` = the sum of
  `existing` that is not `refused`; `chosen` = the sum and `count` = the number of taken
  files with no problem of their own. If `listed + chosen > MAX_SESSION_BYTES`, each of
  those files gets: for `count === 1` today's sentence (unchanged text, with
  `formatBytes(listed + chosen)`), otherwise the sentence of D23 with `count` and
  `formatBytes(listed + chosen)`. Otherwise their problem is `null`. A file with its own
  problem keeps it and is not counted. Rewrite the third bullet of the doc comment.
- Check, in `src/lib/uploads.test.ts`:
  - Rewrite the test at `:411` as `planPick refuses every file of a selection that takes
    the upload over 2 GB, and a refused file does not count`: with the same input, all
    three candidates have the D23 sentence (`/^The 3 files chosen would bring the upload to
    .* over the 2 GB limit for one upload/`, `/None of them was sent/`). The old
    expectation (the first file passes) is the QA failure.
  - `planPick refuses both files of LH-DATA-17-04 before either is sent`: two files of
    1,127,500,000 bytes, nothing listed: both refused, the sentence contains `2.1 GB`.
  - `planPick keeps a selection that fits, and today's sentence for one file too many`:
    two halves of the limit → both `null`; then one more byte picked alone with those two
    listed → `/would bring the files to/`.
  - `planPick does not count a file refused for its own reason toward the total`: a
    `.xlsm` of 3 GiB beside two small CSVs → the CSVs are `null`.

### T12. A list cut at 100 says so, and "New subfolder" says why it is off (LH-DATA-35-10, LH-DATA-35-07; DATA-51)

- Anchors: `src/features/connectors/uploads-panel.tsx:200-214` (the New subfolder button),
  `:237-241` (the Delete reason), `:246-379` (the table's container);
  `src/lib/uploads.ts:49-53` (where the limits are);
  `src/features/connectors/uploads-panel.test.tsx:407-422`, `:477-491`.
- Change:
  1. `src/lib/uploads.ts`: `export const UPLOAD_LIST_LIMIT = 100`, documented as the API's
     `LIST_LIMIT` (`routes/uploads.rs:226`): the list has no paging.
  2. `uploads-panel.tsx`: wrap the table's container (`<div className="min-w-0 rounded-md
     border">…</div>`) in `<div className="min-w-0 space-y-2">` and, after the container,
     when `uploads.length >= UPLOAD_LIST_LIMIT`, a
     `<p className="text-xs text-muted-foreground" data-testid="upload-list-cut">` with the
     sentence of D23 (the second form when `searching`), the number from the constant.
  3. Compute `const nestReason = openFolder && !canNestIn(folders, openFolder.id) ?
     "Folders can be nested at most 3 levels deep." : null` once; the button's `disabled`
     and `title` read it; under the buttons, beside the Delete reason, when it is not
     `null`: `<p className="text-xs text-muted-foreground"
     data-testid="folder-nest-reason">{nestReason}</p>`.
- Check, in `uploads-panel.test.tsx`: `says the list is cut when a hundred files came back,
  and not for ninety-nine` (100 rows → the first sentence by test id; 99 → absent; 100 rows
  with a search typed → the second sentence); extend the test at `:477` to read the reason
  by its test id, and assert it is absent in a folder that can take a subfolder (the test
  at `:448`).

### T13. One list of formats (LH-DATA-9-02; DATA-51)

- Anchors: `src/features/connectors/connector-create-page.tsx:383-385`;
  `uploads-panel.tsx:128-131`; `upload-file-page.tsx:29`, `:404`; `upload-file-step.tsx:21`;
  tests `connector-create-page.test.tsx:85`, `uploads-panel.test.tsx:157-162`,
  `upload-file-page.test.tsx:299`, `:325`.
- Change: in `src/lib/uploads.ts`, beside the limits:
  `export const ACCEPTED_FORMATS = "CSV, TSV, JSON, JSON Lines, Avro, Excel or Parquet"` and
  `export const ACCEPTED_FILES = "CSV, TSV, JSON, JSON Lines, Avro, .xls, .xlsx, .parquet,
  .gz and .zip"`, with one comment: decision 12 of `upload-file.md`; a format added to the
  API is added here and nowhere else in the console. Use them:
  `Have a file instead? Upload ${ACCEPTED_FORMATS} files` (the link);
  `Upload ${ACCEPTED_FORMATS} files to bring them in as raw tables.` (the empty state);
  `Choose ${ACCEPTED_FORMATS} files` (`upload-file-page.tsx:29`);
  `Bring ${ACCEPTED_FORMATS} files, or a gzip or zip of them, in as raw tables. …` (`:404`,
  the rest of the sentence unchanged); `{ label: "Accepted", value: ACCEPTED_FILES }`
  (`upload-file-step.tsx:21`). The refusal sentence at `src/lib/uploads.ts:286` is not
  touched: it repeats the API's `OTHER_BINARY` word for word.
- Check: update the three tests' expected text (the link's name, the empty state's
  sentence and its test title, the page description's pattern); `:299` passes unchanged.
  Add to `src/lib/uploads.test.ts`: `the formats phrase names every format the Accepted
  fact lists` (for each of "CSV", "TSV", "JSON", "JSON Lines", "Avro" both constants
  contain it; `ACCEPTED_FORMATS` contains "Excel" and "Parquet", `ACCEPTED_FILES` contains
  ".xlsx" and ".parquet").

### T14. Two people with one name do not collide in the list (LH-DATA-12-01 F3; DATA-52)

- Anchors: `src/features/catalog/asset-activity.tsx:493-498` (`key={u.name}`), `:215`
  (comment); `src/services/contracts/assets.ts:183`, `:259-262`;
  `src/services/contracts/lakehouse.ts:66-71` (comment);
  `rust/crates/lakehouse-iceberg/src/rest.rs:158-162` (comment);
  `src/features/catalog/asset-detail-tabs.test.tsx:872`.
- Change: the rows of "Frequent users" are keyed by position and name
  (`key={`${index}:${u.name}`}`): names are not unique (two people may share one, and the
  API now sends names). Comments only otherwise: at `assets.ts:183` say `name` is the
  person's display name, or "Unknown user"; at `:262` the same for `user`; at
  `asset-activity.tsx:215`, `lakehouse.ts:66-71` and `rest.rs:158-162` say a merge load
  records no author and an upload records `<name> (file upload)`.
- Check: in `asset-detail-tabs.test.tsx`, `lists two frequent users who share a name as
  two rows`: `frequentUsers` with the same name twice and different counts renders both
  counts, and the console logs no duplicate-key error (spy on `console.error`).

### T15. The Check step says what a type keeps, and the page gives the name to query (LH-DATA-9-32, LH-DATA-9-07; DATA-53)

- Anchors: `src/features/connectors/upload-columns.tsx:27-34` (the props), `:76-93` (the
  Type cell); `upload-check-step.tsx:167` (`parquet`), `:327`;
  `upload-table-step.tsx:64-75`; `upload-review-step.tsx:163-170` (the Table group);
  `upload-run-view.tsx:143-162`; `src/lib/asset-query.ts:35`;
  tests `upload-file-page.test.tsx:870-886` (the Parquet tests), `:1021-1026`,
  `:1452-1464`.
- Change:
  1. `src/lib/uploads.ts`: `export function keepsNanoseconds(fileType: string | undefined):
     boolean` — true when the label starts with `timestamp (ns` (the API's label,
     `upload_parquet.rs:559-590`).
  2. `UploadColumnsEditor` gains an optional prop `fileTypes?: readonly (string |
     undefined)[]`; `upload-check-step.tsx:327` passes
     `fileTypes={parquet?.columns.map((c) => c.type)}`. In the Type cell, after the
     "Guessed:" line: when `column.type === "decimal"` the Decimal sentence of D23; when
     `column.type === "timestamp" && keepsNanoseconds(fileTypes?.[index])` the nanosecond
     sentence. Same classes as the "Guessed:" line. Both also show when the columns are
     fixed.
  3. `upload-table-step.tsx`: under the `upload-destination` paragraph, only when `valid`:
     `<p className="text-xs text-muted-foreground">In Query Studio, on the Trino engine:{" "}
     <span className="break-all font-mono" data-testid="upload-query-name">
     {trinoBronzeName(tableName)}</span></p>`.
  4. `upload-review-step.tsx`: the Table group gains, after "Table",
     `{ label: "In Query Studio (Trino)", value: <Wrapped mono>{trinoBronzeName(tableName)}
     </Wrapped> }`.
  5. `upload-run-view.tsx`: in the `ingested` branch, after the tiles' grid, when
     `upload.bronzeTable !== undefined`, a `<p className="text-sm text-muted-foreground">`
     with the last sentence of D23, the name in a `break-all font-mono` span.
  `rawTableFullName` and every `bronze.<table>` shown today stay: that is the Catalog's
  name.
- Check: in `src/lib/uploads.test.ts`, `keepsNanoseconds is true only for a nanosecond
  timestamp label` (`timestamp (ns)`, `timestamp (ns, UTC)` true; `timestamp (us, UTC)`,
  `int64`, `undefined` false). In `upload-file-page.test.tsx`: next to the Parquet tests,
  `says under a decimal column what is stored, and under a nanosecond timestamp that it is
  cut` (the fixture's `decimal(10,2)` column shows the Decimal sentence; add a column with
  the file type `timestamp (ns, UTC)` and the type `timestamp` to that test's own preview
  and expect the nanosecond sentence; a `timestamp (us, UTC)` column shows none); extend
  the test at `:1021` to read `iceberg.bronze.stock` from `upload-query-name` and to find
  it gone for a name that does not pass; update the label list at `:1455` (add "In Query
  Studio (Trino)" after "Table") and assert its value; in the loaded-state test at
  `:1481`, expect the sentence with `iceberg.bronze.stock`.

### T16. The documents say what is now true (DATA-50, DATA-51, DATA-52, DATA-53)

Edit only these lines. A table cell stays on one line; a bullet is wrapped like the bullets
around it. In a "Signed" cell never write that the product owner signed the new wording.

- `docs/core/features/upload-column-types.md`:
  - `:58`, decision 5: append to the Default cell: ` A fraction of a second longer than six
    digits is cut to microseconds, not rounded; the guess from text still wants at most
    six.` The Signed cell becomes `2026-10-16, product owner: the default; changed
    2026-11-12 (`DATA-53`), *(proposed, not yet signed)*`.
  - `:63`, decision 10: append to the Default cell: ` A Parquet `uint64` starts as Decimal,
    which holds every value of it; a nanosecond timestamp starts as Timestamp and the Check
    step says its digits after the sixth are cut.` The Signed cell becomes `2026-10-16,
    product owner: the default; changed 2026-11-12 (`DATA-53`), *(proposed, not yet
    signed)*`.
  - After `:70` add the bullet:
    ```text
    - A timestamp keeps microseconds. A value with a longer fraction (a nanosecond Parquet column) is loaded with the digits after the sixth cut, not rounded, and the Check step says so.
    ```
- `docs/core/features/upload-file.md`, after `:247` (the bullet that ends "A load that
  sends no column list stores every column as text.") add:
  ```text
  - The Upload file page shows a table's name twice: `bronze.<table>` as the Catalog lists it, and `iceberg.bronze.<table>`, the name Query Studio reads on the Trino engine (`DATA-53`).
  - A file named like a format the upload reads that does not open as one is refused as a file that could not be opened, not as a format that is not read (`DATA-51`).
  ```
- `docs/core/features/upload-folders.md:64`: append to that bullet: ` When 100 are shown
  the tab says "Showing the newest 100 files; search to find older ones."; it does not know
  how many there are in all.`
- `docs/core/features/asset-page-depth.md`:
  - after `:74` add:
    ```text
    - A version written by a file upload names the person who started the load, as "<name> (file upload)", with the name they had then; a version written by a connector's replace or append names the connector's run. A merge load and every version written before 2026-11-12 say "unknown" (`DATA-52`).
    - Frequent users are shown by name. A person who no longer exists is "Unknown user", and all such people share one row.
    ```
  - `:84`: the text in brackets becomes `(author is read and shown; a merge load, and
    versions written before 2026-11-12, say "unknown")`.
- `CHANGELOG.md`, under `## [Unreleased]`, `### Fixed`, in the style of the entries there
  (what a customer sees, the backlog ID, the limit where there is one): a file loads into a
  table name of up to 128 characters, where a name past 50 failed (`DATA-50`); the upload
  page's six findings in one entry (`DATA-51`); people by name in an asset's usage, and the
  uploader on a table's versions, with "old versions stay unknown" (`DATA-52`); the type
  findings: `uint64`, nanosecond timestamps, the query route's decimals, the name to query
  (`DATA-53`).
- Do not edit `docs/core/BACKLOG.md`, `PRODUCT.md`, `HANDOFF.md`, `AGENTS.md`,
  `docs/CODE-STANDARD.md` or any acceptance checklist row (the reviewer does what is needed,
  after merge).

## 3. Verification

The build machine is a strong ARM64 host (20 CPUs, 121 GB of memory, Docker running); see
section 3a for the checkout and the cargo settings. CI on the pull request runs on x86_64
and is the final word: a check that is green here and red there is red.

Per commit, only for what the commit touched. Never `cargo clean`. Never a bare
`cargo test -p lakehouse-api <filter>`: a unit test runs with `--lib` and its name, an
integration test file runs with `--test <file>`.

- Rust, from `rust/`: `cargo fmt --check`, then for each crate the commit touched
  `cargo clippy -p <crate> --all-targets -- -D warnings`, then the tests of the task:

  | Task | Crates for clippy | Tests |
  | --- | --- | --- |
  | T4 | `lakehouse-api` | `cargo test -p lakehouse-api --lib a_binary_file_named_like_an_accepted_format_is_told_it_could_not_be_opened`; `… --lib a_parquet_file_needs_both_its_first_bytes_and_a_parquet_name`; `cargo test -p lakehouse-api --test upload_routes` |
  | T5 | `lakehouse-api` | `cargo test -p lakehouse-api --lib the_reasons_the_api_knows_are_the_reasons_in_the_shared_fixture`; `… --lib the_unpacked_sentences_name_the_unpacked_cap`; `… --lib upload_formats`; `cargo test -p lakehouse-api --test upload_routes` |
  | T6 | `lakehouse-api` | `cargo test -p lakehouse-api --lib routes::catalog_governance` |
  | T8 | `lakehouse-api` | `cargo test -p lakehouse-api --lib the_run_config_names_who_started_the_load`; `… --lib the_author_of_a_load_is_the_name_without_control_characters_or_file_upload_alone`; `… --lib the_other_files_are_read_from_the_body`; `… --lib the_job_is_told_the_converted_object`; `cargo test -p lakehouse-api --test upload_routes` |
  | T9 | `lakehouse-api` | `cargo test -p lakehouse-api --lib upload_types` |
  | T10 | `lakehouse-clickhouse`, `lakehouse-api` | `cargo test -p lakehouse-clickhouse --lib query_with_options`; `cargo test -p lakehouse-api --lib a_null_cell_is_null_and_every_other_cell_keeps_its_text`; `cargo test -p lakehouse-api --test query_result_cap` |
  | T14 | `lakehouse-iceberg` (a comment only) | none |

- Python, in a virtual environment under `/home/rismana/rantai/` made once
  (`python3 -m venv /home/rismana/rantai/venv-data-p2 && . /home/rismana/rantai/venv-data-p2/bin/activate
  && pip install ./dagster pytest==8.3.4`, from the checkout, as CI installs it), one file
  per call, `(cd dagster && python -m pytest dispar_orchestrate/<file> -q)`:
  T1 `test_sink_long_table_name.py` (add `-rx` to list the expected failures);
  T2 the same, and `test_sink_empty_table.py`, `test_sink_history.py`,
  `test_sink_history_guard.py`, `test_typed_load.py`, `test_snapshot_author.py`;
  T5 `test_file_ingest.py`; T7 `test_snapshot_author.py`, `test_typed_load.py`,
  `test_iceberg_stream.py`, `test_iceberg_snapshot.py`; T8 `test_file_ingest.py`;
  T9 `test_upload_types.py`. If the install fails on this machine for a driver package with
  no ARM64 wheel, name the package in the handoff and write *not verified locally, CI* for
  the Python tests: CI's "Dagster · unit tests" and "Dagster · loader tests" run them.
- TypeScript: `bun install --frozen-lockfile` once, then per commit `bun run typecheck`,
  `bun run lint`, and `bun test` on the touched test files: `src/lib/uploads.test.ts` (T11,
  T12, T13, T15), `src/features/connectors/uploads-panel.test.tsx` (T12, T13),
  `src/features/connectors/connector-create-page.test.tsx` (T13),
  `src/features/connectors/upload-file-page.test.tsx` (T13, T15),
  `src/features/catalog/asset-detail-tabs.test.tsx` (T14). Do not run prettier.
- Always: `python3 ops/lint/check_bare_iceberg_count.py && python3
  ops/lint/check_intra_package_imports.py` (T3 and T8 add queries to the gate through
  `count_rows`, which keeps the `WHERE 1`).
- No compose file changes, so no `docker compose` check. The G9 gate is CI's (T3).

Before handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-clickhouse -p lakehouse-iceberg -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-clickhouse --lib query_with_options
cargo test -p lakehouse-api --lib routes::uploads
cargo test -p lakehouse-api --lib routes::catalog_governance
cargo test -p lakehouse-api --lib upload_types
cargo test -p lakehouse-api --lib upload_formats
cargo test -p lakehouse-api --test upload_routes
cargo test -p lakehouse-api --test query_result_cap
cd .. && bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
(cd dagster && python -m pytest dispar_orchestrate -q -m "not loader")
(cd dagster && python -m pytest dispar_orchestrate -q -m loader)
git status   # nothing generated staged
```

The full `cargo test --workspace` and the G9 gate are CI's on the pull request; list them in
the handoff as *not verified locally, CI*. The handoff quotes each command with its counts,
names every test added or changed so it can be found in the CI log, and says *not verified*
for anything not run (rule 7).

What the planner read and did not run, and the developer's runs must confirm. If one of
these answers differently, report the answer and stop that task; the plan is corrected, not
the assertion.

- That `load_via_sink` fails with `InvalidSchemaName` for a first typed replace into a name
  past 50 characters, and loads after T2 (only `dlt.Schema(...)` and
  `make_schema_with_default_name` were called by the planner; T1 and T2 are the proof).
- That a first load which does not open the table already works for a 128-character name
  (T1, test 4).
- That the catalog server and the query engine take a 128-character table name (the G9
  step of T3).
- That `table.name()[-1]` is the table's name for a table of the real catalog (T7's tests
  use a local one; the G9 check of T8 is the proof).
- That the engine writes a quoted decimal with every digit and that the setting may be sent
  with the request (T10's integration test).
- That `pc.utf8_slice_codeunits` cuts a nine-digit fraction as T9 needs (its tests).
- That a constant expression in the `#[error(…)]` arguments compiles for a unit variant
  with the workspace's `thiserror` 2.0.21 (the form is used at
  `connector_discover.rs:158`).
- That the new sentences render as the console tests look for them.

## 3a. Machine

- Checkout: `/home/rismana/rantai/lakehouse-agents-1`.
- Before any cargo command:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1 CARGO_BUILD_JOBS=6`.
- The machine is shared. Work only under `/home/rismana/rantai/`. Start no container you do
  not remove.
- The Rust integration tests start containers: one Postgres through
  `lakehouse-test-support`, reused when it already runs, and `query_result_cap` starts a
  ClickHouse of its own. Before the first `cargo test --test …`, run `docker ps -a` and
  write down what is there. After the last one, remove what your runs started and nothing
  else (`docker rm -f <id>`); a container that was there before belongs to another
  checkout. Say in the handoff which you removed.
- The optional gate run of T3 uses its own compose project name and ends with `down -v`.

## 4. Out of scope

- **Shortening the pipeline name `bronze_ingest_<table>`** (D5). Not needed for the fix,
  and it would orphan the stored state of existing long-named tables.
- **The catalog server's or the engine's own limit on a table name**, if the gate finds
  one (D7): reported, not worked around here.
- **An empty text cell stored as no value** (`LH-DATA-9-32`, D18) and **a decimal column
  that keeps the file's precision and scale** (D19): signed decisions; questions 1 and 2.
- **The total of a cut list** ("the newest 100 of N", D11): question 3.
- **Decimals read as floats outside `POST /api/query/run`**: dashboard tiles
  (`routes/support.rs:413`) and the download of a result read numbers as numbers, and a
  change there changes what charts, exports and the assistant's dashboard tools receive.
  Not a QA case of this slice.
- **`user` in the Query History list** (`lakehouse-store/src/queries.rs:1330`) still
  carries the id; the list holds only the caller's own rows and the page does not show it.
- **A backfill of authors**, the catalog's `author` of an uploaded table (stays `upload`),
  and an author for merge loads (`dlt`'s own write).
- **A nanosecond note for Avro files**: the preview carries a declared type per column for
  Parquet only. An Avro timestamp longer than microseconds, if a file has one, fits after
  T9 and is cut without the note.
- **A damaged file whose first bytes read as text**: it is taken as delimited text and
  fails at the Check step or in the load with that step's own message (`sniff`,
  `upload_parse.rs:107-135`).
- **The comment at `src/lib/uploads.ts:232`** ("one of the seven") and other counts in
  comments this slice does not touch.
- Everything else in the QA round that is not on rows `DATA-50` to `DATA-53`.

### Open questions for the owner

None of these blocks the plan.

1. Should an empty cell of a **text** column be stored as no value (NULL)? Today it is an
   empty text, also for a null in a Parquet string column, as decision 7 of
   `upload-column-types.md` and "How a Parquet file is read" say. The planner's proposal:
   a null the file itself declares (Parquet, Avro, JSON `null`) becomes NULL, an empty CSV
   cell stays an empty text; its own backlog item, because it changes what existing loads
   of the same file produce.
2. Should a Parquet or Avro decimal keep its declared precision and scale
   (`decimal(10,2)`), instead of the one Decimal (38 digits, 9 after the point) of decision
   2? It needs a type with parameters in the column list, the job, the "add to its rows"
   check and the Check step. The planner's proposal: its own item. Until then the Check
   step says what a Decimal is stored as (T15), and row 31 of `upload-file.md`'s checklist
   ("the columns keep the types the file declared") is true for every type but a decimal's
   precision and scale.
3. Should a cut list say how many files there are ("the newest 100 of N")? It needs the
   list route to answer a total, which changes the shape of `GET /api/uploads` or adds a
   count route. The planner's proposal: a small item of its own, if wanted.
4. The author of an upload's version is the person's display name at the time, kept in the
   table's own history (D15). It stays there when the person is renamed or removed, and is
   visible to whoever may open the table's page. The planner's proposal is to keep it so;
   say if an id resolved when the page is read is wanted instead.
5. A connector's replace and append versions now name their run as author (D15, last
   point). The planner's proposal is to keep that; it is what `DATA-15-AC8` describes.

## 5. Handoff

## 6. Review
