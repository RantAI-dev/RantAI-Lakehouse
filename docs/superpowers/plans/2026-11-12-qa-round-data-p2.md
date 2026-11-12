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
  - Review round 1 R1: a name is answered only for a person who **shares a tenant with the
    caller** (`dashboard_access::user_names_in_tenants`, the "an empty list of tenants narrows
    nothing" rule and convention of `search_active_users`). The history the usage is built
    from has no tenant column, so it can hold a person of another tenant — a refused statement
    is recorded with `owner_id` — and every id of such a person is "Unknown user" in this
    tenant's usage, never their name.
  - Review round 1 R3: a service identity is shown by its own name. A service token's
    queries are recorded with its `service_identity.id`, which no `app_user` row holds, so
    `people_names` makes one more read (`dashboard_access::service_names`) for the ids the
    people read left out, and a live service identity is never merged into the row of
    deleted people. `service_identity` has no tenant column (`0001_init.sql`), so there is
    no tenant to narrow that read by: a service credential belongs to the deployment.
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
  chosen Integer accepts (`upload_types.rs:21-23`). Review round 1 R5 (this changes the
  decision): the Check step says so under **any** Timestamp column whose previewed values
  hold a fraction longer than six digits — whatever the file kind, so a CSV column someone
  set to Timestamp is no longer a silent loss — and under a Parquet column whose file type
  is `timestamp (ns…)` even when the previewed values are shorter. A fraction of ten digits
  or more does not fit the column, so the Check step lists that value instead.
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
  - Under a Timestamp column whose values keep nanoseconds — a Parquet `timestamp (ns…)`
    column, or a previewed value with a fraction longer than six digits, whatever the file
    kind: **"The file keeps
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

Branch `fix/qa-round-data-p2`, based on `51fb4833` (the plan) whose parent is
`94d86f39` (the head of `fix/qa-round-data-p1`, PR #484). Every commit was pushed
with `git push origin HEAD:fix/qa-round-data-p2`; nothing was force-pushed, `main`
is untouched, no pull request was opened.

### Commits (one per task, in plan order, plus one follow-up)

| Task | Commit | Title |
| --- | --- | --- |
| T1 | `4583c971` | test(dagster): reproduce the long table name failure (DATA-50, LH-DATA-9-37) |
| T2 | `692de2b6` | fix(dagster): a table with no schema is not there, without asking dlt (DATA-50, LH-DATA-9-37) |
| T3 | `154ac151` | test(g9): load into a table name of the full 128 characters (DATA-50, LH-DATA-9-37) |
| T4 | `01937b25` | fix(uploads): a damaged file of an accepted format says it could not be opened (DATA-51, LH-DATA-17-05) |
| T5 | `073c5eef` | fix(uploads): the unpacked refusal names the 2 GB limit (DATA-51, LH-DATA-17-13) |
| T6 | `61d8e22f` | fix(catalog): usage shows people by name, never by their id (DATA-52, LH-DATA-12-01 F3, LH-DATA-15-02, TC-1730) |
| T7 | `e5e2fe46` | fix(dagster): a replace and an append record the load's author (DATA-52, LH-DATA-15-08) |
| T8 | `cd3d5b87` | feat(uploads): the API tells the job who started the load (DATA-52, LH-DATA-15-08) |
| T9 | `bf0f031e` | fix(upload-types): a uint64 starts as Decimal and a timestamp takes nine digits (DATA-53, LH-DATA-9-32) |
| T10 | `0002b45c` | fix(query): the query route keeps a decimal's digits (DATA-53, LH-DATA-9-32) |
| T11 | `d5bb463a` | fix(upload): a selection over the 2 GB limit sends nothing (DATA-51, LH-DATA-17-04) |
| T12 | `73836763` | fix(uploads-panel): the cut list says so, and New subfolder says why it is off (DATA-51, LH-DATA-35-10, LH-DATA-35-07) |
| T13 | `4f89dc50` | fix(console): one list of formats for the upload (DATA-51, LH-DATA-9-02) |
| T14 | `c34a8173` | fix(catalog): two people with one name are two rows (DATA-52, LH-DATA-12-01 F3) |
| T15 | `87d031cc` | feat(console): the Check step says what a type keeps, and the page gives the name to query (DATA-53, LH-DATA-9-32, LH-DATA-9-07) |
| T16 | `d031121e` | docs(features): what the data round changed on the pages (DATA-50, DATA-51, DATA-52, DATA-53) |
| T8 (follow-up) | `5621b13e` | test(dagster): the loader and job tests' run configs carry the author (DATA-52, LH-DATA-15-08) |

`5621b13e` is not a task of its own: three Python tests outside the ones T8 names
build the job's config or its params and broke once `author` became required
(below, "Deviations").

### Tests added, and what they said before the fix

Each of these was run once against the code as it was before its commit (the
product change reverted in place, then restored), in the foreground.

- T1 `dagster/dispar_orchestrate/test_sink_long_table_name.py`, four functions,
  five tests with the parameters.
  Before any product change: `2 passed, 3 xfailed in 1.62s`, the three xfails being
  `InvalidSchemaName` (`test_a_first_typed_replace_into_a_long_table_name_loads[52]`
  and `[128]`, `test_clearing_a_long_named_table_that_was_never_loaded_says_there_is_nothing_to_empty`).
  `test_a_first_load_without_a_column_list_into_a_128_character_name_loads` passed
  before the fix too, as D4/D5 read it. (The first draft of test 1 called `.name` on
  what `make_schema_with_default_name` returns, a `str`; fixed before the commit.)
- T2: no new test; the two xfail marks removed, the same file `5 passed in 5.91s`.
- T4 `routes::uploads::tests::a_binary_file_named_like_an_accepted_format_is_told_it_could_not_be_opened`.
  Before: `panicked ... assertion left == right failed: a.parquet`
  `left: "This is not a file this upload reads. Upload CSV, TSV, JSON, JSON Lines, Avro, Parquet or Excel (.xls, .xlsx) files, a .gz of a text or JSON file, or a .zip of such files."`
  `right: "This file could not be opened as a Parquet file."`
  Plus the integration test
  `a_damaged_parquet_file_is_refused_at_completion_as_a_file_that_could_not_be_opened`
  in `tests/upload_routes.rs` (the unit test is the one run before the fix; the
  integration one passes at 176 passed in the same file's run).
- T5 `routes::uploads::tests::the_unpacked_sentences_name_the_unpacked_cap`. Before:
  `left: "The file is larger than the 2 GB limit once unpacked."`
  `right: "The file is larger than the upload limit once unpacked."` (the fixture had
  the new sentence, the code the old one).
  `the_reasons_the_api_knows_are_the_reasons_in_the_shared_fixture` and
  `test_there_are_twelve_different_failure_reasons_among_them_the_header_row_with_no_columns`
  changed their expected count from eleven to twelve (a rule change, said in the
  commit body), and
  `test_the_old_unpacked_sentence_stays_in_the_list_and_is_no_longer_the_one_raised`
  is new in `test_file_ingest.py`.
- T6 `frequent_users_are_named_from_the_user_table_and_ranked_by_their_queries`. Before:
  `left: Array [Object {"name": String("busy.old-handle"), "queries": Number(2)}, Object {"name": String("quiet.old-handle"), "queries": Number(1)}]`
  `right: Array [Object {"name": String("Busy"), "queries": Number(2)}, Object {"name": String("Quiet"), "queries": Number(1)}]`.
  New too: `a_person_who_no_longer_exists_is_unknown_user_and_never_an_id`,
  `a_row_without_an_owner_is_named_by_the_id_in_its_user_name_or_shown_as_stored`, and
  the caller's name asserted in `usage_counts_everyone_but_lists_only_the_callers_queries`.
- T7 `test_snapshot_author.py::test_a_replace_and_an_append_record_the_author_of_the_load`.
  Before: `assert [None, None] == ['a person (file upload)', 'a person (file upload)']`
  (`At index 0 diff: None != 'a person (file upload)'`). New too:
  `test_a_load_without_an_author_records_none_and_nothing_stays_registered`; and
  `test_typed_load.py`'s type-changing-replace test gained the author assertion.
- T8 `routes::uploads::tests::the_run_config_names_who_started_the_load`. Before:
  `left: Null right: "Rina Wijaya (file upload)"`. New too:
  `the_author_of_a_load_is_the_name_without_control_characters_or_file_upload_alone` and
  `test_file_ingest.py::test_the_sink_is_told_the_author_the_run_was_launched_with`.
- T9 `upload_types::tests::a_files_and_a_tables_own_types_become_the_starting_types`.
  Before: `assertion left == right failed: uint64` `left: (Integer, None)`
  `right: (Decimal, None)`. New too:
  `a_uint64_s_largest_value_fits_the_type_its_column_starts_with` and
  `the_guess_keeps_six_digits_of_a_fraction_and_a_chosen_timestamp_takes_nine`; the
  fixture's seven-digit case changed (a rule change, D17, said in the commit body),
  three cases added to `cast.json`, `guess.json` untouched.
- T10 `lakehouse-clickhouse` `query_with_options_asks_for_quoted_decimals_only_when_told`.
  Before: `panicked ... cancel_http_readonly_queries_on_client_close=1` (the query
  string the assert printed had no `output_format_json_quote_decimals`). Integration
  test `a_decimal_keeps_every_digit_the_engine_holds` in `tests/query_result_cap.rs`.
- T11 `planPick refuses every file of a selection that takes the upload over 2 GB, and a
  refused file does not count`. Before: `actual: ""` against
  `/^The 3 files chosen would bring the upload to .* over the 2 GB limit for one upload/`;
  and `planPick refuses both files of LH-DATA-17-04 before either is sent` failed the
  same way.
- T12 `says the list is cut when a hundred files came back, and not for ninety-nine`.
  Before: `Unable to find an element by: [data-testid="upload-list-cut"]`; the extended
  third-level folder test failed with `Unable to find an element by:
  [data-testid="folder-nest-reason"]`.
- T13 `ConnectorCreatePage > offers a link to upload a file ...`. Before:
  `Unable to find role="link" and name "Have a file instead? Upload CSV, TSV, JSON, JSON
  Lines, Avro, Excel or Parquet files"`. New too:
  `uploads-panel.test.tsx`'s empty state (title and sentence changed) and
  `src/lib/uploads.test.ts::the formats phrase names every format the Accepted fact lists`.
- T14 `lists two frequent users who share a name as two rows`. Before: the console
  logged `Encountered two children with the same key, "Rina Wijaya"` twice and the
  test failed on `Expected to not contain: "same key"`.
- T15 `says under a decimal column what is stored, and under a nanosecond timestamp that
  it is cut`. Before: `Unable to find an element with the text: Stored exactly, with up
  to 38 digits, 9 of them after the point.`; the table-step test failed with
  `Unable to find an element by: [data-testid="upload-query-name"]`; the review test's
  label list and the loaded-view test failed on the new label and sentence. New too:
  `src/lib/uploads.test.ts::keepsNanoseconds is true only for a nanosecond timestamp label`.
- T16: documents only.

### Commands run, with their real counts

Rust, from `rust/` with `CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1
CARGO_BUILD_JOBS=6` — the "before handoff" block of section 3, on `d031121e`
(`5621b13e` after it touches only three Python test files, so `fmt`, `clippy`,
`typecheck` and `lint` were re-run on it and the Rust/TS test binaries are the same):

- `cargo fmt --check` → clean.
- `cargo clippy -p lakehouse-clickhouse -p lakehouse-iceberg -p lakehouse-api --all-targets -- -D warnings` → `Finished` with no warning (`50.13s`; re-run on `5621b13e` in `0.34s`, cached).
- `cargo test -p lakehouse-clickhouse --lib query_with_options` → `2 passed; 0 failed`.
- `cargo test -p lakehouse-api --lib routes::uploads` → `61 passed; 0 failed`.
- `cargo test -p lakehouse-api --lib routes::catalog_governance` → `34 passed; 0 failed`.
- `cargo test -p lakehouse-api --lib upload_types` → `9 passed; 0 failed`.
- `cargo test -p lakehouse-api --lib upload_formats` → `16 passed; 0 failed`.
- `cargo test -p lakehouse-api --test upload_routes` → `176 passed; 0 failed` (`133.23s`; the same file also ran at `01937b25` and `073c5eef`, 176 passed each time).
- `cargo test -p lakehouse-api --test query_result_cap` → `10 passed; 0 failed` (`13.44s`).

Per commit, the scoped runs the table of section 3 names, all in the foreground:
T4 `--lib a_binary_file_named_like_an_accepted_format_is_told_it_could_not_be_opened`
(1 passed after the fix, the failure above before it) and
`--lib a_parquet_file_needs_both_its_first_bytes_and_a_parquet_name` (1 passed);
T5 the two `--lib` tests above (1 passed each) and `--lib upload_formats` (16 passed);
T6 `--lib routes::catalog_governance` (34 passed);
T8 the four `--lib` tests of the table (1 passed each);
T9 `--lib upload_types` (9 passed);
T10 `--lib query_with_options` (2 passed) and
`--lib a_null_cell_is_null_and_every_other_cell_keeps_its_text` (1 passed);
T14 no Rust test (a comment only), `cargo clippy -p lakehouse-iceberg --all-targets -- -D warnings` ran.

TypeScript, from the checkout:

- `bun install --frozen-lockfile` → `Checked 779 installs across 753 packages (no changes)`.
- `bun run typecheck` → no output (clean).
- `bun run lint` → `✖ 11 problems (0 errors, 11 warnings)`; none is in a file this slice
  changed the logic of (`src/lib/uploads.test.ts`'s two unused imports are in the base
  commit too, and the rest are untouched files: `data-table`, `sidebar`, `sso-page`,
  `alerts-page`, `login-page`, `use-copilot`, `use-data-table`, `dashboard-specs`,
  `http.ts`).
- `bun run test` → `4050 pass, 1 skip, 0 fail, 12246 expect() calls; Ran 4051 tests
  across 380 files` (`90.53s`).

Python, in `/home/rismana/rantai/venv-agents-1` (`uv venv`, then
`uv pip install ./dagster pytest==8.3.4`; `dlt 1.30.0`, `pyiceberg 0.12.0`):

- `(cd dagster && python -m pytest dispar_orchestrate -q -m "not loader")` →
  `2 failed, 2222 passed, 1 skipped, 313 deselected, 38 warnings in 16.98s`.
- `(cd dagster && python -m pytest dispar_orchestrate -q -m loader)` →
  `313 passed, 2225 deselected, 263 warnings in 815.92s (0:13:35)`.
- The two failures are not this branch's: both fail at `94d86f39` in a scratch
  worktree with the same venv —
  `test_adapters_oracle.py::test_a_tls_dial_really_encrypts_and_checks_the_certificate_dn`
  and `test_parquet_ingest.py::test_floats_are_written_in_the_shortest_form_that_reads_back[3.4028234663852886e+38-True-3.4028235e38]`
  (`2 failed, 16 passed` there when run alone).
- Per task, one file per call: T1 `test_sink_long_table_name.py` `2 passed, 3 xfailed`
  before T2 and `5 passed` after it; T2 also `test_sink_empty_table.py` 25 passed,
  `test_sink_history.py` 23 passed, `test_sink_history_guard.py` 43 passed,
  `test_typed_load.py` 20 passed, `test_snapshot_author.py` 5 passed;
  T5 `test_file_ingest.py` 209 passed, 1 skipped (211 passed after T8);
  T7 `test_snapshot_author.py` 7 passed, `test_typed_load.py` 20 passed,
  `test_iceberg_stream.py` 8 passed, `test_iceberg_snapshot.py` 7 passed;
  T8 `test_file_ingest.py` 211 passed, 1 skipped; T9 `test_upload_types.py` 94 passed.

Lints, always: `python3 ops/lint/check_bare_iceberg_count.py` →
`R11 guard: no bare count() against a Bronze Iceberg table found.`;
`python3 ops/lint/check_intra_package_imports.py` →
`OK: all intra-package dispar_orchestrate imports resolve`.

`git status` on the final commit → clean (nothing generated staged; no
`__pycache__/`, `*.pyc`, `rust/target/`).

Containers (section 3a): `docker ps -a` listed 32 containers before the first
`cargo test --test …` and 32 after the last, the same ids. The test harness reused
the running Postgres (`sleepy_pike`) and its own ClickHouse; my runs left nothing
behind, so nothing was removed.

### Not verified locally, CI

- `cargo test --workspace` (and CI's `cargo nextest run --workspace` partitions plus
  `cargo test --doc`): the plan makes it CI's, and this slice runs the files it names.
- The G9 gate, including T3's `step_long_table_name` and T8's author check in it: CI job
  "Rust · G9 file upload" on the pull request. Its docker commands were not run on this
  machine (the plan allows them only when that job fails without saying why). So that
  the **catalog server and the query engine take a 128-character table name** (D6/T3)
  and that **a real table's versions carry `<name> (file upload)` end to end** are
  *not verified locally, CI*.
- That `table.name()[-1]` is the table's name for a table of the real catalog (section 3's
  list of what the planner did not run): T7's tests use a local catalog. T8's G9 check in
  `step_long_table_name` reads a real table's versions, so CI is its proof.
- The CI run itself is on x86_64; everything above ran on this ARM64 machine.
- No compose file changed, so no `docker compose config`/`up` check was run (section 3).

### MISMATCH

None: every anchor was found by its text, the T1 reproduction failed with exactly
`InvalidSchemaName` (D4), and no decision had to be worked around. Two notes, not
mismatches:

- Some anchors had moved from the line the plan names (the plan read them at
  `d6d22a2c5`, this branch starts at `51fb4833` on `94d86f39`): `classify_head`'s
  `OtherBinary` arm is at `routes/uploads.rs:1045` now, the reasons and their fixture
  test further down the same file, `run_config` at `:2763`. Found by the quoted text.
- D9 says the job's sentence is "pinned to the API's built sentence" by a test. Here it
  is pinned by `the_unpacked_sentences_name_the_unpacked_cap` (API: `JOB_TOO_LARGE_UNPACKED
  == Refusal::TooLargeUnpacked.to_string()`, and the sentence names the cap) plus the
  shared fixture that both sides' count tests compare (twelve reasons). The job does not
  read the Rust source; the fixture is the join.

### Deviations worth a reviewer's look

- **T8 (`5621b13e`).** Making `author` a required field of `CONFIG_SCHEMA` broke three
  tests outside T8's list, all of which build that config or `FileIngestParams`:
  `test_several_files.py::test_the_op_config_takes_a_list_of_keys_and_the_params_keep_their_order`
  (`KeyError: 'author'`; it also asserts the config holds exactly the schema's fields),
  `test_authored_factory.py::TestFileIngestRunConfigReader::test_a_real_runs_config_resolves_to_the_table_the_load_wrote`
  (`DagsterInvalidConfigError: Missing required config entry "author"`) and
  `test_load_memory.py`'s probe (`TypeError: FileIngestParams.__init__() missing 1
  required positional argument: 'author'`). Each gained the field with the reason; no
  assertion was weakened. T8's own commit had been pushed already, so this is its own
  commit rather than an amend.
- **T5, the two names.** As the plan's items 3 and 5 say, the Rust constant of the old
  sentence is `JOB_TOO_LARGE_UNPACKED_UNNAMED` and the Python one is
  `TOO_LARGE_UNPACKED_UNNAMED`, while each side's `...UNPACKED` without the suffix is the
  new sentence; the Python raise sites therefore needed no edit. Each constant's comment
  says which sentence it holds and why the old one is kept.
- **T6, the caller's own name.** `people_names` inserts `caller.display_name` only when
  it is not blank, so a caller with no name is "Unknown user" like anyone else (D14's
  blank-name rule) rather than shown as an empty name.

## 6. Review

### Round 1 (planner, 2026-11-12), at `59f8a39e`

Read against the plan and `AGENTS.md`; nothing was run for this review, CI on the draft pull
request (#489) runs the suites and the upload gate. 2 BLOCKERs (R1, R2) and 7 SHOULD-FIX. The
long-name fix, the required `author` field and the quoted decimals showed no defect, and no
existing test was weakened.

**Planner's rulings** (where a ruling and a finding's "Fix" differ, the ruling holds).

- **R1 (adds to D14).** As the finding says: a name is resolved only for a person who shares a
  tenant with the caller; every other id reads "Unknown user". Say the rule in D14 and on the
  feature page.
- **R2: option (a), carry the author.** A PostgreSQL connector's load, a Kafka batch and the
  schema commit of a history load record the run's label, so what the documents say becomes
  true. Keep the label's form the other connector paths use.
- **R3 (adds to D14).** A service identity is shown by its own name (one more read for the ids
  `app_user` did not answer, in the caller's tenant like R1), never merged with deleted people.
- **R5 (changes D17).** The Check step shows the sentence under any Timestamp column whose
  sampled values hold a fraction longer than six digits, whatever the file kind; the bullet in
  `upload-column-types.md` says exactly when it appears.
- **R4, R6, R7, R8, R9.** As the findings say.

### R1. BLOCKER: The Usage panel can name a person of another tenant, because the name lookup has no tenant filter.

- Where: `rust/crates/lakehouse-api/src/routes/catalog_governance.rs:677-710` (`people_names`),
  `rust/crates/lakehouse-store/src/dashboard_access.rs:257-269` (`user_names`:
  `SELECT id, name … FROM app_user WHERE id = ANY($1)`),
  `rust/crates/lakehouse-store/src/queries.rs:472-502` (`history_mentioning`: `query_history` has no
  tenant column and the read has no tenant condition; "by anyone").
- What happens: person P belongs to tenant B only and types a statement that names tenant A's table.
  The tenant gate refuses it, and the refusal is recorded with `owner_id = P`
  (`routes/query.rs:1857-1880`, status `blocked`). `summarize_usage` counts every status
  (`catalog_governance.rs:515-531`), so P's row is in `reads`. `people_names` then asks `user_names`
  for P's id and gets P's display name, and a member of tenant A who opens the asset sees
  "<P's display name> · 1 query" under Frequent users. Before this branch that row showed P's id
  (what `query_history.user_name` holds). So a display name now crosses tenants. Other name reads in
  the store are tenant scoped (`report_subscriptions.rs:726-731`, `saved_query_access.rs:1460-1468`);
  this one is not. The plan (D14) does not decide it.
- Fix: resolve names only for ids that share a tenant with the caller. Add a store read next to
  `user_names` (for example `user_names_in_tenants(pool, ids, tenant_ids)`, the same select with
  `AND EXISTS (SELECT 1 FROM app_user_tenant ut WHERE ut.user_id = app_user.id AND ut.tenant_id = ANY($2))`),
  call it from `people_names` with `caller.tenant_ids`, and leave every other id out of the map so it
  is "Unknown user". Keep the caller's own insertion. Say the rule in D14 and on the feature page.
- Check: a database test (`lakehouse-store/tests/dashboard_access.rs` for the read, and one for
  `people_names`): two users, one in the caller's tenant and one in another tenant only; the map holds
  the first and not the second, and `frequentUsers` from it shows "Unknown user" for the second and
  never the second's name. Temporary wrong change that must make it fail: drop the `EXISTS` condition
  (or call `user_names` again).

### R2. BLOCKER: The documents and five comments say a connector's replace or append names its run; a PostgreSQL connector and a Kafka batch still record no author.

- Where: `docs/core/features/asset-page-depth.md:75` and `:86`, `CHANGELOG.md:53`,
  `src/services/contracts/lakehouse.ts:68-71`, `src/features/catalog/asset-activity.tsx:215-218`,
  `rust/crates/lakehouse-iceberg/src/rest.rs:158-163`; the code that contradicts them:
  `dagster/dispar_orchestrate/ingest_factory.py:774-782` (PostgreSQL goes through
  `dlt_pipeline.run_bronze_ingest`), `dagster/dispar_orchestrate/dlt_pipeline.py:346`
  (`SinkConfig.from_bronze_ingest_config(cfg)`, no `author`), `ingest_factory.py:1064-1065`
  (Kafka batch, no `author`), `dagster/dispar_orchestrate/adapters/history.py:504-518`
  (`ensure_columns` runs `pipeline.run` outside `snapshot_properties_for`).
- What happens: T7 records the author only when `SinkConfig.author` is set. `_run_author` is passed
  at three sites (`ingest_factory.py:738`, `:815`, `:893`): Clear, Oracle and the generic adapters.
  A PostgreSQL connector's load (replace, append, incremental, and its history mode) and every Kafka
  micro-batch build their `SinkConfig` with the default `author=""`, so `snapshot_properties("")` is
  `{}` and the version still says "unknown" after 2026-11-12. The zero-row commit a history load makes
  to create the table or add a column (`ensure_columns`) carries none either (the existing test only
  asserts the author is `in` the list). The feature page's spec row ("a merge load, and versions
  written before 2026-11-12, say unknown"), its new bullet, the changelog entry and open question 5 to
  the owner all state a behaviour the most common connector does not have (principle 2, rule 1). D15's
  "so a connector's replace and append versions now name their run" is not met for these paths.
- Fix: one of the two, the planner decides. (a) Carry the author: `run_bronze_ingest` gains
  `author: str = ""` and passes it to `SinkConfig.from_bronze_ingest_config(cfg, author=author)`;
  `ingest_factory.py:778` passes `_run_author(connector_id, run_id)`; the Kafka batch passes its
  connector's label; `history.load_history` wraps its `ensure_columns` call in
  `iceberg_stream.snapshot_properties_for(table, snapshot_properties(author))`. (b) Or correct the six
  texts to name what records an author (uploads; Oracle and the generic adapters) and what does not
  (PostgreSQL connectors, Kafka batches, merge loads, the schema commit of a history load).
- Check: for (a), a unit test that `ingest_factory`'s PostgreSQL branch hands `run_bronze_ingest` the
  run's label and that `run_bronze_ingest` builds a `SinkConfig` whose `author` is that label (capture
  the `load_via_sink` call, no network), and in `test_snapshot_author.py` assert every snapshot of a
  first history load has the author (`set(_authors(lab, "h")) == {"connector conn-x, run 9z8y"}`).
  Temporary wrong change: drop the `author=` argument at `dlt_pipeline.py:346`.

### R3. SHOULD-FIX: A live service identity is shown as "Unknown user" and merged into the row of deleted people.

- Where: `rust/crates/lakehouse-api/src/routes/catalog_governance.rs:370-378` (`person`), `:688-701`
  (`people_names` reads `app_user` only); `rust/crates/lakehouse-auth/src/service_token.rs:216-220`
  (`PrincipalId::Service(service_identity_id)`); `routes/query.rs:1995` and `:2073`
  (`owner_id: Some(p.id.uuid())` for any principal).
- What happens: a service token that runs queries is recorded with its `service_identity.id` as
  `owner_id`. That id is not in `app_user`, so `person` answers "Unknown user" and the token's queries
  are added to the same row as people who were deleted. The page then says an existing, named caller
  no longer exists, and one "Unknown user" row can be mostly a service's queries. Before, the row
  showed the id. D14 covers "an id with no user (deleted)" only.
- Fix: in `people_names`, also read the names of the ids found in `service_identity`
  (`SELECT id, name FROM service_identity WHERE id = ANY($1)`, one more read for the ids `app_user`
  did not answer) and put them in the map; or decide a second label for them. Say which in D14.
- Check: unit test on `summarize_usage` with a map that holds a service's id and name: its row shows
  the name and is not merged with a deleted person's; plus the database test of R4 with a
  `service_identity` row. Temporary wrong change: remove the second read.

### R4. SHOULD-FIX: No test reaches `people_names` or its wiring in `usage`; every new test hands `summarize_usage` a hand-made map.

- Where: `rust/crates/lakehouse-api/src/routes/catalog_governance.rs:677-710`, `:729`; tests
  `:1663-1669` (`named`) and the tests that use it. `grep -rn "frequentUsers\|people_names" tests src`
  finds no other test.
- What happens: the read, the blank-name filter, the "lookup failed, everyone but the caller is
  Unknown user" branch and the insertion of the caller are specified in T6 item 6 and D14 and have no
  test. If `usage` passed `&HashMap::new()`, or the filter kept blank names, every test of this branch
  would still pass while the page showed "Unknown user" for everyone (or an empty name). The QA case
  (names instead of ids on the page) is therefore not proven by a test.
- Fix: add a database test for `people_names` (the crate's tests already get a Postgres through
  `lakehouse_test_support`): three `app_user` rows, one with a blank name, mentions that name all
  three plus an id with no row.
- Check: the map holds the two named users and the caller, not the blank one and not the missing id;
  with the pool closed (or a dropped table) the map holds the caller only. Temporary wrong change:
  remove the `.filter(|(_, name, _)| !name.trim().is_empty())` line, and separately pass an empty map
  at `:729`.

### R5. SHOULD-FIX: A Timestamp a person chose (or a table's own Timestamp column on "add to its rows") now loses digits 7 to 9 of a CSV, JSON, Excel or Avro value with nothing on the Check step, where the value was listed as not fitting before.

- Where: `rust/crates/lakehouse-api/src/upload_types.rs:484` (`fits` takes `MAX_FRACTION_DIGITS`),
  `:750-775` (`check_rows` lists what does not fit, for every file kind);
  `dagster/dispar_orchestrate/upload_types.py:152-155`, `:212-216` (the cut);
  `src/features/connectors/upload-columns.tsx:108-112` (the note needs a Parquet file label);
  `docs/core/features/upload-column-types.md:75` ("… and the Check step says so").
- What happens: a CSV column with `2026-10-16T13:30:05.1234567` is guessed Text. If the person sets it
  to Timestamp, or adds the file to a table whose column is a timestamp (types fixed), the Check step
  used to list the value under "values that do not fit" and the person chose stop or empty. Now
  `fits` is true, nothing is listed, and the job stores `…05.123456`. The note is shown only when
  `fileTypes[index]` starts with `timestamp (ns`, which only a Parquet preview has. This is what D17
  says, so the developer followed the plan; but it is a silent loss the feature page denies, and rule
  14 prefers the earlier honest refusal. The plan lists only Avro as out of scope.
- Fix (planner decides): show the nanosecond sentence under any Timestamp column whose sampled values
  hold a fraction longer than six digits (the preview has the rows), or keep six digits as the fit for
  every file but Parquet nanosecond columns; and make the bullet at `upload-column-types.md:75` say
  exactly when the note appears.
- Check: console test: a CSV preview whose column is set to Timestamp and whose row holds a 7-digit
  fraction shows the sentence (or, with the other fix, the API's check lists the value). Temporary
  wrong change: restrict the condition to `keepsNanoseconds(...)` again.

### R6. SHOULD-FIX: `quote_decimals` was wired into two client functions the plan did not name, and two new comments say things the code does not do.

- Where: `rust/crates/lakehouse-clickhouse/src/lib.rs:161-166` (the field's comment: "Read by
  `ChClient::query_with_options` only"), `:591-593` (`query_stream_with_options`), `:677-679`
  (`byte_stream`); `dagster/dispar_orchestrate/adapters/iceberg_snapshot.py:123-127`.
- What happens: T10 item 1 says the option is read by `query_with_options` only. The branch also sends
  `output_format_json_quote_decimals=1` from the row stream and the byte stream when the flag is set.
  No caller sets it there today, so nothing changes now, but the comment is false (rule 1), the two
  sites have no test, and a later caller that copies the query route's options into a stream would
  change what a tile or a download receives, which D20 excludes. Second comment: the docstring of
  `write_one_snapshot` lists "the history mode, emptying a table" among the paths "that write through
  here"; its only callers are `sink.py:977` and `iceberg_stream.py:214`. History writes through
  `history.write_versions` and emptying through `iceberg_table.delete`.
- Fix: remove the two extra `if options.quote_decimals` blocks (or, if they are wanted, say so in the
  plan, correct the field comment and test them); reword the `write_one_snapshot` docstring to name
  its two callers and say the other paths carry the author through their own calls.
- Check: extend `query_with_options_asks_for_quoted_decimals_only_when_told` with a
  `query_stream_with_options` and a `byte_stream` call made with `quote_decimals: true` and assert the
  request's query string does not hold the setting. Temporary wrong change: put one block back.

### R7. SHOULD-FIX: The "newest 100 files" sentences hold a typed 100, not the constant, so the sentence and the condition can drift.

- Where: `src/features/connectors/uploads-panel.tsx:391-396`; the plan, T12 item 2 ("the number from
  the constant").
- What happens: the condition reads `UPLOAD_LIST_LIMIT`, the two sentences say "100" as text. With
  the constant at 200 the page would say "Showing the newest 100 files" under 200 rows, and no test
  would fail (the test compares the full sentence for 100 rows only).
- Fix: build both sentences with `${UPLOAD_LIST_LIMIT}`.
- Check: in `uploads-panel.test.tsx`, build the expected sentences from the imported
  `UPLOAD_LIST_LIMIT` and the row counts from it (`many(UPLOAD_LIST_LIMIT)`, `many(UPLOAD_LIST_LIMIT - 1)`).
  Temporary wrong change: set the constant to 99 with "100" still typed in the component; the test
  must fail on the sentence.

### R8. SHOULD-FIX: T1's first test does not make the schema at the limit, so "a table name of 50 is accepted" is not pinned.

- Where: `dagster/dispar_orchestrate/test_sink_long_table_name.py:37-43`; the plan, T1 test 1
  ("`dlt.Schema("bronze_ingest_" + "a" * 50)` is made").
- What happens: the test asserts `make_schema_with_default_name(prefix + "a" * 50)` returns its input
  (a string function, `dlt/extract/utils.py:329-335`) and that 51, 52 and 128 raise. It never calls
  `Schema(...)` with the 64-character name, so if a later `dlt` lowered the limit the test would still
  pass while 50-character names started to fail. No other test loads a name of 50 or 51.
- Fix: add `Schema(PIPELINE_PREFIX + "a" * 50)` (no exception) before the loop, as T1 says; keep the
  existing assertion.
- Check: the test itself. Temporary wrong change: make the accepted call use `"a" * 51`; it must
  raise `InvalidSchemaName` and fail the test.

### R9. SHOULD-FIX: Two decision cells of `upload-column-types.md` run two sentences together.

- Where: `docs/core/features/upload-column-types.md:58` ("… a mix stays text A fraction of a second
  …") and `:63` ("… to Decimal or Text A Parquet `uint64` …").
- What happens: the appended sentence follows a cell that ended without a full stop (the plan's text
  was appended literally), so the signed table reads as one broken sentence in both rows.
- Fix: put a full stop before the appended sentence in both cells. The "Signed" cells are correct
  (they say *proposed, not yet signed*).
- Check: read the two rows; no test.

### Round 1 fixes (developer)

Branch `fix/qa-round-data-p2`, off `8f1a9b05` (the review). Merge of `origin/main` into the
branch: `a54a4e79` (clean, `docs/core/BACKLOG.md` only). One finding per commit, each pushed.

| Finding | Commit | Change |
| --- | --- | --- |
| R1 BLOCKER | `dcc337a6` | `people_names` reads names through the new `dashboard_access::user_names_in_tenants`, so a name is answered only for a person in one of `caller.tenant_ids`; D14 and the feature page say the rule. |
| R2 BLOCKER | `e369be9c` | Option (a): `run_bronze_ingest` takes `author` and puts it in its `SinkConfig` (the PostgreSQL branch passes `_run_author`), the Kafka micro-batch passes its connector's label, and `history.ensure_columns` commits inside `iceberg_stream.snapshot_properties_for`. |
| R3 | `42f89238` | `people_names` makes one more read (`dashboard_access::service_names`) for the ids the people read left out, so a service identity is shown by its own name and not merged with deleted people. |
| R4 | `8518713e` | A database test goes through `usage` itself over `query_history` rows (named person, caller, blank name, id with no row, and a closed pool). |
| R5 | `117bcff8` | `cutsNanosecondDigits` shows the nanosecond sentence under any Timestamp column whose previewed values hold a fraction of seven to nine digits, or whose file type is `timestamp (ns…)`; D17, D23 and the feature bullet say exactly when. |
| R6 | `aeaa4ceb` | The `quote_decimals` block is gone from `query_stream_with_options` and `byte_stream` (the field comment is true again), and `write_one_snapshot`'s docstring names its two callers. |
| R7 | `3d6fa508` | Both cut-list sentences interpolate `UPLOAD_LIST_LIMIT`; the test builds its sentences and row counts from the same import. |
| R8 | `068d69de` | The first test of `test_sink_long_table_name.py` makes `Schema(PIPELINE_PREFIX + "a" * 50)` and keeps the helper assertion. |
| R9 | `3255a20c` | A full stop before the sentence appended to decision cells 5 and 10 of `upload-column-types.md`. |

**Proof** (the temporary wrong change the finding's Check names, and the failure it produced):

- R1 — `catalog_governance::tests::names::a_name_of_another_tenants_person_never_reaches_the_usage`;
  calling `user_names` again (no tenant condition):
  `assertion failed: !names.contains_key(&theirs)`.
- R2 — `test_run_bronze_ingest_hands_the_runs_label_to_the_sink`; dropping `author=` at
  `dlt_pipeline.py`'s `SinkConfig`: `AssertionError: assert ['', ''] == ['connector
  conn-x, run 9z8y', '']`. `test_every_snapshot_of_a_first_history_load_names_the_run`;
  removing the `snapshot_properties_for` wrap in `ensure_columns`: `AssertionError: assert
  {None, 'conne...-x, run 9z8y'} == {'connector conn-x, run 9z8y'}` (extra item `None`:
  the create commit of the load had no author). `test_a_stream_batch_records_the_runs_author_on_its_commits`
  pins the Kafka label.
- R3 — `catalog_governance::tests::names::a_service_identity_is_shown_by_its_own_name`;
  removing the second read: `assertion `left == right` failed: left: None, right:
  Some("svc-reporting")`.
- R4 — `catalog_governance::tests::names::the_asset_usage_names_people_through_the_read_not_a_hand_made_map`;
  removing the blank-name filter: the array held `{"name": "", "queries": 1}` plus a separate
  `Unknown user` row; passing `HashMap::new()` at the `usage` call site: `[{"name": "Unknown
  user", "queries": 4}]` where three rows were expected.
- R5 — `says under a CSV column set to Timestamp that a seven-digit fraction loses its last
  digits`; restricting the condition to `keepsNanoseconds(...)`: `Unable to find an element
  with the text: The file keeps nanoseconds. …` for a CSV value of `…05.1234567`.
  `cutsNanosecondDigits` also pins the seven-to-nine window (ten digits or more do not fit and
  the Check step lists them).
- R6 — `query_with_options_asks_for_quoted_decimals_only_when_told`; putting the block back in
  `query_stream_with_options`: `request 2:
  output_format_json_quote_decimals=1&cancel_http_readonly_queries_on_client_close=1`.
- R7 — `says the list is cut when a hundred files came back, and not for ninety-nine`;
  `UPLOAD_LIST_LIMIT = 99` with "100" typed in the component: `Expected: "Showing the newest 99
  files; search to find older ones." Received: "Showing the newest 100 files; search to find
  older ones."`.
- R8 — `test_dlt_refuses_a_schema_named_after_a_pipeline_of_more_than_64_characters`; making
  `Schema(... "a" * 51)` in the accepted call: `InvalidSchemaName` raised by `dlt.Schema`.
- R9 — read the two rows; no test.

**Not verified / notes**

- `MISMATCH: R3 (parenthetical "in the caller's tenant like R1")` — `service_identity` has no
  tenant column (`migrations/0001_init.sql:118-133`), and
  `lakehouse-auth/src/service_token.rs` reports `tenant_ids: Vec::new()` for every service
  principal. Applying the R1 tenant rule there would answer "Unknown user" for every service,
  contradicting the ruling's first clause, so the second read has no tenant to narrow by; D14
  says that and why. The rest of R3 is built as ruled.
- `cargo test --workspace` was **not** run, per the brief (CI runs the full suite). The scoped
  Rust commands below are the only Rust tests run.

**Final checks, on `3255a20c`** (`CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-1`,
`CARGO_BUILD_JOBS=6`; Python in `/home/rismana/rantai/venv-agents-1`):

```
cd rust && cargo fmt --check
  clean (no output)
cargo clippy -p lakehouse-api -p lakehouse-store -p lakehouse-iceberg --all-targets -- -D warnings
  Finished `dev` profile [unoptimized + debuginfo] target(s) in 32.61s (no warnings)
cargo test -p lakehouse-store --test dashboard_access
  test result: ok. 10 passed; 0 failed
cargo test -p lakehouse-api --lib catalog_governance
  test result: ok. 38 passed; 0 failed (3492 filtered out)
cargo test -p lakehouse-clickhouse --lib
  test result: ok. 42 passed; 0 failed
python -m pytest dispar_orchestrate/test_dlt_pipeline.py test_ingest_factory.py \
  test_snapshot_author.py test_sink_long_table_name.py test_iceberg_snapshot.py -q
  118 passed in 16.36s
bun run typecheck    clean
bun run lint         11 problems (0 errors, 11 warnings) — the existing warnings, none new
bun run test         4052 pass, 0 fail; Ran 4053 tests across 380 files [79.32s]
python3 ops/lint/check_bare_iceberg_count.py   OK
python3 ops/lint/check_intra_package_imports.py OK
git status           clean
```

### Round 2 (planner, 2026-11-12), at `b0f25af2`

Read the round-1 fix commits. R1 to R9 are fixed, one commit each, each new test shown red on
the wrong change before it was kept. A person's name is resolved only inside the caller's
tenants (`user_names_in_tenants`); a PostgreSQL connector's load, a Kafka batch and the schema
commit of a history load now carry the run's label, so the documents say what the code does.

**The developer's MISMATCH on R3 is right.** `service_identity` has no tenant column and a
service principal has no tenants, so "in the caller's tenant" cannot apply to it. A service
identity is an identity of the installation, not a person of a tenant: it is shown by its own
name, as built, and D14 says why.

No open BLOCKER. Still for CI to prove: the upload gate with a 128-character table name (T3)
and the author on a real table's versions (T8). Merge when CI on #489 is green.

### CI failure on #489: Dagster loader tests (developer)

CI's `Dagster · loader tests` job failed on `ac308a94`, in `pytest (loader)`
(`python -m pytest dispar_orchestrate -v -m loader` from `dagster/`), and the log was not
readable, so the job was reproduced here first: Python 3.11.18 in
`/home/rismana/rantai/venv-agents-3`, `pip install ./dagster pytest==8.3.4`, exactly what
the job installs — `dlt` 1.30.0, `pyiceberg` 0.12.0, `dagster` 1.13.20.

**The failing test, as first seen** (one failure, nothing else):

```
FAILED dispar_orchestrate/test_sink_history.py::test_a_history_refusal_is_recorded_as_a_rejected_run_with_its_fixed_message
E   TypeError: test_a_history_refusal_is_recorded_as_a_rejected_run_with_its_fixed_message.
    <locals>.refuse() got an unexpected keyword argument 'author'
        dispar_orchestrate/ingest_factory.py:778: TypeError
======== 1 failed, 313 passed, 2227 deselected, 481 warnings in 826.49s (0:13:46) ========
```

**Cause: the test, not the product** (case (a) of the brief). Review round 1 R2
(`e369be9c`) made `_run_one_object`'s PostgreSQL branch call
`run_bronze_ingest(…, author=_run_author(connector_id, run_id))`, and updated every stub of
that function in `test_ingest_factory.py` but not the `refuse(cfg, plan, gate=None)` stub in
`test_sink_history.py`. That module is marked `loader`, so the fast job
(`-m "not loader"`) never reached the new keyword and only the loader job did. The stub now
takes `author: str = ""` like the real function; its assertion is unchanged, because the
refusal is raised before any load and what the stub receives is not what this test is about
— the label actually reaching the sink is asserted by
`test_run_one_object_routes_a_postgres_driver_sql_connector_through_dlt_pipeline`. No
product code changed: the run's own author behaviour is correct as the plan decided it
(`test_snapshot_author.py`, `test_dlt_pipeline.py` and `test_ingest_factory.py` were green
in the same run).

**Commit:** `0437bf8f` `test(dagster): give the history-refusal stub the author the run now
passes`, pushed to `fix/qa-round-data-p2`.

**Final checks, on `0437bf8f`** (same venv, from `dagster/`):

```
python -m pytest dispar_orchestrate -v -m loader
  ======== 314 passed, 2227 deselected, 481 warnings in 853.45s (0:14:13) ========
python -m pytest dispar_orchestrate -q -m "not loader"
  2226 passed, 1 skipped, 314 deselected, 256 warnings in 19.32s
python3 ops/lint/check_intra_package_imports.py   OK: all intra-package dispar_orchestrate imports resolve
python3 ops/lint/check_bare_iceberg_count.py      R11 guard: no bare count() against a Bronze Iceberg table found.
git status                                        clean
```
